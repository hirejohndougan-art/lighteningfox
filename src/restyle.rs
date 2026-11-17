//! Restyle only what changed.
//!
//! After a script edits the page, most elements' styles are the same as before. Styling
//! keeps each element's previous style together with a signature of everything it was
//! computed from: its tag and attributes, its parent's style, and (only when the page's CSS
//! can see them) its position among its siblings, whether it's empty, and whether it's
//! hovered, pressed or focused. When the signature and the parent's style are unchanged,
//! the old style is reused; an element that changes is styled again along with everything
//! inside it.
//!
//! Anything this can't follow turns it off for that pass: `:has()`, `@container`, shadow
//! trees, and elements with animations or transitions (those are always restyled).

use crate::css::{Combinator, Stylesheet};
use crate::dom::{Dom, NodeData, NodeId};
use crate::style::Style;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

/// What the page's selectors can see beyond an element's own tag and attributes.
#[derive(Default, Clone, Copy)]
pub struct Features {
    /// Sibling combinators or structural pseudo-classes (:nth-child, :last-of-type…).
    pub siblings: bool,
    /// :empty
    pub empty: bool,
    /// Something this can't track: restyle everything.
    pub full: bool,
}

pub fn features<'a>(sheets: impl Iterator<Item = &'a Stylesheet>) -> Features {
    let mut f = Features::default();
    for sheet in sheets {
        for rule in &sheet.rules {
            if rule.container.is_some() {
                f.full = true;
            }
            for sel in &rule.selectors {
                if sel.combinators.iter().any(|c| matches!(c, Combinator::Adjacent | Combinator::Sibling)) {
                    f.siblings = true;
                }
                for p in sel.parts.iter().flat_map(|p| &p.pseudo) {
                    if p.contains("has(") || p.contains("host") {
                        f.full = true;
                    }
                    if p.contains("child") || p.contains("of-type") || p.contains('+') || p.contains('~') {
                        f.siblings = true;
                    }
                    if p.contains("empty") {
                        f.empty = true;
                    }
                }
            }
        }
    }
    f
}

fn mix(h: &mut u64, v: u64) {
    *h = (*h ^ v).wrapping_mul(0x100000001b3);
}

fn mix_str(h: &mut u64, s: &str) {
    for b in s.bytes() {
        mix(h, b as u64);
    }
    mix(h, 0xff);
}

/// Everything the whole pass depends on: the stylesheets' contents, the window size (for
/// @media), the reader's settings and the visited-links list.
pub fn global_key(sheets: &[Stylesheet], visited_gen: u64) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for s in sheets {
        mix(&mut h, s.rules.len() as u64);
        for r in &s.rules {
            mix(&mut h, r.selectors.len() as u64);
            if let Some(m) = &r.media {
                mix_str(&mut h, m);
            }
            if let Some(l) = &r.layer {
                mix_str(&mut h, l);
            }
            for sel in &r.selectors {
                for p in &sel.parts {
                    mix_str(&mut h, p.tag.as_deref().unwrap_or("*"));
                    p.classes.iter().for_each(|c| mix_str(&mut h, c));
                    p.pseudo.iter().for_each(|c| mix_str(&mut h, c));
                    if let Some(i) = &p.id {
                        mix_str(&mut h, i);
                    }
                    for (k, op, v) in &p.attrs {
                        mix_str(&mut h, k);
                        mix(&mut h, *op as u64);
                        mix_str(&mut h, v.as_deref().unwrap_or(""));
                    }
                }
                sel.combinators.iter().for_each(|c| mix(&mut h, *c as u64));
            }
            for d in &r.decls {
                mix_str(&mut h, &d.name);
                mix_str(&mut h, &d.value);
                mix(&mut h, d.important as u64);
            }
        }
    }
    let (w, vh) = crate::css::viewport();
    mix(&mut h, w.to_bits() as u64);
    mix(&mut h, vh.to_bits() as u64);
    let s = crate::settings::get();
    mix(&mut h, s.min_font_size as u64);
    mix(&mut h, s.reduce_motion as u64);
    mix(&mut h, visited_gen);
    h
}

/// The elements on the hover, active and focus chains (an element and its ancestors).
pub struct States {
    hover: HashSet<NodeId>,
    active: HashSet<NodeId>,
    focus_within: HashSet<NodeId>,
    focus: Option<NodeId>,
}

impl States {
    pub fn new(dom: &Dom, hover: Option<NodeId>, active: Option<NodeId>, focus: Option<NodeId>) -> States {
        let chain = |start: Option<NodeId>| {
            let mut set = HashSet::new();
            let mut cur = start.filter(|&n| n < dom.nodes.len());
            while let Some(n) = cur {
                if !set.insert(n) {
                    break;
                }
                cur = dom.nodes[n].parent;
            }
            set
        };
        States { hover: chain(hover), active: chain(active), focus_within: chain(focus), focus }
    }
}

/// Element `id`'s inputs (everything but its parent's style).
pub fn signature(dom: &Dom, id: NodeId, f: &Features, st: &States) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let NodeData::Element { tag, attrs } = &dom.nodes[id].data else { return 0 };
    mix_str(&mut h, tag);
    for (k, v) in attrs {
        mix_str(&mut h, k);
        mix_str(&mut h, v);
        // dir="auto" looks at the element's text.
        if k == "dir" && v.eq_ignore_ascii_case("auto") {
            mix_str(&mut h, &dom.text_content(id));
        }
    }
    mix(&mut h, st.hover.contains(&id) as u64 | (st.active.contains(&id) as u64) << 1 | (st.focus_within.contains(&id) as u64) << 2 | ((st.focus == Some(id)) as u64) << 3);
    if f.siblings {
        // Position among the element siblings and what they are.
        let parent = dom.nodes[id].parent.unwrap_or(0);
        for &s in &dom.nodes[parent].children {
            if let NodeData::Element { tag, .. } = &dom.nodes[s].data {
                mix_str(&mut h, tag);
                mix_str(&mut h, dom.attr(s, "id").unwrap_or(""));
                mix_str(&mut h, dom.attr(s, "class").unwrap_or(""));
                mix(&mut h, (s == id) as u64);
            }
        }
    }
    if f.empty {
        let empty = dom.nodes[id].children.iter().all(|&k| matches!(&dom.nodes[k].data, NodeData::Text(t) if t.is_empty()) || matches!(&dom.nodes[k].data, NodeData::Comment(_)));
        mix(&mut h, empty as u64);
    }
    h
}

/// One element's remembered style: its signature, its parent's style, and the result.
pub struct Entry {
    pub sig: u64,
    pub parent: Rc<Style>,
    pub style: Rc<Style>,
}

#[derive(Default)]
pub struct Cache {
    pub key: u64,
    pub root: Option<Rc<Style>>,
    pub nodes: Vec<Option<Entry>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    static VISITED_GEN: Cell<u64> = const { Cell::new(0) };
}

/// The visited-links list changed: styles that depended on it are stale.
pub fn visited_changed() {
    VISITED_GEN.with(|g| g.set(g.get() + 1));
}

pub fn visited_gen() -> u64 {
    VISITED_GEN.with(Cell::get)
}

/// The cache from the last pass, if it was made under the same key.
pub fn take(key: u64) -> Cache {
    let c = CACHE.with(|c| std::mem::take(&mut *c.borrow_mut()));
    if c.key == key { c } else { Cache { key, ..Default::default() } }
}

pub fn store(c: Cache) {
    CACHE.with(|cell| *cell.borrow_mut() = c);
}

/// Forget everything (the next pass styles every element).
pub fn clear() {
    CACHE.with(|c| *c.borrow_mut() = Cache::default());
}

/// Is reuse allowed at all? `IRONSWIFT_FULL_RESTYLE=1` turns it off.
pub fn enabled() -> bool {
    std::env::var_os("IRONSWIFT_FULL_RESTYLE").is_none()
}

#[cfg(test)]
mod tests {
    use crate::dom::NodeData;

    fn dump(s: &[std::rc::Rc<crate::style::Style>]) -> Vec<String> {
        s.iter().map(|x| format!("{x:?}")).collect()
    }

    /// After each kind of DOM change, the incremental result equals a full restyle.
    #[test]
    fn incremental_matches_full() {
        let css = "ul li { color: red } li:last-child { font-weight: bold } li + li { margin-top: 3px } .hot { color: blue } .box .hot { padding: 2px }
                   p:empty { height: 5px } div[data-x=a] span { font-style: italic } li::before { content: counter(list-item) '. ' }";
        let sheet = crate::css::parse(css);
        let sheets = std::slice::from_ref(&sheet);
        let mut dom = crate::html::parse("<div class=box><ul><li>a</li><li>b</li></ul><p></p><span>s</span></div>");
        let check = |dom: &crate::dom::Dom, label: &str| {
            let inc = crate::style::compute(dom, sheets);
            super::clear();
            let full = crate::style::compute(dom, sheets);
            assert_eq!(dump(&inc), dump(&full), "after {label}");
        };
        crate::style::compute(&dom, sheets);
        // Add a list item at the end: the old last one loses :last-child.
        let ul = dom.find_all("ul")[0];
        let li = dom.create(NodeData::Element { tag: "li".into(), attrs: vec![] });
        dom.insert(ul, li, None);
        check(&dom, "appending a list item");
        // A class change.
        let first = dom.find_all("li")[0];
        dom.set_attr(first, "class", "hot");
        check(&dom, "a class change");
        // An ancestor's attribute that a descendant selector reads.
        let div = dom.find_all("div")[0];
        dom.set_attr(div, "data-x", "a");
        check(&dom, "an ancestor attribute");
        // Text into the empty paragraph.
        let p = dom.find_all("p")[0];
        let t = dom.create(NodeData::Text("now full".into()));
        dom.insert(p, t, None);
        check(&dom, "filling an empty element");
        // Removing an element.
        dom.detach(first);
        check(&dom, "removing an element");
        // Nothing changed: everything reused, same result.
        check(&dom, "no change");
    }
}
