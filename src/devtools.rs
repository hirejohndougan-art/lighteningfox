//! Developer tools (F12, Ctrl+Shift+I, or "Inspect" in the right-click menu): a snapshot of
//! the page in a new tab — its element tree, console messages and errors, and the network
//! requests it made — taken when you open it. Reopen (F12 again) to refresh.

use crate::dom::{Dom, NodeData, NodeId};
use std::sync::Mutex;
use std::time::Instant;

static SNAPSHOT: Mutex<String> = Mutex::new(String::new());
/// The page being inspected (its document number), for running console code in it.
static INSPECTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Code run from the console and what it gave back (newest last).
static HISTORY: Mutex<Vec<(String, Result<String, String>)>> = Mutex::new(Vec::new());
const HISTORY_SPOT: &str = "<!--console-history-->";

/// Remember which page the developer tools are looking at. A new page starts a new
/// console history.
pub fn set_inspected(doc: u64) {
    if INSPECTED.swap(doc, std::sync::atomic::Ordering::Relaxed) != doc {
        if let Ok(mut h) = HISTORY.lock() {
            h.clear();
        }
    }
}

pub fn inspected() -> u64 {
    INSPECTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The console's Run button goes to `about:devtools-run?code=…`: the code to run, if `url`
/// is such a request.
pub fn run_request(url: &str) -> Option<String> {
    let q = url.strip_prefix("about:devtools-run")?;
    let q = q.strip_prefix('?').unwrap_or(q);
    let code = q.split('&').find_map(|kv| kv.strip_prefix("code="))?;
    let code = crate::net::percent_decode_str(&code.replace('+', " "));
    (!code.trim().is_empty()).then_some(code)
}

/// Keep the result of code run in the console.
pub fn record(code: &str, result: Result<String, String>) {
    if let Ok(mut h) = HISTORY.lock() {
        h.push((code.to_string(), result));
        let n = h.len();
        if n > 200 {
            h.drain(..n - 200);
        }
    }
}

/// How many times code has been run (so each result page has its own address).
pub fn runs() -> usize {
    HISTORY.lock().map_or(0, |h| h.len())
}

fn history_html() -> String {
    let h = HISTORY.lock().map(|h| h.clone()).unwrap_or_default();
    let mut out = String::new();
    for (code, r) in &h {
        out.push_str(&format!("<div class=\"c in\">&gt; {}</div>", esc(code)));
        match r {
            Ok(v) => out.push_str(&format!("<div class=\"c out\">{}</div>", esc(v))),
            Err(e) => out.push_str(&format!("<div class=\"c err\">Uncaught {}</div>", esc(e))),
        }
    }
    out.push_str(
        "<form class=\"run\" action=\"about:devtools-run\" method=\"get\"><span>&gt;</span>\
         <input name=\"code\" autofocus autocomplete=\"off\" placeholder=\"Type JavaScript to run in the page, then press Enter\">\
         <button>Run</button></form>",
    );
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The snapshot taken last (to hand it from a site process to the window).
pub fn snapshot() -> String {
    SNAPSHOT.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Use a snapshot taken elsewhere (by the page's site process).
pub fn set_snapshot(s: String) {
    if let Ok(mut g) = SNAPSHOT.lock() {
        *g = s;
    }
}

/// The page shown at about:devtools.
pub fn page() -> String {
    let s = SNAPSHOT.lock().map(|s| s.clone()).unwrap_or_default();
    let s = s.replace(HISTORY_SPOT, &history_html());
    if s.is_empty() {
        "<!DOCTYPE html><title>Developer tools</title><body style=\"font-family:sans-serif;margin:32px\"><h1>Developer tools</h1><p>Press F12 on a page to inspect it.</p>".into()
    } else {
        s
    }
}

/// Take a snapshot of the page now in the window (its DOM must be the current one).
pub fn capture(dom: &Dom, url: &str, title: &str, loaded: Instant, scripts: usize, errors: usize) {
    let mut rows = String::new();
    let mut count = 0usize;
    for &c in &dom.nodes[Dom::ROOT].children {
        element_rows(dom, c, 0, &mut rows, &mut count);
    }
    if count >= MAX_ROWS {
        rows.push_str("<div class=\"more\">… (the rest of the tree is left out)</div>");
    }
    // Console: what this page's scripts logged since it loaded.
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).saturating_sub(loaded.elapsed().as_secs() + 1);
    let console: String = crate::crashlog::lines()
        .into_iter()
        .filter(|(t, l)| *t >= since && l.starts_with("[js"))
        .rev()
        .take(400)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(_, l)| {
            let class = if l.starts_with("[js error]") || l.starts_with("[js err") { "err" } else if l.starts_with("[js warn") { "warn" } else { "log" };
            format!("<div class=\"c {class}\">{}</div>", esc(&l))
        })
        .collect();
    let console = if console.is_empty() { "<div class=\"none\">No console messages.</div>".to_string() } else { console };
    let net: String = crate::http::requests_since(Some(loaded))
        .iter()
        .map(|r| {
            let (status, class) = match &r.result {
                Ok(s) => (s.to_string(), if *s >= 400 { "err" } else { "" }),
                Err(e) => (esc(e), "err"),
            };
            format!(
                "<tr class=\"{class}\"><td>{}</td><td>{status}</td><td class=\"u\">{}</td><td>{}</td><td>{} ms</td></tr>",
                esc(&r.method),
                esc(&r.url),
                esc(r.content_type.split(';').next().unwrap_or("")),
                r.took.as_millis()
            )
        })
        .collect();
    let net = if net.is_empty() { "<tr><td colspan=5 class=\"none\">No requests recorded since the page loaded.</td></tr>".to_string() } else { net };
    let html = format!(
        "<!DOCTYPE html><html><head><title>Developer tools: {t}</title><style>\
         body{{font-family:sans-serif;margin:0;color:#1f2937}}header{{background:#f1f5f9;border-bottom:1px solid #cbd5e1;padding:12px 20px}}\
         h1{{font-size:18px;margin:0 0 4px}}.sub{{color:#64748b;font-size:13px}}nav a{{margin-right:16px;color:#1d4ed8;font-size:14px}}\
         section{{padding:12px 20px}}h2{{font-size:15px;border-bottom:1px solid #e2e8f0;padding-bottom:4px}}\
         .el{{font-family:monospace;font-size:12.5px;white-space:pre;line-height:1.5}}.tag{{color:#881280}}.an{{color:#994500}}.av{{color:#1a1aa6}}.tx{{color:#334155}}.cm{{color:#236e25}}\
         .c{{font-family:monospace;font-size:12.5px;padding:3px 6px;border-bottom:1px solid #f1f5f9;white-space:pre-wrap}}.c.err{{background:#fef2f2;color:#b91c1c}}.c.warn{{background:#fffbeb;color:#92400e}}\
         table{{border-collapse:collapse;width:100%;font-size:12.5px}}td{{border-bottom:1px solid #f1f5f9;padding:3px 6px;vertical-align:top}}td.u{{font-family:monospace;word-break:break-all}}tr.err td{{color:#b91c1c}}\
         .none,.more{{color:#94a3b8;font-style:italic}}\
         .c.in{{color:#1d4ed8}}.c.out{{color:#334155}}form.run{{display:flex;gap:6px;align-items:center;margin-top:8px;font-family:monospace}}\
         form.run input{{flex:1;font-family:monospace;font-size:13px;padding:4px}}</style></head><body>\
         <header><h1>Developer tools</h1><div class=\"sub\">{t} · {u} · {n} nodes · {scripts} script{ss}, {errors} error{es} · snapshot taken when opened (press F12 on the page again to refresh)</div>\
         <nav><a href=\"#elements\">Elements</a><a href=\"#console\">Console</a><a href=\"#network\">Network</a></nav></header>\
         <section id=\"elements\"><h2>Elements</h2><div class=\"el\">{rows}</div></section>\
         <section id=\"console\"><h2>Console</h2>{console}{HISTORY_SPOT}</section>\
         <section id=\"network\"><h2>Network</h2><table><tr><th align=left>Method</th><th align=left>Status</th><th align=left>URL</th><th align=left>Type</th><th align=left>Time</th></tr>{net}</table></section>\
         </body></html>",
        t = esc(if title.is_empty() { url } else { title }),
        u = esc(url),
        n = dom.nodes.len(),
        ss = if scripts == 1 { "" } else { "s" },
        es = if errors == 1 { "" } else { "s" },
    );
    if let Ok(mut s) = SNAPSHOT.lock() {
        *s = html;
    }
}

const MAX_ROWS: usize = 5000;

fn element_rows(dom: &Dom, id: NodeId, depth: usize, out: &mut String, count: &mut usize) {
    if *count >= MAX_ROWS {
        return;
    }
    *count += 1;
    let pad = "  ".repeat(depth);
    match &dom.nodes[id].data {
        NodeData::Element { tag, attrs } => {
            let a: String = attrs
                .iter()
                .filter(|(k, _)| !k.starts_with("__"))
                .map(|(k, v)| {
                    let v: String = v.chars().take(120).collect();
                    format!(" <span class=\"an\">{}</span>=<span class=\"av\">\"{}\"</span>", esc(k), esc(&v))
                })
                .collect();
            out.push_str(&format!("{pad}<span class=\"tag\">&lt;{}</span>{a}<span class=\"tag\">&gt;</span>\n", esc(tag)));
            if let Some(&root) = dom.shadow_roots.get(&id) {
                out.push_str(&format!("{pad}  <span class=\"cm\">#shadow-root</span>\n"));
                for &c in &dom.nodes[root].children {
                    element_rows(dom, c, depth + 2, out, count);
                }
            }
            if let Some(&content) = dom.template_contents.get(&id) {
                out.push_str(&format!("{pad}  <span class=\"cm\">#content</span>\n"));
                for &c in &dom.nodes[content].children {
                    element_rows(dom, c, depth + 2, out, count);
                }
            }
            for &c in &dom.nodes[id].children {
                element_rows(dom, c, depth + 1, out, count);
            }
        }
        NodeData::Text(t) => {
            let t = t.trim();
            if !t.is_empty() {
                let short: String = t.chars().take(160).collect();
                let more = if t.chars().count() > 160 { "…" } else { "" };
                out.push_str(&format!("{pad}<span class=\"tx\">\"{}{more}\"</span>\n", esc(&short)));
            } else {
                *count -= 1;
            }
        }
        NodeData::Comment(c) => {
            let short: String = c.chars().take(100).collect();
            out.push_str(&format!("{pad}<span class=\"cm\">&lt;!--{}--&gt;</span>\n", esc(&short)));
        }
        NodeData::Doctype { name, .. } => out.push_str(&format!("{pad}<span class=\"cm\">&lt;!DOCTYPE {}&gt;</span>\n", esc(name))),
        _ => {}
    }
}

#[cfg(test)]
mod console_tests {
    use super::*;

    #[test]
    fn run_requests() {
        assert_eq!(run_request("about:devtools-run?code=1+%2B+2").as_deref(), Some("1 + 2"));
        assert_eq!(run_request("about:devtools-run?code="), None);
        assert_eq!(run_request("https://example.com/?code=1"), None);
    }
}
