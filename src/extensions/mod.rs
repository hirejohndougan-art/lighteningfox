//! Browser extensions (the WebExtensions API, as in Chrome and Firefox).
//!
//! An extension is a folder with a `manifest.json` (Manifest V2 or V3), installed unpacked in
//! the profile's `extensions/` folder or named in `IRONSWIFT_EXTENSIONS` (folders separated by
//! `:`). What works:
//!
//! - **Content scripts** (`content_scripts`): JavaScript run in matching pages at
//!   `document_start`, `document_end` or `document_idle`, and CSS added to them (content.rs).
//! - **Background scripts** (MV2 `background.scripts`, MV3 `background.service_worker`): run on
//!   a thread of their own in the browser process (background.rs).
//! - **`chrome.storage`** (`local`, `sync`, `session`) with `onChanged`, kept in the profile.
//! - **`chrome.runtime`** messaging between content scripts and the background
//!   (`sendMessage`/`onMessage`), `getURL`, `getManifest`, `id`.
//! - **`chrome.tabs`**: `query`, `get`, `create`, `update`, `remove`, `sendMessage`, and the
//!   `onCreated`/`onUpdated`/`onRemoved`/`onActivated` events (tabs.rs).
//! - **`chrome.webRequest.onBeforeRequest`**, including `["blocking"]` listeners that cancel
//!   or redirect requests (webrequest.rs).
//!
//! The same API objects (api.js) are given to content scripts and the background; the
//! `browser.*` namespace is the same object, and every method both takes a callback and
//! returns a promise.
//!
//! Everything an extension keeps — its files, its storage, its background scripts — lives in
//! the browser process. A content script in a sandboxed site process (process.rs) asks for its
//! code and its storage with one kind of request, `ext <json>`, answered by `serve` here.

pub mod background;
pub mod content;
pub mod pattern;
pub mod tabs;
pub mod webrequest;

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// How long the browser waits for an extension's background script (`secs`), or 30 seconds in
/// tests: a test machine running every test at once, with every function compiled first
/// (`BOA_JIT=always` in CI), can take longer than a user's browser ever should, and the tests
/// check answers, not the time limit.
pub(crate) const fn wait_limit(secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(if cfg!(test) { 30 } else { secs })
}

/// The JavaScript side of the API, shared by content scripts and backgrounds.
pub const API_JS: &str = include_str!("api.js");

/// One content-script entry of a manifest.
#[derive(Debug, Clone, Default)]
pub struct ContentScript {
    pub matches: Vec<String>,
    pub exclude_matches: Vec<String>,
    pub js: Vec<String>,
    pub css: Vec<String>,
    /// "document_start", "document_end" or "document_idle" (the default).
    pub run_at: String,
    pub all_frames: bool,
}

/// An installed extension.
#[derive(Debug, Clone)]
pub struct Extension {
    pub id: String,
    pub dir: PathBuf,
    pub name: String,
    pub version: String,
    pub manifest: Value,
    pub content_scripts: Vec<ContentScript>,
    /// Background scripts, in order (one for an MV3 service worker).
    pub background: Vec<String>,
    /// API permissions ("storage", "tabs", "webRequest", "webRequestBlocking"…).
    pub permissions: Vec<String>,
    /// Host permissions: match patterns for the sites it may see.
    pub hosts: Vec<String>,
}

impl Extension {
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }

    /// May the extension see requests to / pages at `url`?
    pub fn may_see(&self, url: &str) -> bool {
        self.hosts.iter().any(|p| pattern::matches(p, url))
    }

    /// The URL of one of its files (`chrome.runtime.getURL`).
    pub fn url(&self, path: &str) -> String {
        format!("ironswift-extension://{}/{}", self.id, path.trim_start_matches('/'))
    }

    /// One of its files, as text. Paths can't leave the extension's folder.
    pub fn read(&self, path: &str) -> Option<String> {
        let rel = Path::new(path.trim_start_matches('/'));
        if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return None;
        }
        std::fs::read_to_string(self.dir.join(rel)).ok()
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|s| s.as_str().map(String::from)).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// A short, stable id for the extension in `dir`: Firefox's `browser_specific_settings` id if
/// it has one, else the folder name (lower case letters, digits, `-` and `_`).
fn id_for(dir: &Path, manifest: &Value) -> String {
    let gecko = manifest.pointer("/browser_specific_settings/gecko/id").or_else(|| manifest.pointer("/applications/gecko/id"));
    let raw = gecko.and_then(Value::as_str).map(String::from).unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let id: String = raw.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    if id.is_empty() { "extension".into() } else { id }
}

/// Read a manifest (comments allowed, as Chrome allows them).
pub fn parse_manifest(dir: &Path, text: &str) -> Result<Extension, String> {
    let manifest: Value = serde_json::from_str(&strip_comments(text)).map_err(|e| format!("manifest.json: {e}"))?;
    if !manifest.is_object() {
        return Err("manifest.json: not a JSON object".into());
    }
    let version = manifest.get("manifest_version").and_then(Value::as_u64).unwrap_or(2);
    let mut content_scripts = Vec::new();
    if let Some(Value::Array(list)) = manifest.get("content_scripts") {
        for c in list {
            content_scripts.push(ContentScript {
                matches: strings(c.get("matches")),
                exclude_matches: strings(c.get("exclude_matches")),
                js: strings(c.get("js")),
                css: strings(c.get("css")),
                run_at: c.get("run_at").and_then(Value::as_str).unwrap_or("document_idle").to_string(),
                all_frames: c.get("all_frames").and_then(Value::as_bool).unwrap_or(false),
            });
        }
    }
    let background = match manifest.get("background") {
        Some(b) => {
            let mut v = strings(b.get("scripts"));
            v.extend(strings(b.get("service_worker")));
            v
        }
        None => Vec::new(),
    };
    let mut permissions = Vec::new();
    let mut hosts = strings(manifest.get("host_permissions"));
    for p in strings(manifest.get("permissions")) {
        // MV2 puts host patterns among the permissions.
        if p == "<all_urls>" || p.contains("://") {
            hosts.push(p);
        } else {
            permissions.push(p);
        }
    }
    let _ = version;
    let id = id_for(dir, &manifest);
    Ok(Extension {
        name: manifest.get("name").and_then(Value::as_str).unwrap_or(&id).to_string(),
        version: manifest.get("version").and_then(Value::as_str).unwrap_or("0").to_string(),
        id,
        dir: dir.to_path_buf(),
        manifest,
        content_scripts,
        background,
        permissions,
        hosts,
    })
}

/// Remove `//` and `/* */` comments outside strings.
fn strip_comments(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut in_str) = (0, false);
    while i < c.len() {
        if in_str {
            out.push(c[i]);
            if c[i] == '\\' && i + 1 < c.len() {
                out.push(c[i + 1]);
                i += 1;
            } else if c[i] == '"' {
                in_str = false;
            }
        } else if c[i] == '"' {
            in_str = true;
            out.push('"');
        } else if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        } else {
            out.push(c[i]);
        }
        i += 1;
    }
    out
}

/// The folders extensions are installed in.
fn extension_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(list) = std::env::var("IRONSWIFT_EXTENSIONS") {
        dirs.extend(list.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    if let Some(p) = crate::cookies::profile_dir() {
        if let Ok(rd) = std::fs::read_dir(p.join("extensions")) {
            let mut found: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("manifest.json").is_file()).collect();
            found.sort();
            dirs.extend(found);
        }
    }
    dirs
}

static ALL: std::sync::RwLock<Option<&'static [Extension]>> = std::sync::RwLock::new(None);

/// The installed extensions (read once). Empty in a site process: there the browser answers.
pub fn all() -> &'static [Extension] {
    if let Some(a) = *ALL.read().unwrap_or_else(|e| e.into_inner()) {
        return a;
    }
    let mut slot = ALL.write().unwrap_or_else(|e| e.into_inner());
    if let Some(a) = *slot {
        return a;
    }
    let list: &'static [Extension] = Box::leak(load().into_boxed_slice());
    *slot = Some(list);
    list
}

fn load() -> Vec<Extension> {
    if crate::process::in_renderer() {
        return Vec::new();
    }
    let mut out: Vec<Extension> = Vec::new();
    for dir in extension_dirs() {
        let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else {
            log!("[extensions] {}: no manifest.json", dir.display());
            continue;
        };
        match parse_manifest(&dir, &text) {
            Ok(e) if out.iter().any(|o| o.id == e.id) => log!("[extensions] {}: an extension with id {} is already loaded", dir.display(), e.id),
            Ok(e) => {
                log!("[extensions] loaded {} {} ({})", e.name, e.version, e.id);
                out.push(e);
            }
            Err(err) => log!("[extensions] {}: {err}", dir.display()),
        }
    }
    out
}

/// Tests: add an extension from a folder and start its background. Returns it.
#[cfg(test)]
pub fn install_for_tests(dir: &Path) -> &'static Extension {
    let e = parse_manifest(dir, &std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let current = all();
    if let Some(found) = current.iter().find(|x| x.id == e.id) {
        return found;
    }
    let mut v = current.to_vec();
    v.push(e.clone());
    let list: &'static [Extension] = Box::leak(v.into_boxed_slice());
    *ALL.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
    let e = list.iter().find(|x| x.id == e.id).unwrap();
    background::start(e);
    e
}

pub fn by_id(id: &str) -> Option<&'static Extension> {
    all().iter().find(|e| e.id == id)
}

/// Start the extensions' background scripts (browser process; once).
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        if crate::process::in_renderer() {
            return;
        }
        for e in all() {
            if !e.background.is_empty() {
                background::start(e);
            }
        }
    });
}

// ---------------------------------------------------------------- storage

/// Storage areas, loaded on first use: (extension id, area) -> items.
static STORAGE: Mutex<Option<HashMap<(String, String), Map<String, Value>>>> = Mutex::new(None);

fn storage_file(id: &str, area: &str) -> Option<PathBuf> {
    if area == "session" {
        return None; // in memory only
    }
    crate::cookies::profile_dir().map(|p| p.join("extension-data").join(id).join(format!("storage-{area}.json")))
}

fn with_area<R>(id: &str, area: &str, f: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
    let mut guard = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let all = guard.get_or_insert_with(HashMap::new);
    let items = all.entry((id.to_string(), area.to_string())).or_insert_with(|| {
        storage_file(id, area)
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    });
    f(items)
}

fn save_area(id: &str, area: &str, items: &Map<String, Value>) {
    if std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1") {
        return;
    }
    if let Some(f) = storage_file(id, area) {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, Value::Object(items.clone()).to_string());
    }
}

/// `chrome.storage.<area>.<method>(arg)`. Returns the result and, for changes, tells the
/// background and the extension's content scripts (`storage.onChanged`).
pub fn storage(id: &str, area: &str, method: &str, arg: &Value) -> Result<Value, String> {
    if !matches!(area, "local" | "sync" | "session" | "managed") {
        return Err(format!("unknown storage area {area}"));
    }
    let mut changes = Map::new();
    let result = with_area(id, area, |items| -> Result<Value, String> {
        match method {
            "get" => Ok(match arg {
                Value::Null => Value::Object(items.clone()),
                Value::String(k) => Value::Object(items.get(k).map(|v| Map::from_iter([(k.clone(), v.clone())])).unwrap_or_default()),
                Value::Array(keys) => Value::Object(keys.iter().filter_map(Value::as_str).filter_map(|k| items.get(k).map(|v| (k.to_string(), v.clone()))).collect()),
                // An object: its values are the defaults.
                Value::Object(defaults) => Value::Object(defaults.iter().map(|(k, d)| (k.clone(), items.get(k).cloned().unwrap_or_else(|| d.clone()))).collect()),
                _ => Value::Object(Map::new()),
            }),
            "getBytesInUse" => Ok(json!(Value::Object(items.clone()).to_string().len())),
            "set" => {
                let Value::Object(new) = arg else { return Err("storage.set needs an object".into()) };
                for (k, v) in new {
                    let old = items.insert(k.clone(), v.clone());
                    if old.as_ref() != Some(v) {
                        let mut c = Map::new();
                        if let Some(o) = old {
                            c.insert("oldValue".into(), o);
                        }
                        c.insert("newValue".into(), v.clone());
                        changes.insert(k.clone(), Value::Object(c));
                    }
                }
                Ok(Value::Null)
            }
            "remove" => {
                let keys: Vec<String> = match arg {
                    Value::String(k) => vec![k.clone()],
                    Value::Array(a) => a.iter().filter_map(|s| s.as_str().map(String::from)).collect(),
                    _ => Vec::new(),
                };
                for k in keys {
                    if let Some(o) = items.remove(&k) {
                        changes.insert(k, json!({ "oldValue": o }));
                    }
                }
                Ok(Value::Null)
            }
            "clear" => {
                for (k, o) in std::mem::take(items) {
                    changes.insert(k, json!({ "oldValue": o }));
                }
                Ok(Value::Null)
            }
            _ => Err(format!("storage.{area}.{method} isn't supported")),
        }
    })?;
    if !changes.is_empty() {
        with_area(id, area, |items| save_area(id, area, items));
        let event = json!({ "type": "storage", "area": area, "changes": changes });
        background::deliver_event(id, &event);
        content::broadcast(id, None, event);
    }
    Ok(result)
}

// ---------------------------------------------------------------- requests from pages

/// Answer a content script's request (browser process). `req.op` says what it wants:
///
/// - `content {url, top}`: the scripts and styles for a page, and where its event queue is.
/// - `storage {ext, area, method, arg, url}`
/// - `send {ext, msg, url}`: `runtime.sendMessage` to the background; waits for the answer.
/// - `poll {url, seq}`: events for the page's content scripts (messages, storage changes).
/// - `reply {id, response}`: a content script answering `tabs.sendMessage`.
///
/// `url` is the page asking: an extension's storage and messages are only for pages its
/// content scripts run in.
pub fn serve(req: &Value) -> Value {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let url = s("url");
    let ext = by_id(&s("ext"));
    let allowed = |e: &Extension| content::runs_in(e, &url);
    let r = match s("op").as_str() {
        "content" => Ok(content::for_page(&url, req.get("top").and_then(Value::as_bool).unwrap_or(true))),
        "storage" => match ext {
            Some(e) if allowed(e) && !(e.has("storage") || e.has("unlimitedStorage")) => Err("the \"storage\" permission is needed".into()),
            Some(e) if allowed(e) => storage(&e.id, &s("area"), &s("method"), req.get("arg").unwrap_or(&Value::Null)),
            _ => Err("no such extension here".into()),
        },
        "send" => match ext {
            Some(e) if allowed(e) => Ok(background::send_message(e, req.get("msg").cloned().unwrap_or(Value::Null), &url)),
            _ => Err("no such extension here".into()),
        },
        "poll" => Ok(content::poll(&url, req.get("seq").and_then(Value::as_u64).unwrap_or(0))),
        "reply" => {
            content::reply(req.get("id").and_then(Value::as_u64).unwrap_or(0), req.get("response").cloned());
            Ok(Value::Null)
        }
        other => Err(format!("unknown request {other}")),
    };
    match r {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    }
}

/// Ask the browser (from a site process) or answer here.
pub fn call(req: &Value) -> Value {
    if crate::process::in_renderer() {
        match crate::process::remote_ext(&req.to_string()) {
            Some(text) => serde_json::from_str(&text).unwrap_or_else(|_| json!({ "error": "bad answer" })),
            None => json!({ "error": "the browser didn't answer" }),
        }
    } else {
        start();
        serve(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests() {
        let m = r#"{
            // comments are allowed
            "manifest_version": 3, "name": "Dark", "version": "1.2",
            "permissions": ["storage", "tabs", "https://*.example.com/*"],
            "host_permissions": ["<all_urls>"],
            "background": {"service_worker": "bg.js"},
            "content_scripts": [{"matches": ["*://*/*"], "js": ["a.js", "b.js"], "css": ["dark.css"], "run_at": "document_start"}],
            "browser_specific_settings": {"gecko": {"id": "dark@example.org"}}
        }"#;
        let e = parse_manifest(Path::new("/x/dark"), m).unwrap();
        assert_eq!(e.id, "dark-example-org");
        assert_eq!(e.background, vec!["bg.js"]);
        assert_eq!(e.content_scripts[0].js, vec!["a.js", "b.js"]);
        assert_eq!(e.content_scripts[0].run_at, "document_start");
        assert!(e.has("storage") && !e.has("https://*.example.com/*"));
        assert!(e.may_see("https://a.test/") && e.hosts.len() == 2);
        assert_eq!(e.url("/p/x.png"), "ironswift-extension://dark-example-org/p/x.png");
        assert!(e.read("../../etc/passwd").is_none());
        assert!(parse_manifest(Path::new("/x"), "[1]").is_err());
        assert_eq!(strip_comments(r#"{"a": "//x" /* c */}"#), r#"{"a": "//x" }"#);
    }

    #[test]
    fn storage_areas() {
        let id = format!("test-storage-{}", std::process::id());
        storage(&id, "session", "set", &json!({"a": 1, "b": {"c": true}})).unwrap();
        assert_eq!(storage(&id, "session", "get", &json!("a")).unwrap(), json!({"a": 1}));
        assert_eq!(storage(&id, "session", "get", &json!({"a": 0, "z": 9})).unwrap(), json!({"a": 1, "z": 9}));
        storage(&id, "session", "remove", &json!(["a"])).unwrap();
        assert_eq!(storage(&id, "session", "get", &Value::Null).unwrap(), json!({"b": {"c": true}}));
        storage(&id, "session", "clear", &Value::Null).unwrap();
        assert_eq!(storage(&id, "session", "get", &Value::Null).unwrap(), json!({}));
        assert!(storage(&id, "bogus", "get", &Value::Null).is_err());
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;

    fn test_ext() -> &'static Extension {
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        install_for_tests(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/extensions/test-ext"))
    }

    #[test]
    fn web_request_blocking() {
        let e = test_ext();
        assert_eq!(e.background.len(), 2);
        use webrequest::{check, Decision};
        let h = vec![("Accept".to_string(), "*/*".to_string())];
        assert_eq!(check("GET", "https://blocked.test/ad.js", &h), Decision::Cancel);
        assert_eq!(check("GET", "https://moved.test/lib.js", &h), Decision::Redirect("https://example.org/moved".into()));
        // The type filter: only scripts are moved.
        assert_eq!(check("GET", "https://moved.test/pic.png", &h), Decision::Proceed);
        assert_eq!(check("GET", "https://elsewhere.test/ad.js", &h), Decision::Proceed);
        // Through the network layer: the request never goes out.
        let err = crate::http::open("GET", "https://blocked.test/x", &h, None, std::time::Duration::from_secs(5)).err().unwrap();
        assert!(err.contains("blocked by an extension"), "{err}");
    }

    #[test]
    fn content_script_requests() {
        let e = test_ext();
        // Pages its content scripts don't run in can't use its storage or messages.
        let r = serve(&json!({"op": "storage", "ext": e.id, "area": "local", "method": "get", "arg": null, "url": "https://other.test/"}));
        assert!(r.get("error").is_some());
        let page = format!("file://{}/tests/extension-test.html", env!("CARGO_MANIFEST_DIR"));
        let r = serve(&json!({"op": "content", "url": page, "top": true}));
        let exts = r["ok"]["extensions"].as_array().unwrap();
        let mine = exts.iter().find(|x| x["ext"] == json!(e.id)).unwrap();
        // start.js and end.js; never.js is excluded.
        let runs = mine["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(!runs.iter().any(|r| r[1].as_str().unwrap().contains("never")));
        assert!(mine["css"].as_str().unwrap().contains("#styled"));
        let r = serve(&json!({"op": "send", "ext": e.id, "msg": {"kind": "echo", "text": "yo"}, "url": page}));
        assert_eq!(r["ok"]["response"]["echo"], json!("yo"));
    }
}
