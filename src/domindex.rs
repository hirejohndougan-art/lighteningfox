//! Elements indexed by class and tag name, so `querySelectorAll(".item")`,
//! `getElementsByClassName` and `getElementsByTagName` look at the matching elements
//! instead of walking the whole document.
//!
//! The index covers the document tree (not shadow trees or detached nodes) and is rebuilt
//! lazily: any DOM change makes it stale, and it is only rebuilt once a page asks a second
//! query between two changes, so a script that alternates one change and one query doesn't
//! pay for rebuilding it every time.

use crate::css::Selector;
use crate::dom::{Dom, NodeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Default)]
struct Index {
    /// DOM generation the index was built for.
    built: u64,
    /// Queries asked in the current generation (built on the second).
    asked: (u64, u32),
    classes: HashMap<String, Rc<Vec<NodeId>>>,
    tags: HashMap<String, Rc<Vec<NodeId>>>,
}

thread_local! {
    static INDEX: RefCell<Index> = RefCell::new(Index { built: u64::MAX, ..Default::default() });
}

fn build(d: &Dom, ix: &mut Index) {
    let mut classes: HashMap<String, Vec<NodeId>> = HashMap::new();
    let mut tags: HashMap<String, Vec<NodeId>> = HashMap::new();
    d.walk(Dom::ROOT, |n| {
        if let Some(t) = d.tag(n) {
            tags.entry(t.to_string()).or_default().push(n);
            if let Some(c) = d.attr(n, "class") {
                for name in c.split_ascii_whitespace() {
                    let list = classes.entry(name.to_string()).or_default();
                    // `class="a a"` lists the element once.
                    if list.last() != Some(&n) {
                        list.push(n);
                    }
                }
            }
        }
        true
    });
    ix.classes = classes.into_iter().map(|(k, v)| (k, Rc::new(v))).collect();
    ix.tags = tags.into_iter().map(|(k, v)| (k, Rc::new(v))).collect();
}

/// The elements a single selector could match, in document order, if the index can narrow
/// them down: those with its rightmost compound's class (or else tag). None = walk instead.
pub fn candidates(d: &Dom, generation: u64, sel: &Selector) -> Option<Rc<Vec<NodeId>>> {
    let last = sel.parts.last()?;
    let class = last.classes.first();
    let tag = last.tag.as_deref().filter(|t| *t != "*");
    if class.is_none() && tag.is_none() {
        return None;
    }
    INDEX.with(|ix| {
        let mut ix = ix.borrow_mut();
        if ix.built != generation {
            if ix.asked.0 != generation {
                ix.asked = (generation, 0);
            }
            ix.asked.1 += 1;
            if ix.asked.1 < 2 {
                return None;
            }
            build(d, &mut ix);
            ix.built = generation;
        }
        let empty = || Rc::new(Vec::new());
        Some(match class {
            Some(c) => ix.classes.get(c.as_str()).cloned().unwrap_or_else(empty),
            None => ix.tags.get(&tag?.to_ascii_lowercase()).cloned().unwrap_or_else(empty),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_and_tag_candidates() {
        let d = crate::html::parse("<div class='a b'><p class='a a'>x</p><span>y</span></div><p>z</p>");
        let sel = |s: &str| crate::css::parse_selector_list(s).into_iter().next().unwrap();
        // First query in a generation walks; the second builds the index.
        assert!(candidates(&d, 7, &sel(".a")).is_none());
        let a = candidates(&d, 7, &sel(".a")).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(candidates(&d, 7, &sel("div p")).unwrap().len(), 2);
        assert_eq!(candidates(&d, 7, &sel(".missing")).unwrap().len(), 0);
        assert!(candidates(&d, 7, &sel("*")).is_none());
        assert!(candidates(&d, 7, &sel("[x]")).is_none());
    }
}
