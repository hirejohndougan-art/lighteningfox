//! Content blocking with the filter lists other blockers use (EasyList, EasyPrivacy, the uBlock
//! Origin lists, Peter Lowe's list), and a warning page for sites known to spread malware or
//! phish for passwords.
//!
//! - **Network rules** in Adblock Plus / uBlock Origin syntax stop requests before they leave
//!   the computer: `||ads.example^$script,third-party`, `/regex/`, `@@` exceptions,
//!   `$important`, `$badfilter`, `$domain=` / `$to=` / `$denyallow=`, `$method=`, `$popup`.
//!   `$redirect=` answers a blocked request with a harmless stand-in (an empty script, a blank
//!   GIF, a no-op `gtm.js`…) so pages that expect it keep working. `$removeparam=` strips
//!   tracking parameters from addresses; `$csp=` adds a Content Security Policy to pages.
//!   Navigating to a site a filter blocks outright (`||site^`, `$document`) shows a warning page.
//! - **Cosmetic rules** hide what's left of the ads: `##.ad-banner`, `example.com##.promo`,
//!   `example.*##…`, `:style(…)`, and procedural filters (`#?#`, `:has-text()`, `:upward()`,
//!   `:matches-css()`, `:remove()`…) run by `cosmetic.js` as the page changes.
//! - **Scriptlets** (`example.com##+js(set-constant, adBlockDetected, false)`) defuse
//!   anti-adblock and ad scripts from inside the page. They are our own implementations
//!   (`scriptlets.js`) of the scriptlets uBlock Origin's lists name.
//! - Lists are preprocessed like uBlock Origin does (`!#if`, `!#else`, `!#endif`, `!#include`).
//! - **Dangerous sites**: host lists (URLhaus malware hosts, Phishing Army) and uBlock's badware
//!   list are checked before a page is opened. A match shows a warning page; you can still go on.
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
/// What a rule with no type option applies to (not documents or popups).
const T_ALL: u16 = 511;
/// The page itself (a navigation).
const T_DOC: u16 = 512;
/// A window a page opens (window.open).
const T_POPUP: u16 = 1024;

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

/// What a network rule does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Block (with `value` = a stand-in resource for `$redirect=`).
    Block,
    /// `$redirect-rule=`: only says which stand-in to use when something else blocks.
    Redirect,
    /// `$removeparam=` (`value`: the parameter, `/regex/`, or "" for all of them).
    RemoveParam,
    /// `$csp=` (`value`: the policy).
    Csp,
}

/// Page-wide exceptions (`@@||site^$document`, `$elemhide`, `$generichide`…).
const PAGE_ALL: u8 = 1;
const PAGE_GHIDE: u8 = 2;
const PAGE_SHIDE: u8 = 4;
const PAGE_GBLOCK: u8 = 8;

#[derive(Debug, Clone, Default)]
struct Opts {
    /// Request types the rule applies to.
    types: u16,
    /// The rule named its types (a rule without them is a plain "block this host" rule).
    explicit_types: bool,
    /// Some(true) = only third-party requests, Some(false) = only first-party.
    third_party: Option<bool>,
    /// `domain=` — pages the rule applies on / doesn't apply on.
    on: Vec<String>,
    not_on: Vec<String>,
    /// `to=` / `denyallow=` — the request's own host must (not) be one of these.
    to_on: Vec<String>,
    to_not: Vec<String>,
    important: bool,
    match_case: bool,
}

struct Rule {
    /// For `||host…` rules: the host, and the rest of the pattern matched right after it.
    host: Option<String>,
    /// Glob with `*` (anything) and `^` (a separator or the end).
    pattern: String,
    /// `/…/` rules, and text every match must contain (checked first: it's much cheaper).
    regex: Option<regress::Regex>,
    regex_lit: Option<String>,
    anchor_start: bool,
    anchor_end: bool,
    opts: Opts,
    exception: bool,
    action: Action,
    value: String,
    /// Turned off by a `$badfilter` rule.
    disabled: bool,
    /// For a page-wide exception: what it turns off (`PAGE_…` flags).
    page: u8,
    /// The rule as written, for the log.
    text: String,
}

/// Rules indexed so a request only looks at the few that could match it.
#[derive(Default)]
struct Index {
    rules: Vec<Rule>,
    /// `||host` rules by host.
    by_host: HashMap<String, Vec<usize>>,
    /// Other rules by a word that must appear in the URL.
    by_token: HashMap<String, Vec<usize>>,
    /// Rules with no usable word but limited to some sites (`domain=`), by those sites.
    by_page: HashMap<String, Vec<usize>>,
    /// Rules with no usable word (checked for every request).
    untokened: Vec<usize>,
}

impl Index {
    fn add(&mut self, rule: Rule) {
        let idx = self.rules.len();
        let token = if rule.regex.is_some() { None } else { token_of(&rule.pattern) };
        match (&rule.host, token) {
            (Some(h), _) => self.by_host.entry(h.clone()).or_default().push(idx),
            (None, Some(t)) => self.by_token.entry(t).or_default().push(idx),
            (None, None) if !rule.opts.on.is_empty() => {
                for d in &rule.opts.on {
                    self.by_page.entry(d.clone()).or_default().push(idx);
                }
            }
            (None, None) => self.untokened.push(idx),
        }
        self.rules.push(rule);
    }

    /// Rules that could match a request. `page_keys`: the page's host,
    /// its parent domains and their `name.*` forms.
    fn candidates<'s, 'q>(&'s self, host: &'q str, url_lower: &'q str, page_keys: &'q [String]) -> impl Iterator<Item = &'s Rule> + use<'s, 'q> {
        let by_host = suffixes(host).filter_map(|s| self.by_host.get(s)).flatten();
        let by_token = url_tokens(url_lower).filter_map(|t| self.by_token.get(t)).flatten();
        let by_page = page_keys.iter().filter_map(|k| self.by_page.get(k)).flatten();
        by_host.chain(by_token).chain(by_page).chain(self.untokened.iter()).map(|&i| &self.rules[i])
            .filter(|r| !r.disabled)
    }
}

/// A site-specific cosmetic rule: a selector, and the declarations to apply (None = hide).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Cosmetic {
    sel: String,
    style: Option<String>,
}

#[derive(Default)]
pub struct Engine {
    /// Blocking, exception and redirect rules.
    net: Index,
    /// `$removeparam` and `$csp` rules.
    mods: Index,
    /// `@@||site^$document`, `$elemhide`, `$generichide`, `$specifichide`, `$genericblock`.
    page_rules: Index,
    /// Rule texts `$badfilter` turned off (applied when the lists are all in).
    badfilters: HashSet<String>,
    /// Element hiding: generic selectors keyed by their leading class (`.x`), id (`#x`) or
    /// attribute (`[x…]`), and those with none of these (always included).
    hide_by_class: HashMap<String, Vec<String>>,
    hide_by_id: HashMap<String, Vec<String>>,
    hide_by_attr: HashMap<String, Vec<String>>,
    hide_always: Vec<String>,
    /// Cosmetic rules for particular sites: domain (or `name.*`) -> rules.
    hide_on: HashMap<String, Vec<Cosmetic>>,
    /// Procedural cosmetic rules for particular sites (run by cosmetic.js).
    procedural_on: HashMap<String, Vec<String>>,
    /// `#@#` exceptions: (domain or "" for everywhere, selector).
    unhide: HashSet<(String, String)>,
    /// `##+js(…)` scriptlets: domain -> the call as written inside `+js(…)`.
    scriptlets_on: HashMap<String, Vec<(String, bool)>>,
    /// HTML filters for inline scripts (`##^script:has-text(…)`): domain -> what to look for.
    script_removals_on: HashMap<String, Vec<String>>,
    /// Other HTML filters (`##^.ad-frame`, `##^div:has-text(…)`): domain -> selector. The
    /// elements are taken out of the page's HTML before it's used.
    html_filters_on: HashMap<String, Vec<String>>,
    /// Is the list being added trusted (uBlock Origin's own lists and your own rules)? Only
    /// trusted lists may use the `trusted-…` scriptlets, as in uBlock Origin.
    loading_trusted: bool,
    /// `#@#+js(…)` exceptions: (domain, call; "" = every scriptlet).
    unscriptlet: HashSet<(String, String)>,
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
||googletagmanager.com/gtm.js$script,redirect=googletagmanager_gtm.js,important
||google-analytics.com/analytics.js$script,redirect=google-analytics_analytics.js,important
||googletagservices.com/tag/js/gpt.js$script,redirect=googletagservices_gpt.js,important
||securepubads.g.doubleclick.net/tag/js/gpt.js$script,redirect=googletagservices_gpt.js,important
||pagead2.googlesyndication.com/pagead/js/adsbygoogle.js$script,redirect=googlesyndication_adsbygoogle.js,important
||imasdk.googleapis.com/js/sdkloader/ima3.js$script,redirect=google-ima.js,important
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
    ("ublock-filters.txt", "https://ublockorigin.github.io/uAssets/filters/filters.txt", Kind::Ads),
    ("ublock-privacy.txt", "https://ublockorigin.github.io/uAssets/filters/privacy.txt", Kind::Trackers),
    ("ublock-quick-fixes.txt", "https://ublockorigin.github.io/uAssets/filters/quick-fixes.txt", Kind::Ads),
    ("ublock-unbreak.txt", "https://ublockorigin.github.io/uAssets/filters/unbreak.txt", Kind::Ads),
    ("peter-lowe.txt", "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext", Kind::Ads),
    ("ublock-badware.txt", "https://ublockorigin.github.io/uAssets/filters/badware.txt", Kind::Badware),
    ("malware-hosts.txt", "https://urlhaus.abuse.ch/downloads/hostfile/", Kind::Dangerous),
    ("phishing-hosts.txt", "https://phishing.army/download/phishing_army_blocklist.txt", Kind::Dangerous),
    ("easylist-cookies.txt", "https://secure.fanboy.co.nz/fanboy-cookiemonster.txt", Kind::Annoyances),
    ("ublock-cookies.txt", "https://ublockorigin.github.io/uAssets/filters/annoyances-cookies.txt", Kind::Annoyances),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ads,
    Trackers,
    /// Filter rules (not a host list) for scam, malware and phishing sites.
    Badware,
    /// Host lists of malware and phishing sites.
    Dangerous,
    /// Cookie notices and other nuisances (off unless you turn it on).
    Annoyances,
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase())
}

/// The host and each parent domain: a.b.example.com, b.example.com, example.com, com.
fn suffixes(host: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(host), |h| h.split_once('.').map(|(_, r)| r))
}

/// Is `host` the domain (or under it)? `name.*` matches name.com, name.co.uk, www.name.de…
fn domain_matches(host: &str, domain: &str) -> bool {
    if let Some(name) = domain.strip_suffix(".*") {
        return entity_forms(host).any(|e| e.strip_suffix(".*") == Some(name));
    }
    host == domain || host.ends_with(domain) && host.as_bytes().get(host.len() - domain.len() - 1) == Some(&b'.')
}

/// The `name.*` forms of a host and its parents: www.news.co.uk -> www.news.*, news.*.
fn entity_forms(host: &str) -> impl Iterator<Item = String> + '_ {
    let site = crate::cookies::site_of(host);
    let suffix = site.split_once('.').map(|(_, s)| s.to_string()).unwrap_or_default();
    let tail = format!(".{suffix}");
    suffixes(host).filter_map(move |h| (!suffix.is_empty()).then(|| h.strip_suffix(&tail)).flatten().filter(|n| !n.is_empty()).map(|n| format!("{n}.*")))
}

/// The keys cosmetic and scriptlet rules for this page are filed under.
fn page_keys(host: &str) -> Vec<String> {
    let mut keys: Vec<String> = suffixes(host).map(String::from).collect();
    keys.extend(entity_forms(host));
    keys
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
            match rest.first() {
                None => true,
                Some(b'*') => glob(rest, s, end),
                // Only positions where the next literal character is can start the rest.
                Some(&c) if c != b'^' => s.iter().enumerate().filter(|(_, b)| **b == c).any(|(i, _)| glob(rest, &s[i..], end)),
                Some(_) => (0..=s.len()).any(|i| glob(rest, &s[i..], end)),
            }
        }
        Some(b'^') => match s.first() {
            None => glob(&p[1..], s, end),
            Some(&c) if is_sep(c) => glob(&p[1..], &s[1..], end),
            _ => false,
        },
        Some(&c) => s.first() == Some(&c) && glob(&p[1..], &s[1..], end),
    }
}

/// One request, as rules look at it.
struct Req<'a> {
    url: &'a str,
    lower: &'a str,
    host: &'a str,
    page_host: &'a str,
    third: bool,
    ty: u16,
}

impl Rule {
    fn matches_url(&self, url: &str, url_lower: &str, host: &str) -> bool {
        let s = if self.opts.match_case { url } else { url_lower };
        if let Some(re) = &self.regex {
            if self.regex_lit.as_ref().is_some_and(|l| !url_lower.contains(l.as_str())) {
                return false;
            }
            return re.find(s).is_some();
        }
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

    fn applies(&self, ty: u16, third: bool, page_host: &str, host: &str) -> bool {
        let o = &self.opts;
        if o.types & ty == 0 {
            return false;
        }
        if o.third_party.is_some_and(|t| t != third) {
            return false;
        }
        if !o.on.is_empty() && !o.on.iter().any(|d| domain_matches(page_host, d)) {
            return false;
        }
        if o.not_on.iter().any(|d| domain_matches(page_host, d)) {
            return false;
        }
        if !o.to_on.is_empty() && !o.to_on.iter().any(|d| domain_matches(host, d)) {
            return false;
        }
        !o.to_not.iter().any(|d| domain_matches(host, d))
    }

    fn hits(&self, q: &Req) -> bool {
        self.applies(q.ty, q.third, q.page_host, q.host) && self.matches_url(q.url, q.lower, q.host)
    }

    /// `||example.com^` and nothing else: blocks the whole site, pages included.
    fn whole_site(&self) -> bool {
        self.host.is_some() && matches!(self.pattern.as_str(), "" | "^") && !self.opts.explicit_types && self.opts.on.is_empty() && self.opts.third_party.is_none()
    }
}

/// A network rule's `$options`, parsed.
#[derive(Default)]
struct Parsed {
    opts: Opts,
    action: Option<Action>,
    value: String,
    /// Page-wide exception flags (`$document`, `$elemhide`…).
    page: u8,
    badfilter: bool,
}

/// Parse a network rule's `$options`. None = the rule uses an option we can't honor, so it's
/// skipped (better to miss an ad than to block something the rule didn't mean).
fn parse_opts(s: &str, exception: bool) -> Option<Parsed> {
    let mut p = Parsed::default();
    let mut not_types = 0u16;
    for opt in s.split(',') {
        let opt = opt.trim();
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
            "object" | "object-subrequest" => T_OBJECT,
            "other" => T_OTHER,
            "popup" if !neg => T_POPUP,
            "document" | "doc" if !exception && !neg => T_DOC,
            "all" if !neg => T_ALL | T_DOC | T_POPUP,
            _ => 0,
        };
        if bit != 0 {
            if neg { not_types |= bit } else { p.opts.types |= bit }
            continue;
        }
        match name {
            "third-party" | "3p" => p.opts.third_party = Some(!neg),
            "first-party" | "1p" => p.opts.third_party = Some(neg),
            "strict3p" => p.opts.third_party = Some(true),
            "strict1p" => p.opts.third_party = Some(false),
            "domain" | "from" => {
                for d in value.split('|').filter(|d| !d.is_empty()) {
                    if d.contains('/') {
                        return None; // regex domains
                    }
                    match d.strip_prefix('~') {
                        Some(d) => p.opts.not_on.push(d.to_ascii_lowercase()),
                        None => p.opts.on.push(d.to_ascii_lowercase()),
                    }
                }
            }
            "to" => {
                for d in value.split('|').filter(|d| !d.is_empty()) {
                    if d.contains('/') {
                        return None;
                    }
                    match d.strip_prefix('~') {
                        Some(d) => p.opts.to_not.push(d.to_ascii_lowercase()),
                        None => p.opts.to_on.push(d.to_ascii_lowercase()),
                    }
                }
            }
            "denyallow" => p.opts.to_not.extend(value.split('|').filter(|d| !d.is_empty()).map(|d| d.to_ascii_lowercase())),
            "important" => p.opts.important = true,
            "match-case" => p.opts.match_case = true,
            "badfilter" => p.badfilter = true,
            // Requests here are almost all GET: keep rules that cover GET, skip the rest.
            "method" => {
                let ms: Vec<&str> = value.split('|').collect();
                let pos: Vec<&str> = ms.iter().filter(|m| !m.starts_with('~')).copied().collect();
                if ms.iter().any(|m| m.eq_ignore_ascii_case("~get")) || (!pos.is_empty() && !pos.iter().any(|m| m.eq_ignore_ascii_case("get"))) {
                    return None;
                }
            }
            "document" | "doc" if exception => p.page |= PAGE_ALL,
            "elemhide" | "ehide" if exception => p.page |= PAGE_GHIDE | PAGE_SHIDE,
            "generichide" | "ghide" if exception => p.page |= PAGE_GHIDE,
            "specifichide" | "shide" if exception => p.page |= PAGE_SHIDE,
            "genericblock" if exception => p.page |= PAGE_GBLOCK,
            "redirect" => {
                p.value = value.split(':').next().unwrap_or("").to_string();
                p.action = Some(if exception { Action::Redirect } else { Action::Block });
            }
            "redirect-rule" => {
                p.value = value.split(':').next().unwrap_or("").to_string();
                p.action = Some(Action::Redirect);
            }
            "rewrite" => {
                p.value = value.to_string();
                p.action = Some(Action::Block);
            }
            "empty" => {
                p.value = "empty".into();
                p.action = Some(Action::Block);
            }
            "mp4" => {
                p.value = "noop-1s.mp4".into();
                p.action = Some(Action::Block);
            }
            "removeparam" | "queryprune" => {
                p.value = value.to_string();
                p.action = Some(Action::RemoveParam);
            }
            "csp" => {
                p.value = value.to_string();
                p.action = Some(Action::Csp);
            }
            "inline-script" => {
                p.value = "script-src 'unsafe-eval' * blob: data:".into();
                p.action = Some(Action::Csp);
            }
            "inline-font" => {
                p.value = "font-src *".into();
                p.action = Some(Action::Csp);
            }
            "reason" => {}
            // header=, permissions=, replace=, urltransform=, cname, webrtc, popunder…
            _ => return None,
        }
    }
    if p.opts.types == 0 {
        p.opts.types = if not_types != 0 { T_ALL & !not_types } else { T_ALL };
    } else {
        p.opts.explicit_types = true;
        p.opts.types &= !not_types;
    }
    if not_types != 0 {
        p.opts.explicit_types = true;
    }
    // Pages and frames: what `$csp` and `$removeparam` apply to unless types are named.
    if matches!(p.action, Some(Action::Csp | Action::RemoveParam)) && !p.opts.explicit_types {
        p.opts.types = T_ALL | T_DOC;
    }
    Some(p)
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

/// Plain text that every match of a regular expression must contain (lower-case), found
/// conservatively: the longest literal run outside groups, classes and alternations.
fn regex_literal(re: &str) -> Option<String> {
    if re.contains('|') {
        return None;
    }
    let b: Vec<char> = re.chars().collect();
    let (mut best, mut cur) = (String::new(), String::new());
    let mut depth = 0i32;
    let mut i = 0;
    let flush = |cur: &mut String, best: &mut String| {
        if cur.len() > best.len() {
            *best = cur.clone();
        }
        cur.clear();
    };
    while i < b.len() {
        let c = b[i];
        // The character just read, if it's a literal one.
        let lit = match c {
            '\\' => {
                i += 1;
                match b.get(i) {
                    Some(&n) if "./-_:=?&#%~".contains(n) => Some(n),
                    _ => None,
                }
            }
            '(' | '[' => {
                // Skip a whole class; groups only change depth.
                if c == '[' {
                    while i < b.len() && b[i] != ']' {
                        if b[i] == '\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                } else {
                    depth += 1;
                }
                None
            }
            ')' => {
                depth -= 1;
                None
            }
            c if c.is_ascii_alphanumeric() || "/-_:=&#%~,;@!'\"<>".contains(c) => Some(c),
            _ => None,
        };
        i += 1;
        match lit {
            Some(ch) if depth == 0 => {
                // A quantifier after it makes this character optional or repeated.
                if matches!(b.get(i), Some('?' | '*' | '{')) {
                    flush(&mut cur, &mut best);
                } else {
                    cur.push(ch.to_ascii_lowercase());
                }
            }
            _ => flush(&mut cur, &mut best),
        }
    }
    flush(&mut cur, &mut best);
    (best.len() >= 3).then_some(best)
}

fn url_tokens(url_lower: &str) -> impl Iterator<Item = &str> {
    url_lower.split(|c: char| !(c.is_ascii_alphanumeric() || c == '%')).filter(|t| t.len() >= 3)
}

/// Procedural operators (run by cosmetic.js, not by the style engine).
const PROCEDURAL: &[&str] = &[
    ":-abp-", ":has-text(", ":contains(", ":upward(", ":xpath(", ":matches-css", ":min-text-length(", ":watch-attr(", ":others(", ":if(", ":if-not(", ":nth-ancestor(",
    ":matches-path(", ":matches-attr(", ":matches-media(", ":matches-prop(", ":remove(", ":remove-attr(", ":remove-class(", ":shadow(",
];

fn is_procedural(sel: &str) -> bool {
    PROCEDURAL.iter().any(|e| sel.contains(e))
}

/// `sel:style(decls)` -> (sel, decls).
fn split_style(sel: &str) -> Option<(&str, &str)> {
    let i = sel.rfind(":style(")?;
    let decls = sel[i + 7..].strip_suffix(')')?;
    Some((&sel[..i], decls))
}

/// Declarations from `:style()` / `#$#` made safe to put in a style sheet: no `url()` (no
/// requests), no nesting, and every declaration `!important`.
fn safe_decls(decls: &str) -> Option<String> {
    let d = decls.trim();
    if d.is_empty() || d.contains(['{', '}', '<', '\\']) || d.to_ascii_lowercase().contains("url(") || d.contains("expression(") {
        return None;
    }
    let out: Vec<String> = d
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let s = s.trim_end_matches("!important").trim_end_matches(char::is_whitespace);
            format!("{s} !important")
        })
        .collect();
    (!out.is_empty()).then(|| out.join(";"))
}

/// The class, id or attribute a generic selector starts with (`.ad-box div` -> class "ad-box").
enum Key {
    Class(String),
    Id(String),
    Attr(String),
}

fn leading_key(sel: &str) -> Option<Key> {
    let name_of = |rest: &str| -> String { rest.chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect() };
    match sel.as_bytes().first()? {
        b'.' => Some(Key::Class(name_of(&sel[1..]))).filter(|k| !matches!(k, Key::Class(n) if n.is_empty())),
        b'#' => Some(Key::Id(name_of(&sel[1..]))).filter(|k| !matches!(k, Key::Id(n) if n.is_empty())),
        b'[' => {
            let n = name_of(&sel[1..]).to_ascii_lowercase();
            (!n.is_empty()).then_some(Key::Attr(n))
        }
        _ => {
            // tag.class / tag#id / tag[attr…] forms: use the first class, id or attribute.
            let i = sel.find(['.', '#', '['])?;
            if sel[..i].chars().all(|c| c.is_ascii_alphanumeric()) {
                return leading_key(&sel[i..]);
            }
            None
        }
    }
}

/// `!#if` conditions: what this browser counts as true (the rest, like env_firefox or
/// cap_html_filtering, is false).
fn env_true(token: &str) -> bool {
    matches!(token, "true" | "ext_ublock" | "cap_user_stylesheet" | "env_desktop")
}

/// Evaluate a `!#if` expression: tokens joined with `!`, `&&`, `||` and parentheses.
fn eval_cond(expr: &str) -> bool {
    fn or(t: &[&str], i: &mut usize) -> bool {
        let mut v = and(t, i);
        while t.get(*i) == Some(&"||") {
            *i += 1;
            let r = and(t, i);
            v = v || r;
        }
        v
    }
    fn and(t: &[&str], i: &mut usize) -> bool {
        let mut v = not(t, i);
        while t.get(*i) == Some(&"&&") {
            *i += 1;
            let r = not(t, i);
            v = v && r;
        }
        v
    }
    fn not(t: &[&str], i: &mut usize) -> bool {
        match t.get(*i) {
            Some(&"!") => {
                *i += 1;
                !not(t, i)
            }
            Some(&"(") => {
                *i += 1;
                let v = or(t, i);
                if t.get(*i) == Some(&")") {
                    *i += 1;
                }
                v
            }
            Some(tok) => {
                *i += 1;
                env_true(tok)
            }
            None => false,
        }
    }
    let spaced = expr.replace("&&", " && ").replace("||", " || ").replace('!', " ! ").replace('(', " ( ").replace(')', " ) ");
    let toks: Vec<&str> = spaced.split_whitespace().collect();
    let mut i = 0;
    or(&toks, &mut i)
}

/// Drop the lines `!#if` blocks exclude.
fn preprocess(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    // For each open `!#if`: is this branch on, and was the enclosing one on?
    let mut stack: Vec<(bool, bool)> = Vec::new();
    let mut on = true;
    for line in text.lines() {
        let l = line.trim();
        if let Some(cond) = l.strip_prefix("!#if ") {
            stack.push((on, eval_cond(cond)));
            on = on && eval_cond(cond);
        } else if l == "!#else" {
            if let Some((outer, cond)) = stack.last() {
                on = *outer && !*cond;
            }
        } else if l == "!#endif" {
            on = stack.pop().map_or(true, |(outer, _)| outer);
        } else if on {
            out.push(l);
        }
    }
    out
}

/// Split a `+js(name, arg, …)` call into its parts (`\,` is a comma inside an argument;
/// quotes around an argument are taken off).
pub fn split_call(call: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut chars = call.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&',') => {
                cur.push(',');
                chars.next();
            }
            ',' => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts
        .into_iter()
        .map(|p| {
            let p = p.trim();
            for q in ['"', '\'', '`'] {
                if p.len() >= 2 && p.starts_with(q) && p.ends_with(q) {
                    return p[1..p.len() - 1].to_string();
                }
            }
            p.to_string()
        })
        .collect()
}

/// Scriptlets only trusted lists may use: they can change a page's code and data at will.
fn trusted_only(name: &str) -> bool {
    let n = name.trim_end_matches(".js");
    n.starts_with("trusted-") || matches!(n, "rpnt" | "replace-node-text")
}

/// A scriptlet's text argument: `/regex/flags` or plain text.
enum TextPattern {
    Re(regress::Regex, bool),
    Plain(String),
}

fn text_pattern(s: &str) -> Option<TextPattern> {
    if s.len() > 2 && s.starts_with('/') {
        if let Some(end) = s.rfind('/').filter(|&e| e > 0) {
            let flags = &s[end + 1..];
            if flags.chars().all(|c| "gimsu".contains(c)) {
                let re_flags: String = flags.chars().filter(|c| *c != 'g').collect();
                return regress::Regex::with_flags(&s[1..end], re_flags.as_str()).ok().map(|r| TextPattern::Re(r, flags.contains('g')));
            }
        }
    }
    (!s.is_empty()).then(|| TextPattern::Plain(s.to_string()))
}

impl TextPattern {
    fn test(&self, text: &str) -> bool {
        match self {
            TextPattern::Re(r, _) => r.find(text).is_some(),
            TextPattern::Plain(p) => text.contains(p.as_str()),
        }
    }

    /// Replace the first match (every match for a `/…/g` regex or plain text, as
    /// String.prototype.replace / replaceAll would), with `$1`, `$&` and `$$` in `with`.
    fn replace(&self, text: &str, with: &str) -> String {
        match self {
            TextPattern::Plain(p) => text.replacen(p.as_str(), with, 1),
            TextPattern::Re(r, global) => {
                let mut out = String::with_capacity(text.len());
                let mut last = 0;
                for m in r.find_iter(text) {
                    out.push_str(&text[last..m.start()]);
                    let w = with.as_bytes();
                    let mut i = 0;
                    while i < w.len() {
                        if w[i] == b'$' && i + 1 < w.len() {
                            let n = w[i + 1];
                            if n == b'$' {
                                out.push('$');
                                i += 2;
                                continue;
                            }
                            if n == b'&' {
                                out.push_str(&text[m.range()]);
                                i += 2;
                                continue;
                            }
                            if n.is_ascii_digit() {
                                let g = (n - b'0') as usize;
                                if let Some(range) = m.group(g) {
                                    out.push_str(&text[range]);
                                }
                                i += 2;
                                continue;
                            }
                        }
                        let ch = with[i..].chars().next().unwrap_or('$');
                        out.push(ch);
                        i += ch.len_utf8();
                    }
                    last = m.end();
                    if !global {
                        break;
                    }
                }
                out.push_str(&text[last..]);
                out
            }
        }
    }
}

impl Engine {
    pub fn add_list(&mut self, text: &str, kind: Kind) {
        self.add_list_from(text, kind, false);
    }

    /// Add a list; `trusted`: it may use the scriptlets that can run arbitrary edits.
    pub fn add_list_from(&mut self, text: &str, kind: Kind, trusted: bool) {
        self.loading_trusted = trusted;
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
        for line in preprocess(text) {
            self.add_rule(line);
        }
        self.finish();
    }

    /// After lists are added: apply `$badfilter` and count the rules.
    fn finish(&mut self) {
        if !self.badfilters.is_empty() {
            for idx in [&mut self.net, &mut self.mods, &mut self.page_rules] {
                for r in idx.rules.iter_mut() {
                    if self.badfilters.contains(&r.text) {
                        r.disabled = true;
                    }
                }
            }
        }
        let cosmetic: usize = self.hide_on.values().map(Vec::len).sum::<usize>() + self.procedural_on.values().map(Vec::len).sum::<usize>() + self.scriptlets_on.values().map(Vec::len).sum::<usize>();
        let generic: usize = self.hide_by_class.values().chain(self.hide_by_id.values()).chain(self.hide_by_attr.values()).map(Vec::len).sum::<usize>() + self.hide_always.len();
        self.rule_count = self.net.rules.len() + self.mods.rules.len() + self.page_rules.rules.len() + cosmetic + generic;
    }

    fn add_cosmetic(&mut self, domains: &str, sep: &str, body: &str) {
        let exception = sep.contains('@');
        if domains.contains(['/', '|', '$', '"', '\'']) {
            return;
        }
        let doms: Vec<String> = domains.split(',').map(str::trim).filter(|d| !d.is_empty() && *d != "*").map(|d| d.to_ascii_lowercase()).collect();
        let positive: Vec<String> = doms.iter().filter(|d| !d.starts_with('~')).cloned().collect();
        let negative: Vec<String> = doms.iter().filter_map(|d| d.strip_prefix('~')).map(String::from).collect();

        // Scriptlets: `##+js(…)` (only for named sites, as in uBlock Origin).
        if let Some(call) = body.strip_prefix("+js(").and_then(|c| c.strip_suffix(')')) {
            let call = call.trim().to_string();
            if exception {
                let sites = if positive.is_empty() { vec![String::new()] } else { positive };
                for d in sites {
                    self.unscriptlet.insert((d, call.clone()));
                }
            } else if !positive.is_empty() && !call.is_empty() {
                for d in &negative {
                    self.unscriptlet.insert((d.clone(), call.clone()));
                }
                for d in positive {
                    self.scriptlets_on.entry(d).or_default().push((call.clone(), self.loading_trusted));
                }
            }
            return;
        }
        // HTML filters (`##^…`): inline scripts by their text are dropped before they run;
        // other elements are removed as the page loads.
        if let Some(html) = body.strip_prefix('^') {
            if exception || positive.is_empty() || html.starts_with("responseheader(") {
                return;
            }
            if let Some(needle) = html.strip_prefix("script:has-text(").and_then(|r| r.strip_suffix(')')) {
                for d in positive {
                    self.script_removals_on.entry(d).or_default().push(needle.to_string());
                }
            } else if !html.is_empty() {
                for d in positive {
                    self.html_filters_on.entry(d).or_default().push(html.to_string());
                }
            }
            return;
        }
        if body.starts_with('+') || body.is_empty() {
            return;
        }
        // AdGuard's `#$#sel { decls }`: the same as `sel:style(decls)`.
        let adguard;
        let body = if sep.contains('$') {
            let Some((sel, rest)) = body.split_once('{') else { return };
            let Some(decls) = rest.trim().strip_suffix('}') else { return };
            adguard = format!("{}:style({})", sel.trim(), decls.trim());
            adguard.as_str()
        } else {
            body
        };
        let procedural = sep.contains('?') || is_procedural(body) || body.ends_with(":remove()") || split_style(body).is_some_and(|(s, _)| is_procedural(s));

        if exception {
            let sites = if positive.is_empty() { vec![String::new()] } else { positive };
            for d in sites {
                self.unhide.insert((d, body.to_string()));
            }
            return;
        }
        if positive.is_empty() {
            // Generic: plain selectors only (styles and procedural filters need a site).
            if procedural || body.contains(":style(") {
                return;
            }
            for d in &negative {
                self.unhide.insert((d.clone(), body.to_string()));
            }
            let sel = body.to_string();
            match leading_key(body) {
                Some(Key::Class(k)) => self.hide_by_class.entry(k).or_default().push(sel),
                Some(Key::Id(k)) => self.hide_by_id.entry(k).or_default().push(sel),
                Some(Key::Attr(k)) => self.hide_by_attr.entry(k).or_default().push(sel),
                None => self.hide_always.push(sel),
            }
            return;
        }
        for d in &negative {
            self.unhide.insert((d.clone(), body.to_string()));
        }
        if procedural {
            for d in positive {
                self.procedural_on.entry(d).or_default().push(body.to_string());
            }
            return;
        }
        let item = match split_style(body) {
            Some((sel, decls)) => match safe_decls(decls) {
                Some(style) if !sel.trim().is_empty() => Cosmetic { sel: sel.trim().to_string(), style: Some(style) },
                _ => return,
            },
            None => Cosmetic { sel: body.to_string(), style: None },
        };
        for d in positive {
            self.hide_on.entry(d).or_default().push(item.clone());
        }
    }

    fn add_rule(&mut self, line: &str) {
        if line.is_empty() || line.starts_with('!') || (line.starts_with('[') && line.ends_with(']') && line.contains("Adblock")) {
            return;
        }
        // Cosmetic rules: [domains]##selector and the other separators (the first `#` that
        // starts one). A `#` in a network rule's pattern (`||site.com/page#x`) comes after a
        // `/`, `|`, `$` or `^`, which a list of domains never has.
        const SEPS: &[&str] = &["#@$?#", "#@?#", "#@$#", "#@#", "#$?#", "#?#", "#$#", "#%#", "##"];
        for (i, _) in line.match_indices('#') {
            let Some(sep) = SEPS.iter().find(|s| line[i..].starts_with(*s)) else { continue };
            let (domains, body) = (&line[..i], line[i + sep.len()..].trim());
            if domains.contains(['/', '|', '$', '^', '=', '?', '&']) {
                break;
            }
            // `#%#` (AdGuard scripts) and `#$?#` aren't supported; `#@$?#` is their exception.
            if !matches!(*sep, "#%#" | "#$?#" | "#@$?#") {
                self.add_cosmetic(domains, sep, body);
            }
            return;
        }
        // Hosts-file lines in filter lists: "0.0.0.0 ads.example".
        if let Some(h) = line.strip_prefix("0.0.0.0 ").or_else(|| line.strip_prefix("127.0.0.1 ")) {
            let h = h.split('#').next().unwrap_or("").trim();
            if h.contains('.') && !h.contains(char::is_whitespace) && h != "0.0.0.0" {
                self.add_rule(&format!("||{h}^"));
            }
            return;
        }
        let (exception, body) = match line.strip_prefix("@@") {
            Some(b) => (true, b),
            None => (false, line),
        };
        // Options start at the last `$` (a `$` can appear inside a pattern or a regex).
        let is_regex_pat = |p: &str| p.starts_with('/') && p.ends_with('/') && p.len() > 2;
        let (pat, opts) = match body.rfind('$') {
            Some(i) if (!body[i + 1..].contains('/') || body[i + 1..].contains("domain=") || body[i + 1..].contains("removeparam=") || body[i + 1..].contains("csp=")) && !(is_regex_pat(body) && !is_regex_pat(&body[..i])) => {
                (&body[..i], &body[i + 1..])
            }
            _ => (body, ""),
        };
        let Some(parsed) = (if opts.is_empty() { Some(Parsed { opts: Opts { types: T_ALL, ..Default::default() }, ..Default::default() }) } else { parse_opts(opts, exception) }) else { return };
        if parsed.badfilter {
            // The rule it turns off: the same text without the badfilter option.
            let kept: Vec<&str> = opts.split(',').filter(|o| o.trim() != "badfilter").collect();
            let target = if kept.is_empty() { format!("{}{pat}", if exception { "@@" } else { "" }) } else { format!("{}{pat}${}", if exception { "@@" } else { "" }, kept.join(",")) };
            self.badfilters.insert(target);
            return;
        }
        let mut regex = None;
        let mut regex_lit = None;
        let mut p = pat;
        let mut host = None;
        let mut anchor_start = false;
        if is_regex_pat(pat) {
            let flags = if parsed.opts.match_case { "" } else { "i" };
            match regress::Regex::with_flags(&pat[1..pat.len() - 1], flags) {
                Ok(r) => {
                    regex = Some(r);
                    regex_lit = regex_literal(&pat[1..pat.len() - 1]);
                }
                Err(_) => return,
            }
            p = "";
        } else if let Some(rest) = p.strip_prefix("||") {
            let end = rest.find(['^', '/', '*', ':', '?', '|']).unwrap_or(rest.len());
            let h = &rest[..end];
            if !h.is_empty() && !h.contains('*') {
                host = Some(h.trim_end_matches('.').to_ascii_lowercase());
                p = &rest[end..];
            } else {
                p = rest;
            }
        } else if let Some(rest) = p.strip_prefix('|') {
            anchor_start = true;
            p = rest;
        }
        let anchor_end = p.ends_with('|') && regex.is_none();
        let p = p.trim_end_matches('|');
        // An unanchored pattern may start anywhere: a leading `*` adds nothing.
        let p = if host.is_none() && !anchor_start { p.trim_start_matches('*') } else { p };
        let pattern = if parsed.opts.match_case { p.to_string() } else { p.to_ascii_lowercase() };
        let action = parsed.action.unwrap_or(Action::Block);
        if parsed.page != 0 {
            // A page-wide exception (`@@||site^$document` and the like).
            let rule = Rule { host, pattern, regex, regex_lit, anchor_start, anchor_end, opts: parsed.opts, exception, action, value: parsed.value, disabled: false, page: parsed.page, text: line.to_string() };
            self.page_rules.add(rule);
            return;
        }
        // A pattern that would match every request is only allowed when options narrow it.
        let narrowed = !parsed.opts.on.is_empty() || !parsed.opts.to_on.is_empty() || parsed.opts.explicit_types || action != Action::Block || exception;
        if host.is_none() && regex.is_none() && pattern.trim_matches('*').is_empty() && !narrowed {
            return;
        }
        let rule = Rule { host, pattern, regex, regex_lit, anchor_start, anchor_end, opts: parsed.opts, exception, action, value: parsed.value, disabled: false, page: 0, text: line.to_string() };
        match action {
            Action::Block | Action::Redirect => self.net.add(rule),
            Action::RemoveParam | Action::Csp => self.mods.add(rule),
        }
    }

    /// Page-wide exception flags for a page.
    fn page_flags(&self, page: &str, page_host: &str) -> u8 {
        if self.page_rules.rules.is_empty() {
            return 0;
        }
        let lower = page.to_ascii_lowercase();
        let keys = page_keys(page_host);
        self.page_rules
            .candidates(page_host, &lower, &keys)
            .filter(|r| (r.opts.on.is_empty() || r.applies(r.opts.types, false, page_host, page_host)) && r.matches_url(page, &lower, page_host))
            .fold(0, |a, r| a | r.page)
    }

    /// The rule that blocks this request, if any (tests and the fuzz target).
    #[allow(dead_code)]
    pub fn check(&self, url: &str, page: &str, dest: Dest) -> Option<&str> {
        self.decide(url, page, type_bit(dest)).map(|(rule, _)| rule)
    }

    /// Should this request be blocked? The blocking rule, and the stand-in to answer with
    /// (a `$redirect` resource name), if any.
    fn decide(&self, url: &str, page: &str, ty: u16) -> Option<(&str, Option<&str>)> {
        let host = host_of(url)?;
        let page_host = host_of(page).unwrap_or_default();
        let flags = self.page_flags(page, &page_host);
        if flags & PAGE_ALL != 0 {
            return None;
        }
        let third = crate::cookies::site_of(&host) != crate::cookies::site_of(&page_host);
        let lower = url.to_ascii_lowercase();
        let q = Req { url, lower: &lower, host: &host, page_host: &page_host, third, ty };
        let (mut block, mut important, mut allow, mut allow_important) = (None::<&Rule>, None::<&Rule>, false, false);
        let mut redirects: Vec<&Rule> = Vec::new();
        let mut no_redirect: Vec<&str> = Vec::new();
        let keys = page_keys(&page_host);
        for r in self.net.candidates(&host, &lower, &keys) {
            if flags & PAGE_GBLOCK != 0 && !r.exception && r.opts.on.is_empty() {
                continue;
            }
            if !r.hits(&q) {
                continue;
            }
            match (r.action, r.exception) {
                (Action::Block, false) => {
                    if r.opts.important {
                        important.get_or_insert(r);
                    } else {
                        block.get_or_insert(r);
                    }
                    if !r.value.is_empty() {
                        redirects.push(r);
                    }
                }
                (Action::Block, true) => {
                    allow = true;
                    allow_important |= r.opts.important;
                }
                (Action::Redirect, false) => redirects.push(r),
                (Action::Redirect, true) => no_redirect.push(&r.value),
                _ => {}
            }
        }
        let rule = if allow_important {
            None
        } else if important.is_some() {
            important
        } else if allow {
            None
        } else {
            block
        }?;
        let redirect = if no_redirect.iter().any(|v| v.is_empty()) {
            None
        } else {
            // The first stand-in that isn't excepted, preferring an important rule's.
            redirects.sort_by_key(|r| !r.opts.important);
            redirects.iter().map(|r| r.value.as_str()).find(|v| !no_redirect.contains(v) && resource(v, ty).is_some())
        };
        Some((&rule.text, redirect))
    }

    /// Would opening this page be blocked outright (`||site^`, `$document`)?
    fn doc_blocked(&self, url: &str) -> Option<&str> {
        let host = host_of(url)?;
        let flags = self.page_flags(url, &host);
        if flags & PAGE_ALL != 0 {
            return None;
        }
        let lower = url.to_ascii_lowercase();
        let q = Req { url, lower: &lower, host: &host, page_host: &host, third: false, ty: T_DOC };
        let mut hit = None;
        let keys = page_keys(&host);
        for r in self.net.candidates(&host, &lower, &keys) {
            if r.action != Action::Block {
                continue;
            }
            let doc = r.opts.types & T_DOC != 0 || r.whole_site();
            if !doc {
                continue;
            }
            let q = Req { ty: if r.opts.types & T_DOC != 0 { T_DOC } else { r.opts.types & T_ALL }, ..q };
            if !r.hits(&q) {
                continue;
            }
            if r.exception {
                return None;
            }
            hit.get_or_insert(r.text.as_str());
        }
        hit
    }

    /// Is a popup (window.open) to `url` from `page` blocked?
    fn popup_blocked(&self, url: &str, page: &str) -> Option<&str> {
        let host = host_of(url)?;
        let page_host = host_of(page).unwrap_or_default();
        if self.page_flags(page, &page_host) & PAGE_ALL != 0 {
            return None;
        }
        let lower = url.to_ascii_lowercase();
        let third = crate::cookies::site_of(&host) != crate::cookies::site_of(&page_host);
        let q = Req { url, lower: &lower, host: &host, page_host: &page_host, third, ty: T_POPUP };
        let mut hit = None;
        let keys = page_keys(&page_host);
        for r in self.net.candidates(&host, &lower, &keys).filter(|r| r.action == Action::Block && r.hits(&q)) {
            if r.exception {
                return None;
            }
            hit.get_or_insert(r.text.as_str());
        }
        hit
    }

    /// The address with the parameters `$removeparam` rules strip taken out (None = unchanged).
    fn remove_params(&self, url: &str, page: &str, ty: u16) -> Option<String> {
        let (before_frag, frag) = match url.split_once('#') {
            Some((a, f)) => (a, Some(f)),
            None => (url, None),
        };
        let (base, query) = before_frag.split_once('?')?;
        let host = host_of(url)?;
        let page_host = host_of(page).unwrap_or_default();
        if self.page_flags(page, &page_host) & PAGE_ALL != 0 {
            return None;
        }
        let lower = url.to_ascii_lowercase();
        let third = crate::cookies::site_of(&host) != crate::cookies::site_of(&page_host);
        let q = Req { url, lower: &lower, host: &host, page_host: &page_host, third, ty };
        let mut strip: Vec<&str> = Vec::new();
        let mut keep: Vec<&str> = Vec::new();
        let keys = page_keys(&page_host);
        for r in self.mods.candidates(&host, &lower, &keys).filter(|r| r.action == Action::RemoveParam && r.hits(&q)) {
            if r.exception { keep.push(&r.value) } else { strip.push(&r.value) }
        }
        if keep.iter().any(|v| v.is_empty()) {
            return None;
        }
        strip.retain(|v| !keep.contains(v));
        if strip.is_empty() {
            return None;
        }
        let res: Vec<(bool, Option<regress::Regex>, &str)> = strip
            .iter()
            .map(|v| {
                let (neg, v) = match v.strip_prefix('~') {
                    Some(v) => (true, v),
                    None => (false, *v),
                };
                if v.len() > 2 && v.starts_with('/') {
                    let end = v.rfind('/').unwrap_or(0);
                    let flags: String = v[end + 1..].chars().filter(|c| *c == 'i').collect();
                    (neg, regress::Regex::with_flags(&v[1..end.max(1)], flags.as_str()).ok(), v)
                } else {
                    (neg, None, v)
                }
            })
            .collect();
        let kept: Vec<&str> = query
            .split('&')
            .filter(|kv| {
                let name = kv.split('=').next().unwrap_or("");
                !res.iter().any(|(neg, re, v)| {
                    let m = match (re, *v) {
                        (Some(re), _) => re.find(kv).is_some(),
                        (None, "") => true,
                        (None, v) if v.starts_with('/') => false, // a regex that didn't compile
                        (None, v) => name == v,
                    };
                    m != *neg
                })
            })
            .collect();
        if kept.len() == query.split('&').count() {
            return None;
        }
        let mut out = base.to_string();
        if !kept.is_empty() {
            out.push('?');
            out.push_str(&kept.join("&"));
        }
        if let Some(f) = frag {
            out.push('#');
            out.push_str(f);
        }
        Some(out)
    }

    /// Extra Content Security Policies `$csp` rules add to this page (or frame).
    fn csp_for(&self, url: &str, parent: &str) -> Vec<String> {
        let Some(host) = host_of(url) else { return Vec::new() };
        let page_host = host_of(parent).unwrap_or_else(|| host.clone());
        if self.page_flags(url, &host) & PAGE_ALL != 0 {
            return Vec::new();
        }
        let lower = url.to_ascii_lowercase();
        let third = crate::cookies::site_of(&host) != crate::cookies::site_of(&page_host);
        let q = Req { url, lower: &lower, host: &host, page_host: &page_host, third, ty: if url == parent { T_DOC } else { T_FRAME } };
        let (mut add, mut keep): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        let keys = page_keys(&page_host);
        for r in self.mods.candidates(&host, &lower, &keys).filter(|r| r.action == Action::Csp && r.hits(&q)) {
            if r.exception { keep.push(&r.value) } else { add.push(&r.value) }
        }
        if keep.iter().any(|v| v.is_empty()) {
            return Vec::new();
        }
        let mut out: Vec<String> = add.into_iter().filter(|v| !v.is_empty() && !keep.contains(v)).map(String::from).collect();
        out.dedup();
        out
    }

    #[allow(dead_code)]
    /// CSS that hides ad leftovers in this page: site-specific rules, plus generic rules whose
    /// class, id or attribute actually appears in the page.
    pub fn hiding_css(&self, page: &str, classes: &HashSet<String>, ids: &HashSet<String>) -> String {
        self.hiding_css_with(page, classes, ids, &HashSet::new())
    }

    fn hiding_css_with(&self, page: &str, classes: &HashSet<String>, ids: &HashSet<String>, attrs: &HashSet<String>) -> String {
        let page_host = host_of(page).unwrap_or_default();
        if page_host.is_empty() {
            return String::new();
        }
        let flags = self.page_flags(page, &page_host);
        if flags & PAGE_ALL != 0 {
            return String::new();
        }
        let keys = page_keys(&page_host);
        let unhidden = |s: &str| self.unhide.contains(&(String::new(), s.to_string())) || keys.iter().any(|d| self.unhide.contains(&(d.clone(), s.to_string())));
        let mut seen = HashSet::new();
        let mut css = String::new();
        if flags & PAGE_GHIDE == 0 {
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
            for a in attrs {
                if let Some(v) = self.hide_by_attr.get(a) {
                    sels.extend(v.iter().map(String::as_str));
                }
            }
            sels.extend(self.hide_always.iter().map(String::as_str));
            for s in sels {
                if seen.insert(s.to_string()) && !unhidden(s) {
                    // One rule per selector: a selector we can't parse then drops only itself.
                    css.push_str(s);
                    css.push_str("{display:none!important}\n");
                }
            }
        }
        if flags & PAGE_SHIDE == 0 {
            for d in &keys {
                for c in self.hide_on.get(d).into_iter().flatten() {
                    let raw = match &c.style {
                        Some(st) => format!("{}:style({})", c.sel, st),
                        None => c.sel.clone(),
                    };
                    if !seen.insert(raw.clone()) || unhidden(&raw) || (c.style.is_none() && unhidden(&c.sel)) {
                        continue;
                    }
                    css.push_str(&c.sel);
                    match &c.style {
                        Some(st) => {
                            css.push('{');
                            css.push_str(st);
                            css.push_str("}\n");
                        }
                        None => css.push_str("{display:none!important}\n"),
                    }
                }
            }
        }
        css
    }

    /// Procedural cosmetic rules for this page (run by cosmetic.js).
    fn procedural_for(&self, page: &str) -> Vec<&str> {
        let Some(page_host) = host_of(page) else { return Vec::new() };
        if self.page_flags(page, &page_host) & (PAGE_ALL | PAGE_SHIDE) != 0 {
            return Vec::new();
        }
        let keys = page_keys(&page_host);
        let unhidden = |s: &str| self.unhide.contains(&(String::new(), s.to_string())) || keys.iter().any(|d| self.unhide.contains(&(d.clone(), s.to_string())));
        let mut out: Vec<&str> = keys.iter().filter_map(|k| self.procedural_on.get(k)).flatten().map(String::as_str).filter(|s| !unhidden(s)).collect();
        out.dedup();
        out
    }

    /// Scriptlet calls for this page, each split into name and arguments.
    fn scriptlets_for(&self, page: &str) -> Vec<Vec<String>> {
        let Some(page_host) = host_of(page) else { return Vec::new() };
        if self.page_flags(page, &page_host) & PAGE_ALL != 0 {
            return Vec::new();
        }
        let keys = page_keys(&page_host);
        // Turned off on this site: every scriptlet (`#@#+js()`), or this one.
        let all_off = keys.iter().any(|d| self.unscriptlet.contains(&(d.clone(), String::new())));
        let off = |c: &str| all_off || [String::new()].iter().chain(keys.iter()).any(|d| self.unscriptlet.contains(&(d.clone(), c.to_string())));
        let mut seen = HashSet::new();
        keys.iter()
            .filter_map(|k| self.scriptlets_on.get(k))
            .flatten()
            .filter(|(c, _)| seen.insert(c.as_str()) && !off(c))
            .map(|(c, trusted)| (split_call(c), *trusted))
            .filter(|(parts, trusted)| parts.first().is_some_and(|n| !n.is_empty() && (*trusted || !trusted_only(n))))
            .map(|(parts, _)| parts)
            .collect()
    }

    /// HTML filters for this page (selectors, each maybe ending in `:has-text(…)`).
    fn html_filters_for(&self, page: &str) -> Vec<String> {
        let Some(page_host) = host_of(page) else { return Vec::new() };
        if self.page_flags(page, &page_host) & PAGE_ALL != 0 {
            return Vec::new();
        }
        let keys = page_keys(&page_host);
        let unhidden = |s: &str| keys.iter().chain([String::new()].iter()).any(|d| self.unhide.contains(&(d.clone(), format!("^{s}"))));
        let mut out: Vec<String> = keys.iter().filter_map(|k| self.html_filters_on.get(k)).flatten().filter(|s| !unhidden(s)).cloned().collect();
        out.dedup();
        out
    }

    /// What an inline script should become before it runs: None = don't run it (an HTML
    /// filter removes it), else its text with the `rpnt` edits filters make to scripts.
    fn filter_script(&self, page: &str, code: &str) -> Option<String> {
        let Some(page_host) = host_of(page) else { return Some(code.to_string()) };
        if self.page_flags(page, &page_host) & PAGE_ALL != 0 {
            return Some(code.to_string());
        }
        let keys = page_keys(&page_host);
        let unhidden = |s: &str| keys.iter().chain([String::new()].iter()).any(|d| self.unhide.contains(&(d.clone(), format!("^script:has-text({s})"))));
        for needle in keys.iter().filter_map(|k| self.script_removals_on.get(k)).flatten() {
            if text_pattern(needle).is_some_and(|p| p.test(code)) && !unhidden(needle) {
                log!("[content blocking] {page}: dropped an inline script (##^script:has-text({needle}))");
                return None;
            }
        }
        let mut out = code.to_string();
        for call in self.scriptlets_for(page) {
            let name = call[0].trim_end_matches(".js");
            if !matches!(name, "rpnt" | "replace-node-text" | "trusted-replace-node-text" | "trusted-rpnt") || call.get(1).map(String::as_str) != Some("script") {
                continue;
            }
            let (Some(find), Some(with)) = (call.get(2), call.get(3)) else { continue };
            // Extra arguments come in name/value pairs: `condition` limits it to scripts that match.
            let extra: HashMap<&str, &str> = call[4..].chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].as_str(), c[1].as_str())).collect();
            if let Some(cond) = extra.get("condition") {
                if !text_pattern(cond).is_some_and(|p| p.test(&out)) {
                    continue;
                }
            }
            if let Some(p) = text_pattern(find) {
                out = p.replace(&out, with);
            }
        }
        Some(out)
    }

    /// Is this host (or a parent domain) on a malware or phishing list?
    pub fn is_dangerous(&self, url: &str) -> bool {
        let Some(host) = host_of(url) else { return false };
        suffixes(&host).any(|h| self.dangerous.contains(h))
    }
}

// ---------------------------------------------------------------- stand-in resources

/// A blank 1×1 GIF.
const GIF_1X1: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
];

/// A transparent 1×1 PNG.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

/// Stand-in scripts for ad and analytics libraries (our own, in redirects.js): sections
/// start with `//@ name alias alias…`.
const REDIRECT_JS: &str = include_str!("redirects.js");

const VAST: &str = r#"<?xml version="1.0" encoding="UTF-8"?><VAST version="3.0"></VAST>"#;
const VMAP: &str = r#"<?xml version="1.0" encoding="UTF-8"?><vmap:VMAP xmlns:vmap="http://www.iab.net/videosuite/vmap" version="1.0"></vmap:VMAP>"#;

/// A stand-in by its uBlock Origin name (or alias): (content type, body).
fn resource(name: &str, ty: u16) -> Option<(&'static str, Vec<u8>)> {
    let name = name.trim();
    let r: (&'static str, Vec<u8>) = match name {
        "1x1.gif" | "1x1-transparent.gif" | "abp-resource:1x1-transparent-gif" => ("image/gif", GIF_1X1.to_vec()),
        "2x2.png" | "2x2-transparent.png" | "abp-resource:2x2-transparent-png" | "3x2.png" | "3x2-transparent.png" | "abp-resource:3x2-transparent-png" | "32x32.png" | "32x32-transparent.png"
        | "abp-resource:32x32-transparent-png" => ("image/png", PNG_1X1.to_vec()),
        "noop.css" | "abp-resource:blank-css" => ("text/css", Vec::new()),
        "noop.html" | "noopframe" | "abp-resource:blank-html" | "click2load.html" => ("text/html", b"<!DOCTYPE html><html><head></head><body></body></html>".to_vec()),
        "noop.js" | "noopjs" | "abp-resource:blank-js" | "blank-js" => ("text/javascript", b"(function(){})();".to_vec()),
        "noop.json" | "noopjson" => ("application/json", b"{}".to_vec()),
        "noop.txt" | "nooptext" | "abp-resource:blank-text" => ("text/plain", Vec::new()),
        "noop-0.1s.mp3" | "noopmp3-0.1s" | "abp-resource:blank-mp3" | "noop-0.5s.mp3" => ("audio/mpeg", Vec::new()),
        "noop-1s.mp4" | "noopmp4-1s" | "abp-resource:blank-mp4" => ("video/mp4", Vec::new()),
        "noop-vast2.xml" | "noopvast-2.0" | "noop-vast3.xml" | "noopvast-3.0" | "noop-vast4.xml" | "noopvast-4.0" => ("text/xml", VAST.as_bytes().to_vec()),
        "noop-vmap1.xml" | "noop-vmap1.0.xml" | "noopvmap-1.0" => ("text/xml", VMAP.as_bytes().to_vec()),
        "empty" => {
            return match ty {
                T_SCRIPT => resource("noop.js", ty),
                T_STYLE => resource("noop.css", ty),
                T_IMAGE => resource("1x1.gif", ty),
                T_FRAME | T_DOC => resource("noop.html", ty),
                T_MEDIA => resource("noop-1s.mp4", ty),
                _ => resource("noop.txt", ty),
            };
        }
        _ => {
            let code = redirect_script(name)?;
            ("text/javascript", code.into_bytes())
        }
    };
    Some(r)
}

/// The stand-in script named `name` from redirects.js.
fn redirect_script(name: &str) -> Option<String> {
    let mut out: Option<String> = None;
    for line in REDIRECT_JS.lines() {
        if let Some(names) = line.strip_prefix("//@ ") {
            if out.is_some() {
                break;
            }
            if names.split_whitespace().any(|n| n == name) {
                out = Some(String::new());
            }
        } else if let Some(o) = out.as_mut() {
            o.push_str(line);
            o.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------- the shared engine

static ENGINE: RwLock<Option<Arc<Engine>>> = RwLock::new(None);
/// Hosts you chose to visit anyway after the warning (until Ironswift closes).
static PROCEED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn dir() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("filters"))
}

fn list_on(kind: Kind, s: &crate::settings::Settings) -> bool {
    match kind {
        Kind::Ads => s.block_ads,
        Kind::Trackers => s.block_trackers,
        Kind::Badware | Kind::Dangerous => s.safe_browsing,
        Kind::Annoyances => s.block_annoyances,
    }
}

fn build() -> Engine {
    let s = crate::settings::get();
    let mut e = Engine::default();
    let dir = dir();
    let read = |name: &str| dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(name)).ok());
    let mut have_full = false;
    for (name, _, kind) in LISTS {
        if let (true, Some(text)) = (list_on(*kind, &s), read(name)) {
            e.add_list_from(&text, *kind, name.starts_with("ublock-"));
            have_full |= matches!(kind, Kind::Ads | Kind::Trackers);
        }
    }
    if !have_full && (s.block_ads || s.block_trackers) {
        e.add_list(BUILT_IN, Kind::Trackers);
    }
    if let Some(text) = read("custom.txt") {
        e.add_list_from(&text, Kind::Ads, true);
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

fn web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Sites where you turned blocking off.
fn exempt(page: &str) -> bool {
    let Some(h) = host_of(page) else { return false };
    let list = crate::settings::get().no_blocking;
    list.iter().any(|d| domain_matches(&h, d))
}

/// Why a page's request should be stopped (None = let it through).
pub fn blocked(url: &str, page: &str, dest: Dest) -> Option<String> {
    decide(url, page, dest).map(|b| b.why)
}

/// What to do with a request that's blocked.
pub struct Blocked {
    pub why: String,
    /// A stand-in to answer with instead of an error: (content type, body).
    pub stand_in: Option<(&'static str, Vec<u8>)>,
}

/// The requests blocked for each top-level page (by its address); each address counts once.
static COUNTS: Mutex<Option<HashMap<String, HashSet<String>>>> = Mutex::new(None);

fn count_blocked(url: &str, page: &str) {
    let top = crate::security::frame_info(page).map(|f| f.top).unwrap_or_else(|| page.to_string());
    if let Ok(mut c) = COUNTS.lock() {
        let map = c.get_or_insert_with(HashMap::new);
        if map.len() > 200 {
            map.clear();
        }
        map.entry(top).or_default().insert(url.to_string());
    }
}

/// Requests blocked so far for this page (and its frames).
pub fn blocked_count(page: &str) -> usize {
    COUNTS.lock().ok().and_then(|c| c.as_ref().and_then(|m| m.get(page).map(HashSet::len))).unwrap_or(0)
}

/// A page is loading afresh: start its count again.
pub fn reset_count(page: &str) {
    if let Ok(mut c) = COUNTS.lock() {
        if let Some(m) = c.as_mut() {
            m.remove(page);
        }
    }
}

/// For the toolbar's shield: None for pages blocking doesn't apply to (about:, file:…), else
/// whether blocking is on for this page's site.
pub fn blocking_on(page: &str) -> Option<bool> {
    let s = crate::settings::get();
    (web(page) && (s.block_ads || s.block_trackers)).then(|| !exempt(page))
}

/// The site the shield menu turns blocking on or off for (example.com for www.example.com).
pub fn site_for(page: &str) -> Option<String> {
    host_of(page).map(|h| crate::cookies::site_of(&h))
}

/// Turn blocking on or off for a page's site.
pub fn set_blocking_on(page: &str, on: bool) {
    let (Some(host), Some(site)) = (host_of(page), site_for(page)) else { return };
    let mut list = crate::settings::get().no_blocking;
    // On: every entry that covers this host goes; off: the site is added.
    list.retain(|d| !domain_matches(&host, d));
    if !on {
        list.push(site);
    }
    crate::settings::set_no_blocking(list);
}

/// Should a page's request be stopped, and answered with a stand-in?
pub fn decide(url: &str, page: &str, dest: Dest) -> Option<Blocked> {
    if !web(url) || !web(page) || exempt(page) {
        return None;
    }
    let e = engine();
    let ty = type_bit(dest);
    let (rule, redirect) = e.decide(url, page, ty)?;
    let stand_in = redirect.and_then(|r| resource(r, ty));
    count_blocked(url, page);
    match &stand_in {
        Some(_) => log!("[blocked] {url} ({rule}, answered with {})", redirect.unwrap_or("")),
        None => log!("[blocked] {url} ({rule})"),
    }
    Some(Blocked { why: format!("blocked by filter rule {rule}"), stand_in })
}

/// The address of a request (or `page` itself, for a navigation) with the tracking
/// parameters `$removeparam` rules name taken out.
pub fn clean_url(url: &str, page: &str, dest: Option<Dest>) -> String {
    if !web(url) || !web(page) || exempt(page) || !url.contains('?') {
        return url.to_string();
    }
    let ty = dest.map_or(T_DOC, type_bit);
    match engine().remove_params(url, page, ty) {
        Some(clean) => {
            log!("[removeparam] {url} -> {clean}");
            clean
        }
        None => url.to_string(),
    }
}

/// Content Security Policies filter lists add to a page or frame (`$csp`). `parent`: the
/// page a frame is in (the page itself for a top-level page).
pub fn extra_csp(url: &str, parent: &str) -> Vec<String> {
    if !web(url) || exempt(if web(parent) { parent } else { url }) {
        return Vec::new();
    }
    engine().csp_for(url, parent)
}

/// Should window.open(url) from this page be stopped (`$popup` rules)?
pub fn popup_blocked(url: &str, page: &str) -> bool {
    if !web(url) || !web(page) || exempt(page) {
        return false;
    }
    match engine().popup_blocked(url, page) {
        Some(rule) => {
            log!("[popup blocked] {url} ({rule})");
            true
        }
        None => false,
    }
}

/// Element-hiding CSS for a page (empty when there's nothing to hide).
pub fn hiding_css(page: &str, dom: &crate::dom::Dom) -> String {
    if !web(page) || exempt(page) {
        return String::new();
    }
    let mut classes = HashSet::new();
    let mut ids = HashSet::new();
    let mut attrs = HashSet::new();
    dom.walk(crate::dom::Dom::ROOT, |n| {
        if let Some(c) = dom.attr(n, "class") {
            classes.extend(c.split_ascii_whitespace().map(String::from));
        }
        if let Some(i) = dom.attr(n, "id") {
            ids.insert(i.to_string());
        }
        for k in dom.attr_names(n) {
            if !attrs.contains(k) {
                attrs.insert(k.to_ascii_lowercase());
            }
        }
        true
    });
    engine().hiding_css_with(page, &classes, &ids, &attrs)
}

/// The scriptlets and the procedural cosmetic filter runner, written from scratch.
const SCRIPTLETS_JS: &str = include_str!("scriptlets.js");
const COSMETIC_JS: &str = include_str!("cosmetic.js");

fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\u003c"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Code to run in a page before any of its own scripts: the scriptlets its filters name and
/// the procedural cosmetic filters for it. None when there's nothing to run.
pub fn page_script(page: &str) -> Option<String> {
    if !web(page) || exempt(page) {
        return None;
    }
    let e = engine();
    let calls = e.scriptlets_for(page);
    let procs = e.procedural_for(page);
    if calls.is_empty() && procs.is_empty() {
        return None;
    }
    log!("[content blocking] {page}: scriptlets {}; {} procedural filters", calls.iter().map(|c| c[0].as_str()).collect::<Vec<_>>().join(", "), procs.len());
    Some(code_for(&calls, &procs))
}

/// scriptlets.js with only the `def(…)` blocks for these scriptlet names (and the helpers).
fn scriptlets_needed(names: &HashSet<&str>) -> String {
    let mut out = String::with_capacity(SCRIPTLETS_JS.len() / 2);
    let mut keep = true;
    let mut inside = false;
    for line in SCRIPTLETS_JS.lines() {
        if !inside {
            if let Some(rest) = line.strip_prefix("def('") {
                let def_names = rest.split('\'').next().unwrap_or("");
                keep = def_names.split(' ').any(|n| names.contains(n));
                inside = !line.ends_with("});");
            } else {
                keep = true;
            }
        } else if line == "});" {
            inside = false;
            if keep {
                out.push_str(line);
                out.push('\n');
            }
            keep = true;
            continue;
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// HTML filters (`##^…`) for a page: (plain CSS selector, text the element must contain).
pub fn html_filters(page: &str) -> Vec<(String, Option<HtmlText>)> {
    if !web(page) || exempt(page) {
        return Vec::new();
    }
    engine()
        .html_filters_for(page)
        .into_iter()
        .filter_map(|f| match f.rfind(":has-text(") {
            Some(i) if f.ends_with(')') => {
                let needle = &f[i + 10..f.len() - 1];
                let base = if i == 0 { "*" } else { &f[..i] };
                text_pattern(needle).map(|p| (base.to_string(), Some(HtmlText(p))))
            }
            _ => (!f.contains(":has-text(")).then(|| (f, None)),
        })
        .collect()
}

/// Text an HTML filter's element must contain (`:has-text(…)`).
pub struct HtmlText(TextPattern);

impl HtmlText {
    pub fn found_in(&self, text: &str) -> bool {
        self.0.test(text)
    }
}

/// An inline script about to run in `page`: None = drop it, else the (possibly edited) code.
pub fn filter_inline_script(page: &str, code: &str) -> Option<String> {
    if !web(page) || exempt(page) {
        return Some(code.to_string());
    }
    engine().filter_script(page, code)
}

/// The code that runs these scriptlet calls and procedural filters in a page.
pub(crate) fn code_for(calls: &[Vec<String>], procs: &[&str]) -> String {
    let mut code = String::new();
    if !calls.is_empty() {
        let list: Vec<String> = calls.iter().map(|c| format!("[{}]", c.iter().map(|a| js_string(a)).collect::<Vec<_>>().join(","))).collect();
        let names: HashSet<&str> = calls.iter().map(|c| c[0].trim_end_matches(".js")).collect();
        code.push_str(&format!("(function(calls){{{}\n}})([{}]);\n", scriptlets_needed(&names), list.join(",")));
    }
    if !procs.is_empty() {
        let list: Vec<String> = procs.iter().map(|s| js_string(s)).collect();
        code.push_str(&format!("(function(rules){{{COSMETIC_JS}\n}})([{}]);\n", list.join(",")));
    }
    code
}

/// Should opening this page show the warning page first (a dangerous site, or one a filter
/// blocks outright)? The reason, if so.
pub fn dangerous(url: &str) -> bool {
    warning_reason(url).is_some()
}

fn warning_reason(url: &str) -> Option<String> {
    if !web(url) {
        return None;
    }
    let host = host_of(url).unwrap_or_default();
    if PROCEED.lock().is_ok_and(|p| p.contains(&host)) {
        return None;
    }
    let s = crate::settings::get();
    let e = engine();
    if s.safe_browsing && e.is_dangerous(url) {
        return Some(String::new());
    }
    if exempt(url) {
        return None;
    }
    e.doc_blocked(url).map(String::from)
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

/// The warning shown instead of a dangerous or blocked site.
pub fn warning_page(url: &str) -> String {
    let host = host_of(url).unwrap_or_default();
    let rule = warning_reason(url).filter(|r| !r.is_empty());
    let (title, lead, more) = match &rule {
        None => (
            "This site may harm your computer or steal your information",
            format!("<b>{}</b> is on a list of sites that spread malware or try to trick people into giving away passwords and card numbers.", esc(&host)),
            "The lists come from URLhaus (abuse.ch), Phishing Army and uBlock Origin and are updated automatically.".to_string(),
        ),
        Some(r) => (
            "This site was blocked by a filter list",
            format!("A filter list you use blocks <b>{}</b>: it serves ads, tracks people, or is known for scams or malware.", esc(&host)),
            format!("The rule that matched: <code>{}</code>.", esc(r)),
        ),
    };
    format!(
        r#"<!DOCTYPE html><html><head><title>Site blocked</title><style>
body{{font-family:sans-serif;background:#b91c1c;color:white;margin:0;min-height:100vh}}
main{{max-width:640px;margin:0 auto;padding:80px 24px}} h1{{font-size:28px}}
p{{font-size:16px;line-height:1.5}} .url{{background:rgba(0,0,0,.25);padding:6px 10px;border-radius:6px;word-break:break-all}}
a.back{{display:inline-block;background:white;color:#b91c1c;padding:10px 20px;border-radius:8px;text-decoration:none;font-weight:bold}}
.more{{margin-top:40px;font-size:13px;opacity:.85}} .more a{{color:white}} code{{word-break:break-all}}
</style></head><body><main>
<h1>{title}</h1>
<p>{lead}</p>
<p class="url">{url_esc}</p>
<p><a class="back" href="about:home">Go back to safety</a></p>
<p class="more">{more} If you understand the risk, you can <a href="about:blocked?proceed={url_enc}">visit this site anyway</a>.</p>
</main></body></html>"#,
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

fn get(url: &str) -> Result<Vec<u8>, String> {
    let req = crate::net::Request { method: "GET", url, headers: &[], body: None, top: None, cookies: false, extra: &[] };
    match crate::net::http(&req) {
        Ok(r) if r.status == 200 => Ok(r.body),
        Ok(r) => Err(format!("HTTP {}", r.status)),
        Err(e) => Err(e),
    }
}

/// Put the files a list names with `!#include` in place of those lines (only files beside
/// the list itself, one level deep, as uBlock Origin does).
fn expand_includes(text: &str, list_url: &str, fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> String {
    if !text.contains("!#include ") {
        return text.to_string();
    }
    let base = &list_url[..list_url.rfind('/').map_or(list_url.len(), |i| i + 1)];
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if let Some(name) = line.trim().strip_prefix("!#include ") {
            let name = name.trim();
            if !name.is_empty() && !name.contains("..") && !name.contains("://") && !name.starts_with('/') {
                match fetch(&format!("{base}{name}")) {
                    Ok(body) => {
                        out.push_str(&String::from_utf8_lossy(&body));
                        out.push('\n');
                    }
                    Err(e) => log!("[filters] {base}{name}: {e}"),
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Fetch the lists now (`force`: even fresh ones). Returns a summary for about:settings.
pub fn update_now(force: bool) -> String {
    let Some(dir) = dir() else { return "No profile folder.".into() };
    let _ = std::fs::create_dir_all(&dir);
    let s = crate::settings::get();
    let mut done = Vec::new();
    for (name, url, kind) in LISTS {
        let path = dir.join(name);
        let fresh = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < MAX_AGE);
        if !list_on(*kind, &s) || (fresh && !force) {
            continue;
        }
        match get(url) {
            Ok(body) if body.len() > 100 => {
                let text = expand_includes(&String::from_utf8_lossy(&body), url, &get);
                let tmp = path.with_extension("tmp");
                if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
                    done.push(*name);
                }
            }
            Ok(_) => log!("[filters] {url}: empty"),
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
    let s = crate::settings::get();
    LISTS
        .iter()
        .filter(|(_, _, kind)| list_on(*kind, &s))
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
        let e = eng("||ads.example.com^\n||track.net^$third-party\n/banner/*/ad_\n||cdn.site.com/js/ads.js$script\n@@||ads.example.com/allowed/\n|https://exact.com/x.js|\n||imp.org^$image,domain=news.com|~sports.news.com\n||boom.com^$header=x-ad\n");
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
        let e = eng("||ad.com^\n@@||trusted.org^$document\n##.ad-box\n@@||plain.net^$elemhide\n");
        assert!(e.check("https://ad.com:8080/x", "https://a.com/", Dest::Image).is_some());
        assert!(e.check("https://ad.com", "https://a.com/", Dest::Image).is_some());
        assert!(e.check("https://ad.community/x", "https://a.com/", Dest::Image).is_none());
        assert!(e.check("https://ad.com/x", "https://www.trusted.org/", Dest::Image).is_none());
        // $elemhide turns off hiding only, not blocking.
        assert!(e.check("https://ad.com/x", "https://plain.net/", Dest::Image).is_some());
        let classes: HashSet<String> = ["ad-box".to_string()].into();
        assert!(e.hiding_css("https://plain.net/", &classes, &HashSet::new()).is_empty());
        assert!(!e.hiding_css("https://other.net/", &classes, &HashSet::new()).is_empty());
    }

    #[test]
    fn regex_important_badfilter() {
        let e = eng("/^https?:\\/\\/[a-z]+\\.cdn\\.net\\/ad[0-9]+\\.js/$script\n||ok.com^\n@@||ok.com^\n||must.com^$important\n@@||must.com^\n||gone.com^\n||gone.com^$badfilter\n||x.org^$important\n@@||x.org^$important\n");
        assert!(e.check("https://img.cdn.net/ad123.js", "https://a.com/", Dest::Script).is_some());
        assert!(e.check("https://img.cdn.net/app.js", "https://a.com/", Dest::Script).is_none());
        assert!(e.check("https://ok.com/x", "https://a.com/", Dest::Script).is_none());
        assert!(e.check("https://must.com/x", "https://a.com/", Dest::Script).is_some(), "important beats an exception");
        assert!(e.check("https://gone.com/x", "https://a.com/", Dest::Script).is_none(), "badfilter");
        assert!(e.check("https://x.org/x", "https://a.com/", Dest::Script).is_none(), "important exception");
    }

    #[test]
    fn regex_literals() {
        assert_eq!(regex_literal(r"^https?:\/\/[a-z]+\.example\.com\/ads\/"), Some(".example.com/ads/".to_string()));
        assert_eq!(regex_literal(r"/banner[0-9]+\.gif"), Some("/banner".to_string()));
        assert_eq!(regex_literal(r"(?:ad|track)\.js"), None);
        assert_eq!(regex_literal(r"adverts?/"), Some("advert".to_string()));
        let e = eng("/\\/promo(tion)?s\\/[0-9]+\\.jpg/$image\n/^https:\\/\\/[a-z0-9]{10}\\.com\\/[a-z]{5}\\.js$/$script,domain=site.org\n");
        assert!(e.check("https://img.com/promos/12.jpg", "https://a.com/", Dest::Image).is_some());
        assert!(e.check("https://img.com/promotions/12.jpg", "https://a.com/", Dest::Image).is_some());
        assert!(e.check("https://abcdefghij.com/qwert.js", "https://www.site.org/", Dest::Script).is_some());
        assert!(e.check("https://abcdefghij.com/qwert.js", "https://other.org/", Dest::Script).is_none());
    }

    #[test]
    fn to_denyallow_entity() {
        let e = eng("*$script,3p,denyallow=cdn.com|jsdelivr.net,domain=strict.org\n||ads.net^$domain=google.*\n");
        assert!(e.check("https://x.com/a.js", "https://strict.org/", Dest::Script).is_some());
        assert!(e.check("https://cdn.com/a.js", "https://strict.org/", Dest::Script).is_none());
        assert!(e.check("https://x.com/a.js", "https://other.org/", Dest::Script).is_none());
        assert!(e.check("https://ads.net/a.js", "https://www.google.co.uk/", Dest::Script).is_some());
        assert!(e.check("https://ads.net/a.js", "https://google.de/", Dest::Script).is_some());
        assert!(e.check("https://ads.net/a.js", "https://notgoogle.de/", Dest::Script).is_none());
    }

    #[test]
    fn redirects() {
        let e = eng("||googletagmanager.com/gtm.js$script,redirect=googletagmanager_gtm.js\n||ads.com/pixel$image,redirect=1x1.gif\n||ads.com^$script\n||ads.com/lib.js$script,redirect-rule=noopjs\n||only.com/x.js$redirect-rule=noopjs\n");
        let (_, r) = e.decide("https://www.googletagmanager.com/gtm.js?id=GTM-1", "https://shop.com/", T_SCRIPT).unwrap();
        assert_eq!(r, Some("googletagmanager_gtm.js"));
        let (ty, body) = resource(r.unwrap(), T_SCRIPT).unwrap();
        assert_eq!(ty, "text/javascript");
        assert!(String::from_utf8(body).unwrap().contains("dataLayer"));
        assert_eq!(e.decide("https://ads.com/pixel?id=1", "https://shop.com/", T_IMAGE).unwrap().1, Some("1x1.gif"));
        assert_eq!(e.decide("https://ads.com/lib.js", "https://shop.com/", T_SCRIPT).unwrap().1, Some("noopjs"));
        assert!(e.decide("https://only.com/x.js", "https://shop.com/", T_SCRIPT).is_none(), "redirect-rule alone doesn't block");
        for name in ["google-analytics_analytics.js", "googletagservices_gpt.js", "googlesyndication_adsbygoogle.js", "google-ima.js", "scorecardresearch_beacon.js", "fuckadblock.js-3.2.0", "amazon_apstag.js", "noop.js", "1x1.gif", "noop-vast3.xml"] {
            assert!(resource(name, T_SCRIPT).is_some(), "{name}");
        }
    }

    #[test]
    fn removeparam_and_csp() {
        let e = eng("$removeparam=utm_source\n$removeparam=/^ref_/\n||shop.com^$removeparam=sid\n@@||keep.com^$removeparam\n||news.com^$csp=script-src 'self'\n@@||news.com/live^$csp\n");
        assert_eq!(e.remove_params("https://a.com/p?utm_source=x&id=2&ref_a=1#top", "https://a.com/", T_DOC).as_deref(), Some("https://a.com/p?id=2#top"));
        assert_eq!(e.remove_params("https://shop.com/?sid=1", "https://shop.com/", T_DOC).as_deref(), Some("https://shop.com/"));
        assert_eq!(e.remove_params("https://keep.com/?utm_source=x", "https://keep.com/", T_DOC), None);
        assert_eq!(e.remove_params("https://a.com/?id=1", "https://a.com/", T_DOC), None);
        assert_eq!(e.csp_for("https://news.com/story", "https://news.com/story"), vec!["script-src 'self'".to_string()]);
        assert!(e.csp_for("https://news.com/live/1", "https://news.com/live/1").is_empty());
    }

    #[test]
    fn document_and_popup_blocking() {
        let e = eng("||scam.example^\n||evil.com^$doc\n||tracker.net^$third-party\n||pop.ads^$popup\n@@||fine.example^$document\n||fine.example^\n");
        assert!(e.doc_blocked("https://scam.example/login").is_some(), "a whole-site rule blocks pages too");
        assert!(e.doc_blocked("https://www.evil.com/").is_some());
        assert!(e.doc_blocked("https://tracker.net/").is_none(), "third-party rules don't block pages");
        assert!(e.doc_blocked("https://fine.example/").is_none());
        assert!(e.popup_blocked("https://pop.ads/x", "https://a.com/").is_some());
        assert!(e.check("https://pop.ads/x", "https://a.com/", Dest::Script).is_none(), "$popup only blocks popups");
    }

    #[test]
    fn preprocessing() {
        let e = eng("!#if env_firefox\n||firefox-only.com^\n!#else\n||others.com^\n!#endif\n!#if !env_mobile && ext_ublock\n||ubo.com^\n!#endif\n!#if false\n!#if true\n||nested.com^\n!#endif\n!#endif\n");
        let b = |u: &str| e.check(u, "https://a.com/", Dest::Script).is_some();
        assert!(!b("https://firefox-only.com/"));
        assert!(b("https://others.com/"));
        assert!(b("https://ubo.com/"));
        assert!(!b("https://nested.com/"));
        let inc = expand_includes("a\n!#include sub.txt\n!#include ../x.txt\nb\n", "https://lists.example/filters/main.txt", &|u: &str| {
            assert_eq!(u, "https://lists.example/filters/sub.txt");
            Ok(b"||sub.com^".to_vec())
        });
        assert_eq!(inc, "a\n||sub.com^\nb\n");
    }

    #[test]
    fn hiding() {
        let e = eng("##.ad-box\n##.promo > a\n###sidebar-ad\nnews.com##.paywall-ad\nnews.com#@#.ad-box\n##div:has-text(Sponsored)\n##a[href^=\"https://ads.\"]\nnews.com##.hero:style(margin-top: 0)\nexample.*##.promo-bar\nnews.com#?#.card:has-text(Sponsored)\nnews.com##.slot:remove()\nnews.com#$#.top { height: 0 }\n");
        let classes: HashSet<String> = ["ad-box", "promo", "content"].iter().map(|s| s.to_string()).collect();
        let ids: HashSet<String> = ["sidebar-ad"].iter().map(|s| s.to_string()).collect();
        let attrs: HashSet<String> = ["href".to_string()].into();
        let css = e.hiding_css_with("https://blog.com/", &classes, &ids, &attrs);
        assert!(css.contains(".ad-box{display:none!important}"));
        assert!(css.contains(".promo > a{"));
        assert!(css.contains("#sidebar-ad{"));
        assert!(css.contains("a[href^=\"https://ads.\"]{"));
        assert!(!css.contains("paywall") && !css.contains("has-text"));
        let css = e.hiding_css("https://www.news.com/", &classes, &ids);
        assert!(css.contains(".paywall-ad{") && !css.contains(".ad-box{"));
        assert!(css.contains(".hero{margin-top: 0 !important}"));
        assert!(css.contains(".top{height: 0 !important}"));
        assert!(e.hiding_css("https://www.example.co.uk/", &classes, &ids).contains(".promo-bar{"));
        assert_eq!(e.procedural_for("https://news.com/"), vec![".card:has-text(Sponsored)", ".slot:remove()"]);
        assert!(e.procedural_for("https://blog.com/").is_empty());
    }

    #[test]
    fn unsafe_styles_are_dropped() {
        let e = eng("a.com##.x:style(background: url(https://track.me/p.gif))\na.com##.y:style(color: red} body{display:none)\n");
        assert!(e.hiding_css("https://a.com/", &HashSet::new(), &HashSet::new()).is_empty());
    }

    #[test]
    fn scriptlet_rules() {
        let e = eng("news.com,blog.*##+js(set-constant, adBlockDetected, false)\nnews.com##+js(nostif, /adb\\,lock/, 2000)\n##+js(generic, ignored)\nsports.news.com#@#+js(set-constant, adBlockDetected, false)\nfree.com#@#+js()\nfree.com##+js(aopr, ads)\n");
        assert_eq!(e.scriptlets_for("https://www.news.com/"), vec![vec!["set-constant", "adBlockDetected", "false"], vec!["nostif", "/adb,lock/", "2000"]]);
        assert_eq!(e.scriptlets_for("https://blog.org/").len(), 1);
        assert_eq!(e.scriptlets_for("https://sports.news.com/").len(), 1);
        assert!(e.scriptlets_for("https://free.com/").is_empty());
        assert!(e.scriptlets_for("https://other.com/").is_empty());
        assert_eq!(split_call("json-prune, 'a.b c', \"x\""), vec!["json-prune", "a.b c", "x"]);
    }

    #[test]
    fn trusted_scriptlets_and_script_filters() {
        let rules = "news.com##+js(trusted-set-constant, a, 1)\nnews.com##+js(rpnt, script, /showAds\\((\\d)\\)/, hideAds($1))\nnews.com##+js(set, b, true)\nnews.com##^script:has-text(adblockDetected)\nnews.com##^.ad-frame\n";
        let mut e = Engine::default();
        e.add_list_from(rules, Kind::Ads, false);
        let names = |e: &Engine| e.scriptlets_for("https://news.com/").into_iter().map(|c| c[0].clone()).collect::<Vec<_>>();
        assert_eq!(names(&e), vec!["set"], "an untrusted list can't use trusted scriptlets");
        assert_eq!(e.filter_script("https://news.com/", "showAds(1);"), Some("showAds(1);".into()));
        assert_eq!(e.filter_script("https://news.com/", "if (adblockDetected) wall();"), None, "##^script:has-text drops the script");
        assert_eq!(e.html_filters_for("https://news.com/"), vec![".ad-frame"]);
        let mut e = Engine::default();
        e.add_list_from(rules, Kind::Ads, true);
        assert_eq!(names(&e), vec!["trusted-set-constant", "rpnt", "set"]);
        assert_eq!(e.filter_script("https://news.com/", "x(); showAds(2); showAds(3);"), Some("x(); hideAds(2); showAds(3);".into()));
        assert_eq!(e.filter_script("https://other.com/", "showAds(2);"), Some("showAds(2);".into()));
        assert_eq!(text_pattern("/a(b)/g").unwrap().replace("ab ab", "[$1$&$$]"), "[bab$] [bab$]");
        assert_eq!(text_pattern("x.y").unwrap().replace("x.y x.y", "z"), "z x.y");
    }

    #[test]
    fn hosts_lines_and_network_rules_with_hashes() {
        let e = eng("0.0.0.0 ads.hosts.example\n127.0.0.1 localhost\n||site.com/page#anchor\n");
        assert!(e.check("https://ads.hosts.example/x", "https://a.com/", Dest::Image).is_some());
        assert!(e.check("http://localhost/x", "https://a.com/", Dest::Image).is_none());
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
        assert_eq!(e.decide("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://news.com/", T_SCRIPT).unwrap().1, Some("googletagservices_gpt.js"));
    }

    /// The bundled JavaScript parses (a syntax error there would break every page it's added to).
    #[test]
    fn bundled_scripts_parse() {
        // Boa's parser recurses deeply in debug builds: give it room.
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let mut ctx = boa_engine::Context::default();
                for (name, code) in [("scriptlets.js", format!("(function(calls){{{SCRIPTLETS_JS}\n}})")), ("cosmetic.js", format!("(function(rules){{{COSMETIC_JS}\n}})")), ("redirects.js", REDIRECT_JS.to_string())] {
                    let src = boa_engine::Source::from_bytes(code.as_bytes());
                    assert!(boa_engine::Script::parse(src, None, &mut ctx).is_ok(), "{name} doesn't parse");
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn scriptlet_subsets() {
        let all = scriptlets_needed(&HashSet::new());
        assert!(!all.contains("def('set-constant"), "no blocks without a name");
        assert!(all.contains("function trapChain") && all.contains("run the calls"));
        let some = scriptlets_needed(&["aopr", "trusted-prevent-xhr"].into_iter().collect());
        assert!(some.contains("def('abort-on-property-read aopr'") && some.contains("def('trusted-prevent-xhr'"));
        assert!(!some.contains("def('json-prune'") && !some.contains("def('set-constant"));
        // Every block is closed: the whole file is all of its blocks.
        let every: HashSet<&str> = SCRIPTLETS_JS.lines().filter_map(|l| l.strip_prefix("def('")).flat_map(|r| r.split('\'').next().unwrap_or("").split(' ')).collect();
        assert_eq!(scriptlets_needed(&every).lines().count(), SCRIPTLETS_JS.lines().count());
    }

    /// With the real lists: IRONSWIFT_LISTS=dir cargo test --release real_lists -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_lists() {
        let Ok(dir) = std::env::var("IRONSWIFT_LISTS") else { return };
        let t = std::time::Instant::now();
        let mut e = Engine::default();
        for f in ["easylist.txt", "easyprivacy.txt", "ublock-filters.txt", "ublock-privacy.txt", "ublock-quick-fixes.txt", "ublock-unbreak.txt", "peter-lowe.txt"] {
            if let Ok(text) = std::fs::read_to_string(format!("{dir}/{f}")) {
                e.add_list(&text, Kind::Ads);
            }
        }
        println!(
            "loaded {} rules in {:?} (network {}, host-indexed {}, token-indexed {}, untokened {}, modifiers {}, page rules {}, generic hiding {}+{}+{}+{}, sites with cosmetic {}, procedural {}, scriptlets {})",
            e.rule_count,
            t.elapsed(),
            e.net.rules.len(),
            e.net.by_host.len(),
            e.net.by_token.len(),
            e.net.untokened.len(),
            e.mods.rules.len(),
            e.page_rules.rules.len(),
            e.hide_by_class.len(),
            e.hide_by_id.len(),
            e.hide_by_attr.len(),
            e.hide_always.len(),
            e.hide_on.len(),
            e.procedural_on.len(),
            e.scriptlets_on.len()
        );
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
            let got = e.decide(u, p, type_bit(d));
            println!("{u}: {got:?}");
            assert_eq!(got.is_some(), want, "{u}");
        }
        println!("youtube scriptlets: {:?}", e.scriptlets_for("https://www.youtube.com/"));
        println!("removeparam: {:?}", e.remove_params("https://example.com/?utm_source=a&fbclid=b&id=1", "https://example.com/", T_DOC));
    }
}
