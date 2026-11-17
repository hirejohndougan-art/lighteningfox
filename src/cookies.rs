//! Cookies: parsing Set-Cookie, choosing which cookies go with a request, keeping
//! them on disk between runs, and blocking third-party cookies.
//!
//! Privacy defaults: a cookie is only stored or sent when the request goes to the same
//! site as the page you're on, so ad and tracking networks embedded in pages can't follow
//! you from site to site. Set `LF_THIRD_PARTY_COOKIES=1` to allow them.

use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lower-case domain without a leading dot.
    pub domain: String,
    /// True when set without a Domain attribute: only sent back to exactly this host.
    pub host_only: bool,
    pub path: String,
    /// Unix seconds; `None` = a session cookie, forgotten when the browser closes.
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

const MAX_COOKIE_BYTES: usize = 4096;
const MAX_PER_DOMAIN: usize = 180;
const MAX_TOTAL: usize = 3000;

#[derive(Default)]
pub struct Jar {
    cookies: Vec<Cookie>,
    loaded: bool,
}

fn jar() -> &'static Mutex<Jar> {
    static JAR: OnceLock<Mutex<Jar>> = OnceLock::new();
    JAR.get_or_init(|| Mutex::new(Jar::default()))
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// (scheme, lower-case host, path) of an http(s) URL.
fn parts(url: &str) -> Option<(&str, String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host.starts_with('[') { host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { host.split(':').next().unwrap_or("").to_string() };
    let path = rest[end..].split(['?', '#']).next().unwrap_or("");
    let path = if path.starts_with('/') { path.to_string() } else { "/".to_string() };
    Some((scheme, host.to_ascii_lowercase(), path))
}

/// The "site" a host belongs to: its registrable domain (example.co.uk, example.com).
/// A short built-in list covers the common two-part public suffixes.
pub fn site_of(host: &str) -> String {
    let host = host.trim_end_matches('.');
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') || !host.contains('.') {
        return host.to_string();
    }
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    const SECOND: &[&str] = &["co", "com", "org", "net", "gov", "ac", "edu", "ne", "or", "go", "gob", "mil", "nic", "ltd", "plc", "sch", "nhs", "police"];
    let take = if n >= 3 && labels[n - 1].len() == 2 && SECOND.contains(&labels[n - 2]) { 3 } else { 2 };
    labels[n.saturating_sub(take)..].join(".")
}

fn site_of_url(url: &str) -> Option<String> {
    parts(url).map(|(_, h, _)| site_of(&h))
}

/// Whether cookies may flow for a request to `url` made by a page at `top`.
pub fn allowed(url: &str, top: &str) -> bool {
    if std::env::var("LF_THIRD_PARTY_COOKIES").is_ok_and(|v| v == "1") {
        return true;
    }
    match (site_of_url(url), site_of_url(top)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn domain_match(host: &str, domain: &str) -> bool {
    host == domain || (host.ends_with(domain) && host.as_bytes().get(host.len() - domain.len() - 1) == Some(&b'.') && host.parse::<std::net::IpAddr>().is_err())
}

fn path_match(req: &str, cookie: &str) -> bool {
    req == cookie || (req.starts_with(cookie) && (cookie.ends_with('/') || req.as_bytes().get(cookie.len()) == Some(&b'/')))
}

fn default_path(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => path[..i].to_string(),
    }
}

/// Parse an HTTP date like "Wed, 21 Oct 2026 07:28:00 GMT" (and the old variants) to Unix seconds.
pub fn parse_http_date(s: &str) -> Option<i64> {
    let mut day = None;
    let mut month = None;
    let mut year = None;
    let mut time = None;
    for tok in s.split(|c: char| !c.is_ascii_alphanumeric() && c != ':').filter(|t| !t.is_empty()) {
        if time.is_none() && tok.matches(':').count() == 2 {
            let p: Vec<i64> = tok.split(':').filter_map(|x| x.parse().ok()).collect();
            if p.len() == 3 {
                time = Some(p[0] * 3600 + p[1] * 60 + p[2]);
                continue;
            }
        }
        if let Ok(n) = tok.parse::<i64>() {
            if day.is_none() && tok.len() <= 2 {
                day = Some(n);
            } else if year.is_none() {
                year = Some(match n {
                    0..=69 => n + 2000,
                    70..=99 => n + 1900,
                    _ => n,
                });
            }
            continue;
        }
        let m = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
        if month.is_none() && tok.len() >= 3 {
            if let Some(i) = m.iter().position(|mm| tok[..3].eq_ignore_ascii_case(mm)) {
                month = Some(i as i64 + 1);
            }
        }
    }
    let (y, m, d) = (year?, month?, day?);
    // Days from 1970-01-01 (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + time.unwrap_or(0))
}

/// Parse one Set-Cookie header (or a document.cookie assignment) received from `url`.
pub fn parse(header: &str, url: &str, from_script: bool) -> Option<Cookie> {
    let (scheme, host, path) = parts(url)?;
    let mut segs = header.split(';');
    let pair = segs.next()?.trim();
    let (name, value) = match pair.split_once('=') {
        Some((n, v)) => (n.trim(), v.trim()),
        None => ("", pair),
    };
    if name.is_empty() && value.is_empty() || name.len() + value.len() > MAX_COOKIE_BYTES {
        return None;
    }
    let value = value.trim_matches('"');
    let mut c = Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: host.clone(),
        host_only: true,
        path: default_path(&path),
        expires: None,
        secure: false,
        http_only: false,
        same_site: SameSite::Lax,
    };
    let mut max_age: Option<i64> = None;
    for attr in segs {
        let (k, v) = match attr.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => (attr.trim(), ""),
        };
        match k.to_ascii_lowercase().as_str() {
            "domain" => {
                let d = v.trim_start_matches('.').to_ascii_lowercase();
                if !d.is_empty() {
                    // Must cover the host that set it, and can't be a whole public suffix.
                    if !domain_match(&host, &d) || site_of(&d) != site_of(&host) || !d.contains('.') && d != host {
                        return None;
                    }
                    c.domain = d;
                    c.host_only = false;
                }
            }
            "path" if v.starts_with('/') => c.path = v.to_string(),
            "expires" => {
                if max_age.is_none() {
                    c.expires = parse_http_date(v);
                }
            }
            "max-age" => {
                if let Ok(n) = v.parse::<i64>() {
                    max_age = Some(n);
                    c.expires = Some(if n <= 0 { 0 } else { now() + n });
                }
            }
            "secure" => c.secure = true,
            "httponly" => c.http_only = true,
            "samesite" => {
                c.same_site = match v.to_ascii_lowercase().as_str() {
                    "strict" => SameSite::Strict,
                    "none" => SameSite::None,
                    _ => SameSite::Lax,
                }
            }
            _ => {}
        }
    }
    if from_script && c.http_only {
        return None;
    }
    if c.secure && scheme != "https" && host != "localhost" {
        return None;
    }
    if c.name.starts_with("__Secure-") && !c.secure {
        return None;
    }
    if c.name.starts_with("__Host-") && (!c.secure || !c.host_only || c.path != "/") {
        return None;
    }
    Some(c)
}

impl Jar {
    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.loaded = true;
            self.cookies = load_file();
        }
    }

    fn store(&mut self, c: Cookie, from_script: bool) {
        self.ensure_loaded();
        let pos = self.cookies.iter().position(|o| o.name == c.name && o.domain == c.domain && o.path == c.path);
        if let Some(i) = pos {
            // A script can't overwrite an HttpOnly cookie.
            if from_script && self.cookies[i].http_only {
                return;
            }
            self.cookies.remove(i);
        }
        let expired = c.expires.is_some_and(|e| e <= now());
        let persistent = c.expires.is_some() || pos.is_some();
        if !expired {
            self.cookies.push(c);
        }
        self.trim();
        if persistent {
            save_file(&self.cookies);
        }
    }

    /// Drop expired cookies, then the oldest ones past the limits.
    fn trim(&mut self) {
        let t = now();
        self.cookies.retain(|c| c.expires.is_none_or(|e| e > t));
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut keep = vec![true; self.cookies.len()];
        for (i, c) in self.cookies.iter().enumerate().rev() {
            let n = seen.entry(c.domain.clone()).or_default();
            *n += 1;
            if *n > MAX_PER_DOMAIN {
                keep[i] = false;
            }
        }
        let mut i = 0;
        self.cookies.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        if self.cookies.len() > MAX_TOTAL {
            let extra = self.cookies.len() - MAX_TOTAL;
            self.cookies.drain(..extra);
        }
    }

    fn matching(&mut self, url: &str, for_script: bool) -> Vec<&Cookie> {
        self.ensure_loaded();
        let Some((scheme, host, path)) = parts(url) else { return Vec::new() };
        let t = now();
        let mut out: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| {
                (if c.host_only { host == c.domain } else { domain_match(&host, &c.domain) })
                    && path_match(&path, &c.path)
                    && (!c.secure || scheme == "https" || host == "localhost")
                    && c.expires.is_none_or(|e| e > t)
                    && !(for_script && c.http_only)
            })
            .collect();
        // Longer paths first, as browsers send them.
        out.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        out
    }
}

fn join(cookies: &[&Cookie]) -> String {
    cookies.iter().map(|c| if c.name.is_empty() { c.value.clone() } else { format!("{}={}", c.name, c.value) }).collect::<Vec<_>>().join("; ")
}

/// The Cookie header for a request to `url` from a page at `top` (None = send nothing).
pub fn header_for(url: &str, top: &str) -> Option<String> {
    if !allowed(url, top) {
        return None;
    }
    let mut j = jar().lock().ok()?;
    let h = join(&j.matching(url, false));
    (!h.is_empty()).then_some(h)
}

/// Store the Set-Cookie headers of a response to `url`, requested by a page at `top`.
pub fn receive(url: &str, top: &str, headers: &[String]) {
    if headers.is_empty() || !allowed(url, top) {
        return;
    }
    if let Ok(mut j) = jar().lock() {
        for h in headers {
            if let Some(c) = parse(h, url, false) {
                j.store(c, false);
            }
        }
    }
}

/// `document.cookie` for a page: every non-HttpOnly cookie that applies to it.
pub fn script_get(url: &str) -> String {
    jar().lock().map(|mut j| join(&j.matching(url, true))).unwrap_or_default()
}

/// `document.cookie = "…"` from a page's script.
pub fn script_set(url: &str, s: &str) {
    if let (Some(c), Ok(mut j)) = (parse(s, url, true), jar().lock()) {
        j.store(c, true);
    }
}

/// Forget every cookie (for "clear browsing data").
#[allow(dead_code)]
pub fn clear() {
    if let Ok(mut j) = jar().lock() {
        j.cookies.clear();
        j.loaded = true;
        save_file(&j.cookies);
    }
}

/// Where the browser keeps its data: $LF_PROFILE, else ~/.local/share/lighteningfox.
pub fn profile_dir() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("LF_PROFILE") {
        return Some(p.into());
    }
    let base = std::env::var("XDG_DATA_HOME").ok().map(std::path::PathBuf::from).or_else(|| std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".local/share")))?;
    Some(base.join("lighteningfox"))
}

fn cookie_file() -> Option<std::path::PathBuf> {
    profile_dir().map(|d| d.join("cookies.tsv"))
}

/// One cookie per line, tab-separated. Only cookies with an expiry date are kept.
fn save_file(cookies: &[Cookie]) {
    let Some(path) = cookie_file() else { return };
    let mut out = String::from("# LighteningFox cookies: domain\thost_only\tpath\texpires\tsecure\thttp_only\tsame_site\tname\tvalue\n");
    for c in cookies.iter().filter(|c| c.expires.is_some()) {
        let ss = match c.same_site {
            SameSite::Strict => "strict",
            SameSite::Lax => "lax",
            SameSite::None => "none",
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c.domain,
            c.host_only as u8,
            c.path,
            c.expires.unwrap_or(0),
            c.secure as u8,
            c.http_only as u8,
            ss,
            c.name.replace(['\t', '\n'], " "),
            c.value.replace(['\t', '\n'], " ")
        ));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write then rename, so a crash never leaves a half-written file.
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, out).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn load_file() -> Vec<Cookie> {
    let Some(text) = cookie_file().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    let t = now();
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(9, '\t').collect();
            if f.len() != 9 {
                return None;
            }
            let expires: i64 = f[3].parse().ok()?;
            (expires > t).then(|| Cookie {
                domain: f[0].into(),
                host_only: f[1] == "1",
                path: f[2].into(),
                expires: Some(expires),
                secure: f[4] == "1",
                http_only: f[5] == "1",
                same_site: match f[6] {
                    "strict" => SameSite::Strict,
                    "none" => SameSite::None,
                    _ => SameSite::Lax,
                },
                name: f[7].into(),
                value: f[8].into(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_and_matching() {
        assert_eq!(parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"), Some(1445412480));
        assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT"), Some(784111777));
        assert_eq!(site_of("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("a.b.example.com"), "example.com");

        let c = parse("sid=abc; Path=/; Domain=.example.com; Max-Age=3600; Secure; HttpOnly", "https://www.example.com/login", false).unwrap();
        assert_eq!((c.domain.as_str(), c.host_only, c.http_only, c.secure), ("example.com", false, true, true));
        assert!(c.expires.unwrap() > now());
        // Can't set a cookie for another site, or for a whole public suffix.
        assert!(parse("x=1; Domain=evil.com", "https://www.example.com/", false).is_none());
        assert!(parse("x=1; Domain=co.uk", "https://bbc.co.uk/", false).is_none());
        // Scripts can't create HttpOnly cookies; http pages can't set Secure ones.
        assert!(parse("x=1; HttpOnly", "https://a.com/", true).is_none());
        assert!(parse("x=1; Secure", "http://a.com/", false).is_none());
        assert_eq!(parse("x=1", "https://a.com/docs/page", false).unwrap().path, "/docs");

        // Third-party blocking: same site is fine, other sites are not.
        assert!(allowed("https://cdn.example.com/x.js", "https://www.example.com/"));
        assert!(!allowed("https://tracker.ads.net/p.gif", "https://www.example.com/"));

        let mut j = Jar { cookies: Vec::new(), loaded: true };
        j.cookies.push(c);
        j.cookies.push(parse("theme=dark; Max-Age=60", "https://www.example.com/", false).unwrap());
        j.cookies.push(parse("deep=1; Path=/docs", "https://www.example.com/", false).unwrap());
        assert_eq!(join(&j.matching("https://shop.example.com/", false)), "sid=abc");
        assert_eq!(join(&j.matching("https://www.example.com/docs/a", false)), "deep=1; sid=abc; theme=dark");
        assert_eq!(join(&j.matching("https://www.example.com/", true)), "theme=dark");
        assert_eq!(join(&j.matching("http://www.example.com/", false)), "theme=dark");
    }
}
