//! The baseline JIT (task J3) against the interpreter: each script runs twice, with the JIT
//! off and with every function compiled on first use (`BOA_JIT=always`), and any difference in
//! the outcome (the result as text, or the error) is a crash.
//!
//! Scripts can't see time or randomness here (`Math.random` and `Date.now` are fixed), so the
//! two runs see the same world.
#![no_main]
use boa_engine::jit::{JitMode, set_mode};
use boa_engine::{Context, Source};
use libfuzzer_sys::fuzz_target;

const FIXED: &str = "Math.random = () => 0.5; Date.now = () => 0;";

fn run(code: &str, mode: JitMode) -> String {
    set_mode(mode);
    let mut ctx = Context::default();
    ctx.runtime_limits_mut().set_loop_iteration_limit(100_000);
    ctx.runtime_limits_mut().set_recursion_limit(400);
    ctx.runtime_limits_mut().set_stack_size_limit(10_000);
    let _ = ctx.eval(Source::from_bytes(FIXED));
    let outcome = match ctx.eval(Source::from_bytes(code.as_bytes())) {
        Ok(v) => match v.to_string(&mut ctx) {
            Ok(s) => format!("ok {}", s.to_std_string_escaped()),
            Err(_) => format!("ok <{}>", v.type_of()),
        },
        Err(e) => format!("error {e}"),
    };
    let _ = ctx.run_jobs();
    outcome
}

fuzz_target!(|data: &[u8]| {
    let Ok(code) = std::str::from_utf8(data) else { return };
    let interpreted = run(code, JitMode::Off);
    let compiled = run(code, JitMode::Always);
    assert_eq!(interpreted, compiled, "the JIT changed the outcome of {code:?}");
});
