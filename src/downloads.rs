//! Downloads: files the browser saves instead of showing (PDFs, archives, anything with
//! `Content-Disposition: attachment`). Each runs on its own thread, writing to
//! `<name>.part` in the Downloads folder and renaming it when complete. about:downloads lists
//! them with progress.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Active,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone)]
pub struct Download {
    pub id: u32,
    pub url: String,
    pub path: PathBuf,
    pub total: Option<u64>,
    pub received: u64,
    pub state: State,
    pub started: Instant,
    cancel: Arc<AtomicBool>,
}

static LIST: Mutex<Vec<Download>> = Mutex::new(Vec::new());

/// The user's Downloads folder (XDG_DOWNLOAD_DIR from ~/.config/user-dirs.dirs, else ~/Downloads).
pub fn folder() -> PathBuf {
    if let Ok(d) = std::env::var("IRONSWIFT_DOWNLOADS") {
        return d.into();
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    let dirs = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).unwrap_or_default();
    for line in dirs.lines() {
        if let Some(v) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") {
            let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
            return v.into();
        }
    }
    home.join("Downloads")
}

/// The file name a server suggests (`Content-Disposition: attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf`
/// or `filename="report.csv"`), else the last part of the URL, made safe for a file name.
pub fn file_name(url: &str, disposition: Option<&str>) -> String {
    let from_header = disposition.and_then(|d| {
        let parts: Vec<&str> = d.split(';').map(str::trim).collect();
        let star = parts.iter().find_map(|p| p.strip_prefix("filename*=")).map(|v| {
            let v = v.trim_matches('"');
            let encoded = v.splitn(3, '\'').nth(2).unwrap_or(v);
            crate::net::percent_decode_str(encoded)
        });
        star.or_else(|| parts.iter().find_map(|p| p.strip_prefix("filename=")).map(|v| v.trim_matches('"').to_string()))
    });
    let from_url = || {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.path_segments().and_then(|mut s| s.next_back().map(str::to_string)))
            .filter(|s| !s.is_empty())
            .map(|s| crate::net::percent_decode_str(&s))
    };
    let name = from_header.filter(|s| !s.trim().is_empty()).or_else(from_url).unwrap_or_else(|| "download".into());
    // No folders, no hidden files, no control characters.
    let safe: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let safe = safe.trim().trim_start_matches('.').to_string();
    if safe.is_empty() { "download".into() } else { safe.chars().take(200).collect() }
}

/// A name in `dir` that isn't taken yet: `report.csv`, then `report (1).csv`, …
fn unique(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let taken = |p: &Path| p.exists() || p.with_file_name(format!("{}.part", p.file_name().unwrap_or_default().to_string_lossy())).exists();
    let mut p = dir.join(name);
    let mut n = 1;
    while taken(&p) {
        p = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    p
}

/// Start saving a response. `prefix` is any of the body already read (to look at its type).
pub fn start(url: &str, name: &str, total: Option<u64>, prefix: Vec<u8>, mut body: impl Read + Send + 'static) -> u32 {
    let dir = folder();
    let _ = std::fs::create_dir_all(&dir);
    let path = unique(&dir, name);
    let cancel = Arc::new(AtomicBool::new(false));
    let id = {
        let mut list = LIST.lock().expect("downloads");
        let id = list.last().map_or(1, |d| d.id + 1);
        list.push(Download { id, url: url.to_string(), path: path.clone(), total, received: 0, state: State::Active, started: Instant::now(), cancel: cancel.clone() });
        id
    };
    log!("[download] {url} -> {}", path.display());
    std::thread::spawn(move || {
        let part = path.with_file_name(format!("{}.part", path.file_name().unwrap_or_default().to_string_lossy()));
        let result = (|| -> Result<(), String> {
            let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            f.write_all(&prefix).map_err(|e| e.to_string())?;
            let mut got = prefix.len() as u64;
            update(id, |d| d.received = got);
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".into());
                }
                let n = body.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                got += n as u64;
                update(id, |d| d.received = got);
            }
            f.flush().map_err(|e| e.to_string())?;
            std::fs::rename(&part, &path).map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => update(id, |d| d.state = State::Done),
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                let state = if e == "cancelled" { State::Cancelled } else { State::Failed(e) };
                update(id, |d| d.state = state);
            }
        }
    });
    id
}

fn update(id: u32, f: impl FnOnce(&mut Download)) {
    if let Ok(mut list) = LIST.lock() {
        if let Some(d) = list.iter_mut().find(|d| d.id == id) {
            f(d);
        }
    }
}

pub fn list() -> Vec<Download> {
    LIST.lock().map(|l| l.clone()).unwrap_or_default()
}

/// Downloads still running.
pub fn active() -> usize {
    LIST.lock().map_or(0, |l| l.iter().filter(|d| d.state == State::Active).count())
}

/// Overall progress of running downloads (0–1), when their sizes are known.
pub fn progress() -> Option<f32> {
    let l = LIST.lock().ok()?;
    let running: Vec<&Download> = l.iter().filter(|d| d.state == State::Active).collect();
    let total: u64 = running.iter().map(|d| d.total.unwrap_or(0)).sum();
    if running.is_empty() || total == 0 || running.iter().any(|d| d.total.is_none()) {
        return None;
    }
    Some(running.iter().map(|d| d.received).sum::<u64>() as f32 / total as f32)
}

fn human(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} bytes"),
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// about:downloads (`?cancel=id`, `?open=id`, `?folder`, `?clear`).
pub fn page(query: &str) -> String {
    let q = query.trim_start_matches('?');
    if let Some(id) = q.strip_prefix("cancel=").and_then(|v| v.parse::<u32>().ok()) {
        if let Some(d) = list().into_iter().find(|d| d.id == id) {
            d.cancel.store(true, Ordering::Relaxed);
        }
    } else if let Some(id) = q.strip_prefix("open=").and_then(|v| v.parse::<u32>().ok()) {
        if let Some(d) = list().into_iter().find(|d| d.id == id && d.state == State::Done) {
            let _ = std::process::Command::new("xdg-open").arg(&d.path).spawn();
        }
    } else if q == "folder" {
        let _ = std::process::Command::new("xdg-open").arg(folder()).spawn();
    } else if q == "clear" {
        if let Ok(mut l) = LIST.lock() {
            l.retain(|d| d.state == State::Active);
        }
    }
    let rows: String = list()
        .iter()
        .rev()
        .map(|d| {
            let name = d.path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let (status, bar, actions) = match &d.state {
                State::Active => {
                    let secs = d.started.elapsed().as_secs_f64().max(0.1);
                    let rate = d.received as f64 / secs;
                    let pct = d.total.map(|t| (d.received as f64 / t.max(1) as f64 * 100.0).min(100.0));
                    let text = match d.total {
                        Some(t) => format!("{} of {} · {}/s", human(d.received), human(t), human(rate as u64)),
                        None => format!("{} · {}/s", human(d.received), human(rate as u64)),
                    };
                    (text, format!("<div class=\"bar\"><div style=\"width:{:.0}%\"></div></div>", pct.unwrap_or(30.0)), format!("<a href=\"about:downloads?cancel={}\">Cancel</a>", d.id))
                }
                State::Done => (format!("Done · {}", human(d.received)), String::new(), format!("<a href=\"about:downloads?open={}\">Open</a>", d.id)),
                State::Failed(e) => (format!("Failed: {}", esc(e)), String::new(), format!("<a href=\"{}\">Try again</a>", esc(&d.url))),
                State::Cancelled => ("Cancelled".into(), String::new(), format!("<a href=\"{}\">Download again</a>", esc(&d.url))),
            };
            format!(
                "<div class=\"row\"><div class=\"t\"><b>{}</b><div class=\"u\">{}</div>{bar}<div class=\"s\">{status}</div></div><div class=\"a\">{actions}</div></div>",
                esc(&name),
                esc(&d.url)
            )
        })
        .collect();
    let empty = if rows.is_empty() { "<p class=\"sub\">Nothing downloaded in this session.</p>" } else { "" };
    format!(
        "<!DOCTYPE html><html><head><title>Downloads</title><style>body{{font-family:sans-serif;margin:32px auto;max-width:860px;padding:0 20px;color:#1f2937}}\
         .sub{{color:#64748b}}.row{{display:flex;gap:12px;align-items:center;padding:10px;border-bottom:1px solid #e5e7eb}}.t{{flex:1;overflow:hidden}}\
         .u{{color:#64748b;font-size:12px}}.s{{color:#475569;font-size:13px;margin-top:2px}}.bar{{height:6px;background:#e5e7eb;border-radius:3px;margin-top:6px}}\
         .bar div{{height:6px;background:#4f46e5;border-radius:3px}}.a a{{color:#1d4ed8}}</style></head><body><h1>Downloads</h1>\
         <p class=\"sub\">Saved to {} · <a href=\"about:downloads?folder\">Open folder</a> · <a href=\"about:downloads?clear\">Clear list</a></p>{rows}{empty}</body></html>",
        esc(&folder().to_string_lossy())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_headers_and_urls() {
        assert_eq!(file_name("https://a.com/x/report.pdf?v=2", None), "report.pdf");
        assert_eq!(file_name("https://a.com/get", Some("attachment; filename=\"sales report.csv\"")), "sales report.csv");
        assert_eq!(file_name("https://a.com/get", Some("attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf; filename=\"resume.pdf\"")), "résumé.pdf");
        assert_eq!(file_name("https://a.com/", None), "download");
        assert_eq!(file_name("https://a.com/x", Some("attachment; filename=\"../../.bashrc\"")), "_.._.bashrc");
        assert_eq!(file_name("https://a.com/My%20File.zip", None), "My File.zip");
    }

    #[test]
    fn unique_names() {
        let d = std::env::temp_dir().join(format!("ironswift-dl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        std::fs::write(d.join("a.txt"), "x").unwrap();
        std::fs::write(d.join("a (1).txt"), "x").unwrap();
        assert_eq!(unique(&d, "a.txt"), d.join("a (2).txt"));
        assert_eq!(unique(&d, "b.txt"), d.join("b.txt"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
