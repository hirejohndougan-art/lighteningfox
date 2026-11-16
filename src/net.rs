//! Networking: load http(s):// and file:// URLs, and resolve relative links.

use std::io::Read;

pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) LighteningFox/0.1";

/// Fetch a URL. Returns (final URL after redirects, body text).
pub fn fetch(url: &str) -> Result<(String, String), String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let agent = ureq::AgentBuilder::new()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .try_proxy_from_env(true)
            .build();
        let resp = agent.get(url).call().map_err(|e| e.to_string())?;
        let final_url = resp.get_url().to_string();
        let mut bytes = Vec::new();
        resp.into_reader().take(20_000_000).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok((final_url, String::from_utf8_lossy(&bytes).into_owned()))
    } else {
        let path = url.strip_prefix("file://").unwrap_or(url);
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let abs = std::fs::canonicalize(path).map(|p| p.display().to_string()).unwrap_or(path.to_string());
        Ok((format!("file://{abs}"), String::from_utf8_lossy(&bytes).into_owned()))
    }
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
