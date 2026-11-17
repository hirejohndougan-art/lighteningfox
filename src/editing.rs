//! `contenteditable`: typing into page elements (rich text editors, comment boxes).
//!
//! The caret is a position in one of the editing host's text nodes. While a host is being
//! edited, layout reports where each of its words was drawn (`record_word`), which is how a
//! click becomes a caret position and how the caret finds its place on screen. Edits change
//! the DOM directly and fire `beforeinput` (cancellable) and `input`, like other browsers.

use std::cell::{Cell, RefCell};

use crate::dom::{Dom, NodeData, NodeId};
use crate::font::Fonts;

/// A caret: `offset` characters into text node `node`, inside editing host `host`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub host: NodeId,
    pub node: NodeId,
    pub offset: usize,
}

/// A word layout drew for a text node in the editing host.
#[derive(Debug, Clone)]
struct Word {
    node: NodeId,
    x: f32,
    baseline: f32,
    size: f32,
    font: u8,
    text: String,
    /// Where the word starts in its text node, in characters.
    start: usize,
}

thread_local! {
    static HOST: Cell<Option<NodeId>> = const { Cell::new(None) };
    static WORDS: RefCell<Vec<Word>> = const { RefCell::new(Vec::new()) };
}

/// Which host layout should report words for (None: no editing, nothing recorded).
pub fn set_host(host: Option<NodeId>) {
    HOST.with(|h| h.set(host));
    WORDS.with(|w| w.borrow_mut().clear());
}

/// The element being edited, whose words layout records.
pub fn host() -> Option<NodeId> {
    HOST.with(Cell::get)
}

/// Called when a layout starts.
pub fn begin_layout() {
    if HOST.with(Cell::get).is_some() {
        WORDS.with(|w| w.borrow_mut().clear());
    }
}

/// Called by layout for each word it draws (`text` as drawn, without an added hyphen).
pub fn record_word(dom: &Dom, node: NodeId, x: f32, baseline: f32, size: f32, font: u8, text: &str) {
    let Some(host) = HOST.with(Cell::get) else { return };
    if !matches!(dom.nodes.get(node).map(|n| &n.data), Some(NodeData::Text(_))) || !dom.is_inside(node, host) {
        return;
    }
    WORDS.with(|w| {
        let mut w = w.borrow_mut();
        // Where it starts: just past the previous word of the same node, skipping spaces.
        let after = w.iter().rev().find(|p| p.node == node).map_or(0, |p| p.start + p.text.chars().count());
        let chars: Vec<char> = match &dom.nodes[node].data {
            NodeData::Text(t) => t.chars().collect(),
            _ => Vec::new(),
        };
        let mut start = after;
        while start < chars.len() && chars[start].is_ascii_whitespace() {
            start += 1;
        }
        w.push(Word { node, x, baseline, size, font, text: text.to_string(), start });
    });
}

/// The editing host an element is in: the outermost element of a `contenteditable` region.
pub fn host_of(dom: &Dom, node: NodeId) -> Option<NodeId> {
    let mut cur = Some(node);
    let mut host = None;
    while let Some(n) = cur {
        if let Some(v) = dom.attr(n, "contenteditable") {
            match v.trim().to_ascii_lowercase().as_str() {
                "" | "true" | "plaintext-only" => host = Some(n),
                "false" => break,
                _ => {}
            }
        }
        cur = dom.nodes[n].parent;
    }
    host
}

fn text_of(dom: &Dom, node: NodeId) -> &str {
    match &dom.nodes[node].data {
        NodeData::Text(t) => t,
        _ => "",
    }
}

fn len(dom: &Dom, node: NodeId) -> usize {
    text_of(dom, node).chars().count()
}

/// The host's text nodes and line breaks, in document order.
fn pieces(dom: &Dom, host: NodeId) -> Vec<NodeId> {
    dom.descendants(host).into_iter().filter(|&n| matches!(dom.nodes[n].data, NodeData::Text(_)) || dom.tag(n) == Some("br")).collect()
}

/// A caret for a host that was just clicked or focused: at the end of its text (adding an
/// empty text node when it has none).
pub fn caret_at_end(dom: &mut Dom, host: NodeId) -> Caret {
    let texts: Vec<NodeId> = pieces(dom, host).into_iter().filter(|&n| dom.tag(n).is_none()).collect();
    match texts.last() {
        Some(&n) => Caret { host, node: n, offset: len(dom, n) },
        None => {
            let n = dom.add(NodeData::Text(String::new()), host);
            Caret { host, node: n, offset: 0 }
        }
    }
}

/// The caret for a click at (x, y) in page coordinates (unzoomed).
pub fn caret_at_point(dom: &mut Dom, fonts: &Fonts, host: NodeId, x: f32, y: f32) -> Caret {
    let best = WORDS.with(|w| {
        let w = w.borrow();
        // The line under the click (else the nearest one), then the nearest word on it.
        let line_dist = |wd: &Word| {
            let (top, bottom) = (wd.baseline - wd.size, wd.baseline + wd.size * 0.35);
            if y < top { top - y } else if y > bottom { y - bottom } else { 0.0 }
        };
        let nearest_line = w.iter().map(line_dist).fold(f32::INFINITY, f32::min);
        w.iter()
            .filter(|wd| (line_dist(wd) - nearest_line).abs() < 0.5)
            .min_by(|a, b| {
                let d = |wd: &Word| {
                    let end = wd.x + fonts.width(wd.font, &wd.text, wd.size);
                    if x < wd.x { wd.x - x } else if x > end { x - end } else { 0.0 }
                };
                d(a).total_cmp(&d(b))
            })
            .cloned()
    });
    match best {
        Some(wd) if dom.nodes.get(wd.node).is_some() => {
            let mut acc = wd.x;
            let mut i = 0;
            for c in wd.text.chars() {
                let cw = fonts.width(wd.font, &c.to_string(), wd.size);
                if x < acc + cw / 2.0 {
                    break;
                }
                acc += cw;
                i += 1;
            }
            Caret { host, node: wd.node, offset: (wd.start + i).min(len(dom, wd.node)) }
        }
        _ => caret_at_end(dom, host),
    }
}

/// Where to draw the caret: (x, top, height) in page coordinates (unzoomed).
/// `host_box`: the host's content box, for a caret on an empty line.
pub fn caret_rect(dom: &Dom, fonts: &Fonts, c: &Caret, host_box: (f32, f32), default_size: f32) -> (f32, f32, f32) {
    WORDS.with(|w| {
        let w = w.borrow();
        let own: Vec<&Word> = w.iter().filter(|wd| wd.node == c.node).collect();
        // In a word (or right after one).
        if let Some(wd) = own.iter().rev().find(|wd| wd.start <= c.offset && c.offset <= wd.start + wd.text.chars().count()) {
            let x = wd.x + fonts.width(wd.font, &wd.text.chars().take(c.offset - wd.start).collect::<String>(), wd.size);
            return (x, wd.baseline - wd.size * 0.95, wd.size * 1.2);
        }
        // In the spaces after a word of this node.
        if let Some(wd) = own.iter().rev().find(|wd| wd.start + wd.text.chars().count() <= c.offset) {
            let x = wd.x + fonts.width(wd.font, &wd.text, wd.size) + fonts.width(wd.font, " ", wd.size);
            return (x, wd.baseline - wd.size * 0.95, wd.size * 1.2);
        }
        if let Some(wd) = own.first() {
            return (wd.x, wd.baseline - wd.size * 0.95, wd.size * 1.2);
        }
        // An empty text node: after the last word before it, or at the start of a new line
        // when line breaks come in between.
        let order = pieces(dom, c.host);
        let pos = order.iter().position(|&n| n == c.node).unwrap_or(order.len());
        let mut breaks = 0;
        for &n in order[..pos].iter().rev() {
            if dom.tag(n) == Some("br") {
                breaks += 1;
                continue;
            }
            if let Some(wd) = w.iter().rev().find(|wd| wd.node == n) {
                let line = wd.size * 1.2;
                if breaks > 0 {
                    return (host_box.0, wd.baseline - wd.size * 0.95 + line * breaks as f32, line);
                }
                let x = wd.x + fonts.width(wd.font, &wd.text, wd.size);
                return (x, wd.baseline - wd.size * 0.95, line);
            }
        }
        let line = default_size * 1.2;
        (host_box.0, host_box.1 + line * breaks as f32, line)
    })
}

// ---------------------------------------------------------------- changing the text

fn set_text(dom: &mut Dom, node: NodeId, t: String) {
    if let NodeData::Text(s) = &mut dom.nodes[node].data {
        *s = t;
    }
}

/// Insert typed text at the caret. Spaces that HTML would collapse become no-break spaces.
pub fn insert_text(dom: &mut Dom, c: &mut Caret, text: &str) {
    let mut chars: Vec<char> = text_of(dom, c.node).chars().collect();
    c.offset = c.offset.min(chars.len());
    for ch in text.chars() {
        let pos = c.offset;
        let prev = pos.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(pos).copied();
        let ch = if ch == ' ' && (prev.is_none_or(|p| p.is_whitespace()) || next.is_none_or(|n| n.is_whitespace())) { '\u{a0}' } else { ch };
        // A no-break space before this character can go back to an ordinary one.
        if ch != ' ' && ch != '\u{a0}' && prev == Some('\u{a0}') && pos >= 2 && !chars[pos - 2].is_whitespace() {
            chars[pos - 1] = ' ';
        }
        chars.insert(pos, ch);
        c.offset += 1;
    }
    set_text(dom, c.node, chars.into_iter().collect());
}

/// Backspace: remove the character before the caret, or the line break before this line.
pub fn delete_backward(dom: &mut Dom, c: &mut Caret) -> bool {
    if c.offset > 0 {
        let mut chars: Vec<char> = text_of(dom, c.node).chars().collect();
        chars.remove(c.offset - 1);
        set_text(dom, c.node, chars.into_iter().collect());
        c.offset -= 1;
        return true;
    }
    let order = pieces(dom, c.host);
    let Some(pos) = order.iter().position(|&n| n == c.node) else { return false };
    for &n in order[..pos].iter().rev() {
        if dom.tag(n) == Some("br") {
            dom.detach(n);
            return true;
        }
        let l = len(dom, n);
        if l > 0 {
            *c = Caret { host: c.host, node: n, offset: l };
            return delete_backward(dom, c);
        }
    }
    false
}

/// Delete: remove the character after the caret, or the line break after this line.
pub fn delete_forward(dom: &mut Dom, c: &mut Caret) -> bool {
    let l = len(dom, c.node);
    if c.offset < l {
        let mut chars: Vec<char> = text_of(dom, c.node).chars().collect();
        chars.remove(c.offset);
        set_text(dom, c.node, chars.into_iter().collect());
        return true;
    }
    let order = pieces(dom, c.host);
    let Some(pos) = order.iter().position(|&n| n == c.node) else { return false };
    for &n in &order[pos + 1..] {
        if dom.tag(n) == Some("br") {
            dom.detach(n);
            return true;
        }
        if len(dom, n) > 0 {
            let mut chars: Vec<char> = text_of(dom, n).chars().collect();
            chars.remove(0);
            set_text(dom, n, chars.into_iter().collect());
            return true;
        }
    }
    false
}

/// Enter: break the line at the caret (a `<br>`, with the rest of the text after it).
pub fn insert_line_break(dom: &mut Dom, c: &mut Caret) {
    let chars: Vec<char> = text_of(dom, c.node).chars().collect();
    let at = c.offset.min(chars.len());
    let (head, tail): (String, String) = (chars[..at].iter().collect(), chars[at..].iter().collect());
    set_text(dom, c.node, head);
    let parent = dom.nodes[c.node].parent.unwrap_or(c.host);
    let next = {
        let kids = &dom.nodes[parent].children;
        kids.iter().position(|&k| k == c.node).and_then(|i| kids.get(i + 1)).copied()
    };
    let br = dom.create(NodeData::Element { tag: "br".into(), attrs: Vec::new() });
    dom.insert(parent, br, next);
    let rest = dom.create(NodeData::Text(tail));
    dom.insert(parent, rest, next);
    *c = Caret { host: c.host, node: rest, offset: 0 };
}

/// Move the caret one character left or right (into the next text node at the ends).
pub fn move_by(dom: &Dom, c: &mut Caret, right: bool) -> bool {
    let l = len(dom, c.node);
    if right && c.offset < l {
        c.offset += 1;
        return true;
    }
    if !right && c.offset > 0 {
        c.offset -= 1;
        return true;
    }
    let texts: Vec<NodeId> = pieces(dom, c.host).into_iter().filter(|&n| dom.tag(n).is_none()).collect();
    let Some(i) = texts.iter().position(|&n| n == c.node) else { return false };
    let next = if right { texts.get(i + 1) } else { i.checked_sub(1).and_then(|j| texts.get(j)) };
    match next {
        Some(&n) => {
            *c = Caret { host: c.host, node: n, offset: if right { 0 } else { len(dom, n) } };
            true
        }
        None => false,
    }
}

/// Home/End: the start or end of the caret's line (as laid out).
pub fn line_edge(fonts: &Fonts, c: &mut Caret, end: bool) {
    let _ = fonts;
    WORDS.with(|w| {
        let w = w.borrow();
        let Some(cur) = w.iter().find(|wd| wd.node == c.node && wd.start <= c.offset && c.offset <= wd.start + wd.text.chars().count()) else { return };
        let line: Vec<&Word> = w.iter().filter(|wd| (wd.baseline - cur.baseline).abs() < 0.5).collect();
        let pick = if end { line.last() } else { line.first() };
        if let Some(wd) = pick {
            c.node = wd.node;
            c.offset = if end { wd.start + wd.text.chars().count() } else { wd.start };
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_edits() {
        let mut dom = crate::html::parse(r#"<div contenteditable><p>Hello <b>big</b> world</p><span contenteditable=false>x</span></div>"#);
        let div = dom.find_all("div")[0];
        let b = dom.find_all("b")[0];
        let span = dom.find_all("span")[0];
        assert_eq!(host_of(&dom, b), Some(div));
        assert_eq!(host_of(&dom, span), None);
        let mut c = caret_at_end(&mut dom, div);
        assert_eq!(text_of(&dom, c.node), "x");
        // Type into "Hello ": a space at the end of a text node stays visible (no-break).
        let hello = dom.descendants(div).into_iter().find(|&n| text_of(&dom, n) == "Hello ").unwrap();
        c = Caret { host: div, node: hello, offset: 5 };
        insert_text(&mut dom, &mut c, ",");
        assert_eq!(text_of(&dom, hello), "Hello, ");
        delete_backward(&mut dom, &mut c);
        assert_eq!(text_of(&dom, hello), "Hello ");
        insert_line_break(&mut dom, &mut c);
        assert_eq!(dom.inner_html(div).split("</p>").next().unwrap(), "<p>Hello<br> <b>big</b> world");
        // Backspace at the start of the new line removes the break.
        assert!(delete_backward(&mut dom, &mut c));
        assert!(!dom.inner_html(div).contains("<br>"));
        // Moving right from the end of "Hello" goes into the next text node.
        c = Caret { host: div, node: hello, offset: 5 };
        assert!(move_by(&dom, &mut c, true));
        assert_ne!(c.node, hello);
    }

    #[test]
    fn spaces() {
        let mut dom = crate::html::parse(r#"<div contenteditable>ab</div>"#);
        let div = dom.find_all("div")[0];
        let mut c = caret_at_end(&mut dom, div);
        insert_text(&mut dom, &mut c, " ");
        assert_eq!(text_of(&dom, c.node), "ab\u{a0}");
        insert_text(&mut dom, &mut c, "c");
        assert_eq!(text_of(&dom, c.node), "ab c");
    }
}
