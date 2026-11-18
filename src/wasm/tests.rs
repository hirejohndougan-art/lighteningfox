//! Tests for the WebAssembly JS API (P1a). Modules are written as text and handed to the page
//! as a `Uint8Array` named `bytes`.

use crate::js::Js;

pub(super) fn page(wat: &str) -> Js {
    let bin = wat::parse_str(wat).expect("test module parses");
    let list: Vec<String> = bin.iter().map(|b| b.to_string()).collect();
    let mut js = Js::new("https://wasm.example/");
    js.run(&format!("var bytes = new Uint8Array([{}]);", list.join(","))).expect("bytes");
    js
}

/// Run script; a string result comes back without the console's quotes.
pub(super) fn run(js: &mut Js, code: &str) -> String {
    let out = js.run(code).unwrap_or_else(|e| format!("error: {e}"));
    if out.starts_with('"') {
        return serde_json::from_str(&out).unwrap_or(out);
    }
    out
}

const MATH: &str = r#"(module
  (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add)
  (func (export "add64") (param i64 i64) (result i64) local.get 0 local.get 1 i64.add)
  (func (export "half") (param f32) (result f32) local.get 0 f32.const 0.5 f32.mul)
  (func (export "sqrt") (param f64) (result f64) local.get 0 f64.sqrt)
  (func (export "swap") (param i32 i32) (result i32 i32) local.get 1 local.get 0)
  (func (export "div") (param i32 i32) (result i32) local.get 0 local.get 1 i32.div_s)
  (func (export "boom") unreachable)
  (@custom "notes" "hello")
)"#;

#[test]
fn namespace_and_validate() {
    let mut js = page(MATH);
    assert_eq!(run(&mut js, "typeof WebAssembly + ' ' + Object.prototype.toString.call(WebAssembly)"), "object [object WebAssembly]");
    // WebIDL: the namespace's operations are enumerable, its interfaces aren't.
    assert_eq!(run(&mut js, "Object.keys(WebAssembly).sort().join()"), "compile,compileStreaming,instantiate,instantiateStreaming,validate");
    assert_eq!(run(&mut js, "WebAssembly.validate(bytes) + ' ' + WebAssembly.validate(new Uint8Array([0, 97, 115, 109, 9, 9]))"), "true false");
    assert_eq!(run(&mut js, "WebAssembly.validate(bytes.buffer)"), "true");
    assert_eq!(run(&mut js, "try { WebAssembly.validate(42) } catch (e) { e.constructor.name }"), "TypeError");
}

#[test]
fn module_reflection() {
    let mut js = page(r#"(module
      (import "env" "log" (func (param i32)))
      (import "env" "k" (global i32))
      (func (export "run"))
      (memory (export "mem") 1)
      (@custom "notes" "hi!"))"#);
    run(&mut js, "var m = new WebAssembly.Module(bytes)");
    assert_eq!(run(&mut js, "JSON.stringify(WebAssembly.Module.imports(m))"), r#"[{"module":"env","name":"log","kind":"function"},{"module":"env","name":"k","kind":"global"}]"#);
    assert_eq!(run(&mut js, "JSON.stringify(WebAssembly.Module.exports(m))"), r#"[{"name":"run","kind":"function"},{"name":"mem","kind":"memory"}]"#);
    let sections = "WebAssembly.Module.customSections(m, 'notes').map(b => String.fromCharCode(...new Uint8Array(b))).join()";
    assert_eq!(run(&mut js, sections), "hi!");
    assert_eq!(run(&mut js, "WebAssembly.Module.customSections(m, 'other').length"), "0");
    assert_eq!(run(&mut js, "String(m)"), "[object WebAssembly.Module]");
    assert_eq!(run(&mut js, "try { WebAssembly.Module.exports({}) } catch (e) { e.constructor.name }"), "TypeError");
}

#[test]
fn numbers_both_ways() {
    let mut js = page(MATH);
    run(&mut js, "var e = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports");
    assert_eq!(run(&mut js, "e.add(2, 3) + ' ' + e.add(0x7fffffff, 1) + ' ' + e.add('4', 1.9)"), "5 -2147483648 5");
    assert_eq!(run(&mut js, "String(e.add64(2n ** 62n, 2n ** 62n))"), "-9223372036854775808");
    assert_eq!(run(&mut js, "try { e.add64(1, 2) } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "e.half(0.1)"), "0.05000000074505806");
    assert_eq!(run(&mut js, "e.sqrt(2)"), "1.4142135623730951");
    assert_eq!(run(&mut js, "JSON.stringify(e.swap(1, 2))"), "[2,1]");
    // Missing arguments are undefined: 0 for i32.
    assert_eq!(run(&mut js, "e.add(5)"), "5");
}

#[test]
fn exported_functions() {
    let mut js = page(MATH);
    run(&mut js, "var i = new WebAssembly.Instance(new WebAssembly.Module(bytes)); var e = i.exports");
    // name is the function index, length the parameter count.
    assert_eq!(run(&mut js, "e.add.name + ' ' + e.add.length + ' ' + e.sqrt.name + ' ' + e.boom.length"), "0 2 3 0");
    assert_eq!(run(&mut js, "e.add === i.exports.add"), "true");
    assert_eq!(run(&mut js, "Object.isFrozen(e) + ' ' + Object.getPrototypeOf(e) + ' ' + Object.keys(e).join()"), "true null add,add64,half,sqrt,swap,div,boom");
    assert_eq!(run(&mut js, "try { new e.add() } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "String(i)"), "[object WebAssembly.Instance]");
}

#[test]
fn traps_are_runtime_errors() {
    let mut js = page(MATH);
    run(&mut js, "var e = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports");
    let r = run(&mut js, "try { e.boom() } catch (x) { [x instanceof WebAssembly.RuntimeError, x instanceof Error, x.name].join() }");
    assert_eq!(r, "true,true,RuntimeError");
    assert_eq!(run(&mut js, "try { e.div(1, 0) } catch (x) { x.name + ': ' + x.message }"), "RuntimeError: integer divide by zero");
    // The instance still works afterwards.
    assert_eq!(run(&mut js, "e.div(7, 2)"), "3");
}

#[test]
fn compile_and_link_errors() {
    let mut js = page(r#"(module (import "env" "f" (func (param i32))) (func (export "g") i32.const 1 call 0))"#);
    let r = run(&mut js, "try { new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 99])) } catch (x) { [x instanceof WebAssembly.CompileError, x.name].join() }");
    assert_eq!(r, "true,CompileError");
    run(&mut js, "var m = new WebAssembly.Module(bytes)");
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(m) } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(m, {}) } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(m, {env: {f: 3}}) } catch (x) { x.name }"), "LinkError");
    // A wasm function of the wrong type as an import.
    run(&mut js, "var other = new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {f() {}}}).exports.g");
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(m, {env: {f: other}}) } catch (x) { x.name }"), "LinkError");
    assert_eq!(run(&mut js, "WebAssembly.LinkError.prototype.name + ' ' + WebAssembly.CompileError.length + ' ' + new WebAssembly.RuntimeError('x').message"), "LinkError 1 x");
}

#[test]
fn imports_call_into_script() {
    let mut js = page(r#"(module
      (import "env" "twice" (func $twice (param i32) (result i32)))
      (import "env" "big" (func $big (param i64) (result i64)))
      (import "env" "pair" (func $pair (result i32 i32)))
      (import "env" "k" (global $k i32))
      (func (export "run") (param i32) (result i32) local.get 0 call $twice global.get $k i32.add)
      (func (export "runBig") (param i64) (result i64) local.get 0 call $big)
      (func (export "sumPair") (result i32) call $pair i32.add)
      (export "twice" (func $twice)))"#);
    run(&mut js, "var seen = []; function twice(x) { seen.push(typeof x); return x * 2 }");
    run(&mut js, "var e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {twice, big: x => x * 3n, pair: () => [40, 2], k: 100}}).exports");
    assert_eq!(run(&mut js, "e.run(21) + ' ' + seen"), "142 number");
    assert_eq!(run(&mut js, "String(e.runBig(5n))"), "15");
    assert_eq!(run(&mut js, "e.sumPair()"), "42");
    // A JavaScript import exported again is the same function.
    assert_eq!(run(&mut js, "e.twice === twice"), "true");
}

#[test]
fn exceptions_pass_through_wasm() {
    let mut js = page(r#"(module
      (import "env" "cb" (func $cb))
      (func (export "call") call $cb)
      (func (export "inc") (param i32) (result i32) local.get 0 i32.const 1 i32.add))"#);
    run(&mut js, "var thrown = {tag: 1}; var mode = 'throw'; var e; var e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {cb() { if (mode === 'throw') throw thrown; mode = String(e.inc(41)); }}}).exports");
    assert_eq!(run(&mut js, "try { e.call(); 'no' } catch (x) { x === thrown }"), "true");
    // wasm -> script -> wasm.
    assert_eq!(run(&mut js, "mode = 'nest'; e.call(); mode"), "42");
    assert_eq!(run(&mut js, "e.inc(1)"), "2");
}

#[test]
fn references() {
    let mut js = page(r#"(module
      (func $id (export "id") (param externref) (result externref) local.get 0)
      (func (export "isNull") (param externref) (result i32) local.get 0 ref.is_null)
      (func (export "self") (result funcref) ref.func $id)
      (func (export "pass") (param funcref) (result funcref) local.get 0))"#);
    run(&mut js, "var e = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports; var o = {}");
    assert_eq!(run(&mut js, "[e.id(o) === o, e.id(undefined) === undefined, e.id(null), e.isNull(null), e.isNull(0)].join()"), "true,true,,1,0");
    assert_eq!(run(&mut js, "e.self() === e.id"), "true");
    assert_eq!(run(&mut js, "e.pass(e.id) === e.id && e.pass(null) === null"), "true");
    assert_eq!(run(&mut js, "try { e.pass(() => 1) } catch (x) { x.constructor.name }"), "TypeError");
}

#[test]
fn promises() {
    let mut js = page(r#"(module (func (export "seven") (result i32) i32.const 7))"#);
    run(&mut js, "var out = []; WebAssembly.instantiate(bytes).then(r => { out.push(r.module instanceof WebAssembly.Module, r.instance.exports.seven()); return WebAssembly.instantiate(r.module) }).then(i => out.push(i instanceof WebAssembly.Instance))");
    run(&mut js, "WebAssembly.compile(new Uint8Array([1, 2, 3])).catch(x => out.push(x.name))");
    run(&mut js, "WebAssembly.compile(5).catch(x => out.push(x.constructor.name))");
    run(&mut js, "WebAssembly.compile(bytes).then(m => out.push(WebAssembly.Module.exports(m)[0].name))");
    let r = run(&mut js, "out.join()");
    for want in ["true", "7", "CompileError", "TypeError", "seven"] {
        assert!(r.split(',').any(|x| x == want), "{want} missing from {r}");
    }
    assert!(r.ends_with("true") || r.contains("true,7"), "{r}");
}

#[test]
fn start_function_traps() {
    let mut js = page(r#"(module (func $s unreachable) (start $s))"#);
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(new WebAssembly.Module(bytes)) } catch (x) { x.name }"), "RuntimeError");
}

#[test]
fn runaway_module_is_stopped() {
    super::set_fuel_for_tests(50_000_000);
    let mut js = page(r#"(module (func (export "spin") (loop br 0)) (func (export "one") (result i32) i32.const 1))"#);
    run(&mut js, "var e = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports");
    // Like a runaway JavaScript loop: not catchable, the script stops.
    let r = js.run("try { e.spin() } catch (x) { 'caught' }");
    assert!(matches!(&r, Err(m) if m.contains("iteration")), "{r:?}");
    assert_eq!(run(&mut js, "e.one()"), "1");
}

#[test]
fn unbounded_recursion_is_a_range_error() {
    let mut js = page(r#"(module (func $f (export "f") call $f))"#);
    run(&mut js, "var e = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports");
    assert_eq!(run(&mut js, "try { e.f() } catch (x) { x.constructor.name }"), "RangeError");
}

#[test]
fn export_indices_from_bytes() {
    let bin = wat::parse_str(r#"(module (import "a" "b" (func)) (func) (func (export "x")) (export "y" (func 0)))"#).unwrap();
    let m = super::values::exports_in_order(&bin);
    assert_eq!(m, vec![("x".to_string(), 0, 2), ("y".to_string(), 0, 0)]);
}
