//! Record and replay network traffic, for debugging sites offline.
//!
//!     IRONSWIFT_RECORD=dir ironswift --png out.png https://site/   # saves every response in dir
//!     IRONSWIFT_REPLAY=dir ironswift --png out.png https://site/   # answers from dir, no network
//!
//! A recorded site behaves the same every time, so a crash or hang on a live site can be
//! captured once (on any machine with internet access) and then worked on anywhere. In replay
//! mode a request that wasn't recorded fails with a `[net]` error, as if the site were down.

use std::io::Read;
use std::path::PathBuf;

fn dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The folder responses are saved to, when recording.
pub fn recording() -> Option<PathBuf> {
    dir("IRONSWIFT_RECORD")
}

/// The folder responses are answered from, when replaying.
pub fn replaying() -> Option<PathBuf> {
    dir("IRONSWIFT_REPLAY")
}

/// The file name for a request: FNV-1a of the method and URL (stable across builds).
fn key(method: &str, url: &str) -> String {
    key_with(method, url, None)
}

/// A request with a body (a GraphQL POST) is also told apart by its body: one URL answers
/// many different queries (NYT's video player got another query's answer on replay).
fn key_with(method: &str, url: &str, body: Option<&[u8]>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let body = body.filter(|b| !b.is_empty());
    let tail: &[u8] = body.unwrap_or(&[]);
    let sep: &[u8] = if body.is_some() { b" " } else { b"" };
    for &b in method.to_ascii_uppercase().as_bytes().iter().chain(b" ").chain(url.as_bytes()).chain(sep).chain(tail) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A saved response: status, status text and headers.
pub struct Saved {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Save one response. The `.meta` file is plain text: the request line, the status line, then
/// one `name: value` header per line.
#[cfg(test)]
pub fn save(dir: &std::path::Path, method: &str, url: &str, r: &Saved) {
    save_with(dir, method, url, None, r);
}

/// `save` for a request that sent `body`: kept under the body too, and as the plain request
/// (what replays of recordings made before bodies counted look for).
pub fn save_with(dir: &std::path::Path, method: &str, url: &str, body: Option<&[u8]>, r: &Saved) {
    let _ = std::fs::create_dir_all(dir);
    if body.is_some_and(|b| !b.is_empty()) {
        write(dir, &key_with(method, url, body), method, url, r);
    }
    let k = key(method, url);
    // A revalidation (304, empty body) of something already saved: keep the full response.
    // Replaying with a fresh profile there is no cached copy for a 304 to refer to (NYT's
    // elections feed came back empty).
    if r.status == 304 && load(dir, method, url).is_some_and(|old| old.status != 304) {
        return;
    }
    write(dir, &k, method, url, r);
}

fn write(dir: &std::path::Path, k: &str, method: &str, url: &str, r: &Saved) {
    let mut meta = format!("{} {}\n{} {}\n", method.to_ascii_uppercase(), url, r.status, r.status_text);
    for (n, v) in &r.headers {
        meta.push_str(&format!("{n}: {}\n", v.replace(['\r', '\n'], " ")));
    }
    let _ = std::fs::write(dir.join(format!("{k}.body")), &r.body);
    let _ = std::fs::write(dir.join(format!("{k}.meta")), meta);
}

/// The saved response for a request, if there is one.
pub fn load(dir: &std::path::Path, method: &str, url: &str) -> Option<Saved> {
    load_key(dir, &key(method, url), url)
}

/// The saved response for a request that sent `body`: the one for that body if there is one,
/// else whatever was saved for the plain request.
pub fn load_with(dir: &std::path::Path, method: &str, url: &str, body: Option<&[u8]>) -> Option<Saved> {
    if body.is_some_and(|b| !b.is_empty()) {
        if let Some(s) = load_key(dir, &key_with(method, url, body), url) {
            return Some(s);
        }
    }
    load(dir, method, url)
}

fn load_key(dir: &std::path::Path, k: &str, url: &str) -> Option<Saved> {
    let meta = std::fs::read_to_string(dir.join(format!("{k}.meta"))).ok()?;
    let mut lines = meta.lines();
    let first = lines.next()?;
    if first.split_once(' ').map(|(_, u)| u) != Some(url) {
        return None;
    }
    let (status, text) = lines.next()?.split_once(' ').unwrap_or(("200", ""));
    let headers = lines.filter_map(|l| l.split_once(": ").map(|(n, v)| (n.to_string(), v.to_string()))).collect();
    let body = std::fs::read(dir.join(format!("{k}.body"))).ok()?;
    Some(Saved { status: status.parse().ok()?, status_text: text.to_string(), headers, body })
}

/// Read a whole body for saving (up to `max` bytes).
pub fn read_body(mut r: impl Read, max: u64) -> std::io::Result<Vec<u8>> {
    let mut v = Vec::new();
    r.by_ref().take(max).read_to_end(&mut v)?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_are_told_apart_by_their_body() {
        let dir = std::env::temp_dir().join(format!("ironswift-rec-post-{}", std::process::id()));
        let url = "https://example.com/graphql";
        let ok = |b: &[u8]| Saved { status: 200, status_text: "OK".into(), headers: vec![], body: b.to_vec() };
        save_with(&dir, "POST", url, Some(b"{\"q\":1}"), &ok(b"one"));
        save_with(&dir, "POST", url, Some(b"{\"q\":2}"), &ok(b"two"));
        assert_eq!(load_with(&dir, "POST", url, Some(b"{\"q\":1}")).unwrap().body, b"one");
        assert_eq!(load_with(&dir, "POST", url, Some(b"{\"q\":2}")).unwrap().body, b"two");
        // An unknown body (or an older recording) gets the last one saved for the URL.
        assert_eq!(load_with(&dir, "POST", url, Some(b"{\"q\":3}")).unwrap().body, b"two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_revalidation_keeps_the_full_response() {
        let dir = std::env::temp_dir().join(format!("ironswift-rec-304-{}", std::process::id()));
        let full = Saved { status: 200, status_text: "OK".into(), headers: vec![], body: b"{}".to_vec() };
        let not_modified = Saved { status: 304, status_text: "Not Modified".into(), headers: vec![], body: Vec::new() };
        save(&dir, "GET", "https://example.com/feed.json", &full);
        save(&dir, "GET", "https://example.com/feed.json", &not_modified);
        let back = load(&dir, "GET", "https://example.com/feed.json").unwrap();
        assert_eq!((back.status, back.body.as_slice()), (200, &b"{}"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_responses_come_back() {
        let d = std::env::temp_dir().join(format!("ironswift-record-test-{}", std::process::id()));
        let r = Saved { status: 404, status_text: "Not Found".into(), headers: vec![("content-type".into(), "text/html".into())], body: b"<p>gone".to_vec() };
        save(&d, "get", "https://a.example/x?y", &r);
        let back = load(&d, "GET", "https://a.example/x?y").unwrap();
        assert_eq!((back.status, back.status_text.as_str(), back.body.as_slice()), (404, "Not Found", &b"<p>gone"[..]));
        assert_eq!(back.headers, r.headers);
        assert!(load(&d, "GET", "https://a.example/other").is_none());
        assert!(load(&d, "POST", "https://a.example/x?y").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
