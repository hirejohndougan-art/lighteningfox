//! The natively implemented DOM members: plain Rust over the page's tree. The generated glue
//! (`generated.rs`) converts arguments and results; these never see a JavaScript value.
//!
//! `this` is `None` for a node object whose id is past the end of the tree (it answers like an
//! empty node, as the JavaScript versions did). Node results are `Option<NodeId>`.
//!
//! Members that change the tree or attributes stay in JavaScript on purpose (see
//! dom_classes.rs): custom elements and mutation observers hook the mutation natives.

use super::conv::DomError;
use crate::dom::{Dom, NodeData, NodeId, Ns, Quirks};
use crate::dom_classes::node_type as type_of;
use crate::js::{element_by_id, parsed_selectors, query, with_dom};

// ------------------------------------------------------------ helpers

fn is_element(d: &Dom, id: NodeId) -> bool {
    type_of(d, id) == 1
}

fn parent(id: Option<NodeId>) -> Option<NodeId> {
    id.and_then(|id| with_dom(|d| d.nodes[id].parent))
}

/// The sibling `step` places away (±1), optionally skipping non-elements.
fn sibling(id: Option<NodeId>, step: isize, elements: bool) -> Option<NodeId> {
    let id = id?;
    with_dom(|d| {
        let p = d.nodes[id].parent?;
        let kids = &d.nodes[p].children;
        let mut i = kids.iter().position(|&k| k == id)? as isize;
        loop {
            i += step;
            if i < 0 || i as usize >= kids.len() {
                return None;
            }
            let k = kids[i as usize];
            if !elements || is_element(d, k) {
                return Some(k);
            }
        }
    })
}

fn children(id: Option<NodeId>, elements: bool) -> Vec<NodeId> {
    let Some(id) = id else { return vec![] };
    with_dom(|d| d.nodes[id].children.iter().copied().filter(|&k| !elements || is_element(d, k)).collect())
}

fn attr(id: Option<NodeId>, name: &str) -> Option<String> {
    id.and_then(|id| with_dom(|d| d.attr(id, name).map(String::from)))
}

fn tag(id: Option<NodeId>) -> String {
    id.and_then(|id| with_dom(|d| d.tag(id).map(String::from))).unwrap_or_default()
}

fn selector_matches(id: NodeId, sel: &str) -> bool {
    let sels = parsed_selectors(sel);
    with_dom(|d| sels.iter().any(|s| crate::style::matches(d, id, s)))
}

/// getElementsByClassName's selector for a class list ("a b" → ".a.b").
fn class_selector(c: &str) -> String {
    c.split_whitespace().map(|x| format!(".{x}")).collect::<Vec<_>>().join("")
}

/// The node and its ancestors, the node first.
fn ancestry(d: &Dom, mut id: NodeId) -> Vec<NodeId> {
    let mut out = vec![id];
    while let Some(p) = d.nodes[id].parent {
        out.push(p);
        id = p;
    }
    out
}

// ------------------------------------------------------------ Node

pub fn node_type(this: Option<NodeId>) -> u16 {
    this.map_or(0, |id| with_dom(|d| type_of(d, id)) as u16)
}

pub fn parent_node(this: Option<NodeId>) -> Option<NodeId> {
    parent(this)
}

pub fn parent_element(this: Option<NodeId>) -> Option<NodeId> {
    parent(this).filter(|&p| with_dom(|d| is_element(d, p)))
}

pub fn child_nodes(this: Option<NodeId>) -> Vec<NodeId> {
    children(this, false)
}

pub fn first_child(this: Option<NodeId>) -> Option<NodeId> {
    this.and_then(|id| with_dom(|d| d.nodes[id].children.first().copied()))
}

pub fn last_child(this: Option<NodeId>) -> Option<NodeId> {
    this.and_then(|id| with_dom(|d| d.nodes[id].children.last().copied()))
}

pub fn next_sibling(this: Option<NodeId>) -> Option<NodeId> {
    sibling(this, 1, false)
}

pub fn previous_sibling(this: Option<NodeId>) -> Option<NodeId> {
    sibling(this, -1, false)
}

pub fn has_child_nodes(this: Option<NodeId>) -> bool {
    this.is_some_and(|id| with_dom(|d| !d.nodes[id].children.is_empty()))
}

pub fn is_connected(this: Option<NodeId>) -> bool {
    this.is_some_and(|id| with_dom(|d| *ancestry(d, id).last().unwrap() == Dom::ROOT))
}

/// textContent: null for the document and doctypes, as the spec says.
pub fn text_content(this: Option<NodeId>) -> Option<String> {
    let Some(id) = this else { return Some(String::new()) };
    with_dom(|d| match &d.nodes[id].data {
        NodeData::Document | NodeData::Doctype { .. } => None,
        NodeData::Comment(s) | NodeData::ProcessingInstruction { data: s, .. } => Some(s.clone()),
        _ => Some(d.text_content(id)),
    })
}

pub fn contains(this: Option<NodeId>, other: Option<NodeId>) -> bool {
    let (Some(id), Some(other)) = (this, other) else { return false };
    with_dom(|d| ancestry(d, other).contains(&id))
}

pub fn is_same_node(this: Option<NodeId>, other: Option<NodeId>) -> bool {
    this.is_some() && this == other
}

const DISCONNECTED: u16 = 0x01;
const PRECEDING: u16 = 0x02;
const FOLLOWING: u16 = 0x04;
const CONTAINS: u16 = 0x08;
const CONTAINED_BY: u16 = 0x10;
const IMPLEMENTATION_SPECIFIC: u16 = 0x20;

/// Where `other` is relative to this node (the DOCUMENT_POSITION_* bits).
pub fn compare_document_position(this: Option<NodeId>, other: Option<NodeId>) -> u16 {
    let (Some(a), Some(b)) = (this, other) else {
        return DISCONNECTED | IMPLEMENTATION_SPECIFIC | FOLLOWING;
    };
    if a == b {
        return 0;
    }
    with_dom(|d| {
        let (pa, pb) = (ancestry(d, a), ancestry(d, b));
        if pa.last() != pb.last() {
            // Different trees: any consistent order will do.
            return DISCONNECTED | IMPLEMENTATION_SPECIFIC | if b < a { PRECEDING } else { FOLLOWING };
        }
        if pa.contains(&b) {
            return CONTAINS | PRECEDING;
        }
        if pb.contains(&a) {
            return CONTAINED_BY | FOLLOWING;
        }
        // Walk down from the root to where the two paths part, then compare those children.
        let (ra, rb): (Vec<_>, Vec<_>) = (pa.iter().rev().copied().collect(), pb.iter().rev().copied().collect());
        let mut i = 0;
        while ra[i + 1] == rb[i + 1] {
            i += 1;
        }
        let kids = &d.nodes[ra[i]].children;
        let pos = |n| kids.iter().position(|&k| k == n).unwrap_or(0);
        if pos(rb[i + 1]) < pos(ra[i + 1]) { PRECEDING } else { FOLLOWING }
    })
}

fn equal(d: &Dom, a: NodeId, b: NodeId) -> bool {
    let same = match (&d.nodes[a].data, &d.nodes[b].data) {
        (NodeData::Document, NodeData::Document) | (NodeData::Fragment, NodeData::Fragment) => true,
        (NodeData::Element { tag: ta, attrs: aa }, NodeData::Element { tag: tb, attrs: ab }) => {
            ta == tb
                && d.ns.get(&a) == d.ns.get(&b)
                && d.ns_names.get(&a) == d.ns_names.get(&b)
                && aa.len() == ab.len()
                && aa.iter().all(|(k, v)| ab.iter().any(|(k2, v2)| k == k2 && v == v2))
        }
        (NodeData::Text(x), NodeData::Text(y)) | (NodeData::Comment(x), NodeData::Comment(y)) => x == y,
        (NodeData::Doctype { name: n1, public_id: p1, system_id: s1 }, NodeData::Doctype { name: n2, public_id: p2, system_id: s2 }) => {
            n1 == n2 && p1.as_deref().unwrap_or("") == p2.as_deref().unwrap_or("") && s1.as_deref().unwrap_or("") == s2.as_deref().unwrap_or("")
        }
        (NodeData::ProcessingInstruction { target: t1, data: d1 }, NodeData::ProcessingInstruction { target: t2, data: d2 }) => t1 == t2 && d1 == d2,
        _ => false,
    };
    let (ka, kb) = (&d.nodes[a].children, &d.nodes[b].children);
    same && ka.len() == kb.len() && ka.iter().zip(kb).all(|(&x, &y)| equal(d, x, y))
}

/// isEqualNode: same kind, name, attributes (in any order), data and children.
pub fn is_equal_node(this: Option<NodeId>, other: Option<NodeId>) -> bool {
    let (Some(a), Some(b)) = (this, other) else { return false };
    a == b || with_dom(|d| equal(d, a, b))
}

// ------------------------------------------------------------ Element

/// DOM: the qualified name, uppercased only for HTML elements (SVG's `linearGradient` and
/// MathML names keep their case).
/// The qualified name (`prefix:local`), upper-cased for HTML elements (in an HTML document).
pub fn tag_name(this: Option<NodeId>) -> String {
    let (html, prefix) = this.map_or((true, None), |id| with_dom(|d| (d.ns_of(id) == Ns::Html, d.prefix(id).map(str::to_string))));
    let name = match prefix {
        Some(p) => format!("{p}:{}", tag(this)),
        None => tag(this),
    };
    if html { name.to_ascii_uppercase() } else { name }
}

pub fn local_name(this: Option<NodeId>) -> String {
    tag(this)
}

pub fn id(this: Option<NodeId>) -> String {
    attr(this, "id").unwrap_or_default()
}

pub fn class_name(this: Option<NodeId>) -> String {
    attr(this, "class").unwrap_or_default()
}

fn attr_names(this: Option<NodeId>) -> Vec<String> {
    this.map(|id| {
        with_dom(|d| match &d.nodes[id].data {
            NodeData::Element { attrs, .. } => attrs.iter().map(|(k, _)| k.clone()).collect(),
            _ => vec![],
        })
    })
    .unwrap_or_default()
}

pub fn has_attributes(this: Option<NodeId>) -> bool {
    !attr_names(this).is_empty()
}

pub fn attribute_names(this: Option<NodeId>) -> Vec<String> {
    attr_names(this)
}

pub fn get_attribute(this: Option<NodeId>, name: String) -> Option<String> {
    attr(this, &name.to_lowercase())
}

pub fn has_attribute(this: Option<NodeId>, name: String) -> bool {
    attr(this, &name.to_lowercase()).is_some()
}

pub fn element_children(this: Option<NodeId>) -> Vec<NodeId> {
    children(this, true)
}

pub fn child_element_count(this: Option<NodeId>) -> u32 {
    children(this, true).len() as u32
}

pub fn first_element_child(this: Option<NodeId>) -> Option<NodeId> {
    children(this, true).first().copied()
}

pub fn last_element_child(this: Option<NodeId>) -> Option<NodeId> {
    children(this, true).last().copied()
}

pub fn next_element_sibling(this: Option<NodeId>) -> Option<NodeId> {
    sibling(this, 1, true)
}

pub fn previous_element_sibling(this: Option<NodeId>) -> Option<NodeId> {
    sibling(this, -1, true)
}

pub fn query_selector(this: Option<NodeId>, selectors: String) -> Option<NodeId> {
    this.and_then(|id| query(id, &selectors, false).first().copied())
}

pub fn query_selector_all(this: Option<NodeId>, selectors: String) -> Vec<NodeId> {
    this.map(|id| query(id, &selectors, true)).unwrap_or_default()
}

pub fn matches(this: Option<NodeId>, selectors: String) -> bool {
    this.is_some_and(|id| selector_matches(id, &selectors))
}

pub fn closest(this: Option<NodeId>, selectors: String) -> Option<NodeId> {
    let mut n = this;
    while let Some(k) = n {
        if !with_dom(|d| is_element(d, k)) {
            break;
        }
        if selector_matches(k, &selectors) {
            return Some(k);
        }
        n = parent(Some(k));
    }
    None
}

/// getElementsByTagName: descendants whose qualified name is the argument (ASCII-lowercased
/// for HTML elements), or all of them for "*".
pub fn elements_by_tag_name(this: Option<NodeId>, qualified_name: String) -> Vec<NodeId> {
    let Some(root) = this else { return vec![] };
    let lower = qualified_name.to_ascii_lowercase();
    with_dom(|d| {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = d.nodes[root].children.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if let NodeData::Element { tag, .. } = &d.nodes[n].data {
                let hit = qualified_name == "*" || {
                    let want = if d.ns_of(n) == Ns::Html { &lower } else { &qualified_name };
                    match d.prefix(n) {
                        Some(p) => want.len() == p.len() + 1 + tag.len() && want.starts_with(p) && want[p.len()..].starts_with(':') && want.ends_with(tag.as_str()),
                        None => want == tag,
                    }
                };
                if hit {
                    out.push(n);
                }
            }
            stack.extend(d.nodes[n].children.iter().rev());
        }
        out
    })
}

pub fn elements_by_class_name(this: Option<NodeId>, class_names: String) -> Vec<NodeId> {
    let sel = class_selector(&class_names);
    if sel.is_empty() {
        return vec![];
    }
    this.map(|id| query(id, &sel, true)).unwrap_or_default()
}

pub fn namespace_uri(this: Option<NodeId>) -> Option<String> {
    let id = this?;
    with_dom(|d| d.namespace_uri(id))
}

pub fn prefix(this: Option<NodeId>) -> Option<String> {
    let id = this?;
    with_dom(|d| d.prefix(id).map(str::to_string))
}

// ------------------------------------------------------------ CharacterData

pub fn character_data(this: Option<NodeId>) -> String {
    let Some(id) = this else { return String::new() };
    with_dom(|d| match &d.nodes[id].data {
        NodeData::Text(s) | NodeData::Comment(s) | NodeData::ProcessingInstruction { data: s, .. } => s.clone(),
        _ => d.text_content(id),
    })
}

/// length counts UTF-16 code units, like JavaScript strings.
pub fn character_length(this: Option<NodeId>) -> u32 {
    character_data(this).encode_utf16().count() as u32
}

/// substringData: `count` UTF-16 units from `offset` (IndexSizeError past the end).
pub fn substring_data(this: Option<NodeId>, offset: u32, count: u32) -> Result<String, DomError> {
    let units: Vec<u16> = character_data(this).encode_utf16().collect();
    let (offset, count) = (offset as usize, count as usize);
    if offset > units.len() {
        return Err(DomError::new("IndexSizeError", "The offset is greater than the data's length."));
    }
    let end = offset.saturating_add(count).min(units.len());
    Ok(String::from_utf16_lossy(&units[offset..end]))
}

/// wholeText: the text of this node and the Text nodes right next to it.
pub fn whole_text(this: Option<NodeId>) -> String {
    let Some(id) = this else { return String::new() };
    with_dom(|d| {
        let text = |k: NodeId| match &d.nodes[k].data {
            NodeData::Text(s) => Some(s.as_str()),
            _ => None,
        };
        let Some(p) = d.nodes[id].parent else { return text(id).unwrap_or_default().to_string() };
        let kids = &d.nodes[p].children;
        let at = kids.iter().position(|&k| k == id).unwrap_or(0);
        let mut start = at;
        while start > 0 && text(kids[start - 1]).is_some() {
            start -= 1;
        }
        kids[start..].iter().map_while(|&k| text(k)).collect()
    })
}

// ------------------------------------------------------------ Document
// The page's document searches the whole page tree (root 0), whatever its `this` id.

pub fn document_element(_: Option<NodeId>) -> Option<NodeId> {
    query(Dom::ROOT, "html", false).first().copied()
}

pub fn document_head(_: Option<NodeId>) -> Option<NodeId> {
    query(Dom::ROOT, "head", false).first().copied()
}

pub fn document_body(_: Option<NodeId>) -> Option<NodeId> {
    query(Dom::ROOT, "body", false).first().copied()
}

pub fn document_element_by_id(_: Option<NodeId>, element_id: String) -> Option<NodeId> {
    element_by_id(&element_id)
}

pub fn document_query_selector(_: Option<NodeId>, selectors: String) -> Option<NodeId> {
    query(Dom::ROOT, &selectors, false).first().copied()
}

pub fn document_query_selector_all(_: Option<NodeId>, selectors: String) -> Vec<NodeId> {
    query(Dom::ROOT, &selectors, true)
}

pub fn document_children(_: Option<NodeId>) -> Vec<NodeId> {
    children(Some(Dom::ROOT), true)
}

pub fn document_child_element_count(_: Option<NodeId>) -> u32 {
    children(Some(Dom::ROOT), true).len() as u32
}

pub fn document_first_element_child(_: Option<NodeId>) -> Option<NodeId> {
    children(Some(Dom::ROOT), true).first().copied()
}

pub fn document_last_element_child(_: Option<NodeId>) -> Option<NodeId> {
    children(Some(Dom::ROOT), true).last().copied()
}

/// Pages are always decoded to UTF-8.
pub fn character_set(_: Option<NodeId>) -> String {
    "UTF-8".into()
}

pub fn content_type(_: Option<NodeId>) -> String {
    "text/html".into()
}

/// "BackCompat" in quirks mode (no or an old doctype), else "CSS1Compat".
pub fn compat_mode(_: Option<NodeId>) -> String {
    with_dom(|d| if d.quirks == Quirks::Quirks { "BackCompat" } else { "CSS1Compat" }).into()
}

/// document.anchors: the `<a>` elements with a name.
pub fn document_anchors(_: Option<NodeId>) -> Vec<NodeId> {
    query(Dom::ROOT, "a[name]", true)
}

/// document.applets is always empty.
pub fn document_applets(_: Option<NodeId>) -> Vec<NodeId> {
    vec![]
}

/// document.embeds and document.plugins: the `<embed>` elements.
pub fn document_embeds(_: Option<NodeId>) -> Vec<NodeId> {
    query(Dom::ROOT, "embed", true)
}
