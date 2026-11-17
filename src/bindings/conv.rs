//! The WebIDL rules the generated bindings (`generated.rs`) use: argument-count checks, turning
//! JavaScript values into Rust ones as the WebIDL spec says (strings, the integer types with
//! their wrap-around, [EnforceRange] and [Clamp], restricted doubles, enums, nodes), and turning
//! the hand-written implementations' results back into JavaScript values.
//! Hand-written; `tools/webidl/gen.py` decides which of these each member calls (some are
//! for argument types no bound member takes yet).
#![allow(dead_code)]

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue};

use crate::dom::NodeId;
use crate::dom_classes::{node_of, wrap, wrap_all};

fn type_error(msg: String) -> boa_engine::JsError {
    JsNativeError::typ().with_message(msg).into()
}

/// The argument at `i` (undefined when missing).
pub fn arg(a: &[JsValue], i: usize) -> JsValue {
    a.get(i).cloned().unwrap_or_default()
}

/// Whether an optional argument was left out (missing or undefined).
pub fn absent(a: &[JsValue], i: usize) -> bool {
    a.get(i).is_none_or(JsValue::is_undefined)
}

/// Throws the TypeError other browsers do when an operation gets too few arguments.
pub fn require(a: &[JsValue], n: usize, op: &str, iface: &str) -> JsResult<()> {
    if a.len() < n {
        let s = if n == 1 { "" } else { "s" };
        return Err(type_error(format!(
            "Failed to execute '{op}' on '{iface}': {n} argument{s} required, but only {} present.",
            a.len()
        )));
    }
    Ok(())
}

/// `this` for a member of a node interface: its node (None for one past the end of the tree,
/// which answers like an empty node), or a TypeError for anything that isn't a node.
pub fn this_node(this: &JsValue, ctx: &mut Context) -> JsResult<Option<NodeId>> {
    match node_of(this, ctx)? {
        Some(id) => Ok(id),
        None => Err(type_error("Illegal invocation: not a DOM node".into())),
    }
}

// ------------------------------------------------------------ JavaScript -> Rust

/// DOMString / USVString / ByteString.
pub fn string(v: &JsValue, ctx: &mut Context) -> JsResult<String> {
    Ok(v.to_string(ctx)?.to_std_string_escaped())
}

/// A nullable string: null and undefined come back as None.
pub fn string_or_null(v: &JsValue, ctx: &mut Context) -> JsResult<Option<String>> {
    if v.is_null_or_undefined() {
        return Ok(None);
    }
    string(v, ctx).map(Some)
}

/// [LegacyNullToEmptyString] DOMString: null becomes "".
pub fn string_null_empty(v: &JsValue, ctx: &mut Context) -> JsResult<String> {
    if v.is_null() {
        return Ok(String::new());
    }
    string(v, ctx)
}

pub fn boolean(v: &JsValue, _: &mut Context) -> JsResult<bool> {
    Ok(v.to_boolean())
}

/// An enum value: one of `allowed`, else a TypeError.
pub fn enumeration(v: &JsValue, allowed: &[&str], name: &str, ctx: &mut Context) -> JsResult<String> {
    let s = string(v, ctx)?;
    if allowed.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(type_error(format!("The provided value '{s}' is not a valid enum value of type {name}.")))
    }
}

/// How an integer argument is converted: [EnforceRange] throws outside the range, [Clamp]
/// clamps, and the default wraps around (ConvertToInt in the WebIDL spec).
#[derive(Clone, Copy, PartialEq)]
pub enum IntMode {
    Wrap,
    Enforce,
    Clamp,
}

/// ConvertToInt for a `bits`-wide integer type, as an f64 holding the result.
pub fn integer(v: &JsValue, bits: u32, signed: bool, mode: IntMode, ctx: &mut Context) -> JsResult<f64> {
    let x = v.to_number(ctx)?;
    let (lo, hi) = if bits == 64 {
        // long long: limited to what a double holds exactly.
        if signed { (-(2f64.powi(53)) + 1.0, 2f64.powi(53) - 1.0) } else { (0.0, 2f64.powi(53) - 1.0) }
    } else if signed {
        (-(2f64.powi(bits as i32 - 1)), 2f64.powi(bits as i32 - 1) - 1.0)
    } else {
        (0.0, 2f64.powi(bits as i32) - 1.0)
    };
    match mode {
        IntMode::Enforce => {
            if !x.is_finite() {
                return Err(type_error("Value is not a finite number.".into()));
            }
            let x = x.trunc();
            if x < lo || x > hi {
                return Err(type_error(format!("Value is outside the range [{lo}, {hi}].")));
            }
            Ok(x + 0.0)
        }
        IntMode::Clamp => {
            if x.is_nan() {
                return Ok(0.0);
            }
            // Round half to even, then clamp.
            let c = x.clamp(lo, hi);
            let r = c.round();
            let r = if (c - c.trunc()).abs() == 0.5 && r % 2.0 != 0.0 { r - c.signum() } else { r };
            Ok(r + 0.0)
        }
        IntMode::Wrap => {
            if !x.is_finite() {
                return Ok(0.0);
            }
            let x = x.trunc();
            if bits == 64 {
                return Ok(x.clamp(lo, hi) + 0.0);
            }
            let m = 2f64.powi(bits as i32);
            let mut r = x.rem_euclid(m);
            if signed && r >= m / 2.0 {
                r -= m;
            }
            Ok(r + 0.0)
        }
    }
}

/// `double` / `float`: a TypeError for NaN and the infinities. `unrestricted` lets them through.
pub fn double(v: &JsValue, unrestricted: bool, ctx: &mut Context) -> JsResult<f64> {
    let x = v.to_number(ctx)?;
    if !unrestricted && !x.is_finite() {
        return Err(type_error("The provided double value is non-finite.".into()));
    }
    Ok(x)
}

/// A node argument. Nullable ones take null/undefined as None; others need a node.
pub fn node(v: &JsValue, nullable: bool, op: &str, iface: &str, pos: usize, ctx: &mut Context) -> JsResult<Option<NodeId>> {
    if nullable && v.is_null_or_undefined() {
        return Ok(None);
    }
    match node_of(v, ctx)? {
        Some(id) => Ok(id),
        None => Err(type_error(format!("Failed to execute '{op}' on '{iface}': parameter {pos} is not of type 'Node'."))),
    }
}

// ------------------------------------------------------------ Rust -> JavaScript

/// An error an implementation reports: thrown as a DOMException with this name
/// ("IndexSizeError", "NotFoundError", …).
#[derive(Debug, Clone, PartialEq)]
pub struct DomError {
    pub name: &'static str,
    pub message: String,
}

impl DomError {
    pub fn new(name: &'static str, message: impl Into<String>) -> Self {
        DomError { name, message: message.into() }
    }
}

/// `new DOMException(message, name)` as a thrown error (a plain Error if the page has no
/// DOMException).
pub fn dom_exception(e: DomError, ctx: &mut Context) -> boa_engine::JsError {
    let ctor = ctx.global_object().get(boa_engine::js_string!("DOMException"), ctx).ok();
    if let Some(c) = ctor.as_ref().and_then(JsValue::as_constructor) {
        let args = [JsValue::from(JsString::from(e.message.as_str())), JsValue::from(JsString::from(e.name))];
        if let Ok(obj) = c.construct(&args, None, ctx) {
            return boa_engine::JsError::from_opaque(obj.into());
        }
    }
    JsNativeError::error().with_message(format!("{}: {}", e.name, e.message)).into()
}

/// A hand-written implementation's result as a JavaScript value. Node results are
/// `Option<NodeId>` (null for None) and node lists `Vec<NodeId>`.
pub trait ToJs {
    fn to_js(self, ctx: &mut Context) -> JsResult<JsValue>;
}

impl ToJs for () {
    fn to_js(self, _: &mut Context) -> JsResult<JsValue> {
        Ok(JsValue::undefined())
    }
}

impl ToJs for bool {
    fn to_js(self, _: &mut Context) -> JsResult<JsValue> {
        Ok(JsValue::from(self))
    }
}

macro_rules! numbers {
    ($($t:ty),*) => {$(
        impl ToJs for $t {
            fn to_js(self, _: &mut Context) -> JsResult<JsValue> {
                Ok(JsValue::from(self as f64))
            }
        }
    )*};
}
numbers!(i8, u8, i16, u16, i32, u32, i64, u64, f32, f64);

impl ToJs for String {
    fn to_js(self, _: &mut Context) -> JsResult<JsValue> {
        Ok(JsValue::from(JsString::from(self.as_str())))
    }
}

impl ToJs for Option<String> {
    fn to_js(self, ctx: &mut Context) -> JsResult<JsValue> {
        match self {
            Some(s) => s.to_js(ctx),
            None => Ok(JsValue::null()),
        }
    }
}

impl ToJs for Option<NodeId> {
    fn to_js(self, ctx: &mut Context) -> JsResult<JsValue> {
        wrap(self, ctx)
    }
}

impl ToJs for Vec<NodeId> {
    fn to_js(self, ctx: &mut Context) -> JsResult<JsValue> {
        wrap_all(self, ctx)
    }
}

impl ToJs for Vec<String> {
    fn to_js(self, ctx: &mut Context) -> JsResult<JsValue> {
        let items: Vec<JsValue> = self.iter().map(|s| JsValue::from(JsString::from(s.as_str()))).collect();
        Ok(JsArray::from_iter(items, ctx).into())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn int(x: f64, bits: u32, signed: bool, mode: IntMode) -> JsResult<f64> {
        let mut ctx = Context::default();
        integer(&JsValue::from(x), bits, signed, mode, &mut ctx)
    }

    #[test]
    fn integers_follow_webidl() {
        use IntMode::*;
        // Wrap-around (ConvertToInt's modulo).
        assert_eq!(int(-1.0, 16, false, Wrap).unwrap(), 65535.0);
        assert_eq!(int(65536.0 + 5.0, 16, false, Wrap).unwrap(), 5.0);
        assert_eq!(int(2147483648.0, 32, true, Wrap).unwrap(), -2147483648.0);
        assert_eq!(int(-3.7, 32, true, Wrap).unwrap(), -3.0);
        assert_eq!(int(f64::NAN, 32, true, Wrap).unwrap(), 0.0);
        assert_eq!(int(f64::INFINITY, 8, false, Wrap).unwrap(), 0.0);
        assert_eq!(int(300.0, 8, false, Wrap).unwrap(), 44.0);
        // [EnforceRange] throws outside the range or for non-finite values.
        assert!(int(256.0, 8, false, Enforce).is_err());
        assert!(int(f64::NAN, 32, true, Enforce).is_err());
        assert_eq!(int(255.9, 8, false, Enforce).unwrap(), 255.0);
        // [Clamp] clamps and rounds half to even.
        assert_eq!(int(300.0, 8, false, Clamp).unwrap(), 255.0);
        assert_eq!(int(-5.0, 8, false, Clamp).unwrap(), 0.0);
        assert_eq!(int(2.5, 8, false, Clamp).unwrap(), 2.0);
        assert_eq!(int(3.5, 8, false, Clamp).unwrap(), 4.0);
        assert_eq!(int(f64::NAN, 8, false, Clamp).unwrap(), 0.0);
    }

    #[test]
    fn doubles_enums_and_argument_counts() {
        let mut ctx = Context::default();
        assert!(double(&JsValue::from(f64::NAN), false, &mut ctx).is_err());
        assert!(double(&JsValue::from(f64::INFINITY), true, &mut ctx).unwrap().is_infinite());
        let ok = enumeration(&JsValue::from(js_str("open")), &["open", "closed"], "ShadowRootMode", &mut ctx);
        assert_eq!(ok.unwrap(), "open");
        assert!(enumeration(&JsValue::from(js_str("x")), &["open", "closed"], "ShadowRootMode", &mut ctx).is_err());
        assert_eq!(string_null_empty(&JsValue::null(), &mut ctx).unwrap(), "");
        assert_eq!(string_or_null(&JsValue::undefined(), &mut ctx).unwrap(), None);
        assert!(require(&[], 1, "f", "X").is_err());
        assert!(require(&[JsValue::undefined()], 1, "f", "X").is_ok());
        assert!(absent(&[JsValue::undefined()], 0) && absent(&[], 3) && !absent(&[JsValue::from(1)], 0));
    }

    fn js_str(s: &str) -> JsString {
        JsString::from(s)
    }
}
