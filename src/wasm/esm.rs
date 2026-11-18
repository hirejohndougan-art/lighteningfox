//! `.wasm` ES modules (task P1c, the WebAssembly/ES module integration proposal):
//! `import { add } from "./math.wasm"`.
//!
//! The module loader (`src/jsmodules.rs`) fetches a module served as `application/wasm` and
//! hands its bytes here. This compiles them and returns the source of a small JavaScript
//! module, parsed in its place with the `.wasm` URL as its own, so its imports resolve against
//! the `.wasm` file. Each module the wasm imports from becomes an `import * as`, and the
//! instance's exports become the module's exports:
//!
//! ```js
//! import * as i0 from "./env.js";
//! const x = __wasmEsmInstance(3, { "./env.js": i0 });
//! const e0 = x["add"]; export { e0 as "add" };
//! ```
//!
//! Exports are copied when the module runs, so they are fixed values rather than live
//! bindings (a mutable global's later changes aren't seen through the import).

use super::{compile_bytes, module_of, state};
use boa_engine::{Context, JsResult};
use std::sync::Arc;

/// The JavaScript module that stands for the WebAssembly module in `bytes`.
pub(crate) fn wrapper(ctx: &mut Context, bytes: Vec<u8>) -> JsResult<String> {
    let id = compile_bytes(ctx, Arc::from(bytes))?;
    let st = state(ctx);
    let module = module_of(&st, id)?;
    let mut specifiers: Vec<String> = Vec::new();
    for import in module.imports() {
        let m = import.module().to_string();
        if !specifiers.contains(&m) {
            specifiers.push(m);
        }
    }
    let exports: Vec<String> = st.tables.borrow().modules[id].exports.iter().map(|(name, _, _)| name.clone()).collect();

    let quote = |s: &str| {
        let mut o = String::new();
        crate::js::json_str(s, &mut o);
        o
    };
    let mut src = String::new();
    for (i, spec) in specifiers.iter().enumerate() {
        src.push_str(&format!("import * as i{i} from {};\n", quote(spec)));
    }
    let imports: Vec<String> = specifiers.iter().enumerate().map(|(i, spec)| format!("{}: i{i}", quote(spec))).collect();
    src.push_str(&format!("const x = __wasmEsmInstance({id}, {{ {} }});\n", imports.join(", ")));
    for (i, name) in exports.iter().enumerate() {
        let name = quote(name);
        src.push_str(&format!("const e{i} = x[{name}]; export {{ e{i} as {name} }};\n"));
    }
    Ok(src)
}
