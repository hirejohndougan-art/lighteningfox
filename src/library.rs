//! Browsing history and bookmarks, kept in the profile folder (history.tsv, bookmarks.tsv),
//! plus the suggestions the address bar shows while you type.

use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// Unix seconds of the latest visit.
    pub last: i64,
    pub count: u32,
}

#[derive(Clone, Debug)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    /// Folder name; empty for bookmarks directly on the bar.
    pub folder: String,
}

#[derive(Default)]
struct Library {
    history: Vec<Visit>,
    bookmarks: Vec<Bookmark>,
    loaded: bool,
}

const MAX_HISTORY: usize = 5000;

fn lib() -> &'static Mutex<Library> {
    static L: OnceLock<Mutex<Library>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Library::default()))
}

fn file(name: &str) -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join(name))
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// Write the whole file privately and atomically.
fn save(name: &str, body: String) {
    let Some(path) = file(name) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, body).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::fs::rename(&tmp, &path);
    }
}

impl Library {
    fn ensure(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let read = |n: &str| file(n).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        for line in read("history.tsv").lines() {
            let f: Vec<&str> = line.splitn(4, '\t').collect();
            if let [url, last, count, title] = f[..] {
                if let (Ok(last), Ok(count)) = (last.parse(), count.parse()) {
                    self.history.push(Visit { url: url.into(), title: title.into(), last, count });
                }
            }
        }
        for line in read("bookmarks.tsv").lines() {
            let f: Vec<&str> = line.splitn(3, '\t').collect();
            if f.len() >= 2 {
                self.bookmarks.push(Bookmark { url: f[0].into(), title: f[1].into(), folder: f.get(2).unwrap_or(&"").trim().into() });
            }
        }
    }

    fn save_history(&self) {
        let body: String = self.history.iter().map(|v| format!("{}\t{}\t{}\t{}\n", clean(&v.url), v.last, v.count, clean(&v.title))).collect();
        save("history.tsv", body);
    }

    fn save_bookmarks(&self) {
        let body: String = self.bookmarks.iter().map(|b| format!("{}\t{}\t{}\n", clean(&b.url), clean(&b.title), clean(&b.folder))).collect();
        save("bookmarks.tsv", body);
    }
}

/// Pages worth remembering: real web pages and local files, not about: pages.
fn recordable(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")
}

/// Record a visit (or update the title of the latest one).
pub fn visit(url: &str, title: &str) {
    if !recordable(url) || std::env::var("LF_PRIVATE").is_ok_and(|v| v == "1") {
        return;
    }
    let Ok(mut l) = lib().lock() else { return };
    l.ensure();
    let now = crate::cookies::now();
    match l.history.iter().position(|v| v.url == url) {
        Some(i) => {
            let mut v = l.history.remove(i);
            // The same page again within a few seconds (a reload, a title update) isn't a new visit.
            if now - v.last > 5 {
                v.count += 1;
            }
            v.last = now;
            if !title.is_empty() {
                v.title = title.to_string();
            }
            l.history.push(v);
        }
        None => l.history.push(Visit { url: url.into(), title: title.into(), last: now, count: 1 }),
    }
    if l.history.len() > MAX_HISTORY {
        let extra = l.history.len() - MAX_HISTORY;
        l.history.drain(..extra);
    }
    l.save_history();
}

pub fn history() -> Vec<Visit> {
    lib().lock().map(|mut l| {
        l.ensure();
        l.history.iter().rev().cloned().collect()
    }).unwrap_or_default()
}

pub fn clear_history() {
    if let Ok(mut l) = lib().lock() {
        l.ensure();
        l.history.clear();
        l.save_history();
    }
}

pub fn bookmarks() -> Vec<Bookmark> {
    lib().lock().map(|mut l| {
        l.ensure();
        l.bookmarks.clone()
    }).unwrap_or_default()
}

pub fn is_bookmarked(url: &str) -> bool {
    lib().lock().map(|mut l| {
        l.ensure();
        l.bookmarks.iter().any(|b| b.url == url)
    }).unwrap_or(false)
}

/// Bookmark the page, or remove the bookmark if it has one. Returns whether it's bookmarked now.
pub fn toggle_bookmark(url: &str, title: &str) -> bool {
    let Ok(mut l) = lib().lock() else { return false };
    l.ensure();
    let now = match l.bookmarks.iter().position(|b| b.url == url) {
        Some(i) => {
            l.bookmarks.remove(i);
            false
        }
        None => {
            l.bookmarks.push(Bookmark { url: url.into(), title: if title.is_empty() { url.into() } else { title.into() }, folder: String::new() });
            true
        }
    };
    l.save_bookmarks();
    now
}

pub fn remove_bookmark(url: &str) {
    if let Ok(mut l) = lib().lock() {
        l.ensure();
        l.bookmarks.retain(|b| b.url != url);
        l.save_bookmarks();
    }
}

/// Put a bookmark in a folder (an empty name puts it back on the bar).
pub fn move_bookmark(url: &str, folder: &str) {
    if let Ok(mut l) = lib().lock() {
        l.ensure();
        let folder = folder.trim().replace(['\t', '\n', '\r'], " ");
        if let Some(b) = l.bookmarks.iter_mut().find(|b| b.url == url) {
            b.folder = folder;
        }
        l.save_bookmarks();
    }
}

/// Folder names in the order they first appear.
pub fn folders(list: &[Bookmark]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in list {
        if !b.folder.is_empty() && !out.contains(&b.folder) {
            out.push(b.folder.clone());
        }
    }
    out
}

/// A suggestion under the address bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub url: String,
    pub title: String,
    pub bookmarked: bool,
}

/// Suggestions for what's typed: bookmarks first, then history, best matches first.
/// Every typed word must appear in the URL or title.
pub fn suggest(typed: &str, max: usize) -> Vec<Suggestion> {
    let words: Vec<String> = typed.to_lowercase().split_whitespace().map(String::from).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let Ok(mut l) = lib().lock() else { return Vec::new() };
    l.ensure();
    let bare = |u: &str| u.split("://").nth(1).unwrap_or(u).trim_start_matches("www.").to_lowercase();
    let matches = |url: &str, title: &str| {
        let hay = format!("{} {}", bare(url), title.to_lowercase());
        words.iter().all(|w| hay.contains(w.as_str()))
    };
    let now = crate::cookies::now();
    // Typed text at the start of the address is the strongest signal.
    let starts = |url: &str| bare(url).starts_with(words[0].as_str());
    let mut out: Vec<(f64, Suggestion)> = Vec::new();
    for b in &l.bookmarks {
        if matches(&b.url, &b.title) {
            let score = 1000.0 + if starts(&b.url) { 500.0 } else { 0.0 };
            out.push((score, Suggestion { url: b.url.clone(), title: b.title.clone(), bookmarked: true }));
        }
    }
    for v in &l.history {
        if out.iter().any(|(_, s)| s.url == v.url) || !matches(&v.url, &v.title) {
            continue;
        }
        let days = ((now - v.last).max(0) as f64) / 86400.0;
        let score = (v.count as f64).ln_1p() * 40.0 / (1.0 + days / 7.0) + if starts(&v.url) { 300.0 } else { 0.0 };
        out.push((score, Suggestion { url: v.url.clone(), title: v.title.clone(), bookmarked: false }));
    }
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter().take(max).map(|(_, s)| s).collect()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const PAGE_STYLE: &str = "body{font-family:sans-serif;margin:32px auto;max-width:860px;padding:0 20px;color:#1f2937}h1{margin-bottom:4px}.sub{color:#64748b;margin-bottom:20px}.row{display:flex;gap:12px;align-items:baseline;padding:8px 10px;border-bottom:1px solid #e5e7eb}.row:hover{background:#f8fafc}.t{flex:1;overflow:hidden}.t a{color:#1d4ed8;text-decoration:none;font-weight:600}.u{color:#64748b;font-size:13px}.when{color:#94a3b8;font-size:12px;white-space:nowrap}.empty{color:#64748b;padding:24px 0}a.btn{display:inline-block;background:#fee2e2;color:#991b1b;padding:6px 12px;border-radius:6px;text-decoration:none;font-size:14px}";

fn ago(t: i64) -> String {
    let d = (crate::cookies::now() - t).max(0);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}

/// about:history — newest first. `?clear` wipes it.
pub fn history_page(query: &str) -> String {
    if query.contains("clear") {
        clear_history();
    }
    let h = history();
    let mut rows = String::new();
    for v in h.iter().take(500) {
        let title = if v.title.is_empty() { &v.url } else { &v.title };
        rows.push_str(&format!(
            "<div class=\"row\"><div class=\"t\"><a href=\"{u}\">{t}</a><div class=\"u\">{ue}</div></div><div class=\"when\">{when} · {c} visit{s}</div></div>",
            u = esc(&v.url),
            t = esc(title),
            ue = esc(&v.url),
            when = ago(v.last),
            c = v.count,
            s = if v.count == 1 { "" } else { "s" }
        ));
    }
    if rows.is_empty() {
        rows = "<p class=\"empty\">No history yet. Pages you visit show up here.</p>".into();
    }
    format!(
        "<!DOCTYPE html><html><head><title>History</title><style>{PAGE_STYLE}</style></head><body><h1>History</h1><p class=\"sub\">{n} page{s}, kept only on this computer. <a class=\"btn\" href=\"about:history?clear\">Clear history</a></p>{rows}</body></html>",
        n = h.len(),
        s = if h.len() == 1 { "" } else { "s" }
    )
}

/// about:bookmarks: grouped by folder. Each row can be removed or moved to a folder.
pub fn bookmarks_page(query: &str) -> String {
    let q = query.trim_start_matches('?');
    let param = |name: &str| {
        q.split('&').find_map(|p| p.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| crate::net::percent_decode_str(&v.replace('+', " "))))
    };
    if let Some(u) = param("remove") {
        remove_bookmark(&u);
    } else if let Some(u) = param("move") {
        move_bookmark(&u, &param("folder").unwrap_or_default());
    }
    let b = bookmarks();
    let folders = folders(&b);
    let options: String = folders.iter().map(|f| format!("<option value=\"{0}\"></option>", esc(f))).collect();
    let row = |bm: &Bookmark| {
        let enc = crate::forms::urlencode(&[(String::new(), bm.url.clone())]);
        format!(
            "<div class=\"row\"><div class=\"t\"><a href=\"{u}\">★ {t}</a><div class=\"u\">{u}</div></div>\
             <form class=\"mv\" action=\"about:bookmarks\" method=\"get\"><input type=\"hidden\" name=\"move\" value=\"{u}\">\
             <input type=\"text\" name=\"folder\" value=\"{f}\" placeholder=\"Folder\" list=\"folders\"><button type=\"submit\">Move</button></form>\
             <a class=\"when\" href=\"about:bookmarks?remove{enc}\">remove</a></div>",
            u = esc(&bm.url),
            t = esc(&bm.title),
            f = esc(&bm.folder),
            enc = enc
        )
    };
    let mut rows: String = b.iter().filter(|x| x.folder.is_empty()).map(row).collect();
    for f in &folders {
        rows.push_str(&format!("<h2>{}</h2>", esc(f)));
        rows.extend(b.iter().filter(|x| &x.folder == f).map(row));
    }
    if b.is_empty() {
        rows = "<p class=\"empty\">No bookmarks yet. Press Ctrl+D or click the ☆ in the address bar to bookmark a page.</p>".into();
    }
    format!(
        "<!DOCTYPE html><html><head><title>Bookmarks</title><style>{PAGE_STYLE}h2{{font-size:17px;margin:22px 0 4px;color:#334155}}.mv input{{width:120px;font-size:13px;padding:3px 6px}}.mv button{{font-size:13px;margin-left:4px}}</style></head><body><h1>Bookmarks</h1><p class=\"sub\">{n} bookmark{s}{fs}. Type a folder name and press Move to file a bookmark; clear it to put the bookmark back on the bar.</p><datalist id=\"folders\">{options}</datalist>{rows}</body></html>",
        n = b.len(),
        s = if b.len() == 1 { "" } else { "s" },
        fs = match folders.len() { 0 => String::new(), 1 => " in 1 folder and the bar".into(), k => format!(" in {k} folders and the bar") },
    )
}
