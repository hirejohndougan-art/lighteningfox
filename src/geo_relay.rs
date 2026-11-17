//! Geolocation for pages in site processes, which are sandboxed away from D-Bus: the browser
//! process finds the position (geo.rs, with GeoClue or the location portal) and relays it.
//!
//! The renderer side stands in for geo.rs's requests. Starting and stopping a request are
//! requests to the browser over the renderer's pipe (process.rs): `geo_start`, `geo_stop`,
//! `geo_gone` (a page went away). Positions and failures come back through `geo_poll`, which
//! the browser answers as soon as it has something (or after a while with nothing), asked by
//! one thread per renderer while any request is open.
//!
//! The browser checks again that the page is one the renderer's process holds and that the
//! user allowed its site to know your location (the browser's own permission list, not the
//! renderer's copy). Requests are kept under a random token the renderer chose, bound to the
//! sites its process serves, so one renderer can't stop or read another's; a renderer that
//! stops polling (it exited) has its requests stopped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a `geo_poll` waits for something to report before answering with nothing.
const POLL_WAIT: Duration = Duration::from_secs(20);
/// Requests whose renderer hasn't polled for this long are stopped (the renderer is gone).
const ABANDONED: Duration = Duration::from_secs(90);
/// Open requests one renderer may have.
pub const MAX_REQUESTS: usize = 64;

// ---------------------------------------------------------------- renderer side

#[derive(Default)]
struct Renderer {
    /// Names this renderer's requests in the browser (random, chosen once).
    token: u64,
    /// Open requests: (page epoch, request id) → one-off?
    open: HashMap<(u64, u64), bool>,
    /// Requests that haven't had their first answer yet (they keep the page busy).
    unanswered: Vec<(u64, u64)>,
    polling: bool,
}

static RENDERER: LazyLock<Mutex<Renderer>> = LazyLock::new(|| {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    Mutex::new(Renderer { token: u64::from_le_bytes(b) | 1, ..Default::default() })
});

fn renderer() -> std::sync::MutexGuard<'static, Renderer> {
    RENDERER.lock().unwrap_or_else(|e| e.into_inner())
}

/// geo::start in a renderer: ask the browser.
pub fn start(epoch: u64, id: u64, once: bool, high: bool, max_age_ms: f64, url: &str, top: &str) {
    let token = {
        let mut r = renderer();
        r.open.insert((epoch, id), once);
        r.unanswered.retain(|k| *k != (epoch, id));
        r.unanswered.push((epoch, id));
        r.token
    };
    let answer = crate::process::relay_call(&[
        b"geo_start",
        token.to_string().as_bytes(),
        epoch.to_string().as_bytes(),
        id.to_string().as_bytes(),
        &[once as u8, high as u8],
        max_age_ms.to_string().as_bytes(),
        url.as_bytes(),
        top.as_bytes(),
    ]);
    match answer.as_deref().and_then(|a| a.first()).map(Vec::as_slice) {
        Some(b"ok") => start_polling(),
        Some(b"denied") => deliver(epoch, crate::geo::fail_code(id, 1, "User denied Geolocation")),
        other => {
            let why = other.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_else(|| "the browser didn't answer".into());
            log!("[geolocation] {why}");
            deliver(epoch, crate::geo::fail_code(id, 2, "The location service isn't available"));
        }
    }
}

/// geo::stop in a renderer.
pub fn stop(epoch: u64, id: u64) {
    let token = {
        let mut r = renderer();
        if r.open.remove(&(epoch, id)).is_none() {
            return;
        }
        r.unanswered.retain(|k| *k != (epoch, id));
        r.token
    };
    crate::process::relay_call(&[b"geo_stop", token.to_string().as_bytes(), epoch.to_string().as_bytes(), id.to_string().as_bytes()]);
}

/// geo::page_gone in a renderer.
pub fn page_gone(epoch: u64) {
    let token = {
        let mut r = renderer();
        let before = r.open.len();
        r.open.retain(|k, _| k.0 != epoch);
        r.unanswered.retain(|k| k.0 != epoch);
        if r.open.len() == before {
            return;
        }
        r.token
    };
    crate::process::relay_call(&[b"geo_gone", token.to_string().as_bytes(), epoch.to_string().as_bytes()]);
}

/// Is a relayed request waiting for its first answer? (geo::busy)
pub fn waiting() -> bool {
    RENDERER.lock().is_ok_and(|r| !r.unanswered.is_empty())
}

/// The request id a script call names: `__geoDone(<id>, …` or `__geoFail(<id>, …`, and
/// whether it's a failure.
fn id_of(code: &str) -> Option<(u64, bool)> {
    let (rest, failed) = match code.strip_prefix("__geoDone(") {
        Some(r) => (r, false),
        None => (code.strip_prefix("__geoFail(")?, true),
    };
    Some((rest[..rest.find(',')?].trim().parse().ok()?, failed))
}

/// Hand an answer from the browser to its page.
fn deliver(epoch: u64, code: String) {
    let Some((id, failed)) = id_of(&code) else { return };
    {
        let mut r = renderer();
        r.unanswered.retain(|k| *k != (epoch, id));
        match r.open.get(&(epoch, id)) {
            // One-off requests end with their answer, and every request with a failure.
            Some(&once) if once || failed => {
                r.open.remove(&(epoch, id));
            }
            Some(_) => {}
            None if !failed => return, // stopped meanwhile
            None => {}
        }
    }
    crate::geo::push(epoch, code);
}

fn start_polling() {
    {
        let mut r = renderer();
        if r.polling {
            return;
        }
        r.polling = true;
    }
    let spawned = std::thread::Builder::new().name("geolocation-relay".into()).spawn(|| loop {
        let token = {
            let mut r = renderer();
            if r.open.is_empty() {
                r.polling = false;
                return;
            }
            r.token
        };
        match crate::process::relay_call_waiting(&[b"geo_poll", token.to_string().as_bytes()], POLL_WAIT + Duration::from_secs(10)) {
            Some(answers) => {
                for pair in answers.chunks(2) {
                    let [epoch, code] = pair else { continue };
                    let Ok(epoch) = String::from_utf8_lossy(epoch).parse::<u64>() else { continue };
                    deliver(epoch, String::from_utf8_lossy(code).into_owned());
                }
            }
            // The browser didn't answer: the requests fail.
            None => {
                let open: Vec<(u64, u64)> = renderer().open.keys().copied().collect();
                for (epoch, id) in open {
                    deliver(epoch, crate::geo::fail_code(id, 2, "The location service isn't available"));
                }
            }
        }
    });
    if spawned.is_err() {
        renderer().polling = false;
    }
}

// ---------------------------------------------------------------- browser side

struct Relayed {
    /// The sites and top-level sites of the renderer that chose the token.
    owner: (Vec<String>, Vec<String>),
    last_poll: Instant,
    /// The renderer's page epochs → the keys geo.rs knows their requests by.
    keys: HashMap<u64, u64>,
    /// Open requests (key, id).
    requests: Vec<(u64, u64)>,
}

static RELAYED: LazyLock<Mutex<HashMap<u64, Relayed>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Keys for relayed pages in geo.rs, well apart from the browser's own page epochs.
static NEXT_KEY: AtomicU64 = AtomicU64::new(1 << 62);

fn relayed() -> std::sync::MutexGuard<'static, HashMap<u64, Relayed>> {
    RELAYED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Stop the requests of renderers that stopped polling (they exited or hung).
fn start_sweeping() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("geolocation-sweep".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_secs(30));
            sweep(ABANDONED);
        });
    });
}

fn sweep(after: Duration) {
    let gone: Vec<u64> = {
        let mut m = relayed();
        let tokens: Vec<u64> = m.iter().filter(|(_, r)| r.last_poll.elapsed() > after).map(|(t, _)| *t).collect();
        tokens.iter().filter_map(|t| m.remove(t)).flat_map(|r| r.keys.into_values()).collect()
    };
    for key in gone {
        crate::geo::page_gone(key);
    }
}

/// Answer a renderer's `geo_*` request. `owner` = its sites and top-level sites; `doc_ok`
/// says whether it holds a document at a URL shown in a top-level page.
pub(crate) fn serve(parts: &[Vec<u8>], owner: (Vec<String>, Vec<String>), doc_ok: impl Fn(&str, &str) -> bool) -> Vec<Vec<u8>> {
    let arg = |i: usize| parts.get(i).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    let num = |i: usize| arg(i).parse::<u64>().ok();
    let refused = || vec![b"not this process's requests".to_vec()];
    let Some(token) = num(1) else { return vec![b"bad request".to_vec()] };
    // Is the token this renderer's? The first to use it (with geo_start) owns it.
    let owned = |claim: bool| -> bool {
        let mut m = relayed();
        match m.get(&token) {
            Some(r) => r.owner == owner,
            None if claim => {
                m.insert(token, Relayed { owner: owner.clone(), last_poll: Instant::now(), keys: HashMap::new(), requests: Vec::new() });
                start_sweeping();
                true
            }
            None => false,
        }
    };
    match parts.first().map(Vec::as_slice) {
        Some(b"geo_start") => {
            let (Some(epoch), Some(id)) = (num(2), num(3)) else { return vec![b"bad request".to_vec()] };
            let flags = parts.get(4).cloned().unwrap_or_default();
            let (once, high) = (flags.first() == Some(&1), flags.get(1) == Some(&1));
            let max_age = arg(5).parse::<f64>().unwrap_or(0.0);
            let (url, top) = (arg(6), arg(7));
            if !doc_ok(&url, &top) {
                return vec![b"not this process's page".to_vec()];
            }
            // The browser's own permission list decides.
            if crate::permissions::state(&url, &top, crate::permissions::Kind::Location) != "granted" {
                return vec![b"denied".to_vec()];
            }
            if !owned(true) {
                return refused();
            }
            let key = {
                let mut m = relayed();
                let Some(r) = m.get_mut(&token) else { return refused() };
                if !r.requests.iter().any(|q| q.1 == id && r.keys.get(&epoch) == Some(&q.0)) && r.requests.len() >= MAX_REQUESTS {
                    return vec![b"too many location requests".to_vec()];
                }
                let key = *r.keys.entry(epoch).or_insert_with(|| NEXT_KEY.fetch_add(1, Ordering::SeqCst));
                r.requests.retain(|q| *q != (key, id));
                r.requests.push((key, id));
                key
            };
            crate::geo::start(key, id, once, high, max_age);
            vec![b"ok".to_vec()]
        }
        Some(b"geo_stop") if owned(false) => {
            let (Some(epoch), Some(id)) = (num(2), num(3)) else { return vec![b"bad request".to_vec()] };
            let key = {
                let mut m = relayed();
                let Some(r) = m.get_mut(&token) else { return refused() };
                let Some(&key) = r.keys.get(&epoch) else { return vec![b"ok".to_vec()] };
                r.requests.retain(|q| *q != (key, id));
                key
            };
            crate::geo::stop(key, id);
            vec![b"ok".to_vec()]
        }
        Some(b"geo_gone") if owned(false) => {
            let Some(epoch) = num(2) else { return vec![b"bad request".to_vec()] };
            let key = {
                let mut m = relayed();
                let Some(r) = m.get_mut(&token) else { return refused() };
                let key = r.keys.remove(&epoch);
                if let Some(k) = key {
                    r.requests.retain(|q| q.0 != k);
                }
                key
            };
            if let Some(k) = key {
                crate::geo::page_gone(k);
            }
            vec![b"ok".to_vec()]
        }
        Some(b"geo_poll") if owned(false) => {
            let until = Instant::now() + POLL_WAIT;
            loop {
                let keys: Vec<(u64, u64)> = {
                    let mut m = relayed();
                    let Some(r) = m.get_mut(&token) else { return Vec::new() };
                    r.last_poll = Instant::now();
                    // Requests geo.rs has finished (one-offs answered, failures) are done here too.
                    r.requests.retain(|q| crate::geo::watching(q.0));
                    r.keys.iter().map(|(e, k)| (*e, *k)).collect()
                };
                let mut out = Vec::new();
                for (epoch, key) in &keys {
                    for code in crate::geo::take_events(*key) {
                        out.push(epoch.to_string().into_bytes());
                        out.push(code.into_bytes());
                    }
                }
                let open = relayed().get(&token).is_some_and(|r| !r.requests.is_empty());
                if !out.is_empty() || !open || Instant::now() >= until {
                    return out;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        _ => refused(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(s: &str) -> (Vec<String>, Vec<String>) {
        (vec![s.to_string()], vec![s.to_string()])
    }

    fn req(parts: &[&[u8]]) -> Vec<Vec<u8>> {
        parts.iter().map(|p| p.to_vec()).collect()
    }

    #[test]
    fn answers_are_matched_to_their_requests() {
        assert_eq!(id_of("__geoDone(42, {\"latitude\":1})"), Some((42, false)));
        assert_eq!(id_of("__geoFail(7, 2, \"x\")"), Some((7, true)));
        assert_eq!(id_of("__wsEvent(1,\"open\")"), None);
    }

    /// The browser side: only allowed pages the renderer holds get positions, and only its
    /// own renderer can read or stop them.
    #[test]
    fn a_renderer_gets_positions_only_for_its_allowed_pages() {
        let _serial = crate::geo::TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: as in jsapi_tests::geolocation (the same place, and a scratch profile so the
        // permission isn't saved in yours).
        unsafe {
            std::env::set_var("IRONSWIFT_FAKE_LOCATION", "29.4608,-95.0513,30");
            if std::env::var_os("IRONSWIFT_PROFILE").is_none() {
                std::env::set_var("IRONSWIFT_PROFILE", std::env::temp_dir().join(format!("ironswift-jsapi-{}", std::process::id())));
            }
        }
        let page = "https://example.com/map";
        crate::permissions::set(page, crate::permissions::Kind::Location, true);
        let ok = |_: &str, _: &str| true;
        // A page the process doesn't hold, or one the user didn't allow, gets nothing.
        let r = serve(&req(&[b"geo_start", b"901", b"5", b"1", &[1, 0], b"0", page.as_bytes(), page.as_bytes()]), site("example.com"), |_, _| false);
        assert_eq!(r, req(&[b"not this process's page"]));
        let r = serve(&req(&[b"geo_start", b"901", b"5", b"1", &[1, 0], b"0", b"https://other.example/", b"https://other.example/"]), site("other.example"), ok);
        assert_eq!(r, req(&[b"denied"]));
        // An allowed page does; another renderer can't poll or stop its request.
        let r = serve(&req(&[b"geo_start", b"901", b"5", b"1", &[1, 0], b"0", page.as_bytes(), page.as_bytes()]), site("example.com"), ok);
        assert_eq!(r, req(&[b"ok"]));
        assert_eq!(serve(&req(&[b"geo_poll", b"901"]), site("evil.com"), ok), req(&[b"not this process's requests"]));
        assert_eq!(serve(&req(&[b"geo_stop", b"901", b"5", b"1"]), site("evil.com"), ok), req(&[b"not this process's requests"]));
        let r = serve(&req(&[b"geo_poll", b"901"]), site("example.com"), ok);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(r[0], b"5");
        assert!(String::from_utf8_lossy(&r[1]).starts_with("__geoDone(1, {\"latitude\":29.4608,\"longitude\":-95.0513,\"accuracy\":30"), "{r:?}");
        // The one-off request is finished; a watch keeps going until it's stopped.
        assert!(relayed()[&901].requests.is_empty());
        let r = serve(&req(&[b"geo_start", b"901", b"5", b"2", &[0, 0], b"0", page.as_bytes(), page.as_bytes()]), site("example.com"), ok);
        assert_eq!(r, req(&[b"ok"]));
        let r = serve(&req(&[b"geo_poll", b"901"]), site("example.com"), ok);
        assert!(String::from_utf8_lossy(&r[1]).starts_with("__geoDone(2,"), "{r:?}");
        assert_eq!(serve(&req(&[b"geo_gone", b"901", b"5"]), site("example.com"), ok), req(&[b"ok"]));
        assert!(!crate::geo::watching(relayed()[&901].keys.get(&5).copied().unwrap_or(1 << 62)));
        // A renderer that stops polling has its requests stopped.
        serve(&req(&[b"geo_start", b"901", b"6", b"3", &[0, 0], b"0", page.as_bytes(), page.as_bytes()]), site("example.com"), ok);
        let key = relayed()[&901].keys[&6];
        assert!(crate::geo::watching(key));
        sweep(Duration::ZERO);
        assert!(!crate::geo::watching(key));
        crate::permissions::forget(&crate::net::origin_of(page));
    }

    /// The real thing: the sandboxed renderer (`ironswift --frame-renderer <site>`, no D-Bus)
    /// shows a page the user allowed, which asks where it is and reports it with fetch(). This
    /// test plays the browser, answering the renderer's requests as the window does, with a
    /// made-up position. Needs the release binary (`cargo build --release`); skipped without it.
    #[test]
    fn a_sandboxed_renderer_gets_its_position_from_the_browser() {
        use crate::frames::{get, put};
        use std::io::{Read, Write};
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/ironswift");
        if !bin.exists() {
            eprintln!("no release binary; skipping");
            return;
        }
        let _serial = crate::geo::TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: as in jsapi_tests::geolocation.
        unsafe {
            std::env::set_var("IRONSWIFT_FAKE_LOCATION", "29.4608,-95.0513,30");
            if std::env::var_os("IRONSWIFT_PROFILE").is_none() {
                std::env::set_var("IRONSWIFT_PROFILE", std::env::temp_dir().join(format!("ironswift-jsapi-{}", std::process::id())));
            }
        }
        let web = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let web_port = web.local_addr().unwrap().port();
        let (seen_tx, seen) = std::sync::mpsc::channel::<String>();
        let page = "<!doctype html><script>navigator.geolocation.getCurrentPosition(\
                    (p) => fetch('/report?at=' + p.coords.latitude + ',' + p.coords.longitude),\
                    (e) => fetch('/report?error=' + e.code));</script>";
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
                let body = if path.starts_with("/report") { "ok" } else { page };
                let _ = seen_tx.send(path);
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        let url = format!("http://127.0.0.1:{web_port}/page.html");
        // The site is allowed: in the browser's list, and in the renderer's copy of it.
        crate::permissions::set(&url, crate::permissions::Kind::Location, true);
        let profile = std::env::temp_dir().join(format!("ironswift-geo-relay-{}", std::process::id()));
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("permissions.tsv"), format!("http://127.0.0.1:{web_port}\tgeolocation\tallow\n")).unwrap();
        let mut child = std::process::Command::new(&bin)
            .args(["--frame-renderer", "127.0.0.1"])
            .env("IRONSWIFT_PROFILE", &profile)
            .env_remove("IRONSWIFT_FAKE_LOCATION")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let stdin = std::sync::Arc::new(Mutex::new(child.stdin.take().unwrap()));
        let mut stdout = child.stdout.take().unwrap();
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
        crate::permissions::forget(&crate::net::origin_of(&url));
        let _ = std::fs::remove_dir_all(&profile);
        assert_eq!(result, "/report?at=29.4608,-95.0513", "requests seen before: {paths:?}");
    }
}
