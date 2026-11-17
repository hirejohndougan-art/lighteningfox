//! Filter-list parsing (EasyList syntax) and matching on arbitrary rules and URLs.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[macro_use]
#[path = "stubs.rs"]
mod stubs;
use stubs::{cookies, net, platform, settings, validation};

#[path = "../../src/dom.rs"]
#[allow(dead_code)]
mod dom;
#[path = "../../src/security.rs"]
#[allow(dead_code)]
mod security;
#[path = "../../src/blocklist.rs"]
#[allow(dead_code)]
mod blocklist;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (rules, urls) = text.split_once("\n\n").unwrap_or((&text, "https://ads.example/x.js"));
    let mut e = blocklist::Engine::default();
    e.add_list(rules, blocklist::Kind::Ads);
    e.add_list(rules, blocklist::Kind::Dangerous);
    for u in urls.lines() {
        let _ = e.check(u, "https://news.example/", security::Dest::Script);
        let _ = e.check(u, u, security::Dest::Image);
        let _ = e.is_dangerous(u);
        let _ = blocklist::clean_url(u, "https://news.example/", None);
    }
    let classes: std::collections::HashSet<String> = urls.split_whitespace().map(String::from).collect();
    let _ = e.hiding_css("https://news.example/", &classes, &classes);
});
