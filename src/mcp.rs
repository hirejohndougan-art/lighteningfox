//! `ironswift --mcp`: a Model Context Protocol server over stdin/stdout, so Claude (or any MCP
//! client) can drive the browser headless and find, test and fix compatibility gaps.
//!
//! No network port: only the program that started it can talk to it. Messages are JSON-RPC 2.0,
//! one per line. Everything the browser logs goes to stderr, so stdout carries only replies.
//! See docs/CLAUDE_CONNECTION.md.

use crate::paint::Canvas;
use crate::{css, geometry, images, js, layout, net, Page};
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL: &str = "2025-06-18";

/// The browser state a session drives: one page at a time.
struct Session {
    fonts: &'static crate::Fonts,
    page: Option<Page>,
    lay: Option<layout::Layout>,
    width: usize,
    height: usize,
    scroll: f32,
    /// The log mark when the page started loading: its errors, console and requests follow it.
    mark: u64,
}

pub fn serve(fonts: &'static crate::Fonts) {
    images::freeze_animations();
    let mut s = Session { fonts, page: None, lay: None, width: 1280, height: 800, scroll: 0.0, mark: 0 };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => s.handle(&msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

fn ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}]})
}

fn failed(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": true})
}

/// A JavaScript string literal.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

impl Session {
    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone(); // notifications get no reply
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL);
                ok(
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "ironswift", "version": env!("CARGO_PKG_VERSION")},
                        "instructions": "Drive the Ironswift browser headless: open_url, then look at the page (screenshot, compare_with_chrome, script_errors, unsupported_features) and fix what differs in the engine. See docs/CLAUDE_CONNECTION.md for the loop."
                    }),
                )
            }
            "ping" => ok(&id, json!({})),
            "tools/list" => ok(&id, json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                ok(&id, self.call(name, &args))
            }
            _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unknown method {method}")}}),
        })
    }

    fn call(&mut self, name: &str, a: &Value) -> Value {
        let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_string);
        let n = |k: &str| a.get(k).and_then(Value::as_f64);
        if name != "open_url" && name != "run_site_check" && name != "run_wpt" && self.page.is_none() {
            return failed("No page is open: call open_url first.");
        }
        match name {
            "open_url" => {
                let Some(url) = s("url") else { return failed("url is required") };
                if let Some(w) = n("width") {
                    self.width = (w as usize).clamp(200, 4000);
                }
                if let Some(h) = n("height") {
                    self.height = (h as usize).clamp(200, 4000);
                }
                self.open(&net::normalize_input(&url));
                text(self.summary())
            }
            "click" => match s("selector") {
                Some(sel) => self.click(&sel),
                None => failed("selector is required"),
            },
            "type" => match (s("selector"), s("text")) {
                (Some(sel), Some(t)) => self.type_text(&sel, &t),
                _ => failed("selector and text are required"),
            },
            "scroll" => {
                let y = match (n("y"), n("by")) {
                    (Some(y), _) => y as f32,
                    (None, Some(d)) => self.scroll + d as f32,
                    _ => self.scroll + self.height as f32 * 0.8,
                };
                let max = self.lay.as_ref().map_or(0.0, |l| (l.height - self.height as f32).max(0.0));
                self.scroll = y.clamp(0.0, max);
                geometry::set_scroll(self.scroll);
                self.settle(500);
                text(format!("Scrolled to y = {} (page is {} px tall).", self.scroll, self.lay.as_ref().map_or(0.0, |l| l.height)))
            }
            "wait_for" => self.wait_for(s("selector"), n("timeout_ms").unwrap_or(5000.0) as u64),
            "screenshot" => {
                let full = a.get("full_page").and_then(Value::as_bool).unwrap_or(false);
                match self.png(full) {
                    Ok(bytes) => image_result(&bytes, &format!("{}×{} {}", self.width, if full { "full page" } else { "viewport" }, self.url())),
                    Err(e) => failed(e),
                }
            }
            "compare_with_chrome" => self.compare(s("chrome_png")),
            "script_errors" => text(self.lines(|l| l.starts_with("[js error]")).join("\n")).or_empty("No script errors."),
            "console_log" => text(self.lines(|l| l.starts_with("[js log]") || l.starts_with("[js warn]") || l.starts_with("[js info]") || l.starts_with("[js debug]") || l.starts_with("[js trace]")).join("\n")).or_empty("Nothing logged."),
            "failed_requests" => text(self.lines(is_failed_request).join("\n")).or_empty("No failed requests."),
            "dom" => {
                let code = match s("selector") {
                    Some(sel) => format!("(function(){{var e=document.querySelector({});return e?e.outerHTML:'no element matches'}})()", js_str(&sel)),
                    None => "document.documentElement.outerHTML".into(),
                };
                let max = n("max_chars").unwrap_or(100_000.0) as usize;
                self.js(&code).map(|t| text(cut(&t, max))).unwrap_or_else(failed)
            }
            "computed_style" => {
                let Some(sel) = s("selector") else { return failed("selector is required") };
                let props: Vec<String> = a.get("properties").and_then(Value::as_array).map(|v| v.iter().filter_map(|p| p.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let code = format!(
                    "(function(){{var e=document.querySelector({}); if(!e) return 'no element matches'; var cs=getComputedStyle(e), p={}, o={{}}; if(!p.length) for(var i=0;i<cs.length;i++) p.push(cs.item(i)); p.forEach(function(k){{o[k]=cs.getPropertyValue(k)}}); return JSON.stringify(o,null,1)}})()",
                    js_str(&sel),
                    serde_json::to_string(&props).unwrap_or_else(|_| "[]".into())
                );
                self.js(&code).map(text).unwrap_or_else(failed)
            }
            "box" => {
                let Some(sel) = s("selector") else { return failed("selector is required") };
                let code = format!(
                    "(function(){{var l=document.querySelectorAll({}); if(!l.length) return 'no element matches'; return JSON.stringify([].slice.call(l,0,20).map(function(e){{var r=e.getBoundingClientRect(), cs=getComputedStyle(e); return {{tag:e.tagName.toLowerCase(), x:r.left, y:r.top+scrollY, width:r.width, height:r.height, display:cs.display, position:cs.position}}}}),null,1)}})()",
                    js_str(&sel)
                );
                self.js(&code).map(text).unwrap_or_else(failed)
            }
            "run_js" => {
                let Some(code) = s("code") else { return failed("code is required") };
                let r = self.js(&code);
                self.after_script();
                r.map(text).unwrap_or_else(|e| failed(format!("Uncaught: {e}")))
            }
            "unsupported_features" => text(self.unsupported()),
            "run_site_check" => {
                let urls: Vec<String> = a.get("urls").and_then(Value::as_array).map(|v| v.iter().filter_map(|u| u.as_str().map(str::to_string)).collect()).unwrap_or_default();
                tool_script("site_check.py", &urls)
            }
            "run_wpt" => match (s("wpt_root"), s("dir")) {
                (Some(root), Some(dir)) => tool_script("wpt.py", &[root, dir]),
                _ => failed("wpt_root (a web-platform-tests checkout) and dir (such as css/css-flexbox) are required"),
            },
            _ => failed(format!("unknown tool {name}")),
        }
    }

    fn url(&self) -> String {
        self.page.as_ref().map_or_else(String::new, |p| p.url.clone())
    }

    /// Load a page, give its scripts and requests up to 5 s to settle, and lay it out.
    fn open(&mut self, url: &str) {
        self.page = None;
        self.lay = None;
        self.scroll = 0.0;
        self.mark = crate::crashlog::count();
        geometry::set_viewport(self.width as f32, self.height as f32);
        css::set_viewport(self.width as f32, self.height as f32);
        geometry::set_scroll(0.0);
        self.page = Some(crate::load(url));
        self.settle(5000);
    }

    /// Run timers and requests (up to `ms`), follow a navigation scripts asked for, restyle
    /// and lay out again.
    fn settle(&mut self, ms: u64) {
        let Some(page) = self.page.as_mut() else { return };
        if let Some(j) = page.js.as_mut() {
            j.settle(std::time::Duration::from_millis(ms));
        }
        if let Some(t) = js::take_navigation() {
            let t = net::resolve(&page.url, &t);
            self.open(&t);
            return;
        }
        let page = self.page.as_mut().unwrap();
        if js::take_dirty() {
            page.restyle();
        }
        let mut lay = page.layout(self.fonts, self.width as f32, self.height as f32);
        if page.load_lazy_now(&lay, self.height as f32) {
            lay = page.layout(self.fonts, self.width as f32, self.height as f32);
        }
        if let Some(y) = geometry::take_scroll_request() {
            self.scroll = y.clamp(0.0, (lay.height - self.height as f32).max(0.0));
            geometry::set_scroll(self.scroll);
        }
        if let Some(j) = page.js.as_mut() {
            j.after_layout();
        }
        self.lay = Some(lay);
    }

    fn after_script(&mut self) {
        self.settle(1000);
    }

    fn summary(&self) -> String {
        let (Some(page), Some(lay)) = (self.page.as_ref(), self.lay.as_ref()) else { return "Nothing loaded.".into() };
        let st = page.js.as_ref().map(|j| j.stats.clone()).unwrap_or_default();
        format!(
            "Loaded {}\ntitle: {}\n{} nodes, page {}×{} px\nJS: {} scripts, {} errors (script_errors lists them)\n{} failed requests",
            page.url,
            page.title,
            js::with_dom(|d| d.nodes.len()),
            self.width,
            lay.height.ceil(),
            st.scripts,
            st.errors,
            self.lines(is_failed_request).len()
        )
    }

    fn js(&mut self, code: &str) -> Result<String, String> {
        match self.page.as_mut().and_then(|p| p.js.as_mut()) {
            Some(j) => j.eval_text(code),
            None => Err("This page has no scripts, so there's no JavaScript engine to ask.".into()),
        }
    }

    /// The node `selector` picks, by its id in the DOM.
    fn node(&mut self, selector: &str) -> Result<crate::dom::NodeId, String> {
        let r = self.js(&format!("(function(){{var e=document.querySelector({});return e?String(e.__id):'-1'}})()", js_str(selector)))?;
        match r.parse::<i64>() {
            Ok(n) if n >= 0 => Ok(n as usize),
            _ => Err(format!("No element matches {selector}")),
        }
    }

    fn click(&mut self, selector: &str) -> Value {
        let node = match self.node(selector) {
            Ok(n) => n,
            Err(e) => return failed(e),
        };
        let before = self.url();
        let allowed = self.page.as_mut().and_then(|p| p.js.as_mut()).is_none_or(|j| j.click(node));
        let mut did = String::from("Clicked");
        if allowed {
            // The click's default action: follow a link, tick a box, submit a form.
            let code = format!(
                "(function(){{var e=document.querySelector({}); if(!e) return ''; var t=(e.type||'').toLowerCase();
                 if(e.tagName==='INPUT'&&(t==='checkbox'||t==='radio')){{e.checked=t==='radio'?true:!e.checked; e.dispatchEvent(new Event('input',{{bubbles:true}})); e.dispatchEvent(new Event('change',{{bubbles:true}})); return 'checked='+e.checked}}
                 var f=e.closest('button,input[type=submit],input[type=image]'); if(f&&f.form&&(f.type||'submit')==='submit'){{ f.form.requestSubmit ? f.form.requestSubmit(f) : f.form.submit(); return 'submitted' }}
                 var a=e.closest('a[href],area[href]'); if(a&&!a.target) return 'link '+a.href; return ''}})()",
                js_str(selector)
            );
            match self.js(&code) {
                Ok(r) if r.starts_with("link ") => {
                    let href = r[5..].to_string();
                    if !href.starts_with("javascript:") && !href.contains('#') || href.split('#').next() != Some(before.split('#').next().unwrap_or("")) {
                        self.open(&href);
                        return text(format!("Followed the link.\n{}", self.summary()));
                    }
                }
                Ok(r) if !r.is_empty() => did = format!("Clicked ({r})"),
                _ => {}
            }
        } else {
            did = "Clicked (the page cancelled the default action)".into();
        }
        self.after_script();
        if self.url() != before {
            return text(format!("{did}; the page navigated.\n{}", self.summary()));
        }
        text(did)
    }

    fn type_text(&mut self, selector: &str, t: &str) -> Value {
        let code = format!(
            "(function(){{var e=document.querySelector({}); if(!e) return 'no element matches'; e.focus&&e.focus();
             if('value' in e){{ e.value=(e.value||'')+{}; }} else if(e.isContentEditable){{ e.textContent+= {} }}
             e.dispatchEvent(new InputEvent('input',{{bubbles:true,data:{}}})); e.dispatchEvent(new Event('change',{{bubbles:true}})); return 'value now: '+('value' in e?e.value:e.textContent)}})()",
            js_str(selector),
            js_str(t),
            js_str(t),
            js_str(t)
        );
        let r = self.js(&code);
        self.after_script();
        r.map(text).unwrap_or_else(failed)
    }

    fn wait_for(&mut self, selector: Option<String>, timeout_ms: u64) -> Value {
        let t0 = std::time::Instant::now();
        loop {
            self.settle(200);
            if let Some(sel) = &selector {
                if self.node(sel).is_ok() {
                    return text(format!("{sel} is there after {} ms.", t0.elapsed().as_millis()));
                }
            } else if !js::network_busy() {
                return text(format!("Idle after {} ms.", t0.elapsed().as_millis()));
            }
            if t0.elapsed().as_millis() as u64 >= timeout_ms {
                return failed(format!("Still waiting after {timeout_ms} ms."));
            }
        }
    }

    /// The page as a PNG: the window at the current scroll position, or the whole page (up to
    /// 16,000 px).
    fn png(&mut self, full: bool) -> Result<Vec<u8>, String> {
        let (Some(page), Some(lay)) = (self.page.as_ref(), self.lay.as_ref()) else { return Err("Nothing loaded.".into()) };
        let canvas = if full {
            let h = (lay.height.ceil() as usize).clamp(100, 16000);
            let mut c = Canvas::new(self.width, h);
            c.clear(lay.page_bg);
            c.paint(self.fonts, &page.images, &lay.items, 0.0, 0.0);
            c
        } else {
            let mut c = Canvas::new(self.width, self.height);
            c.clear(lay.page_bg);
            c.paint(self.fonts, &page.images, &lay.items, 0.0, self.scroll);
            c
        };
        encode_png(&canvas)
    }

    /// Ironswift next to Chrome: the likeness score site_check.py gives, and a picture of
    /// where they differ.
    fn compare(&mut self, chrome_png: Option<String>) -> Value {
        let dir = std::env::temp_dir().join(format!("ironswift-mcp-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mine = dir.join("ironswift.png");
        let mut save = |full| self.png(full).map(|b| std::fs::write(&mine, b));
        if let Err(e) = save(true) {
            return failed(e);
        }
        let theirs = match chrome_png {
            Some(p) => std::path::PathBuf::from(p),
            None => {
                let Some(exe) = chrome() else {
                    return failed("No Chrome or Chromium found. Install one, or pass chrome_png (a screenshot of the same page at the same width, such as a recording's chrome.png).");
                };
                let p = dir.join("chrome.png");
                let _ = std::fs::remove_file(&p);
                let _ = std::process::Command::new(exe)
                    .args(["--headless=new", "--disable-gpu", "--no-sandbox", "--hide-scrollbars", &format!("--window-size={},{}", self.width, 1100), &format!("--screenshot={}", p.display()), &self.url()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                if !p.exists() {
                    return failed("Chrome didn't produce a screenshot.");
                }
                p
            }
        };
        let (Some(a), Some(b)) = (read_gray(&mine), read_gray(&theirs)) else { return failed("Couldn't read the screenshots.") };
        let score = likeness(&a, &b, self.width);
        let diff = diff_png(&mine, &theirs, self.width);
        let mut content = vec![json!({"type": "text", "text": format!("Likeness {score:.1} of 100 (85+ looks like Chrome; site_check.py's measure, top 1100 px).\nLeft: Ironswift, right: Chrome, bottom: where they differ.")})];
        if let Ok(d) = diff {
            content.push(json!({"type": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(d)}));
        }
        json!({"content": content})
    }

    /// The lines logged since the page started loading that `keep` picks.
    fn lines(&self, keep: impl Fn(&str) -> bool) -> Vec<String> {
        crate::crashlog::since(self.mark).into_iter().filter(|l| keep(l)).collect()
    }

    /// What the page touched that Ironswift doesn't have: JavaScript globals its scripts name,
    /// CSS properties its stylesheets set, and HTML elements in its document.
    fn unsupported(&mut self) -> String {
        let mut out = String::new();
        // JavaScript: globals the errors name, then well-known Web APIs this engine lacks that
        // the page's scripts mention.
        let errs = self.lines(|l| l.starts_with("[js error]"));
        let mut named: Vec<String> = Vec::new();
        for e in &errs {
            for pat in [" is not defined", " is not a constructor", " is not a function", "not a callable function"] {
                if let Some(i) = e.find(pat) {
                    let word: String = e[..i].chars().rev().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.')).collect::<Vec<_>>().into_iter().rev().collect();
                    if !word.is_empty() && !named.contains(&word) {
                        named.push(word);
                    }
                }
            }
        }
        let sources = self.script_sources();
        let missing = self.js(&format!("JSON.stringify({}.filter(function(n){{return typeof globalThis[n]==='undefined'}}))", serde_json::to_string(WEB_APIS).unwrap_or_default())).unwrap_or_default();
        let missing: Vec<String> = serde_json::from_str(&missing).unwrap_or_default();
        let used: Vec<&String> = missing.iter().filter(|n| sources.iter().any(|s| mentions(s, n))).collect();
        out.push_str("JavaScript\n");
        if named.is_empty() && used.is_empty() {
            out.push_str("  nothing missing found\n");
        }
        for n in &named {
            out.push_str(&format!("  {n} (in a script error)\n"));
        }
        for n in used {
            out.push_str(&format!("  {n} (the page's scripts mention it; not defined here)\n"));
        }
        // CSS: declarations the cascade doesn't understand.
        let mut props: std::collections::BTreeMap<String, usize> = Default::default();
        if let Some(page) = self.page.as_ref() {
            for sheet in &page.sheets {
                for rule in &sheet.rules {
                    for d in &rule.decls {
                        let name = d.name.to_ascii_lowercase();
                        if name.starts_with("--") || d.value.contains("var(") {
                            continue;
                        }
                        if !engine_knows(&name) && !css::supports(&format!("({name}: {})", d.value)) {
                            *props.entry(name).or_default() += 1;
                        }
                    }
                }
            }
        }
        out.push_str("CSS properties (times set)\n");
        if props.is_empty() {
            out.push_str("  nothing unsupported found\n");
        }
        let mut props: Vec<_> = props.into_iter().collect();
        props.sort_by(|a, b| b.1.cmp(&a.1));
        for (p, n) in props.iter().take(60) {
            out.push_str(&format!("  {p} ({n})\n"));
        }
        // HTML: elements that aren't HTML, SVG or MathML and aren't custom elements.
        let tags: Vec<String> = js::with_dom(|d| {
            let mut t: Vec<String> = d.descendants(crate::dom::Dom::ROOT).into_iter().filter_map(|n| d.tag(n).map(str::to_ascii_lowercase)).filter(|t| !t.contains('-') && !KNOWN_TAGS.contains(&t.as_str())).collect();
            t.sort();
            t.dedup();
            t
        });
        out.push_str("HTML elements\n");
        if tags.is_empty() {
            out.push_str("  nothing unknown found\n");
        }
        for t in tags {
            out.push_str(&format!("  <{t}>\n"));
        }
        out
    }

    /// The text of the page's scripts: inline ones, and external ones fetched again (from the
    /// cache or the recording), up to 8 MB in all.
    fn script_sources(&mut self) -> Vec<String> {
        let base = self.url();
        let (inline, srcs): (Vec<String>, Vec<String>) = js::with_dom(|d| {
            let mut inline = Vec::new();
            let mut srcs = Vec::new();
            for n in d.descendants(crate::dom::Dom::ROOT) {
                if d.tag(n) != Some("script") {
                    continue;
                }
                match d.attr(n, "src") {
                    Some(s) if !s.is_empty() => srcs.push(net::resolve(&base, s)),
                    _ => inline.push(d.text_content(n)),
                }
            }
            (inline, srcs)
        });
        let mut out = inline;
        let mut total: usize = out.iter().map(String::len).sum();
        for u in srcs.iter().take(80) {
            if total > 8 << 20 {
                break;
            }
            let req = net::Request { method: "GET", url: u, headers: &[], body: None, top: Some(base.as_str()), cookies: false, extra: &[] };
            if let Ok(r) = net::http(&req) {
                total += r.body.len();
                out.push(String::from_utf8_lossy(&r.body).into_owned());
            }
        }
        out
    }
}

trait OrEmpty {
    fn or_empty(self, msg: &str) -> Value;
}

impl OrEmpty for Value {
    /// A text result with nothing in it says so instead.
    fn or_empty(self, msg: &str) -> Value {
        if self["content"][0]["text"].as_str().is_some_and(str::is_empty) {
            text(msg)
        } else {
            self
        }
    }
}

/// The style code mentions the property by name (as a string): the cascade handles it.
fn engine_knows(prop: &str) -> bool {
    const SRC: [&str; 6] = [include_str!("style.rs"), include_str!("css.rs"), include_str!("cascade.rs"), include_str!("svg_inline.rs"), include_str!("anim.rs"), include_str!("pseudo.rs")];
    let quoted = format!("\"{prop}\"");
    SRC.iter().any(|s| s.contains(&quoted))
}

fn is_failed_request(l: &str) -> bool {
    l.starts_with("[blocked]") || l.contains("couldn't load") || l.starts_with("[js warn] fetch failed") || l.contains("not in the recording") && l.starts_with("[net]") || l.starts_with("[net] HTTP")
}

/// Does script text mention `name` as a word (not part of a longer identifier)?
fn mentions(src: &str, name: &str) -> bool {
    let b = src.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    src.match_indices(name).any(|(i, _)| {
        let before = i.checked_sub(1).map(|j| b[j]);
        let after = b.get(i + name.len()).copied();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… ({} more characters; pass a selector or a larger max_chars)", &s[..end], s.len() - end)
}

fn image_result(png: &[u8], caption: &str) -> Value {
    json!({"content": [
        {"type": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(png)},
        {"type": "text", "text": caption}
    ]})
}

fn encode_png(c: &Canvas) -> Result<Vec<u8>, String> {
    let rgb: Vec<u8> = c.buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    rgb_png(c.w as u32, c.h as u32, &rgb)
}

fn rgb_png(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(rgb)).map_err(|e| e.to_string())?;
    Ok(out)
}

fn chrome() -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    for name in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"] {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p.display().to_string());
            }
        }
    }
    None
}

/// A screenshot in gray levels: (width, height, pixels).
fn read_gray(path: &std::path::Path) -> Option<(usize, usize, Vec<u8>)> {
    let img = image::open(path).ok()?.to_luma8();
    Some((img.width() as usize, img.height() as usize, img.into_raw()))
}

/// Compared over the top 1100 px.
const COMPARE_H: usize = 1100;

/// site_check.py's likeness: both shots cut to the top 1100 px on a white page `width` wide,
/// shrunk 16 times (box average; PIL's bilinear filter is close), then the mean difference.
fn likeness(a: &(usize, usize, Vec<u8>), b: &(usize, usize, Vec<u8>), width: usize) -> f64 {
    let h = COMPARE_H.min(a.1.max(b.1));
    let small = |im: &(usize, usize, Vec<u8>)| -> Vec<f64> {
        let (sw, sh) = (width / 16, (h / 16).max(1));
        let mut out = vec![0.0; sw * sh];
        for (sy, row) in out.chunks_mut(sw).enumerate() {
            for (sx, v) in row.iter_mut().enumerate() {
                let mut sum = 0u32;
                for y in sy * 16..sy * 16 + 16 {
                    for x in sx * 16..sx * 16 + 16 {
                        sum += if x < im.0 && y < im.1 { im.2[y * im.0 + x] as u32 } else { 255 };
                    }
                }
                *v = sum as f64 / 256.0;
            }
        }
        out
    };
    let (sa, sb) = (small(a), small(b));
    let mean = sa.iter().zip(&sb).map(|(x, y)| (x - y).abs()).sum::<f64>() / sa.len().max(1) as f64;
    ((100.0 - mean / 255.0 * 100.0 * 2.5) * 10.0).round() / 10.0
}

/// Ironswift and Chrome side by side over their top 1100 px, with a third panel underneath
/// marking where they differ (red), at half size.
fn diff_png(mine: &std::path::Path, theirs: &std::path::Path, width: usize) -> Result<Vec<u8>, String> {
    let a = image::open(mine).map_err(|e| e.to_string())?.to_rgb8();
    let b = image::open(theirs).map_err(|e| e.to_string())?.to_rgb8();
    let (w, h) = (width, COMPARE_H);
    let px = |im: &image::RgbImage, x: usize, y: usize| -> [u8; 3] {
        if (x as u32) < im.width() && (y as u32) < im.height() { im.get_pixel(x as u32, y as u32).0 } else { [255, 255, 255] }
    };
    // Half size: left Ironswift, right Chrome; below, the difference over Ironswift's shot.
    let (ow, oh) = (w, h);
    let mut out = vec![255u8; ow * oh * 3];
    let half_h = h / 2;
    for y in 0..half_h {
        for x in 0..w / 2 {
            let (p, q) = (px(&a, x * 2, y * 2), px(&b, x * 2, y * 2));
            let o = (y * ow + x) * 3;
            out[o..o + 3].copy_from_slice(&p);
            let o = (y * ow + w / 2 + x) * 3;
            out[o..o + 3].copy_from_slice(&q);
        }
    }
    for y in 0..half_h.min(oh - half_h) {
        for x in 0..w / 2 {
            let (p, q) = (px(&a, x * 2, y * 2), px(&b, x * 2, y * 2));
            let d: u32 = p.iter().zip(&q).map(|(x, y)| x.abs_diff(*y) as u32).sum();
            let g = ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3 / 2 + 127) as u8;
            let c = if d > 60 { [220, 30, 30] } else { [g, g, g] };
            let o = ((half_h + y) * ow + w / 4 + x) * 3;
            out[o..o + 3].copy_from_slice(&c);
        }
    }
    rgb_png(ow as u32, oh as u32, &out)
}

/// Run one of the repo's tools (site_check.py, wpt.py) with this binary and return its output.
fn tool_script(name: &str, args: &[String]) -> Value {
    let Some(root) = repo_root() else {
        return failed(format!("Can't find tools/{name}: run ironswift from the repository (target/release/ironswift) or set IRONSWIFT_ROOT."));
    };
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let out = std::process::Command::new("python3")
        .arg(root.join("tools").join(name))
        .args(args)
        .env("IRONSWIFT_BIN", &exe)
        .current_dir(&root)
        .stdin(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) => {
            let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
            let err = String::from_utf8_lossy(&o.stderr);
            if !o.status.success() {
                t.push_str(&format!("\n(exit {}){}", o.status, if err.trim().is_empty() { String::new() } else { format!("\n{}", cut(&err, 4000)) }));
            }
            text(cut(&t, 60_000))
        }
        Err(e) => failed(format!("Couldn't run python3: {e}")),
    }
}

fn repo_root() -> Option<std::path::PathBuf> {
    if let Ok(r) = std::env::var("IRONSWIFT_ROOT") {
        return Some(r.into());
    }
    let mut starts = Vec::new();
    if let Ok(e) = std::env::current_exe() {
        starts.push(e);
    }
    if let Ok(d) = std::env::current_dir() {
        starts.push(d);
    }
    starts.into_iter().find_map(|s| s.ancestors().find(|a| a.join("tools/site_check.py").is_file()).map(std::path::Path::to_path_buf))
}

/// The tools, with their arguments.
fn tools() -> Value {
    let sel = json!({"type": "string", "description": "A CSS selector (the first match is used)"});
    let tool = |name: &str, desc: &str, props: Value, required: &[&str]| json!({"name": name, "description": desc, "inputSchema": {"type": "object", "properties": props, "required": required}});
    json!([
        tool("open_url", "Load a page (scripts and requests get up to 5 s to settle) in a fresh headless page and return a summary: title, size, script errors, failed requests.",
             json!({"url": {"type": "string"}, "width": {"type": "number", "description": "Window width in CSS px (default 1280)"}, "height": {"type": "number", "description": "Window height (default 800)"}}), &["url"]),
        tool("click", "Click an element: its click event, then the default action (follow a link, tick a box, submit a form).", json!({"selector": sel}), &["selector"]),
        tool("type", "Type text into a field (appends to its value, fires input and change).", json!({"selector": sel, "text": {"type": "string"}}), &["selector", "text"]),
        tool("scroll", "Scroll the window: to y, by a distance, or (with neither) down by most of a screen.", json!({"y": {"type": "number"}, "by": {"type": "number"}}), &[]),
        tool("wait_for", "Wait until an element matching selector exists, or (without one) until the network is idle.", json!({"selector": sel, "timeout_ms": {"type": "number"}}), &[]),
        tool("screenshot", "A PNG of the window at the current scroll position, or of the whole page.", json!({"full_page": {"type": "boolean"}}), &[]),
        tool("compare_with_chrome", "Ironswift next to Chrome: the 0-100 likeness score from tools/site_check.py and a picture marking the differences. Runs headless Chrome if installed, or takes chrome_png.",
             json!({"chrome_png": {"type": "string", "description": "Path to Chrome's screenshot of the same page at the same width (such as a site recording's chrome.png)"}}), &[]),
        tool("script_errors", "The page's script errors, with where they were thrown.", json!({}), &[]),
        tool("console_log", "What the page wrote to the console.", json!({}), &[]),
        tool("failed_requests", "Requests that failed: blocked by content blocking, network errors, HTTP errors, or not in a recording.", json!({}), &[]),
        tool("dom", "The document as HTML, or one element's outerHTML.", json!({"selector": sel, "max_chars": {"type": "number"}}), &[]),
        tool("computed_style", "An element's computed style: the properties named, or all.", json!({"selector": sel, "properties": {"type": "array", "items": {"type": "string"}}}), &["selector"]),
        tool("box", "Layout boxes (position and size in page px, display, position) of up to 20 elements matching selector.", json!({"selector": sel}), &["selector"]),
        tool("run_js", "Evaluate JavaScript in the page and return the value (strings as they are, anything else as JSON).", json!({"code": {"type": "string"}}), &["code"]),
        tool("unsupported_features", "What the page uses that Ironswift lacks: JavaScript globals (from errors and from its scripts' text), CSS properties its stylesheets set, unknown HTML elements.", json!({}), &[]),
        tool("run_site_check", "Run tools/site_check.py (all tracked sites, or the URLs given): grades, likeness and script errors per site. Needs network access; slow (a minute or more per site).",
             json!({"urls": {"type": "array", "items": {"type": "string"}}}), &[]),
        tool("run_wpt", "Run tools/wpt.py on one web-platform-tests directory: pass/fail counts and the failing tests.", json!({"wpt_root": {"type": "string"}, "dir": {"type": "string", "description": "Such as css/css-flexbox"}}), &["wpt_root", "dir"]),
    ])
}

/// Web APIs pages commonly use: any missing here that a page's scripts name is reported.
const WEB_APIS: &[&str] = &[
    "AbortController", "AbortSignal", "AudioContext", "BroadcastChannel", "Blob", "CSS", "CSSStyleSheet", "CanvasRenderingContext2D", "Cache", "CacheStorage", "ClipboardItem",
    "CompressionStream", "CustomEvent", "DOMParser", "DOMRect", "DecompressionStream", "DocumentFragment", "ElementInternals", "EventSource", "FileReader", "FontFace", "FormData",
    "Headers", "HTMLDialogElement", "IDBFactory", "IntersectionObserver", "IntersectionObserverEntry", "Intl", "MediaQueryList", "MediaSource", "MessageChannel", "MutationObserver",
    "Notification", "OffscreenCanvas", "Path2D", "PerformanceObserver", "PointerEvent", "PromiseRejectionEvent", "ReadableStream", "Request", "ResizeObserver", "Response",
    "ServiceWorker", "ServiceWorkerContainer", "SharedArrayBuffer", "SharedWorker", "TextDecoder", "TextEncoder", "TextDecoderStream", "TransformStream", "TrustedTypes", "URL",
    "URLPattern", "URLSearchParams", "VisualViewport", "WebAssembly", "WebSocket", "WebTransport", "Worker", "WritableStream", "XMLHttpRequest", "XPathEvaluator", "XPathResult",
    "crypto", "customElements", "fetch", "indexedDB", "localStorage", "matchMedia", "navigation", "queueMicrotask", "requestIdleCallback", "scheduler", "sessionStorage",
    "structuredClone", "trustedTypes", "visualViewport", "webkitURL", "RTCPeerConnection", "PaymentRequest", "PublicKeyCredential", "CredentialsContainer", "Gamepad",
    "SpeechSynthesisUtterance", "speechSynthesis", "BarcodeDetector", "EyeDropper", "Highlight", "CSSHighlights", "ViewTransition", "CloseWatcher", "CookieStore", "cookieStore",
    "LockManager", "StorageManager", "WakeLock", "Bluetooth", "USB", "HID", "Serial", "XRSystem", "GPU",
];

/// HTML, SVG and MathML element names: anything else that isn't a custom element (no hyphen)
/// is unknown to browsers too, or to Ironswift.
const KNOWN_TAGS: &[&str] = &[
    "html", "head", "body", "title", "base", "link", "meta", "style", "script", "noscript", "template", "slot", "article", "section", "nav", "aside", "h1", "h2", "h3", "h4", "h5",
    "h6", "hgroup", "header", "footer", "address", "p", "hr", "pre", "blockquote", "ol", "ul", "menu", "li", "dl", "dt", "dd", "figure", "figcaption", "main", "search", "div", "a",
    "em", "strong", "small", "s", "cite", "q", "dfn", "abbr", "ruby", "rt", "rp", "data", "time", "code", "var", "samp", "kbd", "sub", "sup", "i", "b", "u", "mark", "bdi", "bdo",
    "span", "br", "wbr", "ins", "del", "picture", "source", "img", "iframe", "embed", "object", "param", "video", "audio", "track", "map", "area", "table", "caption", "colgroup",
    "col", "tbody", "thead", "tfoot", "tr", "td", "th", "form", "label", "input", "button", "select", "datalist", "optgroup", "option", "textarea", "output", "progress", "meter",
    "fieldset", "legend", "details", "summary", "dialog", "canvas", "center", "font", "big", "strike", "tt", "nobr", "marquee", "frameset", "frame", "noframes", "acronym",
    "applet", "basefont", "dir", "listing", "xmp", "plaintext", "image", "selectedcontent",
    // SVG
    "svg", "g", "defs", "symbol", "use", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "text", "tspan", "textpath", "lineargradient", "radialgradient", "stop",
    "pattern", "clippath", "mask", "filter", "fegaussianblur", "feoffset", "feblend", "fecolormatrix", "fecomposite", "feflood", "femerge", "femergenode", "femorphology",
    "fedropshadow", "feturbulence", "fedisplacementmap", "fecomponenttransfer", "fefunca", "fefuncr", "fefuncg", "fefuncb", "feimage", "fetile", "fespecularlighting",
    "fediffuselighting", "fepointlight", "fedistantlight", "fespotlight", "feconvolvematrix", "foreignobject", "marker", "desc", "metadata", "animate", "animatetransform",
    "animatemotion", "set", "mpath", "view", "switch",
    // MathML
    "math", "mi", "mn", "mo", "ms", "mtext", "mrow", "mfrac", "msqrt", "mroot", "msub", "msup", "msubsup", "munder", "mover", "munderover", "mtable", "mtr", "mtd", "mspace",
    "mstyle", "mpadded", "mphantom", "semantics", "annotation", "annotation-xml", "menclose", "merror", "mmultiscripts", "mprescripts", "none",
];
