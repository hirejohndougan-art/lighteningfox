//! `chrome.tabs`: the window publishes its tabs here (`publish`), and takes the commands
//! extensions give (`next_command`). Tab ids are positions + 1, as the window has no other ids
//! for its tabs; a tab's id changes when a tab before it closes.
//!
//! Without a window (snapshots, tests) the pages that ran content scripts stand in for tabs.

use super::Extension;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub url: String,
    pub title: String,
}

/// What the window does for an extension.
#[derive(Debug, PartialEq)]
pub enum Command {
    /// Open a URL in a new tab (in front if true).
    Open(String, bool),
    /// Load a URL in the tab at this position.
    Navigate(usize, String),
    Reload(usize),
    Switch(usize),
    Close(usize),
}

/// (tabs, the active one) as the window last showed them; None without a window.
static SNAPSHOT: Mutex<Option<(Vec<Tab>, usize)>> = Mutex::new(None);
static COMMANDS: Mutex<VecDeque<Command>> = Mutex::new(VecDeque::new());

fn snapshot() -> (Vec<Tab>, usize) {
    if let Some(s) = SNAPSHOT.lock().ok().and_then(|s| s.clone()) {
        return s;
    }
    let pages = super::content::known_pages();
    let n = pages.len();
    (pages.into_iter().map(|url| Tab { title: String::new(), url }).collect(), n.saturating_sub(1))
}

/// The window's tabs (call every frame or so; cheap when nothing changed). Tells the
/// backgrounds what changed (onCreated, onUpdated, onRemoved, onActivated).
pub fn publish(tabs: Vec<Tab>, active: usize) {
    let old = {
        let Ok(mut s) = SNAPSHOT.lock() else { return };
        if s.as_ref().is_some_and(|(t, a)| *t == tabs && *a == active) {
            return;
        }
        s.replace((tabs.clone(), active))
    };
    let bgs = super::background::running();
    if bgs.is_empty() {
        return;
    }
    let (old_tabs, old_active) = old.unwrap_or_default();
    let mut events: Vec<(String, Value, bool)> = Vec::new(); // (event, args, has url/title)
    for (i, t) in tabs.iter().enumerate() {
        match old_tabs.get(i) {
            None => events.push(("onCreated".into(), json!([tab_json(t, i, i == active)]), true)),
            Some(o) if o != t => {
                let mut change = json!({ "status": "complete" });
                if o.url != t.url {
                    change["url"] = json!(t.url);
                }
                if o.title != t.title {
                    change["title"] = json!(t.title);
                }
                events.push(("onUpdated".into(), json!([i + 1, change, tab_json(t, i, i == active)]), true));
            }
            _ => {}
        }
    }
    for i in tabs.len()..old_tabs.len() {
        events.push(("onRemoved".into(), json!([i + 1, { "windowId": 1, "isWindowClosing": false }]), false));
    }
    if active != old_active || old_tabs.is_empty() {
        events.push(("onActivated".into(), json!([{ "tabId": active + 1, "windowId": 1 }]), false));
    }
    for id in bgs {
        let Some(e) = super::by_id(&id) else { continue };
        for (name, args, private) in &events {
            let mut args = args.clone();
            if *private {
                if let Some(arr) = args.as_array_mut() {
                    for a in arr.iter_mut() {
                        hide(e, a);
                    }
                }
            }
            super::background::deliver_event(&id, &json!({ "type": "tabs", "name": name, "args": args }));
        }
    }
}

/// Take the next thing extensions asked the window to do.
pub fn next_command() -> Option<Command> {
    COMMANDS.lock().ok()?.pop_front()
}

/// Put a command back at the front (the window does one thing a frame).
pub fn push_front(c: Command) {
    if let Ok(mut q) = COMMANDS.lock() {
        q.push_front(c);
    }
}

fn push(c: Command) {
    if let Ok(mut q) = COMMANDS.lock() {
        q.push_back(c);
    }
}

fn tab_json(t: &Tab, i: usize, active: bool) -> Value {
    json!({
        "id": i + 1, "index": i, "windowId": 1, "active": active, "highlighted": active, "selected": active,
        "pinned": false, "incognito": false, "discarded": false, "status": "complete",
        "url": t.url, "title": t.title,
    })
}

/// Without the "tabs" permission or access to the site, a tab's URL and title are hidden.
fn hide(e: &Extension, tab: &mut Value) {
    let url = tab.get("url").and_then(Value::as_str).unwrap_or("").to_string();
    if url.is_empty() || e.has("tabs") || e.may_see(&url) {
        return;
    }
    if let Some(o) = tab.as_object_mut() {
        o.remove("url");
        o.remove("title");
    }
}

/// `tabs.query(queryInfo)`.
pub fn query(e: &Extension, q: &Value) -> Value {
    let (tabs, active) = snapshot();
    let patterns: Vec<String> = match q.get("url") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|s| s.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    };
    let out: Vec<Value> = tabs
        .iter()
        .enumerate()
        .filter(|(i, t)| {
            q.get("active").and_then(Value::as_bool).is_none_or(|a| a == (*i == active))
                && q.get("index").and_then(Value::as_u64).is_none_or(|x| x as usize == *i)
                && (patterns.is_empty() || patterns.iter().any(|p| super::pattern::matches(p, &t.url)))
                && q.get("title").and_then(Value::as_str).is_none_or(|p| super::pattern::glob(p, &t.title))
        })
        .map(|(i, t)| {
            let mut v = tab_json(t, i, i == active);
            hide(e, &mut v);
            v
        })
        .collect();
    Value::Array(out)
}

fn position(tab_id: u64) -> Option<usize> {
    let (tabs, _) = snapshot();
    let i = (tab_id as usize).checked_sub(1)?;
    (i < tabs.len()).then_some(i)
}

/// The URL of a tab, for tabs.sendMessage.
pub fn url_of(tab_id: u64) -> Option<String> {
    let (tabs, _) = snapshot();
    position(tab_id).map(|i| tabs[i].url.clone())
}

/// The tab showing `url` (for a message's `sender.tab`).
pub fn tab_for_url(url: &str, e: &Extension) -> Option<Value> {
    let (tabs, active) = snapshot();
    // The active tab first: that's most often where a content script just ran.
    let order = std::iter::once(active).chain(0..tabs.len());
    for i in order {
        if tabs.get(i).is_some_and(|t| t.url == url) {
            let mut v = tab_json(&tabs[i], i, i == active);
            hide(e, &mut v);
            return Some(v);
        }
    }
    None
}

pub fn get(e: &Extension, tab_id: u64) -> Result<Value, String> {
    let (tabs, active) = snapshot();
    let i = position(tab_id).ok_or_else(|| format!("No tab with id: {tab_id}."))?;
    let mut v = tab_json(&tabs[i], i, i == active);
    hide(e, &mut v);
    Ok(v)
}

fn check_url(url: &str) -> Result<String, String> {
    let u = url.trim();
    if u.starts_with("javascript:") {
        return Err("javascript: URLs aren't allowed".into());
    }
    Ok(if u.is_empty() { "about:blank".into() } else { u.to_string() })
}

pub fn create(_e: &Extension, props: &Value) -> Result<Value, String> {
    let url = check_url(props.get("url").and_then(Value::as_str).unwrap_or(""))?;
    let front = props.get("active").and_then(Value::as_bool).unwrap_or(true);
    let (tabs, _) = snapshot();
    push(Command::Open(url.clone(), front));
    let t = Tab { url, title: String::new() };
    Ok(tab_json(&t, tabs.len(), front))
}

pub fn update(e: &Extension, tab_id: Option<u64>, props: &Value) -> Result<Value, String> {
    let (tabs, active) = snapshot();
    let i = match tab_id {
        Some(id) => position(id).ok_or_else(|| format!("No tab with id: {id}."))?,
        None => active,
    };
    if let Some(url) = props.get("url").and_then(Value::as_str) {
        push(Command::Navigate(i, check_url(url)?));
    }
    if props.get("active").and_then(Value::as_bool) == Some(true) && i != active {
        push(Command::Switch(i));
    }
    let mut v = tabs.get(i).map(|t| tab_json(t, i, i == active)).unwrap_or(Value::Null);
    hide(e, &mut v);
    Ok(v)
}

pub fn remove(ids: Vec<u64>) -> Result<Value, String> {
    let mut pos: Vec<usize> = ids.iter().map(|&id| position(id).ok_or_else(|| format!("No tab with id: {id}."))).collect::<Result<_, _>>()?;
    // Close from the end so earlier positions stay right.
    pos.sort_unstable_by(|a, b| b.cmp(a));
    pos.dedup();
    for p in pos {
        push(Command::Close(p));
    }
    Ok(Value::Null)
}

pub fn reload(tab_id: Option<u64>) -> Result<Value, String> {
    let (_, active) = snapshot();
    let i = match tab_id {
        Some(id) => position(id).ok_or_else(|| format!("No tab with id: {id}."))?,
        None => active,
    };
    push(Command::Reload(i));
    Ok(Value::Null)
}

