//! Content Security Policy parsing and matching on arbitrary policies and URLs.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[macro_use]
#[path = "stubs.rs"]
mod stubs;
use stubs::{cookies, net, platform, settings};

#[path = "../../src/security.rs"]
#[allow(dead_code)]
mod security;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    // First line: the policy; the rest: URLs and inline code to check against it.
    let mut lines = text.lines();
    let policy = lines.next().unwrap_or("");
    let headers = vec![("content-security-policy".to_string(), policy.to_string()), ("x-frame-options".to_string(), "SAMEORIGIN".to_string())];
    let p = security::PagePolicy::from_headers("https://site.example/page", &headers, &[policy.to_string()]);
    for line in lines {
        for dest in [security::Dest::Script, security::Dest::Image, security::Dest::Frame, security::Dest::Connect] {
            let _ = p.csp.allows_url(dest, line, Some(line), true);
        }
        let _ = p.csp.allows_inline(security::Inline::Script, line, None);
        let _ = p.csp.allows_form_action(line);
        let _ = p.may_be_framed_by("https://site.example/page", line, line);
        let _ = security::page_may_navigate(line, line);
        let _ = security::Sandbox::parse(line);
    }
    let _ = p.csp.allows_eval();
});
