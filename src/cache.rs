//! The HTTP disk cache (in `<profile>/cache`), following HTTP's caching rules:
//!
//! - A response may be stored unless it says `no-store` or varies on more than encoding.
//! - It's reused without asking the server while fresh: `Cache-Control: max-age`, else
//!   `Expires`, else 10% of its age since `Last-Modified` (at most a day).
//! - When stale (or marked `no-cache`) but it has an `ETag` or `Last-Modified`, the server is
//!   asked "has it changed?" (`If-None-Match` / `If-Modified-Since`); a `304 Not Modified` means
//!   the stored copy is used again, without downloading it.
//!
//! At most 200 MB is kept; the least recently used entries go first. Private mode stores nothing.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TOTAL: u64 = 200 * 1024 * 1024;
/// Responses bigger than this aren't worth the disk.
const MAX_ENTRY: usize = 20 * 1024 * 1024;

pub struct Entry {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    stored: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn enabled() -> bool {
    // A site process can't reach the cache: the browser looks it up when it fetches for it.
    !crate::process::in_renderer() && !std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1") && !std::env::var("IRONSWIFT_NO_CACHE").is_ok_and(|v| v == "1")
}

fn dir() -> Option<PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("cache"))
}

/// FNV-1a: a stable file name for a URL.
fn key(url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// The `Cache-Control` directives, lower-cased: ("max-age", Some("60")), ("no-store", None)…
fn directives(headers: &[(String, String)]) -> Vec<(String, Option<String>)> {
    headers
        .iter()
        .filter(|(k, _)| k == "cache-control" || k == "pragma")
        .flat_map(|(_, v)| v.split(',').map(str::to_string).collect::<Vec<_>>())
        .map(|d| {
            let d = d.trim().to_ascii_lowercase();
            match d.split_once('=') {
                Some((k, v)) => (k.trim().to_string(), Some(v.trim().trim_matches('"').to_string())),
                None => (d, None),
            }
        })
        .collect()
}

fn has(dirs: &[(String, Option<String>)], name: &str) -> bool {
    dirs.iter().any(|(k, _)| k == name)
}

/// HTTP dates (`Sun, 06 Nov 1994 08:49:37 GMT`) to seconds since 1970.
fn parse_http_date(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let day: u64 = parts[1].parse().ok()?;
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| parts[2].to_ascii_lowercase().starts_with(m))? as u64 + 1;
    let year: u64 = parts[3].parse().ok()?;
    let t: Vec<u64> = parts[4].split(':').filter_map(|x| x.parse().ok()).collect();
    if t.len() != 3 || year < 1970 {
        return None;
    }
    // Days since 1970 (civil-from-days, inverted).
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + t[0] * 3600 + t[1] * 60 + t[2])
}

/// How many seconds a response stays fresh after it was stored.
fn lifetime(headers: &[(String, String)]) -> u64 {
    let dirs = directives(headers);
    if has(&dirs, "no-cache") {
        return 0;
    }
    if let Some(age) = dirs.iter().find(|(k, _)| k == "max-age").and_then(|(_, v)| v.as_deref()?.parse::<u64>().ok()) {
        return age;
    }
    let date = header(headers, "date").and_then(parse_http_date).unwrap_or_else(now);
    if let Some(exp) = header(headers, "expires") {
        return parse_http_date(exp).map_or(0, |e| e.saturating_sub(date));
    }
    if let Some(lm) = header(headers, "last-modified").and_then(parse_http_date) {
        return (date.saturating_sub(lm) / 10).min(86_400);
    }
    0
}

/// Whether a response may be stored at all.
pub fn storable(status: u16, headers: &[(String, String)], len: usize) -> bool {
    let dirs = directives(headers);
    let varies = header(headers, "vary").is_some_and(|v| v.split(',').any(|f| !f.trim().eq_ignore_ascii_case("accept-encoding")));
    status == 200
        && len <= MAX_ENTRY
        && !has(&dirs, "no-store")
        && !varies
        && (lifetime(headers) > 0 || header(headers, "etag").is_some() || header(headers, "last-modified").is_some())
}

/// What the cache has for a URL: a fresh copy to use as is, or a stale one with the headers to
/// ask the server whether it changed.
pub enum Lookup {
    Fresh(Entry),
    Revalidate(Entry, Vec<(String, String)>),
    Miss,
}

pub fn lookup(url: &str) -> Lookup {
    if !enabled() {
        return Lookup::Miss;
    }
    let Some(e) = read(url) else { return Lookup::Miss };
    if now().saturating_sub(e.stored) < lifetime(&e.headers) {
        touch(url);
        return Lookup::Fresh(e);
    }
    let mut cond = Vec::new();
    if let Some(etag) = header(&e.headers, "etag") {
        cond.push(("If-None-Match".to_string(), etag.to_string()));
    }
    if let Some(lm) = header(&e.headers, "last-modified") {
        cond.push(("If-Modified-Since".to_string(), lm.to_string()));
    }
    if cond.is_empty() { Lookup::Miss } else { Lookup::Revalidate(e, cond) }
}

/// Store a response (headers that describe the transfer rather than the content are dropped:
/// the body is stored already decompressed).
pub fn store(url: &str, status: u16, headers: &[(String, String)], body: &[u8]) {
    if !enabled() || !storable(status, headers, body.len()) {
        return;
    }
    let Some(d) = dir() else { return };
    let _ = std::fs::create_dir_all(&d);
    let kept: Vec<&(String, String)> = headers
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "set-cookie" | "content-encoding" | "content-length" | "transfer-encoding" | "connection" | "alt-svc"))
        .collect();
    let mut meta = format!("{url}\n{status}\n{}\n", now());
    for (k, v) in kept {
        meta.push_str(&format!("{k}: {}\n", v.replace('\n', " ")));
    }
    let k = key(url);
    let ok = std::fs::write(d.join(format!("{k}.meta")), meta).is_ok() && std::fs::write(d.join(format!("{k}.body")), body).is_ok();
    if ok {
        trim(&d);
    }
}

/// The server said 304 Not Modified: the stored copy is good for another lifetime; headers it
/// sent along replace the stored ones.
pub fn refreshed(url: &str, new_headers: &[(String, String)]) -> Option<Entry> {
    let mut e = read(url)?;
    for (k, v) in new_headers {
        if matches!(k.as_str(), "cache-control" | "expires" | "etag" | "last-modified" | "date") {
            e.headers.retain(|(hk, _)| hk != k);
            e.headers.push((k.clone(), v.clone()));
        }
    }
    store(url, e.status, &e.headers, &e.body);
    Some(e)
}

fn read(url: &str) -> Option<Entry> {
    let d = dir()?;
    let k = key(url);
    let meta = std::fs::read_to_string(d.join(format!("{k}.meta"))).ok()?;
    let mut lines = meta.lines();
    if lines.next()? != url {
        return None; // a different URL with the same hash
    }
    let status: u16 = lines.next()?.parse().ok()?;
    let stored: u64 = lines.next()?.parse().ok()?;
    let headers = lines.filter_map(|l| l.split_once(": ").map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    let body = std::fs::read(d.join(format!("{k}.body"))).ok()?;
    Some(Entry { status, headers, body, stored })
}

/// Mark an entry as just used (for least-recently-used eviction).
fn touch(url: &str) {
    if let Some(d) = dir() {
        let p = d.join(format!("{}.meta", key(url)));
        if let Ok(f) = std::fs::File::options().append(true).open(&p) {
            let _ = f.set_modified(SystemTime::now());
        }
    }
}

/// Keep the cache under its size limit, dropping the least recently used entries.
fn trim(d: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    let mut entries: Vec<(SystemTime, String, u64)> = Vec::new();
    let mut total = 0u64;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(k) = name.strip_suffix(".meta") else { continue };
        let meta_len = e.metadata().map_or(0, |m| m.len());
        let body_len = std::fs::metadata(d.join(format!("{k}.body"))).map_or(0, |m| m.len());
        let used = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        total += meta_len + body_len;
        entries.push((used, k.to_string(), meta_len + body_len));
    }
    if total <= MAX_TOTAL {
        return;
    }
    entries.sort();
    for (_, k, size) in entries {
        if total <= MAX_TOTAL * 9 / 10 {
            break;
        }
        let _ = std::fs::remove_file(d.join(format!("{k}.meta")));
        let _ = std::fs::remove_file(d.join(format!("{k}.body")));
        total = total.saturating_sub(size);
    }
}

/// Delete everything (Settings → Clear cached files).
pub fn clear() -> u64 {
    let Some(d) = dir() else { return 0 };
    let mut freed = 0;
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            freed += e.metadata().map_or(0, |m| m.len());
            let _ = std::fs::remove_file(e.path());
        }
    }
    freed
}

/// Disk space the cache uses now.
pub fn size() -> u64 {
    dir().and_then(|d| std::fs::read_dir(d).ok()).map_or(0, |rd| rd.flatten().map(|e| e.metadata().map_or(0, |m| m.len())).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn dates() {
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784111777));
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
    }

    #[test]
    fn freshness_rules() {
        assert_eq!(lifetime(&h(&[("cache-control", "public, max-age=600")])), 600);
        assert_eq!(lifetime(&h(&[("cache-control", "max-age=600, no-cache")])), 0);
        assert_eq!(lifetime(&h(&[("date", "Sun, 06 Nov 1994 08:49:37 GMT"), ("expires", "Sun, 06 Nov 1994 09:49:37 GMT")])), 3600);
        // Heuristic: a tenth of the time since it last changed.
        assert_eq!(lifetime(&h(&[("date", "Sun, 06 Nov 1994 08:49:37 GMT"), ("last-modified", "Sun, 06 Nov 1994 07:49:37 GMT")])), 360);
    }

    #[test]
    fn what_may_be_stored() {
        assert!(storable(200, &h(&[("cache-control", "max-age=60")]), 10));
        assert!(storable(200, &h(&[("etag", "\"x\"")]), 10));
        assert!(!storable(200, &h(&[("cache-control", "no-store, max-age=60")]), 10));
        assert!(!storable(404, &h(&[("cache-control", "max-age=60")]), 10));
        assert!(!storable(200, &h(&[("cache-control", "max-age=60"), ("vary", "Cookie")]), 10));
        assert!(storable(200, &h(&[("cache-control", "max-age=60"), ("vary", "Accept-Encoding")]), 10));
        assert!(!storable(200, &h(&[]), 10));
    }
}
