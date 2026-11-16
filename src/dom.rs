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
        (0..self.nodes.len()).filter(|&i| self.tag(i) == Some(tag)).collect()
    }
}
