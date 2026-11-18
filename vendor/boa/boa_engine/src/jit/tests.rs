use super::{JitBackend, JitMode, TierState, mode::TEST_MODE};
use crate::{Context, JsValue, builtins::function::OrdinaryFunction, vm::CodeBlock};
use boa_gc::Gc;
use boa_parser::Source;
use cranelift_codegen::ir::{InstBuilder, types};

/// Runs `body` with this thread's JIT mode set to `mode`.
pub(super) fn with_mode<R>(mode: JitMode, body: impl FnOnce() -> R) -> R {
    TEST_MODE.with(|m| m.set(Some(mode)));
    let result = body();
    TEST_MODE.with(|m| m.set(None));
    result
}

/// Evaluates `source` (which must end with a function expression) and returns its code block.
pub(super) fn code_of(context: &mut Context, source: &str) -> Gc<CodeBlock> {
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
fn number_loop_is_compiled() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, SUM);
        let result = context.eval(Source::from_bytes("sum(10)")).unwrap();
        assert_eq!(result, JsValue::new(80));
        assert_eq!(code.jit.state(), TierState::Compiled);
    });
}

#[test]
fn property_access_is_compiled() {
    // J3: the interpreter does the property read for compiled code, which goes on after it.
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, "function f(o) { return o.x + 1; } f");
        assert_eq!(context.eval(Source::from_bytes("f({ x: 1 })")).unwrap(), JsValue::new(2));
        assert_eq!(code.jit.state(), TierState::Compiled);
    });
}

#[test]
fn generators_and_async_are_left_to_the_interpreter() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        for (source, run) in [("function* g(a) { yield a + 1; } g", "[...g(1)]"), ("async function h(a) { return a + 1; } h", "h(1)")] {
            let code = code_of(&mut context, source);
            let _ = context.eval(Source::from_bytes(run));
            assert!(!matches!(code.jit.state(), TierState::Compiled), "{source}: {:?}", code.jit.state());
            assert!(code.jit.code().is_none());
        }
    });
}

#[test]
fn try_catch_is_compiled() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, "function f(a) { try { return a.b.c + 1; } catch { return 0; } } f");
        assert_eq!(context.eval(Source::from_bytes("f({ b: { c: 1 } }) + f(null)")).unwrap(), JsValue::new(2));
        assert_eq!(code.jit.state(), TierState::Compiled);
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
        assert_eq!(code.jit.state(), TierState::Compiled);
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
        assert_eq!(code.jit.state(), TierState::Compiled);
    });
}

#[test]
fn on_leaves_mostly_object_code_to_the_interpreter() {
    // J4: in the default mode a function made mostly of instructions the interpreter would run
    // for compiled code anyway stays interpreted; number code still compiles, and `always`
    // compiles both.
    const OBJECTS: &str = "function g(o) { o.a = o.b; o.c = o.d; return o.e.f; } g";
    with_mode(JitMode::On, || {
        let mut context = Context::default();
        let objects = code_of(&mut context, OBJECTS);
        let numbers = code_of(&mut context, SUM);
        let calls = super::CALL_THRESHOLD + 1;
        context
            .eval(Source::from_bytes(&format!(
                "for (let i = 0; i < {calls}; i++) {{ g({{ b: 1, d: 2, e: {{ f: 3 }} }}); sum(3); }}"
            )))
            .unwrap();
        assert!(
            matches!(objects.jit.state(), TierState::Rejected(r) if r.reason == "mostly interpreter work"),
            "{:?}",
            objects.jit.state()
        );
        assert_eq!(numbers.jit.state(), TierState::Compiled);
    });
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let objects = code_of(&mut context, OBJECTS);
        let result = context.eval(Source::from_bytes("g({ b: 1, d: 2, e: { f: 3 } })")).unwrap();
        assert_eq!(result, JsValue::new(3));
        assert_eq!(objects.jit.state(), TierState::Compiled);
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

// ---- J2: compiled code gives the interpreter's results ----

/// Runs `driver` after defining `functions` (only plain number code, so each one compiles
/// with the JIT forced on), with the JIT off and with it forced on, and checks the results are
/// the same, `-0` and `NaN` included. Also checks every named function really ran compiled.
pub(super) fn same(functions: &[(&str, &str)], driver: &str) {
    let mut source = String::new();
    for (name, body) in functions {
        source.push_str(&format!("function {name}{body}\n"));
    }
    source.push_str(&format!(
        "const __show = (x) => Object.is(x, -0) ? '-0' : typeof x === 'string' ? JSON.stringify(x) : String(x);\n\
         const __out = []; const out = (...v) => __out.push(...v.map(__show));\n{driver}\n__out.join(',')"
    ));
    let run = |mode| {
        with_mode(mode, || {
            let mut context = Context::default();
            let result = match context.eval(Source::from_bytes(&source)) {
                Ok(v) => v.to_string(&mut context).unwrap().to_std_string_escaped(),
                Err(e) => format!("threw {e}"),
            };
            let states: Vec<(String, TierState)> = functions
                .iter()
                .map(|(name, _)| {
                    let f = context.global_object().get(crate::js_string!(*name), &mut context).unwrap();
                    let object = f.as_object().unwrap();
                    let state = object.downcast_ref::<OrdinaryFunction>().unwrap().code.jit.state();
                    ((*name).to_string(), state)
                })
                .collect();
            (result, states)
        })
    };
    let (interpreted, _) = run(JitMode::Off);
    let (compiled, states) = run(JitMode::Always);
    assert_eq!(interpreted, compiled, "results differ for {source}");
    for (name, state) in states {
        assert!(
            matches!(state, TierState::Compiled) || matches!(state, TierState::Rejected(r) if r.reason == "bails out"),
            "{name} didn't run compiled: {state:?}"
        );
    }
}

const NUMBERS: &str = "[7, 3, -7, 0, -0, 1, -1, 1.5, 0.25, 2147483647, -2147483648, 2147483646, 65536, 31, 32, NaN, Infinity, -Infinity, 1e21, -5]";

#[test]
fn arithmetic_matches_the_interpreter() {
    let ops = ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", ">>>"];
    let names: Vec<String> = (0..ops.len()).map(|i| format!("op{i}")).collect();
    let bodies: Vec<String> = ops.iter().map(|op| format!("(a, b) {{ return a {op} b; }}")).collect();
    let functions: Vec<(&str, &str)> = names.iter().map(String::as_str).zip(bodies.iter().map(String::as_str)).collect();
    let calls: String = names.iter().map(|n| format!("out({n}(a, b));")).collect();
    same(&functions, &format!("for (const a of {NUMBERS}) for (const b of {NUMBERS}) {{ {calls} }}"));
}

#[test]
fn unary_operators_match() {
    same(
        &[("neg", "(a) { return -a; }"), ("pos", "(a) { return +a; }"), ("bnot", "(a) { return ~a; }"), ("lnot", "(a) { return !a; }")],
        &format!("for (const a of {NUMBERS}) out(neg(a), pos(a), bnot(a), lnot(a));"),
    );
}

#[test]
fn comparisons_and_branches_match() {
    let ops = ["<", "<=", ">", ">=", "==", "!=", "===", "!=="];
    let names: Vec<String> = (0..ops.len()).map(|i| format!("cmp{i}")).collect();
    let bodies: Vec<String> = ops.iter().map(|op| format!("(a, b) {{ return a {op} b; }}")).collect();
    let mut functions: Vec<(&str, &str)> = names.iter().map(String::as_str).zip(bodies.iter().map(String::as_str)).collect();
    functions.push(("and", "(a, b) { return a && b; }"));
    functions.push(("or", "(a, b) { return a || b; }"));
    functions.push(("pick", "(a, b) { if (a < b) return 1; if (a > b) return 2; if (a === b) return 3; return 4; }"));
    let calls: String = functions.iter().map(|(n, _)| format!("out({n}(a, b));")).collect();
    same(&functions, &format!("for (const a of {NUMBERS}) for (const b of [0, -0, 1, 7, NaN, 1.5, true, false, undefined]) {{ {calls} }}"));
}

#[test]
fn loops_match() {
    same(
        &[
            ("g", "(n) { let c = 0; for (let i = 0; i < n; i++) { if (i % 3 === 0) c += 2; else if (i > 10) c -= 1; else c++; } while (c > 5) c = c - 4; do { c += 0.5; } while (c < 2); return c; }"),
            ("fib", "(n) { let a = 0, b = 1; for (let i = 0; i < n; i++) { const t = a + b; a = b; b = t; } return a; }"),
            ("down", "(n) { let s = 0; for (let i = n; i > 0; i--) s += i / 2; return s; }"),
        ],
        "for (const n of [0, 1, 2, 12, 100, 1000, 80]) out(g(n), fib(n), down(n));",
    );
}

#[test]
fn inc_and_dec_at_the_limits() {
    same(
        &[("inc", "(x) { let a = x; a++; ++a; return a; }"), ("dec", "(x) { let b = x; b--; --b; return b; }"), ("post", "(x) { let a = x; const old = a++; return old; }")],
        "for (const x of [0, 2147483647, -2147483648, 2147483646, -2147483647, 1.5, -0, NaN, Infinity]) out(inc(x), dec(x), post(x));",
    );
}

#[test]
fn values_that_need_the_interpreter() {
    // Strings, objects, null, booleans and missing arguments: each bails out where needed and
    // the interpreter finishes.
    same(
        &[("mix", "(a, b) { let s = a + b; let t = a * 2; return s + t; }"), ("count", "(n) { let c = 0; for (let i = 0; i < n; i++) c++; return c; }")],
        "for (const [a, b] of [[1, 2], ['1', 2], [1, '2'], [{}, 1], [null, 1], [true, 1], [undefined, 1], [1, undefined], [[1], [2]]]) out(mix(a, b));
         out(mix(1), mix(), count(5), count('5'), count(null), count({ valueOf() { return 3; } }), count(true));",
    );
}

#[test]
fn values_machine_code_cant_handle_go_to_the_interpreter_in_place() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, SUM);
        let r = context.eval(Source::from_bytes("let t = 0; for (let k = 0; k < 50; k++) t += sum(k); t")).unwrap();
        assert!(r.is_number());
        assert_eq!(code.jit.state(), TierState::Compiled);
        assert!(code.jit.code().is_some());
        // A string argument: the interpreter does `i < n` for compiled code (J3), which goes
        // on; nothing bails out.
        let r = context.eval(Source::from_bytes("sum('10')")).unwrap();
        assert_eq!(r, JsValue::new(80));
        let r = context.eval(Source::from_bytes("function g(a) { return a + 1; } for (let i = 0; i < 200; i++) g('x'); g('y')")).unwrap();
        assert_eq!(r.to_string(&mut context).unwrap().to_std_string_escaped(), "y1");
        assert_eq!(code.jit.bailouts(), 0);
        assert_eq!(code.jit.state(), TierState::Compiled);
    });
}

#[test]
fn hot_loop_enters_compiled_code_mid_call() {
    with_mode(JitMode::On, || {
        let mut context = Context::default();
        let code = code_of(&mut context, SUM);
        let r = context.eval(Source::from_bytes("sum(100000)")).unwrap();
        assert_eq!(code.jit.state(), TierState::Compiled);
        assert_eq!(code.jit.calls(), 1);
        // Same result as a fully interpreted run.
        let expected = with_mode(JitMode::Off, || {
            let mut c = Context::default();
            c.eval(Source::from_bytes(&format!("{SUM}; sum(100000)"))).unwrap()
        });
        assert_eq!(r, expected);
    });
}

#[test]
fn loop_limit_still_applies_in_compiled_loops() {
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        context.runtime_limits_mut().set_loop_iteration_limit(1000);
        let code = code_of(&mut context, "function spin(n) { let c = 0; while (c < n) c++; return c; } spin");
        assert_eq!(context.eval(Source::from_bytes("spin(500)")).unwrap(), JsValue::new(500));
        assert_eq!(code.jit.state(), TierState::Compiled);
        assert!(context.eval(Source::from_bytes("spin(5000)")).is_err());
    });
}
