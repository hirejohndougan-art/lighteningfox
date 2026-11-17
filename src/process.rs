//! Site processes: each top-level site's pages are parsed, run and laid out in a process of
//! their own (`ironswift --site-renderer <site>`), so a page that finds a bug in the engine is
//! stuck in a sandbox with only its own site's data.
//!
//! The browser process keeps the window, the network, cookies and storage. A tab showing a
//! web page holds a small *shell* document — one full-window frame — and the page runs in its
//! site's process as a top-level frame host (frames.rs): input goes to it, pictures come back,
//! and its navigations, title changes and form posts go to the window, which loads the next
//! page in the right site's process.
//!
//! The site process can't open network connections, write files, start programs, or read the
//! cookie jar or the cache (sandbox.rs). It asks the browser instead, over its pipes:
//!
//! - **http**: one request (no redirects; net.rs follows them and adds cookies), answered
//!   with the whole response.
//! - **cookies** (the Cookie header, Set-Cookie headers, `document.cookie`) and **storage**
//!   (localStorage, IndexedDB) — only for its own site as the top-level site: a process for
//!   one site can't read another site's cookies or storage.
//! - **download**: a navigation turned out to be a file; the browser saves it.
//!
//! Wire format (frames.rs `put`/`get`): browser → process: `open <id> <url> <w> <h> <post?>
//! <type> <body>`, `event <id> <event>`, `close <id>`, `resp <rid> <parts>`; process →
//! browser: `upd <id> <update>`, `req <rid> <parts>`.
//!
//! The pool: a process per site while there's room — two per GiB of memory above the first
//! 2 GiB, 2 to 20 (`IRONSWIFT_SITE_PROCESS_MAX` overrides) — and while over 512 MiB is free.
//! Past that, a new site joins the process with the fewest pages (`allow <site>`), which may
//! then hold pages, cookies and storage of all its sites, as in Firefox's shared pool.
//!
//! On with `IRONSWIFT_SITE_PROCESS=1` (or the "Site isolation" setting).

use crate::frames::{get, put, Event, Update};
use crate::{cookies, forms, net, sandbox, FrameJob, FrameSrc};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Site processes are on (browser side).
static ON: AtomicBool = AtomicBool::new(false);
/// This is a site process.
static IN_RENDERER: AtomicBool = AtomicBool::new(false);

/// Turn site processes on if the setting or IRONSWIFT_SITE_PROCESS=1 asks for it (the window
/// calls this; snapshots and tests render in-process).
pub fn enable_from_settings() {
    let env = std::env::var("IRONSWIFT_SITE_PROCESS").ok();
    let on = match env.as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => crate::settings::get().site_processes,
    };
    if on {
        ON.store(true, Ordering::SeqCst);
        log!("[process] site processes on: each site's pages run in their own sandboxed process");
    }
}

/// The browser sends pages to site processes.
pub fn enabled() -> bool {
    ON.load(Ordering::SeqCst) && !IN_RENDERER.load(Ordering::SeqCst)
}

/// Running inside a site process (requests go to the browser).
pub fn in_renderer() -> bool {
    IN_RENDERER.load(Ordering::Relaxed)
}

/// The registrable domain ("site") of an http(s) address.
pub fn site_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    matches!(u.scheme(), "http" | "https").then_some(())?;
    u.host_str().map(cookies::site_of)
}

// ---------------------------------------------------------------- the shell

/// A token only the browser knows, so a web page can't mark its own iframes as shells.
fn nonce() -> &'static str {
    static N: OnceLock<String> = OnceLock::new();
    N.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        format!("{:016x}", h.finish())
    })
}

/// Form posts on their way to a site process (the first load of a tab), by address.
static POSTS: Mutex<Option<HashMap<String, forms::PostBody>>> = Mutex::new(None);

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The shell document for a web page, when site processes are on: (address, HTML).
pub fn shell_for(url: &str, post: Option<&forms::PostBody>) -> Option<(String, String)> {
    if !enabled() || site_of(url).is_none() {
        return None;
    }
    let target = if post.is_some() { url.to_string() } else { net::strip_tracking(url) };
    if let (Some(p), Ok(mut m)) = (post, POSTS.lock()) {
        m.get_or_insert_with(HashMap::new).insert(target.clone(), forms::PostBody { content_type: p.content_type.clone(), bytes: p.bytes.clone() });
    }
    let (w, h, ..) = crate::geometry::viewport();
    let (w, h) = (if w >= 1.0 { w } else { 1024.0 }, if h >= 1.0 { h } else { 700.0 });
    let u = escape(&target);
    let html = format!(
        "<!DOCTYPE html><html><head><meta charset=utf-8><title>{u}</title><style>html,body{{margin:0;padding:0;overflow:hidden;height:100%}}\
         iframe{{display:block;border:0;width:100vw;height:100vh}}</style></head><body><iframe data-ironswift-site=\"{n}\" src=\"{u}\" width=\"{w}\" height=\"{h}\"></iframe></body></html>",
        n = nonce(),
        w = w.round(),
        h = h.round()
    );
    Some((target, html))
}

/// Is this iframe the shell's frame for the page at `base`?
pub fn is_shell_frame(dom: &crate::dom::Dom, id: crate::dom::NodeId, base: &str) -> bool {
    enabled() && site_of(base).is_some() && dom.attr(id, "data-ironswift-site") == Some(nonce()) && dom.attr(id, "src") == Some(base)
}

/// The form body a tab's first load posts (handed over with `open`).
pub fn take_post(url: &str) -> Option<forms::PostBody> {
    POSTS.lock().ok()?.as_mut()?.remove(url)
}

// ---------------------------------------------------------------- message parts

fn put_list(m: &mut Vec<u8>, parts: &[&[u8]]) {
    put(m, &(parts.len() as u32).to_le_bytes());
    for p in parts {
        put(m, p);
    }
}

fn get_list(r: &mut impl std::io::Read) -> std::io::Result<Vec<Vec<u8>>> {
    let n = u32::from_le_bytes(get(r)?.try_into().map_err(|_| std::io::Error::other("bad count"))?) as usize;
    if n > 4096 {
        return Err(std::io::Error::other("too many parts"));
    }
    (0..n).map(|_| get(r)).collect()
}

fn get_u32(r: &mut impl std::io::Read) -> std::io::Result<u32> {
    Ok(u32::from_le_bytes(get(r)?.try_into().map_err(|_| std::io::Error::other("bad number"))?))
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Headers as "name\0value\n" lines (names and values never hold either).
fn headers_bytes(h: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in h {
        out.extend_from_slice(k.replace(['\0', '\n'], " ").as_bytes());
        out.push(0);
        out.extend_from_slice(v.replace(['\0', '\n'], " ").as_bytes());
        out.push(b'\n');
    }
    out
}

fn headers_from(b: &[u8]) -> Vec<(String, String)> {
    text(b).lines().filter_map(|l| l.split_once('\0').map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

// ---------------------------------------------------------------- browser side

/// One running site process and the sites it may hold pages of.
struct Proc {
    child: std::process::Child,
    stdin: Arc<Mutex<std::process::ChildStdin>>,
    /// Usually one site; more once the pool is full (see `limit`).
    sites: Vec<String>,
}

/// Running processes, by a number of their own.
static PROCS: Mutex<Option<HashMap<u32, Proc>>> = Mutex::new(None);
static NEXT_PROC: AtomicU32 = AtomicU32::new(1);
/// Each open page: its process, its site, and where its updates go.
static ROUTES: Mutex<Option<HashMap<u32, (u32, String, Sender<Update>)>>> = Mutex::new(None);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn write_to(stdin: &Mutex<std::process::ChildStdin>, msg: &[u8]) -> bool {
    stdin.lock().map(|mut s| s.write_all(msg).and_then(|_| s.flush()).is_ok()).unwrap_or(false)
}

/// /proc/meminfo: (total, available) in MiB.
fn memory_mib() -> Option<(u64, u64)> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| info.lines().find(|l| l.starts_with(name))?.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kib| kib / 1024);
    Some((field("MemTotal:")?, field("MemAvailable:")?))
}

/// How many site processes the pool may run (each costs roughly 60–150 MB): two per GiB above
/// the first 2 GiB, between 2 and 20; IRONSWIFT_SITE_PROCESS_MAX overrides it.
fn limit() -> usize {
    if let Some(n) = std::env::var("IRONSWIFT_SITE_PROCESS_MAX").ok().and_then(|v| v.trim().parse::<usize>().ok()) {
        return n.max(1);
    }
    memory_mib().map_or(8, |(total, _)| pool_size(total))
}

fn pool_size(total_mib: u64) -> usize {
    ((total_mib.saturating_sub(2048) / 512) as usize).clamp(2, 20)
}

/// Where a page of `site` goes: the process already holding the site; else a new process
/// while the pool has room and memory isn't short (under 512 MiB free); else the process with
/// the fewest pages, which then holds this site too.
fn choose(site: &str, procs: &HashMap<u32, Proc>, pages: &HashMap<u32, usize>, limit: usize, low_memory: bool) -> Option<u32> {
    if let Some((k, _)) = procs.iter().find(|(_, p)| p.sites.iter().any(|s| s == site)) {
        return Some(*k);
    }
    if procs.len() < limit && !(low_memory && !procs.is_empty()) {
        return None;
    }
    procs.keys().min_by_key(|k| (pages.get(k).copied().unwrap_or(0), **k)).copied()
}

/// The process to open a page of `site` in (started if need be): its number and input.
fn ensure(site: &str) -> Option<(u32, Arc<Mutex<std::process::ChildStdin>>)> {
    let mut g = PROCS.lock().ok()?;
    let procs = g.get_or_insert_with(HashMap::new);
    // Drop processes that have ended.
    procs.retain(|_, p| p.child.try_wait().ok().flatten().is_none());
    let pages: HashMap<u32, usize> = ROUTES.lock().ok().and_then(|r| r.as_ref().map(|r| r.values().fold(HashMap::new(), |mut m, (k, _, _)| {
        *m.entry(*k).or_insert(0) += 1;
        m
    }))).unwrap_or_default();
    let low = memory_mib().is_some_and(|(_, avail)| avail < 512);
    if let Some(k) = choose(site, procs, &pages, limit(), low) {
        let p = procs.get_mut(&k)?;
        if !p.sites.iter().any(|s| s == site) {
            // Sharing: tell the process it may load this site too.
            let mut m = Vec::new();
            put(&mut m, b"allow");
            put(&mut m, site.as_bytes());
            if !write_to(&p.stdin, &m) {
                return None;
            }
            p.sites.push(site.to_string());
            log!("[process] pool full: {site} shares process {k} with {}", p.sites[..p.sites.len() - 1].join(", "));
        }
        return Some((k, p.stdin.clone()));
    }
    let exe = std::env::current_exe().ok()?;
    let mut child = std::process::Command::new(exe)
        .arg("--site-renderer")
        .arg(site)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| log!("[process] couldn't start the process for {site}: {e}"))
        .ok()?;
    let stdin = Arc::new(Mutex::new(child.stdin.take()?));
    let mut stdout = child.stdout.take()?;
    let key = NEXT_PROC.fetch_add(1, Ordering::SeqCst);
    let stdin_r = stdin.clone();
    std::thread::Builder::new()
        .name(format!("site-{site}"))
        .spawn(move || {
            let r = (|| -> std::io::Result<()> {
                loop {
                    let kind = get(&mut stdout)?;
                    match kind.as_slice() {
                        b"upd" => {
                            let id = get_u32(&mut stdout)?;
                            let u = Update::decode(&mut stdout)?;
                            if let Ok(routes) = ROUTES.lock() {
                                // Only its own pages: a process can't speak for another's.
                                if let Some((k, _, tx)) = routes.as_ref().and_then(|r| r.get(&id)) {
                                    if *k == key {
                                        let _ = tx.send(u);
                                    }
                                }
                            }
                        }
                        b"req" => {
                            let rid = get_u32(&mut stdout)?;
                            let parts = get_list(&mut stdout)?;
                            let stdin = stdin_r.clone();
                            // Requests can take a while (the network): each on its own thread.
                            let _ = std::thread::Builder::new().name("site-request".into()).spawn(move || {
                                let sites = PROCS.lock().ok().and_then(|g| g.as_ref()?.get(&key).map(|p| p.sites.clone())).unwrap_or_default();
                                let answer = std::panic::catch_unwind(|| serve_request(&sites, &parts)).unwrap_or_default();
                                let refs: Vec<&[u8]> = answer.iter().map(Vec::as_slice).collect();
                                let mut m = Vec::new();
                                put(&mut m, b"resp");
                                put(&mut m, &rid.to_le_bytes());
                                put_list(&mut m, &refs);
                                write_to(&stdin, &m);
                            });
                        }
                        _ => return Err(std::io::Error::other("unknown message")),
                    }
                }
            })();
            if let Err(e) = r {
                log!("[process] site process {key} ended: {e}");
            }
            // Its pages keep their last picture; the next page of its sites starts afresh.
            if let Ok(mut r) = ROUTES.lock() {
                if let Some(r) = r.as_mut() {
                    r.retain(|_, (k, _, _)| *k != key);
                }
            }
            let gone = PROCS.lock().ok().and_then(|mut g| g.as_mut()?.remove(&key));
            if let Some(mut p) = gone {
                let _ = p.child.kill();
                let _ = p.child.wait();
            }
        })
        .ok()?;
    log!("[process] started process {key} for {site} (pid {}, {} of at most {})", child.id(), procs.len() + 1, limit());
    procs.insert(key, Proc { child, stdin: stdin.clone(), sites: vec![site.to_string()] });
    Some((key, stdin))
}

fn stdin_for(id: u32) -> Option<Arc<Mutex<std::process::ChildStdin>>> {
    let key = ROUTES.lock().ok()?.as_ref()?.get(&id)?.0;
    PROCS.lock().ok()?.as_ref()?.get(&key).map(|p| p.stdin.clone())
}

/// Open a tab's page in its site's process; its updates go to `updates`.
pub fn open(job: &FrameJob, updates: Sender<Update>) -> Option<u32> {
    let FrameSrc::Url(url) = &job.src else { return None };
    let site = site_of(url)?;
    let (key, stdin) = ensure(&site)?;
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    ROUTES.lock().ok()?.get_or_insert_with(HashMap::new).insert(id, (key, site, updates));
    let post = take_post(url);
    let mut m = Vec::new();
    put(&mut m, b"open");
    put(&mut m, &id.to_le_bytes());
    put(&mut m, url.as_bytes());
    put(&mut m, &job.w.to_le_bytes());
    put(&mut m, &job.h.to_le_bytes());
    put(&mut m, &[post.is_some() as u8]);
    put(&mut m, post.as_ref().map_or(&b""[..], |p| p.content_type.as_bytes()));
    put(&mut m, post.as_ref().map_or(&b""[..], |p| &p.bytes[..]));
    if !write_to(&stdin, &m) {
        close(id);
        return None;
    }
    Some(id)
}

pub fn event(id: u32, ev: &Event) {
    let Some(stdin) = stdin_for(id) else { return };
    let mut m = Vec::new();
    put(&mut m, b"event");
    put(&mut m, &id.to_le_bytes());
    m.extend(ev.encode());
    write_to(&stdin, &m);
}

/// Close a page; a process with no pages left is ended.
pub fn close(id: u32) {
    let stdin = stdin_for(id);
    let key = ROUTES.lock().ok().and_then(|mut r| r.as_mut()?.remove(&id)).map(|(k, _, _)| k);
    if let Some(stdin) = stdin {
        let mut m = Vec::new();
        put(&mut m, b"close");
        put(&mut m, &id.to_le_bytes());
        write_to(&stdin, &m);
    }
    let Some(key) = key else { return };
    let idle = ROUTES.lock().map(|r| r.as_ref().is_none_or(|r| r.values().all(|(k, _, _)| *k != key))).unwrap_or(false);
    if idle {
        // Closing its input ends it.
        let p = PROCS.lock().ok().and_then(|mut g| g.as_mut()?.remove(&key));
        if let Some(mut p) = p {
            drop(p.stdin);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                if p.child.try_wait().ok().flatten().is_none() {
                    let _ = p.child.kill();
                }
                let _ = p.child.wait();
            });
        }
    }
}

/// How many site processes are running (for tests and about:memory-style pages).
#[allow(dead_code)]
pub fn running() -> usize {
    PROCS.lock().map(|g| g.as_ref().map_or(0, |p| p.len())).unwrap_or(0)
}

/// Is `top` (the page asking) on one of the process's sites? A process speaks only for them.
fn mine(sites: &[String], top: &str) -> bool {
    site_of(top).is_some_and(|s| sites.contains(&s))
}

/// A storage key (js.rs `storage_key`) a site process may use: its sites' origins, or other
/// origins embedded in their pages ("<origin> in <site>").
fn key_ok(sites: &[String], key: &str) -> bool {
    match key.split_once(" in ") {
        Some((_, top)) => sites.iter().any(|s| s == top),
        None => site_of(key).is_some_and(|s| sites.contains(&s)),
    }
}

/// Answer one request from a process holding pages of `sites`.
fn serve_request(sites: &[String], parts: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let arg = |i: usize| parts.get(i).map(|b| text(b)).unwrap_or_default();
    let yes = |b: bool| vec![vec![b as u8]];
    match parts.first().map(Vec::as_slice) {
        Some(b"http") => {
            let (method, url) = (arg(1), arg(2));
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return vec![b"err".to_vec(), b"[net] site processes only fetch http(s)".to_vec()];
            }
            let headers = headers_from(parts.get(3).map_or(&[][..], Vec::as_slice));
            let body = (parts.get(4).is_some_and(|b| b.first() == Some(&1))).then(|| parts.get(5).cloned().unwrap_or_default());
            let secs = arg(6).parse::<u64>().unwrap_or(30).clamp(1, 24 * 3600);
            match crate::http::open(&method, &url, &headers, body.as_deref(), Duration::from_secs(secs)).and_then(|s| s.read_all(&url)) {
                Ok(r) => vec![b"ok".to_vec(), r.status.to_string().into_bytes(), r.status_text.into_bytes(), headers_bytes(&r.headers), r.body],
                Err(e) => vec![b"err".to_vec(), e.into_bytes()],
            }
        }
        Some(b"ext") => {
            // Only for pages of the process's own sites.
            let req: serde_json::Value = serde_json::from_str(&arg(1)).unwrap_or_default();
            let url = req.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if !site_of(url).is_some_and(|s| sites.contains(&s)) {
                return vec![br#"{"error":"not this process's page"}"#.to_vec()];
            }
            crate::extensions::start();
            vec![crate::extensions::serve(&req).to_string().into_bytes()]
        }
        Some(b"cookie_header") if mine(sites, &arg(2)) => match cookies::header_for(&arg(1), &arg(2)) {
            Some(h) => vec![vec![1], h.into_bytes()],
            None => vec![vec![0]],
        },
        Some(b"cookie_receive") if mine(sites, &arg(2)) => {
            let set: Vec<String> = parts.iter().skip(3).map(|b| text(b)).collect();
            cookies::receive(&arg(1), &arg(2), &set);
            yes(true)
        }
        Some(b"cookie_get") if mine(sites, &arg(2)) => vec![cookies::script_get(&arg(1), &arg(2)).into_bytes()],
        Some(b"cookie_set") if mine(sites, &arg(2)) => {
            cookies::script_set(&arg(1), &arg(2), &arg(3));
            yes(true)
        }
        Some(b"store_load") if matches!(arg(1).as_str(), "storage" | "indexeddb") && key_ok(sites, &arg(2)) => vec![crate::js::load_store_key(&arg(2), &arg(1)).into_bytes()],
        Some(b"store_save") if matches!(arg(1).as_str(), "storage" | "indexeddb") && key_ok(sites, &arg(2)) => {
            let quota = if arg(1) == "storage" { 5 * 1024 * 1024 } else { 100 * 1024 * 1024 };
            yes(crate::js::save_store_key(&arg(2), &arg(1), arg(3), quota))
        }
        Some(b"download") => {
            let url = arg(1);
            if site_of(&url).is_some() {
                download(&url);
            }
            yes(true)
        }
        // Anything else (or another site's data) is refused.
        _ => {
            log!("[process] refused a request from the process for {}", sites.join(", "));
            Vec::new()
        }
    }
}

/// Save a file a page navigated to (the browser fetches it again, with its cookies).
fn download(url: &str) {
    let url = url.to_string();
    let _ = std::thread::Builder::new().name("site-download".into()).spawn(move || {
        let req = net::Request { method: "GET", url: &url, headers: &[], body: None, top: None, cookies: true, extra: &[] };
        match net::http_stream(&req) {
            Ok((final_url, st)) => {
                let name = crate::downloads::file_name(&final_url, st.header("content-disposition"));
                let total = st.header("content-length").and_then(|v| v.trim().parse::<u64>().ok());
                crate::downloads::start(&final_url, &name, total, Vec::new(), st.body);
            }
            Err(e) => log!("[download] {url}: {e}"),
        }
    });
}

// ---------------------------------------------------------------- site process side

static OUT: OnceLock<Mutex<std::io::Stdout>> = OnceLock::new();
static PENDING: Mutex<Option<HashMap<u32, Sender<Vec<Vec<u8>>>>>> = Mutex::new(None);
static NEXT_RID: AtomicU32 = AtomicU32::new(1);

fn send_out(m: &[u8]) -> bool {
    OUT.get().and_then(|o| o.lock().ok()).map(|mut o| o.write_all(m).and_then(|_| o.flush()).is_ok()).unwrap_or(false)
}

/// Ask the browser and wait for the answer (None: no answer).
fn call(parts: &[&[u8]], wait: Duration) -> Option<Vec<Vec<u8>>> {
    let rid = NEXT_RID.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = channel();
    PENDING.lock().ok()?.get_or_insert_with(HashMap::new).insert(rid, tx);
    let mut m = Vec::new();
    put(&mut m, b"req");
    put(&mut m, &rid.to_le_bytes());
    put_list(&mut m, parts);
    let r = if send_out(&m) { rx.recv_timeout(wait).ok() } else { None };
    if let Ok(mut p) = PENDING.lock() {
        if let Some(p) = p.as_mut() {
            p.remove(&rid);
        }
    }
    r
}

/// http.rs: one request, made by the browser. None outside a site process.
pub fn remote_http(method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>, timeout: Duration) -> Option<Result<crate::http::Raw, String>> {
    if !in_renderer() {
        return None;
    }
    let hb = headers_bytes(headers);
    let secs = timeout.as_secs().max(1).to_string();
    let parts: [&[u8]; 7] = [b"http", method.as_bytes(), url.as_bytes(), &hb, &[body.is_some() as u8], body.unwrap_or(&[]), secs.as_bytes()];
    let wait = timeout.min(Duration::from_secs(600)) + Duration::from_secs(5);
    let Some(r) = call(&parts, wait) else { return Some(Err(format!("[net] {url}: the browser didn't answer"))) };
    Some(match r.first().map(Vec::as_slice) {
        Some(b"ok") if r.len() == 5 => Ok(crate::http::Raw {
            status: text(&r[1]).parse().unwrap_or(0),
            status_text: text(&r[2]),
            headers: headers_from(&r[3]),
            body: r[4].clone(),
        }),
        Some(b"err") => Err(r.get(1).map(|b| text(b)).unwrap_or_default()),
        _ => Err(format!("[net] {url}: refused")),
    })
}

const QUICK: Duration = Duration::from_secs(10);

/// cookies.rs `header_for`, asked of the browser. None outside a site process.
pub fn remote_cookie_header(url: &str, top: &str) -> Option<Option<String>> {
    in_renderer().then(|| call(&[b"cookie_header", url.as_bytes(), top.as_bytes()], QUICK).filter(|r| r.first().is_some_and(|b| b == &[1])).and_then(|r| r.get(1).map(|b| text(b))))
}

/// cookies.rs `receive`, sent to the browser. False outside a site process.
pub fn remote_cookie_receive(url: &str, top: &str, headers: &[String]) -> bool {
    if !in_renderer() {
        return false;
    }
    let mut parts: Vec<&[u8]> = vec![b"cookie_receive", url.as_bytes(), top.as_bytes()];
    parts.extend(headers.iter().map(|h| h.as_bytes()));
    call(&parts, QUICK);
    true
}

/// `document.cookie`. None outside a site process.
pub fn remote_cookie_get(url: &str, top: &str) -> Option<String> {
    in_renderer().then(|| call(&[b"cookie_get", url.as_bytes(), top.as_bytes()], QUICK).and_then(|r| r.first().map(|b| text(b))).unwrap_or_default())
}

/// `document.cookie = …`. False outside a site process.
pub fn remote_cookie_set(url: &str, top: &str, s: &str) -> bool {
    in_renderer() && {
        call(&[b"cookie_set", url.as_bytes(), top.as_bytes(), s.as_bytes()], QUICK);
        true
    }
}

/// A page's saved store (js.rs). None outside a site process.
pub fn remote_store_load(key: &str, dir: &str) -> Option<String> {
    in_renderer().then(|| call(&[b"store_load", dir.as_bytes(), key.as_bytes()], QUICK).and_then(|r| r.first().map(|b| text(b))).unwrap_or_else(|| "{}".into()))
}

/// Save a page's store (js.rs). None outside a site process.
pub fn remote_store_save(key: &str, dir: &str, json: &str) -> Option<bool> {
    in_renderer().then(|| call(&[b"store_save", dir.as_bytes(), key.as_bytes(), json.as_bytes()], QUICK).is_some_and(|r| r.first().is_some_and(|b| b == &[1])))
}

/// An extension request from a content script (extensions/mod.rs `serve`), as JSON. None
/// outside a site process.
pub fn remote_ext(json: &str) -> Option<String> {
    in_renderer().then(|| call(&[b"ext", json.as_bytes()], QUICK).and_then(|r| r.first().map(|b| text(b))).unwrap_or_default())
}

/// downloads.rs: a navigation is a file; the browser saves it. False outside a site process.
pub fn remote_download(url: &str) -> bool {
    in_renderer() && {
        call(&[b"download", url.as_bytes()], QUICK);
        true
    }
}

/// `ironswift --site-renderer <site>`: run the pages of one site the browser opens, one thread
/// each, until stdin closes.
pub fn serve(site: &str) {
    IN_RENDERER.store(true, Ordering::SeqCst);
    if site.is_empty() || !sandbox::protect_site_renderer() {
        log!("[process] refusing to run without a site and a sandbox");
        return;
    }
    if crate::load_fonts().is_none() {
        return;
    }
    let _ = OUT.set(Mutex::new(std::io::stdout()));
    let mut pages: HashMap<u32, Sender<Event>> = HashMap::new();
    // The sites this process may hold pages of: its own, and any the browser adds when the
    // pool is full.
    let mut sites = vec![site.to_string()];
    let mut input = std::io::stdin().lock();
    let r = (|| -> std::io::Result<()> {
        loop {
            let kind = get(&mut input)?;
            match kind.as_slice() {
                b"open" => {
                    let id = get_u32(&mut input)?;
                    let url = text(&get(&mut input)?);
                    let w = get_u32(&mut input)?.clamp(1, 8000);
                    let h = get_u32(&mut input)?.clamp(1, 8000);
                    let has_post = get(&mut input)?.first() == Some(&1);
                    let content_type = text(&get(&mut input)?);
                    let bytes = get(&mut input)?;
                    if !site_of(&url).is_some_and(|s| sites.contains(&s)) {
                        log!("[process] {site}: refused to open {url}");
                        continue;
                    }
                    if has_post {
                        POSTS.lock().map_err(|_| std::io::Error::other("lock"))?.get_or_insert_with(HashMap::new).insert(url.clone(), forms::PostBody { content_type, bytes });
                    }
                    let (reply, _) = channel();
                    let job = FrameJob { src: FrameSrc::Url(url.clone()), w, h, parent: url.clone(), top: url, sandbox: None, reply, top_level: true };
                    let (etx, erx) = channel();
                    pages.insert(id, etx);
                    let _ = std::thread::Builder::new().name("page".into()).stack_size(crate::MAIN_STACK).spawn(move || {
                        let send = |u: Update| {
                            let mut m = Vec::new();
                            put(&mut m, b"upd");
                            put(&mut m, &id.to_le_bytes());
                            m.extend(u.encode());
                            send_out(&m)
                        };
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::frames::run_host(job, erx, send)));
                    });
                }
                b"event" => {
                    let id = get_u32(&mut input)?;
                    let ev = Event::decode(&mut input)?;
                    if let Some(tx) = pages.get(&id) {
                        let _ = tx.send(ev);
                    }
                }
                b"close" => {
                    pages.remove(&get_u32(&mut input)?);
                }
                b"allow" => {
                    let s = text(&get(&mut input)?);
                    if !s.is_empty() && !sites.contains(&s) {
                        sites.push(s);
                    }
                }
                b"resp" => {
                    let rid = get_u32(&mut input)?;
                    let parts = get_list(&mut input)?;
                    let tx = PENDING.lock().ok().and_then(|mut p| p.as_mut()?.remove(&rid));
                    if let Some(tx) = tx {
                        let _ = tx.send(parts);
                    }
                }
                _ => return Err(std::io::Error::other("unknown message")),
            }
        }
    })();
    if let Err(e) = r {
        if e.kind() != std::io::ErrorKind::UnexpectedEof {
            log!("[process] {site}: {e}");
        }
    }
    // The browser is gone: end now, without waiting for page threads.
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_headers_round_trip() {
        let mut m = Vec::new();
        put_list(&mut m, &[b"http", b"", "é".as_bytes()]);
        assert_eq!(get_list(&mut m.as_slice()).unwrap(), vec![b"http".to_vec(), Vec::new(), "é".as_bytes().to_vec()]);
        let h = vec![("set-cookie".to_string(), "a=1; Path=/".to_string()), ("x".to_string(), String::new())];
        assert_eq!(headers_from(&headers_bytes(&h)), h);
    }

    #[test]
    fn a_site_process_speaks_only_for_its_site() {
        let ex = ["example.com".to_string()];
        assert!(mine(&ex, "https://www.example.com/a"));
        assert!(!mine(&ex, "https://evil.org/"));
        assert!(!mine(&ex, "file:///etc/passwd"));
        assert!(key_ok(&ex, "https://example.com"));
        assert!(key_ok(&ex, "https://cdn.other.net in example.com"));
        assert!(!key_ok(&ex, "https://bank.com"));
        assert!(!key_ok(&ex, "https://bank.com in bank.com"));
        // A shared process holds both its sites, and still no others.
        let two = ["example.com".to_string(), "news.org".to_string()];
        assert!(mine(&two, "https://news.org/") && !mine(&two, "https://bank.com/"));
        // Refused outright: no answer parts.
        assert!(serve_request(&ex, &[b"cookie_get".to_vec(), b"https://bank.com/".to_vec(), b"https://bank.com/".to_vec()]).is_empty());
        assert!(serve_request(&ex, &[b"store_load".to_vec(), b"storage".to_vec(), b"https://bank.com".to_vec()]).is_empty());
        assert!(serve_request(&ex, &[b"store_load".to_vec(), b"../../x".to_vec(), b"https://example.com".to_vec()]).is_empty());
        assert_eq!(serve_request(&ex, &[b"http".to_vec(), b"GET".to_vec(), b"file:///etc/passwd".to_vec()])[0], b"err".to_vec());
    }

    #[test]
    fn pool_size_follows_memory() {
        assert_eq!(pool_size(2048), 2);
        assert_eq!(pool_size(4096), 4);
        assert_eq!(pool_size(8192), 12);
        assert_eq!(pool_size(64 * 1024), 20);
    }

    #[test]
    fn pool_reuses_processes_when_full() {
        let spawn = |site: &str| {
            let child = std::process::Command::new("sleep").arg("5").stdin(std::process::Stdio::piped()).spawn().unwrap();
            let mut child = child;
            let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
            Proc { child, stdin, sites: vec![site.to_string()] }
        };
        let mut procs = HashMap::new();
        procs.insert(1, spawn("a.com"));
        procs.insert(2, spawn("b.com"));
        let pages = HashMap::from([(1, 3), (2, 1)]);
        // A site already running goes back to its process.
        assert_eq!(choose("a.com", &procs, &pages, 2, false), Some(1));
        // Room left: a new process.
        assert_eq!(choose("c.com", &procs, &pages, 3, false), None);
        // Full: the least busy process.
        assert_eq!(choose("c.com", &procs, &pages, 2, false), Some(2));
        // Room left but memory short: share.
        assert_eq!(choose("c.com", &procs, &pages, 3, true), Some(2));
        for p in procs.values_mut() {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }

    #[test]
    fn web_pages_cant_make_shells() {
        let doc = crate::html::parse("<iframe data-ironswift-site=\"guess\" src=\"https://bank.com/\"></iframe>");
        let id = doc.find_all("iframe")[0];
        assert!(!is_shell_frame(&doc, id, "https://bank.com/"));
        assert!(shell_for("https://example.com/", None).is_none(), "off unless enabled");
    }
}
