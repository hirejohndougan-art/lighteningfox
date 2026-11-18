//! WebAssembly (task P1a): the `WebAssembly` global, on the `wasmi` interpreter.
//!
//! Why wasmi and not wasmtime: see "WebAssembly" in `docs/ARCHITECTURE.md`.
//!
//! The JavaScript side (`src/wasm.js`) builds the API (`Module`, `Instance`, `compile`,
//! `instantiate`, `validate`, the error classes) on a few natives here that work on module
//! ids. Every page (script engine) has one wasmi [`Store`]; all its instances live in it, so
//! one instance's exports can be another's imports. Nothing in a store is freed before the
//! page goes away (wasmi has no garbage collection of store items).
//!
//! Calls can nest: script calls a wasm export, which calls a JavaScript import, which calls
//! another export. The outermost call borrows the store; while a JavaScript import runs, the
//! wasm `Caller` it was given stands in for the store (`CALLERS`), and the script engine the
//! outermost call came from is reached through `CONTEXTS`.

mod objects;
mod values;
// Streaming, `.wasm` ES modules, CSP and postMessage (task P1c).
mod csp;
pub(crate) mod esm;
mod share;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod loading_tests;
#[cfg(test)]
mod objects_tests;
#[cfg(test)]
mod conformance_tests;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use boa_engine::error::RuntimeLimitError;
use boa_engine::object::builtins::{AlignedVec, JsArray, JsArrayBuffer, JsUint8Array};
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::{Context, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction};
use wasmi::{AsContextMut, Caller, Engine, Extern, ExternType, Func, FuncType, Module, Store, StoreContextMut, TrapCode, Val};

/// How much work one call from script into WebAssembly may do (wasmi "fuel", about one unit
/// per instruction) before it's stopped as a runaway script, like Boa's loop limit for
/// JavaScript. About 20 seconds of wasm on a 2-core 2026 laptop (3 billion units a second).
pub(crate) const FUEL: u64 = 64_000_000_000;

thread_local! {
    /// Test hook: a smaller fuel budget (0 = use [`FUEL`]).
    static FUEL_OVERRIDE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The `Caller`s of the JavaScript imports running now, innermost last.
    static CALLERS: RefCell<Vec<*mut Caller<'static, Host>>> = const { RefCell::new(Vec::new()) };
    /// The script engines that called into wasm, innermost last.
    static CONTEXTS: RefCell<Vec<*mut Context>> = const { RefCell::new(Vec::new()) };
    /// Exceptions thrown by JavaScript imports, on their way out through wasm frames.
    static THROWN: RefCell<Vec<JsError>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn set_fuel_for_tests(fuel: u64) {
    FUEL_OVERRIDE.with(|f| f.set(fuel));
}

/// What a store keeps for its host functions.
#[derive(Default)]
pub(crate) struct Host {
    /// The JavaScript functions imported by wasm, by the index their host function holds.
    pub(crate) js_funcs: Vec<JsObject>,
    /// JavaScript values held as `externref`s, by the index the `ExternRef` holds.
    pub(crate) externs: Vec<JsValue>,
}

/// An error a JavaScript import threw; the exception itself waits in `THROWN`.
#[derive(Debug)]
struct Thrown;

impl std::fmt::Display for Thrown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JavaScript exception")
    }
}

impl wasmi::errors::HostError for Thrown {}

/// A compiled module and what the JS API needs to know about it.
struct Compiled {
    module: Module,
    /// The module's bytes, for sending it to another page or worker (`share.rs`).
    bytes: Arc<[u8]>,
    /// Name, kind and index of each export, in the module's order.
    exports: Vec<(String, u8, u32)>,
    /// Module name, name and kind of each import, in the module's order.
    imports: Vec<(String, String, u8)>,
}

/// Everything but the store; borrowed only briefly, never across a call into wasm or script.
#[derive(Default)]
struct Tables {
    modules: Vec<Compiled>,
    /// The function each exported-function object calls (the index its closure holds).
    funcs: Vec<Func>,
    /// Exported-function objects by wasm function, so one function is always one object.
    objects: HashMap<String, JsObject>,
    /// The other way: which `funcs` entry an exported-function object calls.
    of_object: HashMap<JsObject, u32>,
    /// `CompileError`, `LinkError`, `RuntimeError` from `wasm.js`.
    errors: Option<[JsObject; 3]>,
    /// Memories, tables and globals script can see.
    slots: objects::Slots,
}

pub(crate) struct State {
    engine: Engine,
    store: RefCell<Store<Host>>,
    tables: RefCell<Tables>,
}

/// The page's WebAssembly state (kept in the script engine's host data).
struct Shared(Rc<State>);

fn engine_config() -> wasmi::Config {
    let mut cfg = wasmi::Config::default();
    cfg.consume_fuel(true);
    cfg
}

fn state(ctx: &mut Context) -> Rc<State> {
    if let Some(s) = ctx.get_data::<Shared>() {
        return s.0.clone();
    }
    let engine = Engine::new(&engine_config());
    let store = Store::new(&engine, Host::default());
    let s = Rc::new(State { engine, store: RefCell::new(store), tables: RefCell::new(Tables::default()) });
    ctx.insert_data(Shared(s.clone()));
    s
}

/// A wasm function's identity, for the exported-function cache (wasmi's `Func` has no `Eq`;
/// its debug form names the store and the function's slot in it).
fn func_key(f: &Func) -> String {
    format!("{f:?}")
}

/// Run `f` with the store: the innermost running import's `Caller` if wasm is running,
/// otherwise the page's store itself.
fn with_store<R>(st: &State, f: impl FnOnce(&mut StoreContextMut<'_, Host>) -> R) -> R {
    let top = CALLERS.with(|c| c.borrow().last().copied());
    match top {
        Some(p) => {
            // SAFETY: the pointer is to the `Caller` of an import that is running further up
            // this thread's stack (pushed and popped around the script call in `host_func`),
            // and nothing else uses it while script runs.
            let caller = unsafe { &mut *p };
            f(&mut caller.as_context_mut())
        }
        None => f(&mut st.store.borrow_mut().as_context_mut()),
    }
}

/// Call into wasm from script (a function call or an instantiation): sets the fuel for an
/// outermost call, makes `ctx` reachable for the imports, and turns failures into the
/// JavaScript errors the spec asks for.
fn enter<R>(ctx: &mut Context, st: &State, f: impl FnOnce(&mut StoreContextMut<'_, Host>) -> Result<R, wasmi::Error>) -> JsResult<R> {
    let outermost = CALLERS.with(|c| c.borrow().is_empty());
    CONTEXTS.with(|c| c.borrow_mut().push(ctx as *mut Context));
    let r = with_store(st, |s| {
        if outermost {
            let fuel = FUEL_OVERRIDE.with(|f| f.get());
            let _ = s.set_fuel(if fuel == 0 { FUEL } else { fuel });
        }
        f(s)
    });
    CONTEXTS.with(|c| c.borrow_mut().pop());
    // Back in script: memories the call grew get new buffers.
    objects::refresh_buffers(st);
    r.map_err(|e| js_error(ctx, st, e, Kind::Runtime))
}

#[derive(Clone, Copy)]
enum Kind {
    Compile,
    Link,
    Runtime,
}

fn throw(ctx: &mut Context, st: &State, kind: Kind, msg: &str) -> JsError {
    let ctor = st.tables.borrow().errors.as_ref().map(|e| e[kind as usize].clone());
    let Some(ctor) = ctor else {
        return JsNativeError::error().with_message(msg.to_string()).into();
    };
    match ctor.construct(&[JsValue::from(JsString::from(msg))], None, ctx) {
        Ok(o) => JsError::from_opaque(o.into()),
        Err(e) => e,
    }
}

/// The JavaScript error for a wasmi error. `default` is the class for errors that aren't
/// traps (what the call was doing: compiling, linking or running).
fn js_error(ctx: &mut Context, st: &State, e: wasmi::Error, default: Kind) -> JsError {
    if e.downcast_ref::<Thrown>().is_some() {
        if let Some(thrown) = THROWN.with(|t| t.borrow_mut().pop()) {
            return thrown;
        }
    }
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => return RuntimeLimitError::LoopIteration.into(),
        Some(TrapCode::StackOverflow) => {
            return JsNativeError::range().with_message("Maximum call stack size exceeded").into();
        }
        Some(_) => return throw(ctx, st, Kind::Runtime, &e.to_string()),
        None => {}
    }
    let kind = match e.kind() {
        wasmi::errors::ErrorKind::Instantiation(_) | wasmi::errors::ErrorKind::Linker(_) => Kind::Link,
        wasmi::errors::ErrorKind::Memory(_) | wasmi::errors::ErrorKind::Table(_) | wasmi::errors::ErrorKind::Global(_) => {
            Kind::Runtime
        }
        _ => default,
    };
    throw(ctx, st, kind, &e.to_string())
}

/// The bytes of a `Uint8Array` (`wasm.js` turns every BufferSource into one).
fn bytes_arg(a: &[JsValue], ctx: &mut Context) -> JsResult<Vec<u8>> {
    let arr = a.first().and_then(|v| v.as_object()).and_then(|o| JsUint8Array::from_object(o.clone()).ok());
    match arr {
        Some(arr) => arr.to_vec(ctx),
        None => Err(JsNativeError::typ().with_message("WebAssembly: expected a BufferSource").into()),
    }
}

fn index_arg(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<usize> {
    Ok(a.get(i).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(-1.0) as usize)
}

fn module_of(st: &State, id: usize) -> JsResult<Module> {
    st.tables
        .borrow()
        .modules
        .get(id)
        .map(|m| m.module.clone())
        .ok_or_else(|| JsNativeError::typ().with_message("WebAssembly: not a module").into())
}

pub fn install(ctx: &mut Context) {
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
        ("__wasmErrors", 3, n_errors),
        ("__wasmValidate", 1, n_validate),
        ("__wasmCompile", 1, n_compile),
        ("__wasmImports", 1, n_imports),
        ("__wasmExports", 1, n_exports),
        ("__wasmCustomSections", 2, n_custom_sections),
        ("__wasmInstantiate", 2, n_instantiate),
        ("__wasmShareNative", 1, share::n_share),
        ("__wasmAdoptNative", 1, share::n_adopt),
    ];
    for (name, len, f) in natives.iter().chain(objects::NATIVES) {
        let _ = ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f));
    }
    if let Err(e) = ctx.eval_cached(include_str!("../wasm.js").as_bytes()) {
        log!("[ironswift] wasm.js failed: {e}");
    }
    if let Err(e) = ctx.eval_cached(include_str!("../wasm_loading.js").as_bytes()) {
        log!("[ironswift] wasm_loading.js failed: {e}");
    }
}

/// __wasmErrors(CompileError, LinkError, RuntimeError)
fn n_errors(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let get = |i: usize| a.get(i).and_then(|v| v.as_object()).ok_or_else(|| JsNativeError::typ().with_message("__wasmErrors"));
    let errors = [get(0)?, get(1)?, get(2)?];
    state(ctx).tables.borrow_mut().errors = Some(errors);
    Ok(JsValue::undefined())
}

/// __wasmValidate(bytes) -> whether they are a valid module.
fn n_validate(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let bytes = bytes_arg(a, ctx)?;
    let st = state(ctx);
    Ok(Module::validate(&st.engine, &bytes).is_ok().into())
}

/// __wasmCompile(bytes) -> module id; throws a CompileError (also when the page's CSP
/// doesn't allow WebAssembly, `csp.rs`).
fn n_compile(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let bytes = bytes_arg(a, ctx)?;
    let st = state(ctx);
    csp::check(ctx, &st)?;
    Ok(compile_bytes(ctx, Arc::from(bytes))?.into())
}

/// Compile a module into this page's tables and return its id; throws a CompileError.
fn compile_bytes(ctx: &mut Context, bytes: Arc<[u8]>) -> JsResult<usize> {
    let st = state(ctx);
    let module = match Module::new(&st.engine, &bytes) {
        Ok(m) => m,
        Err(e) => return Err(throw(ctx, &st, Kind::Compile, &e.to_string())),
    };
    let exports = values::exports_in_order(&bytes);
    let imports = values::imports_in_order(&bytes);
    let mut t = st.tables.borrow_mut();
    t.modules.push(Compiled { module, bytes, exports, imports });
    Ok(t.modules.len() - 1)
}


/// __wasmImports(id) -> [[module, name, kind], …] in the module's order.
fn n_imports(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = index_arg(a, 0, ctx)?;
    let st = state(ctx);
    module_of(&st, id)?;
    const KINDS: [&str; 5] = ["function", "table", "memory", "global", "tag"];
    let list: Vec<[String; 3]> = st.tables.borrow().modules[id]
        .imports
        .iter()
        .map(|(m, n, k)| [m.clone(), n.clone(), KINDS.get(*k as usize).copied().unwrap_or("tag").to_string()])
        .collect();
    let out = JsArray::new(ctx)?;
    for [m, n, k] in list {
        let entry = JsArray::from_iter([m, n, k].map(|s| JsValue::from(JsString::from(s))), ctx);
        out.push(entry, ctx)?;
    }
    Ok(out.into())
}

/// __wasmExports(id) -> [[name, kind], …] in the module's order.
fn n_exports(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = index_arg(a, 0, ctx)?;
    let st = state(ctx);
    module_of(&st, id)?;
    const KINDS: [&str; 5] = ["function", "table", "memory", "global", "tag"];
    let list: Vec<[String; 2]> = st.tables.borrow().modules[id]
        .exports
        .iter()
        .map(|(name, kind, _)| [name.clone(), KINDS.get(*kind as usize).copied().unwrap_or("tag").to_string()])
        .collect();
    let out = JsArray::new(ctx)?;
    for [n, k] in list {
        let entry = JsArray::from_iter([n, k].map(|s| JsValue::from(JsString::from(s))), ctx);
        out.push(entry, ctx)?;
    }
    Ok(out.into())
}

/// __wasmCustomSections(id, name) -> [ArrayBuffer, …]
fn n_custom_sections(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = index_arg(a, 0, ctx)?;
    let name = a.get(1).cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
    let st = state(ctx);
    let module = module_of(&st, id)?;
    let datas: Vec<Vec<u8>> = module.custom_sections().filter(|s| s.name() == name).map(|s| s.data().to_vec()).collect();
    let out = JsArray::new(ctx)?;
    for d in datas {
        let buf = JsArrayBuffer::from_byte_block(AlignedVec::from_iter(0, d), ctx)?;
        out.push(buf, ctx)?;
    }
    Ok(out.into())
}

/// The items of a script array.
fn array_items(v: Option<&JsValue>, ctx: &mut Context) -> JsResult<Vec<JsValue>> {
    match v.and_then(|v| v.as_object()) {
        Some(o) => {
            let arr = JsArray::from_object(o.clone())?;
            let n = arr.length(ctx)?;
            (0..n).map(|i| arr.get(i, ctx)).collect()
        }
        None => Ok(Vec::new()),
    }
}

/// __wasmInstantiate(id, [import values in the module's order], [their Memory, Table or
/// Global slots, or -1]) -> [name, kind, value, …] of the exports in the module's order: a
/// function, or the slot of a memory, table or global. Throws LinkError, RuntimeError (from
/// the start function) or whatever an import threw.
fn n_instantiate(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = index_arg(a, 0, ctx)?;
    let st = state(ctx);
    let module = module_of(&st, id)?;
    let given = array_items(a.get(1), ctx)?;
    let slots = array_items(a.get(2), ctx)?;
    // Script gave the values in the module's order; wasmi wants functions, tables, memories,
    // then globals (each in the module's order).
    let order: Vec<usize> = {
        let t = st.tables.borrow();
        let kinds: Vec<u8> = t.modules[id].imports.iter().map(|i| i.2).collect();
        (0..4u8).flat_map(|k| kinds.iter().enumerate().filter(move |(_, kind)| **kind == k).map(|(i, _)| i)).collect()
    };
    let wanted: Vec<(String, String, ExternType)> =
        module.imports().map(|i| (i.module().to_string(), i.name().to_string(), i.ty().clone())).collect();
    let mut externs = Vec::with_capacity(wanted.len());
    for (w, (m, n, ty)) in wanted.into_iter().enumerate() {
        let i = order.get(w).copied().unwrap_or(w);
        let v = given.get(i).cloned().unwrap_or_default();
        let what = format!("import {m:?} {n:?}");
        let slot = slots.get(i).and_then(JsValue::as_number).filter(|n| *n >= 0.0);
        let item = match slot.and_then(|s| objects::extern_of(&st, &ty, s as usize)) {
            Some(item) => item,
            None => resolve_import(ctx, &st, &what, ty, v)?,
        };
        externs.push(item);
    }
    let instance = enter(ctx, &st, |s| wasmi::Instance::new(s, &module, &externs))?;
    let order = st.tables.borrow().modules[id].exports.clone();
    let exports: Vec<(String, u32, Option<Extern>)> =
        with_store(&st, |s| order.into_iter().map(|(name, _, index)| (name.clone(), index, instance.get_export(&*s, &name))).collect());
    let out = JsArray::new(ctx)?;
    for (name, index, ext) in exports {
        let (kind, value) = match ext {
            Some(Extern::Func(f)) => ("function", JsValue::from(function_object(ctx, &st, f, Some(index))?)),
            Some(Extern::Memory(m)) => ("memory", objects::memory_slot(&st, m).into()),
            Some(Extern::Table(t)) => ("table", objects::table_slot(&st, t).into()),
            Some(Extern::Global(g)) => ("global", objects::global_slot(&st, g).into()),
            None => continue,
        };
        out.push(JsValue::from(JsString::from(name)), ctx)?;
        out.push(JsValue::from(JsString::from(kind)), ctx)?;
        out.push(value, ctx)?;
    }
    Ok(out.into())
}

/// The wasm item to give a module for one of its imports.
fn resolve_import(ctx: &mut Context, st: &State, what: &str, ty: ExternType, v: JsValue) -> JsResult<Extern> {
    match ty {
        ExternType::Func(fty) => {
            let Some(obj) = v.as_object().filter(|o| o.is_callable()) else {
                return Err(throw(ctx, st, Kind::Link, &format!("{what}: function import requires a callable")));
            };
            // A wasm function passed back in is used as itself (its type must match).
            let known = st.tables.borrow().of_object.get(&obj).map(|&i| st.tables.borrow().funcs[i as usize]);
            if let Some(f) = known {
                let same = with_store(st, |s| f.ty(&*s) == fty);
                if !same {
                    return Err(throw(ctx, st, Kind::Link, &format!("{what}: imported function does not match the expected type")));
                }
                return Ok(Extern::Func(f));
            }
            Ok(Extern::Func(host_func(st, fty, obj)))
        }
        ExternType::Global(gty) => {
            if gty.mutability().is_mut() {
                return Err(throw(ctx, st, Kind::Link, &format!("{what}: a mutable global import needs a WebAssembly.Global")));
            }
            let content = gty.content();
            let bad_type = match content {
                wasmi::ValType::I64 => !v.is_bigint(),
                wasmi::ValType::I32 | wasmi::ValType::F32 | wasmi::ValType::F64 => !v.is_number(),
                wasmi::ValType::V128 => true,
                _ => false,
            };
            if bad_type {
                return Err(throw(ctx, st, Kind::Link, &format!("{what}: global import must be a Number or BigInt of the right type")));
            }
            let val = values::to_wasm(ctx, st, &v, content)?;
            Ok(Extern::Global(with_store(st, |s| wasmi::Global::new(s, val, wasmi::Mutability::Const))))
        }
        ExternType::Memory(_) => Err(throw(ctx, st, Kind::Link, &format!("{what}: memory import must be a WebAssembly.Memory"))),
        ExternType::Table(_) => Err(throw(ctx, st, Kind::Link, &format!("{what}: table import must be a WebAssembly.Table"))),
    }
}

/// A host function that calls the JavaScript function `callee`.
fn host_func(st: &State, ty: FuncType, callee: JsObject) -> Func {
    let original = callee.clone();
    let func = with_store(st, |s| {
        let index = s.data().js_funcs.len();
        s.data_mut().js_funcs.push(callee);
        let results: Vec<wasmi::ValType> = ty.results().to_vec();
        Func::new(&mut *s, ty, move |mut caller: Caller<'_, Host>, params: &[Val], out: &mut [Val]| {
            let Some(ctx) = CONTEXTS.with(|c| c.borrow().last().copied()) else {
                return Err(wasmi::Error::new("WebAssembly import called with no script running"));
            };
            // SAFETY: the script engine that called into wasm is further up this thread's
            // stack and doesn't touch itself until the call returns (see `enter`).
            let ctx = unsafe { &mut *ctx };
            let callee = caller.data().js_funcs[index].clone();
            // SAFETY: only the lifetime is erased; the pointer is popped before `caller` goes
            // out of scope at the end of this closure.
            let p = &mut caller as *mut Caller<'_, Host> as *mut Caller<'static, Host>;
            CALLERS.with(|c| c.borrow_mut().push(p));
            let r = call_js(ctx, &callee, params, &results, out);
            CALLERS.with(|c| c.borrow_mut().pop());
            r.map_err(|e| {
                THROWN.with(|t| t.borrow_mut().push(e));
                wasmi::Error::host(Thrown)
            })
        })
    });
    // Exported again, it is the original JavaScript function.
    st.tables.borrow_mut().objects.insert(func_key(&func), original);
    func
}

/// Run a JavaScript import: wasm arguments to script values, and the return value to wasm.
fn call_js(ctx: &mut Context, callee: &JsObject, params: &[Val], results: &[wasmi::ValType], out: &mut [Val]) -> JsResult<()> {
    let st = state(ctx);
    // Script is about to run: memories wasm grew get new buffers.
    objects::refresh_buffers(&st);
    let args = params.iter().map(|v| values::to_js(ctx, &st, v)).collect::<JsResult<Vec<_>>>()?;
    let ret = callee.call(&JsValue::undefined(), &args, ctx)?;
    match results.len() {
        0 => {}
        1 => out[0] = values::to_wasm(ctx, &st, &ret, results[0])?,
        n => {
            // Several results: the function returns something iterable with exactly n items.
            let items = values::iterate(ctx, &ret)?;
            if items.len() != n {
                return Err(JsNativeError::typ().with_message(format!("WebAssembly import returned {} values, expected {n}", items.len())).into());
            }
            for (i, v) in items.iter().enumerate() {
                out[i] = values::to_wasm(ctx, &st, v, results[i])?;
            }
        }
    }
    Ok(())
}

/// The JavaScript function for a wasm function: the same object every time, and the original
/// JavaScript function if it is an import passed through.
pub(crate) fn function_object(ctx: &mut Context, st: &State, f: Func, index: Option<u32>) -> JsResult<JsObject> {
    let key = func_key(&f);
    if let Some(o) = st.tables.borrow().objects.get(&key) {
        return Ok(o.clone());
    }
    let params = with_store(st, |s| f.ty(&*s).params().len());
    let slot = {
        let mut t = st.tables.borrow_mut();
        t.funcs.push(f);
        (t.funcs.len() - 1) as u32
    };
    let native = NativeFunction::from_copy_closure_with_captures(
        |_, args, slot: &u32, ctx| call_export(*slot, args, ctx),
        slot,
    );
    let name = JsString::from(index.map_or_else(|| slot.to_string(), |i| i.to_string()));
    let obj: JsObject = FunctionObjectBuilder::new(ctx.realm(), native).name(name).length(params).constructor(false).build().into();
    let mut t = st.tables.borrow_mut();
    t.objects.insert(key, obj.clone());
    t.of_object.insert(obj.clone(), slot);
    Ok(obj)
}

/// Calling an exported function from script.
fn call_export(slot: u32, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let st = state(ctx);
    let Some(f) = st.tables.borrow().funcs.get(slot as usize).copied() else {
        return Err(JsNativeError::typ().with_message("WebAssembly: stale function").into());
    };
    let ty = with_store(&st, |s| f.ty(&*s));
    let mut params = Vec::with_capacity(ty.params().len());
    for (i, t) in ty.params().iter().enumerate() {
        let v = args.get(i).cloned().unwrap_or_default();
        params.push(values::to_wasm(ctx, &st, &v, *t)?);
    }
    let mut results: Vec<Val> = ty.results().iter().map(|t| Val::default_for_ty(*t)).collect();
    enter(ctx, &st, |s| f.call(s, &params, &mut results))?;
    match results.len() {
        0 => Ok(JsValue::undefined()),
        1 => values::to_js(ctx, &st, &results[0]),
        _ => {
            let vals = results.iter().map(|v| values::to_js(ctx, &st, v)).collect::<JsResult<Vec<_>>>()?;
            Ok(JsArray::from_iter(vals, ctx).into())
        }
    }
}
