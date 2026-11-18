//! Tests for `WebAssembly.Memory`, `Table` and `Global` (P1b).

use super::tests::{page, run};

const MEM: &str = r#"(module
  (import "env" "seen" (func $seen))
  (memory (export "mem") 1 3)
  (func (export "store") (param i32 i32) local.get 0 local.get 1 i32.store8)
  (func (export "load") (param i32) (result i32) local.get 0 i32.load8_u)
  (func (export "grow") (param i32) (result i32) local.get 0 memory.grow)
  (func (export "growThenCall") i32.const 1 memory.grow drop call $seen))"#;

#[test]
fn memory_buffer_is_the_modules_memory() {
    let mut js = page(MEM);
    run(&mut js, "var seen = ''; var i = new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {seen() { seen = [old.byteLength, i.exports.mem.buffer.byteLength].join() }}}); var e = i.exports; var old");
    assert_eq!(run(&mut js, "e.mem instanceof WebAssembly.Memory && e.mem === i.exports.mem && String(e.mem)"), "[object WebAssembly.Memory]");
    assert_eq!(run(&mut js, "e.mem.buffer === e.mem.buffer && e.mem.buffer.byteLength"), "65536");
    // Writes from wasm show in a typed array, and the other way round.
    assert_eq!(run(&mut js, "var u8 = new Uint8Array(e.mem.buffer); e.store(10, 42); u8[10]"), "42");
    assert_eq!(run(&mut js, "u8[20] = 7; e.load(20)"), "7");
    // Script can't take the buffer away from the memory.
    assert_eq!(run(&mut js, "try { e.mem.buffer.transfer() } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "e.load(10)"), "42");
}

#[test]
fn growing_detaches_the_old_buffer() {
    let mut js = page(MEM);
    run(&mut js, "var seen = ''; var old; var i = new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {seen() { seen = [old.byteLength, i.exports.mem.buffer.byteLength].join() }}}); var e = i.exports");
    // From script.
    assert_eq!(run(&mut js, "old = e.mem.buffer; var u8 = new Uint8Array(old); e.store(5, 9); [e.mem.grow(1), old.byteLength, u8.length, e.mem.buffer.byteLength, new Uint8Array(e.mem.buffer)[5]].join()"), "1,0,0,131072,9");
    // From wasm: the next time script runs (here, an import) it sees the new size.
    assert_eq!(run(&mut js, "old = e.mem.buffer; e.growThenCall(); seen"), "0,196608");
    // grow(0) also replaces the buffer; past the maximum is a RangeError (and the size stays).
    assert_eq!(run(&mut js, "old = e.mem.buffer; [e.mem.grow(0), old.byteLength].join()"), "3,0");
    assert_eq!(run(&mut js, "try { e.mem.grow(1) } catch (x) { x.constructor.name + ' ' + e.mem.buffer.byteLength }"), "RangeError 196608");
    assert_eq!(run(&mut js, "e.grow(1)"), "-1");
}

#[test]
fn memory_from_script() {
    let mut js = page(r#"(module (import "env" "mem" (memory 1)) (func (export "peek") (param i32) (result i32) local.get 0 i32.load))"#);
    run(&mut js, "var mem = new WebAssembly.Memory({initial: 1, maximum: 4}); new Uint32Array(mem.buffer)[1] = 1234567");
    assert_eq!(run(&mut js, "new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {mem}}).exports.peek(4)"), "1234567");
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {mem: {}}}) } catch (x) { x.name }"), "LinkError");
    let small = run(&mut js, "try { new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {mem: new WebAssembly.Memory({initial: 0})}}); 'ok' } catch (x) { x.name }");
    assert_eq!(small, "LinkError");
    for (desc, want) in [
        ("{}", "TypeError"),
        ("{initial: 70000}", "RangeError"),
        ("{initial: 2, maximum: 1}", "RangeError"),
        ("{initial: 1, maximum: 2, shared: true}", "TypeError"),
        ("{initial: -1}", "TypeError"),
    ] {
        assert_eq!(run(&mut js, &format!("try {{ new WebAssembly.Memory({desc}); 'ok' }} catch (x) {{ x.constructor.name }}")), want, "{desc}");
    }
    assert_eq!(run(&mut js, "new WebAssembly.Memory({initial: 2}).buffer.byteLength"), "131072");
}

#[test]
fn tables_both_ways() {
    let mut js = page(r#"(module
      (import "env" "ext" (table $x 1 externref))
      (table $t (export "tbl") 2 funcref)
      (func $one (export "one") (result i32) i32.const 1)
      (func $two (result i32) i32.const 2)
      (elem (table $t) (i32.const 0) func $one $two)
      (type $r (func (result i32)))
      (func (export "callAt") (param i32) (result i32) local.get 0 call_indirect $t (type $r))
      (func (export "extAt") (param i32) (result externref) local.get 0 table.get $x))"#);
    run(&mut js, "var ext = new WebAssembly.Table({element: 'externref', initial: 1}); var tag = {}; ext.set(0, tag); var e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {ext}}).exports");
    assert_eq!(run(&mut js, "e.tbl instanceof WebAssembly.Table && e.tbl.length"), "2");
    assert_eq!(run(&mut js, "e.tbl.get(0) === e.one && e.tbl.get(1)()"), "2");
    assert_eq!(run(&mut js, "e.tbl.set(1, e.one); e.callAt(1)"), "1");
    assert_eq!(run(&mut js, "[e.tbl.grow(1), e.tbl.length, e.tbl.get(2)].join()"), "2,3,");
    assert_eq!(run(&mut js, "try { e.callAt(2) } catch (x) { x.name }"), "RuntimeError");
    assert_eq!(run(&mut js, "try { e.tbl.get(3) } catch (x) { x.constructor.name }"), "RangeError");
    assert_eq!(run(&mut js, "try { e.tbl.set(0, () => 1) } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "e.extAt(0) === tag && ext.get(0) === tag"), "true");
    assert_eq!(run(&mut js, "ext.grow(1, 'x'); ext.get(1) + ' ' + e.extAt(1)"), "x x");
    assert_eq!(run(&mut js, "try { new WebAssembly.Table({element: 'i32', initial: 1}) } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "new WebAssembly.Table({element: 'anyfunc', initial: 2}).get(1)"), "null");
}

#[test]
fn globals_both_ways() {
    let mut js = page(r#"(module
      (import "env" "shared" (global $s (mut f64)))
      (global $g (export "g") (mut i32) (i32.const 5))
      (global (export "k") i64 (i64.const 9))
      (func (export "bump") global.get $g i32.const 1 i32.add global.set $g)
      (func (export "readShared") (result f64) global.get $s)
      (func (export "setShared") (param f64) local.get 0 global.set $s))"#);
    run(&mut js, "var shared = new WebAssembly.Global({value: 'f64', mutable: true}, 1.5); var m = new WebAssembly.Module(bytes); var e = new WebAssembly.Instance(m, {env: {shared}}).exports");
    assert_eq!(run(&mut js, "e.g instanceof WebAssembly.Global && e.g.value"), "5");
    assert_eq!(run(&mut js, "e.bump(); e.g.value"), "6");
    assert_eq!(run(&mut js, "e.g.value = 40; e.bump(); e.g.valueOf()"), "41");
    assert_eq!(run(&mut js, "typeof e.k.value + ' ' + e.k.value"), "bigint 9");
    assert_eq!(run(&mut js, "try { e.k.value = 1n } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "e.readShared()"), "1.5");
    assert_eq!(run(&mut js, "e.setShared(2.25); shared.value"), "2.25");
    assert_eq!(run(&mut js, "shared.value = 3; e.readShared()"), "3");
    // A mutable global import must be a Global of exactly that type.
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(m, {env: {shared: 1.5}}) } catch (x) { x.name }"), "LinkError");
    assert_eq!(run(&mut js, "try { new WebAssembly.Instance(m, {env: {shared: new WebAssembly.Global({value: 'f32', mutable: true})}}) } catch (x) { x.name }"), "LinkError");
    // Defaults and types.
    assert_eq!(run(&mut js, "[new WebAssembly.Global({value: 'i32'}).value, String(new WebAssembly.Global({value: 'i64'}).value), new WebAssembly.Global({value: 'anyfunc'}).value].join()"), "0,0,");
    assert_eq!(run(&mut js, "try { new WebAssembly.Global({value: 'i64'}, 1) } catch (x) { x.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "try { new WebAssembly.Global({value: 'x'}) } catch (x) { x.constructor.name }"), "TypeError");
}

#[test]
fn exported_items_are_shared_between_instances() {
    let mut js = page(r#"(module (memory (export "mem") 1) (global (export "g") (mut i32) (i32.const 0)))"#);
    run(&mut js, "var a = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports");
    let user = r#"(module (import "a" "mem" (memory 1)) (import "a" "g" (global (mut i32)))
        (func (export "poke") i32.const 0 i32.const 77 i32.store8 i32.const 8 global.set 0))"#;
    let bin = wat::parse_str(user).unwrap();
    let list: Vec<String> = bin.iter().map(|b| b.to_string()).collect();
    run(&mut js, &format!("var b = new WebAssembly.Instance(new WebAssembly.Module(new Uint8Array([{}])), {{a}}).exports", list.join(",")));
    assert_eq!(run(&mut js, "b.poke(); [new Uint8Array(a.mem.buffer)[0], a.g.value].join()"), "77,8");
}
