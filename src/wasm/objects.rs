//! `WebAssembly.Memory`, `Table` and `Global` (task P1b): natives on slot numbers; the
//! classes are in `wasm.js`. A wasm memory, table or global made here or exported by an
//! instance gets one slot, so it is always the same script object.
//!
//! `memory.buffer` is an `ArrayBuffer` over the module's memory itself (Boa's external
//! buffers, see `vendor/boa/IRONSWIFT.md`). Growing the memory can move it, so whenever
//! control comes back to script from wasm (`refresh_buffers`), a buffer whose memory changed
//! size is detached, and the next `buffer` read makes a new one, as the spec asks.

use boa_engine::object::builtins::JsArrayBuffer;
use boa_engine::{js_string, Context, JsNativeError, JsResult, JsString, JsValue};
use wasmi::{Global, Memory, MemoryType, Mutability, Nullable, Ref, RefType, Table, TableType, Val, ValType};

use super::{state, values, with_store, State};

/// A memory and the `ArrayBuffer` handed out for it (with the byte length it was made at).
pub(super) struct MemSlot {
    pub(super) mem: Memory,
    buffer: Option<(JsArrayBuffer, usize, usize)>,
}

/// Slots of memories, tables and globals.
#[derive(Default)]
pub(super) struct Slots {
    pub(super) memories: Vec<MemSlot>,
    pub(super) tables: Vec<Table>,
    pub(super) globals: Vec<Global>,
    /// Item (by its debug form) to slot, per kind.
    known: std::collections::HashMap<String, u32>,
}

/// The JS API's limits (https://webassembly.github.io/spec/js-api/#limits).
const MAX_PAGES: u64 = 65536;
const MAX_TABLE: u64 = 10_000_000;

fn detach_key() -> JsValue {
    js_string!("WebAssembly.Memory").into()
}

/// The slot for a memory (one per memory).
pub(super) fn memory_slot(st: &State, mem: Memory) -> u32 {
    let key = format!("m{mem:?}");
    let mut t = st.tables.borrow_mut();
    if let Some(&s) = t.slots.known.get(&key) {
        return s;
    }
    t.slots.memories.push(MemSlot { mem, buffer: None });
    let s = (t.slots.memories.len() - 1) as u32;
    t.slots.known.insert(key, s);
    s
}

pub(super) fn table_slot(st: &State, table: Table) -> u32 {
    let key = format!("t{table:?}");
    let mut t = st.tables.borrow_mut();
    if let Some(&s) = t.slots.known.get(&key) {
        return s;
    }
    t.slots.tables.push(table);
    let s = (t.slots.tables.len() - 1) as u32;
    t.slots.known.insert(key, s);
    s
}

pub(super) fn global_slot(st: &State, global: Global) -> u32 {
    let key = format!("g{global:?}");
    let mut t = st.tables.borrow_mut();
    if let Some(&s) = t.slots.known.get(&key) {
        return s;
    }
    t.slots.globals.push(global);
    let s = (t.slots.globals.len() - 1) as u32;
    t.slots.known.insert(key, s);
    s
}

/// Detach the buffers of memories that grew (wasm ran and may have grown them).
pub(super) fn refresh_buffers(st: &State) {
    let stale: Vec<(usize, JsArrayBuffer)> = {
        let t = st.tables.borrow();
        if t.slots.memories.iter().all(|m| m.buffer.is_none()) {
            return;
        }
        with_store(st, |s| {
            t.slots
                .memories
                .iter()
                .enumerate()
                .filter_map(|(i, m)| {
                    let (buf, len, ptr) = m.buffer.as_ref()?;
                    let moved = m.mem.data_size(&*s) != *len || m.mem.data_ptr(&*s) as usize != *ptr;
                    moved.then(|| (i, buf.clone()))
                })
                .collect()
        })
    };
    for (i, buf) in stale {
        let _ = buf.detach(&detach_key());
        st.tables.borrow_mut().slots.memories[i].buffer = None;
    }
}

fn slot_arg(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<usize> {
    Ok(a.get(i).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(-1.0) as usize)
}

fn u64_arg(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<Option<u64>> {
    let n = a.get(i).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(-1.0);
    Ok((n >= 0.0).then_some(n as u64))
}

fn range(msg: &str) -> boa_engine::JsError {
    JsNativeError::range().with_message(msg.to_string()).into()
}

fn bad_slot() -> boa_engine::JsError {
    JsNativeError::typ().with_message("WebAssembly: not a live Memory, Table or Global").into()
}

fn memory(st: &State, slot: usize) -> JsResult<Memory> {
    st.tables.borrow().slots.memories.get(slot).map(|m| m.mem).ok_or_else(bad_slot)
}

fn table(st: &State, slot: usize) -> JsResult<Table> {
    st.tables.borrow().slots.tables.get(slot).copied().ok_or_else(bad_slot)
}

fn global(st: &State, slot: usize) -> JsResult<Global> {
    st.tables.borrow().slots.globals.get(slot).copied().ok_or_else(bad_slot)
}

pub(super) const NATIVES: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
    ("__wasmMemoryNew", 2, n_memory_new),
    ("__wasmMemoryBuffer", 1, n_memory_buffer),
    ("__wasmMemoryGrow", 2, n_memory_grow),
    ("__wasmTableNew", 4, n_table_new),
    ("__wasmTableLength", 1, n_table_length),
    ("__wasmTableGet", 2, n_table_get),
    ("__wasmTableSet", 3, n_table_set),
    ("__wasmTableGrow", 3, n_table_grow),
    ("__wasmTableElement", 1, n_table_element),
    ("__wasmGlobalNew", 3, n_global_new),
    ("__wasmGlobalGet", 1, n_global_get),
    ("__wasmGlobalSet", 2, n_global_set),
    ("__wasmGlobalType", 1, n_global_type),
];

/// __wasmMemoryNew(initial, maximum or -1) -> slot
fn n_memory_new(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let initial = u64_arg(a, 0, ctx)?.unwrap_or(0);
    let maximum = u64_arg(a, 1, ctx)?;
    if initial > MAX_PAGES || maximum.is_some_and(|m| m > MAX_PAGES) {
        return Err(range("WebAssembly.Memory: size can't be more than 65536 pages (4 GiB)"));
    }
    if maximum.is_some_and(|m| m < initial) {
        return Err(range("WebAssembly.Memory: maximum is smaller than initial"));
    }
    let st = state(ctx);
    let ty = MemoryType::new(initial as u32, maximum.map(|m| m as u32));
    let mem = with_store(&st, |s| Memory::new(&mut *s, ty)).map_err(|e| range(&format!("WebAssembly.Memory: {e}")))?;
    Ok(memory_slot(&st, mem).into())
}

/// __wasmMemoryBuffer(slot) -> ArrayBuffer over the memory (the same one until it grows).
fn n_memory_buffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let st = state(ctx);
    let mem = memory(&st, slot)?;
    refresh_buffers(&st);
    if let Some((buf, _, _)) = st.tables.borrow().slots.memories[slot].buffer.as_ref() {
        return Ok(buf.clone().into());
    }
    let (ptr, len) = with_store(&st, |s| (mem.data_ptr(&*s), mem.data_size(&*s)));
    // SAFETY: the bytes belong to the page's wasm store, which lives as long as the page's
    // script engine; they only move or change length when the memory grows, and every path
    // that can grow it detaches this buffer before script runs again (`refresh_buffers`,
    // `n_memory_grow`).
    let buf = unsafe { JsArrayBuffer::from_external(ptr, len, detach_key(), ctx)? };
    st.tables.borrow_mut().slots.memories[slot].buffer = Some((buf.clone(), len, ptr as usize));
    Ok(buf.into())
}

/// __wasmMemoryGrow(slot, delta) -> the old size in pages.
fn n_memory_grow(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let delta = u64_arg(a, 1, ctx)?.unwrap_or(0);
    let st = state(ctx);
    let mem = memory(&st, slot)?;
    let r = with_store(&st, |s| {
        let cap = mem.ty(&*s).maximum().unwrap_or(MAX_PAGES).min(MAX_PAGES);
        if mem.size(&*s).saturating_add(delta) > cap {
            return None;
        }
        mem.grow(&mut *s, delta).ok()
    });
    let Some(old) = r else {
        return Err(range("WebAssembly.Memory.grow(): Maximum memory size exceeded"));
    };
    // The old buffer is detached even if nothing moved (the spec always refreshes it).
    let stale = st.tables.borrow_mut().slots.memories[slot].buffer.take();
    if let Some((buf, _, _)) = stale {
        let _ = buf.detach(&detach_key());
    }
    Ok(JsValue::from(old as f64))
}

fn ref_type(name: &str) -> Option<RefType> {
    match name {
        "anyfunc" | "funcref" => Some(RefType::Func),
        "externref" => Some(RefType::Extern),
        _ => None,
    }
}

fn to_ref(v: Val) -> Ref {
    match v {
        Val::FuncRef(f) => Ref::Func(f),
        Val::ExternRef(e) => Ref::Extern(e),
        _ => Ref::Func(Nullable::Null),
    }
}

fn from_ref(r: Ref) -> Val {
    match r {
        Ref::Func(f) => Val::FuncRef(f),
        Ref::Extern(e) => Val::ExternRef(e),
    }
}

fn elem_val_type(e: RefType) -> ValType {
    match e {
        RefType::Func => ValType::FuncRef,
        RefType::Extern => ValType::ExternRef,
    }
}

/// The element for a table slot from script: `undefined` (a missing argument) is the default
/// (null for funcref, `undefined` itself for externref).
fn element(ctx: &mut Context, st: &State, e: RefType, v: Option<&JsValue>) -> JsResult<Ref> {
    match (e, v) {
        (RefType::Func, None) => Ok(Ref::Func(Nullable::Null)),
        (_, v) => Ok(to_ref(values::to_wasm(ctx, st, &v.cloned().unwrap_or_default(), elem_val_type(e))?)),
    }
}

/// __wasmTableNew(element, initial, maximum or -1, value?) -> slot
fn n_table_new(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = a.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
    let Some(e) = ref_type(&name) else {
        return Err(JsNativeError::typ().with_message("WebAssembly.Table: element must be \"anyfunc\" or \"externref\"").into());
    };
    let initial = u64_arg(a, 1, ctx)?.unwrap_or(0);
    let maximum = u64_arg(a, 2, ctx)?;
    if initial > MAX_TABLE || maximum.is_some_and(|m| m > u64::from(u32::MAX)) {
        return Err(range("WebAssembly.Table: initial size is too large"));
    }
    if maximum.is_some_and(|m| m < initial) {
        return Err(range("WebAssembly.Table: maximum is smaller than initial"));
    }
    let st = state(ctx);
    let init = element(ctx, &st, e, a.get(3).filter(|_| a.len() > 3))?;
    let ty = TableType::new(e, initial as u32, maximum.map(|m| m as u32));
    let table = with_store(&st, |s| Table::new(&mut *s, ty, init)).map_err(|e| range(&format!("WebAssembly.Table: {e}")))?;
    Ok(table_slot(&st, table).into())
}

fn n_table_length(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let st = state(ctx);
    let t = table(&st, slot)?;
    Ok(JsValue::from(with_store(&st, |s| t.size(&*s)) as f64))
}

/// __wasmTableElement(slot) -> "anyfunc" or "externref"
fn n_table_element(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let st = state(ctx);
    let t = table(&st, slot)?;
    let e = with_store(&st, |s| t.ty(&*s).element());
    Ok(js_string!(if matches!(e, RefType::Func) { "anyfunc" } else { "externref" }).into())
}

fn n_table_get(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let index = u64_arg(a, 1, ctx)?.unwrap_or(u64::MAX);
    let st = state(ctx);
    let t = table(&st, slot)?;
    let Some(r) = with_store(&st, |s| t.get(&*s, index)) else {
        return Err(range("WebAssembly.Table.get(): index out of bounds"));
    };
    values::to_js(ctx, &st, &from_ref(r))
}

/// __wasmTableSet(slot, index, value?)
fn n_table_set(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let index = u64_arg(a, 1, ctx)?.unwrap_or(u64::MAX);
    let st = state(ctx);
    let t = table(&st, slot)?;
    let (e, size) = with_store(&st, |s| (t.ty(&*s).element(), t.size(&*s)));
    if index >= size {
        return Err(range("WebAssembly.Table.set(): index out of bounds"));
    }
    let r = element(ctx, &st, e, a.get(2).filter(|_| a.len() > 2))?;
    with_store(&st, |s| t.set(&mut *s, index, r)).map_err(|e| range(&format!("WebAssembly.Table.set(): {e}")))?;
    Ok(JsValue::undefined())
}

/// __wasmTableGrow(slot, delta, value?) -> the old length.
fn n_table_grow(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let delta = u64_arg(a, 1, ctx)?.unwrap_or(0);
    let st = state(ctx);
    let t = table(&st, slot)?;
    let e = with_store(&st, |s| t.ty(&*s).element());
    let r = element(ctx, &st, e, a.get(2).filter(|_| a.len() > 2))?;
    let grown = with_store(&st, |s| {
        if t.size(&*s).saturating_add(delta) > MAX_TABLE {
            return None;
        }
        t.grow(&mut *s, delta, r).ok()
    });
    grown.map(|old| JsValue::from(old as f64)).ok_or_else(|| range("WebAssembly.Table.grow(): failed to grow table"))
}

pub(super) fn val_type(name: &str) -> Option<ValType> {
    Some(match name {
        "i32" => ValType::I32,
        "i64" => ValType::I64,
        "f32" => ValType::F32,
        "f64" => ValType::F64,
        "v128" => ValType::V128,
        "externref" => ValType::ExternRef,
        "anyfunc" | "funcref" => ValType::FuncRef,
        _ => return None,
    })
}

fn val_type_name(t: ValType) -> &'static str {
    match t {
        ValType::I32 => "i32",
        ValType::I64 => "i64",
        ValType::F32 => "f32",
        ValType::F64 => "f64",
        ValType::V128 => "v128",
        ValType::ExternRef => "externref",
        ValType::FuncRef => "anyfunc",
    }
}

/// __wasmGlobalNew(type, mutable, value) -> slot. `wasm.js` passes the spec's default value
/// when none was given.
fn n_global_new(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = a.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
    let Some(ty) = val_type(&name) else {
        return Err(JsNativeError::typ().with_message("WebAssembly.Global: unknown value type").into());
    };
    if ty == ValType::V128 {
        return Err(JsNativeError::typ().with_message("WebAssembly.Global: v128 globals can't be made from JavaScript").into());
    }
    let mutable = a.get(1).is_some_and(JsValue::to_boolean);
    let st = state(ctx);
    let v = values::to_wasm(ctx, &st, &a.get(2).cloned().unwrap_or_default(), ty)?;
    let m = if mutable { Mutability::Var } else { Mutability::Const };
    let g = with_store(&st, |s| Global::new(&mut *s, v, m));
    Ok(global_slot(&st, g).into())
}

fn n_global_get(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let st = state(ctx);
    let g = global(&st, slot)?;
    let v = with_store(&st, |s| g.get(&*s));
    values::to_js(ctx, &st, &v)
}

fn n_global_set(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let st = state(ctx);
    let g = global(&st, slot)?;
    let ty = with_store(&st, |s| g.ty(&*s));
    if !ty.mutability().is_mut() {
        return Err(JsNativeError::typ().with_message("WebAssembly.Global: can't set the value of an immutable global").into());
    }
    let v = values::to_wasm(ctx, &st, &a.get(1).cloned().unwrap_or_default(), ty.content())?;
    with_store(&st, |s| g.set(&mut *s, v)).map_err(|e| JsNativeError::typ().with_message(format!("WebAssembly.Global: {e}")))?;
    Ok(JsValue::undefined())
}

/// __wasmGlobalType(slot) -> [value type, mutable]
fn n_global_type(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = slot_arg(a, 0, ctx)?;
    let st = state(ctx);
    let g = global(&st, slot)?;
    let ty = with_store(&st, |s| g.ty(&*s));
    let arr = boa_engine::object::builtins::JsArray::from_iter(
        [JsValue::from(JsString::from(val_type_name(ty.content()))), JsValue::from(ty.mutability().is_mut())],
        ctx,
    );
    Ok(arr.into())
}

/// For `wasm.js` and the import code: the slot's item as an `Extern`.
pub(super) fn extern_of(st: &State, kind: &wasmi::ExternType, slot: usize) -> Option<wasmi::Extern> {
    let t = st.tables.borrow();
    Some(match kind {
        wasmi::ExternType::Memory(_) => wasmi::Extern::Memory(t.slots.memories.get(slot)?.mem),
        wasmi::ExternType::Table(_) => wasmi::Extern::Table(*t.slots.tables.get(slot)?),
        wasmi::ExternType::Global(_) => wasmi::Extern::Global(*t.slots.globals.get(slot)?),
        wasmi::ExternType::Func(_) => return None,
    })
}

