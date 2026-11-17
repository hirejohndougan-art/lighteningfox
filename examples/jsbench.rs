//! Runs JavaScript files in a bare Boa context and times them. Used to measure Boa speed work
//! (batch M). Not part of the browser.
//!
//!   cargo run --release --example jsbench -- file1.js file2.js ...
//!   cargo run --release --example jsbench -- --v8            # the V8 benchmark suite
//!   cargo run --release --example jsbench -- --v8 --quick     # no warm-up, about 10 s
//!   cargo run --release --example jsbench -- --v8 Richards    # one benchmark of the suite
//!
//! Scripts may call `print(...)` and `load(path)`. Prints the time each file took.

use boa_engine::{js_string, Context, JsResult, JsValue, NativeFunction, Source};
use boa_engine::property::Attribute;
use std::path::{Path, PathBuf};
use std::time::Instant;

const V8_FILES: [&str; 8] = [
    "richards.js",
    "deltablue.js",
    "crypto.js",
    "raytrace.js",
    "earley-boyer.js",
    "regexp.js",
    "splay.js",
    "navier-stokes.js",
];

fn print(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mut parts = Vec::new();
    for a in args {
        parts.push(a.to_string(ctx)?.to_std_string_escaped());
    }
    println!("{}", parts.join(" "));
    Ok(JsValue::undefined())
}

fn load(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let path = args.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
    let code = std::fs::read(&path).map_err(|e| {
        boa_engine::JsNativeError::error().with_message(format!("load {path}: {e}"))
    })?;
    ctx.eval(Source::from_bytes(&code))
}

fn new_context() -> Context {
    let mut ctx = Context::default();
    ctx.register_global_builtin_callable(js_string!("print"), 1, NativeFunction::from_fn_ptr(print))
        .unwrap();
    ctx.register_global_builtin_callable(js_string!("load"), 1, NativeFunction::from_fn_ptr(load))
        .unwrap();
    ctx
}

fn run(ctx: &mut Context, label: &str, code: &[u8]) -> bool {
    let start = Instant::now();
    let r = ctx.eval(Source::from_bytes(code));
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    match r {
        Ok(_) => {
            eprintln!("{label}: {ms:.1} ms");
            true
        }
        Err(e) => {
            eprintln!("{label}: error: {e}");
            false
        }
    }
}

fn v8_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/bench/v8")
}

const V8_RUNNER: &str = r#"
if (__quick) {
  // One timed second per benchmark, no warm-up: rougher numbers, about 10 s in all.
  BenchmarkSuite.config.doWarmup = false;
  BenchmarkSuite.suites.forEach(function (s) {
    s.benchmarks.forEach(function (b) { b.minIterations = 1; });
  });
}
BenchmarkSuite.RunSuites({
  NotifyResult: function (name, result) { print(name + ': ' + result); },
  NotifyError: function (name, error) { print(name + ': ' + error); },
  NotifyScore: function (score) { print('Score: ' + score); },
}, __skip);
"#;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let v8 = args.iter().any(|a| a == "--v8");
    args.retain(|a| a != "--quick" && a != "--v8");
    let mut ctx = new_context();
    ctx.register_global_property(js_string!("__quick"), quick, Attribute::all()).ok();
    let mut ok = true;
    if v8 {
        let dir = v8_dir();
        let base = std::fs::read(dir.join("base.js")).expect("tools/bench/v8/base.js");
        ok &= run(&mut ctx, "base.js", &base);
        for f in V8_FILES {
            let code = std::fs::read(dir.join(f)).expect("benchmark file");
            ok &= run(&mut ctx, f, &code);
        }
        // Names of suites to skip when only some were asked for.
        let skip = if args.is_empty() {
            "[]".to_string()
        } else {
            let wanted: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
            format!(
                "BenchmarkSuite.suites.map(function (s) {{ return s.name; }}).filter(function (n) {{ return [{}].indexOf(n) < 0; }})",
                wanted.join(",")
            )
        };
        let runner = format!("var __skip = {skip};\n{V8_RUNNER}");
        ok &= run(&mut ctx, "suite", runner.as_bytes());
    } else {
        for f in &args {
            let code = std::fs::read(f).unwrap_or_else(|e| panic!("{f}: {e}"));
            ok &= run(&mut ctx, f, &code);
        }
    }
    if !ok {
        std::process::exit(1);
    }
}
