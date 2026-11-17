//! The browser's parsers, compiled on their own for fuzzing. The modules are the real source
//! files from ../src; the two small modules below stand in for the parts of the browser the
//! CSS parser reaches into (settings and URL resolution).

#![allow(dead_code, unused_imports, clippy::all)]

#[path = "../../src/css.rs"]
pub mod css;
#[path = "../../src/dom.rs"]
pub mod dom;
#[path = "../../src/entities.rs"]
pub mod entities;
#[path = "../../src/html.rs"]
pub mod html;
#[path = "../../src/jsnest.rs"]
pub mod jsnest;
#[path = "../../src/tokenizer.rs"]
pub mod tokenizer;
#[path = "../../src/treebuilder.rs"]
pub mod treebuilder;

/// Stand-in for the browser's settings (the CSS parser reads one value).
pub mod settings {
    #[derive(Default)]
    pub struct Settings {
        pub reduce_motion: bool,
    }
    pub fn get() -> Settings {
        Settings::default()
    }
}

/// Stand-in for URL resolution: joins like a browser would for the cases CSS needs.
pub mod net {
    pub fn resolve(base: &str, href: &str) -> String {
        if href.contains("://") || href.starts_with("data:") {
            href.to_string()
        } else {
            format!("{}/{}", base.trim_end_matches('/'), href.trim_start_matches('/'))
        }
    }
}

fn text(data: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(data)
}

/// Parse a whole HTML document and walk the result (dumping it checks the tree's links).
pub fn html_document(data: &[u8]) {
    let dom = html::parse(&text(data));
    let _ = treebuilder::dump(&dom);
}

/// innerHTML-style parsing, with a context element taken from the first byte.
pub fn html_fragment(data: &[u8]) {
    const CONTEXTS: &[&str] = &["div", "body", "table", "tr", "select", "template", "textarea", "script", "svg", "math", "html", "td", "title", "plaintext"];
    let Some((&first, rest)) = data.split_first() else { return };
    let ctx = CONTEXTS[first as usize % CONTEXTS.len()];
    let ns = match ctx {
        "svg" => dom::Ns::Svg,
        "math" => dom::Ns::MathMl,
        _ => dom::Ns::Html,
    };
    let dom = treebuilder::TreeBuilder::parse_fragment(&text(rest), ctx, ns, first & 0x80 != 0);
    let _ = treebuilder::dump(&dom);
}

pub fn xhtml(data: &[u8]) {
    let dom = treebuilder::parse_xml(&text(data));
    let _ = treebuilder::dump(&dom);
}

/// Style sheets, inline declarations and selector lists.
pub fn css(data: &[u8]) {
    let t = text(data);
    let _ = css::parse(&t);
    let _ = css::parse_declarations(&t);
    let _ = css::parse_selector_list(&t);
}

/// Boa's JavaScript parser, as a script and as a module.
/// Scripts the browser would refuse as too deeply nested are skipped, as the browser does.
/// (The limit here is lower than the browser's 1,000 because the fuzzers run on an 8 MB stack
/// and the browser on 512 MB.)
pub fn javascript(data: &[u8]) {
    use boa_parser::{Parser, Source};
    if jsnest::too_deep(&text(data), 250).is_some() {
        return;
    }
    let mut interner = boa_interner::Interner::default();
    let _ = Parser::new(Source::from_bytes(data)).parse_script(&boa_ast::scope::Scope::new_global(), &mut interner);
    let mut interner = boa_interner::Interner::default();
    let _ = Parser::new(Source::from_bytes(data)).parse_module(&boa_ast::scope::Scope::new_global(), &mut interner);
}
