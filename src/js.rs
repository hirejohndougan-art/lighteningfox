//! JavaScript: runs page scripts with the Boa engine (pure Rust).
//!
//! The DOM lives in a thread-local so Boa's native functions can reach it.
//! Rust exposes ~20 small primitives over node ids; `prelude.js` builds the
//! web-facing API (document, Element, events, timers...) on top of them.

use crate::css;
use crate::dom::{Dom, NodeData, NodeId};
use crate::html;
use crate::net;
use crate::style;
use boa_engine::object::builtins::JsArray;
use boa_engine::{js_string, Context, JsResult, JsString, JsValue, NativeFunction, Source};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Results of background requests, waiting to be handed to the page's scripts.
static NET_DONE: std::sync::Mutex<Vec<(u64, u64, Result<net::Response, String>)>> = std::sync::Mutex::new(Vec::new());
static NET_IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Bumped for every new page, so a request from a page you've left is dropped when it finishes.
static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

thread_local! {
    static DOM: RefCell<Dom> = RefCell::new(Dom::new());
    static DIRTY: Cell<bool> = const { Cell::new(false) };
    static NAVIGATE: RefCell<Option<String>> = const { RefCell::new(None) };
    static CURRENT_SCRIPT: Cell<i64> = const { Cell::new(-1) };
    /// The HTML parser while a page is still being parsed (document.write feeds it).
    static PARSER: RefCell<Option<crate::treebuilder::TreeBuilder>> = const { RefCell::new(None) };
    /// Requests from script for the browser: submit a form, move focus.
    static REQUESTS: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
    static HISTORY: RefCell<Vec<HistoryOp>> = const { RefCell::new(Vec::new()) };
    static OPEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Permission requests waiting for the browser's prompt: (request id, kind, page URL).
    static PERMISSION_REQUESTS: RefCell<Vec<(u64, crate::permissions::Kind, String)>> = const { RefCell::new(Vec::new()) };
    /// True while a user's click is being handled (window.open is allowed then).
    static ACTIVATION: Cell<bool> = const { Cell::new(false) };
    /// Bumped on every DOM change; caches built from the DOM remember the generation they saw.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// id attribute -> first element with it, in document order (rebuilt lazily after changes).
    static ID_INDEX: RefCell<(u64, HashMap<String, NodeId>)> = RefCell::new((u64::MAX, HashMap::new()));
    /// Parsed selectors, so a script calling querySelector in a loop parses each string once.
    static SELECTORS: RefCell<HashMap<String, std::rc::Rc<Vec<css::Selector>>>> = RefCell::new(HashMap::new());
}

/// Who a page is, kept beside its script engine where scripts can't change it. Natives that
/// touch cookies, storage or the network act for this page, whatever a script passes them.
pub struct PageCtx {
    /// The document's URL (pushState can move it within the same origin).
    pub url: String,
    /// The top-level page (differs from `url` inside a frame).
    pub top: String,
    pub sandbox: Option<crate::security::Sandbox>,
    pub policy: std::sync::Arc<crate::security::PagePolicy>,
    /// Handler attribute code a policy allowed, waiting for `new Function` to compile it.
    approved: Vec<String>,
}

impl PageCtx {
    /// A sandboxed frame without allow-same-origin has an opaque origin: no cookies or storage,
    /// and it counts as a stranger to every site.
    fn opaque(&self) -> bool {
        self.sandbox.as_ref().is_some_and(|s| !s.same_origin)
    }

    /// The URL to use as "the page" for network rules.
    fn net_page(&self) -> String {
        if self.opaque() { "about:sandbox".into() } else { self.url.clone() }
    }

    /// Where this page's localStorage is kept: its origin, plus the top-level site when it's
    /// embedded in another site (so an embedded tracker can't share storage across sites).
    fn storage_key(&self) -> Option<String> {
        if self.opaque() {
            return None;
        }
        let origin = if self.url.starts_with("file:") { "file".to_string() } else { net::origin_of(&self.url) };
        if origin == "null" {
            return None;
        }
        let site = |u: &str| url::Url::parse(u).ok().and_then(|u| u.host_str().map(crate::cookies::site_of));
        if self.top != self.url && site(&self.top) != site(&self.url) {
            let top = site(&self.top).unwrap_or_else(|| "local".into());
            return Some(format!("{origin} in {top}"));
        }
        Some(origin)
    }
}

fn page_ctx<R>(ctx: &Context, f: impl FnOnce(&PageCtx) -> R) -> Option<R> {
    ctx.get_data::<RefCell<PageCtx>>().map(|p| f(&p.borrow()))
}

/// Script engine hooks: turning strings into code (`eval`, `new Function`, string timers)
/// is only allowed when the page's Content Security Policy allows it.
struct Hooks;

impl boa_engine::context::HostHooks for Hooks {
    fn ensure_can_compile_strings(
        &self,
        _realm: boa_engine::realm::Realm,
        _parameters: &[JsString],
        body: &JsString,
        _direct: bool,
        context: &mut Context,
    ) -> JsResult<()> {
        let Some(page) = context.get_data::<RefCell<PageCtx>>() else { return Ok(()) };
        let mut page = page.borrow_mut();
        let body = body.to_std_string_escaped();
        if let Some(i) = page.approved.iter().position(|c| *c == body) {
            page.approved.swap_remove(i);
            return Ok(());
        }
        if page.policy.csp.is_empty() || page.policy.csp.allows_eval() {
            return Ok(());
        }
        Err(boa_engine::JsNativeError::eval()
            .with_message("Refused to evaluate a string as JavaScript: 'unsafe-eval' is not allowed by the page's Content Security Policy")
            .into())
    }
}

fn bump_generation() {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
}

fn parsed_selectors(sel: &str) -> std::rc::Rc<Vec<css::Selector>> {
    SELECTORS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(v) = c.get(sel) {
            return v.clone();
        }
        if c.len() >= 512 {
            c.clear();
        }
        let v = std::rc::Rc::new(css::parse_selector_list(sel));
        c.insert(sel.to_string(), v.clone());
        v
    })
}

/// The first element in the document with this id.
fn element_by_id(want: &str) -> Option<NodeId> {
    let generation = GENERATION.with(|g| g.get());
    ID_INDEX.with(|ix| {
        let mut ix = ix.borrow_mut();
        if ix.0 != generation {
            let map = &mut ix.1;
            map.clear();
            with_dom(|d| {
                d.walk(Dom::ROOT, |n| {
                    if let Some(id) = d.attr(n, "id") {
                        if !id.is_empty() && !map.contains_key(id) {
                            map.insert(id.to_string(), n);
                        }
                    }
                    true
                })
            });
            ix.0 = generation;
        }
        ix.1.get(want).copied()
    })
}

/// Things scripts ask the browser to do that the window loop carries out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    /// form.submit() (no submit event) or requestSubmit() (fires one): (form, submitter, fire_event)
    Submit(NodeId, Option<NodeId>, bool),
    Reset(NodeId),
    Focus(NodeId),
    Blur(NodeId),
}

/// History changes a page's script asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryOp {
    /// pushState / replaceState: (url, state as JSON)
    Push(String, String),
    Replace(String, String),
    /// location.hash = …, or assign() of a same-document URL with a fragment
    Hash(String, bool),
    /// history.go(n), back(), forward()
    Go(i32),
}

pub fn take_history() -> Vec<HistoryOp> {
    HISTORY.with(|h| std::mem::take(&mut *h.borrow_mut()))
}

pub fn take_requests() -> Vec<Request> {
    REQUESTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

pub fn set_dom(dom: Dom) {
    bump_generation();
    DOM.with(|d| *d.borrow_mut() = dom);
}

pub fn with_dom<R>(f: impl FnOnce(&Dom) -> R) -> R {
    DOM.with(|d| f(&d.borrow()))
}

pub fn with_dom_mut<R>(f: impl FnOnce(&mut Dom) -> R) -> R {
    DIRTY.with(|d| d.set(true));
    bump_generation();
    crate::geometry::dom_changed();
    DOM.with(|d| f(&mut d.borrow_mut()))
}

/// True if scripts changed the DOM since the last call.
pub fn take_dirty() -> bool {
    DIRTY.with(|d| d.replace(false))
}

/// A navigation requested by script (location.href = ...).
pub fn take_navigation() -> Option<String> {
    NAVIGATE.with(|n| n.borrow_mut().take())
}

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub scripts: usize,
    pub errors: usize,
    pub script_time: Duration,
}

pub struct Js {
    ctx: Context,
    start: Instant,
    pub stats: Stats,
    /// This page's tag on background requests (pages in other tabs have their own).
    epoch: u64,
    /// Finds and caches the page's ES modules.
    modules: std::rc::Rc<crate::jsmodules::Loader>,
}

// ------------------------------------------------------------ helpers

fn arg(args: &[JsValue], i: usize) -> JsValue {
    args.get(i).cloned().unwrap_or_default()
}

fn arg_id(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<i64> {
    Ok(arg(args, i).to_number(ctx)? as i64)
}

fn arg_str(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    Ok(arg(args, i).to_string(ctx)?.to_std_string_escaped())
}

fn valid(id: i64) -> Option<NodeId> {
    let n = with_dom(|d| d.nodes.len());
    (id >= 0 && (id as usize) < n).then_some(id as usize)
}

fn s(v: &str) -> JsValue {
    JsValue::from(JsString::from(v))
}

fn ids(list: impl IntoIterator<Item = NodeId>, ctx: &mut Context) -> JsValue {
    JsArray::from_iter(list.into_iter().map(|i| JsValue::from(i as f64)), ctx).into()
}

fn parse_fragment(markup: &str) -> Dom {
    // Parse as a fragment by wrapping so the parser doesn't need a context node.
    html::parse_fragment(markup)
}

fn clone_node(dom: &mut Dom, id: NodeId, deep: bool) -> NodeId {
    let new = dom.create(dom.nodes[id].data.clone());
    if let Some(&ns) = dom.ns.get(&id) {
        dom.ns.insert(new, ns);
    }
    if deep {
        for c in dom.nodes[id].children.clone() {
            let k = clone_node(dom, c, true);
            dom.insert(new, k, None);
        }
        // A template's contents are copied with it.
        if let Some(&content) = dom.template_contents.get(&id) {
            let copy = clone_node(dom, content, true);
            dom.template_contents.insert(new, copy);
        }
    }
    new
}

/// __idList(): every id used in the document (for window.<id> named access).
fn n_id_list(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let ids: Vec<String> = with_dom(|d| {
        let mut v = Vec::new();
        d.walk(Dom::ROOT, |n| {
            if let Some(id) = d.attr(n, "id") {
                if !id.is_empty() {
                    v.push(id.to_string());
                }
            }
            true
        });
        v
    });
    Ok(JsArray::from_iter(ids.iter().map(|i| s(i)), ctx).into())
}

/// __shadowRootList(): every shadow root (for slotchange checks).
fn n_shadow_root_list(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mut roots: Vec<NodeId> = with_dom(|d| d.shadow_hosts.keys().copied().collect());
    roots.sort_unstable();
    Ok(JsArray::from_iter(roots.into_iter().map(|r| JsValue::from(r as f64)), ctx).into())
}

/// __templateContent(template): its contents fragment (made on first use for templates
/// created by scripts).
fn n_template_content(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let Some(id) = valid(id) else { return Ok(JsValue::from(-1)) };
    let c = with_dom_mut(|d| match d.template_contents.get(&id) {
        Some(&c) => c,
        None => {
            let c = d.create(NodeData::Fragment);
            d.template_contents.insert(id, c);
            c
        }
    });
    Ok(JsValue::from(c as f64))
}

// ------------------------------------------------------------ natives

fn n_type(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    Ok(JsValue::from(valid(id).map_or(0, |id| {
        with_dom(|d| match &d.nodes[id].data {
            NodeData::Document => 9,
            NodeData::Element { tag, .. } if tag == "#fragment" => 11,
            NodeData::Element { .. } => 1,
            NodeData::Text(_) => 3,
            NodeData::Comment(_) => 8,
            NodeData::Doctype { .. } => 10,
            NodeData::ProcessingInstruction { .. } => 7,
            NodeData::Fragment => 11,
        })
    })))
}

/// __attachShadow(host, open): the host's new shadow root.
fn n_attach_shadow(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let open = a.get(1).is_some_and(|v| v.to_boolean());
    let Some(id) = valid(id) else { return Ok(JsValue::from(-1)) };
    Ok(JsValue::from(with_dom_mut(|d| d.attach_shadow(id, open)) as f64))
}

/// __shadowRoot(host, any): its open shadow root (any = closed ones too, for internal use), or -1.
fn n_shadow_root(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let any = a.get(1).is_some_and(|v| v.to_boolean());
    let r = valid(id).and_then(|id| with_dom(|d| d.shadow_roots.get(&id).copied().filter(|r| any || d.shadow_hosts.get(r).is_some_and(|h| h.1))));
    Ok(JsValue::from(r.map_or(-1.0, |r| r as f64)))
}

/// __shadowHost(root): [host, open] for a shadow root, or null.
fn n_shadow_host(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let h = valid(id).and_then(|id| with_dom(|d| d.shadow_hosts.get(&id).copied()));
    Ok(match h {
        Some((host, open)) => JsArray::from_iter([JsValue::from(host as f64), JsValue::from(open)], ctx).into(),
        None => JsValue::null(),
    })
}

fn n_parent(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let p = valid(id).and_then(|id| with_dom(|d| d.nodes[id].parent)).map_or(-1.0, |p| p as f64);
    Ok(JsValue::from(p))
}

fn n_children(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let kids = valid(id).map(|id| with_dom(|d| d.nodes[id].children.clone())).unwrap_or_default();
    Ok(ids(kids, ctx))
}

fn n_text(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    Ok(s(&valid(id).map(|id| with_dom(|d| d.text_content(id))).unwrap_or_default()))
}

fn n_set_text(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let text = arg_str(a, 1, ctx)?;
    if let Some(id) = valid(id) {
        with_dom_mut(|d| {
            if let NodeData::Text(t) = &mut d.nodes[id].data {
                *t = text;
            } else {
                d.remove_children(id);
                if !text.is_empty() {
                    let t = d.create(NodeData::Text(text));
                    d.insert(id, t, None);
                }
            }
        });
    }
    Ok(JsValue::undefined())
}

fn n_insert(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, c, b) = (arg_id(a, 0, ctx)?, arg_id(a, 1, ctx)?, arg_id(a, 2, ctx)?);
    if let (Some(p), Some(c)) = (valid(p), valid(c)) {
        let before = valid(b);
        with_dom_mut(|d| d.insert(p, c, before));
    }
    Ok(JsValue::undefined())
}

fn n_detach(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if let Some(id) = valid(arg_id(a, 0, ctx)?) {
        with_dom_mut(|d| d.detach(id));
    }
    Ok(JsValue::undefined())
}

fn n_clone(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let deep = arg(a, 1).to_boolean();
    let new = valid(id).map_or(-1.0, |id| with_dom_mut(|d| clone_node(d, id, deep)) as f64);
    Ok(JsValue::from(new))
}

fn n_tag(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    Ok(s(&valid(id).and_then(|id| with_dom(|d| d.tag(id).map(String::from))).unwrap_or_default()))
}

fn n_get_attr(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let name = arg_str(a, 1, ctx)?;
    Ok(valid(id).and_then(|id| with_dom(|d| d.attr(id, &name).map(s))).unwrap_or(JsValue::null()))
}

fn n_set_attr(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let (name, value) = (arg_str(a, 1, ctx)?, arg_str(a, 2, ctx)?);
    if let Some(id) = valid(id) {
        with_dom_mut(|d| {
            if let NodeData::Element { attrs, .. } = &mut d.nodes[id].data {
                match attrs.iter_mut().find(|(k, _)| *k == name) {
                    Some(slot) => slot.1 = value,
                    None => attrs.push((name, value)),
                }
            }
        });
    }
    Ok(JsValue::undefined())
}

fn n_remove_attr(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let name = arg_str(a, 1, ctx)?;
    if let Some(id) = valid(id) {
        with_dom_mut(|d| {
            if let NodeData::Element { attrs, .. } = &mut d.nodes[id].data {
                attrs.retain(|(k, _)| *k != name);
            }
        });
    }
    Ok(JsValue::undefined())
}

fn n_attr_names(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let names: Vec<String> = valid(id)
        .map(|id| {
            with_dom(|d| match &d.nodes[id].data {
                NodeData::Element { attrs, .. } => attrs.iter().map(|(k, _)| k.clone()).collect(),
                _ => vec![],
            })
        })
        .unwrap_or_default();
    Ok(JsArray::from_iter(names.iter().map(|n| s(n)), ctx).into())
}

fn n_get_html(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let outer = arg(a, 1).to_boolean();
    let html = valid(id)
        .map(|id| {
            with_dom(|d| {
                if outer {
                    let mut o = String::new();
                    d.outer_html_into(id, &mut o);
                    o
                } else {
                    d.inner_html(id)
                }
            })
        })
        .unwrap_or_default();
    Ok(s(&html))
}

fn n_set_html(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let markup = arg_str(a, 1, ctx)?;
    if let Some(id) = valid(id) {
        let frag = parse_fragment(&markup);
        with_dom_mut(|d| {
            d.remove_children(id);
            d.graft(&frag, Dom::ROOT, id);
        });
    }
    Ok(JsValue::undefined())
}

fn query(root: NodeId, sel: &str, all: bool) -> Vec<NodeId> {
    let sels = parsed_selectors(sel);
    if sels.is_empty() {
        return vec![];
    }
    // querySelector("#x …"): the id index finds the one candidate. No element with that id
    // means no match; a candidate that doesn't fit (duplicate ids, outside root) falls back.
    // (Only for searches inside the document: shadow trees and detached nodes aren't indexed.)
    if !all && sels.len() == 1 && with_dom(|d| d.is_inside(root, Dom::ROOT)) {
        if let Some(want) = sels[0].parts.last().and_then(|c| c.id.as_deref()) {
            match element_by_id(want) {
                None => return vec![],
                Some(n) => {
                    if with_dom(|d| n != root && d.is_inside(n, root) && style::matches(d, n, &sels[0])) {
                        return vec![n];
                    }
                }
            }
        }
    }
    with_dom(|d| {
        let mut out = Vec::new();
        d.walk(root, |n| {
            if d.tag(n).is_some() && sels.iter().any(|s| style::matches(d, n, s)) {
                out.push(n);
                return all;
            }
            true
        });
        out
    })
}

fn n_query(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let sel = arg_str(a, 1, ctx)?;
    let all = arg(a, 2).to_boolean();
    let found = valid(id).map(|id| query(id, &sel, all)).unwrap_or_default();
    Ok(ids(found, ctx))
}

fn n_matches(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let sel = arg_str(a, 1, ctx)?;
    let sels = parsed_selectors(&sel);
    let m = valid(id).map_or(false, |id| with_dom(|d| sels.iter().any(|s| style::matches(d, id, s))));
    Ok(JsValue::from(m))
}

/// Node ids -> their element objects, in one native pass (an Array.map in Boa costs far more).
fn n_wrap_list(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let Some(list) = arg(a, 0).as_object() else { return Ok(JsArray::new(ctx)?.into()) };
    let list = JsArray::from_object(list)?;
    let global = ctx.global_object();
    let cache = global.get(js_string!("__wrapCache"), ctx)?;
    let wrap = global.get(js_string!("__wrapFn"), ctx)?;
    let (Some(cache), Some(wrap)) = (cache.as_object(), wrap.as_callable()) else {
        return Ok(list.into());
    };
    let len = list.length(ctx)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let id = list.get(i, ctx)?;
        let key = id.to_u32(ctx)?;
        let mut v = cache.get(key, ctx)?;
        if v.is_undefined() {
            v = wrap.call(&JsValue::undefined(), &[id], ctx)?;
        }
        out.push(v);
    }
    Ok(JsArray::from_iter(out, ctx).into())
}

fn n_by_id(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let want = arg_str(a, 0, ctx)?;
    let found = element_by_id(&want);
    Ok(JsValue::from(found.map_or(-1.0, |n| n as f64)))
}

fn n_create(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let tag = arg_str(a, 0, ctx)?;
    let id = with_dom_mut(|d| d.create(NodeData::Element { tag, attrs: Vec::new() }));
    Ok(JsValue::from(id as f64))
}

fn n_create_text(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_str(a, 0, ctx)?;
    let id = with_dom_mut(|d| d.create(NodeData::Text(text)));
    Ok(JsValue::from(id as f64))
}

fn n_write(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let markup = arg_str(a, 0, ctx)?;
    // While the page is being parsed, the text goes into the parser's input right after
    // the running script.
    if PARSER.with(|p| p.borrow_mut().as_mut().is_some_and(|tb| tb.write(&markup))) {
        return Ok(JsValue::undefined());
    }
    let frag = parse_fragment(&markup);
    let script = CURRENT_SCRIPT.with(|c| c.get());
    with_dom_mut(|d| {
        // Insert right after the running <script>, else at the end of <body>.
        let anchor = valid_in(d, script);
        let (parent, before) = match anchor.and_then(|s| d.nodes[s].parent.map(|p| (p, s))) {
            Some((p, s)) => {
                let kids = &d.nodes[p].children;
                let pos = kids.iter().position(|&k| k == s).unwrap_or(kids.len());
                (p, kids.get(pos + 1).copied())
            }
            None => (d.find_all("body").first().copied().unwrap_or(Dom::ROOT), None),
        };
        let tmp = d.create(NodeData::Element { tag: "#fragment".into(), attrs: Vec::new() });
        d.graft(&frag, Dom::ROOT, tmp);
        for k in d.nodes[tmp].children.clone() {
            d.insert(parent, k, before);
        }
    });
    Ok(JsValue::undefined())
}

fn valid_in(d: &Dom, id: i64) -> Option<NodeId> {
    (id >= 0 && (id as usize) < d.nodes.len()).then_some(id as usize)
}

fn n_log(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let level = arg_str(a, 0, ctx)?;
    let msg = arg_str(a, 1, ctx)?;
    log!("[js {level}] {msg}");
    Ok(JsValue::undefined())
}

fn n_navigate(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let url = arg_str(a, 0, ctx)?;
    NAVIGATE.with(|n| *n.borrow_mut() = Some(url));
    Ok(JsValue::undefined())
}

fn n_request(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let kind = arg_str(a, 0, ctx)?;
    let id = a.get(1).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(-1.0);
    let other = a.get(2).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(-1.0);
    let valid = with_dom(|d| id >= 0.0 && (id as usize) < d.nodes.len());
    if valid {
        let id = id as NodeId;
        let other = (other >= 0.0).then_some(other as NodeId);
        let req = match kind.as_str() {
            "submit" => Request::Submit(id, other, false),
            "requestSubmit" => Request::Submit(id, other, true),
            "reset" => Request::Reset(id),
            "focus" => Request::Focus(id),
            _ => Request::Blur(id),
        };
        REQUESTS.with(|r| r.borrow_mut().push(req));
    }
    Ok(JsValue::undefined())
}

/// __getCookie(): document.cookie for this page (the argument is ignored: a page only ever
/// reads its own cookies).
fn n_get_cookie(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let v = page_ctx(ctx, |p| if p.opaque() { String::new() } else { crate::cookies::script_get(&p.url, &p.top) }).unwrap_or_default();
    Ok(JsValue::from(JsString::from(v)))
}

/// __setCookie(_, "name=value; …")
fn n_set_cookie(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let v = arg_str(a, 1, ctx)?;
    page_ctx(ctx, |p| {
        if !p.opaque() {
            crate::cookies::script_set(&p.url, &p.top, &v);
        }
    });
    Ok(JsValue::undefined())
}

/// __cspInline(kind, code): may this inline handler run? ("attr" = an on… attribute.)
/// If so, `new Function(code)` is allowed once even where eval isn't.
fn n_csp_inline(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let code = arg_str(a, 1, ctx)?;
    let Some(page) = ctx.get_data::<RefCell<PageCtx>>() else { return Ok(JsValue::from(true)) };
    let mut page = page.borrow_mut();
    let ok = page.policy.csp.allows_inline(crate::security::Inline::ScriptAttr, &code, None);
    if ok && !page.policy.csp.is_empty() {
        if page.approved.len() > 64 {
            page.approved.clear();
        }
        page.approved.push(code);
    }
    Ok(JsValue::from(ok))
}

/// __permState(kind) -> "granted" | "denied" | "prompt"
fn n_perm_state(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let kind = arg_str(a, 0, ctx)?;
    let st = match crate::permissions::Kind::parse(&kind) {
        Some(k) => page_ctx(ctx, |p| if p.opaque() { "denied" } else { crate::permissions::state(&p.url, &p.top, k) }).unwrap_or("denied"),
        None => "denied",
    };
    Ok(JsValue::from(js_string!(st)))
}

/// __permRequest(id, kind): ask the user (the answer comes back through __permDone).
fn n_perm_request(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = a.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as u64;
    let kind = arg_str(a, 1, ctx)?;
    let (Some(kind), Some(url)) = (crate::permissions::Kind::parse(&kind), page_ctx(ctx, |p| p.url.clone())) else { return Ok(JsValue::from(false)) };
    let queued = PERMISSION_REQUESTS.with(|r| {
        let mut r = r.borrow_mut();
        // A page can't flood you with prompts.
        if r.len() >= 8 {
            return false;
        }
        r.push((id, kind, url));
        true
    });
    Ok(JsValue::from(queued))
}

/// __notify(title, body): a desktop notification, if the site was allowed to show them.
fn n_notify(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let title = arg_str(a, 0, ctx)?;
    let body = arg_str(a, 1, ctx)?;
    let ok = page_ctx(ctx, |p| {
        let ok = !p.opaque() && crate::permissions::state(&p.url, &p.top, crate::permissions::Kind::Notifications) == "granted";
        if ok {
            crate::permissions::notify(&p.url, &title, &body);
        }
        ok
    })
    .unwrap_or(false);
    Ok(JsValue::from(ok))
}

/// Permission requests from scripts since the last call.
pub fn take_permission_requests() -> Vec<(u64, crate::permissions::Kind, String)> {
    PERMISSION_REQUESTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// __fpSeed() -> a number fixed for this site and this session (0 = fingerprinting
/// protection is off). Canvas and audio readouts get noise derived from it, so a site sees
/// the same values each time it looks, but different sites (and sessions) see different ones.
fn n_fp_seed(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let seed = page_ctx(ctx, |p| crate::security::fingerprint_seed(&p.top)).unwrap_or(0);
    Ok(JsValue::from(seed as f64))
}

/// The document's URL (for the module loader).
pub fn page_url(ctx: &Context) -> String {
    page_ctx(ctx, |p| p.url.clone()).unwrap_or_default()
}

/// __pageUrl(): the document's URL as the browser knows it.
fn n_page_url(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(JsString::from(page_ctx(ctx, |p| p.url.clone()).unwrap_or_default())))
}

/// JSON string literal (also valid JavaScript).
fn json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A response as JSON (also a valid JavaScript expression): {ok, status, statusText, url, headers: [[k, v]…], body}
/// or {error: "…"} for a network failure.
fn response_js(r: &Result<net::Response, String>) -> String {
    let mut o = String::new();
    match r {
        Ok(r) => {
            o.push_str(&format!("{{\"status\":{},\"statusText\":", r.status));
            json_str(&r.status_text, &mut o);
            o.push_str(",\"url\":");
            json_str(&r.url, &mut o);
            o.push_str(",\"headers\":[");
            for (i, (k, v)) in r.headers.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                o.push('[');
                json_str(k, &mut o);
                o.push(',');
                json_str(v, &mut o);
                o.push(']');
            }
            o.push_str("],\"body\":");
            json_str(&String::from_utf8_lossy(&r.body), &mut o);
            o.push('}');
        }
        Err(e) => {
            o.push_str("{\"error\":");
            json_str(e, &mut o);
            o.push('}');
        }
    }
    o
}

/// __netStart(id, page, method, url, headers "k\nv\n…", body|null, credentials, sync)
/// Async requests run on a worker thread; a sync one (XHR with async=false) returns its result.
fn n_net_start(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = a.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as u64;
    // The page making the request is always this page, never what the script says.
    let page = page_ctx(ctx, PageCtx::net_page).unwrap_or_else(|| "about:blank".into());
    let method = arg_str(a, 2, ctx)?;
    let url = arg_str(a, 3, ctx)?;
    let flat = arg_str(a, 4, ctx)?;
    let body = match a.get(5) {
        Some(v) if !v.is_null_or_undefined() => Some(v.to_string(ctx)?.to_std_string_escaped().into_bytes()),
        _ => None,
    };
    let credentials = arg_str(a, 6, ctx)?;
    let sync = a.get(7).is_some_and(JsValue::to_boolean);
    let mut lines = flat.split('\n');
    let mut headers = Vec::new();
    while let (Some(k), Some(v)) = (lines.next(), lines.next()) {
        if !k.is_empty() {
            headers.push((k.to_string(), v.to_string()));
        }
    }
    let csp_ok = page_ctx(ctx, |p| p.policy.csp.allows_url(crate::security::Dest::Connect, &url, None, false)).unwrap_or(true);
    let run = move || {
        if !csp_ok {
            return Err(format!("{url}: blocked by the page's Content Security Policy (connect-src)"));
        }
        if let Some(why) = crate::blocklist::blocked(&url, &page, crate::security::Dest::Connect) {
            return Err(format!("{url}: {why}"));
        }
        net::script_request(&page, &method, &url, &headers, body.as_deref(), &credentials)
    };
    if sync {
        let r = run();
        return Ok(JsValue::from(JsString::from(response_js(&r))));
    }
    NET_IN_FLIGHT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let epoch = EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    std::thread::spawn(move || {
        let r = run();
        if let Ok(mut q) = NET_DONE.lock() {
            q.push((epoch, id, r));
        }
        NET_IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    });
    Ok(JsValue::undefined())
}

fn n_resolve(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let base = arg_str(a, 0, ctx)?;
    let href = arg_str(a, 1, ctx)?;
    Ok(JsValue::from(JsString::from(net::resolve(&base, &href))))
}

/// A form's name/value pairs as JSON: [["name","value"],…]
fn n_form_data(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let pairs = with_dom(|d| if id >= 0 && (id as usize) < d.nodes.len() { crate::forms::form_data(d, id as NodeId, None) } else { Vec::new() });
    let mut o = String::from("[");
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push('[');
        json_str(k, &mut o);
        o.push(',');
        json_str(v, &mut o);
        o.push(']');
    }
    o.push(']');
    Ok(JsValue::from(JsString::from(o)))
}

/// __rect(id) -> "x,y,w,h": the element's box relative to the window.
fn n_rect(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let ok = with_dom(|d| id >= 0 && (id as usize) < d.nodes.len());
    if !ok {
        return Ok(JsValue::from(js_string!("0,0,0,0")));
    }
    let r = with_dom(|d| crate::geometry::client_rect(d, id as NodeId));
    Ok(JsValue::from(JsString::from(format!("{},{},{},{}", r.x, r.y, r.w, r.h))))
}

/// __computed(id) -> JSON of the element's computed style, or "" if unavailable.
fn n_computed(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let ok = with_dom(|d| id >= 0 && (id as usize) < d.nodes.len());
    let s = if ok {
        // Shared borrows nest, so styling and layout may read the DOM again inside.
        with_dom(|d| crate::geometry::computed_style(d, id as NodeId)).unwrap_or_default()
    } else {
        String::new()
    };
    Ok(JsValue::from(JsString::from(s)))
}

/// __viewport() -> "width,height,scrollY,documentHeight"
fn n_viewport(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let (w, h, s, d) = crate::geometry::viewport();
    Ok(JsValue::from(JsString::from(format!("{w},{h},{s},{d}"))))
}

/// __scrollTo(y): scroll the page (in page pixels).
fn n_scroll_to(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let y = a.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0);
    if y.is_finite() {
        crate::geometry::request_scroll(y as f32);
    }
    Ok(JsValue::undefined())
}

/// __elScroll(id) -> "scrollTop,maxScroll,visibleHeight", or "" if it isn't a scrolling box.
fn n_el_scroll(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let s = if id >= 0 { crate::geometry::element_scroll_info(id as NodeId).map(|(t, m, h)| format!("{t},{m},{h}")).unwrap_or_default() } else { String::new() };
    Ok(JsValue::from(JsString::from(s)))
}

/// __setElScroll(id, y): scroll a box; the window lays the page out again.
fn n_set_el_scroll(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(a, 0, ctx)?;
    let y = a.get(1).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as f32;
    if id >= 0 && y.is_finite() {
        if let Some((_, max, _)) = crate::geometry::element_scroll_info(id as NodeId) {
            crate::layout::set_element_scroll(id as NodeId, y.clamp(0.0, max));
            with_dom_mut(|_| ()); // marks the page for a new layout
        }
    }
    Ok(JsValue::undefined())
}

/// Make this page's requests the ones new fetches are tagged with (its tab became active).
pub fn activate(js: &Js) {
    EPOCH.store(js.epoch, std::sync::atomic::Ordering::SeqCst);
}

/// A closed page's undelivered requests can be dropped.
pub fn forget(js: &Js) {
    if let Ok(mut q) = NET_DONE.lock() {
        q.retain(|d| d.0 != js.epoch);
    }
}

/// Take the page's DOM out (its tab is going to the background).
pub fn take_dom() -> Dom {
    bump_generation();
    DOM.with(|d| std::mem::replace(&mut *d.borrow_mut(), Dom::new()))
}

/// Pages that a script asked to open (window.open) during a click.
pub fn take_open_requests() -> Vec<String> {
    OPEN.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

/// __open(url): window.open — only while handling a click, so pages can't pop up windows on their own.
fn n_open(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let url = arg_str(a, 0, ctx)?;
    let sandboxed = page_ctx(ctx, |p| p.sandbox.as_ref().is_some_and(|s| !s.popups)).unwrap_or(false);
    let allowed = ACTIVATION.with(Cell::get) && !sandboxed;
    if allowed {
        OPEN.with(|o| o.borrow_mut().push(url));
    } else {
        log!("[popup blocked] {url}");
    }
    Ok(JsValue::from(allowed))
}

/// True while any background request is still running or waiting to be delivered.
pub fn network_busy() -> bool {
    NET_IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst) > 0 || NET_DONE.lock().map(|q| !q.is_empty()).unwrap_or(false)
}

fn n_history(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let kind = arg_str(a, 0, ctx)?;
    let url = arg_str(a, 1, ctx)?;
    let state = arg_str(a, 2, ctx)?;
    if matches!(kind.as_str(), "push" | "replace") {
        // pushState can only change the address within the page's own origin.
        let Some(page) = ctx.get_data::<RefCell<PageCtx>>() else { return Ok(JsValue::from(false)) };
        let mut page = page.borrow_mut();
        let base = |u: &str| if u.starts_with("http") { net::origin_of(u) } else { u.split(['?', '#']).next().unwrap_or(u).to_string() };
        if base(&url) != base(&page.url) || page.opaque() {
            log!("[security] {kind}State to another origin refused: {url}");
            return Ok(JsValue::from(false));
        }
        page.url = url.clone();
    }
    let op = match kind.as_str() {
        "push" => HistoryOp::Push(url, state),
        "replace" => HistoryOp::Replace(url, state),
        "hash" => HistoryOp::Hash(url, false),
        "hash-replace" => HistoryOp::Hash(url, true),
        _ => HistoryOp::Go(url.parse().unwrap_or(0)),
    };
    HISTORY.with(|h| h.borrow_mut().push(op));
    Ok(JsValue::undefined())
}

/// In the frame-renderer process (which can't write files) site storage lives in memory: an
/// embedded site gets working localStorage and IndexedDB for the session, kept apart per
/// top-level site.
static MEMORY_STORAGE: std::sync::Mutex<Option<HashMap<String, String>>> = std::sync::Mutex::new(None);

/// Keep site storage in memory only (for the frame process).
pub fn use_memory_storage() {
    if let Ok(mut m) = MEMORY_STORAGE.lock() {
        *m = Some(HashMap::new());
    }
}

/// A per-origin file under `<profile>/<dir>/`. `key` is the page's storage key (its origin,
/// plus the top-level site when it's embedded in another site).
fn store_file(key: &str, dir: &str) -> Option<std::path::PathBuf> {
    let name: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let name = if name.is_empty() { "file".to_string() } else { name };
    crate::cookies::profile_dir().map(|d| d.join(dir).join(format!("{name}.json")))
}

/// This page's saved store (`dir`: "storage" for localStorage, "indexeddb") as JSON text, or
/// "{}". The page's origin comes from the browser; any origin a script names is ignored.
fn load_store(ctx: &Context, dir: &str) -> String {
    let Some(key) = page_ctx(ctx, PageCtx::storage_key).flatten() else { return "{}".into() };
    if let Ok(Some(m)) = MEMORY_STORAGE.lock().as_deref() {
        return m.get(&format!("{dir}/{key}")).cloned().unwrap_or_else(|| "{}".into());
    }
    store_file(&key, dir).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_else(|| "{}".into())
}

/// __storageLoad() -> this page's localStorage as JSON text.
fn n_storage_load(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(JsString::from(load_store(ctx, "storage"))))
}

/// __storageSave(_, json): write it back. 5 MB per site.
fn n_storage_save(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let json = arg_str(a, 1, ctx)?;
    Ok(JsValue::from(save_store(ctx, "storage", json, 5 * 1024 * 1024)))
}

/// __idbLoad() -> this page's IndexedDB databases as JSON text, or "{}".
fn n_idb_load(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(JsString::from(load_store(ctx, "indexeddb"))))
}

/// __idbSave(_, json) -> false when over the 100 MB per-site quota or unwritable.
fn n_idb_save(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let json = arg_str(a, 1, ctx)?;
    Ok(JsValue::from(save_store(ctx, "indexeddb", json, 100 * 1024 * 1024)))
}

/// Save this page's store: atomically, readable only by you (or in memory in the frame process).
fn save_store(ctx: &Context, dir: &str, json: String, quota: usize) -> bool {
    if json.len() > quota {
        return false;
    }
    let Some(key) = page_ctx(ctx, PageCtx::storage_key).flatten() else { return false };
    if let Ok(Some(m)) = MEMORY_STORAGE.lock().as_deref_mut() {
        m.insert(format!("{dir}/{key}"), json);
        return true;
    }
    let Some(path) = store_file(&key, dir) else { return false };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, json).is_ok() && {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &path).is_ok()
    }
}

const PRELUDE: &str = include_str!("prelude.js");
const BLOB: &str = include_str!("blob.js");
const INDEXEDDB: &str = include_str!("indexeddb.js");

impl Js {
    pub fn new(url: &str) -> Js {
        let policy = crate::security::policy_for(url).unwrap_or_default();
        Js::with_policy(url, policy)
    }

    /// A script engine for the page at `url`, under the page's security policy.
    pub fn with_policy(url: &str, policy: std::sync::Arc<crate::security::PagePolicy>) -> Js {
        let epoch = EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let modules = std::rc::Rc::new(crate::jsmodules::Loader::default());
        let mut ctx = boa_engine::context::ContextBuilder::new()
            .host_hooks(std::rc::Rc::new(Hooks))
            .module_loader(modules.clone())
            .build()
            .expect("script engine");
        let frame = crate::security::frame_info(url);
        let sandbox = match (&frame.as_ref().and_then(|f| f.sandbox.clone()), &policy.sandbox) {
            (Some(a), Some(b)) => Some(a.and(b)),
            (a, b) => a.clone().or_else(|| b.clone()),
        };
        ctx.insert_data(RefCell::new(PageCtx { url: url.to_string(), top: frame.map_or_else(|| url.to_string(), |f| f.top), sandbox, policy, approved: Vec::new() }));
        // Keep runaway scripts from freezing the browser.
        ctx.runtime_limits_mut().set_loop_iteration_limit(200_000_000);
        ctx.runtime_limits_mut().set_recursion_limit(2_000);
        let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
            ("__type", 1, n_type),
            ("__attachShadow", 2, n_attach_shadow),
            ("__templateContent", 1, n_template_content),
            ("__idList", 0, n_id_list),
            ("__shadowRootList", 0, n_shadow_root_list),
            ("__shadowRoot", 2, n_shadow_root),
            ("__shadowHost", 1, n_shadow_host),
            ("__parent", 1, n_parent),
            ("__children", 1, n_children),
            ("__text", 1, n_text),
            ("__setText", 2, n_set_text),
            ("__insert", 3, n_insert),
            ("__detach", 1, n_detach),
            ("__clone", 2, n_clone),
            ("__tag", 1, n_tag),
            ("__getAttr", 2, n_get_attr),
            ("__setAttr", 3, n_set_attr),
            ("__removeAttr", 2, n_remove_attr),
            ("__attrNames", 1, n_attr_names),
            ("__getHTML", 2, n_get_html),
            ("__setHTML", 2, n_set_html),
            ("__query", 3, n_query),
            ("__matches", 2, n_matches),
            ("__byId", 1, n_by_id),
            ("__wrapList", 1, n_wrap_list),
            ("__create", 1, n_create),
            ("__createText", 1, n_create_text),
            ("__write", 1, n_write),
            ("__log", 2, n_log),
            ("__navigate", 1, n_navigate),
            ("__request", 3, n_request),
            ("__history", 3, n_history),
            ("__open", 1, n_open),
            ("__storageLoad", 1, n_storage_load),
            ("__storageSave", 2, n_storage_save),
            ("__idbLoad", 1, n_idb_load),
            ("__idbSave", 2, n_idb_save),
            ("__netStart", 8, n_net_start),
            ("__resolve", 2, n_resolve),
            ("__rect", 1, n_rect),
            ("__computed", 1, n_computed),
            ("__viewport", 0, n_viewport),
            ("__scrollTo", 1, n_scroll_to),
            ("__elScroll", 1, n_el_scroll),
            ("__setElScroll", 2, n_set_el_scroll),
            ("__formData", 1, n_form_data),
            ("__getCookie", 1, n_get_cookie),
            ("__setCookie", 2, n_set_cookie),
            ("__cspInline", 2, n_csp_inline),
            ("__pageUrl", 0, n_page_url),
            ("__permState", 1, n_perm_state),
            ("__permRequest", 2, n_perm_request),
            ("__notify", 2, n_notify),
            ("__fpSeed", 0, n_fp_seed),
        ];
        for (name, len, f) in natives {
            ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f))
                .expect("register native");
        }
        let esc = |v: &str| v.replace('\\', "\\\\").replace('"', "\\\"");
        let globals = format!(
            "var __nowMs = 0, __readyState = \"loading\", __url = \"{}\", __cookie = \"\", \
             __userAgent = \"{}\", __currentScript = -1;",
            esc(url),
            esc(net::USER_AGENT)
        );
        ctx.eval(Source::from_bytes(globals.as_bytes())).expect("globals");
        if let Err(e) = ctx.eval(Source::from_bytes(PRELUDE.as_bytes())) {
            log!("[ironswift] prelude failed: {e}");
        }
        if let Err(e) = ctx.eval(Source::from_bytes(BLOB.as_bytes())) {
            log!("[ironswift] Blob setup failed: {e}");
        }
        if let Err(e) = ctx.eval(Source::from_bytes(INDEXEDDB.as_bytes())) {
            log!("[ironswift] IndexedDB setup failed: {e}");
        }
        Js { ctx, start: Instant::now(), stats: Stats::default(), epoch, modules }
    }

    fn eval(&mut self, code: &str, label: &str) -> Option<JsValue> {
        // Boa's parser recurses once per nested bracket; a script nested thousands deep would
        // overflow the stack and take the whole browser down, so it's refused like a syntax error.
        if let Some(depth) = crate::jsnest::too_deep(code, MAX_SCRIPT_NESTING) {
            self.stats.errors += 1;
            log!("[js error] {label}: script nested too deeply ({depth} levels; limit {MAX_SCRIPT_NESTING})");
            return None;
        }
        let t = Instant::now();
        let r = self.ctx.eval(Source::from_bytes(code.as_bytes()));
        let _ = self.ctx.run_jobs();
        self.stats.script_time += t.elapsed();
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.stats.errors += 1;
                log!("[js error] {label}: {e}");
                None
            }
        }
    }

    /// Run code typed into the developer console and describe the result (or the error).
    pub fn run(&mut self, code: &str) -> Result<String, String> {
        let r = self.ctx.eval(Source::from_bytes(code.as_bytes()));
        let _ = self.ctx.run_jobs();
        match r {
            Ok(v) => {
                let shown = self.ctx.global_object().get(js_string!("__inspect"), &mut self.ctx).ok().and_then(|f| f.as_callable());
                let text = match shown {
                    Some(f) => f.call(&JsValue::undefined(), &[v.clone()], &mut self.ctx).ok().and_then(|s| s.as_string().map(|s| s.to_std_string_escaped())),
                    None => None,
                };
                Ok(text.unwrap_or_else(|| v.display().to_string()))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// The answer to permission requests `ids` (from the prompt, or automatic).
    pub fn permission_done(&mut self, ids: &[u64], granted: bool) {
        for id in ids {
            self.eval(&format!("__permDone({id}, {granted})"), "permission");
        }
    }

    /// Run every <script> in document order, then fire DOMContentLoaded/load.
    /// `prefetched` = external script bodies already downloaded, keyed by absolute URL.
    /// Run the document's scripts one after another (for documents not parsed here, such
    /// as XHTML), then the load events.
    pub fn run_in_order(&mut self, base_url: &str, prefetched: &HashMap<String, String>) {
        for id in with_dom(|d| d.find_all("script")) {
            self.run_script(id, base_url, prefetched);
        }
        self.finish_loading();
    }

    fn finish_loading(&mut self) {
        CURRENT_SCRIPT.with(|c| c.set(-1));
        self.eval("__namedIds()", "named ids");
        self.eval("__loaded()", "load events");
        self.tick(); // run zero-delay timers right away
    }

    /// Run one `<script>` element (inline or external). Non-JavaScript types are skipped.
    fn run_script(&mut self, id: NodeId, base_url: &str, prefetched: &HashMap<String, String>) {
        let (ty, src, inline) = with_dom(|d| {
            (d.attr(id, "type").unwrap_or("").to_ascii_lowercase(), d.attr(id, "src").map(String::from), d.text_content(id))
        });
        let is_module = ty.trim() == "module";
        let is_js = ty.is_empty() || ty.contains("javascript") || ty.contains("ecmascript");
        if !is_js && !is_module {
            return; // JSON, templates, import maps: not scripts
        }
        // `nomodule` scripts are fallbacks for browsers without modules; this one has them.
        if !is_module && with_dom(|d| d.attr(id, "nomodule").is_some()) {
            return;
        }
        let nonce = with_dom(|d| d.attr(id, "nonce").map(String::from));
        let policy = page_ctx(&self.ctx, |p| p.policy.clone());
        let (code, label) = match src {
            Some(src) => {
                let url = net::resolve(base_url, &src);
                if policy.as_ref().is_some_and(|p| !p.csp.allows_url(crate::security::Dest::Script, &url, nonce.as_deref(), true)) {
                    return;
                }
                let fetched = match prefetched.get(&url) {
                    Some(body) => Ok((url.clone(), body.clone())),
                    None => net::fetch_sub_nonce(&url, base_url, crate::security::Dest::Script, nonce.as_deref()),
                };
                match fetched {
                    Ok((_, body)) => (body, url),
                    Err(e) => {
                        log!("[js] couldn't load {url}: {e}");
                        return;
                    }
                }
            }
            None => {
                if policy.as_ref().is_some_and(|p| !p.csp.allows_inline(crate::security::Inline::Script, &inline, nonce.as_deref())) {
                    return;
                }
                (inline, format!("inline script #{id}"))
            }
        };
        CURRENT_SCRIPT.with(|c| c.set(id as i64));
        self.eval("__namedIds()", "named ids");
        self.ctx.global_object().set(js_string!("__currentScript"), JsValue::from(id as f64), false, &mut self.ctx).ok();
        self.stats.scripts += 1;
        if is_module {
            // An inline module's imports resolve against the page.
            let url = if label.starts_with("inline") { format!("{}#inline-{id}", base_url.split('#').next().unwrap_or(base_url)) } else { label.clone() };
            self.run_module(&code, &url, &label);
        } else {
            self.eval(&code, &label);
        }
    }

    /// Parse, load (with its imports), link and run a module.
    fn run_module(&mut self, code: &str, url: &str, label: &str) {
        let t = Instant::now();
        let module = match self.modules.parse(code, url, &mut self.ctx) {
            Ok(m) => m,
            Err(e) => {
                self.stats.errors += 1;
                log!("[js error] {label}: {e}");
                return;
            }
        };
        let promise = module.load_link_evaluate(&mut self.ctx);
        let _ = self.ctx.run_jobs();
        self.stats.script_time += t.elapsed();
        if let boa_engine::builtins::promise::PromiseState::Rejected(e) = promise.state() {
            self.stats.errors += 1;
            let msg = e.to_string(&mut self.ctx).map(|s| s.to_std_string_escaped()).unwrap_or_default();
            log!("[js error] {label}: {msg}");
        }
    }

    /// Parse the page and run its scripts as the parser reaches them, the way browsers do:
    /// a script sees only the document above it, and document.write adds markup right
    /// after it. `defer` and `async` scripts run once parsing is done.
    pub fn parse_and_run(&mut self, html: &str, base_url: &str, prefetched: &HashMap<String, String>) {
        let mut tb = crate::treebuilder::TreeBuilder::new(html);
        tb.pause_at_scripts = true;
        PARSER.with(|p| *p.borrow_mut() = Some(tb));
        let mut later: Vec<NodeId> = Vec::new();
        loop {
            let next = PARSER.with(|p| p.borrow_mut().as_mut().and_then(|tb| tb.pump()));
            let Some(script) = next else { break };
            // Hand the document to the scripts while one runs, then take it back.
            let dom = PARSER.with(|p| p.borrow_mut().as_mut().map(|tb| std::mem::take(&mut tb.dom))).unwrap_or_default();
            let (has_src, delayed) = (dom.attr(script, "src").is_some(), dom.attr(script, "defer").is_some() || dom.attr(script, "async").is_some());
            // Module scripts, inline or not, wait for the parser like deferred scripts.
            let module = dom.attr(script, "type").is_some_and(|t| t.trim().eq_ignore_ascii_case("module"));
            set_dom(dom);
            if (has_src && delayed) || module {
                later.push(script);
            } else {
                PARSER.with(|p| p.borrow_mut().as_mut().map(|tb| tb.begin_script()));
                self.run_script(script, base_url, prefetched);
                PARSER.with(|p| p.borrow_mut().as_mut().map(|tb| tb.end_script()));
            }
            let dom = DOM.with(|d| std::mem::take(&mut *d.borrow_mut()));
            PARSER.with(|p| p.borrow_mut().as_mut().map(|tb| tb.dom = dom));
        }
        let tb = PARSER.with(|p| p.borrow_mut().take()).expect("parser");
        set_dom(tb.finish());
        // Deferred scripts in document order (async ones are run here too).
        for id in later {
            self.run_script(id, base_url, prefetched);
        }
        self.finish_loading();
    }

    /// Wait (up to `max`) for background requests and short timers, so a page that
    /// loads its content with fetch() has it before a headless screenshot.
    pub fn settle(&mut self, max: std::time::Duration) {
        let t = Instant::now();
        loop {
            self.tick();
            if self.wants_frame() {
                self.run_frame();
            }
            // Also wait for one-off timers due within half a second (pages that fill in
            // content "a moment later").
            let soon = self
                .eval("__nextTimer()", "timers")
                .and_then(|v| v.to_number(&mut self.ctx).ok())
                .is_some_and(|ms| (0.0..=500.0).contains(&ms));
            if !network_busy() && !soon || t.elapsed() >= max {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.tick();
    }

    /// Advance timers to "now" and deliver finished requests. Returns true if anything ran.
    pub fn tick(&mut self) -> bool {
        // Take only this page's finished requests; other tabs' wait in the queue.
        let epoch = self.epoch;
        let done: Vec<_> = NET_DONE
            .lock()
            .map(|mut q| {
                let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q).into_iter().partition(|d| d.0 == epoch);
                *q = rest;
                mine
            })
            .unwrap_or_default();
        let delivered = !done.is_empty();
        for (_, id, r) in done {
            let code = format!("__netDone({id}, {})", response_js(&r));
            self.eval(&code, "network");
        }
        delivered | self.tick_timers()
    }

    /// Run due tasks (messages, timers) one at a time, with a microtask checkpoint after
    /// each. Stops after ~50 ms so a busy page can't freeze the window.
    fn tick_timers(&mut self) -> bool {
        let began = Instant::now();
        let mut ran = false;
        for _ in 0..1000 {
            let now = self.start.elapsed().as_secs_f64() * 1000.0;
            let did = self.eval(&format!("__runTask({now})"), "task").is_some_and(|v| v.to_number(&mut self.ctx).unwrap_or(0.0) > 0.0);
            if !did {
                break;
            }
            ran = true;
            if began.elapsed().as_millis() > 50 {
                break;
            }
        }
        ran
    }

    /// Does the page want an animation frame (a pending requestAnimationFrame)?
    pub fn wants_frame(&mut self) -> bool {
        self.eval("__hasFrames()", "frames").is_some_and(|v| v.to_boolean())
    }

    /// Run this frame's requestAnimationFrame callbacks (call once per drawn frame).
    pub fn run_frame(&mut self) -> bool {
        let now = self.start.elapsed().as_secs_f64() * 1000.0;
        self.eval(&format!("__runFrame({now})"), "animation frame").is_some_and(|v| v.to_number(&mut self.ctx).unwrap_or(0.0) > 0.0)
    }

    /// The browser moved within this document's history: update location, fire popstate/hashchange.
    pub fn history_moved(&mut self, url: &str, state_json: &str, popstate: bool) {
        let mut code = String::from("__historyMoved(");
        json_str(url, &mut code);
        code.push(',');
        json_str(state_json, &mut code);
        code.push_str(if popstate { ",true)" } else { ",false)" });
        self.eval(&code, "history");
    }

    /// Fire an event (input, change, submit, focus…) at a node. Returns false if cancelled.
    pub fn dispatch(&mut self, node: NodeId, kind: &str) -> bool {
        self.eval(&format!("__dispatch({node}, '{kind}')"), kind).map_or(true, |v| v.to_boolean())
    }

    /// Dispatch a click on a node. Returns false if the page called preventDefault().
    pub fn click(&mut self, node: NodeId) -> bool {
        ACTIVATION.with(|a| a.set(true));
        let r = self.eval(&format!("__dispatch({node}, 'click')"), "click").map_or(true, |v| v.to_boolean());
        ACTIVATION.with(|a| a.set(false));
        r
    }
}

/// Brackets a script may nest before it's refused (V8 gives up at a similar depth).
pub const MAX_SCRIPT_NESTING: usize = 1000;
