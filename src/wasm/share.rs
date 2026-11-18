//! A `WebAssembly.Module` sent to another page or worker with `postMessage` (task P1c).
//!
//! wasmi keeps a module's translated code until its engine is dropped (its function table
//! only grows), so each page keeps its own engine and a process-wide one would never free
//! anything. A module therefore crosses as its bytes, shared (`Arc`, no copy), and is
//! compiled again by the receiver: wasmi validates up front and translates functions on their
//! first call, so that is quick (about 2 ms for a 246 KB module).
//!
//! `src/clone.js` calls `__wasmShare(module)` (a token that goes into the message) and the
//! receiver calls `__wasmAdopt(token)`. The last [`KEEP`] shared modules are kept for
//! receivers; a message read after that many newer modules were sent can't be decoded.

use super::{index_arg, state};
use boa_engine::{Context, JsNativeError, JsResult, JsValue};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// How many shared modules are kept for receivers.
const KEEP: usize = 64;

static SHARED: Mutex<VecDeque<(u64, Arc<[u8]>)>> = Mutex::new(VecDeque::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

/// __wasmShareNative(module id) -> token for `__wasmAdoptNative`.
pub(super) fn n_share(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = index_arg(a, 0, ctx)?;
    let st = state(ctx);
    let bytes = st.tables.borrow().modules.get(id).map(|m| m.bytes.clone());
    let Some(bytes) = bytes else {
        return Err(JsNativeError::typ().with_message("A WebAssembly.Module could not be cloned.").into());
    };
    let token = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut shared = SHARED.lock().map_err(|_| JsNativeError::error().with_message("WebAssembly: share failed"))?;
    if shared.len() >= KEEP {
        shared.pop_front();
    }
    shared.push_back((token, bytes));
    Ok(JsValue::from(token as f64))
}

/// __wasmAdoptNative(token) -> module id in this page or worker.
pub(super) fn n_adopt(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let token = a.first().cloned().unwrap_or_default().to_number(ctx)? as u64;
    let bytes = SHARED.lock().ok().and_then(|s| s.iter().find(|(t, _)| *t == token).map(|(_, b)| b.clone()));
    let Some(bytes) = bytes else {
        return Err(JsNativeError::typ().with_message("A WebAssembly.Module could not be cloned: it is no longer available.").into());
    };
    // Compiled already by the sender, so it isn't a compile from bytes for this page's CSP.
    Ok(super::compile_bytes(ctx, bytes)?.into())
}
