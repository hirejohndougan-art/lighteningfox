//! The HTTP client: one shared, pooled client (connections are reused between requests),
//! HTTP/1.1 and HTTP/2, HTTP/3 for sites that advertise it, gzip/Brotli/zstd/deflate
//! decompression, a small DNS cache, the system's certificate store, and the proxy chosen in
//! settings.
//!
//! net.rs builds requests (cookies, redirects, CORS); this module only moves bytes.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A raw HTTP response, before cookies and redirects are handled.
pub struct Raw {
    pub status: u16,
    pub status_text: String,
    /// Lower-case names; repeated headers (set-cookie) appear once per value.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The largest body kept in memory (bigger things are downloads).
const MAX_BODY: u64 = 50_000_000;

// ---------------------------------------------------------------- DNS cache

/// Host lookups are kept for a minute: pages ask for the same few hosts dozens of times.
const DNS_TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
struct CachingResolver {
    cache: Mutex<HashMap<String, (Instant, Vec<SocketAddr>)>>,
}

impl reqwest::dns::Resolve for CachingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        if let Ok(c) = self.cache.lock() {
            if let Some((when, addrs)) = c.get(&host) {
                if when.elapsed() < DNS_TTL {
                    let addrs = addrs.clone();
                    return Box::pin(async move { Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs) });
                }
            }
        }
        let cache = resolver().clone();
        Box::pin(async move {
            // The system resolver (getaddrinfo) blocks, so it runs off the async threads. It
            // honors /etc/hosts and the system's DNS settings.
            let h = host.clone();
            let addrs: Vec<SocketAddr> = tokio::task::spawn_blocking(move || (h.as_str(), 0).to_socket_addrs().map(|a| a.collect::<Vec<_>>()))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            if let Ok(mut c) = cache.cache.lock() {
                if c.len() > 500 {
                    c.clear();
                }
                c.insert(host, (Instant::now(), addrs.clone()));
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn resolver() -> &'static Arc<CachingResolver> {
    static R: OnceLock<Arc<CachingResolver>> = OnceLock::new();
    R.get_or_init(|| Arc::new(CachingResolver::default()))
}

// ---------------------------------------------------------------- clients

/// The proxy and certificate choices a client was built with; a change rebuilds the clients.
fn client_key() -> String {
    let s = crate::settings::get();
    format!("{}|{}|{}", s.proxy_mode, s.proxy_url, std::env::var("IRONSWIFT_EXTRA_CA").unwrap_or_default())
}

fn build(http3: bool) -> Result<reqwest::blocking::Client, String> {
    let s = crate::settings::get();
    let mut b = reqwest::blocking::Client::builder()
        .user_agent(crate::net::USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(6)
        .dns_resolver(resolver().clone());
    b = match s.proxy_mode.as_str() {
        "none" => b.no_proxy(),
        "manual" if !s.proxy_url.trim().is_empty() => {
            let proxy = reqwest::Proxy::all(s.proxy_url.trim()).map_err(|e| format!("proxy setting: {e}"))?;
            // Local addresses never go through the proxy.
            b.proxy(proxy.no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1")))
        }
        // "system": the desktop's proxy settings and the http_proxy/https_proxy/no_proxy variables.
        _ => b,
    };
    // Extra trusted certificates (a company or test certificate authority), on top of the
    // system's certificate store.
    if let Ok(path) = std::env::var("IRONSWIFT_EXTRA_CA") {
        let pem = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| format!("{path}: {e}"))?;
        b = b.tls_certs_merge(certs);
    }
    if http3 {
        b = b.http3_prior_knowledge();
    }
    b.build().map_err(|e| {
        let mut msg = e.to_string();
        let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
        while let Some(s) = src {
            msg.push_str(": ");
            msg.push_str(&s.to_string());
            src = s.source();
        }
        msg
    })
}

fn build_h3_ipv4() -> Result<reqwest::blocking::Client, String> {
    let s = crate::settings::get();
    let mut b = reqwest::blocking::Client::builder()
        .user_agent(crate::net::USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .dns_resolver(resolver().clone())
        .http3_prior_knowledge();
    if s.proxy_mode == "none" {
        b = b.no_proxy();
    }
    if let Ok(path) = std::env::var("IRONSWIFT_EXTRA_CA") {
        if let Ok(certs) = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|p| reqwest::Certificate::from_pem_bundle(&p).map_err(|e| e.to_string())) {
            b = b.tls_certs_merge(certs);
        }
    }
    b.build().map_err(|e| e.to_string())
}

struct Clients {
    key: String,
    normal: reqwest::blocking::Client,
    h3: Option<reqwest::blocking::Client>,
}

static CLIENTS: Mutex<Option<Clients>> = Mutex::new(None);

fn client(http3: bool) -> Result<reqwest::blocking::Client, String> {
    let key = client_key();
    let mut guard = CLIENTS.lock().map_err(|_| "network unavailable")?;
    if guard.as_ref().is_none_or(|c| c.key != key) {
        *guard = Some(Clients { key, normal: build(false)?, h3: None });
    }
    let c = guard.as_mut().expect("clients");
    if http3 {
        if c.h3.is_none() {
            // QUIC opens an IPv6 socket by default; on a machine without IPv6 that fails, so
            // try again on IPv4.
            c.h3 = Some(build(true).or_else(|_| build_h3_ipv4())?);
        }
        return Ok(c.h3.clone().expect("h3 client"));
    }
    Ok(c.normal.clone())
}

// ---------------------------------------------------------------- HTTP/3 discovery

/// Hosts that announced HTTP/3 (`Alt-Svc: h3=":443"`), until when; and hosts where it failed.
static H3_HOSTS: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
static H3_BROKEN: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

fn host_port(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    Some(format!("{}:{}", u.host_str()?, u.port_or_known_default()?))
}

fn h3_known(url: &str) -> bool {
    if !url.starts_with("https://") || std::env::var("IRONSWIFT_HTTP3").is_ok_and(|v| v == "0") {
        return false;
    }
    let Some(hp) = host_port(url) else { return false };
    let broken = H3_BROKEN.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(&hp).copied()));
    if broken.is_some_and(|until| Instant::now() < until) {
        return false;
    }
    H3_HOSTS.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(&hp).copied())).is_some_and(|until| Instant::now() < until)
}

/// Remember a host's `Alt-Svc` announcement of HTTP/3 on the same port.
fn note_alt_svc(url: &str, alt_svc: &str) {
    let Some(hp) = host_port(url) else { return };
    let port = hp.rsplit(':').next().unwrap_or("443").to_string();
    for entry in alt_svc.split(',') {
        let entry = entry.trim();
        if entry == "clear" {
            if let Ok(mut m) = H3_HOSTS.lock() {
                m.get_or_insert_with(HashMap::new).remove(&hp);
            }
            return;
        }
        let mut parts = entry.split(';').map(str::trim);
        let Some(proto) = parts.next() else { continue };
        // h3=":443" (only same-host, same-port alternatives are used).
        if let Some(target) = proto.strip_prefix("h3=") {
            if target.trim_matches('"') != format!(":{port}") {
                continue;
            }
            let max_age = parts.find_map(|p| p.strip_prefix("ma=")).and_then(|v| v.parse::<u64>().ok()).unwrap_or(86_400);
            if let Ok(mut m) = H3_HOSTS.lock() {
                m.get_or_insert_with(HashMap::new).insert(hp.clone(), Instant::now() + Duration::from_secs(max_age.min(7 * 86_400)));
            }
            return;
        }
    }
}

// ---------------------------------------------------------------- requests

/// A response whose body hasn't been read yet (for streaming downloads).
pub struct Stream {
    pub status: u16,
    pub status_text: String,
    /// Lower-case names; repeated headers (set-cookie) appear once per value.
    pub headers: Vec<(String, String)>,
    /// The body still to read (from the network, or handed over whole by the browser to a
    /// site process).
    pub body: Box<dyn std::io::Read + Send>,
}

impl Stream {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Read the whole body (up to 50 MB).
    pub fn read_all(self, url: &str) -> Result<Raw, String> {
        let mut bytes = Vec::new();
        use std::io::Read;
        self.body.take(MAX_BODY).read_to_end(&mut bytes).map_err(|e| format!("[net] {url}: {e}"))?;
        Ok(Raw { status: self.status, status_text: self.status_text, headers: self.headers, body: bytes })
    }
}

/// One request, for the developer tools' network panel.
#[derive(Clone, Debug)]
pub struct Logged {
    pub at: Instant,
    pub method: String,
    pub url: String,
    /// The status code, or the error.
    pub result: Result<u16, String>,
    /// Time until the response headers arrived.
    pub took: Duration,
    pub content_type: String,
}

static REQUEST_LOG: std::sync::Mutex<std::collections::VecDeque<Logged>> = std::sync::Mutex::new(std::collections::VecDeque::new());

fn log_request(method: &str, url: &str, started: Instant, r: &Result<Stream, String>) {
    if let Ok(mut l) = REQUEST_LOG.lock() {
        if l.len() >= 500 {
            l.pop_front();
        }
        l.push_back(Logged {
            at: started,
            method: method.to_string(),
            url: url.to_string(),
            result: r.as_ref().map(|s| s.status).map_err(|e| e.clone()),
            took: started.elapsed(),
            content_type: r.as_ref().ok().and_then(|s| s.header("content-type")).unwrap_or("").to_string(),
        });
    }
}

/// Requests made since `since` (all kept ones if None), oldest first.
pub fn requests_since(since: Option<Instant>) -> Vec<Logged> {
    REQUEST_LOG.lock().map(|l| l.iter().filter(|r| since.is_none_or(|t| r.at >= t)).cloned().collect()).unwrap_or_default()
}

/// Send one request (no redirects followed) and return the response before reading its body.
/// Errors start with a tag the error pages read: `[dns]`, `[timeout]`, `[cert]`, `[refused]`,
/// `[proxy]` or `[net]`. `timeout` covers the whole transfer (long for downloads).
pub fn open(method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>, timeout: Duration) -> Result<Stream, String> {
    let started = Instant::now();
    // In a site process the browser makes the request (process.rs).
    if let Some(r) = crate::process::remote_http(method, url, headers, body, timeout) {
        return r.map(|raw| Stream { status: raw.status, status_text: raw.status_text, headers: raw.headers, body: Box::new(std::io::Cursor::new(raw.body)) });
    }
    // Extensions listening with webRequest may cancel or redirect it.
    match crate::extensions::webrequest::check(method, url, headers) {
        crate::extensions::webrequest::Decision::Proceed => {}
        crate::extensions::webrequest::Decision::Cancel => return Err(format!("{url}: blocked by an extension")),
        crate::extensions::webrequest::Decision::Redirect(to) => {
            return Ok(Stream { status: 307, status_text: "Temporary Redirect".into(), headers: vec![("location".into(), to)], body: Box::new(std::io::Cursor::new(Vec::new())) });
        }
    }
    let r = if let Some(dir) = crate::record::replaying() {
        match crate::record::load(&dir, method, url) {
            Some(s) => Ok(Stream { status: s.status, status_text: s.status_text, headers: s.headers, body: Box::new(std::io::Cursor::new(s.body)) }),
            None => Err(format!("[net] {url}: not in the recording")),
        }
    } else {
        let r = open_inner(method, url, headers, body, timeout);
        match (r, crate::record::recording()) {
            (Ok(st), Some(dir)) => {
                let bytes = crate::record::read_body(st.body, MAX_BODY).map_err(|e| format!("[net] {url}: {e}"))?;
                let saved = crate::record::Saved { status: st.status, status_text: st.status_text, headers: st.headers, body: bytes };
                crate::record::save(&dir, method, url, &saved);
                Ok(Stream { status: saved.status, status_text: saved.status_text, headers: saved.headers, body: Box::new(std::io::Cursor::new(saved.body)) })
            }
            (r, _) => r,
        }
    };
    log_request(method, url, started, &r);
    r
}

fn open_inner(method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>, timeout: Duration) -> Result<Stream, String> {
    if h3_known(url) {
        match client(true).and_then(|c| start(c, true, method, url, headers, body, timeout)) {
            Ok(r) => return Ok(r),
            Err(e) => {
                log!("[net] HTTP/3 failed, using HTTP/2: {e}");
                // Fall back to HTTP/2 and leave this host alone for a while.
                if let (Some(hp), Ok(mut m)) = (host_port(url), H3_BROKEN.lock()) {
                    m.get_or_insert_with(HashMap::new).insert(hp, Instant::now() + Duration::from_secs(600));
                }
            }
        }
    }
    let r = start(client(false)?, false, method, url, headers, body, timeout)?;
    if let Some(alt) = r.header("alt-svc") {
        note_alt_svc(url, alt);
    }
    Ok(r)
}

fn start(client: reqwest::blocking::Client, http3: bool, method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>, timeout: Duration) -> Result<Stream, String> {
    let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| format!("[net] bad method {method}"))?;
    let mut rb = client.request(m, url).timeout(timeout);
    if http3 {
        rb = rb.version(reqwest::Version::HTTP_3);
    }
    for (k, v) in headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body {
        rb = rb.body(b.to_vec());
    }
    let resp = rb.send().map_err(|e| classify(&e, url))?;
    let status = resp.status();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_ascii_lowercase(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    Ok(Stream {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body: Box::new(resp),
    })
}

/// Describe a failure in plain words, tagged for the error page.
fn classify(e: &reqwest::Error, url: &str) -> String {
    let mut chain = e.to_string();
    let mut src: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = src {
        chain.push_str(": ");
        chain.push_str(&s.to_string());
        src = s.source();
    }
    let low = chain.to_ascii_lowercase();
    let tag = if e.is_timeout() || low.contains("timed out") {
        "timeout"
    } else if low.contains("certificate") || low.contains("invalidcertificate") || low.contains("unknownissuer") || low.contains("tls handshake") {
        "cert"
    } else if low.contains("dns") || low.contains("failed to lookup") || low.contains("name or service not known") || low.contains("no address associated") || low.contains("nodename nor servname") {
        "dns"
    } else if low.contains("proxy") {
        "proxy"
    } else if low.contains("connection refused") {
        "refused"
    } else {
        "net"
    };
    format!("[{tag}] {url}: {chain}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_svc_announces_http3() {
        note_alt_svc("https://h3.example:443/x", "h2=\":443\"; ma=60, h3=\":443\"; ma=86400");
        assert!(h3_known("https://h3.example/y"));
        // Another port or another host isn't taken.
        note_alt_svc("https://other.example/", "h3=\":8443\"");
        assert!(!h3_known("https://other.example/"));
        note_alt_svc("https://h3.example/", "clear");
        assert!(!h3_known("https://h3.example/"));
    }
}
