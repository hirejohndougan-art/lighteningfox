//! The JavaScript engine (Boa) parsing and running arbitrary scripts, with the same limits
//! the browser sets, so a page can't crash the browser with a script.
#![no_main]
use boa_engine::{Context, Source};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(code) = std::str::from_utf8(data) else { return };
    let mut ctx = Context::default();
    ctx.runtime_limits_mut().set_loop_iteration_limit(100_000);
    ctx.runtime_limits_mut().set_recursion_limit(400);
    ctx.runtime_limits_mut().set_stack_size_limit(10_000);
    let _ = ctx.eval(Source::from_bytes(code.as_bytes()));
    let _ = ctx.run_jobs();
});
