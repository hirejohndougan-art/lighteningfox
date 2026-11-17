//! `chrome.webRequest.onBeforeRequest`: every request the browser makes (http.rs `open`,
//! which site processes also go through) is shown to the extensions listening for its URL;
//! `["blocking"]` listeners (with the "webRequestBlocking" permission) can cancel it or
//! redirect it. A blocking answer that takes over two seconds lets the request go.
//!
//! The resource `type` is worked out from the request (a page navigation asks for HTML) and
//! the file extension, as the network layer doesn't carry it.

use super::Extension;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

struct Listener {
    ext: String,
    index: u64,
    urls: Vec<String>,
    types: Option<Vec<String>>,
    blocking: bool,
}

static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());
static ANY: AtomicBool = AtomicBool::new(false);
static WAITING: LazyLock<Mutex<HashMap<u64, Sender<Value>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// What to do with a request.
#[derive(Debug, PartialEq)]
pub enum Decision {
    Proceed,
    Cancel,
    Redirect(String),
}

/// Add a listener; returns its index (for removeListener).
pub fn listen(e: &Extension, filter: &Value, blocking: bool) -> Result<u64, String> {
    if !e.has("webRequest") {
        return Err("the \"webRequest\" permission is needed".into());
    }
    if blocking && !e.has("webRequestBlocking") {
        return Err("blocking listeners need the \"webRequestBlocking\" permission".into());
    }
    let urls: Vec<String> = filter.get("urls").and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
    let types = filter.get("types").and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect());
    let index = super::background::next_id();
    if let Ok(mut l) = LISTENERS.lock() {
        l.push(Listener { ext: e.id.clone(), index, urls, types, blocking });
        ANY.store(true, Ordering::SeqCst);
    }
    Ok(index)
}

pub fn unlisten(ext: &str, index: u64) {
    if let Ok(mut l) = LISTENERS.lock() {
        l.retain(|x| !(x.ext == ext && x.index == index));
        ANY.store(!l.is_empty(), Ordering::SeqCst);
    }
}

/// A blocking listener's answer: `{cancel, redirectUrl}`.
pub fn reply(id: u64, response: Value) {
    if let Some(tx) = WAITING.lock().ok().and_then(|mut w| w.remove(&id)) {
        let _ = tx.send(response);
    }
}

/// The webRequest resource type of a request.
pub fn resource_type(url: &str, headers: &[(String, String)]) -> &'static str {
    let accept = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("accept")).map(|(_, v)| v.as_str()).unwrap_or("");
    if accept.starts_with("text/html") {
        return "main_frame";
    }
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    let ext = path.rsplit('/').next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e).unwrap_or("");
    match ext {
        "js" | "mjs" => "script",
        "css" => "stylesheet",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg" | "ico" | "bmp" => "image",
        "woff" | "woff2" | "ttf" | "otf" | "eot" => "font",
        "mp4" | "webm" | "ogg" | "ogv" | "mp3" | "wav" | "m4a" | "opus" | "flac" => "media",
        "json" | "xml" => "xmlhttprequest",
        _ => "other",
    }
}

/// Show a request to the listening extensions (browser process, before it's sent).
pub fn check(method: &str, url: &str, headers: &[(String, String)]) -> Decision {
    if !ANY.load(Ordering::Relaxed) || super::background::on_background_thread() {
        return Decision::Proceed;
    }
    let ty = resource_type(url, headers);
    // Which listeners of which extensions want it.
    let mut wanted: Vec<(String, Vec<u64>, bool)> = Vec::new();
    if let Ok(l) = LISTENERS.lock() {
        for x in l.iter() {
            let Some(e) = super::by_id(&x.ext) else { continue };
            if !e.may_see(url) || !x.urls.iter().any(|p| super::pattern::matches(p, url)) || x.types.as_ref().is_some_and(|t| !t.iter().any(|t| t == ty)) {
                continue;
            }
            match wanted.iter_mut().find(|w| w.0 == x.ext) {
                Some(w) => {
                    w.1.push(x.index);
                    w.2 |= x.blocking;
                }
                None => wanted.push((x.ext.clone(), vec![x.index], x.blocking)),
            }
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    for (ext, listeners, blocking) in wanted {
        let id = super::background::next_id();
        let details = json!({
            "requestId": id.to_string(), "url": url, "method": method.to_ascii_uppercase(), "type": ty,
            "tabId": -1, "frameId": if ty == "main_frame" { 0 } else { -1 }, "parentFrameId": -1, "timeStamp": now,
        });
        let event = json!({ "type": "webRequest", "id": id, "listeners": listeners, "details": details });
        if !blocking {
            super::background::deliver_event(&ext, &event);
            continue;
        }
        let (tx, rx) = channel();
        if let Ok(mut w) = WAITING.lock() {
            w.insert(id, tx);
        }
        let answer = if super::background::deliver_event(&ext, &event) { rx.recv_timeout(Duration::from_secs(2)).ok() } else { None };
        if let Ok(mut w) = WAITING.lock() {
            w.remove(&id);
        }
        let Some(a) = answer else {
            log!("[extensions] {ext} didn't answer for {url} in time; letting it through");
            continue;
        };
        if a.get("cancel").and_then(Value::as_bool) == Some(true) {
            log!("[extensions] {ext} blocked {url}");
            return Decision::Cancel;
        }
        if let Some(to) = a.get("redirectUrl").and_then(Value::as_str) {
            if to != url && !to.starts_with("javascript:") {
                log!("[extensions] {ext} redirected {url} to {to}");
                return Decision::Redirect(to.to_string());
            }
        }
    }
    Decision::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types() {
        let h = |a: &str| vec![("Accept".to_string(), a.to_string())];
        assert_eq!(resource_type("https://a/x", &h("text/html,application/xhtml+xml,*/*;q=0.8")), "main_frame");
        assert_eq!(resource_type("https://a/ads.js?v=1", &h("*/*")), "script");
        assert_eq!(resource_type("https://a/b/c.PNG", &h("*/*")), "image");
        assert_eq!(resource_type("https://a/style.css#x", &[]), "stylesheet");
        assert_eq!(resource_type("https://a/api", &[]), "other");
    }
}
