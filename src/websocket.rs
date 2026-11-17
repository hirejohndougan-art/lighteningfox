//! WebSocket connections for page scripts (`new WebSocket(url)`).
//!
//! Each socket runs on its own thread: it connects (TLS for wss:), does the opening
//! handshake with the page's cookies and Origin, then passes messages both ways. What
//! happens on the socket is queued as script calls (`__wsEvent(...)`) that the page runs on
//! its next tick. The page's security rules apply: its Content Security Policy
//! (connect-src), the tracker blocklist, HSTS upgrades and the mixed-content rule. In a
//! sandboxed renderer the browser process opens the socket instead (ws_relay.rs).

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error, Message};

/// Something the page asked a socket to do.
pub enum Out {
    Text(String),
    Binary(Vec<u8>),
    /// close(code, reason); no code = an empty close frame.
    Close(Option<u16>, String),
    /// The page went away.
    Gone,
}

struct Handle {
    tx: Sender<Out>,
    buffered: Arc<AtomicUsize>,
}

/// Open sockets by (page epoch, socket id).
static SOCKETS: LazyLock<Mutex<HashMap<(u64, u64), Handle>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Script calls waiting for their page: (page epoch, code).
static EVENTS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());
/// Sockets still connecting (a headless render waits for them).
static CONNECTING: AtomicUsize = AtomicUsize::new(0);
/// When a page last sent something (a headless render waits a moment for the answer).
static LAST_SEND: Mutex<Option<Instant>> = Mutex::new(None);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Most a single incoming message may be (larger ones close the socket).
const MAX_MESSAGE: usize = 64 << 20;

pub(crate) fn push(epoch: u64, code: String) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push((epoch, code));
    }
}

fn js_str(s: &str) -> String {
    let mut o = String::new();
    crate::js::json_str(s, &mut o);
    o
}

/// A socket started (true) or finished (false) connecting (ws_relay.rs keeps the count for
/// relayed sockets).
pub(crate) fn connecting_changed(started: bool) {
    if started {
        CONNECTING.fetch_add(1, Ordering::SeqCst);
    } else {
        CONNECTING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Sockets open (or opening) for this epoch.
pub(crate) fn open_count(epoch: u64) -> usize {
    SOCKETS.lock().map(|s| s.keys().filter(|k| k.0 == epoch).count()).unwrap_or(0)
}

/// Queued script calls for the page with this epoch.
pub fn take_events(epoch: u64) -> Vec<String> {
    let Ok(mut q) = EVENTS.lock() else { return Vec::new() };
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q).into_iter().partition(|e| e.0 == epoch);
    *q = rest;
    mine.into_iter().map(|e| e.1).collect()
}

/// Anything a page is waiting on (connections being made, events not yet delivered, an
/// answer expected to something just sent)?
pub fn busy() -> bool {
    CONNECTING.load(Ordering::SeqCst) > 0
        || EVENTS.lock().is_ok_and(|q| !q.is_empty())
        || LAST_SEND.lock().is_ok_and(|t| t.is_some_and(|t| t.elapsed() < Duration::from_millis(500)))
}

/// Close every socket a page opened (it was closed or navigated away from).
pub fn page_gone(epoch: u64) {
    if crate::process::in_renderer() {
        crate::ws_relay::page_gone(epoch);
    }
    let Ok(mut s) = SOCKETS.lock() else { return };
    s.retain(|k, h| {
        if k.0 == epoch {
            let _ = h.tx.send(Out::Gone);
            false
        } else {
            true
        }
    });
    drop(s);
    if let Ok(mut q) = EVENTS.lock() {
        q.retain(|e| e.0 != epoch);
    }
}

pub fn send(epoch: u64, id: u64, msg: Out) {
    let len = match &msg {
        Out::Text(t) => t.len(),
        Out::Binary(b) => b.len(),
        _ => 0,
    };
    if len > 0
        && let Ok(mut t) = LAST_SEND.lock()
    {
        *t = Some(Instant::now());
    }
    // A sandboxed renderer has no network: the browser holds the socket.
    if crate::process::in_renderer() {
        return crate::ws_relay::send(epoch, id, msg);
    }
    if let Ok(s) = SOCKETS.lock()
        && let Some(h) = s.get(&(epoch, id))
    {
        h.buffered.fetch_add(len, Ordering::SeqCst);
        let _ = h.tx.send(msg);
    }
}

pub fn buffered(epoch: u64, id: u64) -> usize {
    SOCKETS.lock().ok().and_then(|s| s.get(&(epoch, id)).map(|h| h.buffered.load(Ordering::SeqCst))).unwrap_or(0)
}

/// Who is opening the socket.
pub struct Opener {
    pub epoch: u64,
    /// The page, for network rules ("about:sandbox" for an opaque sandboxed frame).
    pub page: String,
    pub top: String,
    /// The page's Content Security Policy allows connecting to this URL.
    pub csp_ok: bool,
}

/// Start connecting socket `id` to `url` (ws: or wss:, already checked by the script side).
pub fn open(who: Opener, id: u64, url: String, protocols: Vec<String>) {
    // A sandboxed renderer has no network: the browser opens the socket (ws_relay.rs).
    if crate::process::in_renderer() {
        return crate::ws_relay::open(who, id, url, protocols);
    }
    let (tx, rx) = mpsc::channel();
    let buffered = Arc::new(AtomicUsize::new(0));
    if let Ok(mut s) = SOCKETS.lock() {
        s.insert((who.epoch, id), Handle { tx, buffered: buffered.clone() });
    }
    CONNECTING.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new().name("websocket".into()).spawn(move || {
        let epoch = who.epoch;
        let mut connecting = true;
        let mut done_connecting = || {
            if std::mem::replace(&mut connecting, false) {
                CONNECTING.fetch_sub(1, Ordering::SeqCst);
            }
        };
        match connect(&who, &url, &protocols) {
            Ok((socket, protocol)) => {
                push(epoch, format!("__wsEvent({id},\"open\",{},\"\")", js_str(&protocol)));
                done_connecting();
                pump(epoch, id, socket, rx, buffered);
            }
            Err(e) => {
                log!("[websocket] {url}: {e}");
                push(epoch, format!("__wsEvent({id},\"error\")"));
                push(epoch, format!("__wsEvent({id},\"close\",1006,\"\",false)"));
                done_connecting();
            }
        }
        if let Ok(mut s) = SOCKETS.lock() {
            s.remove(&(epoch, id));
        }
    });
    if spawned.is_err() {
        CONNECTING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The http(s) URL the socket's handshake is made to (for cookies, HSTS and the blocklist).
fn http_form(url: &url::Url) -> String {
    let mut u = url.clone();
    let _ = u.set_scheme(if url.scheme() == "wss" { "https" } else { "http" });
    u.to_string()
}

type Socket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

fn connect(who: &Opener, url: &str, protocols: &[String]) -> Result<(Socket, String), String> {
    if !who.csp_ok {
        return Err("blocked by the page's Content Security Policy (connect-src)".into());
    }
    let mut u = url::Url::parse(url).map_err(|e| format!("bad URL: {e}"))?;
    if !matches!(u.scheme(), "ws" | "wss") {
        return Err("not a ws: or wss: URL".into());
    }
    // A site that asked for HTTPS only gets wss:.
    if u.scheme() == "ws" && crate::hsts::upgrade(&http_form(&u), Some(&who.top)).starts_with("https:") {
        let _ = u.set_scheme("wss");
    }
    let http_url = http_form(&u);
    if let Some(why) = crate::blocklist::blocked(&http_url, &who.page, crate::security::Dest::Connect) {
        return Err(why.to_string());
    }
    let host = u.host_str().ok_or("no host")?.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = u.port_or_known_default().unwrap_or(if u.scheme() == "wss" { 443 } else { 80 });
    let addrs: Vec<_> = (host.as_str(), port).to_socket_addrs().map_err(|e| format!("couldn't find {host}: {e}"))?.collect();
    let mut stream = None;
    let mut last = String::from("no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let stream = stream.ok_or_else(|| format!("couldn't connect to {host}:{port}: {last}"))?;
    let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
    let _ = stream.set_nodelay(true);

    use tungstenite::client::IntoClientRequest;
    let mut req = u.as_str().into_client_request().map_err(|e| e.to_string())?;
    let h = req.headers_mut();
    let origin = crate::net::origin_of(&who.page);
    let mut put = |k: &'static str, v: &str| {
        if let Ok(v) = tungstenite::http::HeaderValue::from_str(v) {
            h.insert(k, v);
        }
    };
    put("Origin", &origin);
    put("User-Agent", crate::net::USER_AGENT);
    if !protocols.is_empty() {
        put("Sec-WebSocket-Protocol", &protocols.join(", "));
    }
    if who.page != "about:sandbox"
        && let Some(c) = crate::cookies::header_for(&http_url, &who.top)
        // In a renderer that's the browser's placeholder, not cookies: never send it.
        && !c.starts_with(crate::process::COOKIE_TOKEN)
    {
        put("Cookie", &c);
    }
    let config = tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
    let (socket, resp) = tungstenite::client_tls_with_config(req, stream, Some(config), None).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(e) => e.to_string(),
        tungstenite::HandshakeError::Interrupted(_) => "the handshake timed out".into(),
    })?;
    let set_cookies: Vec<String> = resp.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok().map(String::from)).collect();
    if !set_cookies.is_empty() && who.page != "about:sandbox" {
        crate::cookies::receive(&http_url, &who.top, &set_cookies);
    }
    let chosen = resp.headers().get("sec-websocket-protocol").and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_string();
    if !chosen.is_empty() && !protocols.iter().any(|p| *p == chosen) {
        return Err(format!("the server picked a subprotocol that wasn't offered ({chosen})"));
    }
    Ok((socket, chosen))
}

fn tcp(socket: &mut Socket) -> Option<&TcpStream> {
    match socket.get_ref() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(s.get_ref()),
        _ => None,
    }
}

/// Pass messages both ways until the socket closes.
fn pump(epoch: u64, id: u64, mut socket: Socket, rx: Receiver<Out>, buffered: Arc<AtomicUsize>) {
    if let Some(s) = tcp(&mut socket) {
        let _ = s.set_read_timeout(Some(Duration::from_millis(15)));
    }
    let mut closing: Option<Instant> = None;
    let mut peer_close: (u16, String) = (1005, String::new());
    let mut gone = false;
    let close_event = |code: u16, reason: &str, clean: bool| {
        format!("__wsEvent({id},\"close\",{code},{},{clean})", js_str(reason))
    };
    loop {
        // What the page wants sent.
        loop {
            let msg = match rx.try_recv() {
                Ok(m) => m,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Out::Gone,
            };
            let r = match msg {
                Out::Text(t) => {
                    let n = t.len();
                    let r = socket.send(Message::text(t));
                    buffered.fetch_sub(n, Ordering::SeqCst);
                    r
                }
                Out::Binary(b) => {
                    let n = b.len();
                    let r = socket.send(Message::binary(b));
                    buffered.fetch_sub(n, Ordering::SeqCst);
                    r
                }
                Out::Close(code, reason) => {
                    closing.get_or_insert_with(Instant::now);
                    let frame = code.map(|c| CloseFrame { code: CloseCode::from(c), reason: reason.into() });
                    socket.close(frame)
                }
                Out::Gone => {
                    gone = true;
                    closing.get_or_insert_with(Instant::now);
                    socket.close(Some(CloseFrame { code: CloseCode::Away, reason: "".into() }))
                }
            };
            if let Err(e) = r
                && !is_wait(&e)
            {
                if !matches!(e, Error::AlreadyClosed | Error::ConnectionClosed) {
                    push(epoch, format!("__wsEvent({id},\"error\")"));
                }
                push(epoch, close_event(1006, "", false));
                return;
            }
            if gone {
                break;
            }
        }
        match socket.read() {
            Ok(Message::Text(t)) => {
                if !gone {
                    push(epoch, format!("__wsEvent({id},\"text\",{})", js_str(t.as_str())));
                }
            }
            Ok(Message::Binary(b)) => {
                if !gone {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&b);
                    push(epoch, format!("__wsEvent({id},\"binary\",\"{b64}\")"));
                }
            }
            Ok(Message::Close(frame)) => {
                if let Some(f) = frame {
                    peer_close = (u16::from(f.code), f.reason.to_string());
                }
                closing.get_or_insert_with(Instant::now);
                if !gone {
                    push(epoch, format!("__wsEvent({id},\"closing\")"));
                }
            }
            Ok(_) => {} // ping, pong (answered automatically)
            Err(e) if is_wait(&e) => {
                let _ = socket.flush();
            }
            Err(Error::ConnectionClosed) | Err(Error::AlreadyClosed) => {
                if !gone {
                    push(epoch, close_event(peer_close.0, &peer_close.1, true));
                }
                return;
            }
            Err(e) => {
                if !gone {
                    log!("[websocket] connection failed: {e}");
                    push(epoch, format!("__wsEvent({id},\"error\")"));
                    push(epoch, close_event(1006, "", false));
                }
                return;
            }
        }
        // A server that never finishes the closing handshake gets dropped.
        if closing.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
            if !gone {
                push(epoch, close_event(1006, "", false));
            }
            return;
        }
    }
}

fn is_wait(e: &Error) -> bool {
    matches!(e, Error::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_url_is_http_form() {
        let u = url::Url::parse("wss://example.com/chat?x=1").unwrap();
        assert_eq!(http_form(&u), "https://example.com/chat?x=1");
        let u = url::Url::parse("ws://example.com:8080/").unwrap();
        assert_eq!(http_form(&u), "http://example.com:8080/");
    }

    /// Echo server on localhost: open, text and binary echo, clean close.
    #[test]
    fn echo_round_trip() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(s).unwrap();
            loop {
                match ws.read() {
                    Ok(m @ (Message::Text(_) | Message::Binary(_))) => ws.send(m).unwrap(),
                    Ok(Message::Close(_)) => {
                        let _ = ws.flush();
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        let epoch = 900_001;
        open(Opener { epoch, page: "http://127.0.0.1/".into(), top: "http://127.0.0.1/".into(), csp_ok: true }, 1, format!("ws://127.0.0.1:{port}/"), vec![]);
        let wait_for = |what: &str| {
            let t = Instant::now();
            let mut seen = Vec::new();
            while t.elapsed() < Duration::from_secs(5) {
                seen.extend(take_events(epoch));
                if seen.iter().any(|e| e.contains(what)) {
                    return seen;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("never saw {what}: {seen:?}");
        };
        wait_for("\"open\"");
        send(epoch, 1, Out::Text("hello".into()));
        let ev = wait_for("\"text\"");
        assert!(ev.iter().any(|e| e == "__wsEvent(1,\"text\",\"hello\")"), "{ev:?}");
        send(epoch, 1, Out::Binary(vec![1, 2, 3]));
        let ev = wait_for("\"binary\"");
        assert!(ev.iter().any(|e| e == "__wsEvent(1,\"binary\",\"AQID\")"), "{ev:?}");
        send(epoch, 1, Out::Close(Some(1000), "bye".into()));
        let ev = wait_for("\"close\"");
        assert!(ev.iter().any(|e| e.starts_with("__wsEvent(1,\"close\",1000,\"bye\",true")), "{ev:?}");
    }

    #[test]
    fn refused_connection_reports_error_then_close() {
        let epoch = 900_002;
        open(Opener { epoch, page: "http://127.0.0.1/".into(), top: "http://127.0.0.1/".into(), csp_ok: false }, 7, "ws://127.0.0.1:9/".into(), vec![]);
        let t = Instant::now();
        let mut seen = Vec::new();
        while t.elapsed() < Duration::from_secs(5) && !seen.iter().any(|e: &String| e.contains("close")) {
            seen.extend(take_events(epoch));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(seen, vec!["__wsEvent(7,\"error\")".to_string(), "__wsEvent(7,\"close\",1006,\"\",false)".to_string()]);
    }
}
