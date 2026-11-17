//! The CSS parser, selector parser and URL rewriting on arbitrary input.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "../../src/css.rs"]
#[allow(dead_code)]
mod css;
#[path = "../../src/csssyntax.rs"]
#[allow(dead_code)]
mod csssyntax;
#[path = "../../src/dom.rs"]
#[allow(dead_code)]
mod dom;

/// Stand-ins for the browser modules css.rs reaches into.
mod settings {
    pub struct Settings {
        pub reduce_motion: bool,
        pub high_contrast: bool,
    }
    pub fn get() -> Settings {
        Settings { reduce_motion: false, high_contrast: false }
    }
}
/// `@keyframes` parsing lives in anim.rs, which needs the whole style engine: left out here.
mod anim {
    #[derive(Debug, Clone, PartialEq)]
    pub struct Keyframes;
    pub fn find_keyframes(_: &str) -> Vec<Keyframes> {
        Vec::new()
    }
}
mod validation {
    #[derive(Debug, Clone, Default)]
    pub struct FormState;
}
mod net {
    pub fn resolve(base: &str, href: &str) -> String {
        url::Url::parse(base).and_then(|b| b.join(href)).map(|u| u.to_string()).unwrap_or_else(|_| href.to_string())
    }
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut sheet = css::parse(&text);
    css::absolutize_urls(&mut sheet, "https://example.com/a/b.css");
    let _ = css::parse_selector_list(&text);
});
