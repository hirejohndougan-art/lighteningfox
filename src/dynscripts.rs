//! Scripts that pages add themselves (`document.head.appendChild(script)`), the way webpack
//! chunks, analytics tags and most script loaders work.
//!
//! When a script inserts a `<script>` into the document: an inline one runs right away; one
//! with `src` is fetched and run on the next turn of the event loop (scripts added this way are
//! async), then gets a `load` (or `error`) event. A script runs at most once (the "already
//! started" flag): moving it, or one the parser already ran, doesn't run it again. Markup set
//! with innerHTML doesn't go through here, so its scripts stay inert, as in other browsers.

use crate::dom::{Dom, NodeData, NodeId};
use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    static STARTED: RefCell<HashSet<NodeId>> = RefCell::new(HashSet::new());
    /// External scripts waiting to be fetched and run, in insertion order.
    static PENDING: RefCell<Vec<NodeId>> = const { RefCell::new(Vec::new()) };
}

/// Mark a script as started (the parser ran it, or it's about to run).
pub fn mark_started(id: NodeId) {
    STARTED.with(|s| s.borrow_mut().insert(id));
}

/// Mark every script inside `root` as started (markup set with innerHTML: its scripts are inert).
pub fn mark_subtree(dom: &Dom, root: NodeId) {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if is_script(dom, n) {
            mark_started(n);
        }
        stack.extend(dom.nodes[n].children.iter().copied());
    }
}

/// A new document: forget the old one's scripts.
pub fn reset() {
    STARTED.with(|s| s.borrow_mut().clear());
    PENDING.with(|p| p.borrow_mut().clear());
}

/// Is `id` in the document (not in a detached tree or a template's contents)?
fn connected(dom: &Dom, id: NodeId) -> bool {
    let mut cur = Some(id);
    let mut guard = 0;
    while let Some(n) = cur {
        if n == Dom::ROOT {
            return true;
        }
        if dom.tag(n) == Some("template") && n != id {
            return false;
        }
        guard += 1;
        if guard > 100_000 {
            return false;
        }
        cur = dom.nodes[n].parent;
    }
    false
}

fn is_script(dom: &Dom, id: NodeId) -> bool {
    matches!(&dom.nodes[id].data, NodeData::Element { tag, .. } if tag == "script") && dom.ns_of(id) == crate::dom::Ns::Html
}

/// After `node` was inserted: the scripts in it that should start now, in tree order, split into
/// (inline ones to run right away, external ones now queued for the next tick).
pub fn inserted(dom: &Dom, node: NodeId) -> Vec<NodeId> {
    if !connected(dom, node) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if is_script(dom, n) {
            found.push(n);
        }
        if dom.tag(n) != Some("template") {
            stack.extend(dom.nodes[n].children.iter().rev().copied());
        }
    }
    let mut inline = Vec::new();
    for id in found {
        if STARTED.with(|s| s.borrow().contains(&id)) {
            continue;
        }
        // A script with neither src nor text waits (a later src or text would start it in other
        // browsers; that case is rare and isn't followed here).
        let has_src = dom.attr(id, "src").is_some_and(|s| !s.trim().is_empty());
        if !has_src && dom.text_content(id).trim().is_empty() {
            continue;
        }
        mark_started(id);
        if has_src {
            PENDING.with(|p| p.borrow_mut().push(id));
        } else {
            inline.push(id);
        }
    }
    inline
}

/// Take the external scripts waiting to run.
pub fn take_pending() -> Vec<NodeId> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_start_once_and_only_when_connected() {
        reset();
        let mut dom = crate::html::parse("<html><head></head><body></body></html>");
        let body = dom.find_all("body")[0];
        let s = dom.create(NodeData::Element { tag: "script".into(), attrs: vec![("src".into(), "a.js".into())] });
        // Not in the document yet: nothing starts.
        assert!(inserted(&dom, s).is_empty());
        assert!(take_pending().is_empty());
        dom.insert(body, s, None);
        assert!(inserted(&dom, s).is_empty());
        assert_eq!(take_pending(), vec![s]);
        // Moving it doesn't run it again.
        assert!(inserted(&dom, s).is_empty());
        assert!(take_pending().is_empty());
        // Inline scripts come back to run right away.
        let i = dom.create(NodeData::Element { tag: "script".into(), attrs: vec![] });
        let t = dom.create(NodeData::Text("x = 1".into()));
        dom.insert(i, t, None);
        dom.insert(body, i, None);
        assert_eq!(inserted(&dom, i), vec![i]);
        reset();
    }
}
