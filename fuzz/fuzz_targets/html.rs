//! The HTML tokenizer and tree builder (and the XHTML/XML path) on arbitrary input.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "../../src/dom.rs"]
#[allow(dead_code)]
mod dom;
#[path = "../../src/entities.rs"]
#[allow(dead_code)]
mod entities;
#[path = "../../src/html.rs"]
#[allow(dead_code)]
mod html;
#[path = "../../src/tokenizer.rs"]
#[allow(dead_code)]
mod tokenizer;
#[path = "../../src/treebuilder.rs"]
#[allow(dead_code)]
mod treebuilder;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let d = html::parse(&text);
    // Walk the result the way layout does, so a broken tree shows up here.
    let _ = d.descendants(dom::Dom::ROOT).len();
    let _ = treebuilder::parse_xml(&text);
    // Pausing at scripts (the path scripts take while a page loads).
    let mut tb = treebuilder::TreeBuilder::new(&text);
    tb.pause_at_scripts = true;
    while tb.pump().is_some() {
        tb.begin_script();
        tb.end_script();
    }
    let _ = tb.finish();
});

/// dom.rs keeps per-document form state; the parser doesn't need any.
mod validation {
    #[derive(Debug, Clone, Default)]
    pub struct FormState;
}
