//! The Document Object Model: a tree of nodes stored in a flat arena.
//! Nodes refer to each other by index (NodeId) instead of pointers,
//! which keeps Rust's borrow checker happy and is fast.

use std::collections::HashMap;

pub type NodeId = usize;

#[derive(Debug, Clone)]
pub enum NodeData {
    Document,
    Element { tag: String, attrs: HashMap<String, String> },
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub data: NodeData,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

#[derive(Debug, Default)]
pub struct Dom {
    pub nodes: Vec<Node>,
}

impl Dom {
    pub fn new() -> Self {
        Dom { nodes: vec![Node { data: NodeData::Document, parent: None, children: vec![] }] }
    }

    pub const ROOT: NodeId = 0;

    pub fn add(&mut self, data: NodeData, parent: NodeId) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { data, parent: Some(parent), children: vec![] });
        self.nodes[parent].children.push(id);
        id
    }

    /// Create a node that isn't in the tree yet (document.createElement).
    pub fn create(&mut self, data: NodeData) -> NodeId {
        self.nodes.push(Node { data, parent: None, children: vec![] });
        self.nodes.len() - 1
    }

    /// Remove a node from its parent (it stays in the arena, detached).
    pub fn detach(&mut self, id: NodeId) {
        if let Some(p) = self.nodes[id].parent.take() {
            self.nodes[p].children.retain(|&c| c != id);
        }
    }

    /// Insert `child` into `parent` before `before` (or at the end).
    pub fn insert(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        // Refuse to create cycles (inserting a node into its own subtree).
        let mut cur = Some(parent);
        while let Some(c) = cur {
            if c == child {
                return;
            }
            cur = self.nodes[c].parent;
        }
        self.detach(child);
        let kids = &mut self.nodes[parent].children;
        let pos = before.and_then(|b| kids.iter().position(|&k| k == b)).unwrap_or(kids.len());
        kids.insert(pos, child);
        self.nodes[child].parent = Some(parent);
    }

    pub fn remove_children(&mut self, id: NodeId) {
        for c in std::mem::take(&mut self.nodes[id].children) {
            self.nodes[c].parent = None;
        }
    }

    /// Copy the children of `other`'s node `from` into this DOM under `to`.
    pub fn graft(&mut self, other: &Dom, from: NodeId, to: NodeId) {
        for &c in &other.nodes[from].children {
            let id = self.create(other.nodes[c].data.clone());
            self.insert(to, id, None);
            self.graft(other, c, id);
        }
    }

    /// Serialize a node's children back to HTML (innerHTML).
    pub fn inner_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        for &c in &self.nodes[id].children {
            self.outer_html_into(c, &mut out);
        }
        out
    }

    pub fn outer_html_into(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].data {
            NodeData::Text(t) => {
                let raw = self.nodes[id].parent.and_then(|p| self.tag(p)).map_or(false, |t| t == "script" || t == "style");
                if raw {
                    out.push_str(t);
                } else {
                    out.push_str(&t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"));
                }
            }
            NodeData::Element { tag, attrs } => {
                out.push('<');
                out.push_str(tag);
                for (k, v) in attrs {
                    out.push_str(&format!(" {}=\"{}\"", k, v.replace('&', "&amp;").replace('"', "&quot;")));
                }
                out.push('>');
                const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];
                if !VOID.contains(&tag.as_str()) {
                    out.push_str(&self.inner_html(id));
                    out.push_str(&format!("</{tag}>"));
                }
            }
            NodeData::Document => out.push_str(&self.inner_html(id)),
        }
    }

    /// Descendants of `id` in document order (not including `id`).
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.nodes[id].children.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            out.push(n);
            stack.extend(self.nodes[n].children.iter().rev().copied());
        }
        out
    }

    pub fn tag(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element { tag, .. } => Some(tag.as_str()),
            _ => None,
        }
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element { attrs, .. } => attrs.get(name).map(|s| s.as_str()),
            _ => None,
        }
    }

    pub fn classes(&self, id: NodeId) -> impl Iterator<Item = &str> {
        self.attr(id, "class").unwrap_or("").split_whitespace()
    }

    /// All text inside a node, concatenated.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].data {
            NodeData::Text(t) => out.push_str(t),
            _ => {
                for &c in &self.nodes[id].children {
                    self.collect_text(c, out);
                }
            }
        }
    }

    /// Depth-first search for elements with a given tag.
    pub fn find_all(&self, tag: &str) -> Vec<NodeId> {
        self.descendants(Dom::ROOT).into_iter().filter(|&i| self.tag(i) == Some(tag)).collect()
    }
}
