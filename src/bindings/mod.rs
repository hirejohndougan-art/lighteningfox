//! Native DOM bindings generated from the WebIDL in the specs.
//!
//! - `generated.rs` is written by `tools/webidl/gen.py` from the spec IDL in
//!   `tools/webidl/idl/` and the list of natively implemented members in
//!   `tools/webidl/bindings.json`. It holds the glue the WebIDL spec asks for (argument
//!   counts, argument and result conversions, `this` checks, function lengths).
//! - `conv.rs` is the hand-written WebIDL conversion runtime that glue calls.
//! - `dom.rs` holds the implementations: plain typed Rust over the DOM tree.
//!
//! To make another member native: add it to bindings.json, write its function in dom.rs with
//! the signature the generator's comment shows, and run `python3 tools/webidl/gen.py`.
//! `dom_classes.rs` puts the members on the prelude's classes.

pub mod conv;
pub mod dom;
mod generated;

use boa_engine::{Context, JsResult, JsValue};

pub type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// One interface's native members: getters by name, methods by name with their `length`.
pub struct Interface {
    pub name: &'static str,
    pub getters: &'static [(&'static str, Native)],
    pub methods: &'static [(&'static str, Native, usize)],
}

pub use generated::INTERFACES;

#[cfg(test)]
mod tests {
    /// The checked-in generated.rs matches the IDL and bindings.json.
    #[test]
    fn generated_bindings_are_current() {
        let Ok(out) = std::process::Command::new("python3").args(["tools/webidl/gen.py", "--check"]).output() else {
            eprintln!("python3 not available; skipping");
            return;
        };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
