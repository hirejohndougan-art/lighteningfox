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
use crate::js::with_dom;

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
pub(crate) fn node_of(v: &JsValue, ctx: &mut Context) -> JsResult<Option<Option<NodeId>>> {
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

// ------------------------------------------------------------ install

use crate::bindings::Native;

fn function(name: &str, f: Native, len: usize, ctx: &mut Context) -> JsValue {
    FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f))
        .name(JsString::from(name))
        .length(len)
        .build()
        .into()
}

/// `{get: {name: fn}, fn: {name: fn}}` for one interface. Getter functions are named
/// "get <name>", as WebIDL says.
fn members(iface: &crate::bindings::Interface, ctx: &mut Context) -> JsObject {
    let mut g = Vec::new();
    for (name, f) in iface.getters {
        g.push((*name, function(&format!("get {name}"), *f, 0, ctx)));
    }
    let mut m = Vec::new();
    for (name, f, len) in iface.methods {
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
/// prototypes for native node objects and returns the native members by interface
/// (`{interfaces: {Node: {get, fn}, …}, wrap}`) for the prelude to put on its classes.
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
    // The generated WebIDL bindings (src/bindings/), by interface name.
    let mut ifaces = Vec::new();
    for iface in crate::bindings::INTERFACES {
        ifaces.push((iface.name, members(iface, ctx)));
    }
    let mut ii = ObjectInitializer::new(ctx);
    for (name, m) in ifaces {
        ii.property(JsString::from(name), m, Attribute::all());
    }
    let interfaces = ii.build();
    let wrap_fn = function("wrap", n_wrap, 1, ctx);
    Ok(ObjectInitializer::new(ctx)
        .property(js_string!("interfaces"), interfaces, Attribute::all())
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
