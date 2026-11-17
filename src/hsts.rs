//! HTTPS everywhere a site asks for it, and no insecure pieces in secure pages:
//!
//! - HSTS: a site that sends `Strict-Transport-Security` over HTTPS is only ever contacted
//!   over HTTPS until the header's max-age runs out (with `includeSubDomains`, its
//!   subdomains too). Remembered in `<profile>/hsts.txt`.
//! - Mixed content: anything an HTTPS page loads over plain HTTP is upgraded to HTTPS; if the
//!   server doesn't answer there, the request fails rather than going out unencrypted.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// host -> (expires at, unix seconds; includes subdomains)
static HOSTS: Mutex<Option<HashMap<String, (u64, bool)>>> = Mutex::new(None);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn file() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("hsts.txt"))
}

fn private() -> bool {
    std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1")
}

fn with<R>(f: impl FnOnce(&mut HashMap<String, (u64, bool)>) -> R) -> R {
    let mut g = HOSTS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(|| {
        let mut m = HashMap::new();
        if let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                let mut p = line.split(' ');
                if let (Some(h), Some(exp), Some(sub)) = (p.next(), p.next().and_then(|e| e.parse().ok()), p.next()) {
                    m.insert(h.to_string(), (exp, sub == "1"));
                }
            }
        }
        m
    });
    f(map)
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// Hosts that are trustworthy without HTTPS (your own computer).
fn local(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host.starts_with("127.") || host == "[::1]" || host == "::1"
}

/// Is `host` (or a parent domain with includeSubDomains) on the HSTS list?
fn known(host: &str) -> bool {
    let t = now();
    with(|m| {
        if m.get(host).is_some_and(|&(exp, _)| exp > t) {
            return true;
        }
        let mut rest = host;
        while let Some((_, parent)) = rest.split_once('.') {
            if m.get(parent).is_some_and(|&(exp, sub)| sub && exp > t) {
                return true;
            }
            rest = parent;
        }
        false
    })
}

/// The URL to actually request: http:// becomes https:// for HSTS hosts, and for anything
/// loaded by an HTTPS page (`top`).
pub fn upgrade(url: &str, top: Option<&str>) -> String {
    if !url.starts_with("http://") {
        return url.to_string();
    }
    let Ok(u) = url::Url::parse(url) else { return url.to_string() };
    let Some(host) = u.host_str().map(str::to_ascii_lowercase) else { return url.to_string() };
    if local(&host) {
        return url.to_string();
    }
    let mixed = top.is_some_and(|t| t.starts_with("https://"));
    if !(mixed || known(&host)) {
        return url.to_string();
    }
    // Same address over HTTPS; a port other than 80 stays (80 is dropped as http's default).
    let mut out = format!("https://{host}");
    if let Some(p) = u.port() {
        out.push_str(&format!(":{p}"));
    }
    out.push_str(u.path());
    if let Some(q) = u.query() {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = u.fragment() {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// A response's Strict-Transport-Security header (only counts over HTTPS).
pub fn record(url: &str, header: &str) {
    if !url.starts_with("https://") {
        return;
    }
    let Some(host) = host_of(url) else { return };
    if host.parse::<std::net::IpAddr>().is_ok() || local(&host) {
        return;
    }
    let mut max_age: Option<u64> = None;
    let mut sub = false;
    for part in header.split(';') {
        let p = part.trim();
        if let Some(v) = p.to_ascii_lowercase().strip_prefix("max-age=") {
            max_age = v.trim_matches('"').parse().ok();
        } else if p.eq_ignore_ascii_case("includesubdomains") {
            sub = true;
        }
    }
    let Some(age) = max_age else { return };
    let text = with(|m| {
        if age == 0 {
            m.remove(&host);
        } else {
            m.insert(host, (now() + age, sub));
        }
        let t = now();
        let mut lines: Vec<String> = m.iter().filter(|(_, (e, _))| *e > t).map(|(h, (e, s))| format!("{h} {e} {}", if *s { 1 } else { 0 })).collect();
        lines.sort();
        lines.join("\n")
    });
    if !private() {
        if let Some(f) = file() {
            let _ = std::fs::write(f, text);
        }
    }
}

/// Forget everything (Clear browsing data).
pub fn clear() {
    with(|m| m.clear());
    if let Some(f) = file() {
        let _ = std::fs::remove_file(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsts_and_mixed_content() {
        // A page on https loading http: upgraded; localhost left alone.
        assert_eq!(upgrade("http://cdn.example.org/a.js", Some("https://example.org/")), "https://cdn.example.org/a.js");
        assert_eq!(upgrade("http://localhost:8080/a.js", Some("https://example.org/")), "http://localhost:8080/a.js");
        assert_eq!(upgrade("http://plain.example.net/", None), "http://plain.example.net/");
        with(|m| {
            m.insert("hsts-test.example".into(), (now() + 100, true));
        });
        assert_eq!(upgrade("http://hsts-test.example/x", None), "https://hsts-test.example/x");
        assert_eq!(upgrade("http://www.hsts-test.example:80/x", None), "https://www.hsts-test.example/x");
    }
}
