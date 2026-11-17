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
mod geometry;
mod grid;
mod html;
mod images;
mod js;
mod layout;
mod library;
mod settings;
mod net;
mod paint;
mod style;
mod textedit;
mod transform;

use font::Fonts;
use minifb::{CursorStyle, Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use paint::Canvas;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Instant;
use std::rc::Rc;
use style::Color;

/// The tab strip above the toolbar, and the toolbar itself.
const TAB_H: f32 = 34.0;
const TOOL_H: f32 = 36.0;
const BAR_H: usize = (TAB_H + TOOL_H) as usize;
const TAB_MAX_W: f32 = 220.0;
const NEW_TAB_W: f32 = 32.0;
/// The bookmark star at the right end of the address field, and the suggestion rows below it.
const STAR_W: f32 = 30.0;
const SUGGEST_H: f32 = 30.0;
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
    if y < TAB_H || y >= BAR_H as f32 {
        return None;
    }
    BUTTONS.iter().position(|(bx, _)| x >= *bx && x < bx + BTN_W)
}
/// What's under the mouse in the tab strip.
#[derive(Clone, Copy, PartialEq, Debug)]
enum TabHit {
    Tab(usize),
    Close(usize),
    New,
}

/// Each tab's (x, width) in a window `w` wide holding `n` tabs.
fn tab_rects(w: f32, n: usize) -> Vec<(f32, f32)> {
    let avail = (w - 8.0 - NEW_TAB_W - 8.0).max(60.0);
    let tw = (avail / n.max(1) as f32).min(TAB_MAX_W).floor();
    (0..n).map(|i| (6.0 + i as f32 * tw, tw)).collect()
}

fn tab_at(x: f32, y: f32, w: f32, n: usize) -> Option<TabHit> {
    if y >= TAB_H || y < 4.0 {
        return None;
    }
    let rects = tab_rects(w, n);
    for (i, &(tx, tw)) in rects.iter().enumerate() {
        if x >= tx && x < tx + tw {
            // The × sits at the right end of a tab.
            return Some(if tw > 60.0 && x >= tx + tw - 26.0 { TabHit::Close(i) } else { TabHit::Tab(i) });
        }
    }
    let end = rects.last().map_or(6.0, |&(tx, tw)| tx + tw);
    (x >= end + 4.0 && x < end + 4.0 + NEW_TAB_W).then_some(TabHit::New)
}

/// A tab that isn't showing: everything the window loop keeps for the visible tab, plus the
/// page's per-thread state (its DOM, box scroll positions, web fonts, on-demand layout).
struct Saved {
    page: Page,
    scroll: f32,
    back: Vec<Entry>,
    forward: Vec<Entry>,
    current: Entry,
    focused: Option<dom::NodeId>,
    address: String,
    dom: dom::Dom,
    scrolls: HashMap<dom::NodeId, f32>,
    families: HashMap<String, Vec<(u16, u16, bool, u8)>>,
    provider: Option<geometry::Provider>,
}

/// A background tab. A page that finishes loading while its tab is hidden waits in
/// `pending` and is built when the tab is shown (so hidden tabs cost little).
struct Slot {
    saved: Option<Saved>,
    loading: Option<Loading>,
    pending: Option<(Loading, Prepared)>,
    title: String,
    url: String,
    /// A sleeping tab keeps its Back/Forward lists while its page is freed.
    back: Vec<Entry>,
    forward: Vec<Entry>,
    /// When it was last the visible tab (for putting it to sleep).
    last_seen: Instant,
}

impl Slot {
    fn new(url: String, title: String) -> Slot {
        Slot { saved: None, loading: None, pending: None, title, url, back: Vec::new(), forward: Vec::new(), last_seen: Instant::now() }
    }

    /// Asleep: no page kept and nothing loading; showing it loads its URL again.
    fn asleep(&self) -> bool {
        self.saved.is_none() && self.loading.is_none() && self.pending.is_none() && self.url != "about:blank"
    }
}

/// The open tabs, saved so the next start can reopen them: first line is the active index.
fn save_session(tabs: &[(String, String)], active: usize) {
    let Some(dir) = cookies::profile_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let mut body = format!("{active}\n");
    for (u, t) in tabs {
        body.push_str(&format!("{}\t{}\n", u.replace(['\t', '\n'], " "), t.replace(['\t', '\n'], " ")));
    }
    let path = dir.join("session.txt");
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn load_session() -> Option<(Vec<(String, String)>, usize)> {
    let text = std::fs::read_to_string(cookies::profile_dir()?.join("session.txt")).ok()?;
    let mut lines = text.lines();
    let active: usize = lines.next()?.trim().parse().ok()?;
    let urls: Vec<(String, String)> = lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| match l.split_once('\t') {
            Some((u, t)) => (u.to_string(), if t.is_empty() { u.to_string() } else { t.to_string() }),
            None => (l.to_string(), l.to_string()),
        })
        .collect();
    (!urls.is_empty()).then(|| {
        let a = active.min(urls.len() - 1);
        (urls, a)
    })
}

/// How long a background tab may go unseen before it sleeps (LF_TAB_SLEEP_SECONDS overrides).
fn sleep_after() -> Option<std::time::Duration> {
    if let Some(secs) = std::env::var("LF_TAB_SLEEP_SECONDS").ok().and_then(|v| v.parse::<u64>().ok()) {
        return Some(std::time::Duration::from_secs(secs));
    }
    let m = settings::get().sleep_minutes;
    (m > 0).then(|| std::time::Duration::from_secs(m as u64 * 60))
}

/// What the tab strip and shortcuts asked for; carried out at the top of the next frame.
enum TabCmd {
    Switch(usize),
    Close(usize),
    /// Open a URL in a new tab, in front or in the background.
    Open(String, bool),
}

impl Page {
    /// A page with nothing in it, touching no per-thread state (a placeholder while tabs swap).
    fn empty() -> Page {
        Page {
            url: "about:blank".into(),
            title: String::new(),
            sheets: Vec::new(),
            styles: Vec::new(),
            images: images::Images::default(),
            js: None,
            hover_rules: false,
            doc: 0,
            form_defaults: Vec::new(),
        }
    }

    fn label(&self) -> String {
        if self.title.trim().is_empty() { self.url.clone() } else { self.title.trim().to_string() }
    }
}

/// The <title> of a fetched page, for a background tab's label.
fn prepared_title(p: &Prepared) -> String {
    p.dom.find_all("title").first().map(|&t| p.dom.text_content(t).trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| p.url.clone())
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
    /// Which loaded document this is (history entries pushed by its scripts share it).
    doc: u64,
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
        let lay = js::with_dom(|d| layout::layout(d, &self.styles, fonts, &self.images, width, viewport_h));
        geometry::publish(&lay, width, viewport_h);
        lay
    }
}

/// Everything a page needs from the network, gathered off the UI thread.
struct Prepared {
    url: String,
    dom: dom::Dom,
    sheets: Vec<css::Stylesheet>,
    images: images::Images,
    scripts: HashMap<String, String>,
    fonts: Vec<WebFont>,
}

/// A downloaded @font-face font. `data` is None when an earlier page already loaded it.
struct WebFont {
    family: String,
    weight: (u16, u16),
    italic: bool,
    url: String,
    data: Option<Vec<u8>>,
}

/// Font URLs already turned into fonts (shared with the loading threads, so they skip them).
static LOADED_FONT_URLS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// WOFF2 and WOFF are compressed wrappers around TrueType/OpenType data.
fn decode_font(bytes: Vec<u8>) -> Option<Vec<u8>> {
    match bytes.get(..4) {
        Some(b"wOF2") => wuff::decompress_woff2(&bytes).ok(),
        Some(b"wOFF") => wuff::decompress_woff1(&bytes).ok(),
        _ => Some(bytes),
    }
}

/// Download the @font-face fonts the page's styles actually use (at most 24).
fn fetch_fonts(sheets: &[css::Stylesheet], styles: &[Rc<style::Style>], page_url: &str) -> Vec<WebFont> {
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen: std::collections::HashSet<*const str> = std::collections::HashSet::new();
    for st in styles {
        if let Some(f) = &st.family {
            if seen.insert(Rc::as_ptr(f)) {
                for name in f.split(',') {
                    used.insert(name.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string());
                }
            }
        }
    }
    let loaded = LOADED_FONT_URLS.lock().map(|l| l.clone()).unwrap_or_default();
    let wanted: Vec<(&css::FontFace, String)> = sheets
        .iter()
        .flat_map(|s| &s.font_faces)
        .filter(|f| used.contains(&f.family))
        .filter_map(|f| f.best_url().map(|u| (f, u)))
        .take(24)
        .collect();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = wanted
            .iter()
            .map(|(face, url)| {
                let skip = loaded.contains(url);
                scope.spawn(move || {
                    let data = if skip {
                        None
                    } else {
                        match net::fetch_sub_bytes(url, page_url) {
                            Ok((_, b)) => decode_font(b),
                            Err(e) => {
                                eprintln!("[font] {url}: {e}");
                                return None;
                            }
                        }
                    };
                    Some(WebFont { family: face.family.clone(), weight: face.weight, italic: face.italic, url: url.clone(), data })
                })
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
    })
}

thread_local! {
    /// Web font URL -> font id, so a font shared by many pages is added once.
    static FONT_IDS: RefCell<HashMap<String, u8>> = RefCell::new(HashMap::new());
}

/// Make the page's web fonts available to layout under their family names.
fn install_fonts(fonts: Vec<WebFont>) {
    let Some(registry) = FONTS.with(|c| c.get()) else { return };
    let mut families: HashMap<String, Vec<(u16, u16, bool, u8)>> = HashMap::new();
    for f in fonts {
        let known = FONT_IDS.with(|m| m.borrow().get(&f.url).copied());
        let id = match (known, f.data) {
            (Some(id), _) => Some(id),
            (None, Some(data)) => registry.add_web_font(data, f.weight.0 >= 600, f.italic).inspect(|&id| {
                FONT_IDS.with(|m| m.borrow_mut().insert(f.url.clone(), id));
                if let Ok(mut l) = LOADED_FONT_URLS.lock() {
                    l.push(f.url.clone());
                }
            }),
            (None, None) => None,
        };
        match id {
            Some(id) => families.entry(f.family).or_default().push((f.weight.0, f.weight.1, f.italic, id)),
            None => eprintln!("[font] {}: not a usable font", f.url),
        }
    }
    font::set_web_families(families);
}

/// Fetch the page, its stylesheets, scripts and images, all in parallel where possible.
/// Runs on a worker thread; touches nothing the UI thread owns.
fn fetch_page(url: &str, post: Option<&str>) -> Prepared {
    let (final_url, body) = if url == "about:home" || url.is_empty() {
        ("about:home".to_string(), START_PAGE.to_string())
    } else if url == "about:blank" {
        ("about:blank".to_string(), String::new())
    } else if let Some(q) = url.strip_prefix("about:history") {
        ("about:history".to_string(), library::history_page(q))
    } else if let Some(q) = url.strip_prefix("about:settings") {
        ("about:settings".to_string(), settings::page(q))
    } else if let Some(q) = url.strip_prefix("about:bookmarks") {
        ("about:bookmarks".to_string(), library::bookmarks_page(q))
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
    let fonts = fetch_fonts(&sheets, &styles, &final_url);
    Prepared { url: final_url, dom, sheets, images, scripts, fonts }
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

thread_local! {
    /// The loaded fonts, for laying pages out on demand when scripts ask where things are.
    static FONTS: std::cell::Cell<Option<&'static Fonts>> = const { std::cell::Cell::new(None) };
}

/// Load the fonts once for the life of the program.
#[cfg_attr(not(test), allow(dead_code))]
fn load_fonts() -> Option<&'static Fonts> {
    if let Some(f) = FONTS.with(|c| c.get()) {
        return Some(f);
    }
    let f: &'static Fonts = Box::leak(Box::new(Fonts::load().ok()?));
    FONTS.with(|c| c.set(Some(f)));
    Some(f)
}

/// Turn fetched resources into a live page: install the DOM, run scripts, style it.
/// Runs on the UI thread (JavaScript and the DOM live there).
fn build_page(p: Prepared) -> Page {
    let Prepared { url, dom, sheets, images, scripts, fonts } = p;
    install_fonts(fonts);
    // Start JavaScript only if the page has scripts or inline event handlers:
    // the engine costs several megabytes even when it has nothing to run.
    let has_scripts = dom.descendants(dom::Dom::ROOT).iter().any(|&n| match &dom.nodes[n].data {
        dom::NodeData::Element { tag, attrs } => tag == "script" || attrs.iter().any(|(k, _)| k.starts_with("on")),
        _ => false,
    });
    js::set_dom(dom);
    layout::reset_element_scrolls();
    // Scripts that ask where elements are get a fresh layout of the page as it is then.
    let provider: Option<Box<dyn Fn(f32, f32) -> layout::Layout>> = FONTS.with(|c| c.get()).map(|fonts| {
        let sheets = sheets.clone();
        let images = images.share();
        Box::new(move |w: f32, h: f32| {
            js::with_dom(|d| {
                let styles = style::compute(d, &sheets);
                layout::layout(d, &styles, fonts, &images, w, h)
            })
        }) as Box<dyn Fn(f32, f32) -> layout::Layout>
    });
    geometry::set_provider(provider);
    let js = has_scripts.then(|| {
        let mut js = js::Js::new(&url);
        js.run_scripts(&url, &scripts);
        js
    });
    js::take_dirty();
    style::set_hover(None);
    let hover_rules = style::uses_hover(&sheets);
    let form_defaults = js::with_dom(forms::defaults);
    static DOCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let doc = DOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut page = Page { url, title: String::new(), sheets, styles: Vec::new(), images, js, hover_rules, doc, form_defaults };
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
    // Whether a page was named on the command line (else: restore the last session or go home).
    let mut explicit = false;
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
            other => {
                url = net::normalize_input(other);
                explicit = true;
            }
        }
        i += 1;
    }
    let rss = || std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 4 / 1024;
    let probe = std::env::var("LF_MEM_PROBE").is_ok();
    if probe {
        eprintln!("[mem] start: {} MB", rss());
    }
    let fonts: &'static Fonts = match Fonts::load() {
        Ok(f) => Box::leak(Box::new(f)),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    FONTS.with(|c| c.set(Some(fonts)));
    if probe {
        eprintln!("[mem] fonts loaded: {} MB", rss());
        let js = js::Js::new("about:blank");
        eprintln!("[mem] + one JS engine: {} MB", rss());
        drop(js);
        let page = load(&url);
        eprintln!("[mem] + page loaded: {} MB", rss());
        let lay = page.layout(fonts, width as f32, 768.0);
        eprintln!("[mem] + layout ({} items): {} MB", lay.items.len(), rss());
        return;
    }
    match png {
        Some(out) => render_png(fonts, &url, width, &out),
        None => run_window(fonts, &url, width, explicit),
    }
}

/// Load a page and paint the whole of it (up to 16,000 px tall) into a canvas.
fn render_page(fonts: &Fonts, url: &str, width: usize) -> (Page, layout::Layout, Canvas) {
    geometry::set_viewport(width as f32, 768.0);
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

/// A key press with the modifiers held at that moment.
#[derive(Clone, Copy, Debug)]
struct KeyPress {
    key: Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

/// Collects typed characters and key presses as they happen, so a quick tap is never
/// missed between two frames (the window polls slowly when idle).
struct Typed {
    chars: Rc<RefCell<Vec<char>>>,
    keys: Rc<RefCell<Vec<KeyPress>>>,
    ctrl: [bool; 2],
    shift: [bool; 2],
    alt: [bool; 2],
}
impl minifb::InputCallback for Typed {
    fn add_char(&mut self, c: u32) {
        if let Some(ch) = char::from_u32(c) {
            self.chars.borrow_mut().push(ch);
        }
    }

    fn set_key_state(&mut self, key: Key, down: bool) {
        match key {
            Key::LeftCtrl => self.ctrl[0] = down,
            Key::RightCtrl => self.ctrl[1] = down,
            Key::LeftShift => self.shift[0] = down,
            Key::RightShift => self.shift[1] = down,
            Key::LeftAlt => self.alt[0] = down,
            Key::RightAlt => self.alt[1] = down,
            _ if down => self.keys.borrow_mut().push(KeyPress {
                key,
                ctrl: self.ctrl[0] || self.ctrl[1],
                shift: self.shift[0] || self.shift[1],
                alt: self.alt[0] || self.alt[1],
            }),
            _ => {}
        }
    }
}

/// What a round of editing keys did to a text field.
struct EditResult {
    changed: bool,
    moved: bool,
    enter: bool,
}

/// Apply this frame's keys and typed characters to an editable text: caret movement,
/// selection (Shift), word jumps (Ctrl), deleting, and clipboard (Ctrl+A/C/X/V).
#[allow(clippy::too_many_arguments)]
fn edit_keys(ed: &mut textedit::TextEdit, keys: &[KeyPress], chars: &[char], ctrl_now: bool, multiline: bool, clip: &mut Option<arboard::Clipboard>) -> EditResult {
    let before = ed.clone();
    let mut enter = false;
    for p in keys {
        let (k, ctrl, shift) = (p.key, p.ctrl, p.shift);
        match k {
            Key::Left => ed.left(ctrl, shift),
            Key::Right => ed.right(ctrl, shift),
            Key::Home if ctrl => ed.move_to(0, shift),
            Key::Home => ed.home(shift),
            Key::End if ctrl => {
                let n = ed.len();
                ed.move_to(n, shift)
            }
            Key::End => ed.end(shift),
            Key::Up if multiline => ed.vertical(false, shift),
            Key::Down if multiline => ed.vertical(true, shift),
            Key::Backspace => ed.backspace(ctrl),
            Key::Delete => ed.delete(ctrl),
            Key::A if ctrl => ed.select_all_text(),
            Key::C | Key::X if ctrl && ed.has_selection() => {
                if let Some(c) = clip.as_mut() {
                    let _ = c.set_text(ed.selected_text());
                }
                if k == Key::X {
                    ed.insert("");
                }
            }
            Key::V if ctrl => {
                let pasted = clip.as_mut().and_then(|c| c.get_text().ok()).unwrap_or_default();
                let pasted = if multiline { pasted.replace("\r\n", "\n") } else { pasted.replace(['\r', '\n'], " ") };
                ed.insert(&pasted);
            }
            _ => {}
        }
    }
    for &c in chars {
        match c {
            '\r' | '\n' if multiline => ed.insert("\n"),
            '\r' | '\n' => enter = true,
            c if !c.is_control() && c != '\u{7f}' && !ctrl_now => ed.insert(c.encode_utf8(&mut [0; 4])),
            _ => {}
        }
    }
    EditResult { changed: ed.text != before.text, moved: ed.caret != before.caret || ed.anchor != before.anchor, enter }
}

/// The character position nearest to `x` px into `text`.
fn char_index_at(fonts: &Fonts, font: u8, size: f32, text: &str, x: f32) -> usize {
    let mut acc = 0.0;
    for (i, c) in text.chars().enumerate() {
        let w = fonts.advance(font, c, size);
        if x < acc + w / 2.0 {
            return i;
        }
        acc += w;
    }
    text.chars().count()
}

/// One entry in the tab's history: a URL, the document it belongs to, and the state
/// object a script attached with pushState (as JSON).
#[derive(Clone, Debug)]
struct Entry {
    url: String,
    doc: u64,
    state: String,
}

fn same_document(a: &str, b: &str) -> bool {
    a.split('#').next() == b.split('#').next()
}

/// Where a URL's #fragment points on the page (page y), if anywhere.
fn fragment_target(url: &str) -> Option<f32> {
    let frag = url.split_once('#')?.1;
    let frag = net::percent_decode_str(frag);
    if frag.is_empty() || frag.eq_ignore_ascii_case("top") {
        return Some(0.0);
    }
    js::with_dom(|d| {
        let id = d.descendants(dom::Dom::ROOT).into_iter().find(|&n| d.attr(n, "id") == Some(frag.as_str()))
            .or_else(|| d.descendants(dom::Dom::ROOT).into_iter().find(|&n| d.tag(n) == Some("a") && d.attr(n, "name") == Some(frag.as_str())))?;
        geometry::page_rect(d, id).map(|(r, _)| r.y)
    })
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

fn run_window(fonts: &Fonts, start: &str, width: usize, explicit: bool) {
    // Without a page on the command line, reopen last session's tabs (if that's the setting),
    // else open the home page.
    let session = if explicit || !settings::get().restore_tabs { None } else { load_session() };
    let home = settings::get().home;
    let start: &str = match &session {
        Some((urls, a)) => &urls[*a].0.clone(),
        None if explicit => start,
        None => &home,
    };
    let mut window = Window::new("LighteningFox", width, 768, WindowOptions { resize: true, ..WindowOptions::default() })
        .unwrap_or_else(|e| {
            eprintln!("Could not open a window: {e}");
            std::process::exit(1);
        });
    window.set_target_fps(60);
    let mut idle_frames = 0u32;
    let mut slow = false;
    let typed = Rc::new(RefCell::new(Vec::new()));
    let pressed = Rc::new(RefCell::new(Vec::<KeyPress>::new()));
    window.set_input_callback(Box::new(Typed { chars: typed.clone(), keys: pressed.clone(), ctrl: [false; 2], shift: [false; 2], alt: [false; 2] }));
    // Wayland doesn't repeat held keys for us; there, add the window's own repeats.
    let add_repeats = std::env::var_os("WAYLAND_DISPLAY").is_some();

    let mut back: Vec<Entry> = Vec::new();
    let mut forward: Vec<Entry> = Vec::new();
    let mut hover: Option<usize> = None;
    let mut hovered: Option<dom::NodeId> = None;
    // Pages load on worker threads; results come back tagged with a generation number
    // so a stopped or superseded load is ignored when it arrives.
    let (tx, rx) = mpsc::channel::<(u64, Prepared)>();
    let mut generation = 0u64;
    let mut loading: Option<Loading> = None;
    let mut last_spin = Instant::now();
    let (mut w, mut h) = window.get_size();
    geometry::set_viewport(w as f32, h.saturating_sub(BAR_H) as f32);
    let mut page = load(start);
    library::visit(&page.url, &page.title);
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
    let mut current = Entry { url: page.url.clone(), doc: page.doc, state: "null".into() };
    // Tabs: the visible one lives in the variables above; the others are packed into `tabs`
    // (its own slot holds None).
    let mut tabs: Vec<Option<Slot>> = vec![None];
    let mut active = 0usize;
    // The other tabs of a restored session start asleep: they load when first shown.
    if let Some((urls, a)) = &session {
        tabs = urls.iter().enumerate().map(|(i, (u, t))| (i != *a).then(|| Slot::new(u.clone(), t.clone()))).collect();
        active = *a;
    }
    let mut session_saved: (Vec<(String, String)>, usize) = (Vec::new(), usize::MAX);
    let mut last_housekeeping = Instant::now();
    let mut tab_cmd: Option<TabCmd> = None;
    let mut pending_nav: Option<Go> = None;
    let mut closed: Vec<String> = Vec::new();
    let mut tab_hover: Option<TabHit> = None;
    let mut was_mid = false;
    // Text editing: the address bar and the focused field each have a caret and selection.
    let mut clipboard = arboard::Clipboard::new().ok();
    let mut aedit = textedit::TextEdit::default();
    let mut fedit: Option<textedit::TextEdit> = None;
    // Address bar suggestions while typing, and which one the arrow keys picked.
    let mut suggestions: Vec<library::Suggestion> = Vec::new();
    let mut sugg_sel: Option<usize> = None;
    let mut sugg_hover: Option<usize> = None;
    // For double-click detection: when and where the last click was.
    let mut last_click = (Instant::now() - std::time::Duration::from_secs(5), 0.0f32, 0.0f32);

    // Pack the visible tab away (into a Saved) / show a packed tab.
    macro_rules! stash {
        () => {
            Saved {
                page: std::mem::replace(&mut page, Page::empty()),
                scroll,
                back: std::mem::take(&mut back),
                forward: std::mem::take(&mut forward),
                current: current.clone(),
                focused,
                address: address.clone(),
                dom: js::take_dom(),
                scrolls: layout::take_element_scrolls(),
                families: font::take_web_families(),
                provider: geometry::take_provider(),
            }
        };
    }
    macro_rules! restore {
        ($slot:expr) => {{
            let slot: Slot = $slot;
            let asleep = slot.asleep();
            loading = slot.loading;
            match slot.saved {
                Some(sv) => {
                    page = sv.page;
                    scroll = sv.scroll;
                    back = sv.back;
                    forward = sv.forward;
                    current = sv.current;
                    focused = sv.focused;
                    address = sv.address;
                    js::set_dom(sv.dom);
                    layout::set_element_scrolls(sv.scrolls);
                    font::set_web_families(sv.families);
                    geometry::set_provider(sv.provider);
                    if let Some(j) = page.js.as_ref() {
                        js::activate(j);
                    }
                }
                None => {
                    page = load("about:blank");
                    back = slot.back;
                    forward = slot.forward;
                    current = Entry { url: slot.url.clone(), doc: page.doc, state: "null".into() };
                    // A sleeping tab wakes up by loading its page again.
                    if asleep {
                        pending_nav = Some(Go::To(slot.url.clone()));
                    }
                    focused = None;
                    address = slot.url.clone();
                    scroll = 0.0;
                }
            }
            // A page that finished loading while hidden: hand it to the normal arrival code.
            if let Some((l, p)) = slot.pending {
                let g = l.generation;
                loading = Some(l);
                let _ = tx.send((g, p));
            }
            dropdown = None;
            editing = false;
            fedit = None;
            hovered = None;
            style::set_hover(None);
            layout::set_focus(focused.map(|f| (f, js::with_dom(|d| forms::is_editable(d, f)))));
            lay = page.layout(fonts, w as f32, h.saturating_sub(BAR_H) as f32);
            scroll = scroll.clamp(0.0, (lay.height - h.saturating_sub(BAR_H) as f32).max(0.0));
            window.set_title(&format!("{} — LighteningFox", page.label()));
            dirty = true;
        }};
    }

    while window.is_open() {
        // Tab strip and shortcut actions from the last frame.
        if let Some(cmd) = tab_cmd.take() {
            match cmd {
                TabCmd::Switch(i) if i != active && i < tabs.len() => {
                    let (title, url) = (page.label(), page.url.clone());
                    let (back_list, fwd_list) = (back.clone(), forward.clone());
                    let saved = stash!();
                    tabs[active] = Some(Slot { saved: Some(saved), loading: loading.take(), back: back_list, forward: fwd_list, ..Slot::new(url, title) });
                    let slot = tabs[i].take().expect("tab slot");
                    active = i;
                    restore!(slot);
                }
                TabCmd::Switch(_) => {}
                TabCmd::Open(url, true) => {
                    generation += 1;
                    let (g, t, tx) = (generation, url.clone(), tx.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send((g, fetch_page(&t, None)));
                    });
                    let l = Loading { generation, kind: Go::To(url.clone()), started: Instant::now() };
                    tabs.insert(active + 1, Some(Slot { loading: Some(l), ..Slot::new(url.clone(), url) }));
                    dirty = true;
                }
                TabCmd::Open(url, false) => {
                    let (title, old_url) = (page.label(), page.url.clone());
                    let saved = stash!();
                    tabs[active] = Some(Slot { saved: Some(saved), loading: loading.take(), ..Slot::new(old_url, title) });
                    // A new empty tab goes at the end; one opened from a page sits beside it.
                    let blank = url == "about:blank";
                    let at = if blank { tabs.len() } else { active + 1 };
                    tabs.insert(at, None);
                    active = at;
                    restore!(Slot::new("about:blank".into(), url.clone()));
                    if blank {
                        // A new empty tab: start typing an address.
                        editing = true;
                        address.clear();
                        aedit = textedit::TextEdit::default();
                    } else {
                        pending_nav = Some(Go::To(url));
                    }
                }
                TabCmd::Close(i) if i < tabs.len() => {
                    if tabs.len() == 1 {
                        break;
                    }
                    if i == active {
                        closed.push(page.url.clone());
                        if let Some(j) = page.js.as_ref() {
                            js::forget(j);
                        }
                        drop(std::mem::replace(&mut page, Page::empty()));
                        let _ = (js::take_dom(), layout::take_element_scrolls(), font::take_web_families(), geometry::take_provider());
                        tabs.remove(i);
                        let next = i.min(tabs.len() - 1);
                        let slot = tabs[next].take().expect("tab slot");
                        active = next;
                        restore!(slot);
                        release_free_memory();
                    } else {
                        if let Some(slot) = tabs.remove(i) {
                            closed.push(slot.url.clone());
                            if let Some(j) = slot.saved.as_ref().and_then(|sv| sv.page.js.as_ref()) {
                                js::forget(j);
                            }
                        }
                        if i < active {
                            active -= 1;
                        }
                        release_free_memory();
                        dirty = true;
                    }
                }
                TabCmd::Close(_) => {}
            }
        }
        let mut navigate: Option<Go> = pending_nav.take();
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
        // JavaScript: run due tasks, then this frame's animation callbacks; restyle and
        // reflow if the DOM changed.
        let mut animating = false;
        if let Some(js) = page.js.as_mut() {
            js.tick();
            if js.wants_frame() {
                js.run_frame();
                animating = true;
            }
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

        // This frame's key presses (from the input callback, so none are missed).
        let mut presses: Vec<KeyPress> = pressed.borrow_mut().drain(..).collect();
        if add_repeats {
            let fresh = window.get_keys_pressed(KeyRepeat::No);
            let ctrl = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
            let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
            let alt = window.is_key_down(Key::LeftAlt) || window.is_key_down(Key::RightAlt);
            for key in window.get_keys_pressed(KeyRepeat::Yes).into_iter().filter(|k| !fresh.contains(k)) {
                presses.push(KeyPress { key, ctrl, shift, alt });
            }
        }
        let escape = presses.iter().any(|p| p.key == Key::Escape);
        // Tab shortcuts: Ctrl+T new, Ctrl+W close, Ctrl+(Shift+)Tab next/previous,
        // Ctrl+1…8 / Ctrl+9 (last), Ctrl+Shift+T reopen the last closed tab.
        {
            for p in presses.iter().filter(|p| p.ctrl) {
                let (key, shift) = (p.key, p.shift);
                {
                    let n = tabs.len();
                    let digit = [Key::Key1, Key::Key2, Key::Key3, Key::Key4, Key::Key5, Key::Key6, Key::Key7, Key::Key8].iter().position(|&k| k == key);
                    tab_cmd = match key {
                        Key::T if shift => closed.pop().map(|u| TabCmd::Open(u, false)),
                        Key::T => Some(TabCmd::Open("about:blank".into(), false)),
                        Key::W => Some(TabCmd::Close(active)),
                        Key::Tab if shift => Some(TabCmd::Switch((active + n - 1) % n)),
                        Key::Tab => Some(TabCmd::Switch((active + 1) % n)),
                        Key::Key9 => Some(TabCmd::Switch(n - 1)),
                        Key::H => Some(TabCmd::Open("about:history".into(), false)),
                        Key::O if shift => Some(TabCmd::Open("about:bookmarks".into(), false)),
                        Key::D => {
                            library::toggle_bookmark(&page.url, &page.title);
                            dirty = true;
                            tab_cmd.take()
                        }
                        _ => digit.filter(|&d| d < n).map(TabCmd::Switch).or(tab_cmd.take()),
                    };
                }
            }
        }
        // Typing in the address bar.
        let chars: Vec<char> = typed.borrow_mut().drain(..).collect();
        let typing = !editing && focused.is_some_and(|f| js::with_dom(|d| forms::is_editable(d, f)));
        let ctrl_down = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
        let shift_down = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
        let edit_keys_now: Vec<KeyPress> = if typing || editing { presses.clone() } else { Vec::new() };
        if typing {
            let f = focused.unwrap();
            let multiline = js::with_dom(|d| d.tag(f) == Some("textarea"));
            let value = js::with_dom(|d| forms::value(d, f));
            let ed = fedit.get_or_insert_with(|| textedit::TextEdit::new(&value));
            ed.sync(&value);
            let r = edit_keys(ed, &edit_keys_now, &chars, ctrl_down, multiline, &mut clipboard);
            // A maxlength limit, counted in characters (only stops text from growing).
            if let Some(max) = js::with_dom(|d| d.attr(f, "maxlength").and_then(|m| m.trim().parse::<usize>().ok())) {
                let n = ed.len();
                if r.changed && n > max && n > value.chars().count() {
                    let keep = max.max(value.chars().count());
                    *ed = textedit::TextEdit::new(&ed.text.chars().take(keep).collect::<String>());
                }
            }
            let (text, caret, anchor) = (ed.text.clone(), ed.caret, ed.anchor);
            if text != value {
                js::with_dom_mut(|d| forms::set_value(d, f, &text));
                fire(&mut page, f, "input");
                form_changed = true;
            }
            if r.moved || r.changed {
                layout::set_edit(caret, anchor);
                reflow = true;
            }
            if r.enter {
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
            aedit.sync(&address);
            for k in edit_keys_now.iter().map(|p| p.key) {
                let n = suggestions.len();
                match k {
                    Key::Down if n > 0 => sugg_sel = Some(sugg_sel.map_or(0, |i| (i + 1).min(n - 1))),
                    Key::Up if n > 0 => sugg_sel = sugg_sel.and_then(|i| i.checked_sub(1)),
                    _ => {}
                }
                if matches!(k, Key::Up | Key::Down) {
                    dirty = true;
                }
            }
            let r = edit_keys(&mut aedit, &edit_keys_now, &chars, ctrl_down, false, &mut clipboard);
            address = aedit.text.clone();
            if r.changed {
                suggestions = library::suggest(&address, 6);
                sugg_sel = None;
            }
            if r.enter {
                editing = false;
                let picked = sugg_sel.and_then(|i| suggestions.get(i)).map(|s| s.url.clone());
                navigate = Some(Go::To(picked.unwrap_or_else(|| net::normalize_input(&address))));
                suggestions.clear();
            }
            if r.changed || r.moved || r.enter {
                dirty = true;
            }
            if escape {
                editing = false;
                address = page.url.clone();
                suggestions.clear();
                dirty = true;
            }
        } else {
            stop_key = escape;
            for p in &presses {
                let (key, ctrl, alt) = (p.key, p.ctrl, p.alt);
                let before = scroll;
                // Keys for the focused control come first.
                if let Some(f) = focused {
                    let kind = js::with_dom(|d| forms::control(d, f));
                    let handled = match key {
                        Key::Tab if !ctrl => {
                            let shift = p.shift;
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
                        aedit = textedit::TextEdit::select_all(&address);
                        dirty = true;
                    }
                    Key::F6 => {
                        editing = true;
                        aedit = textedit::TextEdit::select_all(&address);
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
            // About 45px per wheel click (minifb reports roughly 2 per click on X11).
            let delta = -dy * 22.0;
            // The wheel scrolls the innermost scrollable box under the mouse, else the page.
            let inner = window
                .get_mouse_pos(MouseMode::Discard)
                .filter(|&(_, my)| my >= BAR_H as f32)
                .and_then(|(mx, my)| lay.scroller_at(mx, my - BAR_H as f32, scroll, delta));
            match inner {
                Some((id, max)) => {
                    layout::set_element_scroll(id, (layout::element_scroll(id) + delta).clamp(0.0, max));
                    reflow = true;
                }
                None => {
                    scroll += delta;
                    dirty = true;
                }
            }
        }
        if let Some(y) = geometry::take_scroll_request() {
            scroll = y;
            dirty = true;
        }
        scroll = scroll.clamp(0.0, max_scroll);
        geometry::set_scroll(scroll);

        // Mouse: hover cursor and clicks.
        let down = window.get_mouse_down(MouseButton::Left);
        let mid = window.get_mouse_down(MouseButton::Middle);
        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) {
            let sh = (editing && my >= BAR_H as f32 && mx >= FIELD_X).then(|| ((my - BAR_H as f32) / SUGGEST_H) as usize).filter(|&i| i < suggestions.len());
            if sh != sugg_hover {
                sugg_hover = sh;
                dirty = true;
            }
            let th = tab_at(mx, my, w as f32, tabs.len());
            if th != tab_hover {
                tab_hover = th;
                dirty = true;
            }
            // Middle click: close a tab, or open a link in a background tab.
            if mid && !was_mid {
                match th {
                    Some(TabHit::Tab(i) | TabHit::Close(i)) => tab_cmd = Some(TabCmd::Close(i)),
                    _ if my >= BAR_H as f32 => {
                        if let Some(href) = lay.link_at(mx, my - BAR_H as f32, scroll) {
                            let target = net::resolve(&page.url, href);
                            if !target.starts_with("javascript:") && !target.starts_with("mailto:") {
                                tab_cmd = Some(TabCmd::Open(target, true));
                            }
                        }
                    }
                    _ => {}
                }
            }
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
            } else if my >= TAB_H && (my as usize) < BAR_H && mx >= FIELD_X {
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
            } else if down && !was_down && editing && !suggestions.is_empty() && my >= BAR_H as f32 && mx >= FIELD_X && my < BAR_H as f32 + suggestions.len() as f32 * SUGGEST_H {
                // A click on an address suggestion opens it.
                let i = ((my - BAR_H as f32) / SUGGEST_H) as usize;
                if let Some(sg) = suggestions.get(i) {
                    navigate = Some(Go::To(sg.url.clone()));
                }
                editing = false;
                suggestions.clear();
                dirty = true;
            } else if down && !was_down && my >= TAB_H && (my as usize) < BAR_H && mx >= w as f32 - 8.0 - STAR_W {
                library::toggle_bookmark(&page.url, &page.title);
                dirty = true;
            } else if down && !was_down && my < TAB_H {
                tab_cmd = match th {
                    Some(TabHit::Tab(i)) => Some(TabCmd::Switch(i)),
                    Some(TabHit::Close(i)) => Some(TabCmd::Close(i)),
                    Some(TabHit::New) => Some(TabCmd::Open("about:blank".into(), false)),
                    None => None,
                };
            } else if down && !was_down {
                if let Some(b) = btn {
                    editing = false;
                    navigate = Some(match b {
                        0 => Go::Back,
                        1 => Go::Forward,
                        _ => Go::Reload,
                    });
                } else if (my as usize) < BAR_H && mx >= FIELD_X {
                    let double = last_click.0.elapsed().as_millis() < 450 && (mx - last_click.1).abs() < 5.0 && (my - last_click.2).abs() < 5.0;
                    if !editing {
                        // First click selects the whole address, ready to type over.
                        editing = true;
                        aedit = textedit::TextEdit::select_all(&address);
                    } else {
                        let i = char_index_at(fonts, font::REGULAR, 14.0, &aedit.text, mx - FIELD_X - 8.0);
                        if double {
                            aedit.select_word_at(i);
                        } else {
                            aedit.move_to(i, shift_down);
                        }
                    }
                    dirty = true;
                } else if (my as usize) >= BAR_H {
                    if editing {
                        editing = false;
                        address = page.url.clone();
                        suggestions.clear();
                        dirty = true;
                    }
                    // Let the page's JavaScript see the click first.
                    let node = lay.hit_at(mx, view_y, scroll);
                    // Checkboxes and radio buttons change before the click event, so handlers
                    // (React's onChange, for one) see the new state; a cancelled click undoes it.
                    let toggled = node
                        .and_then(|n| js::with_dom(|d| control_for(d, n)))
                        .filter(|&c| js::with_dom(|d| matches!(forms::control(d, c), Some(forms::Control::Checkbox | forms::Control::Radio)) && !forms::is_disabled(d, c)));
                    let before: Vec<(dom::NodeId, bool)> = match toggled {
                        Some(_) => js::with_dom(|d| {
                            d.find_all("input").into_iter().map(|i| (i, d.attr(i, "checked").is_some())).collect()
                        }),
                        None => Vec::new(),
                    };
                    if let Some(c) = toggled {
                        js::with_dom_mut(|d| forms::toggle(d, c));
                    }
                    let allowed = match (node, page.js.as_mut()) {
                        (Some(n), Some(js)) => js.click(n),
                        _ => true,
                    };
                    if let Some(c) = toggled {
                        if allowed {
                            fire(&mut page, c, "input");
                            fire(&mut page, c, "change");
                        } else {
                            js::with_dom_mut(|d| {
                                for &(i, was) in &before {
                                    if was { d.set_attr(i, "checked", "") } else { d.remove_attr(i, "checked") }
                                }
                            });
                        }
                        form_changed = true;
                    }
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
                    // A click inside a single-line text field puts the caret where you clicked
                    // (double-click selects a word).
                    if let Some(c) = ctrl.filter(|&c| js::with_dom(|d| forms::is_editable(d, c) && d.tag(c) == Some("input"))) {
                        if let Some(r) = lay.hits.iter().rev().find(|(_, n)| *n == c).map(|(r, _)| *r) {
                            let st = page.styles[c].clone();
                            let fnt = font::resolve(st.family.as_deref(), st.bold, st.italic, st.mono);
                            let value = js::with_dom(|d| forms::value(d, c));
                            let shown = if js::with_dom(|d| d.attr(c, "type").is_some_and(|t| t.eq_ignore_ascii_case("password"))) { "•".repeat(value.chars().count()) } else { value.clone() };
                            let cx = r.x + st.border[3] + st.padding[3].resolve(r.w);
                            let i = char_index_at(fonts, fnt, st.font_size, &shown, mx - cx);
                            let double = last_click.0.elapsed().as_millis() < 450 && (mx - last_click.1).abs() < 5.0 && (my - last_click.2).abs() < 5.0;
                            let ed = fedit.get_or_insert_with(|| textedit::TextEdit::new(&value));
                            ed.sync(&value);
                            if double {
                                ed.select_word_at(i);
                            } else {
                                ed.move_to(i, shift_down && focused == Some(c));
                            }
                            layout::set_edit(ed.caret, ed.anchor);
                            form_changed = true;
                        }
                    }
                    if let (true, Some(c)) = (allowed, ctrl) {
                        // A click on a <label> is a click on its control.
                        if node.is_some_and(|n| n != c && js::with_dom(|d| !d.descendants(c).contains(&n))) {
                            fire(&mut page, c, "click");
                        }
                        match js::with_dom(|d| forms::control(d, c)) {
                            // Already changed before the click event (above).
                            Some(forms::Control::Checkbox | forms::Control::Radio) => {}
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
                        if !target.starts_with("mailto:") && !target.starts_with("javascript:") {
                            // Ctrl+click: background tab. target="_blank": new tab in front.
                            let ctrl_key = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
                            let blank = node.is_some_and(|n| {
                                js::with_dom(|d| {
                                    let mut cur = Some(n);
                                    while let Some(x) = cur {
                                        if d.tag(x) == Some("a") {
                                            return d.attr(x, "target").is_some_and(|t| t.eq_ignore_ascii_case("_blank"));
                                        }
                                        cur = d.nodes[x].parent;
                                    }
                                    false
                                })
                            });
                            if ctrl_key {
                                tab_cmd = Some(TabCmd::Open(target, true));
                            } else if blank {
                                tab_cmd = Some(TabCmd::Open(target, false));
                            } else {
                                navigate = Some(Go::To(target));
                            }
                        }
                    }
                    // window.open() during this click opens a new tab.
                    if let Some(u) = js::take_open_requests().pop() {
                        tab_cmd = Some(TabCmd::Open(u, false));
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
            if focus_moved {
                // A newly focused field starts with the caret at the end (a click may move it).
                fedit = focused.filter(|&f| js::with_dom(|d| forms::is_editable(d, f))).map(|f| textedit::TextEdit::new(&js::with_dom(|d| forms::value(d, f))));
            }
            match &fedit {
                Some(e) => layout::set_edit(e.caret, e.anchor),
                None => layout::clear_edit(),
            }
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
        if down && !was_down {
            if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) {
                last_click = (Instant::now(), mx, my);
            }
        }
        was_down = down;
        was_mid = mid;

        // The reload button (and Esc) stop a load in progress.
        if matches!(navigate, Some(Go::Reload)) && loading.is_some() || (stop_key && loading.is_some()) {
            navigate = None;
            loading = None;
            address = page.url.clone();
            window.set_title(&format!("{} — LighteningFox", if page.title.is_empty() { &page.url } else { &page.title }));
            dirty = true;
        }
        // History changes from scripts (pushState, location.hash, history.back…).
        let mut replace_entry = false;
        for op in js::take_history() {
            match op {
                js::HistoryOp::Push(url, state) => {
                    back.push(current.clone());
                    forward.clear();
                    current = Entry { url: url.clone(), doc: page.doc, state };
                    library::visit(&url, &page.title);
                    page.url = url;
                }
                js::HistoryOp::Replace(url, state) => {
                    current = Entry { url: url.clone(), doc: page.doc, state };
                    page.url = url;
                }
                js::HistoryOp::Hash(url, replace) => {
                    navigate = Some(Go::To(url));
                    replace_entry = replace;
                }
                js::HistoryOp::Go(n) if n < 0 => navigate = Some(Go::Back),
                js::HistoryOp::Go(n) if n > 0 => navigate = Some(Go::Forward),
                js::HistoryOp::Go(_) => navigate = Some(Go::Reload),
            }
            if !editing {
                address = page.url.clone();
            }
            dirty = true;
        }
        // Moves within the same document need no loading: a #fragment link, or Back/Forward
        // to an entry a script pushed. Scroll to the fragment and tell the page's scripts.
        let same_doc_move = match &navigate {
            Some(Go::To(t)) if t.contains('#') && same_document(t, &page.url) => {
                let e = Entry { url: t.clone(), doc: page.doc, state: "null".into() };
                if replace_entry {
                    current = e;
                } else if e.url != current.url {
                    back.push(std::mem::replace(&mut current, e));
                    forward.clear();
                }
                Some(false)
            }
            Some(Go::Back) if back.last().is_some_and(|e| e.doc == page.doc) => {
                let e = back.pop().unwrap();
                forward.push(std::mem::replace(&mut current, e));
                Some(true)
            }
            Some(Go::Forward) if forward.last().is_some_and(|e| e.doc == page.doc) => {
                let e = forward.pop().unwrap();
                back.push(std::mem::replace(&mut current, e));
                Some(true)
            }
            _ => None,
        };
        if let Some(popstate) = same_doc_move {
            navigate = None;
            let old = std::mem::replace(&mut page.url, current.url.clone());
            if let Some(js) = page.js.as_mut() {
                js.history_moved(&current.url, &current.state, popstate);
            }
            if !popstate || old.split('#').nth(1) != current.url.split('#').nth(1) {
                if let Some(y) = fragment_target(&current.url) {
                    scroll = y.clamp(0.0, (lay.height - h.saturating_sub(BAR_H) as f32).max(0.0));
                }
            }
            address = page.url.clone();
            editing = false;
            if js::take_dirty() {
                page.restyle();
                lay = page.layout(fonts, w as f32, h.saturating_sub(BAR_H) as f32);
            }
            dirty = true;
        }
        // Start a navigation: work out the target without touching history yet;
        // history changes only once the page actually arrives.
        let target = match &navigate {
            Some(Go::To(t)) | Some(Go::Post(t, _)) => Some(t.clone()),
            Some(Go::Back) => back.last().map(|e| e.url.clone()),
            Some(Go::Forward) => forward.last().map(|e| e.url.clone()),
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
            let Some(l) = loading.take_if(|l| l.generation == g) else {
                // A page for a background tab: keep it until the tab is shown.
                if let Some(slot) = tabs.iter_mut().flatten().find(|s| s.loading.as_ref().is_some_and(|l| l.generation == g)) {
                    slot.title = prepared_title(&prepared);
                    let l = slot.loading.take().expect("loading");
                    slot.pending = Some((l, prepared));
                    dirty = true;
                }
                continue;
            };
            let mut state = "null".to_string();
            match l.kind {
                Go::To(_) | Go::Post(..) => {
                    if prepared.url != current.url {
                        back.push(current.clone());
                        forward.clear();
                    }
                }
                Go::Back => {
                    if let Some(e) = back.pop() {
                        state = e.state;
                    }
                    forward.push(current.clone());
                }
                Go::Forward => {
                    if let Some(e) = forward.pop() {
                        state = e.state;
                    }
                    back.push(current.clone());
                }
                Go::Reload => state = current.state.clone(),
            }
            page = build_page(prepared);
            library::visit(&page.url, &page.title);
            current = Entry { url: page.url.clone(), doc: page.doc, state };
            focused = None;
            dropdown = None;
            layout::set_focus(None);
            release_free_memory();
            lay = page.layout(fonts, w as f32, h.saturating_sub(BAR_H) as f32);
            // A URL with a #fragment opens scrolled to it.
            scroll = fragment_target(&page.url).unwrap_or(0.0).clamp(0.0, (lay.height - h.saturating_sub(BAR_H) as f32).max(0.0));
            address = page.url.clone();
            window.set_title(&format!("{} — LighteningFox", page.label()));
            dirty = true;
        }
        // Once a second: put long-unseen background tabs to sleep, and save the session.
        if last_housekeeping.elapsed().as_secs() >= 1 {
            last_housekeeping = Instant::now();
            if let Some(after) = sleep_after() {
                let mut freed = false;
                for slot in tabs.iter_mut().flatten() {
                    if slot.saved.is_some() && slot.loading.is_none() && slot.pending.is_none() && slot.last_seen.elapsed() >= after {
                        let sv = slot.saved.take().expect("saved");
                        if let Some(j) = sv.page.js.as_ref() {
                            js::forget(j);
                        }
                        slot.url = sv.current.url.clone();
                        slot.back = sv.back;
                        slot.forward = sv.forward;
                        freed = true;
                    }
                }
                if freed {
                    eprintln!("[tabs] put background tabs to sleep");
                    release_free_memory();
                    dirty = true;
                }
            }
            let urls: Vec<(String, String)> = tabs
                .iter()
                .map(|s| match s {
                    None => (current.url.clone(), page.label()),
                    Some(s) => (s.saved.as_ref().map_or(s.url.clone(), |sv| sv.current.url.clone()), s.title.clone()),
                })
                .collect();
            if (urls.clone(), active) != session_saved {
                save_session(&urls, active);
                session_saved = (urls, active);
            }
        }
        // Animate the loading bar about ten times a second.
        let any_loading = loading.is_some() || tabs.iter().flatten().any(|s| s.loading.is_some());
        if any_loading && last_spin.elapsed().as_millis() >= 100 {
            last_spin = Instant::now();
            dirty = true;
        }

        // Idle: poll less often. Anything happening (input, redraws, loading) goes back to 60 fps.
        let busy = dirty || animating || any_loading || js::network_busy() || window.get_mouse_down(MouseButton::Left) || !window.get_keys().is_empty();
        if busy {
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
            let labels: Vec<(String, bool)> = tabs
                .iter()
                .enumerate()
                .map(|(i, s)| match s {
                    None if i == active => (page.label(), loading.is_some()),
                    Some(s) => (s.title.clone(), s.loading.is_some()),
                    None => (String::new(), false),
                })
                .collect();
            let ui = Toolbar { address: &address, editing, can_back: !back.is_empty(), can_forward: !forward.is_empty(), hover, loading: spin, tabs: &labels, active, tab_hover, caret: editing.then_some((aedit.caret, aedit.anchor)), bookmarked: library::is_bookmarked(&page.url) };
            draw(&mut canvas, fonts, &page.images, &lay, scroll, &ui);
            if let Some(dd) = &dropdown {
                draw_dropdown(&mut canvas, fonts, dd, scroll);
            }
            if editing && !suggestions.is_empty() {
                draw_suggestions(&mut canvas, fonts, &suggestions, sugg_sel.or(sugg_hover));
            }
            let _ = window.update_with_buffer(&canvas.buf, w, h);
            dirty = false;
        } else {
            window.update();
        }
    }
}

struct Toolbar<'a> {
    tabs: &'a [(String, bool)],
    active: usize,
    tab_hover: Option<TabHit>,
    /// While editing the address: (caret, selection anchor).
    caret: Option<(usize, usize)>,
    bookmarked: bool,
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
    let top = TAB_H;
    // Tab strip
    canvas.fill_rect(0.0, 0.0, w, top, Color::rgb(222, 222, 228));
    let toolbar_bg = Color::rgb(236, 236, 240);
    for (i, ((label, loading), &(tx, tw))) in ui.tabs.iter().zip(tab_rects(w, ui.tabs.len()).iter()).enumerate() {
        let is_active = i == ui.active;
        let hovered = matches!(ui.tab_hover, Some(TabHit::Tab(j) | TabHit::Close(j)) if j == i);
        let bg = if is_active { toolbar_bg } else if hovered { Color::rgb(232, 232, 238) } else { Color::rgb(222, 222, 228) };
        canvas.fill_rect(tx, 5.0, tw - 2.0, top - 5.0, bg);
        if !is_active && i + 1 != ui.active && i + 1 < ui.tabs.len() {
            canvas.fill_rect(tx + tw - 2.0, 11.0, 1.0, top - 18.0, Color::rgb(190, 190, 198));
        }
        // A small orange dot while the tab loads.
        let mut text_x = tx + 10.0;
        if *loading {
            canvas.fill_rect(tx + 9.0, 17.0, 6.0, 6.0, Color::rgb(249, 115, 22));
            text_x += 12.0;
        }
        // Fit the title, leaving room for the ×.
        let room = (tw - (text_x - tx) - 30.0).max(0.0);
        let mut title: String = label.chars().filter(|c| !c.is_control()).collect();
        if fonts.width(font::REGULAR, &title, 13.0) > room {
            while !title.is_empty() && fonts.width(font::REGULAR, &format!("{title}…"), 13.0) > room {
                title.pop();
            }
            title.push('…');
        }
        let ink = if is_active { Color::rgb(30, 30, 40) } else { Color::rgb(80, 80, 92) };
        canvas.draw_text(fonts, text_x, 24.0, &title, font::REGULAR, 13.0, ink);
        if tw > 60.0 {
            let close_hot = ui.tab_hover == Some(TabHit::Close(i));
            if close_hot {
                canvas.fill_rect(tx + tw - 24.0, 10.0, 18.0, 18.0, Color::rgb(205, 205, 212));
            }
            canvas.draw_text(fonts, tx + tw - 20.0, 24.0, "×", font::REGULAR, 15.0, if close_hot || is_active || hovered { ink } else { Color::rgb(140, 140, 150) });
        }
    }
    let end = tab_rects(w, ui.tabs.len()).last().map_or(6.0, |&(tx, tw)| tx + tw);
    if ui.tab_hover == Some(TabHit::New) {
        canvas.fill_rect(end + 4.0, 7.0, NEW_TAB_W, top - 12.0, Color::rgb(205, 205, 212));
    }
    canvas.draw_text(fonts, end + 4.0 + (NEW_TAB_W - fonts.width(font::REGULAR, "+", 18.0)) / 2.0, 26.0, "+", font::REGULAR, 18.0, Color::rgb(60, 60, 70));

    // Toolbar
    canvas.fill_rect(0.0, top, w, bar - top, toolbar_bg);
    canvas.fill_rect(0.0, bar - 1.0, w, 1.0, Color::rgb(200, 200, 205));

    // Back, forward, reload
    let enabled = [ui.can_back, ui.can_forward, true];
    for (i, (bx, label)) in BUTTONS.iter().enumerate() {
        if ui.hover == Some(i) && enabled[i] {
            canvas.fill_rect(*bx, top + 4.0, BTN_W, TOOL_H - 8.0, Color::rgb(214, 214, 222));
        }
        let ink = if enabled[i] { Color::rgb(40, 40, 50) } else { Color::rgb(170, 170, 178) };
        let size = 18.0;
        // While loading, reload becomes stop.
        let label = if i == 2 && ui.loading.is_some() { &"✕" } else { label };
        let tw = fonts.width(font::BOLD, label, size);
        canvas.draw_text(fonts, bx + (BTN_W - tw) / 2.0, top + 24.0, label, font::BOLD, size, ink);
    }

    // Address field
    let field = if ui.editing { Color::rgb(255, 255, 255) } else { Color::rgb(248, 248, 250) };
    canvas.fill_rect(FIELD_X, top + 5.0, w - FIELD_X - 8.0, TOOL_H - 10.0, field);
    // Bookmark star
    let (star, star_ink) = if ui.bookmarked { ("★", Color::rgb(234, 179, 8)) } else { ("☆", Color::rgb(110, 110, 120)) };
    canvas.draw_text(fonts, w - 8.0 - STAR_W + 7.0, top + 24.0, star, font::REGULAR, 17.0, star_ink);
    let tx = FIELD_X + 8.0;
    if let Some((caret, anchor)) = ui.caret {
        let upto = |i: usize| fonts.width(font::REGULAR, &ui.address.chars().take(i).collect::<String>(), 14.0);
        if caret != anchor {
            let (a, b) = (upto(caret.min(anchor)), upto(caret.max(anchor)));
            canvas.fill_rect(tx + a, top + 9.0, b - a, TOOL_H - 18.0, Color::rgb(180, 213, 254));
        }
        canvas.fill_rect((tx + upto(caret)).round(), top + 9.0, 1.0, TOOL_H - 18.0, Color::rgb(30, 30, 40));
    }
    canvas.draw_text(fonts, tx, top + 23.0, ui.address, font::REGULAR, 14.0, Color::rgb(40, 40, 50));
    // Loading bar: a segment sliding along the bottom of the toolbar.
    if let Some(t) = ui.loading {
        let span = w * 0.25;
        let pos = (t * 0.8).fract() * (w + span) - span;
        canvas.fill_rect(pos.max(0.0), bar - 3.0, (pos + span).min(w) - pos.max(0.0), 2.0, Color::rgb(249, 115, 22));
    }
}

/// Address bar suggestions: a panel under the field, one row per page (title, then address).
fn draw_suggestions(canvas: &mut Canvas, fonts: &Fonts, list: &[library::Suggestion], hot: Option<usize>) {
    let (x, y0) = (FIELD_X, BAR_H as f32);
    let w = canvas.w as f32 - FIELD_X - 8.0;
    let h = list.len() as f32 * SUGGEST_H;
    canvas.fill_rect(x + 2.0, y0 + 2.0, w, h + 2.0, Color { r: 0, g: 0, b: 0, a: 35 });
    canvas.fill_rect(x, y0, w, h + 2.0, Color::rgb(200, 200, 208));
    canvas.fill_rect(x + 1.0, y0, w - 2.0, h + 1.0, Color::rgb(255, 255, 255));
    for (i, sg) in list.iter().enumerate() {
        let y = y0 + i as f32 * SUGGEST_H;
        if hot == Some(i) {
            canvas.fill_rect(x + 1.0, y, w - 2.0, SUGGEST_H, Color::rgb(232, 240, 254));
        }
        let icon = if sg.bookmarked { "★" } else { "↺" };
        canvas.draw_text(fonts, x + 10.0, y + 20.0, icon, font::REGULAR, 13.0, if sg.bookmarked { Color::rgb(234, 179, 8) } else { Color::rgb(140, 140, 150) });
        let title = if sg.title.is_empty() { &sg.url } else { &sg.title };
        let mut t: String = title.chars().take(70).collect();
        if t.len() < title.len() {
            t.push('…');
        }
        canvas.draw_text(fonts, x + 32.0, y + 20.0, &t, font::REGULAR, 13.5, Color::rgb(30, 30, 40));
        let tx = x + 32.0 + fonts.width(font::REGULAR, &t, 13.5) + 12.0;
        let room = (x + w - 12.0 - tx).max(0.0);
        let mut u = sg.url.clone();
        while !u.is_empty() && fonts.width(font::REGULAR, &u, 12.0) > room {
            u.pop();
        }
        canvas.draw_text(fonts, tx, y + 20.0, &u, font::REGULAR, 12.0, Color::rgb(29, 78, 216));
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
        ("tests/geometry.html", 700),
        ("tests/sticky.html", 1024),
        ("tests/scroll-box.html", 900),
        ("tests/transforms.html", 900),
        ("tests/history.html", 800),
        ("tests/webfonts.html", 900),
        ("tests/real-world/react.html", 1024),
        ("tests/real-world/vue.html", 1024),
        ("tests/real-world/jquery.html", 1024),
        ("tests/real-world/bootstrap.html", 1024),
    ];

    fn read_png(path: &str) -> Option<(usize, usize, Vec<u8>)> {
        let img = image::open(path).ok()?.to_rgb8();
        Some((img.width() as usize, img.height() as usize, img.into_raw()))
    }

    #[test]
    fn pages_match_reference_images() {
        // Keep test runs out of the real profile (cookies, localStorage), starting fresh each time.
        let profile = std::env::temp_dir().join(format!("lighteningfox-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        // SAFETY: set before any other thread starts in this test.
        unsafe { std::env::set_var("LF_PROFILE", &profile) };
        let fonts = load_fonts().expect("fonts");
        let update = std::env::var("UPDATE_REFERENCES").is_ok();
        std::fs::create_dir_all("tests/reference").unwrap();
        std::fs::create_dir_all("target/screenshot-diffs").unwrap();
        let mut failures = Vec::new();
        for (url, width) in PAGES {
            let name = url.trim_start_matches("tests/").trim_end_matches(".html").replace([':', '/'], "-");
            let reference = format!("tests/reference/{name}.png");
            let (_, _, canvas) = render_page(fonts, url, *width);
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
