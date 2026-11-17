//! The Document Object Model: a tree of nodes stored in a flat arena.
//! Nodes refer to each other by index (NodeId) instead of pointers,
//! which keeps Rust's borrow checker happy and is fast.


pub type NodeId = usize;

/// The namespace of an element. Almost everything is HTML; `<svg>` and `<math>` and what's
/// inside them are SVG and MathML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ns {
    #[default]
    Html,
    Svg,
    MathMl,
}

/// The document's compatibility mode, decided by its doctype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quirks {
    #[default]
    NoQuirks,
    Limited,
    Quirks,
}

#[derive(Debug, Clone)]
pub enum NodeData {
    Document,
    /// Attributes as a short list: most elements have 0–3, where a hash table would waste memory.
    Element { tag: String, attrs: Vec<(String, String)> },
    Text(String),
    Comment(String),
    Doctype { name: String, public_id: Option<String>, system_id: Option<String> },
    ProcessingInstruction { target: String, data: String },
    /// A document fragment: what a `<template>` holds.
    Fragment,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub data: NodeData,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

#[derive(Debug, Default, Clone)]
pub struct Dom {
    pub nodes: Vec<Node>,
    /// Elements that aren't HTML (SVG and MathML); everything else is HTML.
    pub ns: std::collections::HashMap<NodeId, Ns>,
    /// Each `<template>`'s contents fragment.
    pub template_contents: std::collections::HashMap<NodeId, NodeId>,
    pub quirks: Quirks,
    /// Shadow DOM: each host's shadow root, and each root's host and whether it's open.
    pub shadow_roots: std::collections::HashMap<NodeId, NodeId>,
    pub shadow_hosts: std::collections::HashMap<NodeId, (NodeId, bool)>,
    /// In a composed tree (see `composed`): the shadow root each shadow-tree node belongs to.
    pub render_scope: std::collections::HashMap<NodeId, NodeId>,
    /// Form state kept outside attributes: custom validity messages, typed-in fields, picked files.
    pub form_state: crate::validation::FormState,
}

impl Dom {
    pub fn new() -> Self {
        Dom { nodes: vec![Node { data: NodeData::Document, parent: None, children: vec![] }], ..Default::default() }
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
            if let Some(&ns) = other.ns.get(&c) {
                self.ns.insert(id, ns);
            }
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
            NodeData::Comment(c) => {
                out.push_str("<!--");
                out.push_str(c);
                out.push_str("-->");
            }
            NodeData::Doctype { name, .. } => out.push_str(&format!("<!DOCTYPE {name}>")),
            NodeData::ProcessingInstruction { target, data } => out.push_str(&format!("<?{target} {data}>")),
            NodeData::Document | NodeData::Fragment => out.push_str(&self.inner_html(id)),
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

    /// Visit the descendants of `id` in document order without building a list; `f` returns
    /// false to stop early.
    pub fn walk(&self, id: NodeId, mut f: impl FnMut(NodeId) -> bool) {
        let mut stack: Vec<NodeId> = self.nodes[id].children.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if !f(n) {
                return;
            }
            stack.extend(self.nodes[n].children.iter().rev().copied());
        }
    }

    /// True if `id` is `ancestor` or inside it.
    pub fn is_inside(&self, mut id: NodeId, ancestor: NodeId) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.nodes[id].parent {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    /// element.attachShadow(): give `host` a shadow root (or return the one it has).
    pub fn attach_shadow(&mut self, host: NodeId, open: bool) -> NodeId {
        if let Some(&r) = self.shadow_roots.get(&host) {
            return r;
        }
        let root = self.create(NodeData::Fragment);
        self.shadow_roots.insert(host, root);
        self.shadow_hosts.insert(root, (host, open));
        root
    }

    /// The tree as it is drawn: each shadow host shows its shadow tree instead of its own
    /// children, and each `<slot>` shows the host children assigned to it (by their `slot`
    /// attribute), or its own content when none are. Node ids stay the same, so styles,
    /// layout boxes and clicks still line up with the real DOM. None if there's no shadow DOM.
    pub fn composed(&self) -> Option<Dom> {
        if self.shadow_roots.is_empty() {
            return None;
        }
        let mut c = self.clone();
        for (&host, &root) in &self.shadow_roots {
            let light = self.nodes[host].children.clone();
            let kids = self.compose_children(&mut c, root, root, &light);
            for &k in &kids {
                c.nodes[k].parent = Some(host);
            }
            c.nodes[host].children = kids;
            // `:host` rules find the host by this attribute.
            if let NodeData::Element { attrs, .. } = &mut c.nodes[host].data {
                attrs.push(("__shadowhost".into(), root.to_string()));
            }
        }
        Some(c)
    }

    fn compose_children(&self, c: &mut Dom, node: NodeId, root: NodeId, light: &[NodeId]) -> Vec<NodeId> {
        let mut out = Vec::new();
        for &k in &self.nodes[node].children {
            c.render_scope.insert(k, root);
            if self.ns_of(k) == Ns::Html && self.tag(k) == Some("slot") {
                let name = self.attr(k, "name").unwrap_or("");
                let assigned: Vec<NodeId> = light
                    .iter()
                    .copied()
                    .filter(|&l| match &self.nodes[l].data {
                        NodeData::Element { .. } => self.attr(l, "slot").unwrap_or("") == name,
                        NodeData::Text(_) => name.is_empty(),
                        _ => false,
                    })
                    .collect();
                // The slot stays in the flat tree (display: contents by default) with what's
                // assigned to it, or its fallback content, inside: its own styles apply to them
                // (a `display: none` slot hides them) and they inherit from it.
                let kids = if assigned.is_empty() { self.compose_children(c, k, root, light) } else { assigned };
                for &kk in &kids {
                    c.nodes[kk].parent = Some(k);
                }
                c.nodes[k].children = kids;
                out.push(k);
                continue;
            }
            let kids = self.compose_children(c, k, root, light);
            for &kk in &kids {
                c.nodes[kk].parent = Some(k);
            }
            c.nodes[k].children = kids;
            out.push(k);
        }
        out
    }

    pub fn ns_of(&self, id: NodeId) -> Ns {
        if self.ns.is_empty() {
            return Ns::Html;
        }
        self.ns.get(&id).copied().unwrap_or_default()
    }

    pub fn tag(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element { tag, .. } => Some(tag.as_str()),
            _ => None,
        }
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element { attrs, .. } => attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        if let NodeData::Element { attrs, .. } = &mut self.nodes[id].data {
            match attrs.iter_mut().find(|(k, _)| k == name) {
                Some((_, v)) => *v = value.to_string(),
                None => attrs.push((name.to_string(), value.to_string())),
            }
        }
    }

    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        if let NodeData::Element { attrs, .. } = &mut self.nodes[id].data {
            attrs.retain(|(k, _)| k != name);
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
