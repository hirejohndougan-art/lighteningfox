use super::{JitBackend, JitMode, TierState, mode::TEST_MODE};
use crate::{Context, JsValue, builtins::function::OrdinaryFunction, vm::CodeBlock};
use boa_gc::Gc;
use boa_parser::Source;
use cranelift_codegen::ir::{InstBuilder, types};

/// Runs `body` with this thread's JIT mode set to `mode`.
fn with_mode<R>(mode: JitMode, body: impl FnOnce() -> R) -> R {
    TEST_MODE.with(|m| m.set(Some(mode)));
    let result = body();
    TEST_MODE.with(|m| m.set(None));
    result
}

/// Evaluates `source` (which must end with a function expression) and returns its code block.
fn code_of(context: &mut Context, source: &str) -> Gc<CodeBlock> {
    let value = context
        .eval(Source::from_bytes(source))
        .expect("script runs");
    let object = value.as_object().expect("a function");
    let function = object
        .downcast_ref::<OrdinaryFunction>()
        .expect("an ordinary function");
    function.code.clone()
}

const SUM: &str = r"
    function sum(n) {
        let total = 0;
        for (let i = 0; i < n; i++) {
            total += i * 2 - 1;
            if (total > 1e9) { total = total % 7; }
        }
        return total;
    }
    sum";

#[test]
fn mode_parses_boa_jit_values() {
    assert_eq!(JitMode::parse("off"), JitMode::Off);
    assert_eq!(JitMode::parse("0"), JitMode::Off);
    assert_eq!(JitMode::parse(" Always "), JitMode::Always);
    assert_eq!(JitMode::parse("on"), JitMode::On);
    assert_eq!(JitMode::parse("anything else"), JitMode::On);
}

#[test]
fn number_loop_is_eligible() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, SUM);
        let result = context.eval(Source::from_bytes("sum(10)")).unwrap();
        assert_eq!(result, JsValue::new(80));
        assert_eq!(code.jit.state(), TierState::Eligible);
    });
}

#[test]
fn property_access_is_rejected_with_the_opcode() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, "function f(o) { return o.x + 1; } f");
        context.eval(Source::from_bytes("f({ x: 1 })")).unwrap();
        let TierState::Rejected(rejection) = code.jit.state() else {
            panic!("expected a rejection, got {:?}", code.jit.state());
        };
        assert!(rejection.reason.starts_with("GetProperty"), "{rejection:?}");
        assert!(rejection.pc.is_some());
    });
}

#[test]
fn generators_and_try_are_rejected() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, "function f(a) { try { return a + 1; } catch { return 0; } } f");
        context.eval(Source::from_bytes("f(1)")).unwrap();
        assert!(matches!(code.jit.state(), TierState::Rejected(r) if r.pc.is_none()));
    });
}

#[test]
fn tier_up_after_call_threshold() {
    with_mode(JitMode::On, || {
        let mut context = Context::default();
        let code = code_of(&mut context, "function f(a, b) { return a * b + 1; } f");
        let calls = super::CALL_THRESHOLD - 1;
        context
            .eval(Source::from_bytes(&format!("for (let i = 0; i < {calls}; i++) f(i, 2);")))
            .unwrap();
        assert_eq!(code.jit.calls(), calls);
        assert_eq!(code.jit.state(), TierState::Interpreted);
        context.eval(Source::from_bytes("f(3, 4)")).unwrap();
        assert_eq!(code.jit.state(), TierState::Eligible);
    });
}

#[test]
fn tier_up_from_a_hot_loop_in_one_call() {
    with_mode(JitMode::On, || {
        let mut context = Context::default();
        let code = code_of(&mut context, SUM);
        context.eval(Source::from_bytes("sum(100)")).unwrap();
        assert_eq!(code.jit.state(), TierState::Interpreted);
        context.eval(Source::from_bytes("sum(20000)")).unwrap();
        assert_eq!(code.jit.calls(), 2);
        assert!(code.jit.loop_iterations() >= super::LOOP_THRESHOLD);
        assert_eq!(code.jit.state(), TierState::Eligible);
    });
}

#[test]
fn off_never_tiers_up() {
    with_mode(JitMode::Off, || {
        let mut context = Context::default();
        let code = code_of(&mut context, SUM);
        context.eval(Source::from_bytes("sum(20000)")).unwrap();
        assert_eq!(code.jit.state(), TierState::Interpreted);
    });
}

#[test]
fn backend_compiles_and_runs_a_function() {
    let mut backend = JitBackend::new().expect("Cranelift supports this CPU");
    let sig = backend.signature(&[types::F64, types::F64], &[types::F64]);
    let code = backend
        .compile(sig, |b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            b.seal_block(block);
            let (x, y) = (b.block_params(block)[0], b.block_params(block)[1]);
            let sum = b.ins().fadd(x, y);
            b.ins().return_(&[sum]);
        })
        .expect("compiles");
    // SAFETY: `code` was compiled above with exactly this signature in the host convention.
    let add: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(code) };
    assert_eq!(add(2.5, 4.0), 6.5);
    assert_eq!(backend.function_count(), 1);
}

/// W^X: the page holding compiled code is read+execute, not writable.
#[cfg(target_os = "linux")]
#[test]
fn executable_memory_is_never_writable() {
    let mut backend = JitBackend::new().expect("Cranelift supports this CPU");
    let sig = backend.signature(&[], &[types::I32]);
    let code = backend
        .compile(sig, |b| {
            let block = b.create_block();
            b.switch_to_block(block);
            b.seal_block(block);
            let v = b.ins().iconst(types::I32, 42);
            b.ins().return_(&[v]);
        })
        .expect("compiles");
    // SAFETY: compiled above as `fn() -> i32`.
    let f: extern "C" fn() -> i32 = unsafe { std::mem::transmute(code) };
    assert_eq!(f(), 42);

    let address = code as usize;
    let maps = std::fs::read_to_string("/proc/self/maps").expect("readable");
    let perms = maps
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let (start, end) = parts.next()?.split_once('-')?;
            let (start, end) = (
                usize::from_str_radix(start, 16).ok()?,
                usize::from_str_radix(end, 16).ok()?,
            );
            (start..end).contains(&address).then(|| parts.next().map(str::to_string))?
        })
        .expect("compiled code is mapped");
    assert!(perms.starts_with("r-x"), "code page is {perms}");
}
