//! Debug log and crash reports, kept on this computer only.
//!
//! Everything the browser logs (`log!`) also goes into a ring buffer shown on about:log.
//! If the browser crashes, a report (what went wrong, where in the code, the page you were
//! on and the last log lines) is written to `<profile>/crashes/` and listed on about:crashes.

use std::collections::VecDeque;
use std::sync::Mutex;

const KEEP_LINES: usize = 2000;

static LOG: Mutex<VecDeque<(u64, String)>> = Mutex::new(VecDeque::new());
static CURRENT_URL: Mutex<String> = Mutex::new(String::new());

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Add a line to the debug log (the `log!` macro calls this after printing it).
pub fn record(line: String) {
    if let Ok(mut l) = LOG.lock() {
        if l.len() >= KEEP_LINES {
            l.pop_front();
        }
        l.push_back((now_secs(), line));
    }
}

/// The page being shown, for crash reports.
pub fn set_url(url: &str) {
    if let Ok(mut u) = CURRENT_URL.lock() {
        url.clone_into(&mut u);
    }
}

fn crash_dir() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("crashes"))
}

fn clock(t: u64) -> String {
    // UTC time of day; enough to line log entries up with each other.
    format!("{:02}:{:02}:{:02}", (t / 3600) % 24, (t / 60) % 60, t % 60)
}

/// Write crash reports when the browser panics (the default message still prints too).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let when = now_secs();
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let place = info.location().map_or("unknown place".into(), |l| format!("{}:{}", l.file(), l.line()));
        let url = CURRENT_URL.try_lock().map(|u| u.clone()).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let trace = std::backtrace::Backtrace::force_capture();
        let recent: Vec<String> = LOG
            .try_lock()
            .map(|l| l.iter().rev().take(200).rev().map(|(t, s)| format!("{} {s}", clock(*t))).collect())
            .unwrap_or_default();
        let report = format!(
            "Ironswift crash report\n\
             version: {}\ntime (UTC seconds): {when}\nOS: {} {}\nthread: {thread}\n\n\
             What went wrong: {what}\nWhere: {place}\nPage: {}\n\n\
             Last log lines:\n{}\n\nStack:\n{trace}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            if url.is_empty() { "(none)" } else { &url },
            recent.join("\n"),
        );
        if let Some(dir) = crash_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("crash-{when}.txt"));
            if std::fs::write(&path, report).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                eprintln!("[crash] report saved to {}", path.display());
            }
        }
        default(info);
    }));
}

/// Crash reports, newest first: (file name, first lines summary).
fn reports() -> Vec<(String, String)> {
    let Some(dir) = crash_dir() else { return Vec::new() };
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("crash-") || !name.ends_with(".txt") {
                return None;
            }
            let text = std::fs::read_to_string(e.path()).ok()?;
            let what = text.lines().find_map(|l| l.strip_prefix("What went wrong: ")).unwrap_or("").to_string();
            Some((name, what))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const STYLE: &str = "body{font-family:sans-serif;margin:32px auto;max-width:980px;padding:0 20px;color:#1f2937}h1{margin-bottom:4px}.sub{color:#64748b}pre{background:#f8fafc;border:1px solid #e5e7eb;border-radius:8px;padding:12px;font-size:12.5px;white-space:pre-wrap;overflow:hidden}.row{padding:8px 10px;border-bottom:1px solid #e5e7eb}.row a{font-weight:600;color:#1d4ed8;text-decoration:none}.what{color:#64748b;font-size:13px}form input{width:300px;font-size:14px;padding:4px 8px}";

/// about:log — the debug log, optionally filtered (`?q=word`).
pub fn log_page(query: &str) -> String {
    let q = query
        .trim_start_matches('?')
        .split('&')
        .find_map(|p| p.strip_prefix("q="))
        .map(|v| crate::net::percent_decode_str(&v.replace('+', " ")))
        .unwrap_or_default();
    let lines: Vec<String> = LOG
        .lock()
        .map(|l| {
            l.iter()
                .filter(|(_, s)| q.is_empty() || s.to_lowercase().contains(&q.to_lowercase()))
                .map(|(t, s)| format!("{} {}", clock(*t), esc(s)))
                .collect()
        })
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html><html><head><title>Debug log</title><style>{STYLE}</style></head><body><h1>Debug log</h1>\
         <p class=\"sub\">The last {KEEP_LINES} messages from this session (times in UTC). Page errors from JavaScript show up here too. \
         <a href=\"about:crashes\">Crash reports</a></p>\
         <form action=\"about:log\" method=\"get\"><input type=\"text\" name=\"q\" value=\"{}\" placeholder=\"Filter, e.g. js error\"> <button type=\"submit\">Filter</button></form>\
         <p class=\"sub\">{} line{}</p><pre>{}</pre></body></html>",
        esc(&q),
        lines.len(),
        if lines.len() == 1 { "" } else { "s" },
        if lines.is_empty() { "(nothing yet)".to_string() } else { lines.join("\n") }
    )
}

/// about:crashes — saved crash reports; `?view=<file>` shows one.
pub fn crashes_page(query: &str) -> String {
    if let Some(name) = query.trim_start_matches('?').strip_prefix("view=") {
        let name = crate::net::percent_decode_str(name);
        // Only plain report file names, never a path.
        let safe = name.starts_with("crash-") && name.ends_with(".txt") && !name.contains(['/', '\\']) && !name.contains("..");
        let text = crash_dir().filter(|_| safe).and_then(|d| std::fs::read_to_string(d.join(&name)).ok()).unwrap_or_else(|| "No such report.".into());
        return format!(
            "<!DOCTYPE html><html><head><title>Crash report</title><style>{STYLE}</style></head><body><h1>Crash report</h1>\
             <p class=\"sub\"><a href=\"about:crashes\">All reports</a> · saved on this computer only; attach it to a bug report if you like.</p><pre>{}</pre></body></html>",
            esc(&text)
        );
    }
    let list = reports();
    let rows: String = list
        .iter()
        .map(|(f, what)| format!("<div class=\"row\"><a href=\"about:crashes?view={f}\">{f}</a><div class=\"what\">{}</div></div>", esc(what)))
        .collect();
    format!(
        "<!DOCTYPE html><html><head><title>Crash reports</title><style>{STYLE}</style></head><body><h1>Crash reports</h1>\
         <p class=\"sub\">{} report{}, kept on this computer only (nothing is sent anywhere). <a href=\"about:log\">Debug log</a></p>{}</body></html>",
        list.len(),
        if list.len() == 1 { "" } else { "s" },
        if rows.is_empty() { "<p class=\"sub\">No crashes recorded.</p>".to_string() } else { rows }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_keeps_the_newest_lines() {
        for i in 0..KEEP_LINES + 5 {
            record(format!("line {i}"));
        }
        let l = LOG.lock().unwrap();
        assert_eq!(l.len(), KEEP_LINES);
        assert_eq!(l.back().unwrap().1, format!("line {}", KEEP_LINES + 4));
    }

    #[test]
    fn crash_page_refuses_paths() {
        let page = crashes_page("?view=../../etc/passwd");
        assert!(page.contains("No such report."));
    }
}
