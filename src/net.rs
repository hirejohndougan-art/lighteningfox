//! Networking: load http(s):// and file:// URLs, and resolve relative links.

pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) Ironswift/0.1";

/// Fetch a page (a top-level document) as text. Returns (final URL after redirects, body).
pub fn fetch(url: &str) -> Result<(String, String), String> {
    let r = load(url, None, None)?;
    let text = decode_html(&r.body, r.header("content-type"));
    Ok((r.url, text))
}

/// POST a form (application/x-www-form-urlencoded) as a page navigation.
pub fn post_form(url: &str, body: &str) -> Result<(String, String), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        // Local files can't take a POST; show them as a GET with the data in the query.
        return fetch(&format!("{}?{body}", url.split('?').next().unwrap_or(url)));
    }
    let r = load(url, Some(body), None)?;
    let text = decode_html(&r.body, r.header("content-type"));
    Ok((r.url, text))
}

/// Fetch something a page uses (stylesheet, script, image) as text. `page` is the page's URL,
/// which decides whether cookies go along (only to the page's own site).
pub fn fetch_sub(url: &str, page: &str) -> Result<(String, String), String> {
    let r = load(url, None, Some(page))?;
    let text = decode_text(&r.body, r.header("content-type"));
    Ok((r.url, text))
}

/// Like `fetch_sub`, as raw bytes (images).
pub fn fetch_sub_bytes(url: &str, page: &str) -> Result<(String, Vec<u8>), String> {
    load(url, None, Some(page)).map(|r| (r.url, r.body))
}

/// What a navigation turned out to be.
pub enum Navigated {
    /// Something to show: the final URL and the HTML for it.
    Page { url: String, html: String },
    /// A file being saved to the Downloads folder.
    Download { name: String },
}

/// What kind of thing a response is, from its headers and first bytes.
#[derive(PartialEq, Debug)]
enum Kind {
    Html,
    Text,
    Image,
    Download,
}

fn kind_of(content_type: Option<&str>, disposition: Option<&str>, head: &[u8]) -> Kind {
    if disposition.is_some_and(|d| d.trim().to_ascii_lowercase().starts_with("attachment")) {
        return Kind::Download;
    }
    let ct = content_type.unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match ct.as_str() {
        "text/html" | "application/xhtml+xml" => Kind::Html,
        "text/plain" | "text/css" | "text/javascript" | "application/javascript" | "application/json" | "text/xml" | "application/xml" | "text/csv" | "text/markdown" => {
            if ct == "text/csv" { Kind::Download } else { Kind::Text }
        }
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp" | "image/x-icon" | "image/vnd.microsoft.icon" => Kind::Image,
        "" | "application/octet-stream" | "unknown/unknown" => sniff(head, ct.is_empty()),
        _ => Kind::Download,
    }
}

/// Look at the first bytes when the server didn't say what something is (the HTML standard's
/// "MIME sniffing", simplified).
fn sniff(head: &[u8], no_type: bool) -> Kind {
    let start: Vec<u8> = head.iter().skip_while(|b| b.is_ascii_whitespace()).take(16).map(u8::to_ascii_lowercase).collect();
    let html_starts: [&[u8]; 8] = [b"<!doctype html", b"<html", b"<head", b"<body", b"<script", b"<!--", b"<title", b"<p"];
    if html_starts.iter().any(|p| start.starts_with(p)) {
        return Kind::Html;
    }
    if head.starts_with(b"\x89PNG") || head.starts_with(b"\xff\xd8\xff") || head.starts_with(b"GIF8") || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP")) {
        return Kind::Image;
    }
    if no_type && !head.contains(&0) && std::str::from_utf8(head).is_ok() && !head.starts_with(b"%PDF") {
        return Kind::Text;
    }
    Kind::Download
}

/// A page showing text (or code, JSON…) as it is.
fn text_page(url: &str, text: &str) -> String {
    let name = url.rsplit('/').next().unwrap_or(url);
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<!DOCTYPE html><html><head><title>{}</title></head><body style=\"margin:12px\"><pre style=\"white-space:pre-wrap;font-size:13px\">{}</pre></body></html>", esc(name), esc(text))
}

/// A page showing one image, centered on a dark background.
fn image_page(url: &str, content_type: &str, bytes: &[u8]) -> String {
    use base64::Engine;
    let name = url.rsplit('/').next().unwrap_or(url).replace('<', "&lt;");
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        "<!DOCTYPE html><html><head><title>{name}</title></head><body style=\"margin:0;background:#1f2023;min-height:100vh;display:flex;align-items:center;justify-content:center\">\
         <img src=\"data:{content_type};base64,{data}\" style=\"max-width:100%\"></body></html>"
    )
}

/// Navigate to a URL (a link, the address bar, a form): load it and decide whether to show it
/// or save it as a download.
pub fn navigate(url: &str, post: Option<&str>) -> Result<Navigated, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        // Local files and data: URLs: shown by type, never downloaded.
        let r = match post {
            Some(body) if url.starts_with("file://") => http(&Request { method: "GET", url: &format!("{}?{body}", url.split('?').next().unwrap_or(url)), headers: &[], body: None, top: None, cookies: true, extra: &[] })?,
            _ => http(&Request { method: "GET", url, headers: &[], body: None, top: None, cookies: true, extra: &[] })?,
        };
        let ct = r.header("content-type").map(str::to_string);
        let html = match kind_of(ct.as_deref(), None, &r.body[..r.body.len().min(1024)]) {
            Kind::Html => decode_html(&r.body, ct.as_deref()),
            Kind::Image => image_page(&r.url, ct.as_deref().unwrap_or("image/png"), &r.body),
            Kind::Text => text_page(&r.url, &decode_text(&r.body, ct.as_deref())),
            Kind::Download => return Err(format!("[type] {}: Ironswift can't show this kind of file", r.url)),
        };
        return Ok(Navigated::Page { url: r.url, html });
    }
    let headers = match post {
        Some(_) => vec![("content-type".to_string(), "application/x-www-form-urlencoded".to_string())],
        None => Vec::new(),
    };
    let req = Request { method: if post.is_some() { "POST" } else { "GET" }, url, headers: &headers, body: post.map(str::as_bytes), top: None, cookies: true, extra: &[] };
    let (final_url, st) = http_stream(&req)?;
    let ct = st.header("content-type").map(str::to_string);
    let disposition = st.header("content-disposition").map(str::to_string);
    let total = st.header("content-length").and_then(|v| v.trim().parse::<u64>().ok());
    let mut body = st.body;
    // The first bytes, to tell what it is when the headers don't.
    let mut head = vec![0u8; 1024];
    let mut n = 0;
    use std::io::Read;
    while n < head.len() {
        match body.read(&mut head[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) => return Err(format!("[net] {final_url}: {e}")),
        }
    }
    head.truncate(n);
    let kind = kind_of(ct.as_deref(), disposition.as_deref(), &head);
    // Error pages from servers (404…) are shown, not downloaded.
    let kind = if st.status >= 400 && kind == Kind::Download { Kind::Text } else { kind };
    if kind == Kind::Download {
        let name = crate::downloads::file_name(&final_url, disposition.as_deref());
        crate::downloads::start(&final_url, &name, total, head, body);
        return Ok(Navigated::Download { name });
    }
    let mut bytes = head;
    body.take(50_000_000).read_to_end(&mut bytes).map_err(|e| format!("[net] {final_url}: {e}"))?;
    let html = match kind {
        Kind::Html => decode_html(&bytes, ct.as_deref()),
        Kind::Image => image_page(&final_url, ct.as_deref().unwrap_or("image/png"), &bytes),
        _ => text_page(&final_url, &decode_text(&bytes, ct.as_deref())),
    };
    Ok(Navigated::Page { url: final_url, html })
}

/// A response for scripts: status, headers and body.
#[derive(Debug, Clone, Default)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub url: String,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// What a request is for, which decides cookies and redirects.
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: Option<&'a [u8]>,
    /// The page making the request; `None` means the request is a page navigation.
    pub top: Option<&'a str>,
    /// Send and store cookies (fetch's `credentials: "omit"` turns this off).
    pub cookies: bool,
    /// Headers the browser itself adds (Origin, preflight headers), never filtered.
    pub extra: &'a [(String, String)],
}

/// Load a URL: http(s) (with cookies and redirects), file:// or data:.
/// `top` is the page making the request; `None` means the request is the page itself.
fn load(url: &str, post: Option<&str>, top: Option<&str>) -> Result<Response, String> {
    let headers = match post {
        Some(_) => vec![("content-type".to_string(), "application/x-www-form-urlencoded".to_string())],
        None => Vec::new(),
    };
    http(&Request { method: if post.is_some() { "POST" } else { "GET" }, url, headers: &headers, body: post.map(str::as_bytes), top, cookies: true, extra: &[] })
}

// ---------------------------------------------------------------- text encodings

/// The charset named in a Content-Type header (`text/html; charset=Shift_JIS`).
fn charset_param(content_type: Option<&str>) -> Option<&'static encoding_rs::Encoding> {
    let ct = content_type?;
    let v = ct.split(';').find_map(|p| p.trim().strip_prefix("charset=").or_else(|| p.trim().strip_prefix("CHARSET=")))?;
    encoding_rs::Encoding::for_label(v.trim().trim_matches(|c| c == '"' || c == '\'').as_bytes())
}

/// `<meta charset=…>` or `<meta http-equiv=Content-Type content="…; charset=…">` in the first
/// 1024 bytes (the HTML standard's "prescan").
fn meta_charset(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let head = &bytes[..bytes.len().min(1024)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let mut i = 0;
    while let Some(p) = find(&lower[i..], b"<meta") {
        let start = i + p;
        let end = lower[start..].iter().position(|&c| c == b'>').map_or(lower.len(), |e| start + e);
        let tag = &lower[start..end];
        if let Some(c) = find(tag, b"charset") {
            let mut j = c + 7;
            while j < tag.len() && (tag[j] == b' ' || tag[j] == b'=') {
                j += 1;
            }
            while j < tag.len() && (tag[j] == b'"' || tag[j] == b'\'') {
                j += 1;
            }
            let label: Vec<u8> = tag[j..].iter().take_while(|&&c| !matches!(c, b'"' | b'\'' | b' ' | b';' | b'/' | b'>')).copied().collect();
            if let Some(enc) = encoding_rs::Encoding::for_label(&label) {
                // A page can't declare itself UTF-16 from inside an ASCII-compatible prefix.
                return Some(if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE { encoding_rs::UTF_8 } else { enc });
            }
        }
        i = end.max(start + 1);
        if i >= lower.len() {
            break;
        }
    }
    None
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Decode an HTML page: byte-order mark, then the server's charset, then `<meta charset>`,
/// then UTF-8 if the bytes are valid UTF-8, else Windows-1252 (the classic Western default).
pub fn decode_html(bytes: &[u8], content_type: Option<&str>) -> String {
    let enc = encoding_rs::Encoding::for_bom(bytes)
        .map(|(e, _)| e)
        .or_else(|| charset_param(content_type))
        .or_else(|| meta_charset(bytes))
        .unwrap_or(if std::str::from_utf8(bytes).is_ok() { encoding_rs::UTF_8 } else { encoding_rs::WINDOWS_1252 });
    enc.decode(bytes).0.into_owned()
}

/// Decode a stylesheet or script: byte-order mark, then the server's charset, then UTF-8.
pub fn decode_text(bytes: &[u8], content_type: Option<&str>) -> String {
    let enc = encoding_rs::Encoding::for_bom(bytes).map(|(e, _)| e).or_else(|| charset_param(content_type)).unwrap_or(encoding_rs::UTF_8);
    enc.decode(bytes).0.into_owned()
}

/// Make a request and return the response, whatever its status.
pub fn http(req: &Request) -> Result<Response, String> {
    let url = req.url;
    if let Some(rest) = url.strip_prefix("data:") {
        let ty = rest.split([',', ';']).next().unwrap_or("").to_string();
        return decode_data_url(rest).map(|b| Response { status: 200, status_text: "OK".into(), url: url.into(), headers: vec![("content-type".into(), ty)], body: b });
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        let path = url.strip_prefix("file://").unwrap_or(url);
        // The query and fragment aren't part of the file's name; keep them on the URL.
        let cut = path.find(['?', '#']).unwrap_or(path.len());
        let (path, rest) = path.split_at(cut);
        let path = percent_decode(path);
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        let abs = std::fs::canonicalize(&path).map(|p| p.display().to_string()).unwrap_or(path);
        let ty = guess_type(&abs);
        return Ok(Response { status: 200, status_text: "OK".into(), url: format!("file://{abs}{rest}"), headers: vec![("content-type".into(), ty.into())], body: bytes });
    }
    // GETs may come from the disk cache (fresh), or be checked with the server (stale).
    let cacheable = req.method.eq_ignore_ascii_case("GET") && req.body.is_none();
    let lookup = if cacheable { crate::cache::lookup(url) } else { crate::cache::Lookup::Miss };
    let (lookup, conditional) = match lookup {
        crate::cache::Lookup::Fresh(e) => {
            return Ok(Response { status: e.status, status_text: "OK".into(), url: url.into(), headers: e.headers, body: e.body });
        }
        crate::cache::Lookup::Revalidate(e, cond) => (Some(e), cond),
        crate::cache::Lookup::Miss => (None, Vec::new()),
    };
    let mut extra: Vec<(String, String)> = req.extra.to_vec();
    extra.extend(conditional);
    let r2 = Request { extra: &extra, ..*req };
    let (final_url, st) = http_stream(&r2)?;
    let raw = st.read_all(&final_url)?;
    if raw.status == 304 && lookup.is_some() {
        if let Some(e) = crate::cache::refreshed(url, &raw.headers) {
            return Ok(Response { status: e.status, status_text: "OK".into(), url: final_url, headers: e.headers, body: e.body });
        }
    }
    if cacheable && final_url == url {
        crate::cache::store(url, raw.status, &raw.headers, &raw.body);
    }
    Ok(Response { status: raw.status, status_text: raw.status_text, url: final_url, headers: raw.headers, body: raw.body })
}

/// Make a request over http(s), following redirects and handling cookies, and return the final
/// URL and the response before its body is read.
pub fn http_stream(req: &Request) -> Result<(String, crate::http::Stream), String> {
    // Redirects are followed here rather than by the HTTP library, so cookies set along the way
    // are kept and every hop goes through the same rules.
    let mut url = req.url.to_string();
    let mut method = req.method.to_ascii_uppercase();
    let mut body = req.body.map(<[u8]>::to_vec);
    let timeout = if req.top.is_none() && req.method.eq_ignore_ascii_case("GET") { 24 * 3600 } else { 30 };
    for _ in 0..20 {
        let first_party = req.top.unwrap_or(&url).to_string();
        let mut headers: Vec<(String, String)> = Vec::new();
        let accept = if req.top.is_none() { "text/html,application/xhtml+xml,*/*;q=0.8" } else { "*/*" };
        headers.push(("Accept".into(), accept.into()));
        headers.push(("Accept-Language".into(), "en-US,en;q=0.9".into()));
        // Global Privacy Control: "do not sell or share my data" (honored by law in some US states).
        if crate::settings::get().send_gpc {
            headers.push(("Sec-GPC".into(), "1".into()));
        }
        for (k, v) in req.headers {
            if !FORBIDDEN_HEADERS.contains(&k.to_ascii_lowercase().as_str()) {
                headers.push((k.clone(), v.clone()));
            }
        }
        headers.extend(req.extra.iter().cloned());
        if req.cookies {
            if let Some(c) = crate::cookies::header_for(&url, &first_party) {
                headers.push(("Cookie".into(), c));
            }
        }
        let resp = crate::http::open(&method, &url, &headers, body.as_deref(), std::time::Duration::from_secs(timeout))?;
        if req.cookies {
            let set: Vec<String> = resp.headers.iter().filter(|(k, _)| k == "set-cookie").map(|(_, v)| v.clone()).collect();
            crate::cookies::receive(&url, &first_party, &set);
        }
        let status = resp.status;
        if (300..400).contains(&status) && status != 304 {
            if let Some(loc) = resp.header("location") {
                url = resolve(&url, loc.trim());
                // 301/302/303 turn a POST into a GET; 307/308 repeat it.
                if !matches!(status, 307 | 308) && method != "GET" && method != "HEAD" {
                    method = "GET".into();
                    body = None;
                }
                continue;
            }
        }
        let mut resp = resp;
        resp.headers.retain(|(k, _)| k != "set-cookie");
        return Ok((url, resp));
    }
    Err("[net] too many redirects".into())
}

/// Headers pages may not set themselves.
const FORBIDDEN_HEADERS: &[&str] = &[
    "accept-charset", "accept-encoding", "access-control-request-headers", "access-control-request-method", "connection", "content-length",
    "cookie", "cookie2", "date", "dnt", "expect", "host", "keep-alive", "origin", "referer", "set-cookie", "te", "trailer",
    "transfer-encoding", "upgrade", "user-agent", "via", "sec-gpc",
];

fn guess_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "txt" => "text/plain",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// A script's request (fetch or XMLHttpRequest) from the page at `page`, with the
/// same-origin rules browsers apply: another origin's response is only readable if that
/// server allows it (CORS), and cookies only go along with `credentials: include`.
pub fn script_request(page: &str, method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>, credentials: &str) -> Result<Response, String> {
    let page_origin = origin_of(page);
    let target_origin = origin_of(url);
    let local_file = page.starts_with("file://") && url.starts_with("file://");
    let same = page_origin == target_origin && page_origin != "null" || local_file || url.starts_with("data:");
    if !same && !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("{url}: blocked (not allowed from this page)"));
    }
    let cookies = match credentials {
        "omit" => false,
        "include" => true,
        _ => same,
    };
    let method_up = method.to_ascii_uppercase();
    let mut extra: Vec<(String, String)> = Vec::new();
    if !same {
        extra.push(("Origin".into(), page_origin.clone()));
        // Anything beyond a simple GET/HEAD/POST needs the server's permission first.
        let simple_header = |(k, v): &(String, String)| match k.to_ascii_lowercase().as_str() {
            "accept" | "accept-language" | "content-language" => true,
            "content-type" => {
                let v = v.to_ascii_lowercase();
                v.starts_with("application/x-www-form-urlencoded") || v.starts_with("multipart/form-data") || v.starts_with("text/plain")
            }
            _ => false,
        };
        let complex: Vec<String> = headers.iter().filter(|h| !simple_header(h)).map(|(k, _)| k.to_ascii_lowercase()).collect();
        if !matches!(method_up.as_str(), "GET" | "HEAD" | "POST") || !complex.is_empty() {
            let mut pre = vec![("Origin".to_string(), page_origin.clone()), ("Access-Control-Request-Method".to_string(), method_up.clone())];
            if !complex.is_empty() {
                pre.push(("Access-Control-Request-Headers".into(), complex.join(",")));
            }
            let r = http(&Request { method: "OPTIONS", url, headers: &[], body: None, top: Some(page), cookies: false, extra: &pre })?;
            if !(200..300).contains(&r.status) || !cors_ok(&r, &page_origin, cookies) {
                return Err(format!("{url}: blocked by CORS (preflight refused)"));
            }
        }
    }
    let r = http(&Request { method: &method_up, url, headers, body, top: Some(page), cookies, extra: &extra })?;
    if !same && !cors_ok(&r, &page_origin, cookies) {
        return Err(format!("{url}: blocked by CORS (no Access-Control-Allow-Origin for {page_origin})"));
    }
    Ok(r)
}

fn cors_ok(r: &Response, origin: &str, credentials: bool) -> bool {
    match r.header("access-control-allow-origin").map(str::trim) {
        Some("*") => !credentials,
        Some(o) => o == origin && (!credentials || r.header("access-control-allow-credentials").is_some_and(|v| v.trim() == "true")),
        None => false,
    }
}

/// The origin (scheme://host:port) of a URL, as used by the same-origin policy. Hosts are
/// lower-cased and international names converted to their ASCII (punycode) form.
pub fn origin_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => u.origin().ascii_serialization(),
        _ => "null".into(),
    }
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

pub fn percent_decode_str(s: &str) -> String {
    percent_decode(s)
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
    if std::env::var("IRONSWIFT_KEEP_TRACKING_PARAMS").is_ok_and(|v| v == "1") || !crate::settings::get().strip_tracking || !(url.starts_with("http://") || url.starts_with("https://")) {
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
    if !s.contains(' ') {
        // example.com, bücher.de, localhost:8080, 192.168.1.1, [::1]:3000
        let host = s.split(['/', '?', '#']).next().unwrap_or("");
        let local = host == "localhost" || host.starts_with("localhost:") || host.starts_with('[');
        let has_port = host.rsplit_once(':').is_some_and(|(_, p)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        if host.contains('.') || local || has_port {
            let scheme = if local || host.starts_with("127.") || host.starts_with("192.168.") || host.starts_with("10.") { "http" } else { "https" };
            if let Ok(u) = url::Url::parse(&format!("{scheme}://{s}")) {
                return u.to_string();
            }
        }
    }
    let q: String = s
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    crate::settings::search_url(&q)
}

/// Resolve `href` relative to `base`, by the WHATWG URL standard (as browsers do): ports,
/// IPv6 addresses, international domain names, percent-encoding, `.` and `..` segments.
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.trim_matches(|c: char| c.is_ascii_whitespace() || c.is_control());
    if href.starts_with("about:") {
        return href.to_string();
    }
    let joined = match url::Url::parse(base) {
        Ok(b) => b.join(href),
        Err(_) => url::Url::parse(href),
    };
    match joined {
        Ok(u) => u.to_string(),
        // Something no URL parser accepts: keep it as written (it will fail to load).
        Err(_) => href.to_string(),
    }
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
        // WHATWG details: ports, IPv6, international names, spaces, backslashes.
        assert_eq!(resolve("https://a.com:443/x", "y"), "https://a.com/y");
        assert_eq!(resolve("http://[::1]:8080/a/", "b"), "http://[::1]:8080/a/b");
        assert_eq!(resolve("https://a.com/", "https://bücher.de/ä"), "https://xn--bcher-kva.de/%C3%A4");
        assert_eq!(resolve("https://a.com/x/", "a b.html"), "https://a.com/x/a%20b.html");
        assert_eq!(resolve("https://a.com/x/y", "..\\z"), "https://a.com/z");
        assert_eq!(resolve("https://a.com/p?q#f", "#g"), "https://a.com/p?q#g");
    }

    #[test]
    fn origins() {
        use super::origin_of;
        assert_eq!(origin_of("https://A.com:443/x"), "https://a.com");
        assert_eq!(origin_of("http://a.com:8080/"), "http://a.com:8080");
        assert_eq!(origin_of("https://bücher.de/"), "https://xn--bcher-kva.de");
        assert_eq!(origin_of("file:///x"), "null");
    }

    #[test]
    fn address_bar_input() {
        use super::normalize_input;
        assert_eq!(normalize_input("example.com"), "https://example.com/");
        assert_eq!(normalize_input("localhost:8080/x"), "http://localhost:8080/x");
        assert_eq!(normalize_input("bücher.de"), "https://xn--bcher-kva.de/");
        assert!(normalize_input("how do browsers work").contains("how+do+browsers+work"));
    }

    #[test]
    fn content_kinds() {
        use super::{kind_of, Kind};
        assert_eq!(kind_of(Some("text/html; charset=utf-8"), None, b""), Kind::Html);
        assert_eq!(kind_of(Some("application/pdf"), None, b"%PDF"), Kind::Download);
        assert_eq!(kind_of(Some("text/html"), Some("attachment; filename=x.html"), b""), Kind::Download);
        assert_eq!(kind_of(Some("application/json"), None, b"{}"), Kind::Text);
        assert_eq!(kind_of(Some("image/png"), None, b"\x89PNG"), Kind::Image);
        assert_eq!(kind_of(None, None, b"  <!DOCTYPE html><p>"), Kind::Html);
        assert_eq!(kind_of(Some("application/octet-stream"), None, b"\x89PNG\r\n"), Kind::Image);
        assert_eq!(kind_of(Some("application/octet-stream"), None, b"plain words"), Kind::Download);
        assert_eq!(kind_of(None, None, b"plain words"), Kind::Text);
        assert_eq!(kind_of(Some("application/zip"), None, b"PK"), Kind::Download);
    }

    #[test]
    fn encodings() {
        use super::decode_html;
        // Windows-1252 page with no declaration: "café"
        assert_eq!(decode_html(b"caf\xe9", None), "café");
        // Server says Shift_JIS: こんにちは
        let sjis = b"\x82\xb1\x82\xf1\x82\xc9\x82\xbf\x82\xcd";
        assert_eq!(decode_html(sjis, Some("text/html; charset=Shift_JIS")), "こんにちは");
        // <meta charset> in the page
        let mut page = b"<meta charset=\"euc-kr\"><p>".to_vec();
        page.extend_from_slice(b"\xbe\xc8\xb3\xe7");
        assert!(decode_html(&page, Some("text/html")).ends_with("안녕"));
        // Byte-order mark beats everything.
        assert_eq!(decode_html(b"\xef\xbb\xbfok", Some("text/html; charset=iso-8859-2")), "ok");
    }
}
