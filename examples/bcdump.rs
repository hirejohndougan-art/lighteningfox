//! Print the bytecode Boa compiles a script to (with every nested function):
//!   cargo run --release --example bcdump -- file.js
use boa_engine::{Context, Script, Source};

fn main() {
    let path = std::env::args().nth(1).expect("usage: bcdump file.js");
    let src = std::fs::read_to_string(&path).expect("read");
    let mut ctx = Context::default();
    let script = Script::parse(Source::from_bytes(src.as_bytes()), None, &mut ctx).expect("parse");
    let cb = script.codeblock(&mut ctx).expect("compile");
    println!("{}", cb.disassemble_all());
}
