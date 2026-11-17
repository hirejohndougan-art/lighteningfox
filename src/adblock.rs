//! Ad and tracker blocking with Adblock Plus / uBlock Origin filter lists (EasyList,
//! EasyPrivacy and the like).
//!
//! Every subresource request (scripts, stylesheets, images, fonts, frames, fetch/XHR) is
//! checked before it goes out; a match is refused locally, so ad scripts never download, run
//! or take memory. Lists:
//! - a small built-in list (assets/filters/builtin.txt), always on;
//! - EasyList and EasyPrivacy, downloaded into `<profile>/filters/` and refreshed every few days;
//! - any other `.txt` list the user drops into that folder.
//!
//! Supported network syntax: `||host^`, `|` anchors, `*`, `^`, `@@` exceptions, hosts-file
//! lines, and the options `third-party`/`3p`/`first-party`/`1p`, `domain=`, `important`,
//! `match-case`, and the resource types (`script`, `image`, `stylesheet`, `subdocument`,
//! `xmlhttprequest`, `font`, `media`, `other`, with `~` to exclude). Cosmetic (`##`) rules and
//! options that rewrite requests (`redirect`, `csp`, `removeparam`…) are skipped.
//!
//! Matching is indexed by token (a whole run of letters and digits in the rule), so each
//! request only checks the handful of rules that share a token with its URL.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime};

/// What a request is for (filter options can match on it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResType {
    Script,
    Stylesheet,
    Image,
    Font,
    Media,
    /// An iframe's document.
    Subdocument,
    /// fetch() and XMLHttpRequest.
    Xhr,
    Other,
}

impl ResType {
    fn bit(self) -> u16 {
        1 << (self as u16)
    }

    /// A best guess from the URL's file extension, for requests whose caller didn't say.
    pub fn guess(url: &str) -> ResType {
        let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
        let ext = path.rsplit('/').next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e.to_string()).unwrap_or_default();
        match ext.as_str() {
            "js" | "mjs" => ResType::Script,
            "css" => ResType::Stylesheet,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg" | "ico" | "bmp" => ResType::Image,
            "woff" | "woff2" | "ttf" | "otf" | "eot" => ResType::Font,
            "mp4" | "webm" | "mp3" | "ogg" | "m4a" | "wav" | "m3u8" => ResType::Media,
            _ => ResType::Other,
        }
    }
}

fn type_bit(name: &str) -> Option<u16> {
    Some(match name {
        "script" => ResType::Script.bit(),
        "stylesheet" | "css" => ResType::Stylesheet.bit(),
        "image" => ResType::Image.bit(),
        "font" => ResType::Font.bit(),
        "media" => ResType::Media.bit(),
        "subdocument" | "frame" => ResType::Subdocument.bit(),
        "xmlhttprequest" | "xhr" => ResType::Xhr.bit(),
        "other" | "object" | "ping" | "websocket" => ResType::Other.bit(),
        _ => return None,
    })
}

// ---------------------------------------------------------------- one rule

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Lit(String),
    /// `*`: anything.
    Star,
    /// `^`: a separator (anything but a letter, digit or `_ - . %`), or the end of the URL.
    Sep,
}

#[derive(Clone, Debug)]
struct Rule {
    parts: Vec<Part>,
    /// `||`: the pattern starts at the beginning of the host or of one of its subdomains.
    host_anchor: bool,
    /// `|` at the start / end.
    start_anchor: bool,
    end_anchor: bool,
    exception: bool,
    important: bool,
    match_case: bool,
    /// Resource types it applies to (0 = all).
    types: u16,
    third_party: Option<bool>,
    domains: Vec<String>,
    not_domains: Vec<String>,
}

/// Parse one line of a filter list. None for comments, cosmetic rules and anything we can't
/// apply faithfully.
fn parse_rule(line: &str) -> Option<Rule> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
        return None;
    }
    // Cosmetic and scriptlet rules: ##, #@#, #?#, #$#, #%#.
    if line.contains("##") || line.contains("#@#") || line.contains("#?#") || line.contains("#$#") || line.contains("#%#") {
        return None;
    }
    // Hosts-file lines: "0.0.0.0 ads.example.com" or "127.0.0.1 …".
    if let Some(rest) = line.strip_prefix("0.0.0.0 ").or_else(|| line.strip_prefix("127.0.0.1 ")) {
        let host = rest.split_whitespace().next()?;
        if host == "localhost" || host == "0.0.0.0" || !host.contains('.') {
            return None;
        }
        return parse_rule(&format!("||{host}^"));
    }
    let (exception, body) = match line.strip_prefix("@@") {
        Some(b) => (true, b),
        None => (false, line),
    };
    // Regular-expression rules (/…/) are rare in the big lists' network part; skip them.
    if body.starts_with('/') && body.len() > 2 && body.ends_with('/') {
        return None;
    }
    let (pattern, options) = match body.rfind('$') {
        // A '$' inside the pattern is possible but rare; options never contain '/'.
        Some(i) if !body[i + 1..].contains('/') || body[i + 1..].starts_with("domain=") => (&body[..i], Some(&body[i + 1..])),
        _ => (body, None),
    };
    let mut rule = Rule {
        parts: Vec::new(),
        host_anchor: false,
        start_anchor: false,
        end_anchor: false,
        exception,
        important: false,
        match_case: false,
        types: 0,
        third_party: None,
        domains: Vec::new(),
        not_domains: Vec::new(),
    };
    let mut not_types: u16 = 0;
    if let Some(opts) = options {
        for opt in opts.split(',') {
            let opt = opt.trim();
            let (neg, name) = match opt.strip_prefix('~') {
                Some(n) => (true, n),
                None => (false, opt),
            };
            match name {
                "third-party" | "3p" => rule.third_party = Some(!neg),
                "first-party" | "1p" => rule.third_party = Some(neg),
                "important" => rule.important = true,
                "match-case" => rule.match_case = true,
                "all" => {}
                n if n.starts_with("domain=") || n.starts_with("from=") => {
                    for d in n.split_once('=').map(|x| x.1).unwrap_or("").split('|') {
                        match d.strip_prefix('~') {
                            Some(d) if !d.is_empty() => rule.not_domains.push(d.to_ascii_lowercase()),
                            None if !d.is_empty() => rule.domains.push(d.to_ascii_lowercase()),
                            _ => {}
                        }
                    }
                }
                n => match type_bit(n) {
                    Some(b) if neg => not_types |= b,
                    Some(b) => rule.types |= b,
                    // Options that change requests rather than block them, or that apply to
                    // things we don't load (popup, document, csp, redirect, removeparam…):
                    // applying the rule without them would block too much.
                    None => return None,
                },
            }
        }
    }
    if not_types != 0 {
        let all: u16 = (1 << 8) - 1;
        rule.types = (if rule.types == 0 { all } else { rule.types }) & !not_types;
        if rule.types == 0 {
            return None;
        }
    }
    let mut p = pattern;
    if let Some(r) = p.strip_prefix("||") {
        rule.host_anchor = true;
        p = r;
    } else if let Some(r) = p.strip_prefix('|') {
        rule.start_anchor = true;
        p = r;
    }
    if let Some(r) = p.strip_suffix('|') {
        rule.end_anchor = true;
        p = r;
    }
    let p = if rule.match_case { p.to_string() } else { p.to_ascii_lowercase() };
    let mut lit = String::new();
    for c in p.chars() {
        match c {
            '*' | '^' => {
                if !lit.is_empty() {
                    rule.parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                let part = if c == '*' { Part::Star } else { Part::Sep };
                // "**" is the same as "*".
                if !(part == Part::Star && rule.parts.last() == Some(&Part::Star)) {
                    rule.parts.push(part);
                }
            }
            _ => lit.push(c),
        }
    }
    if !lit.is_empty() {
        rule.parts.push(Part::Lit(lit));
    }
    // A rule that matches everything ("*", "", "^") would block the web.
    if !rule.parts.iter().any(|x| matches!(x, Part::Lit(l) if l.len() >= 2)) && rule.domains.is_empty() {
        return None;
    }
    Some(rule)
}

fn is_sep(c: u8) -> bool {
    !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'%'))
}

/// Does `parts` match `url` starting exactly at `at`? (`end_anchor`: must end at the URL's end.)
fn match_at(parts: &[Part], url: &[u8], at: usize, end_anchor: bool) -> bool {
    match parts.split_first() {
        None => !end_anchor || at == url.len(),
        Some((Part::Lit(l), rest)) => {
            let l = l.as_bytes();
            url.len() >= at + l.len() && &url[at..at + l.len()] == l && match_at(rest, url, at + l.len(), end_anchor)
        }
        Some((Part::Sep, rest)) => {
            // `^` also matches the end of the URL.
            if at == url.len() {
                return match_at(rest, url, at, end_anchor);
            }
            is_sep(url[at]) && match_at(rest, url, at + 1, end_anchor)
        }
        Some((Part::Star, rest)) => {
            if rest.is_empty() {
                return true;
            }
            // Try each later position; when the next part is a literal, jump between its
            // occurrences instead of every byte.
            if let Some(Part::Lit(l)) = rest.first() {
                let l = l.as_bytes();
                let mut i = at;
                while i + l.len() <= url.len() {
                    match find(&url[i..], l) {
                        Some(off) => {
                            if match_at(rest, url, i + off, end_anchor) {
                                return true;
                            }
                            i += off + 1;
                        }
                        None => return false,
                    }
                }
                false
            } else {
                (at..=url.len()).any(|i| match_at(rest, url, i, end_anchor))
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A request being checked.
struct Req<'a> {
    /// Lower-case URL, and the URL as given (for match-case rules).
    url: &'a str,
    url_case: &'a str,
    host: &'a str,
    /// Byte offset of the host in the URL.
    host_start: usize,
    page_host: &'a str,
    third_party: bool,
    ty: ResType,
}

impl Rule {
    fn pattern_matches(&self, r: &Req) -> bool {
        let url = if self.match_case { r.url_case.as_bytes() } else { r.url.as_bytes() };
        if self.host_anchor {
            // At the host's start or right after any '.' inside the host.
            let host_end = r.host_start + r.host.len();
            let mut starts = vec![r.host_start];
            starts.extend(r.host.bytes().enumerate().filter(|(_, b)| *b == b'.').map(|(i, _)| r.host_start + i + 1));
            return starts.into_iter().filter(|&s| s < host_end).any(|s| match_at(&self.parts, url, s, self.end_anchor));
        }
        if self.start_anchor {
            return match_at(&self.parts, url, 0, self.end_anchor);
        }
        // Unanchored: anywhere. Jump to occurrences of the first literal when it leads.
        match self.parts.first() {
            Some(Part::Lit(l)) => {
                let l = l.as_bytes();
                let mut i = 0;
                while let Some(off) = find(&url[i..], l) {
                    if match_at(&self.parts, url, i + off, self.end_anchor) {
                        return true;
                    }
                    i += off + 1;
                    if i >= url.len() {
                        break;
                    }
                }
                false
            }
            _ => (0..=url.len()).any(|i| match_at(&self.parts, url, i, self.end_anchor)),
        }
    }

    fn options_match(&self, r: &Req) -> bool {
        if self.types != 0 && self.types & r.ty.bit() == 0 {
            return false;
        }
        if let Some(tp) = self.third_party {
            if tp != r.third_party {
                return false;
            }
        }
        let on = |d: &String| r.page_host == d || r.page_host.ends_with(&format!(".{d}"));
        if !self.domains.is_empty() && !self.domains.iter().any(on) {
            return false;
        }
        if self.not_domains.iter().any(on) {
            return false;
        }
        true
    }

    fn matches(&self, r: &Req) -> bool {
        self.options_match(r) && self.pattern_matches(r)
    }

    /// The token this rule is filed under: the longest whole run of letters and digits in a
    /// literal (whole = bounded by a separator character, an anchor or `^`, never by `*`).
    fn token(&self) -> Option<String> {
        let mut best: Option<String> = None;
        for (i, p) in self.parts.iter().enumerate() {
            let Part::Lit(l) = p else { continue };
            let l = l.to_ascii_lowercase();
            let bytes = l.as_bytes();
            let before_ok = i > 0 && self.parts[i - 1] == Part::Sep || i == 0 && (self.host_anchor || self.start_anchor);
            let after_ok = i + 1 < self.parts.len() && self.parts[i + 1] == Part::Sep || i + 1 == self.parts.len() && self.end_anchor;
            let mut start = 0;
            while start < bytes.len() {
                if !bytes[start].is_ascii_alphanumeric() {
                    start += 1;
                    continue;
                }
                let mut end = start;
                while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                    end += 1;
                }
                let whole = (start > 0 || before_ok) && (end < bytes.len() || after_ok);
                if whole && end - start >= 2 && best.as_ref().is_none_or(|b| b.len() < end - start) {
                    best = Some(l[start..end].to_string());
                }
                start = end;
            }
        }
        best
    }
}

// ---------------------------------------------------------------- the engine

#[derive(Default)]
pub struct Engine {
    by_token: HashMap<String, Vec<Rule>>,
    untokened: Vec<Rule>,
    pub rules: usize,
}

impl Engine {
    pub fn add_list(&mut self, text: &str) -> usize {
        let mut n = 0;
        for line in text.lines() {
            if let Some(rule) = parse_rule(line) {
                match rule.token() {
                    Some(t) => self.by_token.entry(t).or_default().push(rule),
                    None => self.untokened.push(rule),
                }
                n += 1;
            }
        }
        self.rules += n;
        n
    }

    /// Should a request for `url` of type `ty`, made by the page at `page`, be blocked?
    pub fn check(&self, url: &str, page: &str, ty: ResType) -> bool {
        let Ok(u) = url::Url::parse(url) else { return false };
        if !matches!(u.scheme(), "http" | "https" | "ws" | "wss") {
            return false;
        }
        let Some(host) = u.host_str() else { return false };
        let lower = url.to_ascii_lowercase();
        let host_l = host.to_ascii_lowercase();
        let Some(host_start) = lower.find(&host_l) else { return false };
        let page_host = url::Url::parse(page).ok().and_then(|p| p.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
        let req = Req {
            url: &lower,
            url_case: url,
            host: &lower[host_start..host_start + host_l.len()],
            host_start,
            page_host: &page_host,
            third_party: page_host.is_empty() || site_of(&host_l) != site_of(&page_host),
            ty,
        };
        let mut seen: HashSet<&str> = HashSet::new();
        let mut candidates: Vec<&Rule> = self.untokened.iter().collect();
        for tok in lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| t.len() >= 2) {
            if seen.insert(tok) {
                if let Some(rs) = self.by_token.get(tok) {
                    candidates.extend(rs);
                }
            }
        }
        let mut blocked = false;
        for r in candidates.iter().filter(|r| !r.exception) {
            if r.matches(&req) {
                if r.important {
                    return true;
                }
                blocked = true;
            }
        }
        if !blocked {
            return false;
        }
        !candidates.iter().any(|r| r.exception && r.matches(&req))
    }
}

/// The site a host belongs to (its registrable domain, roughly): the last two labels, or three
/// under common two-part endings like co.uk. Good enough to tell first from third party.
pub fn site_of(host: &str) -> String {
    if host.parse::<std::net::IpAddr>().is_ok() || host.trim_matches(['[', ']']).parse::<std::net::Ipv6Addr>().is_ok() {
        return host.to_string();
    }
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    if labels.len() <= 2 {
        return labels.join(".");
    }
    let two = format!("{}.{}", labels[labels.len() - 2], labels[labels.len() - 1]);
    const SECOND_LEVEL: &[&str] = &[
        "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "ltd.uk", "plc.uk", "com.au", "net.au", "org.au", "edu.au", "gov.au", "co.nz", "org.nz",
        "co.jp", "ne.jp", "or.jp", "ac.jp", "co.kr", "or.kr", "com.br", "net.br", "org.br", "com.cn", "net.cn", "org.cn", "com.tw", "com.hk",
        "co.in", "net.in", "org.in", "co.za", "com.mx", "com.ar", "com.tr", "com.sg", "com.my", "co.id", "co.th", "com.ph", "com.vn", "co.il",
        "github.io", "gitlab.io", "blogspot.com", "herokuapp.com", "netlify.app", "vercel.app", "pages.dev", "workers.dev", "appspot.com",
        "cloudfront.net", "azurewebsites.net", "s3.amazonaws.com",
    ];
    if SECOND_LEVEL.contains(&two.as_str()) {
        labels[labels.len() - 3..].join(".")
    } else {
        two
    }
}

// ---------------------------------------------------------------- the browser's blocker

/// Lists downloaded into `<profile>/filters/` and kept fresh.
pub const LISTS: &[(&str, &str, &str)] = &[
    ("easylist.txt", "EasyList (ads)", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy.txt", "EasyPrivacy (trackers)", "https://easylist.to/easylist/easyprivacy.txt"),
];
const REFRESH: Duration = Duration::from_secs(4 * 24 * 3600);
const BUILTIN: &str = include_str!("../assets/filters/builtin.txt");

/// User choices, in `<profile>/adblock.tsv`: `enabled\t0|1` and `allow\t<site>` lines.
#[derive(Clone, Debug)]
struct Prefs {
    enabled: bool,
    allowed: Vec<String>,
}

fn profile_file(name: &str) -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join(name))
}

fn filters_dir() -> Option<std::path::PathBuf> {
    profile_file("filters")
}

fn prefs() -> &'static RwLock<Prefs> {
    static P: OnceLock<RwLock<Prefs>> = OnceLock::new();
    P.get_or_init(|| {
        let mut p = Prefs { enabled: std::env::var("IRONSWIFT_ADBLOCK").map_or(true, |v| v != "0"), allowed: Vec::new() };
        if let Some(text) = profile_file("adblock.tsv").and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                match line.split_once('\t') {
                    Some(("enabled", v)) => p.enabled = v.trim() == "1",
                    Some(("allow", s)) if !s.trim().is_empty() => p.allowed.push(s.trim().to_ascii_lowercase()),
                    _ => {}
                }
            }
        }
        RwLock::new(p)
    })
}

fn save_prefs(p: &Prefs) {
    if std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1") {
        return;
    }
    let mut text = format!("enabled\t{}\n", if p.enabled { 1 } else { 0 });
    for s in &p.allowed {
        text.push_str(&format!("allow\t{s}\n"));
    }
    if let Some(f) = profile_file("adblock.tsv") {
        let _ = std::fs::write(f, text);
    }
}

fn engine() -> &'static RwLock<Engine> {
    static E: OnceLock<RwLock<Engine>> = OnceLock::new();
    E.get_or_init(|| {
        let e = RwLock::new(build());
        start_updates();
        e
    })
}

/// The built-in list plus every list in the filters folder.
fn build() -> Engine {
    let started = std::time::Instant::now();
    let mut e = Engine::default();
    e.add_list(BUILTIN);
    if let Some(dir) = filters_dir() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut files: Vec<_> = rd.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
            files.sort();
            for f in files {
                if let Ok(text) = std::fs::read_to_string(&f) {
                    e.add_list(&text);
                }
            }
        }
    }
    eprintln!("[adblock] {} rules loaded in {} ms", e.rules, started.elapsed().as_millis());
    e
}

/// Download missing or stale lists in the background, then reload.
fn start_updates() {
    if std::env::var("IRONSWIFT_NO_LIST_UPDATES").is_ok() {
        return;
    }
    let _ = std::thread::Builder::new().name("filter-lists".into()).spawn(|| {
        // Let the first page load get the network to itself.
        std::thread::sleep(Duration::from_secs(5));
        update_lists(false);
    });
}

/// Fetch each list that's missing or older than the refresh interval (all of them with
/// `force`). Returns a line per list for the settings page.
pub fn update_lists(force: bool) -> Vec<String> {
    let Some(dir) = filters_dir() else { return vec!["No profile folder".into()] };
    let _ = std::fs::create_dir_all(&dir);
    let mut report = Vec::new();
    let mut changed = false;
    for (file, name, url) in LISTS {
        let path = dir.join(file);
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
        if !force && age.is_some_and(|a| a < REFRESH) {
            continue;
        }
        let req = crate::net::Request { method: "GET", url, headers: &[], body: None, top: None, cookies: false, extra: &[] };
        match crate::net::http(&req) {
            Ok(r) if r.status == 200 && r.body.starts_with(b"[Adblock") || r.status == 200 && r.body.len() > 10_000 => {
                if std::fs::write(&path, &r.body).is_ok() {
                    changed = true;
                    report.push(format!("{name}: updated ({} KB)", r.body.len() / 1024));
                }
            }
            Ok(r) => report.push(format!("{name}: server answered {}", r.status)),
            Err(e) => report.push(format!("{name}: couldn't download ({e})")),
        }
    }
    if changed {
        reload();
    }
    report
}

/// Rebuild the rules from the lists on disk.
pub fn reload() {
    let fresh = build();
    if let Ok(mut e) = engine().write() {
        *e = fresh;
    }
}

/// Blocked request counts per page site this session, for about:blocking.
fn counts() -> &'static Mutex<(u64, HashMap<String, u64>)> {
    static C: OnceLock<Mutex<(u64, HashMap<String, u64>)>> = OnceLock::new();
    C.get_or_init(|| Mutex::new((0, HashMap::new())))
}

/// The browser's check before a page's subresource goes out. Ad blocking off, an allowed site,
/// or a URL the lists don't match: false.
pub fn should_block(url: &str, page: &str, ty: ResType) -> bool {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    let page_site = url::Url::parse(page).ok().and_then(|u| u.host_str().map(site_of)).unwrap_or_default();
    {
        let p = prefs().read().unwrap_or_else(|e| e.into_inner());
        if !p.enabled || p.allowed.iter().any(|a| *a == page_site) {
            return false;
        }
    }
    let blocked = engine().read().map(|e| e.check(url, page, ty)).unwrap_or(false);
    if blocked {
        eprintln!("[adblock] blocked {url}");
        if let Ok(mut c) = counts().lock() {
            c.0 += 1;
            *c.1.entry(page_site).or_default() += 1;
        }
    }
    blocked
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// about:blocking — on/off, lists, per-site exceptions and this session's counts.
/// Actions come in the query: `?on`, `?off`, `?update`, `?allow=site`, `?unallow=site`.
pub fn page(query: &str) -> String {
    let q = query.trim_start_matches('?');
    let param = |name: &str| q.split('&').find_map(|p| p.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| crate::net::percent_decode_str(&v.replace('+', " "))));
    let mut note = String::new();
    {
        let mut p = prefs().write().unwrap_or_else(|e| e.into_inner());
        let mut save = true;
        if q == "on" {
            p.enabled = true;
        } else if q == "off" {
            p.enabled = false;
        } else if let Some(s) = param("allow") {
            let s = site_of(s.trim().trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("").to_ascii_lowercase().as_str());
            if !s.is_empty() && !p.allowed.contains(&s) {
                p.allowed.push(s);
            }
        } else if let Some(s) = param("unallow") {
            p.allowed.retain(|a| *a != s);
        } else {
            save = false;
        }
        if save {
            save_prefs(&p);
        }
    }
    if q == "update" {
        note = update_lists(true).join("<br>");
        if note.is_empty() {
            note = "Nothing to update.".into();
        }
    }
    let p = prefs().read().unwrap_or_else(|e| e.into_inner()).clone();
    let rules = engine().read().map(|e| e.rules).unwrap_or(0);
    let dir = filters_dir();
    let mut lists = String::from("<tr><td>Built-in starter list</td><td>always on</td></tr>");
    for (file, name, _) in LISTS {
        let state = dir
            .as_ref()
            .and_then(|d| std::fs::metadata(d.join(file)).ok())
            .map(|m| {
                let days = m.modified().ok().and_then(|t| SystemTime::now().duration_since(t).ok()).map_or(0, |a| a.as_secs() / 86400);
                format!("{} KB, updated {}", m.len() / 1024, if days == 0 { "today".to_string() } else { format!("{days} day(s) ago") })
            })
            .unwrap_or_else(|| "not downloaded yet".into());
        lists.push_str(&format!("<tr><td>{}</td><td>{}</td></tr>", esc(name), esc(&state)));
    }
    let (total, mut per_site) = counts().lock().map(|c| (c.0, c.1.iter().map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>())).unwrap_or_default();
    per_site.sort_by(|a, b| b.1.cmp(&a.1));
    let sites: String = per_site.iter().take(20).map(|(s, n)| format!("<tr><td>{}</td><td>{n}</td></tr>", esc(s))).collect();
    let allowed: String = p
        .allowed
        .iter()
        .map(|s| format!("<li>{} <a href=\"about:blocking?unallow={}\">remove</a></li>", esc(s), esc(s)))
        .collect();
    format!(
        "<!DOCTYPE html><html><head><title>Ad and tracker blocking</title><style>\
         body{{font-family:sans-serif;margin:32px;max-width:760px;color:#1f2937}}h1{{font-size:24px}}h2{{font-size:17px;margin-top:28px}}\
         table{{border-collapse:collapse}}td{{padding:4px 14px 4px 0;border-bottom:1px solid #e5e7eb}}.state{{font-size:18px;margin:8px 0}}\
         .on{{color:#15803d}}.off{{color:#b91c1c}}.note{{background:#f1f5f9;padding:8px 12px}}a.btn{{display:inline-block;padding:6px 12px;border:1px solid #94a3b8;border-radius:6px;text-decoration:none;color:#1f2937;margin-right:8px}}\
         </style></head><body><h1>Ad and tracker blocking</h1>\
         <p class=\"state\">Blocking is <b class=\"{cls}\">{state}</b>. {rules} rules loaded; {total} requests blocked this session.</p>\
         <p><a class=\"btn\" href=\"about:blocking?{toggle}\">Turn {toggle}</a><a class=\"btn\" href=\"about:blocking?update\">Update lists now</a></p>{note}\
         <h2>Filter lists</h2><table>{lists}</table>\
         <p>Add other Adblock Plus–style lists by saving them as <code>.txt</code> files in <code>{dir}</code>; they load at the next start or update.</p>\
         <h2>Sites where ads are allowed</h2><ul>{allowed}</ul>\
         <form action=\"about:blocking\" method=\"get\"><input type=\"text\" name=\"allow\" placeholder=\"example.com\"> <button type=\"submit\">Allow ads on this site</button></form>\
         <h2>Blocked this session, by site</h2><table>{sites}</table></body></html>",
        cls = if p.enabled { "on" } else { "off" },
        state = if p.enabled { "on" } else { "off" },
        toggle = if p.enabled { "off" } else { "on" },
        note = if note.is_empty() { String::new() } else { format!("<p class=\"note\">{note}</p>") },
        dir = esc(&dir.map(|d| d.display().to_string()).unwrap_or_default()),
        allowed = if allowed.is_empty() { "<li>None</li>".to_string() } else { allowed },
        sites = if sites.is_empty() { "<tr><td>Nothing blocked yet.</td></tr>".to_string() } else { sites },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_of(list: &str) -> Engine {
        let mut e = Engine::default();
        e.add_list(list);
        e
    }

    #[test]
    fn host_anchor_matches_domain_and_subdomains_only() {
        let e = engine_of("||ads.example.com^");
        assert!(e.check("https://ads.example.com/x.js", "https://news.org/", ResType::Script));
        assert!(e.check("https://cdn.ads.example.com/x.js", "https://news.org/", ResType::Script));
        assert!(e.check("https://ads.example.com", "https://news.org/", ResType::Script));
        assert!(!e.check("https://badads.example.com/x.js", "https://news.org/", ResType::Script));
        assert!(!e.check("https://ads.example.com.evil.net/", "https://news.org/", ResType::Script));
        assert!(!e.check("https://example.com/ads.example.com/", "https://news.org/", ResType::Script));
    }

    #[test]
    fn wildcards_separators_and_anchors() {
        let e = engine_of("/banner/*/ad_\n|https://track.\nswf|\n&adtype=^");
        assert!(e.check("https://x.com/banner/300x250/ad_1.png", "https://x.com/", ResType::Image));
        assert!(!e.check("https://x.com/banner/ad_1.png", "https://x.com/", ResType::Image));
        assert!(e.check("https://track.foo.com/p", "https://x.com/", ResType::Other));
        assert!(!e.check("https://x.com/?u=https://track.foo.com", "https://x.com/", ResType::Other));
        assert!(e.check("https://x.com/movie.swf", "https://x.com/", ResType::Other));
        assert!(!e.check("https://x.com/movie.swf?x", "https://x.com/", ResType::Other));
        assert!(e.check("https://x.com/a?b=1&adtype=", "https://x.com/", ResType::Other));
    }

    #[test]
    fn exceptions_and_important() {
        let e = engine_of("||cdn.net/ads/\n@@||cdn.net/ads/ok.js\n||t.net^$important\n@@||t.net^");
        assert!(e.check("https://cdn.net/ads/bad.js", "https://a.com/", ResType::Script));
        assert!(!e.check("https://cdn.net/ads/ok.js", "https://a.com/", ResType::Script));
        assert!(e.check("https://t.net/x", "https://a.com/", ResType::Script));
    }

    #[test]
    fn options() {
        let e = engine_of("||widgets.com^$third-party\n||img.com^$image\n||nojs.com^$~script\n||only.com^$domain=a.com|~b.a.com");
        assert!(e.check("https://widgets.com/w.js", "https://news.org/", ResType::Script));
        assert!(!e.check("https://widgets.com/w.js", "https://www.widgets.com/", ResType::Script));
        assert!(e.check("https://img.com/p.png", "https://x.org/", ResType::Image));
        assert!(!e.check("https://img.com/p.js", "https://x.org/", ResType::Script));
        assert!(!e.check("https://nojs.com/p.js", "https://x.org/", ResType::Script));
        assert!(e.check("https://nojs.com/p.png", "https://x.org/", ResType::Image));
        assert!(e.check("https://only.com/x", "https://www.a.com/", ResType::Other));
        assert!(!e.check("https://only.com/x", "https://b.a.com/", ResType::Other));
        assert!(!e.check("https://only.com/x", "https://z.com/", ResType::Other));
    }

    #[test]
    fn skips_what_it_cannot_apply() {
        let e = engine_of("! comment\n[Adblock Plus 2.0]\nexample.com##.ad\n||x.com^$redirect=noop.js\n||y.com^$popup\n*\n^\n0.0.0.0 tracker.io\n0.0.0.0 localhost");
        assert_eq!(e.rules, 1);
        assert!(e.check("https://tracker.io/a", "https://z.com/", ResType::Other));
    }

    #[test]
    fn third_party_uses_sites() {
        assert_eq!(site_of("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("a.b.example.com"), "example.com");
        assert_eq!(site_of("user.github.io"), "user.github.io");
        assert_eq!(site_of("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn builtin_list_blocks_the_usual_suspects() {
        let e = engine_of(BUILTIN);
        assert!(e.rules > 100);
        let page = "https://news.example/";
        assert!(e.check("https://www.googletagmanager.com/gtm.js?id=GTM-1", page, ResType::Script));
        assert!(e.check("https://securepubads.g.doubleclick.net/tag/js/gpt.js", page, ResType::Script));
        assert!(e.check("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", page, ResType::Script));
        assert!(e.check("https://connect.facebook.net/en_US/fbevents.js", page, ResType::Script));
        assert!(e.check("https://www.facebook.com/tr?id=1&ev=PageView", page, ResType::Image));
        assert!(!e.check("https://www.facebook.com/somepage", page, ResType::Other));
        assert!(!e.check("https://www.google.com/recaptcha/api.js", page, ResType::Script));
        assert!(!e.check("https://cdn.jsdelivr.net/npm/jquery.js", page, ResType::Script));
        assert!(!e.check("https://fonts.googleapis.com/css2?family=Inter", page, ResType::Stylesheet));
    }

    #[test]
    fn fast_on_a_big_list() {
        // 60k synthetic host rules, as in EasyList + EasyPrivacy: a check stays well under a ms.
        let list: String = (0..60_000).map(|i| format!("||host{i}.adnet{}.com^\n", i % 97)).collect();
        let e = engine_of(&list);
        let t = std::time::Instant::now();
        for i in 0..2_000 {
            e.check(&format!("https://cdn{i}.example.com/assets/app.{i}.js"), "https://example.com/", ResType::Script);
        }
        assert!(e.check("https://host123.adnet26.com/x", "https://example.com/", ResType::Script));
        assert!(t.elapsed().as_millis() < 2_000, "{:?}", t.elapsed());
    }
}
