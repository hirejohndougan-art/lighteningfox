//! Values crossing between script and wasm (the JS API's ToJSValue and ToWebAssemblyValue),
//! and the little reading of module bytes the API needs.

use boa_engine::{js_string, Context, JsBigInt, JsNativeError, JsResult, JsValue};
use wasmi::{AsContext, ExternRef, Nullable, Val, ValType};

use super::{function_object, with_store, State};

/// ToJSValue: a wasm value as a script value.
pub(super) fn to_js(ctx: &mut Context, st: &State, v: &Val) -> JsResult<JsValue> {
    Ok(match v {
        Val::I32(x) => JsValue::from(*x),
        Val::I64(x) => JsValue::from(JsBigInt::from(*x)),
        Val::F32(x) => JsValue::from(f64::from(f32::from(*x))),
        Val::F64(x) => JsValue::from(f64::from(*x)),
        Val::V128(_) => return Err(JsNativeError::typ().with_message("WebAssembly: v128 values can't be passed to JavaScript").into()),
        Val::FuncRef(Nullable::Null) | Val::ExternRef(Nullable::Null) => JsValue::null(),
        Val::FuncRef(Nullable::Val(f)) => function_object(ctx, st, *f, None)?.into(),
        Val::ExternRef(Nullable::Val(e)) => with_store(st, |s| {
            let i = e.data(s.as_context()).downcast_ref::<u32>().copied();
            i.and_then(|i| s.data().externs.get(i as usize).cloned()).unwrap_or_default()
        }),
    })
}

/// ToWebAssemblyValue: a script value as a wasm value of type `ty` (may run script, e.g.
/// `valueOf`, and throw).
pub(super) fn to_wasm(ctx: &mut Context, st: &State, v: &JsValue, ty: ValType) -> JsResult<Val> {
    Ok(match ty {
        ValType::I32 => Val::I32(v.to_i32(ctx)?),
        ValType::I64 => Val::I64(v.to_big_int64(ctx)?),
        ValType::F32 => Val::F32((v.to_number(ctx)? as f32).into()),
        ValType::F64 => Val::F64(v.to_number(ctx)?.into()),
        ValType::V128 => return Err(JsNativeError::typ().with_message("WebAssembly: v128 values can't be passed from JavaScript").into()),
        ValType::FuncRef => {
            if v.is_null() {
                return Ok(Val::FuncRef(Nullable::Null));
            }
            let f = v.as_object().and_then(|o| {
                let t = st.tables.borrow();
                t.of_object.get(&o).and_then(|&i| t.funcs.get(i as usize).copied())
            });
            match f {
                Some(f) => Val::FuncRef(Nullable::Val(f)),
                None => return Err(JsNativeError::typ().with_message("WebAssembly: funcref must be null or an exported wasm function").into()),
            }
        }
        ValType::ExternRef => {
            if v.is_null() {
                return Ok(Val::ExternRef(Nullable::Null));
            }
            with_store(st, |s| {
                let i = s.data().externs.len() as u32;
                s.data_mut().externs.push(v.clone());
                Val::ExternRef(Nullable::Val(ExternRef::new(&mut *s, i)))
            })
        }
    })
}

/// The items of an iterable (a JavaScript import returning several results).
pub(super) fn iterate(ctx: &mut Context, v: &JsValue) -> JsResult<Vec<JsValue>> {
    let iterable = v.as_object().is_some_and(|o| {
        let key = boa_engine::JsSymbol::iterator();
        o.has_property(key, ctx).unwrap_or(false)
    });
    if !iterable {
        return Err(JsNativeError::typ().with_message("WebAssembly: a function with several results must return an iterable").into());
    }
    let array = ctx.global_object().get(js_string!("Array"), ctx)?;
    let from = array.as_object().map(|a| a.get(js_string!("from"), ctx)).transpose()?.unwrap_or_default();
    let Some(from) = from.as_callable() else {
        return Err(JsNativeError::typ().with_message("Array.from is missing").into());
    };
    let arr = from.call(&array, std::slice::from_ref(v), ctx)?;
    let arr = boa_engine::object::builtins::JsArray::from_object(arr.as_object().expect("Array.from returns an array"))?;
    let n = arr.length(ctx)?;
    (0..n).map(|i| arr.get(i, ctx)).collect()
}

fn leb(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    let mut out = 0u32;
    let mut shift = 0;
    loop {
        let b = *bytes.get(*pos)?;
        *pos += 1;
        out |= u32::from(b & 0x7f).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some(out);
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
}

/// A module's exports in the module's order (wasmi lists them in its own): name, kind (0
/// function, 1 table, 2 memory, 3 global) and index (an exported function's `name` is its
/// index). Only called on bytes that compiled, so a short read just ends the list.
pub(super) fn exports_in_order(bytes: &[u8]) -> Vec<(String, u8, u32)> {
    let mut out = Vec::new();
    let mut pos = 8;
    while pos < bytes.len() {
        let id = bytes[pos];
        pos += 1;
        let Some(size) = leb(bytes, &mut pos) else { break };
        let end = pos.saturating_add(size as usize).min(bytes.len());
        if id == 7 {
            let mut p = pos;
            let count = leb(bytes, &mut p).unwrap_or(0);
            for _ in 0..count {
                let Some(len) = leb(bytes, &mut p) else { break };
                let Some(name) = bytes.get(p..p + len as usize) else { break };
                p += len as usize;
                let Some(&kind) = bytes.get(p) else { break };
                p += 1;
                let Some(index) = leb(bytes, &mut p) else { break };
                out.push((String::from_utf8_lossy(name).into_owned(), kind, index));
            }
            break;
        }
        pos = end;
    }
    out
}
