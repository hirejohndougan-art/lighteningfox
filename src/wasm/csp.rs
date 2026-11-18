//! The Content Security Policy rule for WebAssembly (task P1c): a page whose policy limits
//! scripts may only compile WebAssembly from bytes if `script-src` (or `default-src`) lists
//! `'wasm-unsafe-eval'` or `'unsafe-eval'`. Blocked compiles throw `WebAssembly.CompileError`,
//! as in Chrome. Loading a `.wasm` ES module is a script load (checked against `script-src` as
//! a URL by the module loader), not a compile from bytes, so it doesn't need the keyword.

use super::{Kind, State, throw};
use boa_engine::{Context, JsResult};

const REFUSED: &str = "WebAssembly.Module(): Refused to compile or instantiate WebAssembly module because \
neither 'wasm-unsafe-eval' nor 'unsafe-eval' is an allowed source of script in the page's Content Security Policy";

/// Throws a `CompileError` if the page's policy doesn't allow compiling WebAssembly.
pub(super) fn check(ctx: &mut Context, st: &State) -> JsResult<()> {
    let allowed = crate::js::page_ctx(ctx, |p| p.policy.csp.is_empty() || p.policy.csp.allows_wasm_eval()).unwrap_or(true);
    if allowed { Ok(()) } else { Err(throw(ctx, st, Kind::Compile, REFUSED)) }
}
