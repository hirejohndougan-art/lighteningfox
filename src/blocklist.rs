//! Content blocking with the filter lists other blockers use, and a warning page for sites
//! known to spread malware or phish for passwords.
//!
//! - **Ads and trackers**: rules in Adblock Plus / uBlock Origin syntax (EasyList,
//!   EasyPrivacy). Network rules (`||ads.example^$script,third-party`, `@@` exceptions) stop
//!   requests before they leave the computer; element-hiding rules (`##.ad-banner`,
//!   `example.com##.promo`) hide what's left of the ad in the page. A short built-in list works
//!   from the first start; the full lists are downloaded into `<profile>/filters/` and kept
//!   up to date in the background.
//! - **Dangerous sites**: host lists (URLhaus malware hosts, Phishing Army) checked before a
//!   page is opened. A match shows a warning page instead; you can still go on.
//! - Your own rules go in `<profile>/filters/custom.txt`, in the same syntax.

use crate::security::Dest;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

/// Request types, as filter options name them.
const T_SCRIPT: u16 = 1;
const T_IMAGE: u16 = 2;
const T_STYLE: u16 = 4;
const T_XHR: u16 = 8;
const T_FRAME: u16 = 16;
const T_FONT: u16 = 32;
const T_MEDIA: u16 = 64;
const T_OBJECT: u16 = 128;
const T_OTHER: u16 = 256;
const T_ALL: u16 = 511;

fn type_bit(dest: Dest) -> u16 {
    match dest {
        Dest::Script => T_SCRIPT,
        Dest::Image => T_IMAGE,
        Dest::Style => T_STYLE,
        Dest::Connect => T_XHR,
        Dest::Frame => T_FRAME,
        Dest::Font => T_FONT,
        Dest::Media => T_MEDIA,
        Dest::Object => T_OBJECT,
        Dest::Other => T_OTHER,
    }
}

#[derive(Debug, Clone, Default)]
struct Opts {
    /// Request types the rule applies to (all when no type is named).
    types: u16,
    /// Some(true) = only third-party requests, Some(false) = only first-party.
    third_party: Option<bool>,
    /// `domain=` — pages the rule applies on / doesn't apply on.
    on: Vec<String>,
    not_on: Vec<String>,
    important: bool,
    match_case: bool,
}

#[derive(Debug, Clone)]
struct Rule {
    /// For `||host…` rules: the host, and the rest of the pattern matched right after it.
    host: Option<String>,
    /// Glob with `*` (anything) and `^` (a separator or the end).
    pattern: String,
    anchor_start: bool,
    anchor_end: bool,
    opts: Opts,
    exception: bool,
    /// The rule as written, for the log.
    text: String,
}

#[derive(Default)]
pub struct Engine {
    rules: Vec<Rule>,
    /// `||host` rules by host.
    by_host: HashMap<String, Vec<usize>>,
    /// Other rules by a word that must appear in the URL.
    by_token: HashMap<String, Vec<usize>>,
    /// Rules with no usable word (checked for every request).
    untokened: Vec<usize>,
    /// Sites with `@@||site^$document` (or `$elemhide`): nothing is blocked or hidden there.
    page_exceptions: Vec<Rule>,
    /// Element hiding: generic selectors keyed by their leading class (`.x`) or id (`#x`).
    hide_by_class: HashMap<String, Vec<String>>,
    hide_by_id: HashMap<String, Vec<String>>,
    /// Element hiding for particular sites: domain -> selectors.
    hide_on: HashMap<String, Vec<String>>,
    /// `#@#` exceptions: (domain or "" for everywhere, selector).
    unhide: HashSet<(String, String)>,
    /// Hosts that spread malware or phish (lower-case).
    dangerous: HashSet<String>,
    pub rule_count: usize,
    pub dangerous_count: usize,
}

/// A short list that works before the full lists have been downloaded: the largest ad and
/// tracking networks (by how many sites embed them).
const BUILT_IN: &str = r#"
! Ironswift built-in rules (the full EasyList / EasyPrivacy lists replace these once downloaded)
||doubleclick.net^
||googlesyndication.com^
||googleadservices.com^
||google-analytics.com^
||googletagmanager.com^$third-party
||googletagservices.com^
||adservice.google.com^
||analytics.google.com^
||connect.facebook.net^$third-party
||facebook.com/tr^
||scorecardresearch.com^
||quantserve.com^
||adnxs.com^
||criteo.com^
||criteo.net^
||taboola.com^
||outbrain.com^
||amazon-adsystem.com^
||adsrvr.org^
||rubiconproject.com^
||pubmatic.com^
||openx.net^
||casalemedia.com^
||moatads.com^
||hotjar.com^$third-party
||mixpanel.com^$third-party
||segment.io^$third-party
||cdn.segment.com^$third-party
||fullstory.com^$third-party
||clarity.ms^
||bat.bing.com^
||ads.linkedin.com^
||px.ads.linkedin.com^
||analytics.tiktok.com^
||ads-twitter.com^
||static.ads-twitter.com^
||yandex.ru/metrika^
||mc.yandex.ru^
||chartbeat.com^$third-party
||newrelic.com^$third-party
||nr-data.net^
||branch.io^$third-party
||adform.net^
||smartadserver.com^
||teads.tv^
||media.net^$third-party
||zedo.com^
||popads.net^
||propellerads.com^
||adcolony.com^
||doubleverify.com^
||imrworldwide.com^
||krxd.net^
||bluekai.com^
||demdex.net^
||everesttech.net^
||omtrdc.net^$third-party
##.adsbygoogle
##ins.adsbygoogle
##.ad-banner
##.ad-slot
##.ad-container
##.advertisement
##.sponsored-ad
##.dfp-ad
##.google-ad
##div[id^="div-gpt-ad"]
##.taboola
##.OUTBRAIN
"#;

/// Lists downloaded in the background: (file name, address, kind).
pub const LISTS: &[(&str, &str, Kind)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt", Kind::Ads),
    ("easyprivacy.txt", "https://easylist.to/easylist/easyprivacy.txt", Kind::Trackers),
    ("malware-hosts.txt", "https://urlhaus.abuse.ch/downloads/hostfile/", Kind::Dangerous),
    ("phishing-hosts.txt", "https://phishing.army/download/phishing_army_blocklist.txt", Kind::Dangerous),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ads,
    Trackers,
    Dangerous,
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase())
}

/// The host and each parent domain: a.b.example.com, b.example.com, example.com, com.
fn suffixes(host: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(host), |h| h.split_once('.').map(|(_, r)| r))
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(domain) && host.as_bytes().get(host.len() - domain.len() - 1) == Some(&b'.')
}

/// A separator for `^`: anything but a letter, digit, or `_ - . %`.
fn is_sep(c: u8) -> bool {
    !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'%'))
}

/// Match a filter glob against `s` (anchored at the start of `s`).
/// `end`: the pattern must reach the end of `s`.
fn glob(p: &[u8], s: &[u8], end: bool) -> bool {
    match p.first() {
        None => !end || s.is_empty(),
        Some(b'*') => {
            let rest = &p[1..];
            if rest.is_empty() {
                return true;
            }
            (0..=s.len()).any(|i| glob(rest, &s[i..], end))
        }
        Some(b'^') => match s.first() {
            None => glob(&p[1..], s, end),
            Some(&c) if is_sep(c) => glob(&p[1..], &s[1..], end),
            _ => false,
        },
        Some(&c) => s.first() == Some(&c) && glob(&p[1..], &s[1..], end),
    }
}

impl Rule {
    fn matches_url(&self, url: &str, url_lower: &str, host: &str) -> bool {
        let s = if self.opts.match_case { url } else { url_lower };
        let p = self.pattern.as_bytes();
        if let Some(h) = &self.host {
            if !domain_matches(host, h) {
                return false;
            }
            // The rest of the pattern starts right after the host (and port).
            let Some(start) = s.find("://").map(|i| i + 3) else { return false };
            let after_host = s[start..].find(|c: char| c == '/' || c == '?' || c == '#').map_or(s.len(), |i| start + i);
            let host_end = s[start..after_host].rfind(host).map(|i| start + i + host.len()).unwrap_or(after_host);
            return glob(p, &s.as_bytes()[host_end..], self.anchor_end);
        }
        let sb = s.as_bytes();
        if self.anchor_start {
            return glob(p, sb, self.anchor_end);
        }
        // Unanchored: try every starting point (a leading literal narrows the search).
        match p.first() {
            Some(&c) if c != b'*' && c != b'^' => sb.iter().enumerate().filter(|(_, b)| **b == c).any(|(i, _)| glob(p, &sb[i..], self.anchor_end)),
            _ => (0..=sb.len()).any(|i| glob(p, &sb[i..], self.anchor_end)),
        }
    }

    fn applies(&self, ty: u16, third: bool, page_host: &str) -> bool {
        if self.opts.types & ty == 0 {
            return false;
        }
        if self.opts.third_party.is_some_and(|t| t != third) {
            return false;
        }
        if !self.opts.on.is_empty() && !self.opts.on.iter().any(|d| domain_matches(page_host, d)) {
            return false;
        }
        !self.opts.not_on.iter().any(|d| domain_matches(page_host, d))
    }
}

/// Parse a network rule's `$options`. None = the rule uses an option we can't honor, so it's skipped.
fn parse_opts(s: &str) -> Option<(Opts, bool)> {
    let mut o = Opts::default();
    let mut doc = false;
    let mut not_types = 0u16;
    for opt in s.split(',') {
        let (neg, name) = match opt.strip_prefix('~') {
            Some(n) => (true, n),
            None => (false, opt),
        };
        let (name, value) = name.split_once('=').unwrap_or((name, ""));
        let bit = match name {
            "script" => T_SCRIPT,
            "image" => T_IMAGE,
            "stylesheet" | "css" => T_STYLE,
            "xmlhttprequest" | "xhr" | "websocket" | "ping" | "beacon" => T_XHR,
            "subdocument" | "frame" => T_FRAME,
            "font" => T_FONT,
            "media" => T_MEDIA,
            "object" => T_OBJECT,
            "other" => T_OTHER,
            "all" => T_ALL,
            _ => 0,
        };
        if bit != 0 {
            if neg { not_types |= bit } else { o.types |= bit }
            continue;
        }
        match name {
            "third-party" | "3p" => o.third_party = Some(!neg),
            "first-party" | "1p" => o.third_party = Some(neg),
            "strict3p" => o.third_party = Some(true),
            "strict1p" => o.third_party = Some(false),
            "domain" | "from" => {
                for d in value.split('|').filter(|d| !d.is_empty()) {
                    match d.strip_prefix('~') {
                        Some(d) => o.not_on.push(d.to_ascii_lowercase()),
                        None => o.on.push(d.to_ascii_lowercase()),
                    }
                }
            }
            "important" => o.important = true,
            "match-case" => o.match_case = true,
            "document" | "doc" | "elemhide" | "ehide" => doc = true,
            // Blocking with a stand-in resource still blocks.
            "redirect" | "redirect-rule" | "rewrite" | "empty" | "mp4" => {}
            "generichide" | "ghide" | "specifichide" | "shide" | "genericblock" | "popup" | "popunder" => return None,
            _ => return None, // csp=, removeparam=, header=, denyallow=, badfilter… not supported
        }
    }
    if o.types == 0 {
        o.types = if not_types != 0 { T_ALL & !not_types } else { T_ALL };
    }
    Some((o, doc))
}

/// A word in the pattern every matching URL must contain (for the index).
fn token_of(pattern: &str) -> Option<String> {
    let bytes = pattern.as_bytes();
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'%' {
            let s = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'%') {
                i += 1;
            }
            // A word next to `*` might continue in the URL, so it can't be an exact token.
            let wild = (s > 0 && bytes[s - 1] == b'*') || (i < bytes.len() && bytes[i] == b'*');
            if !wild && i - s >= 3 && best.is_none_or(|(a, b)| i - s > b - a) {
                best = Some((s, i));
            }
        } else {
            i += 1;
        }
    }
    best.map(|(a, b)| pattern[a..b].to_ascii_lowercase())
}

fn url_tokens(url_lower: &str) -> impl Iterator<Item = &str> {
    url_lower.split(|c: char| !(c.is_ascii_alphanumeric() || c == '%')).filter(|t| t.len() >= 3)
}

/// Selectors that use extended syntax (procedural filters) we don't run.
fn plain_selector(sel: &str) -> bool {
    const EXT: &[&str] = &[":-abp-", ":has-text", ":contains", ":upward", ":xpath", ":matches-", ":style(", ":remove", ":min-text", ":watch-attr", ":others", ":if(", ":if-not", ":nth-ancestor", ":matches-path", ":matches-attr", ":matches-media", ":matches-prop", ":remove-attr", ":remove-class"];
    !sel.is_empty() && !sel.starts_with('+') && !sel.starts_with('^') && !EXT.iter().any(|e| sel.contains(e))
}

/// The class or id a generic selector starts with (`.ad-box div` -> class "ad-box").
fn leading_key(sel: &str) -> Option<(bool, String)> {
    let (is_class, rest) = match sel.as_bytes().first()? {
        b'.' => (true, &sel[1..]),
        b'#' => (false, &sel[1..]),
        _ => {
            // tag.class / tag#id / tag[id^=…] forms: use the first class or id.
            let i = sel.find(['.', '#'])?;
            if sel[..i].chars().all(|c| c.is_ascii_alphanumeric()) {
                return leading_key(&sel[i..]);
            }
            return None;
        }
    };
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
    (!name.is_empty()).then_some((is_class, name))
}

impl Engine {
    pub fn add_list(&mut self, text: &str, kind: Kind) {
        if kind == Kind::Dangerous {
            for line in text.lines() {
                let line = line.split('#').next().unwrap_or("").trim();
                // "0.0.0.0 host", "127.0.0.1 host" (hosts files) or just "host".
                let host = line.split_ascii_whitespace().last().unwrap_or("");
                if host.contains('.') && !matches!(host, "localhost" | "0.0.0.0" | "127.0.0.1" | "broadcasthost") {
                    self.dangerous.insert(host.to_ascii_lowercase());
                }
            }
            self.dangerous_count = self.dangerous.len();
            return;
        }
        for line in text.lines() {
            self.add_rule(line.trim());
        }
        self.rule_count = self.rules.len() + self.hide_by_class.len() + self.hide_by_id.len() + self.hide_on.len();
    }

    fn add_rule(&mut self, line: &str) {
        if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
            return;
        }
        // Element hiding: [domains]##selector, #@# for exceptions.
        for (sep, exception) in [("#@#", true), ("##", false)] {
            if let Some(i) = line.find(sep) {
                let (domains, sel) = (&line[..i], line[i + sep.len()..].trim());
                if domains.contains(['/', '*', '|', '$']) && !domains.is_empty() || !plain_selector(sel) {
                    return;
                }
                let doms: Vec<String> = domains.split(',').filter(|d| !d.is_empty()).map(|d| d.to_ascii_lowercase()).collect();
                let positive: Vec<&String> = doms.iter().filter(|d| !d.starts_with('~')).collect();
                if exception {
                    if positive.is_empty() {
                        self.unhide.insert((String::new(), sel.to_string()));
                    }
                    for d in positive {
                        self.unhide.insert((d.clone(), sel.to_string()));
                    }
                } else if positive.is_empty() {
                    // Generic: indexed by its leading class or id; others are too costly to test on every page.
                    for d in doms.iter().filter_map(|d| d.strip_prefix('~')) {
                        self.unhide.insert((d.to_string(), sel.to_string()));
                    }
                    match leading_key(sel) {
                        Some((true, k)) => self.hide_by_class.entry(k).or_default().push(sel.to_string()),
                        Some((false, k)) => self.hide_by_id.entry(k).or_default().push(sel.to_string()),
                        None => {}
                    }
                } else {
                    for d in positive {
                        self.hide_on.entry(d.clone()).or_default().push(sel.to_string());
                    }
                }
                return;
            }
        }
        if line.contains("#?#") || line.contains("#$#") || line.contains("#%#") {
            return;
        }
        let (exception, body) = match line.strip_prefix("@@") {
            Some(b) => (true, b),
            None => (false, line),
        };
        // Options start at the last `$` (a `$` can appear inside a pattern).
        let (pat, opts) = match body.rfind('$') {
            Some(i) if !body[i + 1..].contains('/') || body[i + 1..].contains("domain=") => (&body[..i], &body[i + 1..]),
            _ => (body, ""),
        };
        let Some((opts, doc)) = (if opts.is_empty() { Some((Opts { types: T_ALL, ..Default::default() }, false)) } else { parse_opts(opts) }) else { return };
        if pat.starts_with('/') && pat.ends_with('/') && pat.len() > 2 {
            return; // regular expressions: not supported
        }
        let mut p = pat;
        let mut host = None;
        let mut anchor_start = false;
        if let Some(rest) = p.strip_prefix("||") {
            let end = rest.find(['^', '/', '*', ':', '?', '|']).unwrap_or(rest.len());
            let h = &rest[..end];
            if !h.is_empty() && !h.contains('*') {
                host = Some(h.to_ascii_lowercase());
                p = &rest[end..];
            } else {
                p = rest;
            }
        } else if let Some(rest) = p.strip_prefix('|') {
            anchor_start = true;
            p = rest;
        }
        let anchor_end = p.ends_with('|');
        let p = p.trim_end_matches('|');
        let pattern = if opts.match_case { p.to_string() } else { p.to_ascii_lowercase() };
        if host.is_none() && pattern.trim_matches('*').is_empty() && !doc {
            return; // would match everything
        }
        let rule = Rule { host: host.clone(), pattern, anchor_start, anchor_end, opts, exception, text: line.to_string() };
        if doc {
            if exception {
                self.page_exceptions.push(rule);
            }
            return;
        }
        let idx = self.rules.len();
        match (&host, token_of(&rule.pattern)) {
            (Some(h), _) => self.by_host.entry(h.clone()).or_default().push(idx),
            (None, Some(t)) => self.by_token.entry(t).or_default().push(idx),
            (None, None) => self.untokened.push(idx),
        }
        self.rules.push(rule);
    }

    fn candidates<'s, 'q>(&'s self, host: &'q str, url_lower: &'q str) -> impl Iterator<Item = &'s Rule> + use<'s, 'q> {
        let by_host = suffixes(host).filter_map(|s| self.by_host.get(s)).flatten();
        let by_token = url_tokens(url_lower).filter_map(|t| self.by_token.get(t)).flatten();
        by_host.chain(by_token).chain(self.untokened.iter()).map(|&i| &self.rules[i])
    }

    /// The rule that blocks this request, if any.
    pub fn check(&self, url: &str, page: &str, dest: Dest) -> Option<&str> {
        let host = host_of(url)?;
        let page_host = host_of(page).unwrap_or_default();
        if self.page_allowed(page, &page_host) {
            return None;
        }
        let third = crate::cookies::site_of(&host) != crate::cookies::site_of(&page_host);
        let ty = type_bit(dest);
        let lower = url.to_ascii_lowercase();
        let mut block: Option<&Rule> = None;
        let mut except = false;
        for r in self.candidates(&host, &lower) {
            if !r.applies(ty, third, &page_host) || !r.matches_url(url, &lower, &host) {
                continue;
            }
            if r.exception {
                except = true;
            } else if r.opts.important {
                return Some(&r.text);
            } else if block.is_none() {
                block = Some(r);
            }
        }
        if except { None } else { block.map(|r| r.text.as_str()) }
    }

    fn page_allowed(&self, page: &str, page_host: &str) -> bool {
        let lower = page.to_ascii_lowercase();
        self.page_exceptions.iter().any(|r| r.matches_url(page, &lower, page_host))
    }

    /// CSS that hides ad leftovers in this page: site-specific rules, plus generic rules whose
    /// class or id actually appears in the page.
    pub fn hiding_css(&self, page: &str, classes: &HashSet<String>, ids: &HashSet<String>) -> String {
        let page_host = host_of(page).unwrap_or_default();
        if page_host.is_empty() || self.page_allowed(page, &page_host) {
            return String::new();
        }
        let mut sels: Vec<&str> = Vec::new();
        for c in classes {
            if let Some(v) = self.hide_by_class.get(c) {
                sels.extend(v.iter().map(String::as_str));
            }
        }
        for i in ids {
            if let Some(v) = self.hide_by_id.get(i) {
                sels.extend(v.iter().map(String::as_str));
            }
        }
        for d in suffixes(&page_host) {
            if let Some(v) = self.hide_on.get(d) {
                sels.extend(v.iter().map(String::as_str));
            }
        }
        let unhidden = |s: &str| self.unhide.contains(&(String::new(), s.to_string())) || suffixes(&page_host).any(|d| self.unhide.contains(&(d.to_string(), s.to_string())));
        let mut seen = HashSet::new();
        let mut css = String::new();
        for s in sels {
            if seen.insert(s) && !unhidden(s) {
                // One rule per selector: a selector we can't parse then drops only itself.
                css.push_str(s);
                css.push_str("{display:none!important}\n");
            }
        }
        css
    }

    /// Is this host (or a parent domain) on a malware or phishing list?
    pub fn is_dangerous(&self, url: &str) -> bool {
        let Some(host) = host_of(url) else { return false };
        suffixes(&host).any(|h| self.dangerous.contains(h))
    }
}

// ---------------------------------------------------------------- the shared engine

static ENGINE: RwLock<Option<Arc<Engine>>> = RwLock::new(None);
/// Hosts you chose to visit anyway after the warning (until Ironswift closes).
static PROCEED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn dir() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("filters"))
}

fn build() -> Engine {
    let s = crate::settings::get();
    let mut e = Engine::default();
    let dir = dir();
    let read = |name: &str| dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(name)).ok());
    let mut have_full = false;
    for (name, _, kind) in LISTS {
        let on = match kind {
            Kind::Ads => s.block_ads,
            Kind::Trackers => s.block_trackers,
            Kind::Dangerous => s.safe_browsing,
        };
        if let (true, Some(text)) = (on, read(name)) {
            e.add_list(&text, *kind);
            have_full |= *kind != Kind::Dangerous;
        }
    }
    if !have_full && (s.block_ads || s.block_trackers) {
        e.add_list(BUILT_IN, Kind::Trackers);
    }
    if let Some(text) = read("custom.txt") {
        e.add_list(&text, Kind::Ads);
    }
    if let Some(text) = read("custom-dangerous.txt") {
        e.add_list(&text, Kind::Dangerous);
    }
    e
}

pub fn engine() -> Arc<Engine> {
    if let Some(e) = ENGINE.read().ok().and_then(|g| g.clone()) {
        return e;
    }
    let e = Arc::new(build());
    if let Ok(mut g) = ENGINE.write() {
        *g = Some(e.clone());
    }
    e
}

/// Rebuild from the files (after an update or a settings change).
pub fn reload() {
    if let Ok(mut g) = ENGINE.write() {
        *g = None;
    }
}

/// Sites where you turned blocking off.
fn exempt(page: &str) -> bool {
    let Some(h) = host_of(page) else { return false };
    let list = crate::settings::get().no_blocking;
    list.iter().any(|d| domain_matches(&h, d))
}

/// Why a page's request should be stopped (None = let it through).
pub fn blocked(url: &str, page: &str, dest: Dest) -> Option<String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) || !(page.starts_with("http://") || page.starts_with("https://")) {
        return None;
    }
    if exempt(page) {
        return None;
    }
    let e = engine();
    let rule = e.check(url, page, dest)?;
    log!("[blocked] {url} ({rule})");
    Some(format!("blocked by filter rule {rule}"))
}

/// Element-hiding CSS for a page (empty when there's nothing to hide).
pub fn hiding_css(page: &str, dom: &crate::dom::Dom) -> String {
    if !(page.starts_with("http://") || page.starts_with("https://")) || exempt(page) {
        return String::new();
    }
    let mut classes = HashSet::new();
    let mut ids = HashSet::new();
    dom.walk(crate::dom::Dom::ROOT, |n| {
        if let Some(c) = dom.attr(n, "class") {
            classes.extend(c.split_ascii_whitespace().map(String::from));
        }
        if let Some(i) = dom.attr(n, "id") {
            ids.insert(i.to_string());
        }
        true
    });
    engine().hiding_css(page, &classes, &ids)
}

/// Should opening this page show the dangerous-site warning first?
pub fn dangerous(url: &str) -> bool {
    if !crate::settings::get().safe_browsing || !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    let host = host_of(url).unwrap_or_default();
    if PROCEED.lock().is_ok_and(|p| p.contains(&host)) {
        return false;
    }
    engine().is_dangerous(url)
}

/// You chose to visit this site despite the warning: don't warn again this session.
pub fn proceed(url: &str) {
    if let (Some(h), Ok(mut p)) = (host_of(url), PROCEED.lock()) {
        p.push(h);
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The warning shown instead of a dangerous site.
pub fn warning_page(url: &str) -> String {
    let host = host_of(url).unwrap_or_default();
    format!(
        r#"<!DOCTYPE html><html><head><title>Dangerous site blocked</title><style>
body{{font-family:sans-serif;background:#b91c1c;color:white;margin:0;min-height:100vh}}
main{{max-width:640px;margin:0 auto;padding:80px 24px}} h1{{font-size:28px}}
p{{font-size:16px;line-height:1.5}} .url{{background:rgba(0,0,0,.25);padding:6px 10px;border-radius:6px;word-break:break-all}}
a.back{{display:inline-block;background:white;color:#b91c1c;padding:10px 20px;border-radius:8px;text-decoration:none;font-weight:bold}}
.more{{margin-top:40px;font-size:13px;opacity:.85}} .more a{{color:white}}
</style></head><body><main>
<h1>This site may harm your computer or steal your information</h1>
<p><b>{host}</b> is on a list of sites that spread malware or try to trick people into giving away passwords and card numbers.</p>
<p class="url">{url_esc}</p>
<p><a class="back" href="about:home">Go back to safety</a></p>
<p class="more">The lists come from URLhaus (abuse.ch) and Phishing Army and are updated automatically. If you understand the risk, you can <a href="about:blocked?proceed={url_enc}">visit this site anyway</a>.</p>
</main></body></html>"#,
        host = esc(&host),
        url_esc = esc(url),
        url_enc = enc(url)
    )
}

// ---------------------------------------------------------------- keeping the lists up to date

/// How old a list may get before it's downloaded again.
const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(4 * 24 * 3600);

/// Download any list that is missing or older than four days, then reload. Runs on its own
/// thread a little after startup so it never slows the first page.
pub fn start_updates() {
    if std::env::var("IRONSWIFT_NO_UPDATES").is_ok_and(|v| v == "1") || cfg!(test) {
        return;
    }
    let _ = std::thread::Builder::new().name("filter-lists".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(20));
        update_now(false);
    });
}

/// Fetch the lists now (`force`: even fresh ones). Returns a summary for about:settings.
pub fn update_now(force: bool) -> String {
    let Some(dir) = dir() else { return "No profile folder.".into() };
    let _ = std::fs::create_dir_all(&dir);
    let s = crate::settings::get();
    let mut done = Vec::new();
    for (name, url, kind) in LISTS {
        let on = match kind {
            Kind::Ads => s.block_ads,
            Kind::Trackers => s.block_trackers,
            Kind::Dangerous => s.safe_browsing,
        };
        let path = dir.join(name);
        let fresh = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < MAX_AGE);
        if !on || (fresh && !force) {
            continue;
        }
        let req = crate::net::Request { method: "GET", url, headers: &[], body: None, top: None, cookies: false, extra: &[] };
        match crate::net::http(&req) {
            Ok(r) if r.status == 200 && r.body.len() > 100 => {
                let tmp = path.with_extension("tmp");
                if std::fs::write(&tmp, &r.body).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
                    done.push(*name);
                }
            }
            Ok(r) => log!("[filters] {url}: HTTP {}", r.status),
            Err(e) => log!("[filters] {url}: {e}"),
        }
    }
    reload();
    let e = engine();
    format!("{} lists updated. {} blocking rules and {} dangerous sites loaded.", done.len(), e.rule_count, e.dangerous_count)
}

/// When each list was last downloaded, for about:settings.
pub fn status() -> String {
    let Some(dir) = dir() else { return String::new() };
    LISTS
        .iter()
        .map(|(name, _, _)| {
            let age = std::fs::metadata(dir.join(name)).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            match age {
                Some(a) if a.as_secs() < 3600 => format!("{name}: updated just now"),
                Some(a) if a.as_secs() < 2 * 86400 => format!("{name}: updated {} hours ago", a.as_secs() / 3600),
                Some(a) => format!("{name}: updated {} days ago", a.as_secs() / 86400),
                None => format!("{name}: not downloaded yet"),
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eng(text: &str) -> Engine {
        let mut e = Engine::default();
        e.add_list(text, Kind::Ads);
        e
    }

    #[test]
    fn network_rules() {
        let e = eng("||ads.example.com^\n||track.net^$third-party\n/banner/*/ad_\n||cdn.site.com/js/ads.js$script\n@@||ads.example.com/allowed/\n|https://exact.com/x.js|\n||imp.org^$image,domain=news.com|~sports.news.com\n||boom.com^$csp=script-src 'none'\n");
        let b = |u: &str, p: &str, d: Dest| e.check(u, p, d).is_some();
        assert!(b("https://ads.example.com/a.js", "https://site.com/", Dest::Script));
        assert!(b("https://x.ads.example.com/a.js", "https://site.com/", Dest::Script));
        assert!(!b("https://notads.example.com/a.js", "https://site.com/", Dest::Script));
        assert!(!b("https://ads.example.com.evil.org/a.js", "https://site.com/", Dest::Script));
        assert!(!b("https://ads.example.com/allowed/x.js", "https://site.com/", Dest::Script), "exception");
        assert!(b("https://track.net/p.gif", "https://site.com/", Dest::Image));
        assert!(!b("https://track.net/p.gif", "https://www.track.net/", Dest::Image), "first-party");
        assert!(b("https://img.com/banner/big/ad_1.png", "https://site.com/", Dest::Image));
        assert!(!b("https://img.com/banner/ad_1.png", "https://site.com/", Dest::Image));
        assert!(b("https://cdn.site.com/js/ads.js", "https://site.com/", Dest::Script));
        assert!(!b("https://cdn.site.com/js/ads.js", "https://site.com/", Dest::Image), "type option");
        assert!(b("https://exact.com/x.js", "https://a.com/", Dest::Script));
        assert!(!b("https://exact.com/x.js?v=2", "https://a.com/", Dest::Script), "end anchor");
        assert!(b("https://imp.org/1.gif", "https://www.news.com/", Dest::Image));
        assert!(!b("https://imp.org/1.gif", "https://sports.news.com/", Dest::Image));
        assert!(!b("https://imp.org/1.gif", "https://other.com/", Dest::Image));
        assert!(!b("https://boom.com/", "https://a.com/", Dest::Script), "unsupported option: rule skipped");
    }

    #[test]
    fn separators_and_page_exceptions() {
        let e = eng("||ad.com^\n@@||trusted.org^$document\n");
        assert!(e.check("https://ad.com:8080/x", "https://a.com/", Dest::Image).is_some());
        assert!(e.check("https://ad.com", "https://a.com/", Dest::Image).is_some());
        assert!(e.check("https://ad.community/x", "https://a.com/", Dest::Image).is_none());
        assert!(e.check("https://ad.com/x", "https://www.trusted.org/", Dest::Image).is_none());
    }

    #[test]
    fn hiding() {
        let e = eng("##.ad-box\n##.promo > a\n###sidebar-ad\nnews.com##.paywall-ad\nnews.com#@#.ad-box\n##div:has-text(Sponsored)\n");
        let classes: HashSet<String> = ["ad-box", "promo", "content"].iter().map(|s| s.to_string()).collect();
        let ids: HashSet<String> = ["sidebar-ad"].iter().map(|s| s.to_string()).collect();
        let css = e.hiding_css("https://blog.com/", &classes, &ids);
        assert!(css.contains(".ad-box{display:none!important}"));
        assert!(css.contains(".promo > a{"));
        assert!(css.contains("#sidebar-ad{"));
        assert!(!css.contains("paywall") && !css.contains("has-text"));
        let css = e.hiding_css("https://www.news.com/", &classes, &ids);
        assert!(css.contains(".paywall-ad{") && !css.contains(".ad-box{"));
    }

    #[test]
    fn dangerous_hosts() {
        let mut e = Engine::default();
        e.add_list("# URLhaus\n127.0.0.1 localhost\n127.0.0.1 malware.example\nphish.test\n", Kind::Dangerous);
        assert!(e.is_dangerous("http://malware.example/payload.exe"));
        assert!(e.is_dangerous("https://login.phish.test/"));
        assert!(!e.is_dangerous("https://example.com/"));
        assert!(!e.is_dangerous("http://localhost/"));
    }

    #[test]
    fn built_in_list_parses() {
        let e = eng(BUILT_IN);
        assert!(e.check("https://www.google-analytics.com/analytics.js", "https://blog.com/", Dest::Script).is_some());
        assert!(e.check("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://news.com/", Dest::Script).is_some());
        assert!(e.check("https://connect.facebook.net/en_US/fbevents.js", "https://shop.com/", Dest::Script).is_some());
        assert!(e.check("https://example.com/app.js", "https://example.com/", Dest::Script).is_none());
    }

    /// With the real lists: IRONSWIFT_LISTS=dir cargo test --release real_lists -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_lists() {
        let Ok(dir) = std::env::var("IRONSWIFT_LISTS") else { return };
        let t = std::time::Instant::now();
        let mut e = Engine::default();
        for f in ["easylist.txt", "easyprivacy.txt"] {
            e.add_list(&std::fs::read_to_string(format!("{dir}/{f}")).unwrap(), Kind::Ads);
        }
        println!("loaded {} rules in {:?} (network {}, host-indexed {}, token-indexed {}, untokened {})", e.rule_count, t.elapsed(), e.rules.len(), e.by_host.len(), e.by_token.len(), e.untokened.len());
        let cases = [
            ("https://www.google-analytics.com/analytics.js", "https://news.example/", Dest::Script, true),
            ("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://news.example/", Dest::Script, true),
            ("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", "https://blog.example/", Dest::Script, true),
            ("https://static.xx.fbcdn.net/rsrc.php/v3/yX/r/abc.js", "https://www.facebook.com/", Dest::Script, false),
            ("https://cdn.jsdelivr.net/npm/jquery@3/dist/jquery.min.js", "https://site.example/", Dest::Script, false),
            ("https://upload.wikimedia.org/wikipedia/commons/a/a9/Example.jpg", "https://en.wikipedia.org/", Dest::Image, false),
            ("https://example.com/ads/banners/728x90.gif", "https://example.com/", Dest::Image, true),
        ];
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            for (u, p, d, _) in &cases {
                std::hint::black_box(e.check(u, p, *d));
            }
        }
        println!("{} checks in {:?}", 1000 * cases.len(), t.elapsed());
        for (u, p, d, want) in cases {
            let got = e.check(u, p, d);
            println!("{u}: {got:?}");
            assert_eq!(got.is_some(), want, "{u}");
        }
    }
}
