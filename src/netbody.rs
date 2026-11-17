//! Binary bodies for fetch() and XMLHttpRequest.
//!
//! A response body that is valid UTF-8 travels to the page as a string (fast for HTML, JSON
//! and text). Any other body (images, fonts, wasm, latin-1 text) is parked here and handed to
//! the page as an ArrayBuffer through `__netBody(key)`, so its bytes arrive unchanged.
//! Request bodies made of bytes (ArrayBuffer, typed arrays, Blob) come in as a Uint8Array.

use boa_engine::object::builtins::{AlignedVec, JsArrayBuffer, JsUint8Array};
use boa_engine::{Context, JsResult, JsValue};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Bodies waiting for the page to collect them: key -> bytes. Collected right after the
    /// result is delivered, so this stays small.
    static BODIES: RefCell<(u64, HashMap<u64, Vec<u8>>)> = RefCell::new((0, HashMap::new()));
}

/// Keep `bytes` for the page and return the key it collects them with.
pub fn park(bytes: Vec<u8>) -> u64 {
    BODIES.with(|b| {
        let mut b = b.borrow_mut();
        b.0 += 1;
        let key = b.0;
        // A page that never collects (it was torn down mid-delivery) shouldn't grow this forever.
        if b.1.len() > 256 {
            b.1.clear();
        }
        b.1.insert(key, bytes);
        key
    })
}

/// __netBody(key) -> ArrayBuffer of a parked body (empty if it was already taken).
pub fn n_net_body(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let key = a.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as u64;
    let bytes = BODIES.with(|b| b.borrow_mut().1.remove(&key)).unwrap_or_default();
    let block = AlignedVec::from_iter(0, bytes);
    Ok(JsArrayBuffer::from_byte_block(block, ctx)?.into())
}

/// A request body from script: a Uint8Array's bytes as they are, anything else as UTF-8 text.
pub fn request_body(v: &JsValue, ctx: &mut Context) -> JsResult<Option<Vec<u8>>> {
    if v.is_null_or_undefined() {
        return Ok(None);
    }
    if let Some(o) = v.as_object() {
        if let Ok(arr) = JsUint8Array::from_object(o.clone()) {
            return Ok(Some(arr.to_vec(ctx)?));
        }
    }
    Ok(Some(v.to_string(ctx)?.to_std_string_escaped().into_bytes()))
}
