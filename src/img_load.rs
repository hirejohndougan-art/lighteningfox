//! Images scripts load themselves: `new Image()` and `<img>` elements whose `src` a script
//! sets, in the document or not. Each loads on a background thread (as an image, so no CORS
//! check, like other browsers); `img.js` polls for results and fires `load` or `error`.
//! Loaded pixels go to the canvas store, where drawImage and WebGL's texImage2D find them.

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::sync::mpsc::{channel, Receiver, Sender};

/// (token, source key, pixels) for each finished load.
type Done = (u32, String, Option<image::RgbaImage>);

thread_local! {
    static LOADS: RefCell<Option<(Sender<Done>, Receiver<Done>)>> = const { RefCell::new(None) };
    static IN_FLIGHT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// At most this many script image loads at once per page (the rest wait their turn in JS).
pub const MAX_LOADS: usize = 16;

pub fn install(ctx: &mut Context) {
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[("__imgStart", 3, n_start), ("__imgPoll", 0, n_poll)];
    for (name, len, f) in natives {
        ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f)).expect("register image native");
    }
    if let Err(e) = ctx.eval(Source::from_bytes(include_str!("img.js").as_bytes())) {
        log!("[ironswift] image loading setup failed: {e}");
    }
}

/// __imgStart(token, url, nodeId) -> false when too many loads are running (try again later).
fn n_start(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let token = a.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as u32;
    let src = a.get(1).map(|v| v.to_string(ctx)).transpose()?.map(|s| s.to_std_string_escaped()).unwrap_or_default();
    // The element's chosen source (srcset/<picture> aware): the key canvas.rs looks up.
    let node = a.get(2).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(-1.0);
    let key = if node >= 0.0 {
        let id = node as crate::dom::NodeId;
        crate::js::with_dom(|d| (id < d.nodes.len()).then(|| crate::srcset::current_src(d, id)).flatten()).unwrap_or_else(|| src.clone())
    } else {
        src.clone()
    };
    let src = key.clone();
    if IN_FLIGHT.with(|n| n.get()) >= MAX_LOADS {
        return Ok(JsValue::from(false));
    }
    let page = crate::js::page_url(ctx);
    let url = crate::net::resolve(&page, src.trim());
    let tx = LOADS.with(|l| {
        let mut l = l.borrow_mut();
        l.get_or_insert_with(channel).0.clone()
    });
    IN_FLIGHT.with(|n| n.set(n.get() + 1));
    let spawned = std::thread::Builder::new().name("script-image".into()).spawn(move || {
        let img = if url.is_empty() { None } else { crate::images::fetch_pixels(&url, &page) };
        let _ = tx.send((token, key, img));
    });
    if spawned.is_err() {
        IN_FLIGHT.with(|n| n.set(n.get().saturating_sub(1)));
        return Ok(JsValue::from(false));
    }
    Ok(JsValue::from(true))
}

/// __imgPoll() -> [[token, ok, width, height], …] for loads that finished (and how many are
/// still running, as a last element [-1, running]).
fn n_poll(_: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let done: Vec<Done> = LOADS.with(|l| l.borrow().as_ref().map(|(_, rx)| rx.try_iter().collect()).unwrap_or_default());
    IN_FLIGHT.with(|n| n.set(n.get().saturating_sub(done.len())));
    let mut rows = Vec::new();
    for (token, key, img) in done {
        let (ok, w, h) = match &img {
            Some(i) => {
                crate::canvas::add_loaded(ctx, &key, i);
                (true, i.width(), i.height())
            }
            None => (false, 0, 0),
        };
        rows.push(JsArray::from_iter([JsValue::from(token), JsValue::from(ok), JsValue::from(w), JsValue::from(h)], ctx).into());
    }
    let running = IN_FLIGHT.with(|n| n.get());
    rows.push(JsArray::from_iter([JsValue::from(-1), JsValue::from(running as u32)], ctx).into());
    Ok(JsArray::from_iter(rows, ctx).into())
}
