//! Faster selector matching.
//!
//! - Rule hashing: each stylesheet's rules are filed under the id, class or tag of their
//!   selectors' rightmost part, so an element only looks at rules that could match it
//!   (`p.note a` can't match a `<div>`, and isn't even checked).
//! - Ancestor bloom filter: while styling the tree top-down, each element carries a small
//!   bit set of its ancestors' ids, classes and tags. A selector like `.sidebar ul li`
//!   is rejected straight away when no ancestor has class `sidebar`, without walking up.

use crate::css::{Combinator, Selector, Stylesheet};
use crate::dom::{Dom, NodeId};
use std::collections::HashMap;

/// A stylesheet's rules filed by what their selectors need on the element itself.
#[derive(Default)]
pub struct SheetIndex {
    by_id: HashMap<String, Vec<u32>>,
    by_class: HashMap<String, Vec<u32>>,
    by_tag: HashMap<String, Vec<u32>>,
    /// Rules with a selector whose rightmost part has no id, class or tag (`*`, `[x]`, `:is(…)`).
    other: Vec<u32>,
}

impl SheetIndex {
    pub fn new(sheet: &Stylesheet) -> SheetIndex {
        let mut ix = SheetIndex::default();
        for (i, rule) in sheet.rules.iter().enumerate() {
            let i = i as u32;
            for sel in &rule.selectors {
                let Some(last) = sel.parts.last() else { continue };
                let list = if let Some(id) = &last.id {
                    ix.by_id.entry(id.clone()).or_default()
                } else if let Some(c) = last.classes.first() {
                    ix.by_class.entry(c.clone()).or_default()
                } else if let Some(t) = &last.tag {
                    ix.by_tag.entry(t.clone()).or_default()
                } else {
                    &mut ix.other
                };
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
        }
        ix
    }

    /// The rules that might match element `id`, in sheet order, into `out`.
    pub fn candidates(&self, dom: &Dom, id: NodeId, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.other);
        if let Some(v) = dom.attr(id, "id").and_then(|i| self.by_id.get(i)) {
            out.extend_from_slice(v);
        }
        if !self.by_class.is_empty() {
            for c in dom.classes(id) {
                if let Some(v) = self.by_class.get(c) {
                    out.extend_from_slice(v);
                }
            }
        }
        if let Some(v) = dom.tag(id).and_then(|t| self.by_tag.get(t)) {
            out.extend_from_slice(v);
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// 256 bits: the ids, classes and tags of an element's ancestors.
pub type Bloom = [u64; 4];

fn hash(kind: u8, s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ kind as u64;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

fn set(b: &mut Bloom, h: u64) {
    for k in [h & 255, (h >> 8) & 255] {
        b[(k >> 6) as usize] |= 1 << (k & 63);
    }
}

fn has(b: &Bloom, h: u64) -> bool {
    [h & 255, (h >> 8) & 255].iter().all(|k| b[(k >> 6) as usize] & (1 << (k & 63)) != 0)
}

/// The bloom for the children of `id`: its own bloom plus its id, classes and tag.
pub fn child_bloom(dom: &Dom, id: NodeId, own: &Bloom) -> Bloom {
    let mut b = *own;
    if let Some(t) = dom.tag(id) {
        set(&mut b, hash(0, t));
        if let Some(i) = dom.attr(id, "id") {
            set(&mut b, hash(1, i));
        }
        for c in dom.classes(id) {
            set(&mut b, hash(2, c));
        }
    }
    b
}

/// False when the selector certainly can't match an element with these ancestors: a part
/// reached only through descendant/child combinators needs an id, class or tag that no
/// ancestor has. (True means "maybe": full matching decides.)
pub fn may_match(sel: &Selector, ancestors: &Bloom) -> bool {
    let n = sel.parts.len();
    if n < 2 {
        return true;
    }
    for k in (0..n - 1).rev() {
        // A sibling combinator to the right of this part: it's not necessarily an ancestor.
        if matches!(sel.combinators[k], Combinator::Adjacent | Combinator::Sibling) {
            return true;
        }
        let p = &sel.parts[k];
        if p.tag.as_deref().is_some_and(|t| !has(ancestors, hash(0, t))) {
            return false;
        }
        if p.id.as_deref().is_some_and(|i| !has(ancestors, hash(1, i))) {
            return false;
        }
        if p.classes.iter().any(|c| !has(ancestors, hash(2, c))) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_and_bloom() {
        let dom = crate::html::parse("<div class=side><ul><li id=x class='a b'>t</li></ul></div><p>q</p>");
        let sheet = crate::css::parse("li {a:1} .b {a:2} #x {a:3} p {a:4} * {a:5} .side li {a:6} .main li {a:7} [title] {a:8}");
        let ix = SheetIndex::new(&sheet);
        let li = dom.find_all("li")[0];
        let mut c = Vec::new();
        ix.candidates(&dom, li, &mut c);
        assert_eq!(c, vec![0, 1, 2, 4, 5, 6, 7]);
        // The li's ancestors: html, body, div.side, ul.
        let mut chain = Vec::new();
        let mut cur = dom.nodes[li].parent;
        while let Some(p) = cur {
            chain.push(p);
            cur = dom.nodes[p].parent;
        }
        let mut b: Bloom = [0; 4];
        for &a in chain.iter().rev() {
            b = child_bloom(&dom, a, &b);
        }
        assert!(may_match(&sheet.rules[5].selectors[0], &b));
        assert!(!may_match(&sheet.rules[6].selectors[0], &b));
    }
}

#[cfg(test)]
mod bench {
    /// `cargo test --release style_speed -- --ignored --nocapture`: how long styling a big
    /// Bootstrap page takes.
    #[test]
    #[ignore]
    fn style_speed() {
        let css = std::fs::read_to_string("tests/real-world/bootstrap.min.css").unwrap();
        let sheet = crate::css::parse(&css);
        let rows: String = (0..600)
            .map(|i| format!("<div class='card mb-3'><div class='card-body'><h5 class='card-title'>Item {i}</h5><p class='card-text text-muted'>Text <a href='#x' class='link-primary'>link</a> <span class='badge bg-secondary'>tag</span></p><ul class='list-group'><li class='list-group-item'>A</li><li class='list-group-item active'>B</li></ul></div></div>"))
            .collect();
        let dom = crate::html::parse(&format!("<div class=container>{rows}</div>"));
        let t = std::time::Instant::now();
        for _ in 0..5 {
            crate::style::compute(&dom, std::slice::from_ref(&sheet));
        }
        eprintln!("style_speed: {} nodes, {} rules: {:?} per restyle", dom.nodes.len(), sheet.rules.len(), t.elapsed() / 5);
    }
}
