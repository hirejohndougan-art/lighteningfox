//! A page's accessibility tree sent from its site process to the window (process.rs, frames.rs):
//! the window's screen-reader bridge (a11y_bridge.rs) shows it like its own page's.
//! Encoded as JSON; everything read back is checked (the process can't be trusted).

use crate::a11y::{self, Node, States, Tree};
use crate::a11y_bridge::PageTree;
use crate::layout::Rect;
use serde_json::{json, Value};

/// At most this many nodes are sent (a huge page is cut, not refused).
const MAX_NODES: usize = 50_000;

pub fn encode(pt: &PageTree) -> Vec<u8> {
    let nodes: Vec<Value> = pt
        .tree
        .nodes
        .iter()
        .take(MAX_NODES)
        .enumerate()
        .map(|(i, n)| {
            let s = &n.states;
            let r = pt.rects.get(i).copied().flatten();
            json!({
                "d": n.dom, "r": n.role, "n": n.name, "ds": n.description, "v": n.value, "l": n.level,
                "rg": n.range.map(|(a, b, c)| [a, b, c]), "h": n.href,
                "c": n.children.iter().filter(|&&c| c < MAX_NODES).collect::<Vec<_>>(),
                "s": [s.focusable, s.focused, s.disabled, s.required, s.readonly, s.invalid, s.multiline, s.password, s.modal],
                "ck": s.checked.map(|c| match c { Some(true) => 1, Some(false) => 0, None => 2 }),
                "p": s.pressed, "e": s.expanded, "se": s.selected, "lv": s.live,
                "b": r.map(|(r, f)| json!([r.x, r.y, r.w, r.h, f])),
            })
        })
        .collect();
    serde_json::to_vec(&json!({ "u": pt.url, "n": nodes })).unwrap_or_default()
}

pub fn decode(b: &[u8]) -> Option<PageTree> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let list = v.get("n")?.as_array()?;
    if list.is_empty() || list.len() > MAX_NODES {
        return None;
    }
    let count = list.len();
    let mut tree = Tree::default();
    let mut rects = Vec::with_capacity(count);
    let st = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let opt_bool = |o: &Value, k: &str| o.get(k).and_then(Value::as_bool);
    for o in list {
        let flags: Vec<bool> = o.get("s").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_bool().unwrap_or(false)).collect()).unwrap_or_default();
        let f = |i: usize| flags.get(i).copied().unwrap_or(false);
        let states = States {
            focusable: f(0),
            focused: f(1),
            disabled: f(2),
            required: f(3),
            readonly: f(4),
            invalid: f(5),
            multiline: f(6),
            password: f(7),
            modal: f(8),
            checked: o.get("ck").and_then(Value::as_u64).map(|c| match c {
                1 => Some(true),
                0 => Some(false),
                _ => None,
            }),
            pressed: opt_bool(o, "p"),
            expanded: opt_bool(o, "e"),
            selected: opt_bool(o, "se"),
            live: match o.get("lv").and_then(Value::as_str) {
                Some("assertive") => Some("assertive"),
                Some(_) => Some("polite"),
                None => None,
            },
        };
        let range = o.get("rg").and_then(Value::as_array).and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?)));
        tree.nodes.push(Node {
            dom: o.get("d").and_then(Value::as_u64)? as usize,
            role: a11y::intern_role(&st(o, "r")),
            name: st(o, "n"),
            description: st(o, "ds"),
            value: o.get("v").and_then(Value::as_str).map(String::from),
            states,
            level: o.get("l").and_then(Value::as_u64).map(|l| l.min(100) as u32),
            range,
            href: o.get("h").and_then(Value::as_str).map(String::from),
            // Only later nodes can be children: no loops.
            children: o.get("c").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|c| c as usize).filter(|&c| c < count).collect()).unwrap_or_default(),
        });
        let rect = o.get("b").and_then(Value::as_array).and_then(|a| {
            let n = |i: usize| a.get(i).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite());
            Some((Rect { x: n(0)?, y: n(1)?, w: n(2)?.max(0.0), h: n(3)?.max(0.0) }, a.get(4)?.as_bool()?))
        });
        rects.push(rect);
    }
    for (i, n) in tree.nodes.iter_mut().enumerate() {
        n.children.retain(|&c| c > i);
    }
    Some(PageTree { tree, rects, url: v.get("u").and_then(Value::as_str).unwrap_or("").to_string(), scroll: 0.0, origin: (0.0, 0.0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trees_cross_the_process_boundary() {
        let mut tree = Tree::default();
        tree.nodes.push(Node { dom: 0, role: "document", name: "T".into(), children: vec![1], ..Node::default() });
        tree.nodes.push(Node { dom: 5, role: "link", name: "Go".into(), href: Some("/x".into()), states: States { focusable: true, ..States::default() }, ..Node::default() });
        let pt = PageTree { tree, rects: vec![None, Some((Rect { x: 1.0, y: 2.0, w: 30.0, h: 10.0 }, false))], url: "https://a.com/".into(), ..PageTree::default() };
        let back = decode(&encode(&pt)).unwrap();
        assert_eq!(back.tree.nodes, pt.tree.nodes);
        assert_eq!(back.url, "https://a.com/");
        assert!(matches!(back.rects[1], Some((r, false)) if r.w == 30.0));
        // A child pointing back up (a loop) is dropped; unknown roles are generic.
        let bad = br#"{"u":"x","n":[{"d":0,"r":"document","c":[0,1]},{"d":1,"r":"evil","c":[0]}]}"#;
        let t = decode(bad).unwrap();
        assert_eq!(t.tree.nodes[0].children, vec![1]);
        assert!(t.tree.nodes[1].children.is_empty());
        assert_eq!(t.tree.nodes[1].role, "generic");
        assert!(decode(b"not json").is_none());
    }
}
