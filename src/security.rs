//! Web security rules that sit between pages and the network:
//!
//! - **Content Security Policy** (`Content-Security-Policy` header and `<meta http-equiv>`):
//!   which scripts, styles, images, fonts, frames and connections a page may use, whether
//!   inline code and `eval` may run, and who may embed the page (`frame-ancestors`).
//! - **Framing**: `X-Frame-Options` and `frame-ancestors`, and the `<iframe sandbox>` flags.
//! - **What a web page may open**: pages on the internet can't navigate to, load or read
//!   local files, the browser's own `about:` pages, or `data:` documents.
//!
//! Policies are registered per page URL (`register`) so that every fetch a page makes — even
//! from the worker threads that download its pieces — is checked against them in `net`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// What a request is for (decides which CSP directive applies).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Media and Object: for <video>/<audio>/<object> once those load anything
pub enum Dest {
    Script,
    Style,
    Image,
    Font,
    /// fetch(), XMLHttpRequest, WebSocket, EventSource
    Connect,
    Frame,
    Media,
    Object,
    /// The page's icon and other things no directive covers.
    Other,
}

/// Where inline code comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // StyleAttr: style="…" attributes aren't checked yet
pub enum Inline {
    /// `<script>` without src
    Script,
    /// `onclick="…"` and other handler attributes, `javascript:` URLs
    ScriptAttr,
    /// `<style>`
    Style,
    /// `style="…"` attributes
    StyleAttr,
}

// ---------------------------------------------------------------- Content Security Policy

#[derive(Clone, Debug, Default)]
struct Policy {
    /// directive name -> source list (lower-cased keywords, original case for nonces/hashes)
    directives: HashMap<String, Vec<String>>,
    report_only: bool,
}

/// All the policies a page is under. Every enforced policy must allow a thing for it to happen.
#[derive(Clone, Debug, Default)]
pub struct Csp {
    policies: Vec<Policy>,
    /// The page's own URL ('self' is its origin).
    page: String,
}

/// A request that a policy refused: logged, like a browser's console message.
fn refused(what: &str, directive: &str, report_only: bool) {
    if report_only {
        log!("[csp report-only] would refuse {what} ({directive})");
    } else {
        log!("[csp] refused {what} ({directive})");
    }
}

impl Csp {
    /// Policies from response headers (`content-security-policy`, `-report-only`) and the
    /// `content` of `<meta http-equiv="Content-Security-Policy">` elements.
    pub fn new(page: &str, headers: &[(String, String)], meta: &[String]) -> Csp {
        let mut policies = Vec::new();
        for (k, v) in headers {
            let k = k.to_ascii_lowercase();
            if k == "content-security-policy" || k == "content-security-policy-report-only" {
                // One header may carry several policies separated by commas.
                for text in v.split(',') {
                    let p = parse_policy(text, k.ends_with("report-only"), false);
                    if !p.directives.is_empty() {
                        policies.push(p);
                    }
                }
            }
        }
        for text in meta {
            let p = parse_policy(text, false, true);
            if !p.directives.is_empty() {
                policies.push(p);
            }
        }
        Csp { policies, page: page.to_string() }
    }

    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// The source list that governs `directive`, following CSP's fallback chain.
    fn list<'a>(p: &'a Policy, chain: &[&str]) -> Option<(&'a str, &'a Vec<String>)> {
        chain.iter().find_map(|d| p.directives.get_key_value(*d).map(|(k, v)| (k.as_str(), v)))
    }

    fn chain(dest: Dest) -> &'static [&'static str] {
        match dest {
            Dest::Script => &["script-src-elem", "script-src", "default-src"],
            Dest::Style => &["style-src-elem", "style-src", "default-src"],
            Dest::Image => &["img-src", "default-src"],
            Dest::Font => &["font-src", "default-src"],
            Dest::Connect => &["connect-src", "default-src"],
            Dest::Frame => &["frame-src", "child-src", "default-src"],
            Dest::Media => &["media-src", "default-src"],
            Dest::Object => &["object-src", "default-src"],
            Dest::Other => &[],
        }
    }

    /// May the page load `url` for this purpose? (`parser_inserted`: the element was in the
    /// page's HTML rather than added by a script; matters for 'strict-dynamic'.)
    pub fn allows_url(&self, dest: Dest, url: &str, nonce: Option<&str>, parser_inserted: bool) -> bool {
        let mut ok = true;
        for p in &self.policies {
            let Some((name, list)) = Self::list(p, Self::chain(dest)) else { continue };
            let allowed = if dest == Dest::Script && has(list, "'strict-dynamic'") {
                // Only nonce-carrying scripts, and whatever trusted scripts add themselves.
                !parser_inserted || nonce.is_some_and(|n| nonce_ok(list, n))
            } else {
                nonce.is_some_and(|n| matches!(dest, Dest::Script | Dest::Style) && nonce_ok(list, n)) || list_matches(list, url, &self.page)
            };
            if !allowed {
                refused(&format!("{dest:?} {url}"), name, p.report_only);
                ok &= p.report_only;
            }
        }
        ok
    }

    /// May this inline code run? `nonce` is the element's nonce attribute.
    pub fn allows_inline(&self, kind: Inline, code: &str, nonce: Option<&str>) -> bool {
        let chain: &[&str] = match kind {
            Inline::Script => &["script-src-elem", "script-src", "default-src"],
            Inline::ScriptAttr => &["script-src-attr", "script-src", "default-src"],
            Inline::Style => &["style-src-elem", "style-src", "default-src"],
            Inline::StyleAttr => &["style-src-attr", "style-src", "default-src"],
        };
        let attr = matches!(kind, Inline::ScriptAttr | Inline::StyleAttr);
        let mut ok = true;
        for p in &self.policies {
            let Some((name, list)) = Self::list(p, chain) else { continue };
            let hashes_or_nonces = list.iter().any(|s| is_nonce(s) || is_hash(s));
            let strict = matches!(kind, Inline::Script | Inline::ScriptAttr) && has(list, "'strict-dynamic'");
            let allowed = if !attr && nonce.is_some_and(|n| nonce_ok(list, n)) {
                true
            } else if (!attr || has(list, "'unsafe-hashes'")) && hash_ok(list, code) {
                true
            } else {
                // 'unsafe-inline' is ignored once a nonce, a hash or 'strict-dynamic' is listed.
                has(list, "'unsafe-inline'") && !hashes_or_nonces && !strict
            };
            if !allowed {
                let sample: String = code.trim().chars().take(40).collect();
                refused(&format!("inline {kind:?} \"{sample}\""), name, p.report_only);
                ok &= p.report_only;
            }
        }
        ok
    }

    /// May the page turn strings into code (`eval`, `new Function`, string timers)?
    pub fn allows_eval(&self) -> bool {
        let mut ok = true;
        for p in &self.policies {
            let Some((name, list)) = Self::list(p, &["script-src", "default-src"]) else { continue };
            if !has(list, "'unsafe-eval'") {
                refused("eval", name, p.report_only);
                ok &= p.report_only;
            }
        }
        ok
    }

    /// May a form on the page submit to `url`?
    pub fn allows_form_action(&self, url: &str) -> bool {
        self.directive_ok("form-action", url)
    }

    fn directive_ok(&self, directive: &str, url: &str) -> bool {
        let mut ok = true;
        for p in &self.policies {
            if let Some(list) = p.directives.get(directive) {
                if !list_matches(list, url, &self.page) {
                    refused(url, directive, p.report_only);
                    ok &= p.report_only;
                }
            }
        }
        ok
    }

    /// `upgrade-insecure-requests`: load the page's http:// pieces over https://.
    pub fn upgrade_insecure(&self) -> bool {
        self.policies.iter().any(|p| !p.report_only && p.directives.contains_key("upgrade-insecure-requests"))
    }

    /// The `sandbox` directive's flags, if any policy has one (headers only).
    pub fn sandbox(&self) -> Option<Sandbox> {
        self.policies
            .iter()
            .filter(|p| !p.report_only)
            .find_map(|p| p.directives.get("sandbox"))
            .map(|flags| Sandbox::parse(&flags.join(" ")))
    }

    /// May a page at `ancestor` embed this one? (`frame-ancestors`; None = no directive.)
    fn frame_ancestor_ok(&self, ancestor: &str) -> Option<bool> {
        let mut seen = false;
        let mut ok = true;
        for p in self.policies.iter().filter(|p| !p.report_only) {
            if let Some(list) = p.directives.get("frame-ancestors") {
                seen = true;
                ok &= list_matches(list, ancestor, &self.page);
            }
        }
        seen.then_some(ok)
    }
}

fn parse_policy(text: &str, report_only: bool, from_meta: bool) -> Policy {
    let mut directives = HashMap::new();
    for part in text.split(';') {
        let mut toks = part.split_ascii_whitespace();
        let Some(name) = toks.next() else { continue };
        let name = name.to_ascii_lowercase();
        // A <meta> policy can't set these (the HTML standard ignores them there).
        if from_meta && matches!(name.as_str(), "frame-ancestors" | "report-uri" | "report-to" | "sandbox") {
            continue;
        }
        if directives.contains_key(&name) {
            continue; // the first occurrence wins
        }
        let list: Vec<String> = toks
            .map(|t| {
                let l = t.to_ascii_lowercase();
                // Nonces and hashes keep their case; everything else compares case-insensitively.
                if l.starts_with("'nonce-") || l.starts_with("'sha") { t.to_string() } else { l }
            })
            .collect();
        directives.insert(name, list);
    }
    Policy { directives, report_only }
}

fn has(list: &[String], kw: &str) -> bool {
    list.iter().any(|s| s == kw)
}

fn is_nonce(s: &str) -> bool {
    s.len() > 8 && s.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("'nonce-"))
}

fn is_hash(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.starts_with("'sha256-") || l.starts_with("'sha384-") || l.starts_with("'sha512-")
}

fn nonce_ok(list: &[String], nonce: &str) -> bool {
    !nonce.is_empty() && list.iter().any(|s| is_nonce(s) && s.get(7..).is_some_and(|n| n.trim_end_matches('\'') == nonce))
}

/// Does the code's SHA-2 digest appear in the list?
fn hash_ok(list: &[String], code: &str) -> bool {
    use base64::Engine;
    use sha2::Digest;
    let b64 = |d: &[u8]| base64::engine::general_purpose::STANDARD.encode(d);
    let mut digests: Vec<(&str, String)> = Vec::new();
    for s in list.iter().filter(|s| is_hash(s)) {
        let (algo, want) = s.trim_matches('\'').split_once('-').unwrap_or(("", ""));
        let algo = algo.to_ascii_lowercase();
        let have = match digests.iter().find(|(a, _)| *a == algo) {
            Some((_, d)) => d.clone(),
            None => {
                let d = match algo.as_str() {
                    "sha256" => b64(&sha2::Sha256::digest(code.as_bytes())),
                    "sha384" => b64(&sha2::Sha384::digest(code.as_bytes())),
                    _ => b64(&sha2::Sha512::digest(code.as_bytes())),
                };
                digests.push((if algo == "sha256" { "sha256" } else if algo == "sha384" { "sha384" } else { "sha512" }, d.clone()));
                d
            }
        };
        // Base64 and base64url spellings are both accepted.
        if have == want || have.replace('+', "-").replace('/', "_") == want {
            return true;
        }
    }
    false
}

/// Does `url` match any source expression in the list, for a page at `page`?
fn list_matches(list: &[String], url: &str, page: &str) -> bool {
    if list.len() == 1 && list[0] == "'none'" {
        return false;
    }
    let Ok(u) = url::Url::parse(url) else { return false };
    let page_u = url::Url::parse(page).ok();
    list.iter().any(|src| source_matches(src, &u, page_u.as_ref()))
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

/// One source expression against a URL (CSP 3, "Does url match expression in origin").
fn source_matches(src: &str, u: &url::Url, page: Option<&url::Url>) -> bool {
    let scheme = u.scheme();
    let network = matches!(scheme, "http" | "https" | "ws" | "wss");
    // Upgrades from the protected scheme are allowed (http → https, ws → wss).
    let scheme_ok = |want: &str| want == scheme || (want == "http" && scheme == "https") || (want == "ws" && scheme == "wss") || (want == "http" && scheme == "wss") || (want == "https" && scheme == "wss");
    match src {
        "*" => return network || page.is_some_and(|p| p.scheme() == scheme),
        "'self'" => {
            let Some(p) = page else { return false };
            if p.scheme() != "http" && p.scheme() != "https" {
                return false; // an opaque origin matches nothing
            }
            let same_host = p.host_str() == u.host_str();
            let port_of = |x: &url::Url| x.port_or_known_default();
            return same_host && (p.origin() == u.origin() || (p.scheme() == "http" && scheme == "https" && (port_of(u) == Some(443) || port_of(p) == port_of(u))) || (scheme_ok(p.scheme()) && port_of(p) == port_of(u)));
        }
        s if s.starts_with('\'') => return false, // keywords, nonces, hashes: not URL matches
        _ => {}
    }
    // scheme-source: "https:", "data:", "blob:"
    if let Some(sch) = src.strip_suffix(':') {
        if !sch.contains('/') && !sch.contains('.') {
            return scheme_ok(sch);
        }
    }
    // host-source: [scheme://]host[:port][/path]
    let (want_scheme, rest) = match src.split_once("://") {
        Some((s, r)) => (Some(s), r),
        None => (None, src),
    };
    match want_scheme {
        Some(s) => {
            if !scheme_ok(s) {
                return false;
            }
        }
        None => {
            // No scheme: the page's own scheme (or its secure upgrade).
            let base = page.map(|p| p.scheme()).unwrap_or("https");
            if !(scheme_ok(base) || (!matches!(base, "http" | "https") && network)) {
                return false;
            }
        }
    }
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], Some(&rest[i..])),
        None => (rest, None),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (h, Some(p)),
        _ => (hostport, None),
    };
    let Some(uhost) = u.host_str() else { return false };
    let uhost = uhost.to_ascii_lowercase();
    let host_ok = if host == "*" {
        true
    } else if let Some(suffix) = host.strip_prefix("*.") {
        uhost.ends_with(&format!(".{suffix}"))
    } else {
        uhost == host
    };
    if !host_ok {
        return false;
    }
    let uport = u.port_or_known_default();
    let port_ok = match port {
        Some("*") => true,
        Some(p) => p.parse::<u16>().ok() == uport,
        None => {
            uport == default_port(want_scheme.unwrap_or(scheme)) || (want_scheme.is_none_or(|s| s == "http") && scheme == "https" && uport == Some(443))
        }
    };
    if !port_ok {
        return false;
    }
    match path {
        None | Some("/") | Some("") => true,
        Some(p) => {
            let p = crate::net::percent_decode_str(p);
            let up = crate::net::percent_decode_str(u.path());
            if p.ends_with('/') { up.starts_with(&p) } else { up == p }
        }
    }
}

// ---------------------------------------------------------------- per-page registry

/// What a page loaded with: its CSP and framing headers.
#[derive(Clone, Debug, Default)]
pub struct PagePolicy {
    pub csp: Csp,
    /// `X-Frame-Options`, upper-cased.
    pub frame_options: Option<String>,
    /// Sandbox flags for this document (from `<iframe sandbox>` or a CSP `sandbox` directive).
    pub sandbox: Option<Sandbox>,
}

impl PagePolicy {
    pub fn from_headers(page: &str, headers: &[(String, String)], meta: &[String]) -> PagePolicy {
        let csp = Csp::new(page, headers, meta);
        let frame_options = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("x-frame-options")).map(|(_, v)| v.trim().to_ascii_uppercase());
        let sandbox = csp.sandbox();
        PagePolicy { csp, frame_options, sandbox }
    }

    /// May a page at `parent` (whose own top-level page is `top`) show this document in a frame?
    pub fn may_be_framed_by(&self, page: &str, parent: &str, top: &str) -> bool {
        // frame-ancestors replaces X-Frame-Options when both are sent.
        let ancestors: Vec<&str> = if parent == top { vec![parent] } else { vec![parent, top] };
        if let Some(_) = self.csp.frame_ancestor_ok(parent) {
            let ok = ancestors.iter().all(|a| self.csp.frame_ancestor_ok(a) == Some(true));
            if !ok {
                log!("[csp] {page} refuses to be framed by {parent} (frame-ancestors)");
            }
            return ok;
        }
        let ok = match self.frame_options.as_deref().map(|v| v.split(',').next().unwrap_or("").trim().to_string()) {
            Some(v) if v == "DENY" => false,
            Some(v) if v == "SAMEORIGIN" => {
                let me = crate::net::origin_of(page);
                me != "null" && ancestors.iter().all(|a| crate::net::origin_of(a) == me)
            }
            _ => true,
        };
        if !ok {
            log!("[frame] {page} refuses to be framed by {parent} (X-Frame-Options)");
        }
        ok
    }
}

static REGISTRY: RwLock<Option<HashMap<String, Arc<PagePolicy>>>> = RwLock::new(None);

/// Remember the policy of the page at `url` so its fetches are checked against it.
pub fn register(url: &str, policy: Arc<PagePolicy>) {
    let mut g = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 256 {
        m.clear();
    }
    if policy.csp.is_empty() && policy.sandbox.is_none() {
        m.remove(url);
    } else {
        m.insert(url.to_string(), policy);
    }
}

/// The policy registered for a page, if it has one.
pub fn policy_for(url: &str) -> Option<Arc<PagePolicy>> {
    REGISTRY.read().ok()?.as_ref()?.get(url).cloned()
}

// ---------------------------------------------------------------- frames

/// What a framed document needs to know about where it's shown.
#[derive(Clone, Debug)]
pub struct FrameInfo {
    /// The top-level page the frame is in (cookies and storage are partitioned by its site).
    pub top: String,
    /// Restrictions from `<iframe sandbox>` (None = not sandboxed).
    pub sandbox: Option<Sandbox>,
}

static FRAMES: RwLock<Option<HashMap<String, FrameInfo>>> = RwLock::new(None);

/// Note that the document at `url` is being shown in a frame.
pub fn set_frame(url: &str, info: FrameInfo) {
    let mut g = FRAMES.write().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 256 {
        m.clear();
    }
    m.insert(url.to_string(), info);
}

/// The frame was drawn: its document no longer needs the entry.
pub fn clear_frame(url: &str) {
    if let Ok(mut g) = FRAMES.write() {
        if let Some(m) = g.as_mut() {
            m.remove(url);
        }
    }
}

pub fn frame_info(url: &str) -> Option<FrameInfo> {
    FRAMES.read().ok()?.as_ref()?.get(url).cloned()
}

// ---------------------------------------------------------------- iframe sandbox

/// `<iframe sandbox="allow-scripts allow-forms …">`: everything not allowed is off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sandbox {
    pub scripts: bool,
    pub same_origin: bool,
    pub forms: bool,
    pub popups: bool,
    pub top_navigation: bool,
    pub modals: bool,
}

impl Sandbox {
    pub fn parse(flags: &str) -> Sandbox {
        let mut s = Sandbox::default();
        for f in flags.split_ascii_whitespace() {
            match f.to_ascii_lowercase().as_str() {
                "allow-scripts" => s.scripts = true,
                "allow-same-origin" => s.same_origin = true,
                "allow-forms" => s.forms = true,
                "allow-popups" => s.popups = true,
                "allow-top-navigation" | "allow-top-navigation-by-user-activation" => s.top_navigation = true,
                "allow-modals" => s.modals = true,
                _ => {}
            }
        }
        s
    }

    /// A frame inside a sandboxed frame is at least as restricted.
    pub fn and(&self, other: &Sandbox) -> Sandbox {
        Sandbox {
            scripts: self.scripts && other.scripts,
            same_origin: self.same_origin && other.same_origin,
            forms: self.forms && other.forms,
            popups: self.popups && other.popups,
            top_navigation: self.top_navigation && other.top_navigation,
            modals: self.modals && other.modals,
        }
    }
}

// ---------------------------------------------------------------- fingerprinting

/// A per-site, per-session number for fingerprinting noise (0 when protection is off).
pub fn fingerprint_seed(top: &str) -> u32 {
    if !crate::settings::get().resist_fingerprinting {
        return 0;
    }
    use std::hash::{Hash, Hasher};
    static SESSION: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let session = *SESSION.get_or_init(|| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        t ^ (std::process::id() as u64).rotate_left(32)
    });
    let site = url::Url::parse(top).ok().and_then(|u| u.host_str().map(crate::cookies::site_of)).unwrap_or_default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    session.hash(&mut h);
    site.hash(&mut h);
    (h.finish() as u32) | 1
}

/// (Used by canvas readouts — getImageData, toDataURL — once <canvas> is built.)
#[allow(dead_code)]
/// Nudge pixel data (RGBA) the way canvas readouts are perturbed: about one value in a
/// hundred moves by ±1, chosen by `seed`. Invisible, but enough that a canvas "fingerprint"
/// differs between sites and sessions.
pub fn noise_rgba(seed: u32, data: &mut [u8]) {
    if seed == 0 {
        return;
    }
    let mut x = seed;
    for (i, v) in data.iter_mut().enumerate() {
        if i % 4 == 3 {
            continue; // leave transparency alone
        }
        // xorshift32
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        if x % 97 == 0 {
            *v = if x & 0x100 != 0 { v.saturating_add(1) } else { v.saturating_sub(1) };
        }
    }
}

// ---------------------------------------------------------------- what pages may reach

fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// The browser's own pages (settings, history…). about:blank is an ordinary empty page.
fn is_privileged(url: &str) -> bool {
    (url.starts_with("about:") && !url.starts_with("about:blank")) || url.starts_with("view-source:")
}

/// The browser's own pages, which web pages may not open, frame or link to.
pub fn is_privileged_url(url: &str) -> bool {
    is_privileged(&url.trim().to_ascii_lowercase())
}

/// May a page at `from` send the tab to `to` (a link, a form, a script)? Pages can't open the
/// browser's own pages (a link could otherwise change settings or clear data), local files,
/// or `data:` documents (often used for phishing). The address bar is not limited.
pub fn page_may_navigate(from: &str, to: &str) -> bool {
    let to_l = to.trim().to_ascii_lowercase();
    let from_trusted = is_privileged(from) || from.starts_with("about:home");
    let ok = if is_privileged(&to_l) {
        from_trusted
    } else if to_l.starts_with("file:") {
        from.starts_with("file:") || from_trusted
    } else if to_l.starts_with("data:") || to_l.starts_with("javascript:") {
        !is_web(from) && to_l.starts_with("data:")
    } else {
        true
    };
    if !ok {
        log!("[security] blocked a navigation from {from} to {}", to.chars().take(80).collect::<String>());
    }
    ok
}

/// May a page at `page` load (or read with fetch) the local file `url`? Web pages never can;
/// a local page can read files in its own folder and below, so an HTML file you download
/// can't read the rest of your home folder.
pub fn file_access_ok(page: &str, url: &str) -> bool {
    if !url.starts_with("file:") {
        return true;
    }
    if !page.starts_with("file:") {
        return is_privileged(page) || page.is_empty();
    }
    let dir_of = |u: &str| -> Option<std::path::PathBuf> {
        let path = crate::net::percent_decode_str(u.strip_prefix("file://")?.split(['?', '#']).next()?);
        let p = std::path::Path::new(&path);
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        Some(p)
    };
    match (dir_of(page).and_then(|p| p.parent().map(|d| d.to_path_buf())), dir_of(url)) {
        (Some(dir), Some(target)) => target.starts_with(&dir),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(v: &str) -> Vec<(String, String)> {
        vec![("content-security-policy".into(), v.into())]
    }

    #[test]
    fn sources() {
        let c = Csp::new("https://site.com/a", &h("default-src 'self'; img-src * data:; script-src 'self' https://cdn.example.com/js/ *.good.org:8443"), &[]);
        assert!(c.allows_url(Dest::Image, "https://anything.net/x.png", None, true));
        assert!(c.allows_url(Dest::Image, "data:image/png;base64,AA", None, true));
        assert!(c.allows_url(Dest::Style, "https://site.com/s.css", None, true));
        assert!(!c.allows_url(Dest::Style, "https://evil.com/s.css", None, true));
        assert!(c.allows_url(Dest::Script, "https://cdn.example.com/js/app.js", None, true));
        assert!(!c.allows_url(Dest::Script, "https://cdn.example.com/other.js", None, true));
        assert!(c.allows_url(Dest::Script, "https://a.good.org:8443/x.js", None, true));
        assert!(!c.allows_url(Dest::Script, "https://good.org:8443/x.js", None, true));
        assert!(!c.allows_url(Dest::Script, "https://a.good.org/x.js", None, true));
        assert!(!c.allows_url(Dest::Connect, "https://api.other.com/", None, true));
        // No policy: everything goes.
        let none = Csp::new("https://site.com/", &[], &[]);
        assert!(none.allows_url(Dest::Script, "https://x.com/a.js", None, true));
        assert!(none.allows_inline(Inline::Script, "alert(1)", None));
        assert!(none.allows_eval());
        // 'none'
        let c = Csp::new("https://site.com/", &h("object-src 'none'; frame-src https:"), &[]);
        assert!(!c.allows_url(Dest::Object, "https://site.com/x.swf", None, true));
        assert!(c.allows_url(Dest::Frame, "https://maps.example/", None, true));
        assert!(!c.allows_url(Dest::Frame, "http://maps.example/", None, true));
        // http page: 'self' also matches its https upgrade.
        let c = Csp::new("http://site.com/", &h("default-src 'self'"), &[]);
        assert!(c.allows_url(Dest::Image, "https://site.com/x.png", None, true));
    }

    #[test]
    fn inline_and_eval() {
        let c = Csp::new("https://s.com/", &h("script-src 'self' 'nonce-abc123'"), &[]);
        assert!(c.allows_inline(Inline::Script, "go()", Some("abc123")));
        assert!(!c.allows_inline(Inline::Script, "go()", Some("wrong")));
        assert!(!c.allows_inline(Inline::Script, "go()", None));
        assert!(!c.allows_inline(Inline::ScriptAttr, "go()", None));
        assert!(!c.allows_eval());
        // sha256 of "alert(1)" (from the CSP spec's examples).
        let c = Csp::new("https://s.com/", &h("script-src 'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI=' 'unsafe-inline'"), &[]);
        assert!(c.allows_inline(Inline::Script, "alert(1)", None));
        assert!(!c.allows_inline(Inline::Script, "alert(2)", None), "'unsafe-inline' is ignored next to a hash");
        let c = Csp::new("https://s.com/", &h("script-src 'unsafe-inline' 'unsafe-eval'"), &[]);
        assert!(c.allows_inline(Inline::ScriptAttr, "go()", None));
        assert!(c.allows_eval());
        // strict-dynamic: parser-inserted scripts need the nonce; script-added ones are trusted.
        let c = Csp::new("https://s.com/", &h("script-src 'nonce-n1' 'strict-dynamic' https:"), &[]);
        assert!(!c.allows_url(Dest::Script, "https://cdn.com/a.js", None, true));
        assert!(c.allows_url(Dest::Script, "https://cdn.com/a.js", Some("n1"), true));
        assert!(c.allows_url(Dest::Script, "https://cdn.com/b.js", None, false));
        // Report-only policies never block.
        let ro = Csp::new("https://s.com/", &[("content-security-policy-report-only".into(), "default-src 'none'".into())], &[]);
        assert!(ro.allows_url(Dest::Script, "https://x.com/a.js", None, true));
        // Two policies: both must allow.
        let two = Csp::new("https://s.com/", &[("content-security-policy".into(), "img-src *".into()), ("content-security-policy".into(), "img-src https://ok.com".into())], &[]);
        assert!(two.allows_url(Dest::Image, "https://ok.com/i.png", None, true));
        assert!(!two.allows_url(Dest::Image, "https://other.com/i.png", None, true));
        // A <meta> policy can't set frame-ancestors.
        let m = PagePolicy::from_headers("https://s.com/", &[], &["frame-ancestors 'none'".to_string()]);
        assert!(m.may_be_framed_by("https://s.com/", "https://evil.com/", "https://evil.com/"));
    }

    #[test]
    fn framing() {
        let deny = PagePolicy::from_headers("https://bank.com/", &[("x-frame-options".into(), "DENY".into())], &[]);
        assert!(!deny.may_be_framed_by("https://bank.com/", "https://bank.com/", "https://bank.com/"));
        let same = PagePolicy::from_headers("https://bank.com/x", &[("x-frame-options".into(), "sameorigin".into())], &[]);
        assert!(same.may_be_framed_by("https://bank.com/x", "https://bank.com/", "https://bank.com/"));
        assert!(!same.may_be_framed_by("https://bank.com/x", "https://evil.com/", "https://evil.com/"));
        let fa = PagePolicy::from_headers("https://w.com/", &[("content-security-policy".into(), "frame-ancestors https://*.partner.com".into()), ("x-frame-options".into(), "DENY".into())], &[]);
        assert!(fa.may_be_framed_by("https://w.com/", "https://a.partner.com/", "https://a.partner.com/"));
        assert!(!fa.may_be_framed_by("https://w.com/", "https://evil.com/", "https://evil.com/"));
    }

    #[test]
    fn navigation_and_files() {
        assert!(!page_may_navigate("https://evil.com/", "about:settings?save=1&proxy_url=x"));
        assert!(!page_may_navigate("https://evil.com/", "file:///etc/passwd"));
        assert!(!page_may_navigate("https://evil.com/", "data:text/html,<h1>Bank</h1>"));
        assert!(!page_may_navigate("https://evil.com/", "view-source:https://x.com/"));
        assert!(page_may_navigate("https://evil.com/", "https://ok.com/"));
        assert!(page_may_navigate("https://evil.com/", "about:blank"));
        assert!(page_may_navigate("about:settings", "about:settings?clear_data"));
        assert!(page_may_navigate("about:home", "about:settings"));
        assert!(page_may_navigate("file:///home/u/a.html", "file:///home/u/b.html"));
        // Found by fuzzing: a policy with multi-byte text where a nonce prefix would end.
        let c = Csp::new("https://s.com/", &h("script-src 'nonce\u{fffd}x'"), &[]);
        assert!(!c.allows_inline(Inline::Script, "x", Some("x")));
        assert!(!file_access_ok("https://evil.com/", "file:///etc/passwd"));
        assert!(file_access_ok("file:///tmp/site/index.html", "file:///tmp/site/img/a.png"));
        assert!(!file_access_ok("file:///tmp/site/index.html", "file:///etc/passwd"));
        assert!(file_access_ok("https://x.com/", "https://y.com/"));
    }

    #[test]
    fn canvas_noise() {
        let base = vec![128u8; 4000];
        let mut a = base.clone();
        let mut b = base.clone();
        let mut a2 = base.clone();
        noise_rgba(12345, &mut a);
        noise_rgba(12345, &mut a2);
        noise_rgba(999, &mut b);
        assert_eq!(a, a2, "the same site sees the same values");
        assert_ne!(a, b, "another site sees different ones");
        let changed = a.iter().zip(&base).filter(|(x, y)| x != y).count();
        assert!(changed > 5 && changed < 100, "{changed} values changed");
        assert!(a.iter().zip(&base).all(|(x, y)| (*x as i32 - *y as i32).abs() <= 1));
        let mut c = base.clone();
        noise_rgba(0, &mut c);
        assert_eq!(c, base);
    }

    #[test]
    fn sandbox_flags() {
        let s = Sandbox::parse("allow-scripts allow-forms");
        assert!(s.scripts && s.forms && !s.same_origin && !s.popups);
        assert_eq!(Sandbox::parse(""), Sandbox::default());
        assert!(!s.and(&Sandbox::parse("allow-forms")).scripts);
    }
}
