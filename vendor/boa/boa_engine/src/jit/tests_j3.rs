//! J3: compiled code with objects, calls in both directions and exceptions gives the
//! interpreter's results. Each test runs the same program with the JIT off and forced on
//! (`tests::same`), which also checks that every named function ran compiled.

use super::tests::{code_of, same, with_mode};
use super::{JitMode, TierState};
use crate::{Context, JsValue};
use boa_parser::Source;

#[test]
fn objects_and_arrays() {
    same(
        &[
            ("point", "(x, y) { return { x, y, len: Math.hypot(x, y) }; }"),
            ("sumProps", "(o) { let s = 0; for (const k in o) s += o[k]; return s; }"),
            ("fill", "(n) { const a = []; for (let i = 0; i < n; i++) a.push(i * i); a[n] = 'end'; return a.join('|'); }"),
            ("bump", "(o) { o.count = (o.count || 0) + 1; o['k' + o.count] = o.count; delete o.k1; return Object.keys(o).join(); }"),
        ],
        "const o = {}; for (let i = 0; i < 4; i++) out(bump(o)); out(JSON.stringify(point(3, 4)), sumProps({ a: 1, b: 2, c: 3 }), fill(5), sumProps([1, 2, 3]), sumProps('ab'));",
    );
}

#[test]
fn calls_in_both_directions() {
    same(
        &[
            ("fib", "(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }"),
            ("twice", "(f, x) { return f(f(x)); }"),
            ("inc", "(x) { return x + 1; }"),
            ("viaNative", "(a) { return a.map((x) => inc(x) * 2).filter((x) => x % 3 !== 0).reduce((s, x) => s + x, 0); }"),
            ("sorter", "(a) { return a.slice().sort((p, q) => q - p).join(); }"),
            ("method", "(o) { return o.get() + o.get.call({ v: 100 }); }"),
            ("spread", "(...xs) { return Math.max(...xs) + xs.length + arguments.length; }"),
            ("make", "(n) { return new Array(n).fill(n); }"),
            ("ctor", "(v) { this.v = v; }"),
            ("build", "(n) { const r = []; for (let i = 0; i < n; i++) r.push(new ctor(i).v); return r.join(); }"),
        ],
        "out(fib(15), twice(inc, 1), twice((x) => x * 3, 2), viaNative([1, 2, 3, 4, 5, 6]), sorter([3, 1, 2]),
             method({ v: 1, get() { return this.v; } }), spread(1, 9, 3), make(3).join(), build(4));
         out([5, 6].map(inc).join(), [1, 2, 3].map((x) => fib(x)).join(), Reflect.apply(twice, null, [inc, 5]));",
    );
}

#[test]
fn closures_and_environments() {
    same(
        &[
            ("counter", "() { let c = 0; return () => ++c; }"),
            ("adders", "(n) { const fs = []; for (let i = 0; i < n; i++) fs.push((x) => x + i); return fs.map((f) => f(10)).join(); }"),
            ("shadow", "(x) { { let x = 5; x++; } var y = x; { const y2 = y * 2; y = y2; } return y; }"),
        ],
        "const c = counter(); c(); c(); out(c(), adders(4), shadow(3), typeof counter()());",
    );
}

#[test]
fn exceptions_through_compiled_code() {
    same(
        &[
            ("thrower", "(x) { if (x > 2) throw new RangeError('big ' + x); return x; }"),
            ("caller", "(x) { try { return thrower(x) * 10; } catch (e) { return e.name + ':' + e.message; } finally { log.push('f' + x); } }"),
            ("nested", "(x) { try { try { null.p; } finally { log.push('inner'); } } catch (e) { return e instanceof TypeError; } }"),
            ("rethrow", "(x) { try { thrower(x); } catch (e) { throw new Error('wrapped ' + e.message); } return 'ok'; }"),
            ("loopCatch", "(n) { let caught = 0; for (let i = 0; i < n; i++) { try { thrower(i); } catch { caught++; } } return caught; }"),
        ],
        "var log = []; out(caller(1), caller(5), nested(0), loopCatch(6), rethrow(1));
         try { rethrow(9); } catch (e) { out(e.message); }
         try { thrower(7); } catch (e) { out(e.name); }
         out(log.join());",
    );
}

#[test]
fn getters_setters_and_proxies_run_once() {
    same(
        &[
            ("read", "(o) { return o.v + o.v; }"),
            ("write", "(o, v) { o.v = v; return o.v; }"),
            ("proxied", "(p) { p.a = 1; return p.a + p.b; }"),
        ],
        "let n = 0; const o = { get v() { return ++n; }, set v(x) { n += x * 100; } };
         out(read(o), n, write(o, 2), n);
         const seen = []; const p = new Proxy({}, { get(t, k) { seen.push('get ' + String(k)); return 7; }, set(t, k, v) { seen.push('set ' + String(k)); return true; } });
         out(proxied(p), seen.join('/'));",
    );
}

#[test]
fn strings_iterators_and_generators_called_from_compiled_code() {
    same(
        &[
            ("words", "(s) { let r = ''; for (const w of s.split(' ')) r += w[0].toUpperCase() + w.slice(1) + '-'; return r; }"),
            ("useGen", "(n) { function* g() { for (let i = 0; i < n; i++) yield i * 2; } let s = 0; for (const v of g()) s += v; return s; }"),
            ("destructure", "({ a, b: [c, ...d] }) { return a + c + d.length; }"),
            ("templ", "(x) { return `x=${x}, sq=${x * x}`; }"),
        ],
        "out(words('hello big world'), useGen(5), destructure({ a: 1, b: [2, 3, 4] }), templ(4));",
    );
}

#[test]
fn deep_recursion_stops_at_the_same_limit() {
    // Calls don't nest machine frames, so the recursion limit is reached where the
    // interpreter reaches it.
    same(&[("down", "(n) { return n === 0 ? 0 : 1 + down(n - 1); }")], "out(down(300)); try { down(100000); } catch (e) { out(e.name, String(e.message).slice(0, 40)); }");
}

#[test]
fn uncaught_errors_leave_the_same_way() {
    let program = "function f(o) { let s = 0; for (let i = 0; i < 3; i++) s += o.missing.deep; return s; } f({})";
    let run = |mode| {
        with_mode(mode, || {
            let mut context = Context::default();
            match context.eval(Source::from_bytes(program)) {
                Ok(v) => format!("{}", v.display()),
                Err(e) => format!("threw {e}"),
            }
        })
    };
    let (a, b) = (run(JitMode::Off), run(JitMode::Always));
    assert!(a.starts_with("threw TypeError"), "{a}");
    assert_eq!(a, b);
}

#[test]
fn calls_from_rust_into_functions_with_compiled_loops() {
    // `JsObject::call` from Rust: the callee's frame returns to Rust, so an exception in its
    // compiled code must come back as an `Err`, not unwind into frames that aren't its own.
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        let code = code_of(&mut context, "function f(o, n) { let s = 0; for (let i = 0; i < n; i++) { if (i === 3) o.boom(); s += o.v; } return s; } f");
        let f = context.global_object().get(crate::js_string!("f"), &mut context).unwrap();
        let f = f.as_object().unwrap().clone();
        let o = context.eval(Source::from_bytes("({ v: 2, boom() { throw new Error('boom'); } })")).unwrap();
        let ok = f.call(&JsValue::undefined(), &[o.clone(), JsValue::new(3)], &mut context).unwrap();
        assert_eq!(ok, JsValue::new(6));
        let err = f.call(&JsValue::undefined(), &[o, JsValue::new(10)], &mut context).unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
        assert_eq!(code.jit.state(), TierState::Compiled);
        // The context still works afterwards.
        assert_eq!(context.eval(Source::from_bytes("f({ v: 1, boom() {} }, 4)")).unwrap(), JsValue::new(4));
    });
}

#[test]
fn native_loops_never_enter_their_callers_code() {
    // `String.prototype.repeat` charges the loop limit per repetition; that must not tier up
    // or run the calling script's machine code in the middle of the call.
    with_mode(JitMode::Always, || {
        let mut context = Context::default();
        context.eval(Source::from_bytes("var en = new String('english');")).unwrap();
        assert_eq!(context.eval(Source::from_bytes("en.repeat(0)")).unwrap().display().to_string(), "\"\"");
        assert_eq!(context.eval(Source::from_bytes("en.repeat(2)")).unwrap().display().to_string(), "\"englishenglish\"");
        assert_eq!(context.eval(Source::from_bytes("for (let i = 0; i < 3; i++) 'ab'.repeat(2); 'x'.repeat(3)")).unwrap().display().to_string(), "\"xxx\"");
    });
}
