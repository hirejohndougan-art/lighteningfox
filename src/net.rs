//! Networking: load http(s):// and file:// URLs, and resolve relative links.

use std::io::Read;

pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) LighteningFox/0.1";

/// Fetch a page (a top-level document) as text. Returns (final URL after redirects, body).
pub fn fetch(url: &str) -> Result<(String, String), String> {
    let (final_url, bytes) = load(url, None, None)?;
    Ok((final_url, String::from_utf8_lossy(&bytes).into_owned()))
}

/// POST a form (application/x-www-form-urlencoded) as a page navigation.
pub fn post_form(url: &str, body: &str) -> Result<(String, String), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        // Local files can't take a POST; show them as a GET with the data in the query.
        return fetch(&format!("{}?{body}", url.split('?').next().unwrap_or(url)));
    }
    let (final_url, bytes) = load(url, Some(body), None)?;
    Ok((final_url, String::from_utf8_lossy(&bytes).into_owned()))
}

/// Fetch something a page uses (stylesheet, script, image) as text. `page` is the page's URL,
/// which decides whether cookies go along (only to the page's own site).
pub fn fetch_sub(url: &str, page: &str) -> Result<(String, String), String> {
    let (final_url, bytes) = load(url, None, Some(page))?;
    Ok((final_url, String::from_utf8_lossy(&bytes).into_owned()))
}

/// Like `fetch_sub`, as raw bytes (images).
pub fn fetch_sub_bytes(url: &str, page: &str) -> Result<(String, Vec<u8>), String> {
    load(url, None, Some(page))
}

/// Load a URL: http(s) (with cookies and redirects), file:// or data:.
/// `top` is the page making the request; `None` means the request is the page itself.
fn load(url: &str, post: Option<&str>, top: Option<&str>) -> Result<(String, Vec<u8>), String> {
    if let Some(rest) = url.strip_prefix("data:") {
        return decode_data_url(rest).map(|b| (url.to_string(), b));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        let path = url.strip_prefix("file://").unwrap_or(url);
        // The query and fragment aren't part of the file's name; keep them on the URL.
        let cut = path.find(['?', '#']).unwrap_or(path.len());
        let (path, rest) = path.split_at(cut);
        let path = percent_decode(path);
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        let abs = std::fs::canonicalize(&path).map(|p| p.display().to_string()).unwrap_or(path);
        return Ok((format!("file://{abs}{rest}"), bytes));
    }
    // Redirects are followed here rather than by ureq, so cookies set along the way are kept.
    let mut url = url.to_string();
    let mut post = post.map(String::from);
    for _ in 0..20 {
        let agent = ureq::AgentBuilder::new()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .redirects(0)
            .try_proxy_from_env(use_proxy(&url))
            .build();
        let first_party = top.unwrap_or(&url).to_string();
        let mut req = match post {
            Some(_) => agent.post(&url).set("Content-Type", "application/x-www-form-urlencoded"),
            None => agent.get(&url),
        };
        req = req.set("Accept", if top.is_none() { "text/html,application/xhtml+xml,*/*;q=0.8" } else { "*/*" });
        // Global Privacy Control: "do not sell or share my data" (honored by law in some US states).
        req = req.set("Sec-GPC", "1");
        if let Some(c) = crate::cookies::header_for(&url, &first_party) {
            req = req.set("Cookie", &c);
        }
        let result = match &post {
            Some(body) => req.send_string(body),
            None => req.call(),
        };
        let resp = match result {
            Ok(r) => r,
            // Error pages (404, 500…) are still pages to show.
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(e.to_string()),
        };
        let set: Vec<String> = resp.all("set-cookie").iter().map(|s| s.to_string()).collect();
        crate::cookies::receive(&url, &first_party, &set);
        let status = resp.status();
        if (300..400).contains(&status) && status != 304 {
            if let Some(loc) = resp.header("location") {
                url = resolve(&url, loc.trim());
                // 301/302/303 turn a POST into a GET; 307/308 repeat it.
                if !matches!(status, 307 | 308) {
                    post = None;
                }
                continue;
            }
        }
        let mut bytes = Vec::new();
        resp.into_reader().take(30_000_000).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        return Ok((url, bytes));
    }
    Err("too many redirects".into())
}

/// Skip the system proxy for local addresses and anything listed in NO_PROXY.
fn use_proxy(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host).to_ascii_lowercase();
    if host == "localhost" || host.starts_with("127.") || host == "[::1]" {
        return false;
    }
    let no_proxy = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    !no_proxy.split(',').map(str::trim).filter(|d| !d.is_empty()).any(|d| {
        let d = d.trim_start_matches('.').to_ascii_lowercase();
        d == "*" || host == d || host.ends_with(&format!(".{d}"))
    })
}

/// `data:[<mediatype>][;base64],<data>`
fn decode_data_url(rest: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let (meta, data) = rest.split_once(',').ok_or("malformed data: URL")?;
    if meta.ends_with(";base64") {
        let clean: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        let clean = percent_decode(&clean);
        base64::engine::general_purpose::STANDARD
            .decode(clean.trim_end_matches('='))
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(clean.trim_end_matches('=')))
            .map_err(|e| e.to_string())
    } else {
        Ok(percent_decode(data).into_bytes())
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Remove tracking parameters (utm_*, fbclid, gclid…) from a page URL before loading it.
/// They exist only to tell sites and ad networks where you came from.
pub fn strip_tracking(url: &str) -> String {
    const EXACT: &[&str] = &[
        "fbclid", "gclid", "gclsrc", "dclid", "gbraid", "wbraid", "msclkid", "mc_eid", "mc_cid", "yclid", "twclid", "ttclid", "li_fat_id",
        "igshid", "igsh", "_hsenc", "_hsmi", "mkt_tok", "oly_anon_id", "oly_enc_id", "vero_id", "wickedid", "__s", "_openstat", "srsltid", "si",
    ];
    if std::env::var("LF_KEEP_TRACKING_PARAMS").is_ok_and(|v| v == "1") || !(url.starts_with("http://") || url.starts_with("https://")) {
        return url.to_string();
    }
    let (before_frag, frag) = match url.split_once('#') {
        Some((a, f)) => (a, Some(f)),
        None => (url, None),
    };
    let Some((base, query)) = before_frag.split_once('?') else { return url.to_string() };
    // `si` is only a tracker on YouTube and Spotify share links.
    let host = base.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
    let share_si = host.contains("youtu") || host.contains("spotify");
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let k = kv.split('=').next().unwrap_or("").to_ascii_lowercase();
            !(k.starts_with("utm_") || (EXACT.contains(&k.as_str()) && (k != "si" || share_si)))
        })
        .collect();
    let mut out = base.to_string();
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Turn whatever the user typed in the address bar into a URL.
pub fn normalize_input(input: &str) -> String {
    let s = input.trim();
    if s.contains("://") || s.starts_with("about:") {
        return s.to_string();
    }
    if std::path::Path::new(s).exists() {
        return format!("file://{}", std::fs::canonicalize(s).map(|p| p.display().to_string()).unwrap_or(s.into()));
    }
    if s.contains('.') && !s.contains(' ') {
        return format!("https://{s}");
    }
    let q: String = s
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://html.duckduckgo.com/html/?q={q}")
}

/// Resolve `href` relative to `base`.
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.contains("://") || href.starts_with("about:") || href.starts_with("data:") || href.starts_with("mailto:") {
        return href.to_string();
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("file", base));
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let path = path.split(['?', '#']).next().unwrap_or("/");
    if let Some(r) = href.strip_prefix("//") {
        return format!("{scheme}://{r}");
    }
    if href.starts_with('#') {
        return format!("{}{}", base.split('#').next().unwrap_or(base), href);
    }
    if href.starts_with('?') {
        return format!("{scheme}://{host}{path}{href}");
    }
    let joined = if href.starts_with('/') {
        href.to_string()
    } else {
        let dir = &path[..path.rfind('/').map(|i| i + 1).unwrap_or(0)];
        format!("{dir}{href}")
    };
    // Normalize "." and ".." segments.
    let (p, suffix) = match joined.find(['?', '#']) {
        Some(i) => (joined[..i].to_string(), joined[i..].to_string()),
        None => (joined.clone(), String::new()),
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/').skip(1) {
        match seg {
            "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("{scheme}://{host}/{}{suffix}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use super::strip_tracking;
    #[test]
    fn strips_trackers() {
        assert_eq!(strip_tracking("https://a.com/p?id=7&utm_source=x&utm_medium=y&fbclid=abc#top"), "https://a.com/p?id=7#top");
        assert_eq!(strip_tracking("https://a.com/p?utm_campaign=z"), "https://a.com/p");
        assert_eq!(strip_tracking("https://a.com/p?si=keep"), "https://a.com/p?si=keep");
        assert_eq!(strip_tracking("https://youtu.be/x?si=drop"), "https://youtu.be/x");
    }

    #[test]
    fn resolves() {
        assert_eq!(resolve("https://a.com/x/y.html", "z.html"), "https://a.com/x/z.html");
        assert_eq!(resolve("https://a.com/x/y.html", "/z"), "https://a.com/z");
        assert_eq!(resolve("https://a.com/x/y.html", "../z"), "https://a.com/z");
        assert_eq!(resolve("https://a.com/x/", "//b.com/q"), "https://b.com/q");
        assert_eq!(resolve("https://a.com", "q?x=1"), "https://a.com/q?x=1");
        assert_eq!(resolve("file:///home/j/a.html", "b.html"), "file:///home/j/b.html");
    }
}
