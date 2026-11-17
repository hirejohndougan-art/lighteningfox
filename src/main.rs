//! LighteningFox — a web browser engine written from scratch in Rust.
//!
//! Pipeline: fetch -> HTML parse (DOM) -> CSS parse -> style cascade -> layout -> paint.
//!
//! Usage:
//!   lighteningfox [URL or file]                  open a window
//!   lighteningfox URL --png out.png [--width N]  render to an image (no window needed)

mod cookies;
mod css;
mod dom;
mod font;
mod forms;
mod grid;
mod html;
mod images;
mod js;
mod layout;
mod net;
mod paint;
mod style;

use font::Fonts;
use minifb::{CursorStyle, Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use paint::Canvas;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Instant;
use std::rc::Rc;
use style::Color;

const BAR_H: usize = 36;
/// Toolbar buttons: (x, label). Each is BTN_W wide.
const BTN_W: f32 = 30.0;
const BUTTONS: [(f32, &str); 3] = [(6.0, "←"), (38.0, "→"), (70.0, "↻")];
const FIELD_X: f32 = 108.0;

/// Where a navigation should go and how it affects history.
enum Go {
    To(String),
    /// Submit a form with POST: (url, urlencoded body)
    Post(String, String),
    Back,
    Forward,
    Reload,
}

/// Which toolbar button (0 back, 1 forward, 2 reload) is under (x, y), if any.
fn button_at(x: f32, y: f32) -> Option<usize> {
    if y >= BAR_H as f32 {
        return None;
    }
    BUTTONS.iter().position(|(bx, _)| x >= *bx && x < bx + BTN_W)
}
const START_PAGE: &str = include_str!("start.html");

struct Page {
    url: String,
    title: String,
    sheets: Vec<css::Stylesheet>,
    styles: Vec<Rc<style::Style>>,
    images: images::Images,
    /// The JavaScript engine, started only for pages that have scripts.
    js: Option<js::Js>,
    /// The page has :hover rules, so mouse moves can change its styles.
    hover_rules: bool,
    /// Each form control's value as the page loaded, for form.reset().
    form_defaults: Vec<(dom::NodeId, Option<String>, bool)>,
}

impl Page {
    /// Re-run the cascade after the DOM changed.
    fn restyle(&mut self) {
        self.styles = js::with_dom(|d| style::compute(d, &self.sheets));
        let bg_urls = background_urls(&self.styles);
        let (images, url) = (&mut self.images, &self.url);
        js::with_dom(|d| images.load_missing(d, url, &bg_urls));
        self.title = js::with_dom(|d| d.find_all("title").first().map(|&t| d.text_content(t).trim().to_string()).unwrap_or_default());
    }

    fn layout(&self, fonts: &Fonts, width: f32, viewport_h: f32) -> layout::Layout {
        js::with_dom(|d| layout::layout(d, &self.styles, fonts, &self.images, width, viewport_h))
    }
}

/// Everything a page needs from the network, gathered off the UI thread.
struct Prepared {
    url: String,
    dom: dom::Dom,
    sheets: Vec<css::Stylesheet>,
    images: images::Images,
    scripts: HashMap<String, String>,
}

/// Fetch the page, its stylesheets, scripts and images, all in parallel where possible.
/// Runs on a worker thread; touches nothing the UI thread owns.
fn fetch_page(url: &str, post: Option<&str>) -> Prepared {
    let (final_url, body) = if url == "about:home" || url.is_empty() {
        ("about:home".to_string(), START_PAGE.to_string())
    } else {
        let got = match post {
            Some(body) => net::post_form(url, body),
            None => net::fetch(&net::strip_tracking(url)),
        };
        match got {
            Ok(r) => r,
            Err(e) => (url.to_string(), error_page(url, &e)),
        }
    };
    let dom = html::parse(&body);

    // Stylesheets: inline ones in place, external ones fetched in parallel, order kept.
    enum Src {
        Inline(String),
        External(String),
    }
    let mut sources = Vec::new();
    let mut script_urls = Vec::new();
    for id in dom.descendants(dom::Dom::ROOT) {
        match dom.tag(id) {
            Some("style") => sources.push(Src::Inline(dom.text_content(id))),
            Some("link") => {
                let rel = dom.attr(id, "rel").unwrap_or("").to_ascii_lowercase();
                if let (true, Some(href)) = (rel.split_whitespace().any(|r| r == "stylesheet"), dom.attr(id, "href")) {
                    let media = dom.attr(id, "media").unwrap_or("all");
                    if !(media.contains("print") && !media.contains("screen")) {
                        sources.push(Src::External(net::resolve(&final_url, href)));
                    }
                }
            }
            Some("script") => {
                if let Some(src) = dom.attr(id, "src") {
                    script_urls.push(net::resolve(&final_url, src));
                }
            }
            _ => {}
        }
    }
    let page_url: &str = &final_url;
    let (sheets, scripts) = std::thread::scope(|scope| {
        let sheet_jobs: Vec<_> = sources
            .iter()
            .map(|src| match src {
                Src::Inline(text) => {
                    let base = final_url.clone();
                    scope.spawn(move || {
                        let mut sheet = css::parse(text);
                        css::absolutize_urls(&mut sheet, &base);
                        Some(sheet)
                    })
                }
                Src::External(u) => scope.spawn(move || {
                    let (_, text) = net::fetch_sub(u, page_url).ok()?;
                    let mut sheet = css::parse(&text);
                    css::absolutize_urls(&mut sheet, u);
                    Some(sheet)
                }),
            })
            .collect();
        let script_jobs: Vec<_> =
            script_urls.iter().map(|u| scope.spawn(move || net::fetch_sub(u, page_url).ok().map(|(_, b)| (u.clone(), b)))).collect();
        let sheets: Vec<css::Stylesheet> = sheet_jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect();
        let scripts: HashMap<String, String> = script_jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect();
        (sheets, scripts)
    });

    // Images: <img> sources plus CSS background images, fetched in parallel.
    let mut images = images::Images::default();
    let styles = style::compute(&dom, &sheets);
    let bg_urls = background_urls(&styles);
    images.load_missing(&dom, &final_url, &bg_urls);
    Prepared { url: final_url, dom, sheets, images, scripts }
}

fn background_urls(styles: &[Rc<style::Style>]) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for st in styles {
        if let Some(style::BgImage::Url(u)) = &st.bg.image {
            if !urls.contains(u) {
                urls.push(u.clone());
            }
        }
    }
    urls
}

/// Turn fetched resources into a live page: install the DOM, run scripts, style it.
/// Runs on the UI thread (JavaScript and the DOM live there).
fn build_page(p: Prepared) -> Page {
    let Prepared { url, dom, sheets, images, scripts } = p;
    // Start JavaScript only if the page has scripts or inline event handlers:
    // the engine costs several megabytes even when it has nothing to run.
    let has_scripts = dom.descendants(dom::Dom::ROOT).iter().any(|&n| match &dom.nodes[n].data {
        dom::NodeData::Element { tag, attrs } => tag == "script" || attrs.iter().any(|(k, _)| k.starts_with("on")),
        _ => false,
    });
    js::set_dom(dom);
    let js = has_scripts.then(|| {
        let mut js = js::Js::new(&url);
        js.run_scripts(&url, &scripts);
        js
    });
    js::take_dirty();
    style::set_hover(None);
    let hover_rules = style::uses_hover(&sheets);
    let form_defaults = js::with_dom(forms::defaults);
    let mut page = Page { url, title: String::new(), sheets, styles: Vec::new(), images, js, hover_rules, form_defaults };
    page.restyle();
    release_free_memory();
    page
}

/// Hand memory freed by the previous page back to the operating system.
/// (glibc otherwise keeps it reserved for the process.)
fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        malloc_trim(0);
    }
}

fn load(url: &str) -> Page {
    build_page(fetch_page(url, None))
}

fn error_page(url: &str, err: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;");
    format!(
        "<html><body style='font-family:sans-serif;margin:60px;color:#333'><h1>Couldn't load this page</h1>\
         <p><b>{}</b></p><p style='color:#a00'>{}</p></body></html>",
        esc(url),
        esc(err)
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut url = String::from("about:home");
    let mut png: Option<String> = None;
    let mut width = 1024usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--png" => {
                png = args.get(i + 1).cloned();
                i += 1;
            }
            "--width" => {
                width = args.get(i + 1).and_then(|w| w.parse().ok()).unwrap_or(width);
                i += 1;
            }
            other => url = net::normalize_input(other),
        }
        i += 1;
    }
    let rss = || std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 4 / 1024;
    let probe = std::env::var("LF_MEM_PROBE").is_ok();
    if probe {
        eprintln!("[mem] start: {} MB", rss());
    }
    let fonts = match Fonts::load() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if probe {
        eprintln!("[mem] fonts loaded: {} MB", rss());
        let js = js::Js::new("about:blank");
        eprintln!("[mem] + one JS engine: {} MB", rss());
        drop(js);
        let page = load(&url);
        eprintln!("[mem] + page loaded: {} MB", rss());
        let lay = page.layout(&fonts, width as f32, 768.0);
        eprintln!("[mem] + layout ({} items): {} MB", lay.items.len(), rss());
        return;
    }
    match png {
        Some(out) => render_png(&fonts, &url, width, &out),
        None => run_window(&fonts, &url, width),
    }
}

/// Load a page and paint the whole of it (up to 16,000 px tall) into a canvas.
fn render_page(fonts: &Fonts, url: &str, width: usize) -> (Page, layout::Layout, Canvas) {
    let mut page = load(url);
    // Give fetch() calls and short timers a moment to fill the page in before the snapshot.
    if let Some(js) = page.js.as_mut() {
        js.settle(std::time::Duration::from_secs(5));
        if js::take_dirty() {
            page.restyle();
        }
    }
    let lay = page.layout(fonts, width as f32, 768.0);
    let height = (lay.height.ceil() as usize).clamp(100, 16000);
    let mut canvas = Canvas::new(width, height);
    canvas.clear(lay.page_bg);
    canvas.paint(fonts, &page.images, &lay.items, 0.0, 0.0);
    (page, lay, canvas)
}

fn save_png(canvas: &Canvas, out: &str) -> std::io::Result<()> {
    let file = std::fs::File::create(out)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), canvas.w as u32, canvas.h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = canvas.buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    enc.write_header()?.write_image_data(&data).map_err(std::io::Error::other)
}

fn render_png(fonts: &Fonts, url: &str, width: usize, out: &str) {
    let t = std::time::Instant::now();
    let (page, lay, canvas) = render_page(fonts, url, width);
    let height = canvas.h;
    save_png(&canvas, out).expect("write png");
    let st = page.js.as_ref().map(|j| j.stats.clone()).unwrap_or_default();
    let st = &st;
    eprintln!(
        "{}: {} nodes, {} display items, {}x{} px, {:?} total | JS: {} scripts, {} errors, {:?}",
        page.url,
        js::with_dom(|d| d.nodes.len()),
        lay.items.len(),
        width,
        height,
        t.elapsed(),
        st.scripts,
        st.errors,
        st.script_time
    );
}

/// Collects typed characters for the address bar.
struct Typed(Rc<RefCell<Vec<char>>>);
impl minifb::InputCallback for Typed {
    fn add_char(&mut self, c: u32) {
        if let Some(ch) = char::from_u32(c) {
            self.0.borrow_mut().push(ch);
        }
    }
}

/// An open <select> list, drawn over the page.
struct Dropdown {
    select: dom::NodeId,
    /// Top-left in page coordinates, and width.
    x: f32,
    y: f32,
    w: f32,
    options: Vec<(dom::NodeId, String)>,
    hover: Option<usize>,
}
const OPTION_H: f32 = 22.0;

impl Dropdown {
    fn row_at(&self, mx: f32, page_y: f32) -> Option<usize> {
        if mx < self.x || mx > self.x + self.w || page_y < self.y {
            return None;
        }
        let i = ((page_y - self.y) / OPTION_H) as usize;
        (i < self.options.len()).then_some(i)
    }
}

/// The form control a click on `node` acts on: the control itself, the control
/// it's inside (text in a <button>), or the control a <label> points at.
fn control_for(d: &dom::Dom, node: dom::NodeId) -> Option<dom::NodeId> {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if forms::control(d, n).is_some() {
            return Some(n);
        }
        if d.tag(n) == Some("label") {
            return forms::label_target(d, n);
        }
        if d.tag(n) == Some("a") {
            return None;
        }
        cur = d.nodes[n].parent;
    }
    None
}

/// Every control Tab can move to, in document order.
fn focusable(d: &dom::Dom) -> Vec<dom::NodeId> {
    d.descendants(dom::Dom::ROOT).into_iter().filter(|&n| forms::control(d, n).is_some() && !forms::is_disabled(d, n)).collect()
}

/// Fire an event at a node if the page runs scripts. Returns false if a script cancelled it.
fn fire(page: &mut Page, node: dom::NodeId, kind: &str) -> bool {
    match page.js.as_mut() {
        Some(js) => js.dispatch(node, kind),
        None => true,
    }
}

/// Submit a form. With `fire_event`, the page's submit handlers run first and may cancel it.
fn submit_form(page: &mut Page, form: dom::NodeId, submitter: Option<dom::NodeId>, fire_event: bool) -> Option<Go> {
    if fire_event && !fire(page, form, "submit") {
        return None;
    }
    let sub = js::with_dom(|d| forms::submission(d, form, submitter, &page.url));
    if sub.url.starts_with("javascript:") {
        return None;
    }
    Some(match sub.post {
        Some(body) => Go::Post(sub.url, body),
        None => Go::To(sub.url),
    })
}

fn run_window(fonts: &Fonts, start: &str, width: usize) {
    let mut window = Window::new("LighteningFox", width, 768, WindowOptions { resize: true, ..WindowOptions::default() })
        .unwrap_or_else(|e| {
            eprintln!("Could not open a window: {e}");
            std::process::exit(1);
        });
    window.set_target_fps(60);
    let mut idle_frames = 0u32;
    let mut slow = false;
    let typed = Rc::new(RefCell::new(Vec::new()));
    window.set_input_callback(Box::new(Typed(typed.clone())));

    let mut back: Vec<String> = Vec::new();
    let mut forward: Vec<String> = Vec::new();
    let mut hover: Option<usize> = None;
    let mut hovered: Option<dom::NodeId> = None;
    // Pages load on worker threads; results come back tagged with a generation number
    // so a stopped or superseded load is ignored when it arrives.
    let (tx, rx) = mpsc::channel::<(u64, Prepared)>();
    let mut generation = 0u64;
    let mut loading: Option<Loading> = None;
    let mut last_spin = Instant::now();
    let mut page = load(start);
    let (mut w, mut h) = window.get_size();
    let mut lay = page.layout(fonts, w as f32, h.saturating_sub(BAR_H) as f32);
    let mut canvas = Canvas::new(w, h);
    let mut scroll = 0.0f32;
    let mut editing = false;
    let mut address = page.url.clone();
    let mut dirty = true;
    let mut was_down = false;
    // The focused form control (typing goes into it) and an open <select> list.
    let mut focused: Option<dom::NodeId> = None;
    let mut dropdown: Option<Dropdown> = None;
    window.set_title(&format!("{} — LighteningFox", if page.title.is_empty() { &page.url } else { &page.title }));

    while window.is_open() {
        let mut navigate: Option<Go> = None;
        let mut reflow = false;
        let mut stop_key = false;

        // Resize -> relayout.
        let (nw, nh) = window.get_size();
        if (nw, nh) != (w, h) && nw > 0 && nh > 0 {
            lay = page.layout(fonts, nw as f32, nh.saturating_sub(BAR_H) as f32);
            (w, h) = (nw, nh);
            canvas = Canvas::new(w, h);
            dirty = true;
        }
        // JavaScript: run due timers; restyle and reflow if the DOM changed.
        if let Some(js) = page.js.as_mut() {
            js.tick();
        }
        if js::take_dirty() {
            page.restyle();
            lay = page.layout(fonts, w as f32, h.saturating_sub(BAR_H) as f32);
            dirty = true;
        }
        if let Some(target) = js::take_navigation() {
            navigate = Some(Go::To(net::resolve(&page.url, &target)));
        }
        let mut form_changed = false;
        // Requests from scripts: focus(), blur(), form.submit(), form.reset().
        for req in js::take_requests() {
            match req {
                js::Request::Focus(n) if js::with_dom(|d| forms::control(d, n).is_some()) => {
                    focused = Some(n);
                    form_changed = true;
                }
                js::Request::Blur(n) if focused == Some(n) => {
                    focused = None;
                    form_changed = true;
                }
                js::Request::Submit(f, by, fire_event) => {
                    if let Some(go) = submit_form(&mut page, f, by, fire_event) {
                        navigate = Some(go);
                    }
                }
                js::Request::Reset(f) => {
                    let defaults = page.form_defaults.clone();
                    js::with_dom_mut(|d| forms::reset(d, f, &defaults));
                    form_changed = true;
                }
                _ => {}
            }
        }
        let view_h = h.saturating_sub(BAR_H) as f32;
        let max_scroll = (lay.height - view_h).max(0.0);

        // Typing in the address bar.
        let chars: Vec<char> = typed.borrow_mut().drain(..).collect();
        let typing = !editing && focused.is_some_and(|f| js::with_dom(|d| forms::is_editable(d, f)));
        if typing {
            let f = focused.unwrap();
            let multiline = js::with_dom(|d| d.tag(f) == Some("textarea"));
            let mut value = js::with_dom(|d| forms::value(d, f));
            let before = value.clone();
            let mut enter = false;
            for &c in &chars {
                match c {
                    '\u{8}' => {
                        value.pop();
                    }
                    '\r' | '\n' if multiline => value.push('\n'),
                    '\r' | '\n' => enter = true,
                    c if !c.is_control() && c != '\u{7f}' => value.push(c),
                    _ => {}
                }
            }
            // A maxlength limit, counted in characters.
            if let Some(max) = js::with_dom(|d| d.attr(f, "maxlength").and_then(|m| m.trim().parse::<usize>().ok())) {
                if value.chars().count() > max && value.len() > before.len() {
                    value = value.chars().take(max.max(before.chars().count())).collect();
                }
            }
            if value != before {
                js::with_dom_mut(|d| forms::set_value(d, f, &value));
                fire(&mut page, f, "input");
                form_changed = true;
            }
            if enter {
                // Pressing Enter in a field submits its form (through its default button).
                fire(&mut page, f, "change");
                if let Some(form) = js::with_dom(|d| forms::form_of(d, f)) {
                    let by = js::with_dom(|d| forms::default_button(d, form));
                    let go_on = by.map_or(true, |b| fire(&mut page, b, "click"));
                    if go_on {
                        if let Some(go) = submit_form(&mut page, form, by, true) {
                            navigate = Some(go);
                        }
                    }
                }
            }
        }
        if editing {
            for c in chars {
                match c {
                    '\u{8}' => {
                        address.pop();
                    }
                    '\r' | '\n' => {
                        editing = false;
                        navigate = Some(Go::To(net::normalize_input(&address)));
                    }
                    '\u{1b}' => {
                        editing = false;
                        address = page.url.clone();
                    }
                    c if !c.is_control() && c != '\u{7f}' => address.push(c),
                    _ => {}
                }
                dirty = true;
            }
            if window.is_key_pressed(Key::Escape, KeyRepeat::No) {
                editing = false;
                address = page.url.clone();
                dirty = true;
            }
        } else {
            let ctrl = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
            stop_key = window.is_key_pressed(Key::Escape, KeyRepeat::No);
            let alt = window.is_key_down(Key::LeftAlt) || window.is_key_down(Key::RightAlt);
            for key in window.get_keys_pressed(KeyRepeat::Yes) {
                let before = scroll;
                // Keys for the focused control come first.
                if let Some(f) = focused {
                    let kind = js::with_dom(|d| forms::control(d, f));
                    let handled = match key {
                        Key::Tab => {
                            let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
                            let all = js::with_dom(focusable);
                            let i = all.iter().position(|&n| n == f).unwrap_or(0);
                            let next = if shift { (i + all.len() - 1) % all.len().max(1) } else { (i + 1) % all.len().max(1) };
                            if let Some(&n) = all.get(next) {
                                fire(&mut page, f, "blur");
                                focused = Some(n);
                                fire(&mut page, n, "focus");
                            }
                            dropdown = None;
                            form_changed = true;
                            true
                        }
                        Key::Escape => {
                            fire(&mut page, f, "blur");
                            focused = None;
                            dropdown = None;
                            form_changed = true;
                            true
                        }
                        Key::Up | Key::Down if kind == Some(forms::Control::Select) => {
                            let (opts, cur) = js::with_dom(|d| (forms::options(d, f), forms::selected_option(d, f)));
                            let i = cur.and_then(|c| opts.iter().position(|&o| o == c)).unwrap_or(0);
                            let j = if key == Key::Up { i.saturating_sub(1) } else { (i + 1).min(opts.len().saturating_sub(1)) };
                            if let Some(&o) = opts.get(j).filter(|_| j != i) {
                                js::with_dom_mut(|d| forms::select_option(d, f, o));
                                fire(&mut page, f, "input");
                                fire(&mut page, f, "change");
                                form_changed = true;
                            }
                            true
                        }
                        Key::Space if matches!(kind, Some(forms::Control::Checkbox | forms::Control::Radio)) => {
                            js::with_dom_mut(|d| forms::toggle(d, f));
                            fire(&mut page, f, "input");
                            fire(&mut page, f, "change");
                            form_changed = true;
                            true
                        }
                        // Typing keys belong to the text field, not page scrolling.
                        _ => typing && !matches!(key, Key::F5 | Key::F6) && !ctrl && !alt,
                    };
                    if handled {
                        continue;
                    }
                }
                match key {
                    Key::Down => scroll += 48.0,
                    Key::Up => scroll -= 48.0,
                    Key::PageDown | Key::Space => scroll += view_h * 0.9,
                    Key::PageUp => scroll -= view_h * 0.9,
                    Key::Home => scroll = 0.0,
                    Key::End => scroll = max_scroll,
                    Key::L if ctrl => {
                        editing = true;
                        address.clear();
                        dirty = true;
                    }
                    Key::F6 => {
                        editing = true;
                        address.clear();
                        dirty = true;
                    }
                    Key::F5 | Key::R if key == Key::F5 || ctrl => navigate = Some(Go::Reload),
                    Key::Backspace => navigate = Some(Go::Back),
                    Key::Left if alt => navigate = Some(Go::Back),
                    Key::Right if alt => navigate = Some(Go::Forward),
                    _ => {}
                }
                if scroll != before {
                    dirty = true;
                }
            }
        }
        if let Some((_, dy)) = window.get_scroll_wheel() {
            scroll -= dy * 3.0;
            dirty = true;
        }
        scroll = scroll.clamp(0.0, max_scroll);

        // Mouse: hover cursor and clicks.
        let down = window.get_mouse_down(MouseButton::Left);
        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) {
            let view_y = my - BAR_H as f32;
            let hit = if my >= BAR_H as f32 { lay.link_at(mx, view_y, scroll).cloned() } else { None };
            let btn = button_at(mx, my);
            // :hover — restyle when the element under the mouse changes.
            if page.hover_rules {
                let over = if my >= BAR_H as f32 { lay.hit_at(mx, view_y, scroll) } else { None };
                if over != hovered {
                    hovered = over;
                    style::set_hover(over);
                    page.restyle();
                    reflow = true;
                }
            }
            if btn != hover {
                hover = btn;
                dirty = true;
            }
            window.set_cursor_style(if hit.is_some() || btn.is_some() {
                CursorStyle::ClosedHand
            } else if (my as usize) < BAR_H {
                CursorStyle::Ibeam
            } else {
                CursorStyle::Arrow
            });
            // Highlight the option under the mouse in an open list.
            if let Some(dd) = dropdown.as_mut() {
                let row = dd.row_at(mx, my - BAR_H as f32 + scroll);
                if row != dd.hover {
                    dd.hover = row;
                    dirty = true;
                }
            }
            if down && !was_down && dropdown.is_some() {
                // A click while a list is open picks an option or closes the list.
                let dd = dropdown.take().unwrap();
                if let Some(i) = dd.row_at(mx, my - BAR_H as f32 + scroll) {
                    let opt = dd.options[i].0;
                    js::with_dom_mut(|d| forms::select_option(d, dd.select, opt));
                    fire(&mut page, dd.select, "input");
                    fire(&mut page, dd.select, "change");
                }
                form_changed = true;
            } else if down && !was_down {
                if let Some(b) = btn {
                    editing = false;
                    navigate = Some(match b {
                        0 => Go::Back,
                        1 => Go::Forward,
                        _ => Go::Reload,
                    });
                } else if (my as usize) < BAR_H {
                    editing = true;
                    address.clear();
                    dirty = true;
                } else {
                    // Let the page's JavaScript see the click first.
                    let node = lay.hit_at(mx, view_y, scroll);
                    let allowed = match (node, page.js.as_mut()) {
                        (Some(n), Some(js)) => js.click(n),
                        _ => true,
                    };
                    if js::take_dirty() {
                        page.restyle();
                        reflow = true;
                    }
                    if let Some(t) = js::take_navigation() {
                        navigate = Some(Go::To(net::resolve(&page.url, &t)));
                    }
                    // Form controls: focus, check, open a list, submit or reset.
                    let ctrl = node.and_then(|n| js::with_dom(|d| control_for(d, n)));
                    let ctrl = ctrl.filter(|&c| !js::with_dom(|d| forms::is_disabled(d, c)));
                    if ctrl != focused {
                        if let Some(old) = focused {
                            fire(&mut page, old, "change");
                            fire(&mut page, old, "blur");
                        }
                        focused = ctrl;
                        if let Some(n) = ctrl {
                            fire(&mut page, n, "focus");
                        }
                        form_changed = true;
                    }
                    if let (true, Some(c)) = (allowed, ctrl) {
                        // A click on a <label> is a click on its control.
                        if node.is_some_and(|n| n != c && js::with_dom(|d| !d.descendants(c).contains(&n))) {
                            fire(&mut page, c, "click");
                        }
                        match js::with_dom(|d| forms::control(d, c)) {
                            Some(forms::Control::Checkbox | forms::Control::Radio) => {
                                js::with_dom_mut(|d| forms::toggle(d, c));
                                fire(&mut page, c, "input");
                                fire(&mut page, c, "change");
                                form_changed = true;
                            }
                            Some(forms::Control::Select) => {
                                if let Some(r) = lay.hits.iter().rev().find(|(_, n)| *n == c).map(|(r, _)| *r) {
                                    let options = js::with_dom(|d| forms::options(d, c).into_iter().map(|o| (o, forms::option_label(d, o))).collect());
                                    dropdown = Some(Dropdown { select: c, x: r.x, y: r.y + r.h, w: r.w.max(120.0), options, hover: None });
                                    dirty = true;
                                }
                            }
                            Some(forms::Control::Button { submit: true, .. }) => {
                                if let Some(form) = js::with_dom(|d| forms::form_of(d, c)) {
                                    if let Some(go) = submit_form(&mut page, form, Some(c), true) {
                                        navigate = Some(go);
                                    }
                                }
                            }
                            Some(forms::Control::Button { reset: true, .. }) => {
                                if let Some(form) = js::with_dom(|d| forms::form_of(d, c)) {
                                    if fire(&mut page, form, "reset") {
                                        let defaults = page.form_defaults.clone();
                                        js::with_dom_mut(|d| forms::reset(d, form, &defaults));
                                        form_changed = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if let (true, None, Some(href)) = (allowed, ctrl, &hit) {
                    let target = net::resolve(&page.url, href);
                    let same_page = target.split('#').next() == page.url.split('#').next() && target.contains('#');
                    if !same_page && !target.starts_with("mailto:") && !target.starts_with("javascript:") {
                        navigate = Some(Go::To(target));
                    }
                    }
                }
            }
        }
        if form_changed {
            let editable = focused.is_some_and(|f| js::with_dom(|d| forms::control(d, f).is_some()));
            if !editable {
                focused = None;
            }
            let focus_moved = layout::focus() != focused;
            layout::set_focus(focused.map(|f| (f, js::with_dom(|d| forms::is_editable(d, f)))));
            // Scripts may have changed more than the value, and :focus rules may apply; restyle if so.
            if focus_moved || js::take_dirty() || js::with_dom(|d| d.nodes.len()) != page.styles.len() {
                page.restyle();
            }
            reflow = true;
        }
        if reflow && navigate.is_none() {
            lay = page.layout(fonts, w as f32, h.saturating_sub(BAR_H) as f32);
            dirty = true;
        }
        was_down = down;

        // The reload button (and Esc) stop a load in progress.
        if matches!(navigate, Some(Go::Reload)) && loading.is_some() || (stop_key && loading.is_some()) {
            navigate = None;
            loading = None;
            address = page.url.clone();
            window.set_title(&format!("{} — LighteningFox", if page.title.is_empty() { &page.url } else { &page.title }));
            dirty = true;
        }
        // Start a navigation: work out the target without touching history yet;
        // history changes only once the page actually arrives.
        let target = match &navigate {
            Some(Go::To(t)) | Some(Go::Post(t, _)) => Some(t.clone()),
            Some(Go::Back) => back.last().cloned(),
            Some(Go::Forward) => forward.last().cloned(),
            Some(Go::Reload) => Some(page.url.clone()),
            None => None,
        };
        if let (Some(target), Some(kind)) = (target, navigate) {
            let post = if let Go::Post(_, body) = &kind { Some(body.clone()) } else { None };
            generation += 1;
            let (g, t, tx) = (generation, target.clone(), tx.clone());
            std::thread::spawn(move || {
                let _ = tx.send((g, fetch_page(&t, post.as_deref())));
            });
            window.set_title(&format!("Loading {target} — LighteningFox"));
            address = target;
            editing = false;
            loading = Some(Loading { generation, kind, started: Instant::now() });
            dirty = true;
        }
        // A finished load: build the page here, where JavaScript lives.
        while let Ok((g, prepared)) = rx.try_recv() {
            let Some(l) = loading.take_if(|l| l.generation == g) else { continue };
            let old = page.url.clone();
            match l.kind {
                Go::To(_) | Go::Post(..) => {
                    if prepared.url != old {
                        back.push(old);
                        forward.clear();
                    }
                }
                Go::Back => {
                    back.pop();
                    forward.push(old);
                }
                Go::Forward => {
                    forward.pop();
                    back.push(old);
                }
                Go::Reload => {}
            }
            page = build_page(prepared);
            focused = None;
            dropdown = None;
            layout::set_focus(None);
            release_free_memory();
            lay = page.layout(fonts, w as f32, h.saturating_sub(BAR_H) as f32);
            scroll = 0.0;
            address = page.url.clone();
            window.set_title(&format!("{} — LighteningFox", if page.title.is_empty() { &page.url } else { &page.title }));
            dirty = true;
        }
        // Animate the loading bar about ten times a second.
        if loading.is_some() && last_spin.elapsed().as_millis() >= 100 {
            last_spin = Instant::now();
            dirty = true;
        }

        // Idle: poll less often. Anything happening (input, redraws, loading) goes back to 60 fps.
        let active = dirty || loading.is_some() || js::network_busy() || window.get_mouse_down(MouseButton::Left) || !window.get_keys().is_empty();
        if active {
            idle_frames = 0;
            if slow {
                window.set_target_fps(60);
                slow = false;
            }
        } else {
            idle_frames += 1;
            if idle_frames > 90 && !slow {
                window.set_target_fps(12);
                slow = true;
            }
        }
        if dirty {
            let spin = loading.as_ref().map(|l| l.started.elapsed().as_secs_f32());
            let ui = Toolbar { address: &address, editing, can_back: !back.is_empty(), can_forward: !forward.is_empty(), hover, loading: spin };
            draw(&mut canvas, fonts, &page.images, &lay, scroll, &ui);
            if let Some(dd) = &dropdown {
                draw_dropdown(&mut canvas, fonts, dd, scroll);
            }
            let _ = window.update_with_buffer(&canvas.buf, w, h);
            dirty = false;
        } else {
            window.update();
        }
    }
}

struct Toolbar<'a> {
    address: &'a str,
    editing: bool,
    can_back: bool,
    can_forward: bool,
    hover: Option<usize>,
    /// Seconds since the current load started, if a page is loading.
    loading: Option<f32>,
}

struct Loading {
    generation: u64,
    kind: Go,
    started: Instant,
}

fn draw(canvas: &mut Canvas, fonts: &Fonts, images: &images::Images, lay: &layout::Layout, scroll: f32, ui: &Toolbar) {
    canvas.clear(lay.page_bg);
    canvas.paint(fonts, images, &lay.items, BAR_H as f32, scroll);
    let w = canvas.w as f32;
    let bar = BAR_H as f32;
    canvas.fill_rect(0.0, 0.0, w, bar, Color::rgb(236, 236, 240));
    canvas.fill_rect(0.0, bar - 1.0, w, 1.0, Color::rgb(200, 200, 205));

    // Back, forward, reload
    let enabled = [ui.can_back, ui.can_forward, true];
    for (i, (bx, label)) in BUTTONS.iter().enumerate() {
        if ui.hover == Some(i) && enabled[i] {
            canvas.fill_rect(*bx, 4.0, BTN_W, bar - 8.0, Color::rgb(214, 214, 222));
        }
        let ink = if enabled[i] { Color::rgb(40, 40, 50) } else { Color::rgb(170, 170, 178) };
        let size = 18.0;
        // While loading, reload becomes stop.
        let label = if i == 2 && ui.loading.is_some() { &"✕" } else { label };
        let tw = fonts.width(font::BOLD, label, size);
        canvas.draw_text(fonts, bx + (BTN_W - tw) / 2.0, 24.0, label, font::BOLD, size, ink);
    }

    // Address field
    let field = if ui.editing { Color::rgb(255, 255, 255) } else { Color::rgb(248, 248, 250) };
    canvas.fill_rect(FIELD_X, 5.0, w - FIELD_X - 8.0, bar - 10.0, field);
    let text = if ui.editing { format!("{}|", ui.address) } else { ui.address.to_string() };
    canvas.draw_text(fonts, FIELD_X + 8.0, 23.0, &text, font::REGULAR, 14.0, Color::rgb(40, 40, 50));
    // Loading bar: a segment sliding along the bottom of the toolbar.
    if let Some(t) = ui.loading {
        let span = w * 0.25;
        let pos = (t * 0.8).fract() * (w + span) - span;
        canvas.fill_rect(pos.max(0.0), bar - 3.0, (pos + span).min(w) - pos.max(0.0), 2.0, Color::rgb(249, 115, 22));
    }
}

/// An open <select> list: a white panel of options below the control.
fn draw_dropdown(canvas: &mut Canvas, fonts: &Fonts, dd: &Dropdown, scroll: f32) {
    let top = dd.y - scroll + BAR_H as f32;
    let h = dd.options.len() as f32 * OPTION_H + 2.0;
    canvas.fill_rect(dd.x + 2.0, top + 2.0, dd.w, h, Color { r: 0, g: 0, b: 0, a: 40 });
    canvas.fill_rect(dd.x, top, dd.w, h, Color::rgb(140, 140, 150));
    canvas.fill_rect(dd.x + 1.0, top + 1.0, dd.w - 2.0, h - 2.0, Color::rgb(255, 255, 255));
    for (i, (_, label)) in dd.options.iter().enumerate() {
        let y = top + 1.0 + i as f32 * OPTION_H;
        let ink = if dd.hover == Some(i) {
            canvas.fill_rect(dd.x + 1.0, y, dd.w - 2.0, OPTION_H, Color::rgb(11, 107, 211));
            Color::rgb(255, 255, 255)
        } else {
            Color::rgb(20, 20, 30)
        };
        canvas.draw_text(fonts, dd.x + 8.0, y + 15.5, label, font::REGULAR, 13.33, ink);
    }
}

/// Screenshot tests: every page in tests/ is rendered and compared with its approved
/// image in tests/reference/. Run `UPDATE_REFERENCES=1 cargo test` to approve new output.
#[cfg(test)]
mod screenshot_tests {
    use super::*;

    const PAGES: &[(&str, usize)] = &[
        ("about:home", 1024),
        ("tests/images.html", 1024),
        ("tests/css3.html", 900),
        ("tests/backgrounds.html", 1024),
        ("tests/inline-block.html", 900),
        ("tests/js-demo.html", 800),
        ("tests/nav-a.html", 800),
        ("tests/modern.html", 1024),
        ("tests/fixed.html", 1024),
        ("tests/floats.html", 1024),
        ("tests/tables.html", 900),
        ("tests/selectors.html", 700),
        ("tests/grid.html", 900),
        ("tests/forms.html", 800),
        ("tests/fetch.html", 700),
    ];

    fn read_png(path: &str) -> Option<(usize, usize, Vec<u8>)> {
        let img = image::open(path).ok()?.to_rgb8();
        Some((img.width() as usize, img.height() as usize, img.into_raw()))
    }

    #[test]
    fn pages_match_reference_images() {
        let fonts = Fonts::load().expect("fonts");
        let update = std::env::var("UPDATE_REFERENCES").is_ok();
        std::fs::create_dir_all("tests/reference").unwrap();
        std::fs::create_dir_all("target/screenshot-diffs").unwrap();
        let mut failures = Vec::new();
        for (url, width) in PAGES {
            let name = url.trim_start_matches("tests/").trim_end_matches(".html").replace(':', "-");
            let reference = format!("tests/reference/{name}.png");
            let (_, _, canvas) = render_page(&fonts, url, *width);
            if update || !std::path::Path::new(&reference).exists() {
                save_png(&canvas, &reference).unwrap();
                eprintln!("approved {reference}");
                continue;
            }
            let actual = format!("target/screenshot-diffs/{name}.png");
            save_png(&canvas, &actual).unwrap();
            let (rw, rh, want) = read_png(&reference).unwrap();
            let (aw, ah, got) = read_png(&actual).unwrap();
            if (rw, rh) != (aw, ah) {
                failures.push(format!("{url}: size {aw}x{ah}, expected {rw}x{rh} (see {actual})"));
                continue;
            }
            // Allow tiny anti-aliasing differences: count pixels that differ noticeably.
            let differing = want
                .chunks(3)
                .zip(got.chunks(3))
                .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 8))
                .count();
            let share = differing as f64 / (rw * rh) as f64;
            if share > 0.001 {
                failures.push(format!("{url}: {differing} pixels differ ({:.2}%), see {actual}", share * 100.0));
            }
        }
        assert!(failures.is_empty(), "screenshot mismatches:\n{}", failures.join("\n"));
    }
}
