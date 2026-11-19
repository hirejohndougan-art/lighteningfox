//! Web Workers: `new Worker(url)` runs a script on its own thread, with its own script
//! engine, talking to the page only through postMessage.
//!
//! A worker gets a full Ironswift script engine (src/js.rs) on its thread, so fetch,
//! XMLHttpRequest, timers, Blob, TextEncoder, WebSocket and nested workers all work there;
//! src/worker_scope.js then removes what only a document has (document, window,
//! localStorage…) and adds the worker's own globals (postMessage, importScripts, close).
//! Messages cross threads as structured-clone strings (src/clone.js).

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

enum In {
    Message(String),
    Terminate,
}

struct Handle {
    tx: Sender<In>,
    /// Messages sent to the worker that it hasn't handled yet (+1 while it starts up).
    pending: Arc<AtomicUsize>,
    /// The worker has a timer due soon.
    active: Arc<AtomicBool>,
}

/// Running workers by (parent page epoch, worker id).
static WORKERS: LazyLock<Mutex<HashMap<(u64, u64), Handle>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Script calls waiting for the parent page: (parent epoch, code).
static EVENTS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

thread_local! {
    /// On a worker's thread: (parent epoch, worker id).
    static SELF: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
    static CLOSING: Cell<bool> = const { Cell::new(false) };
}

/// Is this thread a worker's?
pub fn in_worker() -> bool {
    SELF.with(Cell::get).is_some()
}

fn push(epoch: u64, code: String) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push((epoch, code));
    }
}

fn js_str(s: &str) -> String {
    let mut o = String::new();
    crate::js::json_str(s, &mut o);
    o
}

pub fn take_events(epoch: u64) -> Vec<String> {
    let Ok(mut q) = EVENTS.lock() else { return Vec::new() };
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q).into_iter().partition(|e| e.0 == epoch);
    *q = rest;
    mine.into_iter().map(|e| e.1).collect()
}

/// Is any worker still working (or has a message for its page)?
pub fn busy() -> bool {
    EVENTS.lock().is_ok_and(|q| !q.is_empty()) || WORKERS.lock().is_ok_and(|w| w.values().any(|h| h.pending.load(Ordering::SeqCst) > 0))
}

/// Does a worker have a timer due within half a second?
pub fn timer_soon() -> bool {
    WORKERS.lock().is_ok_and(|w| w.values().any(|h| h.active.load(Ordering::SeqCst)))
}

/// Stop every worker a page started.
pub fn page_gone(epoch: u64) {
    if let Ok(mut w) = WORKERS.lock() {
        w.retain(|k, h| {
            if k.0 == epoch {
                let _ = h.tx.send(In::Terminate);
                false
            } else {
                true
            }
        });
    }
    if let Ok(mut q) = EVENTS.lock() {
        q.retain(|e| e.0 != epoch);
    }
}

pub fn post(epoch: u64, id: u64, json: String) {
    if let Ok(w) = WORKERS.lock()
        && let Some(h) = w.get(&(epoch, id))
    {
        h.pending.fetch_add(1, Ordering::SeqCst);
        if h.tx.send(In::Message(json)).is_err() {
            h.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub fn terminate(epoch: u64, id: u64) {
    if let Ok(mut w) = WORKERS.lock()
        && let Some(h) = w.remove(&(epoch, id))
    {
        let _ = h.tx.send(In::Terminate);
    }
}

/// Who starts the worker, and where its script is.
pub struct Start {
    pub epoch: u64,
    pub id: u64,
    /// The worker's script URL (http(s), data: or blob:).
    pub url: String,
    /// The script itself, when the page already has it (a blob: URL).
    pub source: Option<String>,
    pub name: String,
    /// The creating page: its URL (blob: workers share its origin) and top-level page.
    pub page: String,
    pub top: String,
    pub policy: Arc<crate::security::PagePolicy>,
    /// `new Worker(url, {type: "module"})`: the script is an ES module (imports, top-level
    /// await) and importScripts() throws.
    pub module: bool,
}

pub fn start(s: Start) -> bool {
    let (tx, rx) = mpsc::channel::<In>();
    let pending = Arc::new(AtomicUsize::new(1));
    let active = Arc::new(AtomicBool::new(false));
    if let Ok(mut w) = WORKERS.lock() {
        w.insert((s.epoch, s.id), Handle { tx, pending: pending.clone(), active: active.clone() });
    }
    let spawned = std::thread::Builder::new().name("worker".into()).stack_size(64 << 20).spawn(move || {
        let (epoch, id) = (s.epoch, s.id);
        SELF.with(|c| c.set(Some((epoch, id))));
        let error = |msg: &str| push(epoch, format!("__workerEvent({id},\"error\",{},{},0)", js_str(msg), js_str(&s.url)));
        // A script that couldn't be loaded or parsed gets a plain "error" Event.
        let load_error = || push(epoch, format!("__workerEvent({id},\"loaderror\")"));
        // A blob: worker belongs to the page that made it; others to where they came from.
        let own_url = if s.url.starts_with("blob:") { s.page.clone() } else { s.url.clone() };
        let source = match s.source.clone() {
            Some(src) => Ok(src),
            None => fetch_script(&s.url, &s.page),
        };
        let mut js = crate::js::Js::for_worker(&own_url, &s.url, &s.top, s.policy.clone(), &s.name);
        match source {
            Ok(src) if s.module => {
                js.run_code("globalThis.importScripts = function importScripts() { throw new TypeError(\"importScripts() can't be used in a module worker\"); };", "module worker").ok();
                if let Err(e) = js.run_module_script(&src, &s.url) {
                    error(&format!("Uncaught {e}"));
                }
            }
            Ok(src) => {
                // A script that doesn't parse never starts; one that throws while running does.
                if let Some(e) = js.syntax_error(&src) {
                    log!("[worker] {}: {e}", s.url);
                    load_error();
                } else if let Err(e) = js.run_code(&src, &s.url) {
                    error(&format!("Uncaught {e}"));
                }
            }
            Err(e) => {
                log!("[worker] couldn't load {}: {e}", s.url);
                load_error();
            }
        }
        if !CLOSING.with(Cell::get) {
            js.tick();
        }
        pending.fetch_sub(1, Ordering::SeqCst);
        while !CLOSING.with(Cell::get) {
            // Sleep until the next timer, a message, or (while requests are out) a short poll.
            let next = js.next_timer_ms();
            active.store((0.0..=500.0).contains(&next), Ordering::SeqCst);
            let mut wait = if next >= 0.0 { Duration::from_millis(next as u64) } else { Duration::from_millis(250) };
            if crate::js::network_busy() {
                wait = wait.min(Duration::from_millis(5));
            }
            match rx.recv_timeout(wait) {
                Ok(In::Message(json)) => {
                    js.deliver_message(&json);
                    // After close(), nothing else runs (not even timers already due).
                    if !CLOSING.with(Cell::get) {
                        js.tick();
                    }
                    pending.fetch_sub(1, Ordering::SeqCst);
                }
                Ok(In::Terminate) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if !CLOSING.with(Cell::get) {
                        js.tick();
                    }
                }
            }
        }
        active.store(false, Ordering::SeqCst);
        // Drain what's left so the page stops waiting on it.
        while let Ok(m) = rx.try_recv() {
            if matches!(m, In::Message(_)) {
                pending.fetch_sub(1, Ordering::SeqCst);
            }
        }
        push(epoch, format!("__workerEvent({id},\"exit\")"));
        if let Ok(mut w) = WORKERS.lock() {
            w.remove(&(epoch, id));
        }
    });
    spawned.is_ok()
}

/// Fetch a worker script or an importScripts() script: it must load (2xx) and, from a web
/// server, be served as JavaScript.
fn fetch_script(url: &str, page: &str) -> Result<String, String> {
    let r = crate::net::script_request(page, "GET", url, &[], None, "same-origin")?;
    if !(200..300).contains(&r.status) {
        return Err(format!("{url}: HTTP {}", r.status));
    }
    if url.starts_with("http") {
        let ct = r.header("content-type").unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let js = ct.is_empty() || ct.ends_with("javascript") || ct.ends_with("ecmascript") || ct == "text/jscript" || ct == "text/livescript";
        if !js {
            return Err(format!("{url}: served as {ct}, not JavaScript"));
        }
    }
    Ok(String::from_utf8_lossy(&r.body).into_owned())
}

// ------------------------------------------------------------ natives inside the worker

use boa_engine::{Context, JsResult, JsValue};

fn me() -> Option<(u64, u64)> {
    SELF.with(Cell::get)
}

/// __wPost(json): postMessage from the worker to its page.
pub fn n_post(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let json = crate::js::arg_str(a, 0, ctx)?;
    // Messages posted after close() are still delivered (the port stays open until then).
    if let Some((epoch, id)) = me() {
        push(epoch, format!("__workerEvent({id},\"message\",{})", js_str(&json)));
    }
    Ok(JsValue::undefined())
}

/// __wError(message): an uncaught error in one of the worker's event handlers.
pub fn n_error(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let msg = crate::js::arg_str(a, 0, ctx)?;
    if let Some((epoch, id)) = me() {
        push(epoch, format!("__workerEvent({id},\"error\",{},\"\",0)", js_str(&msg)));
    }
    Ok(JsValue::undefined())
}

/// __wClose(): the worker's close().
pub fn n_close(_: &JsValue, _a: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    CLOSING.with(|c| c.set(true));
    Ok(JsValue::undefined())
}

/// __wImport(url): importScripts — fetch a script and run it now, in the worker's scope.
/// Returns "" when it ran, or why it couldn't be loaded (the script side throws a
/// NetworkError); errors thrown by the script itself propagate.
pub fn n_import(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let url = crate::js::arg_str(a, 0, ctx)?;
    let (page, csp_ok) = crate::js::page_ctx(ctx, |p| (p.url.clone(), p.policy.csp.allows_url(crate::security::Dest::Script, &url, None, false))).unwrap_or_default();
    let fail = |m: String| Ok(JsValue::from(boa_engine::JsString::from(m)));
    if !csp_ok {
        return fail(format!("{url} is blocked by the Content Security Policy"));
    }
    if url.starts_with("blob:") {
        return fail("importScripts of blob: URLs isn't supported yet".into());
    }
    let body = match fetch_script(&url, &page) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    ctx.eval(boa_engine::Source::from_bytes(body.as_bytes()))?;
    Ok(JsValue::from(boa_engine::JsString::from("")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn wait(epoch: u64, what: &str) -> Vec<String> {
        let t = Instant::now();
        let mut seen = Vec::new();
        while t.elapsed() < Duration::from_secs(10) {
            seen.extend(take_events(epoch));
            if seen.iter().any(|e| e.contains(what)) {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never saw {what}: {seen:?}");
    }

    fn start_with(epoch: u64, src: &str) {
        start_kind(epoch, src, false);
    }

    fn start_kind(epoch: u64, src: &str, module: bool) {
        assert!(start(Start {
            epoch,
            id: 1,
            url: "blob:null/test".into(),
            source: Some(src.into()),
            name: "t".into(),
            page: "about:blank".into(),
            top: "about:blank".into(),
            policy: Default::default(),
            module,
        }));
    }

    #[test]
    fn module_worker() {
        // ffmpeg.wasm (2026-11) starts its worker with {type: "module"}: top-level await
        // works, and importScripts throws so the script falls back to import().
        let epoch = 800_009;
        start_kind(epoch, "const x = await Promise.resolve(4); let how; try { importScripts('x.js'); how = 'classic'; } catch (e) { how = e.name; } postMessage('module ' + x + ' ' + how);", true);
        let ev = wait(epoch, "\"message\"");
        assert!(ev.iter().any(|e| e.contains("module 4 TypeError")), "{ev:?}");
    }

    #[test]
    fn runs_on_its_own_thread_and_echoes() {
        let epoch = 800_001;
        start_with(epoch, "onmessage = (e) => postMessage({ got: e.data, doubled: e.data.n * 2, d: new Date(5), m: new Map([[1, 2]]) });");
        // The page side encodes with __cloneOut; a plain JSON number tree is the same thing.
        post(epoch, 1, "{\"ports\":[],\"m\":{\"$\":\"o\",\"i\":0,\"v\":{\"n\":21}}}".into());
        let ev = wait(epoch, "\"message\"");
        let m = ev.iter().find(|e| e.contains("\"message\"")).unwrap();
        assert!(m.contains("doubled") && m.contains("42"), "{m}");
        assert!(m.contains(r#"\"$\":\"d\""#) && m.contains(r#"\"$\":\"m\""#), "dates and maps survive: {m}");
        terminate(epoch, 1);
        wait(epoch, "\"exit\"");
    }

    #[test]
    fn top_level_let_and_const() {
        // The syntax check before running must not leave the names declared.
        let epoch = 800_005;
        start_with(epoch, "const a = 2; let b = 3; class C {} postMessage('sum ' + (a + b));");
        let ev = wait(epoch, "\"message\"");
        assert!(ev.iter().any(|e| e.contains("sum 5")), "{ev:?}");
    }

    #[test]
    fn worker_scope_has_no_document() {
        let epoch = 800_002;
        start_with(epoch, "postMessage([typeof document, typeof window, typeof self, typeof importScripts, typeof fetch, self === globalThis, typeof WorkerGlobalScope, name].join(','));");
        let ev = wait(epoch, "\"message\"");
        assert!(ev.iter().any(|e| e.contains(r#"undefined,undefined,object,function,function,true,function,t"#)), "{ev:?}");
    }

    #[test]
    fn offscreen_canvas_works_in_a_worker() {
        // It made its hidden canvas with `document`, which a worker hasn't got.
        let epoch = 800_010;
        start_with(epoch, "const c = new OffscreenCanvas(4, 4); const x = c.getContext('2d'); x.fillStyle = 'rgb(255,0,0)'; x.fillRect(0, 0, 4, 4); postMessage('px:' + Array.from(x.getImageData(1, 1, 1, 1).data).join('-'));");
        let ev = wait(epoch, "\"message\"");
        assert!(ev.iter().any(|e| e.contains("px:255-0-0-255")), "{ev:?}");
    }

    #[test]
    fn errors_reach_the_page() {
        let epoch = 800_003;
        start_with(epoch, "onmessage = () => { throw new Error('boom'); };");
        post(epoch, 1, "{\"ports\":[],\"m\":1}".into());
        let ev = wait(epoch, "\"error\"");
        assert!(ev.iter().any(|e| e.contains("boom")), "{ev:?}");
    }

    #[test]
    fn timers_and_close() {
        let epoch = 800_004;
        start_with(epoch, "setTimeout(() => postMessage('never'), 0); setTimeout(() => { postMessage('later'); close(); }, 30); setInterval(() => postMessage('tick'), 5);");
        let ev = wait(epoch, "\"exit\"");
        assert!(ev.iter().any(|e| e.contains("later")) && ev.iter().any(|e| e.contains("tick")), "{ev:?}");
        start_with(epoch + 10, "setTimeout(() => postMessage('never'), 0); close();");
        let ev = wait(epoch + 10, "\"exit\"");
        assert!(!ev.iter().any(|e| e.contains("never")), "timers don't run after close(): {ev:?}");
    }
}
