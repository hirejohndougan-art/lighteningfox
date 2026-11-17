//! DOM objects as native Boa objects.
//!
//! The DOM classes (Node, Element, Text, …) are still declared in `prelude.js`, but the node
//! objects pages see are made here: each one is a native object whose hidden [`NodeHandle`]
//! holds its node id, with the JS class's prototype. The hot read-only members (tree walking,
//! attribute reads, selector queries) are native functions installed on those prototypes; they
//! read the id straight from the handle and hand back node objects from the wrapper cache
//! without going through JavaScript.
//!
//! Members that change the tree or attributes stay in JavaScript on purpose: custom elements
//! and mutation observers hook the global mutation natives (`__insert`, `__setAttr`, …), and a
//! native method calling Rust directly would skip those reactions.
//!
//! Objects made the old way (`new Text(id)`, `document`, shadow roots, fragments from
//! `createDocumentFragment`) carry no handle; the natives fall back to their `__id` property.
//! Anything else as `this` (say `Element.prototype.tagName` read off the prototype) throws a
//! TypeError, as in other browsers.

use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{
    js_string, Context, Finalize, JsData, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace,
};
use boa_engine::object::builtins::JsArray;

use crate::dom::{Dom, NodeData, NodeId};
use crate::js::{with_dom, element_by_id, query, parsed_selectors};

/// The hidden part of a node object: which node it stands for.
#[derive(Debug, JsData)]
pub(crate) struct NodeHandle {
    pub(crate) id: NodeId,
}

impl Finalize for NodeHandle {}

// SAFETY: a NodeHandle holds no garbage-collected values.
unsafe impl Trace for NodeHandle {
    boa_engine::gc::empty_trace!();
}

/// The prototypes node objects get, and the wrapper cache (`__wrapCache`), for one page.
struct Classes {
    element: JsObject,
    text: JsObject,
    comment: JsObject,
    fragment: JsObject,
    cache: JsObject,
    /// The prelude's own wrap, for the kinds made in JavaScript (document, shadow roots).
    slow: JsObject,
}

// ------------------------------------------------------------ wrapping

/// The node object for `id` (null for none): the cached one, or a new native one.
pub(crate) fn wrap(id: Option<NodeId>, ctx: &mut Context) -> JsResult<JsValue> {
    let Some(id) = id else { return Ok(JsValue::null()) };
    let Some(c) = ctx.get_data::<Classes>() else { return Ok(JsValue::null()) };
    let cache = c.cache.clone();
    let hit = cache.get(id as u32, ctx)?;
    if hit.is_object() {
        return Ok(hit);
    }
    let Some(c) = ctx.get_data::<Classes>() else { return Ok(JsValue::null()) };
    let kind = with_dom(|d| {
        if id >= d.nodes.len() {
            return None;
        }
        Some(match &d.nodes[id].data {
            NodeData::Document => 0,
            NodeData::Text(_) => 3,
            NodeData::Comment(_) => 8,
            NodeData::Fragment | NodeData::Element { .. } if node_type(d, id) == 11 => {
                if d.shadow_hosts.contains_key(&id) {
                    0
                } else {
                    11
                }
            }
            _ => 1,
        })
    });
    let proto = match kind {
        None => return Ok(JsValue::null()),
        Some(0) => {
            let slow = c.slow.clone();
            return slow.call(&JsValue::undefined(), &[JsValue::from(id as f64)], ctx);
        }
        Some(3) => c.text.clone(),
        Some(8) => c.comment.clone(),
        Some(11) => c.fragment.clone(),
        Some(_) => c.element.clone(),
    };
    let obj = JsObject::from_proto_and_data(proto, NodeHandle { id });
    // The id is also an ordinary property, as before, for the JavaScript helpers that read it.
    obj.create_data_property_or_throw(js_string!("__id"), JsValue::from(id as f64), ctx)?;
    cache.set(id as u32, obj.clone(), false, ctx)?;
    Ok(obj.into())
}

/// Node objects for a list of ids, as an array.
pub(crate) fn wrap_all(list: impl IntoIterator<Item = NodeId>, ctx: &mut Context) -> JsResult<JsValue> {
    let mut out = Vec::new();
    for id in list {
        out.push(wrap(Some(id), ctx)?);
    }
    Ok(JsArray::from_iter(out, ctx).into())
}

/// Whether native wrapping is set up for this page.
pub(crate) fn installed(ctx: &Context) -> bool {
    ctx.get_data::<Classes>().is_some()
}

// ------------------------------------------------------------ `this`

/// The node a value stands for: its handle, else its `__id` property. Ids past the end of the
/// tree come back as None (they answer like an empty node, as the old wrappers did).
fn node_of(v: &JsValue, ctx: &mut Context) -> JsResult<Option<Option<NodeId>>> {
    let Some(o) = v.as_object() else { return Ok(None) };
    if let Some(h) = o.downcast_ref::<NodeHandle>() {
        let id = h.id;
        return Ok(Some(with_dom(|d| (id < d.nodes.len()).then_some(id))));
    }
    let raw = o.get(js_string!("__id"), ctx)?;
    let Some(n) = raw.as_number() else { return Ok(None) };
    if !(n >= 0.0) {
        return Ok(None);
    }
    let id = n as usize;
    Ok(Some(with_dom(|d| (id < d.nodes.len()).then_some(id))))
}

fn this_node(this: &JsValue, ctx: &mut Context) -> JsResult<Option<NodeId>> {
    match node_of(this, ctx)? {
        Some(id) => Ok(id),
        None => Err(JsNativeError::typ().with_message("Illegal invocation: not a DOM node").into()),
    }
}

fn arg_string(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    Ok(a.get(i).cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped())
}

fn s(v: &str) -> JsValue {
    JsValue::from(JsString::from(v))
}

// ------------------------------------------------------------ tree facts

/// nodeType, as `__type` reports it.
pub(crate) fn node_type(d: &Dom, id: NodeId) -> u32 {
    match &d.nodes[id].data {
        NodeData::Document => 9,
        NodeData::Element { tag, .. } if tag == "#fragment" => 11,
        NodeData::Element { .. } => 1,
        NodeData::Text(_) => 3,
        NodeData::Comment(_) => 8,
        NodeData::Doctype { .. } => 10,
        NodeData::ProcessingInstruction { .. } => 7,
        NodeData::Fragment => 11,
    }
}

fn is_element(d: &Dom, id: NodeId) -> bool {
    node_type(d, id) == 1
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

fn matches(id: NodeId, sel: &str) -> bool {
    let sels = parsed_selectors(sel);
    with_dom(|d| sels.iter().any(|s| crate::style::matches(d, id, s)))
}

/// getElementsByClassName's selector for a class list ("a b" → ".a.b").
fn class_selector(c: &str) -> String {
    c.trim().split_whitespace().map(|x| format!(".{x}")).collect::<Vec<_>>().join("")
}

// ------------------------------------------------------------ Node

fn node_type_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(JsValue::from(id.map_or(0, |id| with_dom(|d| node_type(d, id)))))
}

fn parent_node(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(parent(id), ctx)
}

fn parent_element(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let p = parent(id).filter(|&p| with_dom(|d| is_element(d, p)));
    wrap(p, ctx)
}

fn child_nodes(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap_all(children(id, false), ctx)
}

fn first_child(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(id.and_then(|id| with_dom(|d| d.nodes[id].children.first().copied())), ctx)
}

fn last_child(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(id.and_then(|id| with_dom(|d| d.nodes[id].children.last().copied())), ctx)
}

fn next_sibling(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(sibling(id, 1, false), ctx)
}

fn previous_sibling(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(sibling(id, -1, false), ctx)
}

fn has_child_nodes(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(JsValue::from(id.is_some_and(|id| with_dom(|d| !d.nodes[id].children.is_empty()))))
}

fn is_connected(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(JsValue::from(id.is_some_and(|id| {
        with_dom(|d| {
            let mut n = id;
            while n != Dom::ROOT {
                match d.nodes[n].parent {
                    Some(p) => n = p,
                    None => return false,
                }
            }
            true
        })
    })))
}

fn text_content(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(s(&id.map(|id| with_dom(|d| d.text_content(id))).unwrap_or_default()))
}

fn contains(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let other = node_of(&a.get(0).cloned().unwrap_or_default(), ctx)?.flatten();
    let (Some(id), Some(mut n)) = (id, other) else { return Ok(JsValue::from(false)) };
    Ok(JsValue::from(with_dom(|d| loop {
        if n == id {
            return true;
        }
        match d.nodes[n].parent {
            Some(p) => n = p,
            None => return false,
        }
    })))
}

// ------------------------------------------------------------ Element

fn tag_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(s(&tag(id).to_uppercase()))
}

fn local_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(s(&tag(id)))
}

fn get_attribute(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let name = arg_string(a, 0, ctx)?.to_lowercase();
    Ok(attr(id, &name).map_or(JsValue::null(), |v| s(&v)))
}

fn has_attribute(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let name = arg_string(a, 0, ctx)?.to_lowercase();
    Ok(JsValue::from(id.is_some_and(|id| with_dom(|d| d.attr(id, &name).is_some()))))
}

fn id_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(s(&attr(id, "id").unwrap_or_default()))
}

fn class_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(s(&attr(id, "class").unwrap_or_default()))
}

fn element_children(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap_all(children(id, true), ctx)
}

fn child_element_count(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    Ok(JsValue::from(children(id, true).len() as f64))
}

fn first_element_child(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(children(id, true).first().copied(), ctx)
}

fn last_element_child(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(children(id, true).last().copied(), ctx)
}

fn next_element_sibling(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(sibling(id, 1, true), ctx)
}

fn previous_element_sibling(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    wrap(sibling(id, -1, true), ctx)
}

fn query_selector(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let sel = arg_string(a, 0, ctx)?;
    wrap(id.and_then(|id| query(id, &sel, false).first().copied()), ctx)
}

fn query_selector_all(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let sel = arg_string(a, 0, ctx)?;
    wrap_all(id.map(|id| query(id, &sel, true)).unwrap_or_default(), ctx)
}

fn matches_fn(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let sel = arg_string(a, 0, ctx)?;
    Ok(JsValue::from(id.is_some_and(|id| matches(id, &sel))))
}

fn closest(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let sel = arg_string(a, 0, ctx)?;
    let mut n = id;
    while let Some(k) = n {
        if !with_dom(|d| is_element(d, k)) {
            break;
        }
        if matches(k, &sel) {
            return wrap(Some(k), ctx);
        }
        n = parent(Some(k));
    }
    Ok(JsValue::null())
}

fn by_tag_name(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let t = arg_string(a, 0, ctx)?;
    wrap_all(id.map(|id| query(id, &t, true)).unwrap_or_default(), ctx)
}

fn by_class_name(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_node(this, ctx)?;
    let c = class_selector(&arg_string(a, 0, ctx)?);
    wrap_all(id.map(|id| query(id, &c, true)).unwrap_or_default(), ctx)
}

// ------------------------------------------------------------ Document
// The page's document searches the whole page tree (root 0), whatever its `this` id.

fn doc_by_id(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    this_node(this, ctx)?;
    let want = arg_string(a, 0, ctx)?;
    wrap(element_by_id(&want), ctx)
}

fn doc_query(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    this_node(this, ctx)?;
    let sel = arg_string(a, 0, ctx)?;
    wrap(query(Dom::ROOT, &sel, false).first().copied(), ctx)
}

fn doc_query_all(this: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    this_node(this, ctx)?;
    let sel = arg_string(a, 0, ctx)?;
    wrap_all(query(Dom::ROOT, &sel, true), ctx)
}

fn doc_find(this: &JsValue, ctx: &mut Context, sel: &str) -> JsResult<JsValue> {
    this_node(this, ctx)?;
    wrap(query(Dom::ROOT, sel, false).first().copied(), ctx)
}

fn doc_element(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    doc_find(this, ctx, "html")
}

fn doc_head(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    doc_find(this, ctx, "head")
}

fn doc_body(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    doc_find(this, ctx, "body")
}

// ------------------------------------------------------------ install

type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

const NODE_GET: &[(&str, Native)] = &[
    ("nodeType", node_type_get),
    ("parentNode", parent_node),
    ("parentElement", parent_element),
    ("childNodes", child_nodes),
    ("firstChild", first_child),
    ("lastChild", last_child),
    ("nextSibling", next_sibling),
    ("previousSibling", previous_sibling),
    ("isConnected", is_connected),
    ("textContent", text_content),
];
const NODE_FN: &[(&str, Native, usize)] = &[("hasChildNodes", has_child_nodes, 0), ("contains", contains, 1)];

const ELEMENT_GET: &[(&str, Native)] = &[
    ("tagName", tag_name),
    ("nodeName", tag_name),
    ("localName", local_name),
    ("id", id_get),
    ("className", class_name),
    ("children", element_children),
    ("childElementCount", child_element_count),
    ("firstElementChild", first_element_child),
    ("lastElementChild", last_element_child),
    ("nextElementSibling", next_element_sibling),
    ("previousElementSibling", previous_element_sibling),
];
const ELEMENT_FN: &[(&str, Native, usize)] = &[
    ("getAttribute", get_attribute, 1),
    ("hasAttribute", has_attribute, 1),
    ("querySelector", query_selector, 1),
    ("querySelectorAll", query_selector_all, 1),
    ("matches", matches_fn, 1),
    ("closest", closest, 1),
    ("getElementsByTagName", by_tag_name, 1),
    ("getElementsByClassName", by_class_name, 1),
];

const DOCUMENT_GET: &[(&str, Native)] = &[("documentElement", doc_element), ("head", doc_head), ("body", doc_body)];
const DOCUMENT_FN: &[(&str, Native, usize)] = &[
    ("getElementById", doc_by_id, 1),
    ("querySelector", doc_query, 1),
    ("querySelectorAll", doc_query_all, 1),
];

fn function(name: &str, f: Native, len: usize, ctx: &mut Context) -> JsValue {
    FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f))
        .name(JsString::from(name))
        .length(len)
        .build()
        .into()
}

/// `{get: {name: fn}, fn: {name: fn}}` for one prototype.
fn members(get: &[(&str, Native)], fns: &[(&str, Native, usize)], ctx: &mut Context) -> JsObject {
    let mut g = Vec::new();
    for (name, f) in get {
        g.push((*name, function(name, *f, 0, ctx)));
    }
    let mut m = Vec::new();
    for (name, f, len) in fns {
        m.push((*name, function(name, *f, *len, ctx)));
    }
    let mut gi = ObjectInitializer::new(ctx);
    for (name, f) in g {
        gi.property(JsString::from(name), f, Attribute::all());
    }
    let gobj = gi.build();
    let mut mi = ObjectInitializer::new(ctx);
    for (name, f) in m {
        mi.property(JsString::from(name), f, Attribute::all());
    }
    let mobj = mi.build();
    ObjectInitializer::new(ctx)
        .property(js_string!("get"), gobj, Attribute::all())
        .property(js_string!("fn"), mobj, Attribute::all())
        .build()
}

fn get_obj(o: &JsObject, key: &str, ctx: &mut Context) -> JsResult<JsObject> {
    o.get(JsString::from(key), ctx)?
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message(format!("__domInstall: missing {key}")).into())
}

fn proto_of(o: &JsObject, key: &str, ctx: &mut Context) -> JsResult<JsObject> {
    let class = get_obj(o, key, ctx)?;
    get_obj(&class, "prototype", ctx)
}

/// `__domInstall({Element, Text, Comment, DocumentFragment, cache, slow})`: remembers the
/// prototypes for native node objects and returns the native members, plus `wrap`, for the
/// prelude to put on its classes.
fn n_install(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let Some(cfg) = a.first().and_then(|v| v.as_object()) else {
        return Err(JsNativeError::typ().with_message("__domInstall: no classes").into());
    };
    let classes = Classes {
        element: proto_of(&cfg, "Element", ctx)?,
        text: proto_of(&cfg, "Text", ctx)?,
        comment: proto_of(&cfg, "Comment", ctx)?,
        fragment: proto_of(&cfg, "DocumentFragment", ctx)?,
        cache: get_obj(&cfg, "cache", ctx)?,
        slow: get_obj(&cfg, "slow", ctx)?,
    };
    ctx.insert_data(classes);
    let node = members(NODE_GET, NODE_FN, ctx);
    let element = members(ELEMENT_GET, ELEMENT_FN, ctx);
    let document = members(DOCUMENT_GET, DOCUMENT_FN, ctx);
    let wrap_fn = function("wrap", n_wrap, 1, ctx);
    Ok(ObjectInitializer::new(ctx)
        .property(js_string!("node"), node, Attribute::all())
        .property(js_string!("element"), element, Attribute::all())
        .property(js_string!("document"), document, Attribute::all())
        .property(js_string!("wrap"), wrap_fn, Attribute::all())
        .build()
        .into())
}

/// wrap(id): the node object for an id (null for a negative or missing one).
fn n_wrap(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let n = a.first().cloned().unwrap_or_default();
    if n.is_null_or_undefined() {
        return Ok(JsValue::null());
    }
    let n = n.to_number(ctx)?;
    if !(n >= 0.0) {
        return Ok(JsValue::null());
    }
    wrap(Some(n as usize), ctx)
}

/// Registers `__domInstall` (the prelude calls it once its classes exist).
pub(crate) fn register(ctx: &mut Context) {
    ctx.register_global_callable(js_string!("__domInstall"), 1, NativeFunction::from_fn_ptr(n_install))
        .expect("register __domInstall");
}
