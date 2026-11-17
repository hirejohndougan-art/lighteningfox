//! Match patterns (`*://*.example.com/path/*`, `<all_urls>`), as manifests and
//! `webRequest` filters use them.

/// Does `url` match `pattern`?
pub fn matches(pattern: &str, url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    let scheme = u.scheme();
    if pattern == "<all_urls>" {
        return matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file" | "data");
    }
    let Some((pscheme, rest)) = pattern.split_once("://") else { return false };
    let scheme_ok = match pscheme {
        "*" => matches!(scheme, "http" | "https" | "ws" | "wss"),
        s => s == scheme,
    };
    if !scheme_ok {
        return false;
    }
    // file:///path has an empty host.
    let (phost, ppath) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => return false,
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let phost = phost.to_ascii_lowercase();
    // A port in the pattern (Firefox allows it) must match too.
    let (phost, pport) = match phost.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || p.is_empty() => (h.to_string(), Some(p.to_string())),
        _ => (phost, None),
    };
    if let Some(p) = pport {
        if p != "*" && u.port_or_known_default().map(|x| x.to_string()) != Some(p) {
            return false;
        }
    }
    let host_ok = if phost == "*" {
        true
    } else if let Some(base) = phost.strip_prefix("*.") {
        host == base || host.ends_with(&format!(".{base}"))
    } else {
        host == phost
    };
    if !host_ok {
        return false;
    }
    let mut path = u.path().to_string();
    if let Some(q) = u.query() {
        path.push('?');
        path.push_str(q);
    }
    glob(ppath, &path)
}

/// `*` matches any run of characters; everything else literally.
pub fn glob(pat: &str, s: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pat.chars().collect(), s.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(matches("<all_urls>", "https://a.com/x"));
        assert!(matches("<all_urls>", "file:///home/x.html"));
        assert!(!matches("<all_urls>", "about:blank"));
        assert!(matches("*://*/*", "http://a.com/"));
        assert!(!matches("*://*/*", "file:///x"));
        assert!(matches("https://*.example.com/*", "https://example.com/a"));
        assert!(matches("https://*.example.com/*", "https://www.example.com/a?b=1"));
        assert!(!matches("https://*.example.com/*", "https://badexample.com/"));
        assert!(!matches("https://*.example.com/*", "http://example.com/"));
        assert!(matches("*://ads.test/banner/*", "https://ads.test/banner/1.png"));
        assert!(!matches("*://ads.test/banner/*", "https://ads.test/other.png"));
        assert!(matches("file:///*", "file:///home/x/a.html"));
        assert!(matches("http://localhost:8080/*", "http://localhost:8080/a"));
        assert!(!matches("http://localhost:8080/*", "http://localhost:9090/a"));
        assert!(glob("*ad*.js", "/js/ads.js") && !glob("a*c", "abd"));
    }
}
