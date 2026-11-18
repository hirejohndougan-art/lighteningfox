//! Tests for WebAssembly loading (P1c): streaming, `.wasm` ES modules, CSP, workers and
//! postMessage.

use crate::js::Js;
use crate::security::{Csp, PagePolicy};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SEVEN: &str = r#"(module (func (export "seven") (result i32) i32.const 7))"#;

fn bytes_js(wat: &str) -> String {
    let bin = wat::parse_str(wat).expect("test module parses");
    let list: Vec<String> = bin.iter().map(|b| b.to_string()).collect();
    format!("new Uint8Array([{}])", list.join(","))
}

fn page_with(url: &str, policy: Option<&str>, wat: &str) -> Js {
    let mut js = match policy {
        Some(p) => Js::with_policy(
            url,
            Arc::new(PagePolicy {
                csp: Csp::new(url, &[("content-security-policy".into(), p.into())], &[]),
                frame_options: None,
                sandbox: None,
            }),
        ),
        None => Js::new(url),
    };
    js.run(&format!("var bytes = {};", bytes_js(wat))).expect("bytes");
    js
}

fn run(js: &mut Js, code: &str) -> String {
    let out = js.run(code).unwrap_or_else(|e| format!("error: {e}"));
    if out.starts_with('"') {
        return serde_json::from_str(&out).unwrap_or(out);
    }
    out
}

#[test]
fn compile_and_instantiate_streaming() {
    let mut js = page_with("https://wasm.example/", None, SEVEN);
    run(&mut js, "var out = []; var wasmResponse = () => new Response(bytes, { headers: { 'Content-Type': 'application/wasm; charset=x' } });");
    run(&mut js, "WebAssembly.compileStreaming(wasmResponse()).then(m => out.push('module:' + (m instanceof WebAssembly.Module)))");
    run(&mut js, "WebAssembly.instantiateStreaming(Promise.resolve(wasmResponse())).then(r => out.push('seven:' + r.instance.exports.seven()))");
    // Wrong type, not ok, opaque, already read, not a Response.
    run(&mut js, "WebAssembly.compileStreaming(new Response(bytes, { headers: { 'Content-Type': 'application/octet-stream' } })).catch(e => out.push('mime:' + e.name))");
    run(&mut js, "WebAssembly.compileStreaming(new Response(bytes, { status: 404, headers: { 'Content-Type': 'application/wasm' } })).catch(e => out.push('status:' + e.name))");
    run(&mut js, "WebAssembly.compileStreaming(new Response(bytes, { type: 'opaque', headers: { 'Content-Type': 'application/wasm' } })).catch(e => out.push('opaque:' + e.name))");
    run(&mut js, "var used = wasmResponse(); used.arrayBuffer().then(() => WebAssembly.compileStreaming(used)).catch(e => out.push('used:' + e.name))");
    run(&mut js, "WebAssembly.compileStreaming(bytes).catch(e => out.push('bytes:' + e.name))");
    // Bad module bytes are a CompileError.
    run(&mut js, "WebAssembly.compileStreaming(new Response(new Uint8Array([0, 97, 115, 109, 9]), { headers: { 'Content-Type': 'application/wasm' } })).catch(e => out.push('bad:' + e.name))");
    let r = run(&mut js, "out.sort().join()");
    assert_eq!(r, "bad:CompileError,bytes:TypeError,mime:TypeError,module:true,opaque:TypeError,seven:7,status:TypeError,used:TypeError");
    assert_eq!(run(&mut js, "WebAssembly.compileStreaming.length + ' ' + WebAssembly.instantiateStreaming.name + ' ' + Object.keys(WebAssembly).length"), "1 instantiateStreaming 0");
}

#[test]
fn csp_wasm_unsafe_eval() {
    // Blocked: script-src without either keyword. compile, instantiate and new Module all fail.
    let mut js = page_with("https://wasm.example/", Some("script-src 'self'"), SEVEN);
    assert_eq!(run(&mut js, "try { new WebAssembly.Module(bytes); 'compiled' } catch (e) { e.name }"), "CompileError");
    run(&mut js, "var out = []; WebAssembly.instantiate(bytes).catch(e => out.push(e.name)); WebAssembly.compile(bytes).catch(e => out.push(e.name));");
    assert_eq!(run(&mut js, "out.join()"), "CompileError,CompileError");
    assert_eq!(run(&mut js, "WebAssembly.validate(bytes)"), "true");
    // Allowed by 'wasm-unsafe-eval' (eval itself stays blocked) or 'unsafe-eval'.
    let mut js = page_with("https://wasm.example/", Some("script-src 'self' 'wasm-unsafe-eval'"), SEVEN);
    assert_eq!(run(&mut js, "new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports.seven()"), "7");
    assert_eq!(run(&mut js, "try { eval('1'); 'eval ran' } catch (e) { e.name }"), "EvalError");
    let mut js = page_with("https://wasm.example/", Some("default-src 'self' 'unsafe-eval'"), SEVEN);
    assert_eq!(run(&mut js, "new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports.seven()"), "7");
}

#[test]
fn module_survives_structured_clone() {
    let mut js = page_with("https://wasm.example/", None, SEVEN);
    run(&mut js, "var m = new WebAssembly.Module(bytes); var c = structuredClone({ m });");
    assert_eq!(run(&mut js, "(c.m instanceof WebAssembly.Module) + ' ' + (c.m !== m) + ' ' + new WebAssembly.Instance(c.m).exports.seven()"), "true true 7");
    assert_eq!(run(&mut js, "WebAssembly.Module.exports(c.m)[0].name"), "seven");
    // Something that only looks like a Module can't be cloned.
    assert_eq!(run(&mut js, "try { structuredClone(Object.create(WebAssembly.Module.prototype)); 'cloned' } catch (e) { e.name }"), "TypeError");
}

fn wait_for(epoch: u64, what: &str) -> Vec<String> {
    let t = Instant::now();
    let mut seen = Vec::new();
    while t.elapsed() < Duration::from_secs(10) {
        seen.extend(crate::worker::take_events(epoch));
        if seen.iter().any(|e| e.contains(what)) {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("never saw {what}: {seen:?}");
}

fn start_worker(epoch: u64, src: String) {
    assert!(crate::worker::start(crate::worker::Start {
        epoch,
        id: 1,
        url: "blob:null/wasm-test".into(),
        source: Some(src),
        name: "w".into(),
        page: "https://wasm.example/".into(),
        top: "https://wasm.example/".into(),
        policy: Default::default(),
    }));
}

#[test]
fn webassembly_in_a_worker() {
    let epoch = 810_001;
    start_worker(epoch, format!("const wasmBytes = {}; WebAssembly.instantiate(wasmBytes).then(r => postMessage('worker seven ' + r.instance.exports.seven()));", bytes_js(SEVEN)));
    let ev = wait_for(epoch, "worker seven 7");
    assert!(!ev.iter().any(|e| e.contains("\"error\"")), "{ev:?}");
    crate::worker::terminate(epoch, 1);
}

#[test]
fn module_posted_to_a_worker() {
    let epoch = 810_002;
    start_worker(epoch, "onmessage = (e) => postMessage('got ' + (e.data instanceof WebAssembly.Module) + ' ' + new WebAssembly.Instance(e.data).exports.seven());".into());
    let mut page = page_with("https://wasm.example/", None, SEVEN);
    let encoded = run(&mut page, "__cloneOut(new WebAssembly.Module(bytes))");
    assert!(encoded.contains("\"$\":\"wasm\""), "{encoded}");
    crate::worker::post(epoch, 1, format!("{{\"ports\":[],\"m\":{encoded}}}"));
    wait_for(epoch, "got true 7");
    crate::worker::terminate(epoch, 1);
}

/// A scratch folder of files for `file://` module tests.
fn scratch(name: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ironswift-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    for (file, data) in files {
        std::fs::write(dir.join(file), data).expect("scratch file");
    }
    dir
}

#[test]
fn wasm_es_module() {
    let wasm = wat::parse_str(
        r#"(module
          (import "./env.js" "base" (func $base (result i32)))
          (import "./env.js" "note" (func $note (param i32)))
          (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add call $base i32.add)
          (func (export "odd name!") (result i32) i32.const 1 call $note i32.const 5))"#,
    )
    .unwrap();
    let dir = scratch(
        "wasm-esm",
        &[
            ("math.wasm", &wasm),
            ("env.js", b"export function base() { return 100; }\nexport function note(x) { globalThis.noted = x; }\n"),
            ("main.js", b"import { add } from './math.wasm';\nglobalThis.viaStatic = add(1, 2);\n"),
            ("page.html", b"<!doctype html>"),
        ],
    );
    let url = format!("file://{}/page.html", dir.display());
    let mut js = Js::new(&url);
    run(&mut js, "var out = []; import('./math.wasm').then(ns => out.push(ns.add(2, 3), ns['odd name!'](), globalThis.noted, Object.keys(ns).join('|')), e => out.push('error ' + e))");
    assert_eq!(run(&mut js, "out.join()"), "105,5,1,add|odd name!");
    // A static import from a JavaScript module, and the same URL gives the same module.
    run(&mut js, "import('./main.js').then(() => import('./math.wasm')).then(ns => out.push(viaStatic, ns.add === ns.add))");
    assert_eq!(run(&mut js, "out.slice(4).join()"), "103,true");
    let _ = std::fs::remove_dir_all(&dir);
}
