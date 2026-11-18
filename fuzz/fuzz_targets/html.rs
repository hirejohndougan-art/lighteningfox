//! The HTML tokenizer and tree builder (and the XHTML/XML path, and fragment parsing) on
//! arbitrary input.
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
    // Fragment parsing (innerHTML, insertAdjacentHTML) in tricky contexts, chosen by the
    // first byte (brought over from the parallel-work branch's html_fragment target).
    const CONTEXTS: &[&str] = &["div", "body", "table", "tr", "select", "template", "textarea", "script", "svg", "math", "html", "td", "title", "plaintext"];
    if let Some((&first, rest)) = data.split_first() {
        let ctx = CONTEXTS[first as usize % CONTEXTS.len()];
        let ns = match ctx {
            "svg" => dom::Ns::Svg,
            "math" => dom::Ns::MathMl,
            _ => dom::Ns::Html,
        };
        let frag = treebuilder::TreeBuilder::parse_fragment(&String::from_utf8_lossy(rest), ctx, ns, first & 0x80 != 0);
        let _ = frag.descendants(dom::Dom::ROOT).len();
    }
});

/// dom.rs keeps per-document form state; the parser doesn't need any.
mod validation {
    #[derive(Debug, Clone, Default)]
    pub struct FormState;
}
