//! The box tree: what layout actually lays out, built from the styled DOM.
//!
//! The DOM and the boxes it generates aren't the same tree (CSS 2 §9.2, CSS Display 3):
//!
//! - `display: none` elements make no boxes (layout skips them);
//! - `display: contents` elements make no box of their own: their children (and their
//!   `::before`/`::after`) take their place in their parent;
//! - `::before` and `::after` make boxes that aren't DOM nodes (see `pseudo.rs`);
//! - list items get a marker: its text is worked out here (the `list-item` counter, `<ol
//!   start reversed>`, `<li value>`, `list-style-type`, `::marker { content }`), and an
//!   `inside` marker is an inline box at the start of the item;
//! - a block container with both block-level and inline-level children wraps each run of
//!   inline-level children in an anonymous block box, so layout only ever sees a block
//!   container holding either blocks or lines of inline content.
//!
//! The box tree reuses the DOM's types: every box is a node of a `Dom` with a style. Boxes
//! made from DOM nodes keep the node's id; boxes the DOM doesn't have (pseudo-elements,
//! markers, anonymous boxes) get ids after the DOM's, and `origin` maps any box back to
//! the DOM node it belongs to, so hits and element boxes are reported in DOM ids.

use crate::dom::{Dom, NodeData, NodeId};
use crate::style::{Display, Position, Style, TableRole};
use std::collections::HashMap;
use std::rc::Rc;

/// Tag of an anonymous block box.
pub const ANON_BLOCK: &str = "::anonymous-block";
/// Tag of an `inside` list marker box.
pub const MARKER: &str = "::marker";

/// What a box is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    /// Made by a DOM element or text node (same id as the node).
    Node,
    /// A `::before`/`::after` box or the text in it.
    Pseudo,
    /// An `inside` list marker or its text.
    Marker,
    /// An anonymous block box wrapping a run of inline-level boxes.
    AnonymousBlock,
}

pub struct BoxTree {
    /// The boxes, as a tree of nodes: ids below `real` are the DOM's own nodes.
    pub dom: Dom,
    /// One style per box. Anonymous boxes inherit from their parent.
    pub styles: Vec<Rc<Style>>,
    /// The DOM node each box belongs to.
    origin: Vec<NodeId>,
    #[cfg_attr(not(test), allow(dead_code))]
    kinds: Vec<BoxKind>,
    /// How many of the boxes are DOM nodes.
    real: usize,
    /// Text of each list item's marker (outside markers are drawn by layout beside the item).
    pub markers: HashMap<NodeId, String>,
}

impl BoxTree {
    /// The DOM node a box belongs to (its own id for boxes made by DOM nodes; the element
    /// for pseudo-elements, markers and anonymous boxes).
    pub fn origin(&self, id: NodeId) -> NodeId {
        self.origin.get(id).copied().unwrap_or(id)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn kind(&self, id: NodeId) -> BoxKind {
        self.kinds.get(id).copied().unwrap_or(BoxKind::Node)
    }

    /// Is this box one of the DOM's own nodes (and not generated)?
    pub fn is_dom_node(&self, id: NodeId) -> bool {
        id < self.real
    }
}

thread_local! {
    /// The last tree built and a hash of what it was built from (see `build_cached`).
    static LAST: std::cell::RefCell<Option<(u64, Rc<BoxTree>)>> = const { std::cell::RefCell::new(None) };
}

/// The box tree for a styled DOM, reusing the last one when the DOM and styles are the
/// same (a relayout for a scroll, a resize, a loaded image or a cached box changed nothing
/// the tree is made of). The last tree holds its styles, so no new style can take the
/// address of one the hash saw.
pub fn build_cached(dom: &Dom, styles: &[Rc<Style>]) -> Rc<BoxTree> {
    let key = inputs(dom, styles);
    if let Some(t) = LAST.with(|l| l.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, t)| t.clone())) {
        return t;
    }
    // Drop the old tree before building the new one, so both aren't held at once.
    LAST.with(|l| *l.borrow_mut() = None);
    let t = Rc::new(build(dom, styles));
    LAST.with(|l| *l.borrow_mut() = Some((key, t.clone())));
    t
}

/// A hash of everything `build` and layout read from the DOM and styles: each node's data,
/// children, namespace and picked files, and each style's identity.
fn inputs(dom: &Dom, styles: &[Rc<Style>]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (dom.nodes.len(), styles.len(), dom.quirks as u8).hash(&mut h);
    for (id, n) in dom.nodes.iter().enumerate() {
        match &n.data {
            NodeData::Element { tag, attrs } => {
                1u8.hash(&mut h);
                tag.hash(&mut h);
                attrs.hash(&mut h);
                (dom.ns_of(id) as u8).hash(&mut h);
                if let Some(nn) = dom.ns_names.get(&id) {
                    (&nn.uri, &nn.prefix).hash(&mut h);
                }
                if let Some(files) = dom.form_state.files.get(&id) {
                    files.iter().for_each(|f| f.name.hash(&mut h));
                }
            }
            NodeData::Text(t) => {
                2u8.hash(&mut h);
                t.hash(&mut h);
            }
            other => {
                3u8.hash(&mut h);
                format!("{other:?}").hash(&mut h);
            }
        }
        n.parent.hash(&mut h);
        n.children.hash(&mut h);
        styles.get(id).map(|s| Rc::as_ptr(s) as usize).hash(&mut h);
    }
    h.finish()
}

/// Build the box tree for a styled DOM.
pub fn build(dom: &Dom, styles: &[Rc<Style>]) -> BoxTree {
    let real = dom.nodes.len();
    let (mut tree, mut st) = match crate::pseudo::expand(dom, styles) {
        Some(x) => x,
        None => (dom.clone(), styles[..real.min(styles.len())].to_vec()),
    };
    // Every node needs a style (scripts can add nodes between styling and layout).
    while st.len() < tree.nodes.len() {
        st.push(Rc::new(Style { display: Display::None, ..Style::default() }));
    }
    // Generated ::before/::after boxes belong to the element they're in.
    let mut origin: Vec<NodeId> = (0..tree.nodes.len()).collect();
    let mut kinds = vec![BoxKind::Node; tree.nodes.len()];
    for id in real..tree.nodes.len() {
        let mut o = id;
        while o >= real {
            o = tree.nodes[o].parent.unwrap_or(0);
        }
        origin[id] = o;
        kinds[id] = BoxKind::Pseudo;
    }
    let mut b = Builder { dom: &mut tree, styles: &mut st, origin: &mut origin, kinds: &mut kinds };
    b.unwrap_contents();
    let markers = b.list_markers(real);
    b.blocks_in_inlines();
    b.anonymous_blocks();
    BoxTree { dom: tree, styles: st, origin, kinds, real, markers }
}

struct Builder<'a> {
    dom: &'a mut Dom,
    styles: &'a mut Vec<Rc<Style>>,
    origin: &'a mut Vec<NodeId>,
    kinds: &'a mut Vec<BoxKind>,
}

thread_local! {
    /// Block versions of inline styles (blocks_in_inlines), kept so each build gives the same
    /// style object: relayout.rs recognises unchanged boxes by their style's identity. The
    /// original is held too, so its address isn't reused while the entry exists.
    static AS_BLOCK: std::cell::RefCell<HashMap<usize, (Rc<Style>, Rc<Style>)>> = std::cell::RefCell::new(HashMap::new());
}

fn as_block(st: &Rc<Style>) -> Rc<Style> {
    AS_BLOCK.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 4096 {
            m.clear();
        }
        m.entry(Rc::as_ptr(st) as usize)
            .or_insert_with(|| (st.clone(), Rc::new(Style { display: Display::Block, ..(**st).clone() })))
            .1
            .clone()
    })
}

/// Elements whose content is replaced: `display: contents` on them means no box at all.
fn replaced(tag: &str) -> bool {
    matches!(tag, "img" | "input" | "textarea" | "select" | "iframe" | "video" | "audio" | "canvas" | "embed" | "object" | "br" | "hr" | "meter" | "progress" | "button" | "svg")
}

impl Builder<'_> {
    fn new_box(&mut self, data: NodeData, style: Rc<Style>, origin: NodeId, kind: BoxKind) -> NodeId {
        let id = self.dom.create(data);
        self.styles.push(style);
        self.origin.push(origin);
        self.kinds.push(kind);
        id
    }

    fn is_element(&self, id: NodeId) -> bool {
        matches!(self.dom.nodes[id].data, NodeData::Element { .. })
    }

    /// Tree order, parents before children (scripts can move nodes, so ids aren't in order).
    fn elements_in_order(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![Dom::ROOT];
        while let Some(id) = stack.pop() {
            out.push(id);
            stack.extend(self.dom.nodes[id].children.iter().rev().copied().filter(|&c| self.is_element(c)));
        }
        out
    }

    /// Replace each `display: contents` element with its children.
    fn unwrap_contents(&mut self) {
        if !self.styles.iter().any(|s| s.contents) {
            return;
        }
        let mut stack = vec![Dom::ROOT];
        while let Some(id) = stack.pop() {
            if self.styles[id].display == Display::None && id != Dom::ROOT {
                continue;
            }
            let old = std::mem::take(&mut self.dom.nodes[id].children);
            let mut kids = Vec::with_capacity(old.len());
            // Children of an unwrapped element are looked at in its place (they may be
            // `display: contents` too).
            let mut work: Vec<NodeId> = old.into_iter().rev().collect();
            while let Some(c) = work.pop() {
                let unwrap = match &self.dom.nodes[c].data {
                    NodeData::Element { .. } => self.styles[c].contents && self.styles[c].display != Display::None,
                    _ => false,
                };
                if !unwrap {
                    kids.push(c);
                    continue;
                }
                let tag = self.dom.tag(c).unwrap_or("").to_string();
                let inner = std::mem::take(&mut self.dom.nodes[c].children);
                self.dom.nodes[c].parent = None;
                if replaced(&tag) {
                    // `display: contents` on a replaced element or form control acts as `none`.
                    for k in inner {
                        self.dom.nodes[k].parent = None;
                    }
                    continue;
                }
                work.extend(inner.into_iter().rev());
            }
            for &k in &kids {
                self.dom.nodes[k].parent = Some(id);
            }
            stack.extend(kids.iter().rev().copied().filter(|&k| self.is_element(k)));
            self.dom.nodes[id].children = kids;
        }
    }

    /// Work out each list item's marker; put `inside` markers into the item as inline boxes.
    fn list_markers(&mut self, real: usize) -> HashMap<NodeId, String> {
        let mut markers = HashMap::new();
        if !self.styles.iter().any(|s| s.display == Display::ListItem) {
            return markers;
        }
        let is_list = |b: &Self, id: NodeId| id < real && matches!(b.dom.tag(id), Some("ol" | "ul" | "menu"));
        let roots: Vec<NodeId> = self.elements_in_order().into_iter().filter(|&id| id == Dom::ROOT || is_list(self, id)).collect();
        for parent in roots {
            // Everything in the list in tree order, but not inside nested lists (they have
            // their own list-item counter): counters follow tree order, so a <div> inside
            // an item can change the next item's number.
            let mut kids = Vec::new();
            let mut stack: Vec<NodeId> = self.dom.nodes[parent].children.iter().rev().copied().collect();
            while let Some(c) = stack.pop() {
                if !self.is_element(c) || self.styles[c].display == Display::None {
                    continue;
                }
                kids.push(c);
                if !is_list(self, c) {
                    stack.extend(self.dom.nodes[c].children.iter().rev().copied());
                }
            }
            if !kids.iter().any(|&c| self.styles[c].display == Display::ListItem) {
                continue;
            }
            // The list-item counter (CSS Lists 3 §4.4): <ol start> and <ol reversed> set where
            // it starts and which way it goes; each item adds 1 (or −1), `counter-increment:
            // list-item` changes that, and <li value> / `counter-set` set it after the add.
            let list = (parent != Dom::ROOT).then_some(parent);
            let reversed = list.is_some_and(|p| self.dom.tag(p) == Some("ol") && self.dom.attr(p, "reversed").is_some());
            let start = list.filter(|&p| self.dom.tag(p) == Some("ol")).and_then(|p| self.dom.attr(p, "start")).and_then(|v| v.trim().parse::<i32>().ok());
            let step = if reversed { -1 } else { 1 };
            let changes: Vec<(i32, Option<i32>)> = kids.iter().map(|&c| self.list_item_change(c, step)).collect();
            let mut n = match start {
                Some(s) => s - step,
                None if reversed => {
                    // Counts down to 1 at the end, or to the first value set part-way.
                    match changes.iter().position(|c| c.1.is_some()) {
                        // (Matches Gecko and the WPT li-value-reversed tests: an item's
                        // `value` counts its own step; a set on any other element is one more.)
                        Some(i) if self.dom.attr(kids[i], "value").is_some() && self.dom.tag(kids[i]) == Some("li") => {
                            changes[i].1.unwrap_or(0) - changes[..=i].iter().map(|c| c.0).sum::<i32>()
                        }
                        Some(i) => changes[i].1.unwrap_or(0) + 1 - changes[..i].iter().map(|c| c.0).sum::<i32>(),
                        None => 1 - changes.iter().map(|c| c.0).sum::<i32>(),
                    }
                }
                None => 0,
            };
            for (&li, &(inc, set)) in kids.iter().zip(&changes) {
                n = set.unwrap_or(n.saturating_add(inc));
                if self.styles[li].display != Display::ListItem {
                    continue;
                }
                let st = self.styles[li].clone();
                let marker_style = st.pseudo.as_ref().and_then(|p| p.marker.clone());
                let custom = marker_style
                    .as_ref()
                    .and_then(|m| m.content.as_deref())
                    .and_then(|c| crate::pseudo::content_text(c, self.dom, li, &Default::default(), &mut 0));
                let text = match custom {
                    Some(t) => t,
                    None if st.list_style == crate::style::ListStyle::None || st.list_type == "none" => continue,
                    None => marker_text(n, st.list_type, st.list_style),
                };
                if text.is_empty() {
                    continue;
                }
                if st.list_inside {
                    self.inside_marker(li, &st, marker_style, &text);
                } else {
                    markers.insert(li, text);
                }
            }
        }
        markers
    }

    /// How an element changes its list's `list-item` counter: (increment, value set after it).
    fn list_item_change(&self, id: NodeId, step: i32) -> (i32, Option<i32>) {
        let st = &self.styles[id];
        let item = st.display == Display::ListItem;
        let mut inc = None;
        let mut set = None;
        for (kind, v) in st.counters.as_deref().unwrap_or(&[]) {
            let words: Vec<&str> = v.split_whitespace().collect();
            let mut i = 0;
            while i < words.len() {
                let num = words.get(i + 1).and_then(|n| n.parse::<i32>().ok());
                if words[i] == "list-item" {
                    match kind {
                        1 => set = Some(num.unwrap_or(0)),
                        // `list-item 1 list-item -1` adds both.
                        2 => inc = Some(inc.unwrap_or(0) + num.unwrap_or(1)),
                        _ => {}
                    }
                }
                i += if num.is_some() { 2 } else { 1 };
            }
        }
        if item && self.dom.tag(id) == Some("li") {
            if let Some(v) = self.dom.attr(id, "value").and_then(|v| v.trim().parse::<i32>().ok()) {
                set = Some(v);
            }
        }
        (inc.unwrap_or(if item { step } else { 0 }), set)
    }

    fn inside_marker(&mut self, li: NodeId, st: &Rc<Style>, marker_style: Option<Rc<Style>>, text: &str) {
        let mut ms = match &marker_style {
            Some(m) => (**m).clone(),
            None => Style::inherit(st),
        };
        ms.display = Display::Inline;
        let ms = Rc::new(ms);
        let origin = self.origin[li];
        let el = self.new_box(NodeData::Element { tag: MARKER.to_string(), attrs: Vec::new() }, ms.clone(), origin, BoxKind::Marker);
        let t = self.new_box(NodeData::Text(format!("{text} ")), ms, origin, BoxKind::Marker);
        let first = self.dom.nodes[li].children.first().copied();
        self.dom.insert(li, el, first);
        self.dom.insert(el, t, None);
    }

    /// An inline element holding a block (`<a><div>card</div></a>`, a custom element wrapping
    /// a section) is laid out as a block box, innermost first, so the block inside keeps its own
    /// width, padding and background. CSS 2 §9.2.1.1 splits the inline around the block
    /// instead; for an inline whose only content is blocks (the usual case) the result is the
    /// same, and inline text beside the block goes on lines of its own either way.
    fn blocks_in_inlines(&mut self) {
        for id in self.elements_in_order().into_iter().rev() {
            if self.styles[id].display != Display::Inline || replaced(self.dom.tag(id).unwrap_or("")) || self.dom.nodes[id].parent.is_none() {
                continue;
            }
            let holds_block = self.dom.nodes[id].children.iter().any(|&c| self.styles[c].display != Display::None && self.block_level(c) && !self.out_of_flow(c) && !self.float(c));
            if holds_block {
                self.styles[id] = as_block(&self.styles[id]);
            }
        }
    }

    fn block_level(&self, id: NodeId) -> bool {
        self.is_element(id) && matches!(self.styles[id].display, Display::Block | Display::ListItem)
    }

    fn out_of_flow(&self, id: NodeId) -> bool {
        self.is_element(id) && matches!(self.styles[id].position, Position::Absolute | Position::Fixed)
    }

    fn float(&self, id: NodeId) -> bool {
        self.is_element(id) && self.styles[id].float.is_some() && !self.out_of_flow(id)
    }

    /// Does this box lay its children out as blocks and lines (rather than as flex or grid
    /// items, table parts, or a replaced element's content)?
    fn block_container(&self, id: NodeId) -> bool {
        let st = &self.styles[id];
        if !self.is_element(id) || st.display == Display::None || st.display == Display::Inline {
            return false;
        }
        if st.flex.container || st.grid.is_some() || !matches!(st.table_role, TableRole::None | TableRole::Cell | TableRole::Caption) {
            return false;
        }
        !replaced(self.dom.tag(id).unwrap_or(""))
    }

    /// Text that collapses away entirely between blocks.
    fn collapsible_space(&self, id: NodeId) -> bool {
        match &self.dom.nodes[id].data {
            NodeData::Text(t) => !self.styles[id].pre && t.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')),
            NodeData::Element { .. } => self.styles[id].display == Display::None,
            _ => true,
        }
    }

    /// Wrap runs of inline-level children of block containers that also hold blocks in
    /// anonymous block boxes. Floats end a run (layout places them between lines, as before).
    fn anonymous_blocks(&mut self) {
        for id in self.elements_in_order() {
            if !self.block_container(id) {
                continue;
            }
            let kids = self.dom.nodes[id].children.clone();
            if !kids.iter().any(|&c| self.styles[c].display != Display::None && self.block_level(c) && !self.out_of_flow(c) && !self.float(c)) {
                continue;
            }
            let mut out: Vec<NodeId> = Vec::with_capacity(kids.len());
            let mut run: Vec<NodeId> = Vec::new();
            // Absolutely positioned boxes go in the run only if inline content follows them
            // (their static position is then inside the run's lines).
            let mut held: Vec<NodeId> = Vec::new();
            for c in kids {
                let breaks = self.styles[c].display != Display::None && !self.out_of_flow(c) && (self.block_level(c) || self.float(c));
                if breaks {
                    self.close_run(id, &mut run, &mut out);
                    out.append(&mut held);
                    out.push(c);
                } else if self.out_of_flow(c) || self.styles[c].display == Display::None {
                    held.push(c);
                } else {
                    run.append(&mut held);
                    run.push(c);
                }
            }
            self.close_run(id, &mut run, &mut out);
            out.append(&mut held);
            for &k in &out {
                self.dom.nodes[k].parent = Some(id);
            }
            self.dom.nodes[id].children = out;
        }
    }

    fn close_run(&mut self, parent: NodeId, run: &mut Vec<NodeId>, out: &mut Vec<NodeId>) {
        if run.is_empty() {
            return;
        }
        if run.iter().all(|&c| self.collapsible_space(c) || self.out_of_flow(c)) {
            // Whitespace between blocks makes no box.
            for c in run.drain(..) {
                if self.out_of_flow(c) || self.is_element(c) {
                    out.push(c);
                } else {
                    self.dom.nodes[c].parent = None;
                }
            }
            return;
        }
        let mut st = Style::inherit(&self.styles[parent]);
        st.display = Display::Block;
        let origin = self.origin[parent];
        let anon = self.new_box(NodeData::Element { tag: ANON_BLOCK.to_string(), attrs: Vec::new() }, Rc::new(st), origin, BoxKind::AnonymousBlock);
        for &c in run.iter() {
            self.dom.nodes[c].parent = Some(anon);
        }
        self.dom.nodes[anon].children = std::mem::take(run);
        self.dom.nodes[anon].parent = Some(parent);
        out.push(anon);
    }
}

/// A marker's text for counter value `n`.
fn marker_text(n: i32, list_type: &str, fallback: crate::style::ListStyle) -> String {
    let ty = match (list_type, fallback) {
        // `list-style-type` set to something this engine didn't recognise keeps the
        // shape it had before (a number or a bullet).
        ("disc", crate::style::ListStyle::Decimal) => "decimal",
        (t, _) => t,
    };
    match ty {
        "disc" | "circle" | "square" => crate::pseudo::format_counter(n, ty),
        _ => format!("{}.", crate::pseudo::format_counter(n, ty)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(html: &str) -> (Dom, BoxTree) {
        let dom = crate::html::parse(html);
        let sheets: Vec<crate::css::Stylesheet> = dom.find_all("style").iter().map(|&s| crate::css::parse(&dom.text_content(s))).collect();
        let styles = crate::style::compute(&dom, &sheets);
        let t = build(&dom, &styles);
        (dom, t)
    }

    /// An unchanged DOM with the same styles gets the same tree back; any edit, or a new
    /// style, builds a new one.
    #[test]
    fn unchanged_inputs_reuse_the_tree() {
        let mut dom = crate::html::parse("<ul><li>a</li><li>b <b>c</b></li></ul><p>text</p>");
        let styles = crate::style::compute(&dom, &[]);
        let a = build_cached(&dom, &styles);
        let b = build_cached(&dom, &styles);
        assert!(Rc::ptr_eq(&a, &b));
        let p = dom.find_all("p")[0];
        dom.set_attr(p, "class", "x");
        let c = build_cached(&dom, &styles);
        assert!(!Rc::ptr_eq(&b, &c));
        let restyled = crate::style::compute(&dom, &[]);
        let d = build_cached(&dom, &restyled);
        assert!(!Rc::ptr_eq(&c, &d));
        let t = dom.nodes[p].children[0];
        dom.nodes[t].data = NodeData::Text("other".into());
        let e = build_cached(&dom, &restyled);
        assert!(!Rc::ptr_eq(&d, &e));
        assert_eq!(kids(&e, p), ["#other"]);
    }

    fn kids(t: &BoxTree, id: NodeId) -> Vec<String> {
        t.dom.nodes[id]
            .children
            .iter()
            .map(|&c| match &t.dom.nodes[c].data {
                NodeData::Element { tag, .. } => tag.clone(),
                NodeData::Text(s) => format!("#{}", s.trim()),
                _ => "?".into(),
            })
            .collect()
    }

    #[test]
    fn inline_runs_beside_blocks_get_anonymous_boxes() {
        let (dom, t) = tree("<body><div id=a>one <b>two</b> <p>three</p> four</div></body>");
        let a = dom.find_all("div")[0];
        assert_eq!(kids(&t, a), vec![ANON_BLOCK, "p", ANON_BLOCK]);
        let anon = t.dom.nodes[a].children[0];
        assert_eq!(t.kind(anon), BoxKind::AnonymousBlock);
        assert_eq!(t.origin(anon), a);
        assert_eq!(kids(&t, anon), vec!["#one", "b", "#"]);
        assert_eq!(t.styles[anon].display, Display::Block);
    }

    #[test]
    fn inlines_holding_blocks_become_blocks() {
        let (_, t) = tree("<body><span id=s><b id=b><div>card</div></b></span><span id=t>text</span></body>");
        let id = |name: &str| (0..t.dom.nodes.len()).find(|&n| t.dom.attr(n, "id") == Some(name)).unwrap();
        assert_eq!(t.styles[id("b")].display, Display::Block);
        assert_eq!(t.styles[id("s")].display, Display::Block, "promoted from the inside out");
        assert_eq!(t.styles[id("t")].display, Display::Inline, "an inline holding only text stays inline");
    }

    #[test]
    fn whitespace_between_blocks_makes_no_box() {
        let (dom, t) = tree("<body><div>\n  <p>a</p>\n  <p>b</p>\n</div></body>");
        let d = dom.find_all("div")[0];
        assert_eq!(kids(&t, d), vec!["p", "p"]);
    }

    #[test]
    fn inline_only_blocks_are_left_alone() {
        let (dom, t) = tree("<body><p>a <i>b</i> c</p></body>");
        let p = dom.find_all("p")[0];
        assert_eq!(kids(&t, p), vec!["#a", "i", "#c"]);
    }

    #[test]
    fn display_contents_children_take_its_place() {
        let (dom, t) = tree("<style>.c{display:contents;border:5px solid red}</style><body><div><span class=c><p>x</p><em>y</em></span></div></body>");
        let d = dom.find_all("div")[0];
        assert_eq!(kids(&t, d), vec!["p", ANON_BLOCK]);
        let span = dom.find_all("span")[0];
        assert_eq!(t.dom.nodes[span].parent, None);
    }

    #[test]
    fn nested_contents_and_pseudo_elements() {
        let (dom, t) = tree("<style>.c{display:contents} .c::before{content:'B'}</style><body><p><span class=c><span class=c>a</span>b</span></p></body>");
        let p = dom.find_all("p")[0];
        // Both spans have a ::before; the outer one's comes first.
        assert_eq!(kids(&t, p), vec![crate::pseudo::BEFORE, crate::pseudo::BEFORE, "#a", "#b"]);
        let before = t.dom.nodes[p].children[0];
        assert_eq!(t.kind(before), BoxKind::Pseudo);
        assert_eq!(t.origin(before), dom.find_all("span")[0]);
    }

    #[test]
    fn list_markers_follow_the_list_item_counter() {
        let (dom, t) = tree("<body><ol start=3><li>a<li>b<li value=10>c<li>d</ol><ol reversed><li>x<li>y</ol></body>");
        let li = dom.find_all("li");
        let m: Vec<&str> = li.iter().map(|l| t.markers.get(l).map(String::as_str).unwrap_or("")).collect();
        assert_eq!(m, vec!["3.", "4.", "10.", "11.", "2.", "1."]);
        // Reversed lists count down to a value set part-way (WPT li-value-reversed-*).
        let (dom, t) = tree("<body><ol reversed><li>a<li>b<li value=7>c<li>d<li value=10>e<li>f</ol></body>");
        let m: Vec<String> = dom.find_all("li").iter().map(|l| t.markers[l].clone()).collect();
        assert_eq!(m, vec!["9.", "8.", "7.", "6.", "10.", "9."]);
    }

    #[test]
    fn marker_types_and_inside_markers() {
        let (dom, t) = tree("<style>ol{list-style:upper-roman inside}</style><body><ol><li>a</li><li>b</li></ol><ul><li>c</ul></body>");
        let li = dom.find_all("li");
        assert!(t.markers.get(&li[0]).is_none());
        assert_eq!(kids(&t, li[1]), vec![MARKER, "#b"]);
        let m = t.dom.nodes[li[1]].children[0];
        assert_eq!(kids(&t, m), vec!["#II."]);
        assert_eq!(t.origin(m), li[1]);
        assert_eq!(t.markers.get(&li[2]).map(String::as_str), Some("\u{2022}"));
    }

    #[test]
    fn hidden_and_positioned_children_dont_split_runs() {
        let (dom, t) = tree("<style>.abs{position:absolute}</style><body><div>a<span class=abs>z</span>b<p>c</p><i class=abs>q</i></div></body>");
        let d = dom.find_all("div")[0];
        assert_eq!(kids(&t, d), vec![ANON_BLOCK, "p", "i"]);
        let anon = t.dom.nodes[d].children[0];
        assert_eq!(kids(&t, anon), vec!["#a", "span", "#b"]);
    }
}


