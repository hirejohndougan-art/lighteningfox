//! Background scripts: each extension's background runs on a thread of its own in the browser
//! process, with a full script engine (js.rs, as Web Workers get) plus `chrome.*` (api.js).
//!
//! Content scripts' messages, storage changes, tab events and webRequest requests arrive as
//! events (`api.__dispatch`); the thread sleeps until one comes or a timer is due.

use super::Extension;
use boa_engine::{Context, JsResult, JsString, JsValue};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// Running backgrounds by extension id.
static RUNNING: LazyLock<Mutex<HashMap<String, Sender<Value>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// `runtime.sendMessage` calls waiting for the background's answer.
static REPLIES: LazyLock<Mutex<HashMap<u64, Sender<Option<Value>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// On a background's thread: its extension id.
    static ME: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Is this thread an extension's background? (Its own requests aren't shown to webRequest.)
pub fn on_background_thread() -> bool {
    ME.with(|m| m.borrow().is_some())
}

pub fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::SeqCst)
}

/// Hand an event to an extension's background (dropped if it has none).
pub fn deliver_event(id: &str, event: &Value) -> bool {
    RUNNING.lock().ok().and_then(|r| r.get(id).map(|tx| tx.send(event.clone()).is_ok())).unwrap_or(false)
}

/// Every running background with this permission (or all, for "").
pub fn running() -> Vec<String> {
    RUNNING.lock().map(|r| r.keys().cloned().collect()).unwrap_or_default()
}

/// `runtime.sendMessage` from a content script in the page at `url`: `{"response": x}`, or
/// null when no listener answered.
pub fn send_message(e: &Extension, msg: Value, url: &str) -> Value {
    let id = next_id();
    let (tx, rx) = channel();
    if let Ok(mut r) = REPLIES.lock() {
        r.insert(id, tx);
    }
    let mut sender = json!({ "id": e.id, "url": url, "origin": crate::net::origin_of(url), "frameId": 0 });
    if let Some(tab) = super::tabs::tab_for_url(url, e) {
        sender["tab"] = tab;
    }
    let event = json!({ "type": "message", "id": id, "msg": msg, "sender": sender });
    let answer = if deliver_event(&e.id, &event) { rx.recv_timeout(Duration::from_secs(5)).ok().flatten() } else { None };
    if let Ok(mut r) = REPLIES.lock() {
        r.remove(&id);
    }
    answer.unwrap_or(Value::Null)
}

/// `__extCall(json)` on a background's thread.
fn n_call(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = a.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
    let req: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let Some(id) = ME.with(|m| m.borrow().clone()) else {
        return Ok(JsValue::from(JsString::from(r#"{"error":"not a background"}"#)));
    };
    let answer = match super::by_id(&id) {
        Some(e) => match handle(e, &req) {
            Ok(v) => json!({ "ok": v }),
            Err(m) => json!({ "error": m }),
        },
        None => json!({ "error": "extension gone" }),
    };
    Ok(JsValue::from(JsString::from(answer.to_string().as_str())))
}

fn handle(e: &Extension, req: &Value) -> Result<Value, String> {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let n = |k: &str| req.get(k).and_then(Value::as_u64).unwrap_or(0);
    match s("op").as_str() {
        "noop" => Ok(Value::Null),
        "storage" => {
            if !e.has("storage") && !e.has("unlimitedStorage") {
                return Err("the \"storage\" permission is needed".into());
            }
            super::storage(&e.id, &s("area"), &s("method"), req.get("arg").unwrap_or(&Value::Null))
        }
        "reply" => {
            if let Some(tx) = REPLIES.lock().ok().and_then(|mut r| r.remove(&n("id"))) {
                let _ = tx.send(req.get("response").cloned().filter(|v| !v.is_null()));
            }
            Ok(Value::Null)
        }
        "tabs.query" => Ok(super::tabs::query(e, req.get("query").unwrap_or(&Value::Null))),
        "tabs.get" => super::tabs::get(e, n("tabId")),
        "tabs.create" => super::tabs::create(e, req.get("props").unwrap_or(&Value::Null)),
        "tabs.update" => super::tabs::update(e, req.get("tabId").and_then(Value::as_u64), req.get("props").unwrap_or(&Value::Null)),
        "tabs.remove" => super::tabs::remove(req.get("tabIds").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()),
        "tabs.reload" => super::tabs::reload(req.get("tabId").and_then(Value::as_u64)),
        "tabs.sendMessage" => super::content::send_to_tab(e, n("tabId"), req.get("msg").cloned().unwrap_or(Value::Null)).map(Value::from),
        "webRequest.listen" => super::webrequest::listen(e, req.get("filter").unwrap_or(&Value::Null), req.get("blocking").and_then(Value::as_bool).unwrap_or(false)).map(Value::from),
        "webRequest.unlisten" => {
            super::webrequest::unlisten(&e.id, n("index"));
            Ok(Value::Null)
        }
        "webRequest.reply" => {
            super::webrequest::reply(n("id"), req.get("response").cloned().unwrap_or(Value::Null));
            Ok(Value::Null)
        }
        other => Err(format!("{other} isn't supported in the background")),
    }
}

fn js_str(s: &str) -> String {
    let mut o = String::new();
    crate::js::json_str(s, &mut o);
    o
}

/// Start an extension's background thread; returns once its scripts have run (or after two
/// seconds), so listeners it adds at startup see the first page's requests.
pub fn start(e: &'static Extension) {
    let (tx, rx) = channel::<Value>();
    if let Ok(mut r) = RUNNING.lock() {
        if r.contains_key(&e.id) {
            return; // already running
        }
        r.insert(e.id.clone(), tx);
    }
    let (ready_tx, ready_rx) = channel::<()>();
    let spawned = std::thread::Builder::new().name(format!("ext-{}", e.id)).stack_size(64 << 20).spawn(move || {
        ME.with(|m| *m.borrow_mut() = Some(e.id.clone()));
        let url = e.url("_generated_background_page.html");
        let mut js = crate::js::Js::for_worker(&url, &url, &url, std::sync::Arc::new(crate::security::PagePolicy::default()), "");
        js.add_native("__extCall", 1, n_call);
        let _ = js.run_code(super::API_JS, "extension API");
        let setup = format!(
            "globalThis.__extBg = __extMakeApi({}, \"background\", {}); globalThis.chrome = globalThis.browser = __extBg;",
            js_str(&e.id),
            e.manifest
        );
        let _ = js.run_code(&setup, "extension API");
        for script in &e.background {
            match e.read(script) {
                Some(code) => {
                    if let Err(err) = js.run_code(&code, &e.url(script)) {
                        log!("[extensions] {}: {script}: {err}", e.id);
                    }
                }
                None => log!("[extensions] {}: background script {script} not found", e.id),
            }
        }
        js.tick();
        let _ = ready_tx.send(());
        let _ = js.run_code("__extBg.__dispatch({type: \"installed\"})", "onInstalled");
        loop {
            let next = js.next_timer_ms();
            let mut wait = if next >= 0.0 { Duration::from_millis(next as u64) } else { Duration::from_millis(250) };
            if crate::js::network_busy() {
                wait = wait.min(Duration::from_millis(5));
            }
            super::content::expire();
            match rx.recv_timeout(wait.min(Duration::from_millis(250))) {
                Ok(ev) => {
                    let _ = js.run_code(&format!("__extBg.__dispatch({ev})"), "extension event");
                    js.tick();
                }
                Err(RecvTimeoutError::Timeout) => {
                    js.tick();
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    if spawned.is_ok() {
        let _ = ready_rx.recv_timeout(Duration::from_secs(2));
    } else if let Ok(mut r) = RUNNING.lock() {
        r.remove(&e.id);
    }
}
