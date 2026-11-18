//! Functions compiled code calls for the operations it doesn't inline. Each one does exactly
//! what the interpreter's fast path for that opcode does (they call the same `*_fast`
//! functions), on registers passed as raw NaN-boxed words.
//!
//! They never allocate and never touch the VM, so calling one can't run the GC or move the
//! register file. Anything but numbers (and for some, booleans, `null` and `undefined`) makes
//! them return [`NONE`], and compiled code bails out to the interpreter at that instruction.

use crate::{JsValue, builtins::Number};
use std::mem::ManuallyDrop;

/// "No result": the operation needs the interpreter. Never a real value (NaN-boxed values
/// keep NaN canonical, so this negative NaN pattern can't occur).
pub(crate) const NONE: u64 = u64::MAX;

/// Tag bits that all pointer kinds (object, string, symbol, bigint) have.
pub(crate) const POINTER_MASK: u64 = 0x7FFC_0000_0000_0000;
/// The upper 32 bits of an int32 value.
pub(crate) const INT32_TAG: u64 = 0x7FF9_0000;

#[inline(always)]
pub(crate) const fn is_pointer(bits: u64) -> bool {
    bits & POINTER_MASK == POINTER_MASK
}

/// Borrows a register word as a value without taking ownership; `None` for pointer kinds.
#[inline(always)]
fn view(bits: u64) -> Option<ManuallyDrop<JsValue>> {
    if is_pointer(bits) {
        return None;
    }
    // SAFETY: not a pointer kind, so it owns nothing.
    Some(ManuallyDrop::new(unsafe { JsValue::jit_from_bits(bits) }))
}

/// A result's word. Number results never hold pointers.
#[inline(always)]
fn word(value: JsValue) -> u64 {
    let bits = value.jit_bits();
    if is_pointer(bits) {
        // Not reachable for the operations here; give the value back to Rust to drop.
        drop(value);
        return NONE;
    }
    std::mem::forget(value);
    bits
}

#[inline(always)]
fn binary(a: u64, b: u64, f: impl FnOnce(&JsValue, &JsValue) -> Option<JsValue>) -> u64 {
    match (view(a), view(b)) {
        (Some(x), Some(y)) => f(&x, &y).map_or(NONE, word),
        _ => NONE,
    }
}

/// 0 = false, 1 = true, 2 = needs the interpreter.
#[inline(always)]
fn compare(a: u64, b: u64, f: impl FnOnce(&JsValue, &JsValue) -> Option<bool>) -> u64 {
    match (view(a), view(b)) {
        (Some(x), Some(y)) => f(&x, &y).map_or(2, u64::from),
        _ => 2,
    }
}

macro_rules! binary_helpers {
    ($($name:ident => $fast:ident),* $(,)?) => {
        $(
            pub(crate) extern "C" fn $name(a: u64, b: u64) -> u64 {
                binary(a, b, JsValue::$fast)
            }
        )*
    };
}

binary_helpers! {
    add => add_fast,
    sub => sub_fast,
    mul => mul_fast,
    div => div_fast,
    rem => rem_fast,
    bitand => bitand_fast,
    bitor => bitor_fast,
    bitxor => bitxor_fast,
    shl => shl_fast,
    shr => shr_fast,
    ushr => ushr_fast,
    equals => equals_fast,
    not_equals => not_equals_fast,
}

macro_rules! compare_helpers {
    ($($name:ident => $fast:ident),* $(,)?) => {
        $(
            pub(crate) extern "C" fn $name(a: u64, b: u64) -> u64 {
                compare(a, b, JsValue::$fast)
            }
        )*
    };
}

compare_helpers! {
    lt => lt_fast,
    le => le_fast,
    gt => gt_fast,
    ge => ge_fast,
}

/// `StrictEq` (0/1/2). Strings need their contents compared, so pointers go to the interpreter.
pub(crate) extern "C" fn strict_equals(a: u64, b: u64) -> u64 {
    compare(a, b, |x, y| Some(x.strict_equals(y)))
}

/// `JumpIfNotEqual`'s `lhs != rhs` (0/1/2).
pub(crate) extern "C" fn not_same(a: u64, b: u64) -> u64 {
    compare(a, b, |x, y| Some(x != y))
}

/// `to_boolean` (0/1/2): strings, objects and bigints go to the interpreter.
pub(crate) extern "C" fn truthy(a: u64) -> u64 {
    view(a).map_or(2, |v| u64::from(v.to_boolean()))
}

/// The number in a register, or `None` for anything else (the interpreter converts those).
#[inline(always)]
fn number(a: u64) -> Option<f64> {
    view(a).and_then(|v| v.as_number())
}

/// `Pos`: `+x` on a number is the number (as the interpreter stores it, a double).
pub(crate) extern "C" fn pos(a: u64) -> u64 {
    number(a).map_or(NONE, |n| word(n.into()))
}

/// `Neg` on a number.
pub(crate) extern "C" fn neg(a: u64) -> u64 {
    number(a).map_or(NONE, |n| word((-n).into()))
}

/// `BitNot` on a number.
pub(crate) extern "C" fn bitnot(a: u64) -> u64 {
    number(a).map_or(NONE, |n| word(Number::not(n).into()))
}

/// `Inc`/`Dec` on a number: writes the new `src` (the old value as a number) and `dst`.
/// Returns 0 on success, 2 when the interpreter has to do it.
pub(crate) extern "C" fn inc(a: u64, out: *mut [u64; 2]) -> u64 {
    step(a, out, 1)
}

pub(crate) extern "C" fn dec(a: u64, out: *mut [u64; 2]) -> u64 {
    step(a, out, -1)
}

fn step(a: u64, out: *mut [u64; 2], by: i32) -> u64 {
    let Some(v) = view(a) else { return 2 };
    // The int32 variant only, as the interpreter's `Inc`/`Dec` (not integral doubles).
    let int32 = match v.variant() {
        crate::value::JsVariant::Integer32(n) => Some(n),
        _ => None,
    };
    let (numeric, value) = match int32 {
        Some(n) if (by > 0 && n < i32::MAX) || (by < 0 && n > i32::MIN) => (JsValue::from(n), JsValue::from(n + by)),
        _ => match v.as_number() {
            Some(n) => (JsValue::from(n), JsValue::from(n + f64::from(by))),
            None => return 2,
        },
    };
    let (numeric, value) = (word(numeric), word(value));
    if numeric == NONE || value == NONE {
        return 2;
    }
    // SAFETY: compiled code passes a pointer to two words in its stack slot.
    unsafe { *out = [numeric, value] };
    0
}
