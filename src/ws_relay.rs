//! WebSockets for pages in sandboxed renderers (site processes and the frame process), which
//! have no network: the browser process opens the socket and relays it.
//!
//! The renderer side stands in for websocket.rs's connections. Opening a socket, sending on
//! it and closing it are requests to the browser over the renderer's pipe (process.rs):
//! `ws_open`, `ws_send`. What happens on the socket (open, messages, errors, close) comes
//! back through `ws_poll`, a request the browser answers as soon as there is something to
//! report (or after a while with nothing), which one thread per renderer keeps asking while
//! any relayed socket is open.
//!
//! The browser side opens the socket with websocket.rs as for any page, so the page's
//! cookies are added there (the renderer never sees them), with HSTS, the tracker blocklist
//! and the mixed-content rule applied. Its sockets are kept under a random token the renderer
//! chose, bound to the sites the renderer's process serves: one renderer can't send on, read
//! or close another's sockets. A renderer may have `MAX_SOCKETS` open at once, and sockets
//! whose renderer stops asking for their events (it exited) are closed.

use crate::websocket::{self, Opener, Out};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Open sockets one renderer may have.
pub const MAX_SOCKETS: usize = 64;
/// How long a `ws_poll` waits for something to report before answering with nothing.
const POLL_WAIT: Duration = Duration::from_secs(20);
/// Sockets whose renderer hasn't polled for this long are closed (the renderer is gone).
const ABANDONED: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------- renderer side

#[derive(Default)]
struct Renderer {
    /// Names this renderer's sockets in the browser (random, chosen once).
    token: u64,
    next: u64,
    /// Relay id → (page epoch, the page's socket id).
    by_rid: HashMap<u64, (u64, u64)>,
    /// Sockets still connecting (no open or error yet).
    connecting: HashSet<u64>,
    polling: bool,
}

static RENDERER: LazyLock<Mutex<Renderer>> = LazyLock::new(|| {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    Mutex::new(Renderer { token: u64::from_le_bytes(b) | 1, ..Default::default() })
});

/// websocket::open in a renderer: ask the browser to open it.
pub fn open(who: Opener, id: u64, url: String, protocols: Vec<String>) {
    let (token, rid) = {
        let Ok(mut r) = RENDERER.lock() else { return };
        r.next += 1;
        let rid = r.next;
        r.by_rid.insert(rid, (who.epoch, id));
        r.connecting.insert(rid);
        (r.token, rid)
    };
    websocket::connecting_changed(true);
    let answer = crate::process::relay_call(&[
        b"ws_open",
        token.to_string().as_bytes(),
        rid.to_string().as_bytes(),
        url.as_bytes(),
        protocols.join("\n").as_bytes(),
        who.page.as_bytes(),
        who.top.as_bytes(),
        &[who.csp_ok as u8],
    ]);
    match answer.as_deref().and_then(|a| a.first()).map(Vec::as_slice) {
        Some(b"ok") => start_polling(),
        other => {
            let why = other.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_else(|| "the browser didn't answer".into());
            log!("[websocket] {url}: {why}");
            deliver(rid, format!("__wsEvent({rid},\"error\")"));
            deliver(rid, format!("__wsEvent({rid},\"close\",1006,\"\",false)"));
        }
    }
}

/// websocket::send in a renderer.
pub fn send(epoch: u64, id: u64, msg: Out) {
    let (token, rid) = {
        let Ok(mut r) = RENDERER.lock() else { return };
        let Some(rid) = r.by_rid.iter().find(|(_, v)| **v == (epoch, id)).map(|(k, _)| *k) else { return };
        if matches!(msg, Out::Gone) {
            r.by_rid.remove(&rid);
            if r.connecting.remove(&rid) {
                websocket::connecting_changed(false);
            }
        }
        (r.token, rid)
    };
    let (kind, data, reason): (&[u8], Vec<u8>, String) = match msg {
        Out::Text(t) => (b"t", t.into_bytes(), String::new()),
        Out::Binary(b) => (b"b", b, String::new()),
        Out::Close(code, reason) => (b"c", code.map(|c| c.to_string()).unwrap_or_default().into_bytes(), reason),
        Out::Gone => (b"g", Vec::new(), String::new()),
    };
    // Waited for, so a page's messages reach the socket in the order it sent them.
    crate::process::relay_call(&[b"ws_send", token.to_string().as_bytes(), rid.to_string().as_bytes(), kind, &data, reason.as_bytes()]);
}

/// websocket::page_gone in a renderer: close the page's sockets.
pub fn page_gone(epoch: u64) {
    let ids: Vec<u64> = RENDERER.lock().map(|r| r.by_rid.values().filter(|v| v.0 == epoch).map(|v| v.1).collect()).unwrap_or_default();
    for id in ids {
        send(epoch, id, Out::Gone);
    }
}

/// The relay id an event's script call names: `__wsEvent(<rid>,…`.
fn rid_of(code: &str) -> Option<(u64, &str)> {
    let rest = code.strip_prefix("__wsEvent(")?;
    let comma = rest.find(',')?;
    Some((rest[..comma].parse().ok()?, &rest[comma..]))
}

/// Hand an event from the browser to its page, named by the page's own socket id.
fn deliver(rid: u64, code: String) {
    let Some((_, rest)) = rid_of(&code) else { return };
    let (target, done_connecting) = {
        let Ok(mut r) = RENDERER.lock() else { return };
        let Some(&(epoch, id)) = r.by_rid.get(&rid) else { return };
        let done = r.connecting.remove(&rid);
        if rest.starts_with(",\"close\"") {
            r.by_rid.remove(&rid);
        }
        ((epoch, id), done)
    };
    websocket::push(target.0, format!("__wsEvent({}{rest}", target.1));
    if done_connecting {
        websocket::connecting_changed(false);
    }
}

fn start_polling() {
    {
        let Ok(mut r) = RENDERER.lock() else { return };
        if r.polling {
            return;
        }
        r.polling = true;
    }
    let spawned = std::thread::Builder::new().name("websocket-relay".into()).spawn(|| loop {
        let token = {
            let Ok(mut r) = RENDERER.lock() else { return };
            if r.by_rid.is_empty() {
                r.polling = false;
                return;
            }
            r.token
        };
        match crate::process::relay_call_waiting(&[b"ws_poll", token.to_string().as_bytes()], POLL_WAIT + Duration::from_secs(10)) {
            Some(events) => {
                for e in events {
                    let code = String::from_utf8_lossy(&e).into_owned();
                    if let Some((rid, _)) = rid_of(&code) {
                        deliver(rid, code);
                    }
                }
            }
            // The browser didn't answer: tell the pages their sockets are gone.
            None => {
                let rids: Vec<u64> = RENDERER.lock().map(|r| r.by_rid.keys().copied().collect()).unwrap_or_default();
                for rid in rids {
                    deliver(rid, format!("__wsEvent({rid},\"error\")"));
                    deliver(rid, format!("__wsEvent({rid},\"close\",1006,\"\",false)"));
                }
            }
        }
    });
    if spawned.is_err() {
        if let Ok(mut r) = RENDERER.lock() {
            r.polling = false;
        }
    }
}

// ---------------------------------------------------------------- browser side

struct Relayed {
    /// The sites and top-level sites of the renderer that chose the token.
    owner: (Vec<String>, Vec<String>),
    last_poll: Instant,
}

static RELAYED: LazyLock<Mutex<HashMap<u64, Relayed>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Is `token` this renderer's (claiming it if it's new)? `claim` only on `ws_open`.
fn owns(token: u64, owner: &(Vec<String>, Vec<String>), claim: bool) -> bool {
    let Ok(mut m) = RELAYED.lock() else { return false };
    match m.get_mut(&token) {
        Some(r) => r.owner == *owner,
        None if claim => {
            m.insert(token, Relayed { owner: owner.clone(), last_poll: Instant::now() });
            start_sweeping();
            true
        }
        None => false,
    }
}

/// Close the sockets of renderers that stopped polling (they exited or hung).
fn start_sweeping() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("websocket-sweep".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_secs(30));
            let gone: Vec<u64> = RELAYED.lock().map(|mut m| {
                let gone: Vec<u64> = m.iter().filter(|(_, r)| r.last_poll.elapsed() > ABANDONED).map(|(t, _)| *t).collect();
                for t in &gone {
                    m.remove(t);
                }
                gone
            }).unwrap_or_default();
            for t in gone {
                websocket::page_gone(t);
            }
        });
    });
}

/// Answer a renderer's `ws_*` request. `owner` = its sites and top-level sites; `doc_ok`
/// says whether it holds a document at a URL shown in a top-level page.
pub(crate) fn serve(parts: &[Vec<u8>], owner: (Vec<String>, Vec<String>), doc_ok: impl Fn(&str, &str) -> bool) -> Vec<Vec<u8>> {
    let arg = |i: usize| parts.get(i).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    let num = |i: usize| arg(i).parse::<u64>().ok();
    let (Some(token), kind) = (num(1), parts.first().map(Vec::as_slice)) else { return vec![b"bad request".to_vec()] };
    match kind {
        Some(b"ws_open") => {
            let (Some(rid), url, protocols, page, top) = (num(2), arg(3), arg(4), arg(5), arg(6)) else { return vec![b"bad request".to_vec()] };
            if !doc_ok(&page, &top) {
                return vec![b"not this process's page".to_vec()];
            }
            if !owns(token, &owner, true) {
                return vec![b"not this process's sockets".to_vec()];
            }
            if websocket::open_count(token) >= MAX_SOCKETS {
                return vec![b"too many open sockets".to_vec()];
            }
            let csp_ok = parts.get(7).is_some_and(|b| b.first() == Some(&1));
            let protocols = protocols.split('\n').filter(|p| !p.is_empty()).map(String::from).collect();
            websocket::open(Opener { epoch: token, page, top, csp_ok }, rid, url, protocols);
            vec![b"ok".to_vec()]
        }
        Some(b"ws_send") if owns(token, &owner, false) => {
            let Some(rid) = num(2) else { return vec![b"bad request".to_vec()] };
            let data = parts.get(4).cloned().unwrap_or_default();
            let msg = match parts.get(3).map(Vec::as_slice) {
                Some(b"t") => Out::Text(String::from_utf8_lossy(&data).into_owned()),
                Some(b"b") => Out::Binary(data),
                Some(b"c") => Out::Close(String::from_utf8_lossy(&data).parse().ok(), arg(5)),
                _ => Out::Gone,
            };
            websocket::send(token, rid, msg);
            vec![b"ok".to_vec()]
        }
        Some(b"ws_poll") if owns(token, &owner, false) => {
            let until = Instant::now() + POLL_WAIT;
            loop {
                if let Ok(mut m) = RELAYED.lock() {
                    if let Some(r) = m.get_mut(&token) {
                        r.last_poll = Instant::now();
                    }
                }
                let events = websocket::take_events(token);
                if !events.is_empty() || Instant::now() >= until || websocket::open_count(token) == 0 {
                    return events.into_iter().map(String::into_bytes).collect();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        _ => vec![b"not this process's sockets".to_vec()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(s: &str) -> (Vec<String>, Vec<String>) {
        (vec![s.to_string()], vec![s.to_string()])
    }

    #[test]
    fn event_ids_are_renamed_for_the_page() {
        assert_eq!(rid_of("__wsEvent(42,\"message\",\"hi\")"), Some((42, ",\"message\",\"hi\")")));
        assert_eq!(rid_of("other()"), None);
    }

    #[test]
    fn a_renderer_reaches_only_its_own_sockets() {
        let ok = |_: &str, _: &str| true;
        let req = |parts: &[&[u8]]| parts.iter().map(|p| p.to_vec()).collect::<Vec<_>>();
        // A page the process doesn't hold can't open a socket.
        let r = serve(&req(&[b"ws_open", b"777", b"1", b"ws://127.0.0.1:9/", b"", b"https://bank.com/", b"https://bank.com/", &[1]]), site("example.com"), |_, _| false);
        assert_eq!(r, vec![b"not this process's page".to_vec()]);
        // The first renderer to use a token owns it; another site's renderer can't use it.
        let r = serve(&req(&[b"ws_open", b"778", b"1", b"ws://127.0.0.1:9/", b"", b"https://example.com/", b"https://example.com/", &[1]]), site("example.com"), ok);
        assert_eq!(r, vec![b"ok".to_vec()]);
        let r = serve(&req(&[b"ws_send", b"778", b"1", b"t", b"hello"]), site("evil.com"), ok);
        assert_eq!(r, vec![b"not this process's sockets".to_vec()]);
        let r = serve(&req(&[b"ws_poll", b"778"]), site("evil.com"), ok);
        assert_eq!(r, vec![b"not this process's sockets".to_vec()]);
        let r = serve(&req(&[b"ws_open", b"778", b"2", b"ws://127.0.0.1:9/", b"", b"https://evil.com/", b"https://evil.com/", &[1]]), site("evil.com"), ok);
        assert_eq!(r, vec![b"not this process's sockets".to_vec()]);
        // The owner sees its socket's failure (nothing listens on port 9).
        let mut events = Vec::new();
        for _ in 0..50 {
            events.extend(serve(&req(&[b"ws_poll", b"778"]), site("example.com"), ok));
            if events.iter().any(|e| e.windows(7).any(|w| w == b"\"close\"")) {
                break;
            }
        }
        let text: Vec<String> = events.iter().map(|e| String::from_utf8_lossy(e).into_owned()).collect();
        assert!(text.iter().any(|e| e.starts_with("__wsEvent(1,\"close\",1006")), "{text:?}");
    }

    /// The real thing: the sandboxed frame renderer (`ironswift --frame-renderer`, with no
    /// network) loads a page that opens a WebSocket, sends "ping" and reports the echo with
    /// fetch(). This test plays the browser: it answers the renderer's requests as the window
    /// does. Needs the release binary (`cargo build --release`); skipped without it.
    #[test]
    fn a_sandboxed_renderer_talks_through_the_browser() {
        use crate::frames::{get, put};
        use std::io::{Read, Write};
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/ironswift");
        if !bin.exists() {
            eprintln!("no release binary; skipping");
            return;
        }
        // A WebSocket echo server.
        let ws = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ws_port = ws.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in ws.incoming().flatten() {
                std::thread::spawn(move || {
                    let Ok(mut ws) = tungstenite::accept(s) else { return };
                    while let Ok(m) = ws.read() {
                        if m.is_text() || m.is_binary() {
                            let _ = ws.send(m);
                        }
                    }
                });
            }
        });
        // A web server for the page; it reports what the page fetches.
        let web = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let web_port = web.local_addr().unwrap().port();
        let (seen_tx, seen) = std::sync::mpsc::channel::<String>();
        let page = format!(
            "<!doctype html><script>const ws = new WebSocket('ws://127.0.0.1:{ws_port}/');\
             ws.onopen = () => ws.send('ping');\
             ws.onmessage = (e) => {{ fetch('/report?got=' + e.data); ws.close(); }};\
             ws.onerror = () => fetch('/report?error');</script>"
        );
        std::thread::spawn(move || {
            for mut s in web.incoming().flatten() {
                let mut req = Vec::new();
                let mut b = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut b) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&b[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
                let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                let body = if path.starts_with("/report") { "ok".to_string() } else { page.clone() };
                let _ = seen_tx.send(path);
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        let profile = std::env::temp_dir().join(format!("ironswift-ws-relay-{}", std::process::id()));
        let mut child = std::process::Command::new(&bin)
            .arg("--frame-renderer")
            .env("IRONSWIFT_PROFILE", &profile)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let stdin = std::sync::Arc::new(Mutex::new(child.stdin.take().unwrap()));
        let mut stdout = child.stdout.take().unwrap();
        let url = format!("http://127.0.0.1:{web_port}/page.html");
        let mut m = Vec::new();
        put(&mut m, b"open");
        put(&mut m, &1u32.to_le_bytes());
        put(&mut m, url.as_bytes());
        put(&mut m, url.as_bytes());
        put(&mut m, url.as_bytes());
        put(&mut m, b"");
        put(&mut m, &300u32.to_le_bytes());
        put(&mut m, &150u32.to_le_bytes());
        stdin.lock().unwrap().write_all(&m).unwrap();
        // Answer its requests as the window would (frames.rs `isolated`).
        let auth = crate::process::Auth { sites: Vec::new(), tops: vec!["127.0.0.1".into()], storage: false, extensions: false };
        let answers = stdin.clone();
        std::thread::spawn(move || loop {
            let Ok(kind) = get(&mut stdout) else { return };
            let Ok(id) = get(&mut stdout) else { return };
            let id = u32::from_le_bytes(id.try_into().unwrap_or([0; 4]));
            match kind.as_slice() {
                b"upd" => {
                    if crate::frames::Update::decode(&mut stdout).is_err() {
                        return;
                    }
                }
                b"req" => {
                    let Ok(parts) = crate::process::get_list(&mut stdout) else { return };
                    let (auth, answers) = (auth.clone(), answers.clone());
                    std::thread::spawn(move || {
                        let answer = crate::process::serve_request(&auth, &parts);
                        let _ = answers.lock().unwrap().write_all(&crate::process::response(id, &answer));
                    });
                }
                _ => return,
            }
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut paths = Vec::new();
        let result = loop {
            match seen.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(p) if p.starts_with("/report") => break p,
                Ok(p) => paths.push(p),
                Err(_) => break String::from("(nothing)"),
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(result, "/report?got=ping", "requests seen before: {paths:?}");
    }
}
