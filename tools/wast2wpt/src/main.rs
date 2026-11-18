//! wast2wpt: turn WebAssembly spec tests (`.wast` files) into testharness.js pages that run
//! every assertion through the JavaScript API, for `tools/wasm_spec.py`.
//!
//!     wast2wpt OUT_DIR file.wast…
//!
//! WPT's own copy of the suite (`wasm/core`) checks results with wrapper modules that use the
//! GC proposal's types, so an engine without GC fails nearly every assertion there. These pages
//! check results in JavaScript instead: numbers exactly (`Object.is`, so -0 counts), NaNs as
//! NaN (script can't see a NaN's payload), references by identity. Assertions that need what
//! script can't express (v128 values, GC references, exceptions, threads) are left out and
//! counted in the page's `skipped` comment.

use std::fmt::Write as _;

use wast::core::{HeapType, NanPattern, WastArgCore, WastRetCore};
use wast::parser::{self, ParseBuffer};
use wast::{QuoteWat, Wast, WastArg, WastDirective, WastExecute, WastInvoke, WastRet, Wat};

const HARNESS: &str = include_str!("harness.js");

/// The spec tests' `spectest` module, as wasm (so its functions and globals have exact types
/// and importing one at the wrong type is a LinkError, as the tests expect).
const SPECTEST: &str = r#"(module
  (func (export "print"))
  (func (export "print_i32") (param i32))
  (func (export "print_i64") (param i64))
  (func (export "print_f32") (param f32))
  (func (export "print_f64") (param f64))
  (func (export "print_i32_f32") (param i32 f32))
  (func (export "print_f64_f64") (param f64 f64))
  (global (export "global_i32") i32 (i32.const 666))
  (global (export "global_i64") i64 (i64.const 666))
  (global (export "global_f32") f32 (f32.const 666.6))
  (global (export "global_f64") f64 (f64.const 666.6))
  (table (export "table") 10 20 funcref)
  (memory (export "memory") 1 2))"#;

fn spectest_bytes() -> Vec<u8> {
    let buf = ParseBuffer::new(SPECTEST).expect("spectest");
    let mut wat: Wat = parser::parse(&buf).expect("spectest");
    wat.encode().expect("spectest")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((out, files)) = args.split_first() else {
        eprintln!("usage: wast2wpt OUT_DIR file.wast…");
        std::process::exit(2);
    };
    std::fs::create_dir_all(out).expect("output folder");
    let harness = format!("const SPECTEST_WASM = {};\n{HARNESS}", bytes_js(&spectest_bytes()));
    std::fs::write(format!("{out}/harness.js"), harness).expect("write harness");
    for file in files {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{file}: {e}");
                continue;
            }
        };
        let name = std::path::Path::new(file).file_name().unwrap().to_string_lossy().to_string();
        match convert(&name, &text) {
            Ok((_, 0, skipped)) => println!("{name}: nothing to run ({skipped} skipped)"),
            Ok((js, tests, skipped)) => {
                let page = format!(
                    "<!doctype html>\n<meta charset=utf-8>\n<title>{name}</title>\n<!-- {tests} assertions, {skipped} skipped -->\n\
                     <script src=\"/resources/testharness.js\"></script>\n<script src=\"/resources/testharnessreport.js\"></script>\n\
                     <script src=\"harness.js\"></script>\n<div id=log></div>\n<script>\n{js}</script>\n"
                );
                std::fs::write(format!("{out}/{name}.html"), page).expect("write page");
                println!("{name}: {tests} assertions, {skipped} skipped");
            }
            Err(e) => eprintln!("{name}: can't parse: {e}"),
        }
    }
}

/// Bytes as a JS string literal of byte values ("\x00asm…"), as WPT's own pages do.
fn bytes_js(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 4 + 2);
    s.push('"');
    for &c in b {
        let _ = write!(s, "\\x{c:02x}");
    }
    s.push('"');
    s
}

fn js_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                for u in c.encode_utf16(&mut [0; 2]) {
                    let _ = write!(out, "\\u{u:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn f64_js(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if x == 0.0 && x.is_sign_negative() {
        "-0".into()
    } else {
        format!("{x:?}")
    }
}

/// A JS expression for an argument, or None if script can't pass it.
fn arg_js(a: &WastArg) -> Option<String> {
    let WastArg::Core(a) = a else { return None };
    Some(match a {
        WastArgCore::I32(x) => x.to_string(),
        WastArgCore::I64(x) => format!("{x}n"),
        // Script has one NaN: a NaN with a sign or payload can't be passed.
        WastArgCore::F32(x) if f32::from_bits(x.bits).is_nan() && x.bits != 0x7fc0_0000 => return None,
        WastArgCore::F64(x) if f64::from_bits(x.bits).is_nan() && x.bits != 0x7ff8_0000_0000_0000 => return None,
        WastArgCore::F32(x) => f64_js(f64::from(f32::from_bits(x.bits))),
        WastArgCore::F64(x) => f64_js(f64::from_bits(x.bits)),
        WastArgCore::RefNull(HeapType::Abstract { .. }) => "null".into(),
        WastArgCore::RefExtern(n) => format!("externref({n})"),
        _ => return None,
    })
}

/// A JS expression checking `v` against an expected result, or None if script can't check it.
fn ret_check(r: &WastRetCore, v: &str) -> Option<String> {
    Some(match r {
        WastRetCore::I32(x) => format!("{v} === {x}"),
        WastRetCore::I64(x) => format!("{v} === {x}n"),
        WastRetCore::F32(NanPattern::Value(x)) => format!("Object.is({v}, {})", f64_js(f64::from(f32::from_bits(x.bits)))),
        WastRetCore::F64(NanPattern::Value(x)) => format!("Object.is({v}, {})", f64_js(f64::from_bits(x.bits))),
        WastRetCore::F32(_) | WastRetCore::F64(_) => format!("Number.isNaN({v})"),
        WastRetCore::RefNull(_) => format!("{v} === null"),
        WastRetCore::RefExtern(Some(n)) => format!("{v} === externref({n})"),
        WastRetCore::RefExtern(None) => format!("({v} !== null)"),
        WastRetCore::RefFunc(None) => format!("typeof {v} === \"function\""),
        WastRetCore::RefFunc(Some(_)) => format!("typeof {v} === \"function\""),
        WastRetCore::Either(alts) => {
            let checks: Option<Vec<String>> = alts.iter().map(|a| ret_check(a, v)).collect();
            format!("({})", checks?.join(" || "))
        }
        _ => return None,
    })
}

fn id_js(id: Option<wast::token::Id>) -> String {
    id.map_or_else(|| "null".into(), |i| js_str(i.name()))
}

fn invoke_js(i: &WastInvoke) -> Option<String> {
    let args: Option<Vec<String>> = i.args.iter().map(arg_js).collect();
    Some(format!("invoke({}, {}, [{}])", id_js(i.module), js_str(i.name), args?.join(", ")))
}

fn exec_js(e: &mut WastExecute) -> Option<String> {
    Some(match e {
        WastExecute::Invoke(i) => invoke_js(i)?,
        WastExecute::Get { module, global, .. } => format!("getGlobal({}, {})", id_js(*module), js_str(global)),
        WastExecute::Wat(w) => {
            let bytes = w.encode().ok()?;
            format!("instantiate({})", bytes_js(&bytes))
        }
    })
}

fn module_id<'a>(q: &QuoteWat<'a>) -> Option<wast::token::Id<'a>> {
    match q {
        QuoteWat::Wat(Wat::Module(m)) => m.id,
        _ => None,
    }
}

fn convert(name: &str, text: &str) -> Result<(String, usize, usize), String> {
    // names.wast uses bidi and other look-alike characters on purpose.
    let mut lexer = wast::lexer::Lexer::new(text);
    lexer.allow_confusing_unicode(true);
    let buf = ParseBuffer::new_with_lexer(lexer).map_err(|e| e.to_string())?;
    let wast: Wast = parser::parse(&buf).map_err(|e| e.to_string())?;
    let mut js = String::new();
    let (mut tests, mut skipped) = (0, 0);
    for directive in wast.directives {
        let (line, _) = directive.span().linecol_in(text);
        let at = js_str(&format!("{name}:{}", line + 1));
        let mut emit = |body: String| {
            let _ = writeln!(js, "step({at}, () => {{ {body} }});");
        };
        match directive {
            WastDirective::Module(mut q) => {
                let id = module_id(&q);
                match q.encode() {
                    Ok(b) => {
                        emit(format!("define({}, () => instantiate({}));", id_js(id), bytes_js(&b)));
                        tests += 1;
                    }
                    Err(_) => skipped += 1,
                }
            }
            WastDirective::ModuleDefinition(mut q) => {
                let id = module_id(&q);
                match q.encode() {
                    Ok(b) => {
                        emit(format!("defineModule({}, {});", id_js(id), bytes_js(&b)));
                        tests += 1;
                    }
                    Err(_) => skipped += 1,
                }
            }
            WastDirective::ModuleInstance { instance, module, .. } => {
                emit(format!("define({}, instantiateDefined({}, {}));", id_js(instance), id_js(module), id_js(instance)));
                tests += 1;
            }
            WastDirective::Register { name, module, .. } => {
                emit(format!("register({}, {});", js_str(name), id_js(module)));
                tests += 1;
            }
            WastDirective::Invoke(i) => match invoke_js(&i) {
                Some(call) => {
                    emit(format!("{call};"));
                    tests += 1;
                }
                None => skipped += 1,
            },
            WastDirective::AssertReturn { mut exec, results, .. } => {
                let call = exec_js(&mut exec);
                let checks: Option<Vec<String>> = results
                    .iter()
                    .enumerate()
                    .map(|(i, r)| match r {
                        WastRet::Core(r) => ret_check(r, &format!("r[{i}]")),
                        _ => None,
                    })
                    .collect();
                match (call, checks) {
                    (Some(call), Some(checks)) => {
                        let checks = if checks.is_empty() { "true".to_string() } else { checks.join(" && ") };
                        emit(format!("const r = results({call}, {}); check(r, {checks});", results.len()));
                        tests += 1;
                    }
                    _ => skipped += 1,
                }
            }
            WastDirective::AssertTrap { mut exec, message, .. } => match exec_js(&mut exec) {
                Some(call) => {
                    emit(format!("throwsRuntimeError(() => {call}, {});", js_str(message)));
                    tests += 1;
                }
                None => skipped += 1,
            },
            WastDirective::AssertExhaustion { call, message, .. } => match invoke_js(&call) {
                Some(call) => {
                    emit(format!("throwsExhaustion(() => {call}, {});", js_str(message)));
                    tests += 1;
                }
                None => skipped += 1,
            },
            WastDirective::AssertInvalid { mut module, .. } | WastDirective::AssertMalformed { mut module, .. } => {
                // A text module that doesn't even parse tests the text format, not the engine.
                match module.encode() {
                    Ok(b) => {
                        emit(format!("invalid({});", bytes_js(&b)));
                        tests += 1;
                    }
                    Err(_) => skipped += 1,
                }
            }
            WastDirective::AssertUnlinkable { mut module, .. } => match module.encode() {
                Ok(b) => {
                    emit(format!("unlinkable({});", bytes_js(&b)));
                    tests += 1;
                }
                Err(_) => skipped += 1,
            },
            // Custom-section validation, exceptions, stack switching and threads: nothing
            // script can check here.
            _ => skipped += 1,
        }
    }
    Ok((js, tests, skipped))
}
