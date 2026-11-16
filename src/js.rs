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

thread_local! {
    static DOM: RefCell<Dom> = RefCell::new(Dom::new());
    static DIRTY: Cell<bool> = const { Cell::new(false) };
    static NAVIGATE: RefCell<Option<String>> = const { RefCell::new(None) };
    static CURRENT_SCRIPT: Cell<i64> = const { Cell::new(-1) };
}

pub fn set_dom(dom: Dom) {
    DOM.with(|d| *d.borrow_mut() = dom);
}

pub fn with_dom<R>(f: impl FnOnce(&Dom) -> R) -> R {
    DOM.with(|d| f(&d.borrow()))
}

fn with_dom_mut<R>(f: impl FnOnce(&mut Dom) -> R) -> R {
    DIRTY.with(|d| d.set(true));
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
    html::parse(markup)
}

fn clone_node(dom: &mut Dom, id: NodeId, deep: bool) -> NodeId {
    let new = dom.create(dom.nodes[id].data.clone());
    if deep {
        for c in dom.nodes[id].children.clone() {
            let k = clone_node(dom, c, true);
            dom.insert(new, k, None);
        }
    }
    new
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
        })
    })))
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
    let sels = css::parse_selector_list(sel);
    if sels.is_empty() {
        return vec![];
    }
    with_dom(|d| {
        let mut out = Vec::new();
        for n in d.descendants(root) {
            if d.tag(n).is_some() && sels.iter().any(|s| style::matches(d, n, s)) {
                out.push(n);
                if !all {
                    break;
                }
            }
        }
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
    let sels = css::parse_selector_list(&sel);
    let m = valid(id).map_or(false, |id| with_dom(|d| sels.iter().any(|s| style::matches(d, id, s))));
    Ok(JsValue::from(m))
}

fn n_by_id(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let want = arg_str(a, 0, ctx)?;
    let found = with_dom(|d| d.descendants(Dom::ROOT).into_iter().find(|&n| d.attr(n, "id") == Some(want.as_str())));
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
    eprintln!("[js {level}] {msg}");
    Ok(JsValue::undefined())
}

fn n_navigate(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let url = arg_str(a, 0, ctx)?;
    NAVIGATE.with(|n| *n.borrow_mut() = Some(url));
    Ok(JsValue::undefined())
}

const PRELUDE: &str = include_str!("prelude.js");

impl Js {
    pub fn new(url: &str) -> Js {
        let mut ctx = Context::default();
        // Keep runaway scripts from freezing the browser.
        ctx.runtime_limits_mut().set_loop_iteration_limit(200_000_000);
        ctx.runtime_limits_mut().set_recursion_limit(2_000);
        let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
            ("__type", 1, n_type),
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
            ("__create", 1, n_create),
            ("__createText", 1, n_create_text),
            ("__write", 1, n_write),
            ("__log", 2, n_log),
            ("__navigate", 1, n_navigate),
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
            eprintln!("[lighteningfox] prelude failed: {e}");
        }
        Js { ctx, start: Instant::now(), stats: Stats::default() }
    }

    fn eval(&mut self, code: &str, label: &str) -> Option<JsValue> {
        let t = Instant::now();
        let r = self.ctx.eval(Source::from_bytes(code.as_bytes()));
        let _ = self.ctx.run_jobs();
        self.stats.script_time += t.elapsed();
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.stats.errors += 1;
                eprintln!("[js error] {label}: {e}");
                None
            }
        }
    }

    /// Run every <script> in document order, then fire DOMContentLoaded/load.
    /// `prefetched` = external script bodies already downloaded, keyed by absolute URL.
    pub fn run_scripts(&mut self, base_url: &str, prefetched: &HashMap<String, String>) {
        let scripts: Vec<NodeId> = with_dom(|d| d.find_all("script"));
        for id in scripts {
            let (ty, src, inline) = with_dom(|d| {
                (
                    d.attr(id, "type").unwrap_or("").to_ascii_lowercase(),
                    d.attr(id, "src").map(String::from),
                    d.text_content(id),
                )
            });
            let is_js = ty.is_empty() || ty.contains("javascript") || ty.contains("ecmascript");
            if !is_js {
                continue; // modules, JSON, templates: not yet
            }
            let (code, label) = match src {
                Some(src) => {
                    let url = net::resolve(base_url, &src);
                    let fetched = match prefetched.get(&url) {
                        Some(body) => Ok((url.clone(), body.clone())),
                        None => net::fetch(&url),
                    };
                    match fetched {
                        Ok((_, body)) => (body, url),
                        Err(e) => {
                            eprintln!("[js] couldn't load {url}: {e}");
                            continue;
                        }
                    }
                }
                None => (inline, format!("inline script #{id}")),
            };
            CURRENT_SCRIPT.with(|c| c.set(id as i64));
            self.ctx
                .global_object()
                .set(js_string!("__currentScript"), JsValue::from(id as f64), false, &mut self.ctx)
                .ok();
            self.stats.scripts += 1;
            self.eval(&code, &label);
        }
        CURRENT_SCRIPT.with(|c| c.set(-1));
        self.eval("__loaded()", "load events");
        self.tick(); // run zero-delay timers right away
    }

    /// Advance timers to "now". Returns true if any ran.
    pub fn tick(&mut self) -> bool {
        let now = self.start.elapsed().as_secs_f64() * 1000.0;
        self.eval(&format!("__tick({now})"), "timers")
            .map_or(false, |v| v.to_number(&mut self.ctx).unwrap_or(0.0) > 0.0)
    }

    /// Dispatch a click on a node. Returns false if the page called preventDefault().
    pub fn click(&mut self, node: NodeId) -> bool {
        self.eval(&format!("__dispatch({node}, 'click')"), "click").map_or(true, |v| v.to_boolean())
    }
}
