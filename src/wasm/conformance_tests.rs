//! Fixes found running WPT `wasm/jsapi` and `wasm/webapi` (P1d).

use super::tests::{page, run};

#[test]
fn imports_keep_the_modules_order() {
    // wasmi groups imports by kind; the JS API reads and lists them in the module's order.
    let mut js = page(r#"(module
      (import "m" "g1" (global i32))
      (import "m" "mem" (memory 1))
      (import "m" "f" (func (result i32)))
      (import "m" "g2" (global i32))
      (func (export "sum") (result i32) call 0 global.get 0 i32.add global.get 1 i32.add))"#);
    run(&mut js, "var m = new WebAssembly.Module(bytes)");
    assert_eq!(run(&mut js, "WebAssembly.Module.imports(m).map(i => i.name + ':' + i.kind).join()"), "g1:global,mem:memory,f:function,g2:global");
    let r = run(&mut js, "var order = []; var ns = {get g1() { order.push('g1'); return 1 }, get mem() { order.push('mem'); return new WebAssembly.Memory({initial: 1}) }, get f() { order.push('f'); return () => 100 }, get g2() { order.push('g2'); return 20 }}; var e = new WebAssembly.Instance(m, {m: ns}).exports; order.join() + ' ' + e.sum()");
    assert_eq!(r, "g1,mem,f,g2 121");
}

#[test]
fn instantiate_reads_imports_at_once() {
    let mut js = page(r#"(module (import "m" "g" (global i32)))"#);
    run(&mut js, "var m = new WebAssembly.Module(bytes); var order = []");
    let r = run(&mut js, "WebAssembly.instantiate(m, {get m() { order.push('m'); return {get g() { order.push('g'); return 0 }} }}); order.join()");
    assert_eq!(r, "m,g");
}

#[test]
fn failed_grow_keeps_the_buffer() {
    let mut js = page("(module)");
    let r = run(&mut js, "var mem = new WebAssembly.Memory({initial: 0, maximum: 1}); var b = mem.buffer; try { mem.grow(2) } catch (e) {} b === mem.buffer");
    assert_eq!(r, "true");
}

#[test]
fn descriptors_read_like_webidl() {
    let mut js = page("(module)");
    let r = run(&mut js, "var log = []; var d = {get initial() { log.push('initial'); return {valueOf() { log.push('initial valueOf'); return 1 }} }, get maximum() { log.push('maximum'); return {valueOf() { log.push('maximum valueOf'); return 2 }} }}; new WebAssembly.Memory(d); log.join()");
    assert_eq!(r, "initial,initial valueOf,maximum,maximum valueOf");
    assert_eq!(run(&mut js, "try { new WebAssembly.Table({element: 'anyfunc', initial: 1, minimum: 1}) } catch (e) { e.constructor.name }"), "TypeError");
    assert_eq!(run(&mut js, "new WebAssembly.Global({value: 'i64'}, undefined).value === 0n"), "true");
}

#[test]
fn namespace_and_interface_shapes() {
    let mut js = page("(module)");
    let r = run(&mut js, "['validate', 'compile', 'instantiate', 'Module'].map(k => Object.getOwnPropertyDescriptor(WebAssembly, k).enumerable).join()");
    assert_eq!(r, "true,true,true,false");
    assert_eq!(run(&mut js, "[WebAssembly.instantiate.length, WebAssembly.Instance.length, WebAssembly.Table.length, WebAssembly.Memory.length].join()"), "1,1,1,1");
    assert_eq!(run(&mut js, "Object.getOwnPropertyDescriptor(WebAssembly.Module, 'exports').enumerable"), "true");
}

#[test]
fn wasm_objects_are_not_cloneable() {
    let mut js = page("(module)");
    let r = run(&mut js, "try { structuredClone(new WebAssembly.Memory({initial: 0})); 'copied' } catch (e) { e.name }");
    assert_eq!(r, "DataCloneError");
}
