//! Content scripts: an extension's JavaScript and CSS in the pages its manifest matches.
//!
//! **Browser side**: `for_page` hands a page its scripts and styles; messages for content
//! scripts (`tabs.sendMessage`) and storage changes wait in a queue that pages poll.
//!
//! **Page side** (wherever the page runs): `start`, `run` and `poll` are called by the page's
//! script engine (js.rs) as it loads and while it runs. Each extension's scripts for one
//! `run_at` point run together, with the extension's `chrome` and `browser` objects in scope,
//! so the page's own scripts don't see them as globals. (The page's and the content scripts'
//! globals are otherwise shared: there is no separate "isolated world" yet.)

use super::Extension;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- browser side

/// Does any of the extension's content scripts run in the page at `url`?
pub fn runs_in(e: &Extension, url: &str) -> bool {
    e.content_scripts.iter().any(|c| matches(c, url))
}

fn matches(c: &super::ContentScript, url: &str) -> bool {
    c.matches.iter().any(|p| super::pattern::matches(p, url)) && !c.exclude_matches.iter().any(|p| super::pattern::matches(p, url))
}

/// Pages that ran content scripts, newest last (they stand in for tabs without a window).
static PAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn known_pages() -> Vec<String> {
    PAGES.lock().map(|p| p.clone()).unwrap_or_default()
}

/// Events waiting for content scripts: (sequence number, extension, page URL or every page, event).
struct Queue {
    seq: u64,
    events: VecDeque<(u64, String, Option<String>, Value)>,
}
static QUEUE: Mutex<Queue> = Mutex::new(Queue { seq: 0, events: VecDeque::new() });

/// `tabs.sendMessage` calls waiting for a content script: id -> (extension, give up at).
static PENDING: LazyLock<Mutex<HashMap<u64, (String, Instant)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// What a page gets: per extension, its scripts by `run_at` and its styles.
pub fn for_page(url: &str, top: bool) -> Value {
    let mut list = Vec::new();
    for e in super::all() {
        let entries: Vec<&super::ContentScript> = e.content_scripts.iter().filter(|c| (top || c.all_frames) && matches(c, url)).collect();
        if entries.is_empty() {
            continue;
        }
        let mut runs: Vec<(String, String)> = Vec::new(); // (run_at, code)
        let mut css = String::new();
        for c in &entries {
            for f in &c.css {
                match e.read(f) {
                    Some(t) => {
                        css.push_str(&t);
                        css.push('\n');
                    }
                    None => log!("[extensions] {}: {f} not found", e.id),
                }
            }
            let mut code = String::new();
            for f in &c.js {
                match e.read(f) {
                    Some(t) => {
                        code.push_str(&t);
                        code.push_str("\n;\n");
                    }
                    None => log!("[extensions] {}: {f} not found", e.id),
                }
            }
            if code.is_empty() {
                continue;
            }
            match runs.iter_mut().find(|(r, _)| *r == c.run_at) {
                Some((_, all)) => all.push_str(&code),
                None => runs.push((c.run_at.clone(), code)),
            }
        }
        list.push(json!({ "ext": e.id, "manifest": e.manifest, "runs": runs, "css": css }));
    }
    if !list.is_empty() {
        if let Ok(mut p) = PAGES.lock() {
            p.retain(|u| u != url);
            p.push(url.to_string());
            let n = p.len();
            if n > 50 {
                p.drain(..n - 50);
            }
        }
    }
    let seq = QUEUE.lock().map(|q| q.seq).unwrap_or(0);
    json!({ "extensions": list, "seq": seq })
}

/// Queue an event for an extension's content scripts (in one page, or all of them).
pub fn broadcast(ext: &str, url: Option<&str>, event: Value) {
    let Ok(mut q) = QUEUE.lock() else { return };
    q.seq += 1;
    let seq = q.seq;
    q.events.push_back((seq, ext.to_string(), url.map(String::from), event));
    while q.events.len() > 500 {
        q.events.pop_front();
    }
}

/// Events for the page at `url` after `seq`.
pub fn poll(url: &str, seq: u64) -> Value {
    let Ok(q) = QUEUE.lock() else { return json!({ "seq": seq, "events": [] }) };
    let events: Vec<Value> = q
        .events
        .iter()
        .filter(|(s, ext, target, _)| *s > seq && target.as_deref().is_none_or(|t| t == url) && super::by_id(ext).is_some_and(|e| runs_in(e, url)))
        .map(|(_, ext, _, ev)| json!({ "ext": ext, "event": ev }))
        .collect();
    json!({ "seq": q.seq, "events": events })
}

/// `tabs.sendMessage(tabId, msg)`: queue it for that tab's page; the answer comes back
/// through `reply`. Returns the id the background waits on.
pub fn send_to_tab(e: &Extension, tab_id: u64, msg: Value) -> Result<u64, String> {
    let url = super::tabs::url_of(tab_id).ok_or_else(|| format!("No tab with id: {tab_id}."))?;
    if !runs_in(e, &url) {
        return Err("Could not establish connection. Receiving end does not exist.".into());
    }
    let id = super::background::next_id();
    if let Ok(mut p) = PENDING.lock() {
        p.insert(id, (e.id.clone(), Instant::now() + Duration::from_secs(5)));
    }
    broadcast(&e.id, Some(&url), json!({ "type": "message", "id": id, "msg": msg, "sender": { "id": e.id } }));
    Ok(id)
}

/// A content script answered (`response` is `{"response": x}`, or null when none of its
/// listeners did).
pub fn reply(id: u64, response: Option<Value>) {
    let Some((ext, _)) = PENDING.lock().ok().and_then(|mut p| p.remove(&id)) else { return };
    let r = response.and_then(|r| r.get("response").cloned()).unwrap_or(Value::Null);
    super::background::deliver_event(&ext, &json!({ "type": "reply", "id": id, "response": r }));
}

/// Give up on `tabs.sendMessage` calls nobody answered.
pub fn expire() {
    let now = Instant::now();
    let gone: Vec<(u64, String)> = match PENDING.lock() {
        Ok(mut p) => {
            let ids: Vec<u64> = p.iter().filter(|(_, (_, t))| *t <= now).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| p.remove(&id).map(|(e, _)| (id, e))).collect()
        }
        Err(_) => Vec::new(),
    };
    for (id, ext) in gone {
        super::background::deliver_event(&ext, &json!({ "type": "reply", "id": id, "error": "Could not establish connection. Receiving end does not exist." }));
    }
}

// ---------------------------------------------------------------- page side

/// The last answer to `content`, so the stylesheet pass and the script engine ask once.
static LAST: Mutex<Option<(String, bool, Instant, Value)>> = Mutex::new(None);

fn scripts_for(url: &str, top: bool) -> Value {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")) {
        return json!({ "extensions": [], "seq": 0 });
    }
    if let Ok(l) = LAST.lock() {
        if let Some((u, t, at, v)) = l.as_ref() {
            if u == url && *t == top && at.elapsed() < Duration::from_secs(30) {
                return v.clone();
            }
        }
    }
    let r = super::call(&json!({ "op": "content", "url": url, "top": top }));
    let v = r.get("ok").cloned().unwrap_or_else(|| json!({ "extensions": [], "seq": 0 }));
    if let Ok(mut l) = LAST.lock() {
        *l = Some((url.to_string(), top, Instant::now(), v.clone()));
    }
    v
}

/// The extensions' styles for a page (added after the page's own sheets).
pub fn page_css(url: &str, top: bool) -> Vec<String> {
    scripts_for(url, top)
        .get("extensions")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|e| e.get("css").and_then(Value::as_str)).filter(|c| !c.trim().is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// A page's content scripts, kept beside its script engine.
pub struct PageState {
    url: String,
    exts: Vec<Value>,
    seq: u64,
    last_poll: Instant,
}

type State = RefCell<PageState>;

fn js_str(s: &str) -> String {
    let mut o = String::new();
    crate::js::json_str(s, &mut o);
    o
}

/// `__extCall(json)` in a page: everything goes to the browser.
fn n_call(_: &boa_engine::JsValue, a: &[boa_engine::JsValue], ctx: &mut boa_engine::Context) -> boa_engine::JsResult<boa_engine::JsValue> {
    let text = a.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
    let mut req: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let url = ctx.get_data::<State>().map(|s| s.borrow().url.clone()).unwrap_or_default();
    // The page says who it is; the browser checks the extension runs there.
    req["url"] = json!(url);
    let answer = match req.get("op").and_then(Value::as_str) {
        Some("noop") => json!({ "ok": null }),
        Some("storage" | "send" | "reply") => super::call(&req),
        Some(op) => json!({ "error": format!("{op} isn't available to content scripts") }),
        None => json!({ "error": "bad request" }),
    };
    Ok(boa_engine::JsValue::from(boa_engine::JsString::from(answer.to_string().as_str())))
}

/// As a page starts loading: find its content scripts, set up their APIs and run the
/// `document_start` ones.
pub fn start(js: &mut crate::js::Js) {
    let url = crate::js::page_url(js.ctx_mut());
    let top = crate::security::frame_info(&url).is_none();
    let found = scripts_for(&url, top);
    let exts: Vec<Value> = found.get("extensions").and_then(Value::as_array).cloned().unwrap_or_default();
    if exts.is_empty() {
        return;
    }
    let seq = found.get("seq").and_then(Value::as_u64).unwrap_or(0);
    js.add_native("__extCall", 1, n_call);
    let _ = js.run_code(super::API_JS, "extension API");
    let mut setup = String::from("Object.defineProperty(globalThis, \"__extContent\", { value: {}, enumerable: false, configurable: true });");
    for e in &exts {
        let id = e.get("ext").and_then(Value::as_str).unwrap_or("");
        let manifest = e.get("manifest").cloned().unwrap_or(Value::Null);
        setup.push_str(&format!("__extContent[{}] = __extMakeApi({}, \"content\", {});", js_str(id), js_str(id), manifest));
    }
    let _ = js.run_code(&setup, "extension API");
    js.ctx_mut().insert_data(RefCell::new(PageState { url, exts, seq, last_poll: Instant::now() }));
    run(js, "document_start");
}

/// Run the content scripts for one point in loading ("document_start", "document_end",
/// "document_idle").
pub fn run(js: &mut crate::js::Js, when: &str) {
    let Some(exts) = js.ctx_mut().get_data::<State>().map(|s| s.borrow().exts.clone()) else { return };
    for e in exts {
        let id = e.get("ext").and_then(Value::as_str).unwrap_or("");
        let Some(runs) = e.get("runs").and_then(Value::as_array) else { continue };
        for r in runs {
            if r.get(0).and_then(Value::as_str) != Some(when) {
                continue;
            }
            let code = r.get(1).and_then(Value::as_str).unwrap_or("");
            // `chrome` and `browser` are in scope for the script but aren't globals the page can
            // see; the script's own `var`s and functions are globals, shared with the
            // extension's other content scripts (as in one isolated world).
            let wrapped = format!(
                "with (Object.assign(Object.create(null), {{ chrome: __extContent[{0}], browser: __extContent[{0}] }})) {{\n{code}\n}}",
                js_str(id)
            );
            let _ = js.run_code(&wrapped, &format!("content script of {id}"));
        }
    }
}

/// Now and then while the page runs: deliver messages and storage changes.
pub fn poll_page(js: &mut crate::js::Js) {
    let Some((url, seq)) = js.ctx_mut().get_data::<State>().and_then(|s| {
        let mut s = s.borrow_mut();
        if s.last_poll.elapsed() < Duration::from_millis(100) {
            return None;
        }
        s.last_poll = Instant::now();
        Some((s.url.clone(), s.seq))
    }) else {
        return;
    };
    let r = super::call(&json!({ "op": "poll", "url": url, "seq": seq }));
    let Some(ok) = r.get("ok") else { return };
    if let Some(s) = js.ctx_mut().get_data::<State>() {
        s.borrow_mut().seq = ok.get("seq").and_then(Value::as_u64).unwrap_or(seq);
    }
    for ev in ok.get("events").and_then(Value::as_array).cloned().unwrap_or_default() {
        let id = ev.get("ext").and_then(Value::as_str).unwrap_or("");
        let event = ev.get("event").cloned().unwrap_or(Value::Null);
        let _ = js.run_code(&format!("__extContent[{}] && __extContent[{}].__dispatch({event})", js_str(id), js_str(id)), "extension event");
    }
}
