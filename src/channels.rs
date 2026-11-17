//! Script-side glue for Web Workers, WebSockets and the clipboard: the natives the page's
//! scripts call (`__workerStart`, `__wsOpen`, `__clipWrite`…) and the JavaScript classes
//! built on them (src/clone.js, src/channels.js, and src/worker_scope.js inside workers).

use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction, Source};

use crate::js::{arg_str, page_ctx};

const CLONE: &str = include_str!("clone.js");
const CHANNELS: &str = include_str!("channels.js");
const WORKER_SCOPE: &str = include_str!("worker_scope.js");

/// Add the natives and classes to a new script engine (run after the prelude).
pub fn install(ctx: &mut Context) {
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
        ("__workerStart", 4, n_worker_start),
        ("__workerPost", 2, n_worker_post),
        ("__workerTerminate", 1, n_worker_terminate),
        ("__wsOpen", 3, n_ws_open),
        ("__wsSend", 3, n_ws_send),
        ("__wsClose", 3, n_ws_close),
        ("__wsBuffered", 1, n_ws_buffered),
        ("__clipWrite", 1, n_clip_write),
        ("__clipRead", 0, n_clip_read),
        ("__wPost", 1, crate::worker::n_post),
        ("__wError", 1, crate::worker::n_error),
        ("__wClose", 0, crate::worker::n_close),
        ("__wImport", 1, crate::worker::n_import),
    ];
    for (name, len, f) in natives {
        let _ = ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f));
    }
    let worker = crate::worker::in_worker();
    if worker {
        let _ = ctx.eval(Source::from_bytes(b"globalThis.__isWorker = true;"));
    }
    for (name, code) in [("structured clone", CLONE), ("workers and sockets", CHANNELS)] {
        if let Err(e) = ctx.eval(Source::from_bytes(code.as_bytes())) {
            log!("[ironswift] {name} setup failed: {e}");
        }
    }
    if worker && let Err(e) = ctx.eval(Source::from_bytes(WORKER_SCOPE.as_bytes())) {
        log!("[ironswift] worker scope setup failed: {e}");
    }
}

/// Script calls waiting for the page with this epoch (worker messages, socket events).
pub fn take_events(epoch: u64) -> Vec<String> {
    let mut v = crate::worker::take_events(epoch);
    v.extend(crate::websocket::take_events(epoch));
    v.extend(crate::geo::take_events(epoch));
    v
}

pub fn busy() -> bool {
    crate::worker::busy() || crate::websocket::busy() || crate::geo::busy()
}

/// Should a headless render keep waiting? (A worker has a timer due soon.)
pub fn settling() -> bool {
    crate::worker::timer_soon()
}

/// The page is gone: stop its workers, close its sockets.
pub fn page_gone(epoch: u64) {
    crate::worker::page_gone(epoch);
    crate::websocket::page_gone(epoch);
    crate::geo::page_gone(epoch);
}

fn epoch(ctx: &Context) -> u64 {
    page_ctx(ctx, |p| p.epoch).unwrap_or(0)
}

fn num(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<f64> {
    a.get(i).map_or(Ok(f64::NAN), |v| v.to_number(ctx))
}

/// __workerStart(id, url, source|null, name) -> started?
fn n_worker_start(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx)? as u64;
    let url = arg_str(a, 1, ctx)?;
    let source = match a.get(2) {
        Some(v) if !v.is_null_or_undefined() => Some(v.to_string(ctx)?.to_std_string_escaped()),
        _ => None,
    };
    let name = arg_str(a, 3, ctx)?;
    let Some((page, top, policy, epoch)) = page_ctx(ctx, |p| (p.url.clone(), p.top.clone(), p.policy.clone(), p.epoch)) else {
        return Ok(JsValue::from(false));
    };
    // The page's own script rules decide where workers may come from.
    if !policy.csp.allows_url(crate::security::Dest::Script, &url, None, false) {
        log!("[worker] {url}: blocked by the page's Content Security Policy");
        return Ok(JsValue::from(false));
    }
    let same_origin = url.starts_with("blob:")
        || url.starts_with("data:")
        || crate::net::origin_of(&url) == crate::net::origin_of(&page) && crate::net::origin_of(&page) != "null"
        || page.starts_with("file:") && url.starts_with("file:") && crate::security::file_access_ok(&page, &url);
    if !same_origin {
        return Ok(JsValue::from(false));
    }
    let ok = crate::worker::start(crate::worker::Start { epoch, id, url, source, name, page, top, policy });
    Ok(JsValue::from(ok))
}

fn n_worker_post(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx)? as u64;
    let json = arg_str(a, 1, ctx)?;
    crate::worker::post(epoch(ctx), id, json);
    Ok(JsValue::undefined())
}

fn n_worker_terminate(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx)? as u64;
    crate::worker::terminate(epoch(ctx), id);
    Ok(JsValue::undefined())
}

/// __wsOpen(id, url, "proto1, proto2")
fn n_ws_open(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx)? as u64;
    let url = arg_str(a, 1, ctx)?;
    let protocols: Vec<String> = arg_str(a, 2, ctx)?.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    let Some(who) = page_ctx(ctx, |p| crate::websocket::Opener {
        epoch: p.epoch,
        page: p.net_page(),
        top: p.top.clone(),
        csp_ok: p.policy.csp.allows_url(crate::security::Dest::Connect, &url, None, false),
    }) else {
        return Ok(JsValue::undefined());
    };
    crate::websocket::open(who, id, url, protocols);
    Ok(JsValue::undefined())
}

/// __wsSend(id, data, binary): binary data comes base64-encoded.
fn n_ws_send(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx)? as u64;
    let data = a.get(1).cloned().unwrap_or_default().to_string(ctx)?;
    let binary = a.get(2).is_some_and(JsValue::to_boolean);
    let msg = if binary {
        use base64::Engine;
        let b = base64::engine::general_purpose::STANDARD.decode(data.to_std_string_escaped()).unwrap_or_default();
        crate::websocket::Out::Binary(b)
    } else {
        // Lone surrogates become U+FFFD, as the spec's UTF-8 encoding does.
        crate::websocket::Out::Text(data.to_std_string_lossy())
    };
    crate::websocket::send(epoch(ctx), id, msg);
    Ok(JsValue::undefined())
}

/// __wsClose(id, code (-1 = none), reason)
fn n_ws_close(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx)? as u64;
    let code = num(a, 1, ctx)?;
    let reason = arg_str(a, 2, ctx)?;
    let code = (code >= 0.0).then_some(code as u16);
    crate::websocket::send(epoch(ctx), id, crate::websocket::Out::Close(code, reason));
    Ok(JsValue::undefined())
}

fn n_ws_buffered(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx)? as u64;
    Ok(JsValue::from(crate::websocket::buffered(epoch(ctx), id) as f64))
}

/// __clipWrite(text|null) -> allowed? (null only checks, and starts the activation window).
fn n_clip_write(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let opaque = page_ctx(ctx, |p| p.opaque()).unwrap_or(true);
    if opaque || crate::worker::in_worker() || !crate::clipboard::may_write(crate::js::user_activated()) {
        return Ok(JsValue::from(false));
    }
    if let Some(v) = a.first().filter(|v| !v.is_null_or_undefined()) {
        let text = v.to_string(ctx)?.to_std_string_lossy();
        crate::clipboard::write(&text);
    }
    Ok(JsValue::from(true))
}

/// __clipRead() -> text, or null if not allowed.
fn n_clip_read(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let ok = page_ctx(ctx, |p| !p.opaque() && crate::permissions::state(&p.url, &p.top, crate::permissions::Kind::ClipboardRead) == "granted").unwrap_or(false);
    if !ok || crate::worker::in_worker() {
        return Ok(JsValue::null());
    }
    Ok(crate::clipboard::read().map_or(JsValue::null(), |t| JsValue::from(JsString::from(t))))
}
