//! Ironswift — a web browser engine written from scratch in Rust.
//!
//! Pipeline: fetch -> HTML parse (DOM) -> CSS parse -> style cascade -> layout -> paint.
//!
//! Usage:
//!   ironswift [URL or file]                  open a window
//!   ironswift URL --png out.png [--width N]  render to an image (no window needed)

/// Print a message to the terminal and keep it in the debug log (about:log).
macro_rules! log {
    ($($t:tt)*) => {{
        let line = format!($($t)*);
        eprintln!("{line}");
        crate::crashlog::record(line);
    }};
}

mod clicks;
mod cookies;
mod netbody;
mod bindings;
mod css;
mod csssyntax;
mod restyle;
mod selmatch;
mod shorthand;
mod cascade;
mod dom;
mod qname;
mod editing;
mod font;
mod filepicker;
mod forms;
mod formui;
mod geometry;
// WebGL (T12) is still being built: its unused fields and helpers are allowed until it's
// finished, then these allows come off (see R3 in docs/TASKS.md).
mod webgl;
mod webgl_raster;
mod webgl_tex;
mod glsl;
mod glsl_gen;
mod glsl_vm;
mod grid;
mod html;
mod http;
mod record;
mod dynscripts;
mod livesheets;
mod hsts;
mod tokenizer;
mod a11y;
mod autofill;
mod i18n;
mod keynav;
mod rtl;
mod uikeys;
mod chrome;
mod favicon;
mod devtools;
mod pdf;
mod treebuilder;
mod zoom;
mod effects;
mod anim;
mod background;
mod gradient;
mod smooth;
mod window;
mod images;
mod img_load;
mod srcset;
mod page_images;
mod js;
mod jsnest;
mod jsmodules;
mod importmap;
mod extensions;
mod webcrypto;
mod platform;
mod domindex;
mod dom_classes;
mod vertical;
mod memory;
mod userinput;
#[cfg(test)]
mod jsapi_tests;
mod layout;
mod doc_extent;
mod cssvars;
mod library;
mod cache;
mod canvas;
mod media;
mod mse;
mod mse_frames;
mod channels;
mod geo;
mod parallel;
mod preshape;
mod clipboard;
mod downloads;
mod entities;
mod conformance;
mod crashlog;
mod settings;
mod contrast;
mod a11y_bridge;
mod security;
mod sysfonts;
mod text;
mod blocklist;
mod permissions;
mod updates;
mod websocket;
mod ws_relay;
mod worker;
mod sandbox;
mod frames;
mod process;
mod tiles;
#[cfg(feature = "gpu")]
mod gpu;
mod net;
mod paint;
mod style;
mod boxtree;
mod relayout;
mod svg_inline;
mod svg_hit;
mod svg_foreign;
mod site_snapshot;
mod pseudo;
mod textedit;
mod transform;
mod validation;

use font::Fonts;
use window::{CursorStyle, Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
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
/// Bottom of the toolbar; the bookmarks bar (when shown) sits under it, then the page.
const TOOL_BOTTOM: usize = (TAB_H + TOOL_H) as usize;
const BOOK_H: f32 = 28.0;
static BOOKMARKS_BAR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Where the page starts: below the toolbar and the bookmarks bar, if it's showing.
fn bar_h() -> usize {
    TOOL_BOTTOM + if BOOKMARKS_BAR.load(std::sync::atomic::Ordering::Relaxed) { BOOK_H as usize } else { 0 }
}

/// What's on the bookmarks bar, in order: loose bookmarks, and each folder where its first
/// bookmark is.
#[derive(Clone, PartialEq, Debug)]
enum BarItem {
    Link(usize),
    Folder(String),
}

fn bar_items(list: &[library::Bookmark]) -> Vec<BarItem> {
    let mut out = Vec::new();
    for (i, b) in list.iter().enumerate() {
        if b.folder.is_empty() {
            out.push(BarItem::Link(i));
        } else if !out.contains(&BarItem::Folder(b.folder.clone())) {
            out.push(BarItem::Folder(b.folder.clone()));
        }
    }
    out
}

fn bar_label(fonts: &Fonts, list: &[library::Bookmark], item: &BarItem) -> String {
    match item {
        BarItem::Link(i) => bookmark_label(fonts, &list[*i]),
        BarItem::Folder(f) => format!("{} ▾", fit_label(fonts, f)),
    }
}

/// Bookmarks-bar chips that fit in width `w`: (x, width) each; the rest go behind "»".
const BOOK_MORE_W: f32 = 30.0;
fn bookmark_rects(fonts: &Fonts, w: f32, list: &[library::Bookmark]) -> Vec<(f32, f32)> {
    let items = bar_items(list);
    let mut out = Vec::new();
    let mut x = 8.0;
    for (i, item) in items.iter().enumerate() {
        let cw = fonts.width(font::REGULAR, &bar_label(fonts, list, item), 12.5) + 20.0;
        // Leave room for "»" unless this is the last one.
        let limit = if i + 1 == items.len() { w - 8.0 } else { w - 8.0 - BOOK_MORE_W };
        if x + cw > limit {
            break;
        }
        out.push((x, cw));
        x += cw + 2.0;
    }
    out
}

fn fit_label(fonts: &Fonts, s: &str) -> String {
    let mut t: String = s.trim().chars().filter(|c| !c.is_control()).collect();
    if fonts.width(font::REGULAR, &t, 12.5) > 150.0 {
        while !t.is_empty() && fonts.width(font::REGULAR, &format!("{t}…"), 12.5) > 150.0 {
            t.pop();
        }
        t.push('…');
    }
    t
}

fn bookmark_label(fonts: &Fonts, b: &library::Bookmark) -> String {
    fit_label(fonts, if b.title.trim().is_empty() { &b.url } else { &b.title })
}

/// An open bookmarks-bar folder: its name, the chip's x, and the row under the mouse.
struct FolderMenu {
    folder: String,
    x: f32,
    hover: Option<usize>,
}
const MENU_ROW_H: f32 = 26.0;
const MENU_W: f32 = 280.0;

impl FolderMenu {
    fn entries<'a>(&self, list: &'a [library::Bookmark]) -> Vec<&'a library::Bookmark> {
        list.iter().filter(|b| b.folder == self.folder).collect()
    }
    /// The menu's left edge, kept inside the window.
    fn left(&self, w: f32) -> f32 {
        self.x.min(w - MENU_W - 4.0).max(0.0)
    }
    fn row_at(&self, list: &[library::Bookmark], w: f32, mx: f32, my: f32) -> Option<usize> {
        let (x, y) = (self.left(w), bar_h() as f32);
        let n = self.entries(list).len();
        (mx >= x && mx < x + MENU_W && my >= y + 4.0 && my < y + 4.0 + n as f32 * MENU_ROW_H).then(|| ((my - y - 4.0) / MENU_ROW_H) as usize)
    }
}

fn draw_folder_menu(canvas: &mut Canvas, fonts: &Fonts, menu: &FolderMenu, list: &[library::Bookmark]) {
    let w = canvas.w as f32;
    let entries = menu.entries(list);
    let (x, y) = (menu.left(w), bar_h() as f32);
    let hgt = entries.len() as f32 * MENU_ROW_H + 8.0;
    canvas.fill_rect(x + 2.0, y + 2.0, MENU_W, hgt, Color::rgb(190, 190, 198));
    canvas.fill_rect(x, y, MENU_W, hgt, Color::rgb(255, 255, 255));
    canvas.fill_rect(x, y, MENU_W, 1.0, Color::rgb(210, 210, 216));
    for (i, b) in entries.iter().enumerate() {
        let ry = y + 4.0 + i as f32 * MENU_ROW_H;
        if menu.hover == Some(i) {
            canvas.fill_rect(x + 4.0, ry, MENU_W - 8.0, MENU_ROW_H, Color::rgb(224, 231, 255));
        }
        let mut t = bookmark_label(fonts, b);
        while fonts.width(font::REGULAR, &t, 13.0) > MENU_W - 40.0 && t.chars().count() > 1 {
            t.pop();
        }
        canvas.draw_text(fonts, x + 12.0, ry + 18.0, &format!("★ {t}"), font::REGULAR, 13.0, Color::rgb(35, 35, 45));
    }
}
const TAB_MAX_W: f32 = 220.0;
const NEW_TAB_W: f32 = 32.0;
/// The bookmark star at the right end of the address field, and the suggestion rows below it.
const STAR_W: f32 = 30.0;
/// The downloads chip, left of the star, while something is downloading.
const DL_W: f32 = 64.0;
/// The content-blocking shield, left of the downloads chip (or the star): how many requests
/// were blocked on this page; a click opens its menu.
const SHIELD_W: f32 = 46.0;

/// Where the shield starts (it ends where the downloads chip or the star begins).
fn shield_x(w: f32, downloads: bool) -> f32 {
    w - 8.0 - STAR_W - if downloads { DL_W } else { 0.0 } - SHIELD_W
}
const SUGGEST_H: f32 = 30.0;
/// Toolbar buttons: (x, label). Each is BTN_W wide.
const BTN_W: f32 = 30.0;
const BUTTONS: [(f32, &str); 3] = [(6.0, "←"), (38.0, "→"), (70.0, "↻")];
const FIELD_X: f32 = 108.0;

/// Where a navigation should go and how it affects history.
enum Go {
    To(String),
    /// Submit a form with POST: (url, urlencoded body)
    Post(String, forms::PostBody),
    Back,
    Forward,
    Reload,
}

/// Which toolbar button (0 back, 1 forward, 2 reload) is under (x, y), if any.
fn button_at(x: f32, y: f32) -> Option<usize> {
    if y < TAB_H || y >= TOOL_BOTTOM as f32 {
        return None;
    }
    BUTTONS.iter().position(|(bx, _)| x >= *bx && x < bx + BTN_W)
}
/// Everything in the browser UI the keyboard can reach (uikeys.rs), in order, as boxes in
/// left-to-right UI coordinates: a permission prompt's buttons, back, forward, reload, the
/// downloads chip, the star, the bookmarks, the tabs and the new-tab button.
fn ui_targets(fonts: &Fonts, w: f32, tabs: usize, bookmarks: &[library::Bookmark], downloads: bool, prompt: Option<&permissions::Prompt>) -> Vec<uikeys::Target> {
    let mut t: Vec<uikeys::Target> = Vec::new();
    if let Some(p) = prompt {
        t.extend(p.button_boxes().into_iter().map(|b| (b.0, b.1, b.2, b.3)));
    }
    for (bx, _) in BUTTONS {
        t.push((bx, TAB_H + 4.0, BTN_W, TOOL_H - 8.0));
    }
    if downloads {
        t.push((w - 8.0 - STAR_W - DL_W, TAB_H + 4.0, DL_W, TOOL_H - 8.0));
    }
    t.push((w - 8.0 - STAR_W, TAB_H + 4.0, STAR_W, TOOL_H - 8.0));
    if !bookmarks.is_empty() {
        let rects = bookmark_rects(fonts, w, bookmarks);
        let more = rects.len() < bar_items(bookmarks).len();
        t.extend(rects.into_iter().map(|(x, cw)| (x, TOOL_BOTTOM as f32 + 2.0, cw, BOOK_H - 6.0)));
        if more {
            t.push((w - 8.0 - BOOK_MORE_W, TOOL_BOTTOM as f32 + 2.0, BOOK_MORE_W, BOOK_H - 6.0));
        }
    }
    let rects = tab_rects(w, tabs);
    let end = rects.last().map_or(6.0, |&(tx, tw)| tx + tw);
    t.extend(rects.into_iter().map(|(tx, tw)| (tx, 5.0, (tw - 2.0).max(1.0), TAB_H - 5.0)));
    t.push((end + 4.0, 7.0, NEW_TAB_W, TAB_H - 12.0));
    t
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

/// How long a background tab may go unseen before it sleeps (IRONSWIFT_TAB_SLEEP_SECONDS overrides).
fn sleep_after() -> Option<std::time::Duration> {
    if let Some(secs) = std::env::var("IRONSWIFT_TAB_SLEEP_SECONDS").ok().and_then(|v| v.parse::<u64>().ok()) {
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
            icon: None,
            title: String::new(),
            sheets: Vec::new(),
            styles: Vec::new(),
            images: images::Images::default(),
            js: None,
            hover_rules: false,
            doc: 0,
            form_defaults: Vec::new(),
            loaded_at: Instant::now(),
            styled_for: (0.0, 0.0),
            live: Rc::new(RefCell::new(livesheets::Live::new("about:blank", Default::default(), &dom::Dom::new(), Vec::new()))),
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

/// The window runs natively on Wayland (winit). IRONSWIFT_X11=1 uses X11 (XWayland) instead,
/// for desktops where that works better.
fn prefer_x11() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    if wayland && x11 && std::env::var("IRONSWIFT_X11").is_ok_and(|v| v == "1") {
        // SAFETY: called first thing in main, before any other thread exists.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
}

/// The start page, with the app icon put in as a data: URL.
fn start_page() -> String {
    use base64::Engine;
    let icon = base64::engine::general_purpose::STANDARD.encode(include_bytes!("../assets/icon/png/ironswift-128.png"));
    START_PAGE.replace("{{ICON}}", &format!("data:image/png;base64,{icon}"))
}

struct Page {
    url: String,
    /// Where the site's icon is (favicon.rs), for the tab strip.
    icon: Option<String>,
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
    /// When it loaded (developer tools show what happened since).
    loaded_at: Instant,
    /// The window size (CSS px) its styles were computed for, for @media rules.
    styled_for: (f32, f32),
    /// Its stylesheets as scripts add and change them (livesheets.rs).
    live: Rc<RefCell<livesheets::Live>>,
}

impl Page {
    /// Re-run the cascade after the DOM changed.
    fn restyle(&mut self) {
        // <style> and <link> elements scripts added or changed.
        if js::with_dom(|d| self.live.borrow_mut().refresh_for_page(d)) {
            self.sheets = (*self.live.borrow().sheets()).clone();
            self.hover_rules = style::uses_hover(&self.sheets);
        }
        if self.styled_for.0 > 0.0 {
            css::set_viewport(self.styled_for.0, self.styled_for.1);
        }
        anim::set_current(self.anim_key());
        self.styles = js::with_dom(|d| match d.composed() {
            Some(c) => style::compute(&c, &self.sheets),
            None => style::compute(d, &self.sheets),
        });
        let bg_urls = background_urls(&self.styles);
        let (images, url) = (&mut self.images, &self.url);
        js::with_dom(|d| images.load_missing(d, url, &bg_urls));
        self.title = js::with_dom(|d| d.find_all("title").first().map(|&t| d.text_content(t).trim().to_string()).unwrap_or_default());
    }

    /// Lay the page out for a window `width`×`viewport_h` device pixels. At a zoom other than
    /// 100% the page is laid out narrower (in CSS pixels) and drawn scaled up, so it reflows
    /// like in other browsers; scripts see CSS pixels.
    fn layout(&mut self, fonts: &Fonts, width: f32, viewport_h: f32) -> layout::Layout {
        // Page zoom times the screen's HiDPI scale: device pixels per CSS pixel.
        let z = zoom::get(&self.url) * zoom::device_scale();
        zoom::set_pixel_ratio(z);
        let (cw, ch) = (width / z, viewport_h / z);
        // @media rules follow the window: a new size can mean different styles.
        if (cw, ch) != self.styled_for {
            self.styled_for = (cw, ch);
            css::set_viewport(cw, ch);
            if self.sheets.iter().any(css::depends_on_viewport) || js::with_dom(css::inline_viewport_units) {
                self.restyle();
            }
        }
        if let Some(js) = &self.js {
            js.publish_canvases(&mut self.images);
            canvas::set_images(self.images.share());
        }
        js::with_dom(|d| svg_inline::publish(d.composed().as_ref().unwrap_or(d), &self.styles, &mut self.images));
        let mut lay = js::with_dom(|d| match d.composed() {
            Some(c) => layout::layout(&c, &self.styles, fonts, &self.images, cw, ch),
            None => layout::layout(d, &self.styles, fonts, &self.images, cw, ch),
        });
        // @container rules depend on the containers' laid-out sizes: style and lay out again
        // when those changed.
        if self.sheets.iter().any(|s| s.rules.iter().any(|r| r.container.is_some())) && cascade::record(&self.styles, &lay.boxes) {
            self.restyle();
            lay = js::with_dom(|d| match d.composed() {
                Some(c) => layout::layout(&c, &self.styles, fonts, &self.images, cw, ch),
                None => layout::layout(d, &self.styles, fonts, &self.images, cw, ch),
            });
            cascade::record(&self.styles, &lay.boxes);
        }
        geometry::publish(&lay, cw, ch);
        if (z - 1.0).abs() < 1e-4 { lay } else { lay.scaled(z) }
    }

    /// This page's CSS animation timeline.
    fn anim_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.loaded_at.hash(&mut h);
        h.finish()
    }

    fn zoom(&self) -> f32 {
        zoom::get(&self.url)
    }

    /// Device pixels per CSS pixel (page zoom × HiDPI scale).
    fn scale(&self) -> f32 {
        self.zoom() * zoom::device_scale()
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
    /// The page's HTML, parsed again with scripts running if it has any.
    source: String,
    /// Only the part of the page downloaded so far (shown while the rest arrives; no scripts).
    partial: bool,
    /// An XHTML document, parsed as XML.
    xml: bool,
    /// The navigation turned out to be a download: there is no page to show.
    download: bool,
    /// The page's Content Security Policy and framing rules.
    policy: std::sync::Arc<security::PagePolicy>,
}

/// A downloaded @font-face font. `data` is None when an earlier page already loaded it.
struct WebFont {
    family: String,
    weight: (u16, u16),
    italic: bool,
    url: String,
    data: Option<Vec<u8>>,
    /// The page that asked for it (to fetch it after all, see `install_fonts`).
    page_url: String,
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
    // Fonts split by unicode-range (Google Fonts and many sites serve a family as several
    // subsets: latin, latin-ext, cyrillic…): faces aren't chosen per character yet, so for each
    // family, weight and style the subset with basic Latin is used and the others are skipped.
    let faces: Vec<&css::FontFace> = sheets.iter().flat_map(|s| &s.font_faces).filter(|f| used.contains(&f.family)).collect();
    let has_latin = |f: &css::FontFace| faces.iter().any(|g| g.family == f.family && g.weight == f.weight && g.italic == f.italic && g.covers('a'));
    let wanted: Vec<(&css::FontFace, String)> = faces
        .iter()
        .copied()
        .filter(|f| f.covers('a') || !has_latin(f))
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
                        match net::fetch_sub_bytes(url, page_url, security::Dest::Font) {
                            Ok((_, b)) => decode_font(b),
                            Err(e) => {
                                log!("[font] {url}: {e}");
                                return None;
                            }
                        }
                    };
                    Some(WebFont { family: face.family.clone(), weight: face.weight, italic: face.italic, url: url.clone(), data, page_url: page_url.to_string() })
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
            // Skipped because a page on another thread loaded it, but font ids belong to each
            // thread's registry (tests style pages on many threads): fetch it here after all.
            (None, None) => net::fetch_sub_bytes(&f.url, &f.page_url, security::Dest::Font)
                .ok()
                .and_then(|(_, b)| decode_font(b))
                .and_then(|data| registry.add_web_font(data, f.weight.0 >= 600, f.italic))
                .inspect(|&id| {
                    FONT_IDS.with(|m| m.borrow_mut().insert(f.url.clone(), id));
                }),
        };
        match id {
            Some(id) => families.entry(f.family).or_default().push((f.weight.0, f.weight.1, f.italic, id)),
            None => log!("[font] {}: not a usable font", f.url),
        }
    }
    font::set_web_families(families);
}

/// Fetch the page, its stylesheets, scripts and images, all in parallel where possible.
/// Runs on a worker thread; touches nothing the UI thread owns.
fn fetch_page(url: &str, post: Option<&forms::PostBody>) -> Prepared {
    fetch_page_with(url, post, &mut |_| {})
}

/// `fetch_page`, also handing over previews of a slow page while it downloads.
fn fetch_page_with(url: &str, post: Option<&forms::PostBody>, preview: &mut dyn FnMut(Prepared)) -> Prepared {
    let mut xml = false;
    let mut headers: Vec<(String, String)> = Vec::new();
    let (final_url, body) = if url == "about:home" || url.is_empty() {
        ("about:home".to_string(), start_page())
    } else if url == "about:blank" {
        ("about:blank".to_string(), String::new())
    } else if let Some(q) = url.strip_prefix("about:history") {
        ("about:history".to_string(), library::history_page(q))
    } else if let Some(q) = url.strip_prefix("about:log") {
        ("about:log".to_string(), crashlog::log_page(q))
    } else if let Some(q) = url.strip_prefix("about:crashes") {
        ("about:crashes".to_string(), crashlog::crashes_page(q))
    } else if url == "about:crash" {
        // Deliberately crash, to test crash reporting (like chrome://crash).
        panic!("about:crash was opened (a deliberate test crash)");
    } else if let Some(q) = url.strip_prefix("about:settings") {
        ("about:settings".to_string(), settings::page(q))
    } else if let Some(q) = url.strip_prefix("about:downloads") {
        ("about:downloads".to_string(), downloads::page(q))
    } else if url.starts_with("about:devtools") {
        ("about:devtools".to_string(), devtools::page())
    } else if let Some(t) = url.strip_prefix("view-source:") {
        (url.to_string(), net::source_page(t))
    } else if let Some(q) = url.strip_prefix("about:bookmarks") {
        ("about:bookmarks".to_string(), library::bookmarks_page(q))
    } else if let Some(q) = url.strip_prefix("about:blocked?proceed=") {
        // "Visit this site anyway" on the dangerous-site warning.
        let target = net::percent_decode_str(q);
        blocklist::proceed(&target);
        return fetch_page_with(&target, None, preview);
    } else if blocklist::dangerous(url) {
        log!("[safe browsing] {url} is on a malware or phishing list, or a filter blocks it");
        (format!("about:blocked?url={}", url), blocklist::warning_page(url))
    } else if let Some(shell) = process::shell_for(url, post) {
        // Site processes on: the page itself loads and runs in its site's process; here it's
        // just the frame that shows it.
        return prepare(shell.0, shell.1, true, false, &[]);
    } else {
        let target = if post.is_some() { url.to_string() } else { net::strip_tracking(url) };
        let mut on_partial = |u: &str, html: &str, h: &[(String, String)]| {
            let mut p = prepare(u.to_string(), html.to_string(), false, false, h);
            p.partial = true;
            preview(p);
        };
        match net::navigate_with(&target, post, &mut on_partial) {
            Ok(net::Navigated::Page { url, html, xml: x, headers: h }) => {
                xml = x;
                headers = h;
                (url, html)
            }
            // A file being saved: the tab stays on the page it was showing.
            Ok(net::Navigated::Download { name }) => {
                log!("[download] saving {name}");
                return Prepared { url: url.to_string(), dom: dom::Dom::new(), sheets: Vec::new(), images: images::Images::default(), scripts: HashMap::new(), fonts: Vec::new(), source: String::new(), partial: false, xml: false, download: true, policy: Default::default() };
            }
            Err(e) => {
                log!("[net] couldn't load {url}: {e}");
                (url.to_string(), error_page(url, &e))
            }
        }
    };
    prepare(final_url, body, true, xml, &headers)
}

/// Parse a page and fetch what it needs: stylesheets and scripts, and with `full`, images
/// and web fonts too (a preview of a half-downloaded page skips those).
fn prepare(final_url: String, body: String, full: bool, xml: bool, headers: &[(String, String)]) -> Prepared {
    let parse_t = Instant::now();
    let dom = if xml { treebuilder::parse_xml(&body) } else { html::parse(&body) };
    if std::env::var("IRONSWIFT_PARSE_TIME").is_ok() {
        eprintln!("parse: {:?} for {} bytes", parse_t.elapsed(), body.len());
    }

    // The page's security policy: CSP from headers and <meta http-equiv>, framing rules, and
    // the sandbox of the frame it's shown in. Registered before anything else is fetched, so
    // every request the page makes is checked against it.
    let mut meta_csp: Vec<String> = dom
        .find_all("meta")
        .into_iter()
        .filter(|&m| dom.attr(m, "http-equiv").is_some_and(|v| v.trim().eq_ignore_ascii_case("content-security-policy")))
        .filter_map(|m| dom.attr(m, "content").map(String::from))
        .collect();
    if full {
        blocklist::reset_count(&final_url);
    }
    // Policies filter lists add (`$csp`), enforced like the page's own.
    meta_csp.extend(blocklist::extra_csp(&final_url, &final_url));
    let mut policy = security::PagePolicy::from_headers(&final_url, headers, &meta_csp);
    if let Some(f) = security::frame_info(&final_url) {
        policy.sandbox = match (f.sandbox, policy.sandbox.take()) {
            (Some(a), Some(b)) => Some(a.and(&b)),
            (a, b) => a.or(b),
        };
    }
    let policy = std::sync::Arc::new(policy);
    security::register(&final_url, policy.clone());
    let csp = &policy.csp;

    // Stylesheets: inline ones in place, external ones fetched in parallel, order kept.
    enum Src {
        Inline(String),
        /// URL, the element's nonce and its media attribute
        External(String, Option<String>, String),
    }
    let mut sources = Vec::new();
    let mut script_urls = Vec::new();
    for id in dom.descendants(dom::Dom::ROOT) {
        let nonce = dom.attr(id, "nonce").map(String::from);
        match dom.tag(id) {
            Some("style") => {
                let text = dom.text_content(id);
                if csp.allows_inline(security::Inline::Style, &text, nonce.as_deref()) {
                    sources.push(Src::Inline(text));
                }
            }
            Some("link") => {
                let rel = dom.attr(id, "rel").unwrap_or("").to_ascii_lowercase();
                if let (true, Some(href)) = (rel.split_whitespace().any(|r| r == "stylesheet"), dom.attr(id, "href")) {
                    let media = dom.attr(id, "media").unwrap_or("all");
                    if !(media.contains("print") && !media.contains("screen")) {
                        sources.push(Src::External(net::resolve(&final_url, href), nonce, media.to_string()));
                    }
                }
            }
            Some("script") => {
                if let Some(src) = dom.attr(id, "src") {
                    script_urls.push((net::resolve(&final_url, src), nonce));
                }
            }
            _ => {}
        }
    }
    // What's left of blocked ads: hidden by the filter lists' element-hiding rules.
    let hiding = if full { blocklist::hiding_css(&final_url, &dom) } else { String::new() };
    if !hiding.is_empty() {
        sources.push(Src::Inline(hiding));
    }
    // Extensions' content-script styles.
    if full {
        for css in extensions::content::page_css(&final_url, security::frame_info(&final_url).is_none()) {
            sources.push(Src::Inline(css));
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
                        css::load_imports(&mut sheet, &base, &|u| net::fetch_sub_nonce(u, page_url, security::Dest::Style, None).ok().map(|(_, t)| t), 0);
                        Some(sheet)
                    })
                }
                Src::External(u, nonce, media) => scope.spawn(move || {
                    let (_, text) = net::fetch_sub_nonce(u, page_url, security::Dest::Style, nonce.as_deref()).ok()?;
                    let mut sheet = css::parse(&text);
                    css::absolutize_urls(&mut sheet, u);
                    css::load_imports(&mut sheet, u, &|v| net::fetch_sub_nonce(v, page_url, security::Dest::Style, nonce.as_deref()).ok().map(|(_, t)| t), 0);
                    css::wrap_media(&mut sheet, media);
                    Some(sheet)
                }),
            })
            .collect();
        let script_jobs: Vec<_> =
            script_urls.iter().map(|(u, n)| scope.spawn(move || net::fetch_sub_nonce(u, page_url, security::Dest::Script, n.as_deref()).ok().map(|(_, b)| (u.clone(), b)))).collect();
        let sheets: Vec<css::Stylesheet> = sheet_jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect();
        let scripts: HashMap<String, String> = script_jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect();
        (sheets, scripts)
    });

    // Images: <img> sources plus CSS background images, fetched in parallel.
    let mut images = images::Images::default();
    let mut fonts = Vec::new();
    if full {
        let styles = style::compute(&dom, &sheets);
        let bg_urls = background_urls(&styles);
        images.load_missing(&dom, &final_url, &bg_urls);
        fonts = fetch_fonts(&sheets, &styles, &final_url);
        render_frames(&dom, &styles, &final_url, &mut images);
    }
    Prepared { url: final_url, dom, sheets, images, scripts, fonts, source: body, partial: false, xml, download: false, policy }
}

// ---------------------------------------------------------------- iframes

/// What an iframe shows: a URL, or `srcdoc` markup (resolved against the parent page).
enum FrameSrc {
    Url(String),
    Doc { html: String, base: String },
}

struct FrameJob {
    src: FrameSrc,
    w: u32,
    h: u32,
    /// The page the iframe is in, and that page's top-level page.
    parent: String,
    top: String,
    /// `<iframe sandbox>` flags (combined with any sandbox the parent itself is under).
    sandbox: Option<security::Sandbox>,
    reply: std::sync::mpsc::Sender<Option<image::RgbaImage>>,
    /// Not an iframe but a tab's page, run in its site's process (process.rs).
    top_level: bool,
}

thread_local! {
    /// How deep in nested iframes this thread is rendering (0 = not the frame worker).
    static FRAME_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
static FRAME_WORKER: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<FrameJob>>> = std::sync::OnceLock::new();

/// The thread that renders iframes. Each iframe is its own document with its own scripts;
/// they run here, one at a time, apart from the page around them.
fn frame_worker() -> Option<std::sync::mpsc::Sender<FrameJob>> {
    let w = FRAME_WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<FrameJob>();
        let _ = std::thread::Builder::new().name("frames".into()).stack_size(MAIN_STACK).spawn(move || {
            FRAME_DEPTH.with(|d| d.set(1));
            for job in rx {
                let img = match isolated::should_isolate(&job).then(|| isolated::render(&job)).flatten() {
                    Some(img) => img,
                    None => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_frame(&job))).ok().flatten(),
                };
                let _ = job.reply.send(img);
            }
        });
        std::sync::Mutex::new(tx)
    });
    w.lock().ok().map(|t| t.clone())
}

/// Load an iframe's document, run its scripts, and draw it at w×h.
fn render_frame(job: &FrameJob) -> Option<image::RgbaImage> {
    let (w, h) = (job.w, job.h);
    let fonts = load_fonts()?;
    let info = security::FrameInfo { top: job.top.clone(), sandbox: job.sandbox.clone() };
    let prepared = match &job.src {
        FrameSrc::Url(u) => {
            // Known as a frame before it's fetched: its cookies go in the top-level site's jar.
            security::set_frame(u, info.clone());
            let p = fetch_page(u, None);
            security::clear_frame(u);
            // X-Frame-Options / frame-ancestors: a page can refuse to be shown inside others.
            if !p.policy.may_be_framed_by(&p.url, &job.parent, &job.top) {
                security::clear_frame(&p.url);
                return Some(refused_frame(w, h));
            }
            p
        }
        // srcdoc: the parent's document (and origin), unless sandboxed.
        FrameSrc::Doc { html, base } => {
            // It shares the parent's URL, so the parent's registered policy is put back after.
            let parent_policy = security::policy_for(base);
            if job.sandbox.is_some() {
                security::set_frame(base, info.clone());
            }
            let p = prepare(base.clone(), html.clone(), true, false, &[]);
            let img = draw_frame(p, fonts, w, h);
            security::clear_frame(base);
            security::register(base, parent_policy.unwrap_or_default());
            return img;
        }
    };
    // The document keeps its frame identity while its scripts run (storage, cookies, sandbox).
    security::set_frame(&prepared.url, info);
    let frame_url = prepared.url.clone();
    let img = draw_frame(prepared, fonts, w, h);
    security::clear_frame(&frame_url);
    img
}

/// A grey box saying the page refused to be embedded (as browsers show).
fn refused_frame(w: u32, h: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(w, h, |x, y| {
        let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
        if border { image::Rgba([203, 213, 225, 255]) } else { image::Rgba([241, 245, 249, 255]) }
    })
}

fn draw_frame(prepared: Prepared, fonts: &'static Fonts, w: u32, h: u32) -> Option<image::RgbaImage> {
    // (Set after preparing: nested frames rendered meanwhile use their own size.)
    geometry::set_viewport(w as f32, h as f32);
    css::set_viewport(w as f32, h as f32); // vw/vh and @media while scripts run at load
    let mut page = build_page(prepared);
    if let Some(js) = page.js.as_mut() {
        js.settle(std::time::Duration::from_millis(1500));
        if js::take_dirty() {
            page.restyle();
        }
    }
    let lay = page.layout(fonts, w as f32, h as f32);
    let mut canvas = Canvas::new(w as usize, h as usize);
    canvas.clear(lay.page_bg);
    canvas.paint(fonts, &page.images, &lay.items, 0.0, 0.0);
    let buf = &canvas.buf;
    Some(image::RgbaImage::from_fn(w, h, |x, y| {
        let p = buf[y as usize * w as usize + x as usize];
        image::Rgba([(p >> 16) as u8, (p >> 8) as u8, p as u8, 255])
    }))
}

use frames::isolated;

/// Render the page's iframes (up to 8, three levels deep, 8 seconds in all) into `images`.
fn render_frames(dom: &dom::Dom, styles: &[Rc<style::Style>], base: &str, images: &mut images::Images) {
    let depth = FRAME_DEPTH.with(|d| d.get());
    if depth >= 3 {
        return;
    }
    let deadline = Instant::now() + std::time::Duration::from_secs(8);
    for id in dom.find_all("iframe").into_iter().take(8) {
        let key = layout::frame_key(dom, id);
        if images.get(&key).is_some() || styles.get(id).is_some_and(|s| s.display == style::Display::None) {
            continue;
        }
        let px = |a: &str| dom.attr(id, a).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
        let st = styles.get(id);
        let w = match st.map(|s| s.width) {
            Some(style::Len::Px(v)) => Some(v),
            _ => px("width"),
        }
        .unwrap_or(300.0)
        .clamp(1.0, 2000.0) as u32;
        let h = match st.map(|s| s.height) {
            Some(style::Len::Px(v)) => Some(v),
            _ => px("height"),
        }
        .unwrap_or(150.0)
        .clamp(1.0, 2000.0) as u32;
        // A tab's page shown from its site process (process.rs).
        if depth == 0 && frames::enabled() && process::is_shell_frame(dom, id, base) {
            let (tx, _) = std::sync::mpsc::channel();
            let job = FrameJob { src: FrameSrc::Url(base.to_string()), w, h, parent: base.to_string(), top: base.to_string(), sandbox: None, reply: tx, top_level: true };
            if let Some(img) = frames::start(job, &key, base, Instant::now() + std::time::Duration::from_secs(20)) {
                images.insert(&key, img);
            }
            continue;
        }
        let src = match (dom.attr(id, "srcdoc"), dom.attr(id, "src").map(str::trim)) {
            (Some(doc), _) => FrameSrc::Doc { html: doc.to_string(), base: base.to_string() },
            (None, Some(s)) if !s.is_empty() && s != "about:blank" && !s.starts_with("javascript:") => FrameSrc::Url(net::resolve(base, s)),
            // An empty frame shows nothing: the iframe's own background shows through.
            _ => continue,
        };
        if let FrameSrc::Url(u) = &src {
            // The page's frame-src policy, the content blockers, and no local files from web pages.
            let policy = security::policy_for(base);
            if policy.as_ref().is_some_and(|p| !p.csp.allows_url(security::Dest::Frame, u, None, true))
                || blocklist::blocked(u, base, security::Dest::Frame).is_some()
                || !security::file_access_ok(base, u)
                || security::is_privileged_url(u)
            {
                continue;
            }
        }
        // This page's own place: its top-level page, and any sandbox it is itself under.
        let me = security::frame_info(base);
        let top = me.as_ref().map_or_else(|| base.to_string(), |f| f.top.clone());
        let own = dom.attr(id, "sandbox").map(security::Sandbox::parse);
        let sandbox = match (own, me.and_then(|f| f.sandbox)) {
            (Some(a), Some(b)) => Some(a.and(&b)),
            (a, b) => a.or(b),
        };
        let job_parts = (base.to_string(), top, sandbox);
        let live = depth == 0 && frames::enabled() && !(matches!(src, FrameSrc::Doc { .. }) && job_parts.2.is_some());
        let img = if live {
            // In the window, frames stay live: their documents keep running and take input (frames.rs).
            let (parent, top, sandbox) = job_parts;
            let (tx, _) = std::sync::mpsc::channel();
            frames::start(FrameJob { src, w, h, parent, top, sandbox, reply: tx, top_level: false }, &key, base, deadline)
        } else if depth == 0 {
            let Some(worker) = frame_worker() else { return };
            let (tx, rx) = std::sync::mpsc::channel();
            let (parent, top, sandbox) = job_parts;
            if worker.send(FrameJob { src, w, h, parent, top, sandbox, reply: tx, top_level: false }).is_err() {
                return;
            }
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(img) => img,
                Err(_) => {
                    isolated::stop_stuck();
                    None
                }
            }
        } else {
            // Already on the frame worker: a frame inside a frame is drawn right here.
            FRAME_DEPTH.with(|d| d.set(depth + 1));
            let (parent, top, sandbox) = job_parts;
            let (tx, _rx) = std::sync::mpsc::channel();
            let img = render_frame(&FrameJob { src, w, h, parent, top, sandbox, reply: tx, top_level: false });
            FRAME_DEPTH.with(|d| d.set(depth));
            img
        };
        if let Some(img) = img {
            images.insert(&key, img);
        }
        if Instant::now() >= deadline {
            break;
        }
    }
}

fn background_urls(styles: &[Rc<style::Style>]) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for st in styles {
        for u in st.bg.urls() {
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
    let Prepared { url, dom, sheets, images, scripts, fonts, source, partial, xml, policy, .. } = p;
    install_fonts(fonts);
    // Start JavaScript only if the page has scripts or inline event handlers:
    // the engine costs several megabytes even when it has nothing to run.
    // A sandboxed frame runs none unless its sandbox allows scripts.
    let scripts_allowed = policy.sandbox.as_ref().is_none_or(|s| s.scripts);
    let has_scripts = !partial && scripts_allowed && dom.descendants(dom::Dom::ROOT).iter().any(|&n| match &dom.nodes[n].data {
        dom::NodeData::Element { tag, attrs } => tag == "script" || attrs.iter().any(|(k, _)| k.starts_with("on")),
        _ => false,
    });
    let icon = favicon::icon_url(&dom, &url);
    let live = Rc::new(RefCell::new(livesheets::Live::new(&url, policy.clone(), &dom, sheets.clone())));
    js::set_dom(dom);
    layout::reset_element_scrolls();
    // Scripts that ask where elements are get a fresh layout of the page as it is then.
    let provider: Option<Box<dyn Fn(f32, f32) -> layout::Layout>> = FONTS.with(|c| c.get()).map(|fonts| {
        let live = live.clone();
        let images = images.share();
        Box::new(move |w: f32, h: f32| {
            js::with_dom(|d| {
                let sheets = {
                    let mut l = live.borrow_mut();
                    l.refresh(d);
                    l.sheets()
                };
                let c = d.composed();
                let d = c.as_ref().unwrap_or(d);
                // Styled without :visited, like getComputedStyle (which reuses these styles).
                let styles = style::without_visited(|| style::compute(d, &sheets));
                geometry::note_styles(&styles);
                layout::layout(d, &styles, fonts, &images, w, h)
            })
        }) as Box<dyn Fn(f32, f32) -> layout::Layout>
    });
    geometry::set_provider(provider);
    {
        let live = live.clone();
        geometry::set_styler(Some(Box::new(move || {
            js::with_dom(|d| {
                let sheets = {
                    let mut l = live.borrow_mut();
                    l.refresh(d);
                    l.sheets()
                };
                let c = d.composed();
                // Scripts must not learn which links were visited.
                style::without_visited(|| style::compute(c.as_ref().unwrap_or(d), &sheets))
            })
        })));
    }
    if has_scripts {
        canvas::set_images(images.share());
    }
    let js = has_scripts.then(|| {
        let mut js = js::Js::with_policy(&url, policy.clone());
        // Parse again with the scripts running as they're reached (the first parse found
        // what to download).
        if xml {
            // XHTML is parsed as XML up front; its scripts then run in document order.
            js.run_in_order(&url, &scripts);
        } else {
            js.parse_and_run(&source, &url, &scripts);
        }
        js
    });
    js::take_dirty();
    style::set_hover(None);
    style::set_active(None);
    // :visited: links to addresses in the history (only their colors change).
    style::set_visited(&url, library::history().into_iter().map(|v| v.url).collect());
    let hover_rules = style::uses_hover(&sheets);
    let form_defaults = js::with_dom(forms::defaults);
    static DOCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let doc = DOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut page = Page { url, icon, title: String::new(), sheets, styles: Vec::new(), images, js, hover_rules, doc, form_defaults, loaded_at: Instant::now(), styled_for: (0.0, 0.0), live };
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
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.to_string());
    let tag = err.strip_prefix('[').and_then(|r| r.split_once(']')).map_or("net", |(t, _)| t);
    // Translated strings; the host is escaped and set in bold.
    let t = |id: &str| esc(&i18n::t(id));
    let with_host = |id: &str| esc(&i18n::tf(id, &[("host", "\u{1}")])).replace('\u{1}', &format!("<b>{}</b>", esc(&host)));
    let (title, explain, tips): (String, String, Vec<String>) = match tag {
        "dns" => (t("err-dns-title"), with_host("err-dns-detail"), vec![t("err-dns-tip1"), t("err-dns-tip2"), t("err-dns-tip3")]),
        "timeout" => (t("err-timeout-title"), with_host("err-timeout-detail"), vec![t("err-timeout-tip1"), t("err-net-tip")]),
        "cert" => (t("err-cert-title"), with_host("err-cert-detail"), vec![t("err-cert-tip1"), t("err-cert-tip2")]),
        "refused" => (t("err-refused-title"), with_host("err-refused-detail"), vec![t("err-refused-tip1"), t("err-refused-tip2")]),
        "proxy" => (t("err-proxy-title"), t("err-proxy-detail"), vec![t("err-proxy-tip")]),
        "type" => (t("err-type-title"), t("err-type-detail"), vec![]),
        _ => (t("err-other-title"), with_host("err-other-detail"), vec![t("err-net-tip"), t("err-other-tip")]),
    };
    let tips: String = tips.iter().map(|t| format!("<li>{t}</li>")).collect();
    // The technical detail, without the tag.
    let detail = err.split_once("] ").map_or(err, |(_, d)| d);
    let lang = i18n::locale();
    let dir = if i18n::rtl() { "rtl" } else { "ltr" };
    format!(
        "<!DOCTYPE html><html lang=\"{lang}\" dir=\"{dir}\"><head><title>{title}</title></head><body style='font-family:sans-serif;margin:60px auto;max-width:640px;padding:0 24px;color:#334155'>\
         <h1 style='color:#1f2937'>{title}</h1><p style='font-size:17px'>{explain}</p><ul style='line-height:1.7'>{tips}</ul>\
         <p><a href=\"{}\">{}</a> · <a href=\"about:settings\">{}</a></p>\
         <details><summary style='color:#64748b'>{}</summary><p style='color:#64748b;font-size:13px'>{}</p></details></body></html>",
        esc(url),
        t("err-try-again"),
        t("err-settings"),
        t("err-details"),
        esc(detail)
    )
}

/// Everything runs on a thread with a large stack: layout, styling and Boa all recurse with the
/// page's nesting, and the 8 MB main-thread stack would let a deeply nested page crash the
/// browser. (The memory is only reserved; pages that don't need it never touch it.)
const MAIN_STACK: usize = 512 << 20;

/// When the program started (for measuring startup time).
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn main() {
    STARTED.get_or_init(Instant::now);
    let run = std::thread::Builder::new().name("ironswift".into()).stack_size(MAIN_STACK).spawn(browser_main);
    match run.map(|t| t.join()) {
        Ok(Ok(())) => {}
        // The panic hook has already written the crash report.
        Ok(Err(_)) => std::process::exit(101),
        Err(_) => browser_main(),
    }
}

fn browser_main() {
    crashlog::install_panic_hook();
    prefer_x11();
    // A private window keeps nothing: its own empty profile folder, deleted when it closes.
    let private_profile = start_private_profile();
    // The browser's language becomes the default for pages' Intl too (before threads start).
    i18n::apply_to_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut url = String::from("about:home");
    // Whether a page was named on the command line (else: restore the last session or go home).
    let mut explicit = false;
    let mut png: Option<String> = None;
    // Print the page's accessibility tree instead of opening a window.
    let mut a11y_dump = false;
    let mut width = 1024usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fetch" => {
                // Developer tool: fetch each URL given and print what came back.
                for u in &args[i + 1..] {
                    let req = net::Request { method: "GET", url: u, headers: &[], body: None, top: Some(u.as_str()), cookies: true, extra: &[] };
                    match net::http(&req) {
                        Ok(r) => println!("{u}\n  {} {}, {} bytes\n  {}", r.status, r.status_text, r.body.len(), String::from_utf8_lossy(&r.body[..r.body.len().min(120)]).replace('\n', " ")),
                        Err(e) => println!("{u}\n  error: {e}"),
                    }
                }
                return;
            }
            "--frame-renderer" => {
                isolated::serve(args.get(i + 1).map_or("", String::as_str));
                return;
            }
            "--site-renderer" => {
                process::serve(args.get(i + 1).map_or("", String::as_str));
                return;
            }
            "--conformance-tokenizer" => {
                let dir = args.get(i + 1).cloned().unwrap_or_else(|| ".".into());
                std::process::exit(conformance::tokenizer_tests(&dir));
            }
            "--conformance-html5lib" => {
                let dir = args.get(i + 1).cloned().unwrap_or_else(|| ".".into());
                std::process::exit(conformance::html5lib(&dir));
            }
            "--png" => {
                png = args.get(i + 1).cloned();
                i += 1;
            }
            "--a11y" => a11y_dump = true,
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
    let rss = crate::platform::rss_mb;
    let probe = std::env::var("IRONSWIFT_MEM_PROBE").is_ok();
    if probe {
        log!("[mem] start: {} MB", rss());
    }
    let fonts: &'static Fonts = match Fonts::load() {
        Ok(f) => Box::leak(Box::new(f)),
        Err(e) => {
            log!("{e}");
            std::process::exit(1);
        }
    };
    FONTS.with(|c| c.set(Some(fonts)));
    if probe {
        log!("[mem] fonts loaded: {} MB", rss());
        let js = js::Js::new("about:blank");
        log!("[mem] + one JS engine: {} MB", rss());
        drop(js);
        let mut page = load(&url);
        log!("[mem] + page loaded: {} MB", rss());
        let lay = page.layout(fonts, width as f32, 768.0);
        log!("[mem] + layout ({} items): {} MB", lay.items.len(), rss());
        return;
    }
    if a11y_dump {
        let (page, _, _) = render_page(fonts, &url, width);
        print!("{}", js::with_dom(|d| a11y::dump(&match d.composed() {
            Some(c) => a11y::build(&c, Some(&page.styles), None),
            None => a11y::build(d, Some(&page.styles), None),
        })));
        return;
    }
    match png {
        Some(out) if site_snapshot::wanted(&url) => {
            let ok = site_snapshot::snapshot(&url, width as u32, 768, &out, std::time::Duration::from_millis(1500), std::time::Duration::from_secs(20));
            log!("{url}: {} through a site process", if ok { "snapshot taken" } else { "no snapshot" });
        }
        Some(out) => render_png(fonts, &url, width, &out),
        None => {
            blocklist::start_updates();
            updates::start();
            run_window(fonts, &url, width, explicit);
            cookies::clear_session_sites();
            updates::install_staged();
            if let Some(dir) = &private_profile {
                let _ = std::fs::remove_dir_all(dir);
            } else if settings::get().clear_on_exit {
                log!("[privacy] clearing cookies and site data on exit");
                settings::clear_data(false);
            }
        }
    }
}

/// Load a page and paint the whole of it (up to 16,000 px tall) into a canvas.
fn render_page(fonts: &Fonts, url: &str, width: usize) -> (Page, layout::Layout, Canvas) {
    // IRONSWIFT_WAIT_FOR="<script expression>" (tools/wpt.py sets it for script tests): media
    // and animations play in real time, and the snapshot waits until the expression is true
    // (the test harness reported), up to IRONSWIFT_WAIT_SECS (default 25); then
    // IRONSWIFT_WAIT_EXPIRED (a script, e.g. the harness's `timeout()`) runs.
    let wait_for = std::env::var("IRONSWIFT_WAIT_FOR").ok().filter(|w| !w.trim().is_empty());
    if wait_for.is_none() {
        images::freeze_animations();
    }
    geometry::set_viewport(width as f32, 768.0);
    css::set_viewport(width as f32, 768.0); // vw/vh and @media while scripts run at load
    let mut page = load(url);
    // Give fetch() calls and short timers a moment to fill the page in before the snapshot.
    if let Some(js) = page.js.as_mut() {
        match &wait_for {
            Some(expr) => {
                let secs = std::env::var("IRONSWIFT_WAIT_SECS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(25.0);
                let expired = std::env::var("IRONSWIFT_WAIT_EXPIRED").ok();
                js.wait_for(expr, std::time::Duration::from_secs_f64(secs.max(0.0)), expired.as_deref());
            }
            None => js.settle(std::time::Duration::from_secs(5)),
        }
        if js::take_dirty() {
            page.restyle();
        }
    }
    let mut lay = page.layout(fonts, width as f32, 768.0);
    // Lazy images the first screen would show load before the snapshot.
    if page.load_lazy_now(&lay, 768.0) {
        lay = page.layout(fonts, width as f32, 768.0);
    }
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
    log!(
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
impl window::InputCallback for Typed {
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

/// Everything Tab can move to, in tab order (links, controls, tabindex elements).
fn focusable(d: &dom::Dom, styles: &[Rc<style::Style>]) -> Vec<dom::NodeId> {
    a11y::tab_order(d, (styles.len() == d.nodes.len()).then_some(styles))
}

/// Run developer-console code in a background tab's page (the one with document number
/// `doc`): its DOM and layout are swapped in while the code runs, and the developer tools
/// snapshot is taken again so the Elements view shows any changes.
fn run_in_background_tab(tabs: &mut [Option<Slot>], doc: u64, code: &str) -> Result<String, String> {
    let Some(sv) = tabs.iter_mut().flatten().filter_map(|s| s.saved.as_mut()).find(|sv| sv.page.doc == doc) else {
        return Err("the inspected page is closed or was navigated away (press F12 on it again)".into());
    };
    let mine = (js::take_dom(), layout::take_element_scrolls(), geometry::take_provider());
    js::set_dom(std::mem::take(&mut sv.dom));
    layout::set_element_scrolls(std::mem::take(&mut sv.scrolls));
    geometry::set_provider(sv.provider.take());
    let r = match sv.page.js.as_mut() {
        Some(j) => j.run(code),
        None => Err("that page doesn't run scripts".into()),
    };
    let (scripts, errors) = sv.page.js.as_ref().map_or((0, 0), |j| (j.stats.scripts, j.stats.errors));
    js::with_dom(|d| devtools::capture(d, &sv.page.url, &sv.page.title, sv.page.loaded_at, scripts, errors));
    sv.dom = js::take_dom();
    sv.scrolls = layout::take_element_scrolls();
    sv.provider = geometry::take_provider();
    js::set_dom(mine.0);
    layout::set_element_scrolls(mine.1);
    geometry::set_provider(mine.2);
    r
}

/// Fire an event at a node if the page runs scripts. Returns false if a script cancelled it.
fn fire(page: &mut Page, node: dom::NodeId, kind: &str) -> bool {
    match page.js.as_mut() {
        Some(js) => js.dispatch(node, kind),
        None => true,
    }
}

/// Saved logins or earlier entries for a text field, as a list under it (None if there are none).
fn open_suggest(page: &Page, lay: &layout::Layout, field: dom::NodeId) -> Option<formui::Suggest> {
    let items = js::with_dom(|d| if matches!(forms::control(d, field), Some(forms::Control::Text { .. })) { autofill::suggestions(d, field, &page.url) } else { Vec::new() });
    if items.is_empty() {
        return None;
    }
    let r = lay.hits.iter().rev().find(|(_, n)| *n == field).map(|(r, _)| *r)?;
    Some(formui::Suggest { node: field, doc: page.doc, x: r.x, y: r.y + r.h + 2.0, w: r.w.max(220.0), items, hover: None })
}

/// Stop editing a contenteditable element.
fn stop_editing(ce: &mut Option<editing::Caret>) {
    if ce.take().is_some() {
        editing::set_host(None);
    }
}

/// Keys that belong to a contenteditable element being edited (not page scrolling).
fn editing_key(key: Key, ctrl: bool, alt: bool) -> bool {
    !alt && (!ctrl || matches!(key, Key::V | Key::A))
        && !matches!(key, Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12 | Key::Tab | Key::PageUp | Key::PageDown)
}

/// Apply this frame's typing to a contenteditable element. Each change fires `beforeinput`
/// (a script can cancel it) and `input`. Returns true if anything changed or moved.
fn contenteditable_keys(page: &mut Page, fonts: &Fonts, c: &mut editing::Caret, presses: &[KeyPress], chars: &[char], ctrl: bool, clip: &mut Option<arboard::Clipboard>) -> bool {
    let plain = js::with_dom(|d| d.attr(c.host, "contenteditable").is_some_and(|v| v.trim().eq_ignore_ascii_case("plaintext-only")));
    let mut any = false;
    // Ask the page first; `data` is the inserted text (if any).
    let edit = |page: &mut Page, c: &mut editing::Caret, input_type: &str, data: Option<&str>, f: &mut dyn FnMut(&mut dom::Dom, &mut editing::Caret) -> bool| -> bool {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let arg = data.map_or("null".to_string(), |d| format!("'{}'", esc(d)));
        let host = c.host;
        if let Some(js) = page.js.as_mut() {
            if js.call_number(&format!("__editEvent({host}, 'beforeinput', '{input_type}', {arg})"), "beforeinput") == 0.0 {
                return false;
            }
        }
        let changed = js::with_dom_mut(|d| f(d, c));
        if changed {
            if let Some(js) = page.js.as_mut() {
                js.call_number(&format!("__editEvent({host}, 'input', '{input_type}', {arg})"), "input");
            }
        }
        changed
    };
    let typed: String = chars.iter().filter(|ch| !ch.is_control()).collect();
    if !typed.is_empty() && !ctrl {
        any |= edit(page, c, "insertText", Some(&typed), &mut |d, c| {
            editing::insert_text(d, c, &typed);
            true
        });
    }
    for p in presses {
        match p.key {
            Key::Backspace => any |= edit(page, c, "deleteContentBackward", None, &mut |d, c| editing::delete_backward(d, c)),
            Key::Delete => any |= edit(page, c, "deleteContentForward", None, &mut |d, c| editing::delete_forward(d, c)),
            Key::Enter | Key::NumPadEnter => {
                let kind = if plain || p.shift { "insertLineBreak" } else { "insertParagraph" };
                any |= edit(page, c, kind, None, &mut |d, c| {
                    editing::insert_line_break(d, c);
                    true
                });
            }
            Key::Left | Key::Right => any |= js::with_dom(|d| editing::move_by(d, c, p.key == Key::Right)),
            Key::Home | Key::End => {
                editing::line_edge(fonts, c, p.key == Key::End);
                any = true;
            }
            Key::V if ctrl => {
                if let Some(text) = clip.as_mut().and_then(|cb| cb.get_text().ok()) {
                    let text = text.replace("\r\n", "\n");
                    any |= edit(page, c, "insertFromPaste", Some(&text), &mut |d, c| {
                        for (i, line) in text.split('\n').enumerate() {
                            if i > 0 {
                                editing::insert_line_break(d, c);
                            }
                            editing::insert_text(d, c, line);
                        }
                        true
                    });
                }
            }
            _ => {}
        }
    }
    any
}

/// The caret in a contenteditable element.
fn draw_edit_caret(canvas: &mut Canvas, fonts: &Fonts, page: &Page, lay: &layout::Layout, c: &editing::Caret, scroll: f32) {
    let z = page.scale();
    let st = page.styles.get(c.host).cloned();
    let (pad_l, pad_t, size, color) = st.map_or((0.0, 0.0, 16.0, Color::rgb(0, 0, 0)), |s| (s.padding[3].resolve(0.0) + s.border[3], s.padding[0].resolve(0.0) + s.border[0], s.font_size, s.color));
    let host_box = lay.boxes.get(&c.host).map(|(r, _)| r).or_else(|| lay.hits.iter().rev().find(|(_, n)| *n == c.host).map(|(r, _)| r)).map_or((0.0, 0.0), |r| (r.x / z + pad_l, r.y / z + pad_t));
    let (x, top, hgt) = js::with_dom(|d| editing::caret_rect(d, fonts, c, host_box, size));
    canvas.fill_rect((x * z).round(), top * z - scroll + bar_h() as f32, (z.round()).max(1.0), hgt * z, color);
}

/// Open the file chooser for a file input.
fn open_file_picker(page: &Page, input: dom::NodeId) -> Result<filepicker::Pending, String> {
    let (multiple, accept) = js::with_dom(|d| (d.attr(input, "multiple").is_some(), d.attr(input, "accept").unwrap_or("").to_string()));
    filepicker::open(input, page.doc, multiple, &accept)
}

/// Submit a form. With `fire_event`, the page's submit handlers run first and may cancel it.
fn submit_form(page: &mut Page, form: dom::NodeId, submitter: Option<dom::NodeId>, fire_event: bool) -> Option<Go> {
    // Interactive submissions check the form first (required, pattern, min/max…) and stop at
    // the first problem, which the window shows in a bubble.
    if fire_event && !js::with_dom(|d| validation::no_validate(d, form, submitter)) {
        let bad = match page.js.as_mut() {
            Some(js) => js.call_number(&format!("__interactiveValidate({form})"), "validate"),
            None => js::with_dom(|d| validation::invalid_controls(d, &d.form_state, form).first().map_or(-1.0, |&n| n as f64)),
        };
        if bad >= 0.0 {
            validation::request_report(bad as dom::NodeId);
        }
        if bad >= 0.0 || bad == -2.0 {
            return None;
        }
    }
    if fire_event && !fire(page, form, "submit") {
        return None;
    }
    let sub = js::with_dom(|d| forms::submission(d, form, submitter, &page.url));
    if sub.url.starts_with("javascript:") || !security::page_may_navigate(&page.url, &sub.url) {
        return None;
    }
    // The page's CSP form-action, and a sandboxed frame without allow-forms.
    if let Some(p) = security::policy_for(&page.url) {
        if !p.csp.allows_form_action(&sub.url) || p.sandbox.as_ref().is_some_and(|s| !s.forms) {
            return None;
        }
    }
    // Remember typed entries, and offer to save a login.
    js::with_dom(|d| autofill::form_submitted(d, form, &page.url, is_private()));
    Some(match sub.post {
        Some(body) => Go::Post(sub.url, body),
        None => Go::To(sub.url),
    })
}

/// In a private window (IRONSWIFT_PRIVATE=1, or opened with Ctrl+Shift+N): use a fresh
/// profile folder in the temporary directory. Returns it so it can be deleted on exit.
fn start_private_profile() -> Option<std::path::PathBuf> {
    if !std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1") || std::env::var("IRONSWIFT_PROFILE").is_ok() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("ironswift-private-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    // Safe here: nothing else is running yet (no threads have started).
    unsafe { std::env::set_var("IRONSWIFT_PROFILE", &dir) };
    Some(dir)
}

/// Ctrl+Shift+N: a new private window, as its own process.
fn open_private_window() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("about:home").env("IRONSWIFT_PRIVATE", "1").env_remove("IRONSWIFT_PROFILE").spawn();
    }
}

fn is_private() -> bool {
    std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1")
}

fn run_window(fonts: &Fonts, start: &str, width: usize, explicit: bool) {
    extensions::start();
    // Iframes in the window stay live (frames.rs); snapshots draw them once.
    frames::enable();
    process::enable_from_settings();
    zoom::use_device_scale();
    zoom::set_pixel_ratio(zoom::device_scale());
    // Without a page on the command line, reopen last session's tabs (if that's the setting),
    // else open the home page.
    let session = if explicit || !settings::get().restore_tabs { None } else { load_session() };
    let home = settings::get().home;
    let start: &str = match &session {
        Some((urls, a)) => &urls[*a].0.clone(),
        None if explicit => start,
        None => &home,
    };
    let mut first_frame_shown = false;
    let mut window = Window::new("Ironswift", width, 768, WindowOptions { resize: true, ..WindowOptions::default() })
        .unwrap_or_else(|e| {
            log!("Could not open a window: {e}");
            std::process::exit(1);
        });
    window.set_target_fps(60);
    // Draw with the graphics card when there is one (see gpu.rs); otherwise in software.
    #[cfg(feature = "gpu")]
    let mut gpu = {
        let (gw, gh) = window.get_size();
        let g = gpu::Gpu::new(&window, gw as u32, gh as u32);
        match &g {
            Some(g) => log!("[gpu] drawing with {}", g.name),
            None => log!("[gpu] no graphics adapter; drawing in software"),
        }
        g
    };
    // The window and graphics are set up: from here on, refuse the system calls a browser
    // never needs (see sandbox.rs).
    // Screen readers (AT-SPI): connects to the accessibility bus; costs nothing until one listens.
    let mut a11y = a11y_bridge::Bridge::new();
    let mut window_active = false;
    sandbox::protect_browser();
    let mut idle_frames = 0u32;
    let mut slow = false;
    let typed = Rc::new(RefCell::new(Vec::new()));
    let pressed = Rc::new(RefCell::new(Vec::<KeyPress>::new()));
    window.set_input_callback(Box::new(Typed { chars: typed.clone(), keys: pressed.clone(), ctrl: [false; 2], shift: [false; 2], alt: [false; 2] }));
    // The window reports held-key repeats itself (on Wayland too).
    let add_repeats = false;

    let mut back: Vec<Entry> = Vec::new();
    let mut forward: Vec<Entry> = Vec::new();
    let mut hover: Option<usize> = None;
    // Bookmarks bar: the list, and which chip the mouse is over (usize::MAX = "»").
    let mut bm_list: Vec<library::Bookmark>;
    let mut bm_hover: Option<usize> = None;
    // Dragging a tab along the strip: where the press started (x), and whether it has moved enough to count.
    let mut tab_drag: Option<(f32, bool)> = None;
    let mut folder_menu: Option<FolderMenu> = None;
    let mut hovered: Option<dom::NodeId> = None;
    let mut active_node: Option<dom::NodeId> = None;
    // Pages load on worker threads; results come back tagged with a generation number
    // so a stopped or superseded load is ignored when it arrives.
    let (tx, rx) = mpsc::channel::<(u64, Prepared)>();
    let mut generation = 0u64;
    let mut loading: Option<Loading> = None;
    // The load whose first part is on screen already.
    let mut preview_of: Option<u64> = None;
    // Browser UI over the page (chrome.rs): find bar, selection, menus, status, notes.
    let mut find: Option<chrome::Find> = None;
    let mut find_edit = textedit::TextEdit::new("");
    let mut selection: Option<(u64, chrome::Selection)> = None;
    let mut sel_press: Option<(f32, f32)> = None;
    let mut ctx_menu: Option<chrome::Menu<Ctx>> = None;
    let mut dl_panel: Option<chrome::Menu<DlAct>> = None;
    let mut status_link: Option<String> = None;
    let mut toast: Option<(String, Instant)> = None;
    // Sites' permission questions (location, notifications…), shown one at a time.
    let mut perm_queue: Vec<permissions::Prompt> = Vec::new();
    let mut ui_action: Option<Ctx> = None;
    let mut runs_cache: (u64, Vec<chrome::TextRun>) = (u64::MAX, Vec::new());
    let mut was_right = false;
    let mut last_spin = Instant::now();
    let (mut w, mut h) = window.get_size();
    geometry::set_viewport(w as f32, h.saturating_sub(bar_h()) as f32);
    css::set_viewport(w as f32, h.saturating_sub(bar_h()) as f32); // vw/vh and @media while scripts run at load
    let mut page = load(start);
    crashlog::set_url(&page.url);
    library::visit(&page.url, &page.title);
    let mut lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
    // Bumped whenever the layout is replaced: the stored page tiles are then out of date.
    let mut page_version: u64 = 1;
    let mut page_tiles = tiles::PageTiles::default();
    let use_tiles = std::env::var("IRONSWIFT_TILES").map_or(true, |v| v != "0");
    // IRONSWIFT_FRAME_STATS=1: print how long drawing takes, every 60 frames.
    let frame_stats = std::env::var("IRONSWIFT_FRAME_STATS").is_ok_and(|v| v == "1");
    let mut draw_times: Vec<f64> = Vec::new();
    let mut canvas = Canvas::new(w, h);
    let mut scroll = 0.0f32;
    let mut smooth = smooth::Smooth::default();
    let mut editing = false;
    let mut address = page.url.clone();
    let mut dirty = true;
    let mut was_down = false;
    // The mouse as the page's scripts have seen it (for mouseover, mousedown… events).
    let mut pointer = userinput::Pointer::default();
    // Live iframes (frames.rs): the one under the mouse, and the one that has the keyboard.
    let mut frame_hover: Option<String> = None;
    let mut frame_focus: Option<String> = None;
    // The focused form control (typing goes into it) and an open <select> list.
    let mut focused: Option<dom::NodeId> = None;
    // Focus moved by the keyboard shows a focus ring; Enter or Space on a focused element
    // becomes a click at its middle (window coordinates) on the next mouse pass.
    let mut focus_visible = false;
    let mut key_click: Option<(f32, f32)> = None;
    // Keyboard focus in the browser UI (an index into ui_targets), and where the mouse was
    // last frame (menus follow the mouse only when it moves, so arrow keys can drive them).
    let mut ui_focus: Option<usize> = None;
    let mut last_mouse = (f32::NAN, f32::NAN);
    let mut dropdown: Option<Dropdown> = None;
    // A form control's validation message, shown after a blocked submission.
    let mut bubble: Option<formui::Bubble> = None;
    // An open file chooser for an <input type=file>.
    let mut file_pick: Option<filepicker::Pending> = None;
    // An open date, time or color picker.
    let mut picker: Option<formui::Picker> = None;
    // The caret in a contenteditable element being typed into.
    let mut ce: Option<editing::Caret> = None;
    // Saved logins or earlier entries offered under a field, and the "Save login?" panel.
    let mut suggest: Option<formui::Suggest> = None;
    let mut save_prompt: Option<formui::SavePrompt> = None;
    window.set_title(&format!("{} — Ironswift{}", if page.title.is_empty() { &page.url } else { &page.title }, if is_private() { " (private)" } else { "" }));
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
    let mut last_download_tick = Instant::now();
    let mut downloads_were_active = false;
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
    // A press on the page waiting for its release (the click), and the last click, for dblclick.
    let mut page_press: Option<clicks::Press> = None;
    let mut last_page_click = clicks::Last::default();

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
            picker = None;
            suggest = None;
            stop_editing(&mut ce);
            editing = false;
            fedit = None;
            hovered = None;
            style::set_hover(None);
            layout::set_focus(focused.map(|f| (f, js::with_dom(|d| forms::is_editable(d, f)))));
            lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
            page_version += 1;
            scroll = scroll.clamp(0.0, (lay.height - h.saturating_sub(bar_h()) as f32).max(0.0));
            window.set_title(&format!("{} — Ironswift{}", page.label(), if is_private() { " (private)" } else { "" }));
            dirty = true;
        }};
    }

    while window.is_open() {
        // A site icon arrived: redraw the tab strip.
        if favicon::arrived() {
            dirty = true;
        }
        // Extensions: show them the tabs, and do what they asked (one thing a frame).
        if !extensions::all().is_empty() {
            let list = tabs
                .iter()
                .map(|s| match s {
                    None => extensions::tabs::Tab { url: current.url.clone(), title: page.label() },
                    Some(s) => extensions::tabs::Tab { url: s.saved.as_ref().map_or(s.url.clone(), |sv| sv.current.url.clone()), title: s.title.clone() },
                })
                .collect();
            extensions::tabs::publish(list, active);
            if tab_cmd.is_none() && pending_nav.is_none() {
                use extensions::tabs::Command as C;
                match extensions::tabs::next_command() {
                    Some(C::Open(url, front)) => tab_cmd = Some(TabCmd::Open(url, !front)),
                    Some(C::Switch(i)) => tab_cmd = Some(TabCmd::Switch(i)),
                    Some(C::Close(i)) => tab_cmd = Some(TabCmd::Close(i)),
                    Some(C::Navigate(i, url)) if i == active => pending_nav = Some(Go::To(url)),
                    Some(C::Reload(i)) if i == active => pending_nav = Some(Go::Reload),
                    // Another tab: show it first, then load.
                    Some(c @ (C::Navigate(i, _) | C::Reload(i))) if i < tabs.len() => {
                        tab_cmd = Some(TabCmd::Switch(i));
                        extensions::tabs::push_front(c);
                    }
                    _ => {}
                }
            }
        }
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
                        let ptx = tx.clone();
                        let _ = tx.send((g, fetch_page_with(&t, None, &mut |p| {
                            let _ = ptx.send((g, p));
                        })));
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
                        if closed.len() > 25 {
                            closed.remove(0);
                        }
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
                            if closed.len() > 25 {
                                closed.remove(0);
                            }
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
        // Styles to recompute before this frame's layout (all changes in a frame share one restyle).
        let mut restyle = false;
        let mut stop_key = false;

        // Bookmarks bar: shown when there are bookmarks (and the setting is on); showing or
        // hiding it moves the page, so lay it out again.
        bm_list = library::bookmarks();
        let want_bar = !bm_list.is_empty() && settings::get().show_bookmarks_bar;
        if want_bar != BOOKMARKS_BAR.load(std::sync::atomic::Ordering::Relaxed) {
            BOOKMARKS_BAR.store(want_bar, std::sync::atomic::Ordering::Relaxed);
            geometry::set_viewport(w as f32, h.saturating_sub(bar_h()) as f32);
            lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
            page_version += 1;
            scroll = scroll.clamp(0.0, (lay.height - h.saturating_sub(bar_h()) as f32).max(0.0));
            dirty = true;
        }
        // Resize -> relayout.
        let (nw, nh) = window.get_size();
        if (nw, nh) != (w, h) && nw > 0 && nh > 0 {
            lay = page.layout(fonts, nw as f32, nh.saturating_sub(bar_h()) as f32);
            page_version += 1;
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
        // CSS transitions and animations move every frame while they run.
        if js::take_dirty() || anim::active() {
            page.restyle();
            lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
            page_version += 1;
            dirty = true;
        }
        // animationstart, transitionend… from this frame's style passes (anim.rs).
        let key = page.anim_key();
        for e in anim::take_events() {
            if let (true, Some(js)) = (e.key == key, page.js.as_mut()) {
                js.user_event(e.node as i64, e.kind, &e.init());
            }
        }
        // Scroll and resize events, IntersectionObserver and ResizeObserver.
        if let Some(js) = page.js.as_mut() {
            js.after_layout();
        }
        // Images: lazy ones scrolling into reach, and animation frames (page_images.rs).
        let it = page.images_tick(&lay, scroll, h.saturating_sub(bar_h()) as f32);
        if it.relayout {
            lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
        }
        if it.relayout || it.repaint {
            page_version += 1;
            dirty = true;
            animating |= it.repaint;
        }
        // Live iframes: new pictures, and links they open in the window or a new tab.
        let fr = frames::pump(page.doc, &page.url, &mut page.images);
        if fr.repaint {
            page_version += 1;
            dirty = true;
        }
        // A page in a site process: its title, its forms, and its size follows the window.
        frames::fit(page.doc, &lay);
        if let Some(t) = fr.title {
            page.title = t;
            window.set_title(&format!("{} — Ironswift{}", page.label(), if is_private() { " (private)" } else { "" }));
            dirty = true;
        }
        if let Some((u, body)) = fr.post.filter(|(t, _)| security::page_may_navigate(&page.url, t)) {
            pending_nav = Some(Go::Post(u, body));
        }
        if let Some(t) = fr.navigate.filter(|t| security::page_may_navigate(&page.url, t)) {
            pending_nav = Some(Go::To(t));
        }
        if let Some(t) = fr.open_tab.filter(|t| security::page_may_navigate(&page.url, t)) {
            tab_cmd = Some(TabCmd::Open(t, false));
        }
        if let Some(target) = js::take_navigation() {
            let target = net::resolve(&page.url, &target);
            if security::page_may_navigate(&page.url, &target) {
                navigate = Some(Go::To(target));
            }
        }
        for (id, kind, url) in js::take_permission_requests() {
            match perm_queue.iter_mut().find(|p| p.origin == url && p.kind == kind && p.doc == page.doc) {
                Some(p) => p.ids.push(id),
                None => perm_queue.push(permissions::Prompt { origin: url, kind, ids: vec![id], doc: page.doc, x: FIELD_X, y: TOOL_BOTTOM as f32 + 6.0 }),
            }
            dirty = true;
        }
        // Questions from a page you've left go away with it.
        let before = perm_queue.len();
        perm_queue.retain(|p| p.doc == page.doc);
        dirty |= perm_queue.len() != before;
        let mut form_changed = false;
        // Requests from scripts: focus(), blur(), form.submit(), form.reset().
        for req in js::take_requests() {
            match req {
                js::Request::Focus(n) if js::with_dom(|d| editing::host_of(d, n) == Some(n)) => {
                    editing::set_host(Some(n));
                    ce = Some(js::with_dom_mut(|d| editing::caret_at_end(d, n)));
                    focused = None;
                    form_changed = true;
                }
                js::Request::Blur(n) if ce.is_some_and(|c| c.host == n) => {
                    stop_editing(&mut ce);
                    form_changed = true;
                }
                js::Request::Focus(n) if js::with_dom(|d| forms::control(d, n).is_some() || a11y::is_focusable(d, n)) => {
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
        let view_h = h.saturating_sub(bar_h()) as f32;
        // Requests from screen readers: focus, press, scroll to or fill in an element; the
        // browser's own buttons and tabs.
        if let Some(bridge) = a11y.as_mut() {
            let max_scroll = (lay.height - view_h).max(0.0);
            if bridge.wants_frame() {
                dirty = true;
            }
            for req in bridge.take_requests() {
                use a11y_bridge::Request as R;
                match req {
                    R::Focus(n) | R::ScrollTo(n) | R::Click(n) if n >= js::with_dom(|d| d.nodes.len()) => {}
                    R::Focus(n) => {
                        if focused != Some(n) {
                            if let Some(old) = focused {
                                fire(&mut page, old, "blur");
                            }
                            focused = Some(n);
                            fire(&mut page, n, "focus");
                            form_changed = true;
                        }
                        editing = false;
                        focus_visible = true;
                        if let Some((r, fixed)) = js::with_dom(|d| keynav::element_rect(&lay, d, n)) {
                            if let Some(s) = keynav::scroll_into_view(r, fixed, scroll, view_h, max_scroll) {
                                scroll = s;
                            }
                        }
                        dirty = true;
                    }
                    R::ScrollTo(n) => {
                        if let Some((r, fixed)) = js::with_dom(|d| keynav::element_rect(&lay, d, n)) {
                            if let Some(s) = keynav::scroll_into_view(r, fixed, scroll, view_h, max_scroll) {
                                scroll = s;
                                dirty = true;
                            }
                        }
                    }
                    R::Click(n) => {
                        // Bring it on screen, then click its middle as the mouse would.
                        if let Some((r, fixed)) = js::with_dom(|d| keynav::element_rect(&lay, d, n)) {
                            if let Some(s) = keynav::scroll_into_view(r, fixed, scroll, view_h, max_scroll) {
                                scroll = s;
                            }
                            key_click = Some(keynav::center_on_screen(r, fixed, bar_h() as f32, scroll));
                            dirty = true;
                        }
                    }
                    R::SetValue(n, v) => {
                        if js::with_dom(|d| forms::is_editable(d, n)) {
                            js::with_dom_mut(|d| forms::set_value(d, n, &v));
                            fire(&mut page, n, "input");
                            fire(&mut page, n, "change");
                            form_changed = true;
                        }
                    }
                    R::Back => navigate = Some(Go::Back),
                    R::Forward => navigate = Some(Go::Forward),
                    R::Reload => navigate = Some(Go::Reload),
                    R::FocusAddress => {
                        editing = true;
                        aedit = textedit::TextEdit::select_all(&address);
                        dirty = true;
                    }
                    R::Tab(i) if i < tabs.len() => tab_cmd = Some(TabCmd::Switch(i)),
                    R::Tab(_) => {}
                }
            }
        }
        if let Some(offer) = autofill::take_offer() {
            save_prompt = Some(formui::SavePrompt { offer, hover: None });
            dirty = true;
        }
        if suggest.as_ref().is_some_and(|sg| sg.doc != page.doc || focused != Some(sg.node)) {
            suggest = None;
            dirty = true;
        }
        // Files picked in the file chooser go into their input (if its page is still shown).
        if let Some(chosen) = file_pick.as_ref().and_then(|p| p.poll()) {
            let p = file_pick.take().unwrap();
            if let (Some(paths), true) = (chosen, p.doc == page.doc) {
                let files: Vec<validation::ChosenFile> = paths.iter().filter_map(|p| validation::ChosenFile::from_path(p)).collect();
                js::with_dom_mut(|d| {
                    d.form_state.files.insert(p.node, files);
                });
                fire(&mut page, p.node, "input");
                fire(&mut page, p.node, "change");
                form_changed = true;
                dirty = true;
            }
        }
        // A blocked submission or reportValidity(): focus the control and say what's wrong.
        if let Some(id) = validation::take_report() {
            let text = js::with_dom(|d| {
                let v = validation::validity(d, &d.form_state, id);
                validation::message(d, &d.form_state, id, &v)
            });
            if !text.is_empty() {
                if focused != Some(id) {
                    if let Some(old) = focused {
                        fire(&mut page, old, "blur");
                    }
                    focused = Some(id);
                    fire(&mut page, id, "focus");
                    form_changed = true;
                }
                if let Some((r, _)) = js::with_dom(|d| geometry::page_rect(d, id)) {
                    if r.y < scroll || r.y + r.h + 60.0 > scroll + view_h {
                        scroll = (r.y - view_h / 3.0).max(0.0);
                    }
                }
                bubble = Some(formui::Bubble::new(id, text));
                dirty = true;
            }
        }
        if bubble.as_ref().is_some_and(|b| b.expired() || focused != Some(b.node) || js::with_dom(|d| validation::validity(d, &d.form_state, b.node).valid())) {
            bubble = None;
            dirty = true;
        }
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
        if escape && folder_menu.take().is_some() {
            dirty = true;
        }
        // Tab shortcuts: Ctrl+T new, Ctrl+W close, Ctrl+(Shift+)Tab next/previous,
        // Find, print, save as PDF, view source, developer tools.
        for p in &presses {
            match p.key {
                Key::F if p.ctrl => {
                    let q = find.as_ref().map(|f| f.query.clone()).unwrap_or_default();
                    find = Some(chrome::Find { query: q.clone(), ..Default::default() });
                    find_edit = textedit::TextEdit::select_all(&q);
                    runs_cache.0 = u64::MAX;
                    editing = false;
                    dirty = true;
                }
                Key::P if p.ctrl => ui_action = Some(Ctx::Print),
                Key::S if p.ctrl => ui_action = Some(Ctx::SavePdf),
                Key::U if p.ctrl => ui_action = Some(Ctx::ViewSource),
                Key::I if p.ctrl && p.shift => ui_action = Some(Ctx::Inspect),
                Key::N if p.ctrl && p.shift => open_private_window(),
                Key::F12 => ui_action = Some(Ctx::Inspect),
                Key::F3 if find.is_some() => ui_action = Some(if p.shift { Ctx::FindPrev } else { Ctx::FindNext }),
                Key::F3 => ui_action = Some(Ctx::Find),
                _ => {}
            }
        }
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
        let finding = find.is_some() && !editing;
        let typing = !finding && !editing && focused.is_some_and(|f| js::with_dom(|d| forms::is_editable(d, f)));
        // keydown/keyup for the page; a cancelled keydown doesn't reach the browser or field.
        let mut chars = chars;
        // A live iframe with the keyboard gets the typing (frames.rs); browser shortcuts stay here.
        if frame_focus.as_ref().is_some_and(|k| !frames::has(page.doc, k)) {
            frame_focus = None;
        }
        if frame_focus.is_none() {
            frame_focus = frames::site_frame(page.doc);
        }
        if let (Some(k), false, false) = (&frame_focus, finding, editing) {
            let mut text: String = chars.drain(..).collect();
            if presses.iter().any(|p| p.ctrl && p.key == Key::V) {
                if let Some(t) = clipboard.as_mut().and_then(|c| c.get_text().ok()) {
                    text.push_str(&t);
                }
            }
            let mine: Vec<_> = presses.iter().filter(|p| frames::for_frame(p.key, p.ctrl)).map(|p| (p.key, p.ctrl, p.shift, p.alt)).collect();
            presses.retain(|p| !frames::for_frame(p.key, p.ctrl) && !(p.ctrl && p.key == Key::V));
            if !mine.is_empty() || !text.is_empty() {
                frames::keys(page.doc, k, mine, text);
            }
        }
        if !finding && !editing {
            if let Some(js) = page.js.as_mut() {
                let mut typed_cancelled = false;
                presses.retain(|p| {
                    let ok = userinput::key(js, p.key, false, false, userinput::Mods { ctrl: p.ctrl, shift: p.shift, alt: p.alt });
                    typed_cancelled |= !ok && userinput::names(p.key, p.shift).0.chars().count() == 1;
                    ok
                });
                if typed_cancelled {
                    chars.clear();
                }
                let mods = userinput::Mods {
                    ctrl: window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl),
                    shift: window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift),
                    alt: window.is_key_down(Key::LeftAlt) || window.is_key_down(Key::RightAlt),
                };
                for k in window.get_keys_released() {
                    userinput::key(js, k, true, false, mods);
                }
            }
        }
        let ctrl_down = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
        let shift_down = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
        let edit_keys_now: Vec<KeyPress> = if typing || editing || finding { presses.clone() } else { Vec::new() };
        if runs_cache.0 != page_version && (find.is_some() || selection.is_some()) {
            runs_cache = (page_version, chrome::text_runs(&lay));
            if let Some(f) = find.as_mut() {
                f.search(&runs_cache.1);
            }
        }
        if finding {
            // Typing goes into the find bar; Enter finds the next match, Esc closes it.
            find_edit.sync(&find.as_ref().unwrap().query);
            let r = edit_keys(&mut find_edit, &edit_keys_now, &chars, ctrl_down, false, &mut clipboard);
            let f = find.as_mut().unwrap();
            let mut jump = false;
            if r.changed {
                f.query = find_edit.text.clone();
                f.current = 0;
                f.search(&runs_cache.1);
                jump = true;
            }
            if r.enter {
                f.next(shift_down);
                jump = true;
            }
            if jump {
                if let Some((y, false)) = f.current_y(&runs_cache.1) {
                    let view = h.saturating_sub(bar_h()) as f32 - chrome::FIND_H;
                    if y < scroll + 20.0 || y > scroll + view - 40.0 {
                        scroll = (y - view / 3.0).max(0.0);
                    }
                }
            }
            if r.changed || r.moved || r.enter {
                dirty = true;
            }
            if escape {
                find = None;
                dirty = true;
            }
        }
        if let (Some(c), false, false) = (ce.as_mut(), finding, editing) {
            if contenteditable_keys(&mut page, fonts, c, &presses, &chars, ctrl_down, &mut clipboard) {
                form_changed = true;
                reflow = true;
                dirty = true;
            }
        }
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
                // Earlier entries that start with what's typed so far.
                suggest = open_suggest(&page, &lay, f);
                dirty = true;
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
            if escape || presses.iter().any(|p| p.key == Key::F6) {
                editing = false;
                address = page.url.clone();
                suggestions.clear();
                dirty = true;
                // F6 goes on from the address bar to the toolbar.
                if !escape {
                    ui_focus = Some(0);
                }
            }
        } else if !finding {
            stop_key = escape;
            for p in &presses {
                let (key, ctrl, alt) = (p.key, p.ctrl, p.alt);
                let before = scroll;
                // An open menu or downloads panel takes the arrow keys, Enter and Space.
                if ctx_menu.is_some() || dl_panel.is_some() {
                    let (rows, hover, center) = match (&ctx_menu, &dl_panel) {
                        (Some(m), _) => (m.actionable(), m.hover, m.hover.and_then(|r| m.row_center(r))),
                        (None, Some(m)) => (m.actionable(), m.hover, m.hover.and_then(|r| m.row_center(r))),
                        _ => (Vec::new(), None, None),
                    };
                    let moved = match key {
                        Key::Down => Some(uikeys::menu_step(&rows, hover, 1)),
                        Key::Up => Some(uikeys::menu_step(&rows, hover, -1)),
                        Key::Home => Some(uikeys::menu_step(&rows, None, 1)),
                        Key::End => Some(uikeys::menu_step(&rows, None, -1)),
                        _ => None,
                    };
                    if let Some(h) = moved {
                        if let Some(m) = ctx_menu.as_mut() {
                            m.hover = h;
                        } else if let Some(m) = dl_panel.as_mut() {
                            m.hover = h;
                        }
                        dirty = true;
                        continue;
                    }
                    if matches!(key, Key::Enter | Key::NumPadEnter | Key::Space) {
                        key_click = center;
                        continue;
                    }
                }
                // Keyboard focus in the toolbar (reached with F6).
                if let Some(i) = ui_focus {
                    let targets = ui_targets(fonts, w as f32, tabs.len(), if BOOKMARKS_BAR.load(std::sync::atomic::Ordering::Relaxed) { &bm_list } else { &[] }, !downloads::list().is_empty(), perm_queue.first());
                    let i = i.min(targets.len().saturating_sub(1));
                    match key {
                        Key::Right | Key::Left | Key::Tab => {
                            // "Next" goes the way the UI reads.
                            let forward = match key {
                                Key::Tab => !p.shift,
                                Key::Right => !rtl::on(),
                                _ => rtl::on(),
                            };
                            ui_focus = uikeys::step(Some(i), targets.len(), forward);
                        }
                        Key::Enter | Key::NumPadEnter | Key::Space => {
                            if let Some(&t) = targets.get(i) {
                                key_click = Some(uikeys::click_point(t, w as f32));
                            }
                        }
                        Key::Escape | Key::F6 => {
                            ui_focus = None;
                            focus_visible = focused.is_some();
                        }
                        _ => {}
                    }
                    dirty = true;
                    continue;
                }
                if ce.is_some() && editing_key(key, ctrl, alt) {
                    continue;
                }
                // Keys for the focused control come first.
                if let Some(f) = focused {
                    let kind = js::with_dom(|d| forms::control(d, f));
                    let handled = match key {
                        Key::Tab if !ctrl => {
                            let shift = p.shift;
                            let all = js::with_dom(|d| focusable(d, &page.styles));
                            let i = all.iter().position(|&n| n == f).unwrap_or(0);
                            let next = if shift { (i + all.len() - 1) % all.len().max(1) } else { (i + 1) % all.len().max(1) };
                            if let Some(&n) = all.get(next) {
                                fire(&mut page, f, "blur");
                                focused = Some(n);
                                fire(&mut page, n, "focus");
                                focus_visible = true;
                                if let Some((r, fixed)) = js::with_dom(|d| keynav::element_rect(&lay, d, n)) {
                                    if let Some(s) = keynav::scroll_into_view(r, fixed, scroll, view_h, max_scroll) {
                                        scroll = smooth.to(scroll, s);
                                    }
                                }
                            }
                            dropdown = None;
                            picker = None;
                            suggest = None;
                            stop_editing(&mut ce);
                            form_changed = true;
                            true
                        }
                        // Enter (and Space, except on links) activates a focused link, button,
                        // <summary> or tabindex element as a click would.
                        Key::Enter | Key::NumPadEnter | Key::Space
                            if !matches!(kind, Some(forms::Control::Text { .. } | forms::Control::TextArea | forms::Control::Checkbox | forms::Control::Radio | forms::Control::Select))
                                && !(key == Key::Space && js::with_dom(|d| a11y::role_of(d, f)) == "link") =>
                        {
                            if let Some((r, fixed)) = js::with_dom(|d| keynav::element_rect(&lay, d, f)) {
                                key_click = Some(keynav::center_on_screen(r, fixed, bar_h() as f32, scroll));
                            }
                            true
                        }
                        Key::Escape => {
                            fire(&mut page, f, "blur");
                            focused = None;
                            dropdown = None;
                            picker = None;
                            suggest = None;
                            stop_editing(&mut ce);
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
                        Key::Space | Key::Enter if kind == Some(forms::Control::Other) && js::with_dom(|d| forms::is_file(d, f)) => {
                            if file_pick.is_none() {
                                match open_file_picker(&page, f) {
                                    Ok(p) => file_pick = Some(p),
                                    Err(e) => toast = Some((e, Instant::now())),
                                }
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
                    // Nothing focused yet: Tab starts at the first focusable element (Shift+Tab
                    // at the last).
                    Key::Tab if !ctrl && focused.is_none() => {
                        let all = js::with_dom(|d| focusable(d, &page.styles));
                        let pick = if p.shift { all.last() } else { all.first() };
                        if let Some(&n) = pick {
                            focused = Some(n);
                            fire(&mut page, n, "focus");
                            focus_visible = true;
                            form_changed = true;
                            if let Some((r, fixed)) = js::with_dom(|d| keynav::element_rect(&lay, d, n)) {
                                if let Some(s) = keynav::scroll_into_view(r, fixed, scroll, view_h, max_scroll) {
                                    scroll = smooth.to(scroll, s);
                                }
                            }
                        }
                    }
                    Key::Down => scroll = smooth.by(scroll, 48.0),
                    Key::Up => scroll = smooth.by(scroll, -48.0),
                    Key::PageDown | Key::Space => scroll = smooth.by(scroll, view_h * 0.9),
                    Key::PageUp => scroll = smooth.by(scroll, -view_h * 0.9),
                    Key::Home => scroll = smooth.to(scroll, 0.0),
                    Key::End => scroll = smooth.to(scroll, max_scroll),
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
                    // Zoom: Ctrl + / Ctrl − / Ctrl 0, remembered for the site.
                    Key::Equal | Key::NumPadPlus | Key::Minus | Key::NumPadMinus | Key::Key0 | Key::NumPad0 if ctrl => {
                        let z = page.zoom();
                        let nz = match key {
                            Key::Equal | Key::NumPadPlus => zoom::step(z, true),
                            Key::Minus | Key::NumPadMinus => zoom::step(z, false),
                            _ => 1.0,
                        };
                        if (nz - z).abs() > 1e-4 {
                            // Keep the same part of the page in view.
                            scroll = scroll / z * nz;
                            zoom::set(&page.url, nz);
                            selection = None;
                            reflow = true;
                            toast = Some((i18n::tf("toast-zoom", &[("pct", &format!("{:.0}", nz * 100.0))]), Instant::now()));
                        }
                    }
                    Key::A if ctrl && !typing => {
                        selection = Some((page.doc, chrome::Selection { all: true, ..Default::default() }));
                        runs_cache.0 = u64::MAX;
                        dirty = true;
                    }
                    Key::C if ctrl && !typing => ui_action = Some(Ctx::CopySelection),
                    Key::Escape if selection.is_some() || ctx_menu.is_some() || dl_panel.is_some() => {
                        selection = None;
                        ctx_menu = None;
                        dl_panel = None;
                        dirty = true;
                    }
                    Key::Backspace => navigate = Some(Go::Back),
                    // Shift+F10 or the Menu key: the right-click menu for the focused element
                    // (or the page), ready for the arrow keys.
                    Key::Menu | Key::F10 if key == Key::Menu || p.shift => {
                        let at = focused
                            .and_then(|f| js::with_dom(|d| keynav::element_rect(&lay, d, f)))
                            .map(|(r, fixed)| keynav::center_on_screen(r, fixed, bar_h() as f32, scroll))
                            .unwrap_or((40.0, bar_h() as f32 + 40.0));
                        let mut m = context_menu(&page, &lay, &runs_cache.1, fonts, selection.map(|s| s.1), at.0, at.1, at.1 - bar_h() as f32, scroll, !back.is_empty(), !forward.is_empty(), w as f32, h as f32);
                        m.hover = uikeys::menu_step(&m.actionable(), None, 1);
                        ctx_menu = Some(m);
                        dirty = true;
                    }
                    Key::Left if alt => navigate = Some(Go::Back),
                    Key::Right if alt => navigate = Some(Go::Forward),
                    _ => {}
                }
                if scroll != before {
                    dirty = true;
                }
            }
        }
        let ctrl_held = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
        if let Some((_, dy)) = window.get_scroll_wheel().filter(|_| ctrl_held) {
            // Ctrl+wheel zooms.
            if dy != 0.0 {
                let z = page.zoom();
                let nz = zoom::step(z, dy > 0.0);
                if (nz - z).abs() > 1e-4 {
                    scroll = scroll / z * nz;
                    zoom::set(&page.url, nz);
                            selection = None;
                    reflow = true;
                    toast = Some((i18n::tf("toast-zoom", &[("pct", &format!("{:.0}", nz * 100.0))]), Instant::now()));
                }
            }
        } else if let Some((_, dy)) = window.get_scroll_wheel().filter(|&(_, dy)| {
            // Over a live iframe that can scroll that way, the frame scrolls (frames.rs).
            let mouse = window.get_mouse_pos(MouseMode::Discard).filter(|&(_, my)| my >= bar_h() as f32);
            let fr = mouse.and_then(|(mx, my)| frames::frame_at(page.doc, &lay, mx, my - bar_h() as f32 + scroll));
            !fr.is_some_and(|(k, fx, fy)| frames::wheel(page.doc, &k, fx, fy, -dy * 22.0 / page.zoom()))
        }) {
            // About 45px per wheel click (minifb reports roughly 2 per click on X11).
            let delta = -dy * 22.0;
            // The page may cancel the wheel event (maps, carousels); then nothing scrolls.
            let mouse = window.get_mouse_pos(MouseMode::Discard).filter(|&(_, my)| my >= bar_h() as f32);
            let z = page.zoom();
            let wanted = match (page.js.as_mut(), mouse) {
                (Some(js), Some((mx, my))) => {
                    let node = lay.hit_at(mx, my - bar_h() as f32, scroll);
                    userinput::wheel(js, node, mx / z, (my - bar_h() as f32) / z, 0.0, delta / z, userinput::Mods::default())
                }
                _ => true,
            };
            // The wheel scrolls the innermost scrollable box under the mouse, else the page.
            let inner = mouse.filter(|_| wanted).and_then(|(mx, my)| lay.scroller_at(mx, my - bar_h() as f32, scroll, delta));
            match inner {
                Some((id, max)) => {
                    layout::set_element_scroll(id, (layout::element_scroll(id) + delta).clamp(0.0, max));
                    if let Some(js) = page.js.as_mut() {
                        userinput::scrolled(js, id);
                    }
                    reflow = true;
                }
                None if wanted => {
                    scroll = smooth.by(scroll, delta);
                    dirty = true;
                }
                None => {}
            }
        }
        if let Some(y) = geometry::take_scroll_request() {
            scroll = y * page.scale();
            dirty = true;
        }
        scroll = scroll.clamp(0.0, max_scroll);
        if let Some(y) = smooth.step(scroll, max_scroll) {
            scroll = y;
            dirty = true;
        }
        scroll = scroll.clamp(0.0, max_scroll);
        geometry::set_scroll(scroll / page.scale());

        // Mouse: hover cursor and clicks.
        // A keyboard activation is a press at the element's middle for this one frame.
        let synthetic = key_click.take();
        if synthetic.is_some() {
            was_down = false;
        } else if window.get_mouse_down(MouseButton::Left) && !was_down {
            focus_visible = false;
        }
        let down = synthetic.is_some() || window.get_mouse_down(MouseButton::Left);
        let mid = window.get_mouse_down(MouseButton::Middle);
        let right = window.get_mouse_down(MouseButton::Right);
        if let Some((mx, my)) = synthetic.or_else(|| window.get_mouse_pos(MouseMode::Discard)) {
            // Right to left, the browser UI is mirrored: over it, x is flipped so the hit tests
            // below work in their usual left-to-right coordinates. Pages and the menus that
            // open where they're asked to keep the real x.
            let over_ui = my < bar_h() as f32
                || (editing && !suggestions.is_empty() && my < TOOL_BOTTOM as f32 + suggestions.len() as f32 * SUGGEST_H)
                || folder_menu.is_some()
                || perm_queue.first().is_some_and(|p| p.contains(w as f32 - mx, my));
            let mx = if over_ui { rtl::ui_x(mx, w as f32) } else { mx };
            // Menus follow the mouse only when it moves (so arrow keys can drive them); a real
            // click ends keyboard focus in the toolbar.
            let mouse_moved = (mx, my) != last_mouse;
            last_mouse = (mx, my);
            if down && !was_down && synthetic.is_none() && ui_focus.take().is_some() {
                dirty = true;
            }
            let sh = (editing && my >= TOOL_BOTTOM as f32 && mx >= FIELD_X).then(|| ((my - TOOL_BOTTOM as f32) / SUGGEST_H) as usize).filter(|&i| i < suggestions.len());
            if sh != sugg_hover {
                sugg_hover = sh;
                dirty = true;
            }
            let th = tab_at(mx, my, w as f32, tabs.len());
            if th != tab_hover {
                tab_hover = th;
                dirty = true;
            }
            // Tab dragging: once the mouse has moved a few pixels, the active tab follows it.
            if !down {
                tab_drag = None;
            } else if let Some((x0, moving)) = tab_drag {
                if moving || (mx - x0).abs() > 6.0 {
                    tab_drag = Some((x0, true));
                    let rects = tab_rects(w as f32, tabs.len());
                    let target = match rects.iter().position(|&(tx, tw)| mx >= tx && mx < tx + tw) {
                        Some(t) => t,
                        None if mx < rects.first().map_or(0.0, |r| r.0) => 0,
                        None => tabs.len() - 1,
                    };
                    if target != active && tab_cmd.is_none() {
                        let slot = tabs.remove(active);
                        tabs.insert(target, slot);
                        active = target;
                        dirty = true;
                    }
                }
            }
            // Which bookmarks-bar chip is under the mouse.
            let bh = (my >= TOOL_BOTTOM as f32 && my < bar_h() as f32)
                .then(|| {
                    let rects = bookmark_rects(fonts, w as f32, &bm_list);
                    match rects.iter().position(|&(x, cw)| mx >= x && mx < x + cw) {
                        Some(i) => Some(i),
                        None if rects.len() < bar_items(&bm_list).len() && mx >= w as f32 - 8.0 - BOOK_MORE_W => Some(usize::MAX),
                        None => None,
                    }
                })
                .flatten();
            if bh != bm_hover {
                bm_hover = bh;
                dirty = true;
            }
            // Middle click: close a tab, or open a link in a background tab (an open folder
            // menu handles its own clicks).
            if mid && !was_mid && folder_menu.is_none() {
                match th {
                    Some(TabHit::Tab(i) | TabHit::Close(i)) => tab_cmd = Some(TabCmd::Close(i)),
                    _ if bm_hover.is_some_and(|i| i != usize::MAX) => {
                        let items = bar_items(&bm_list);
                        if let Some(BarItem::Link(j)) = items.get(bm_hover.unwrap()) {
                            tab_cmd = Some(TabCmd::Open(bm_list[*j].url.clone(), true));
                        }
                    }
                    _ if my >= bar_h() as f32 => {
                        if let Some(href) = lay.link_at(mx, my - bar_h() as f32, scroll) {
                            let target = net::resolve(&page.url, href);
                            if !target.starts_with("javascript:") && !target.starts_with("mailto:") && security::page_may_navigate(&page.url, &target) {
                                tab_cmd = Some(TabCmd::Open(target, true));
                            }
                        }
                    }
                    _ => {}
                }
            }
            let view_y = my - bar_h() as f32;
            // Over a live iframe, the mouse belongs to the frame's document (frames.rs).
            let over_frame = if my >= bar_h() as f32 && ctx_menu.is_none() { frames::frame_at(page.doc, &lay, mx, view_y + scroll) } else { None };
            if over_frame.as_ref().map(|f| &f.0) != frame_hover.as_ref() {
                if let Some(k) = frame_hover.take() {
                    frames::leave(page.doc, &k);
                }
                frame_hover = over_frame.as_ref().map(|f| f.0.clone());
            }
            if let Some((k, fx, fy)) = &over_frame {
                frames::mouse(page.doc, k, *fx, *fy, [down, mid, right]);
            }
            let hit = if my >= bar_h() as f32 && over_frame.is_none() { lay.link_at(mx, view_y, scroll).cloned() } else { None };
            // Mouse events for the page's scripts (moves, hover changes, button presses).
            let mut mousedown_cancelled = false;
            if pointer.changed(mx, my, [down, mid, right]) {
                let z = page.zoom();
                if let Some(js) = page.js.as_mut() {
                    if my >= bar_h() as f32 && ctx_menu.is_none() {
                        let node = lay.hit_at(mx, view_y, scroll);
                        let mods = userinput::Mods { ctrl: ctrl_down, shift: shift_down, alt: false };
                        mousedown_cancelled = pointer.update(js, node, mx / z, view_y / z, [down, mid, right], mods);
                    } else {
                        pointer.left(js);
                    }
                }
            }
            // The status bar shows where a link goes.
            let status = hit.as_ref().map(|h| net::resolve(&page.url, h)).filter(|h| !h.starts_with("javascript:"));
            if status != status_link {
                status_link = status;
                dirty = true;
            }
            // Dragging over the page selects text.
            if down && was_down {
                if let Some(p0) = sel_press {
                    let p1 = (mx, view_y + scroll);
                    if (p1.0 - p0.0).abs() > 4.0 || (p1.1 - p0.1).abs() > 4.0 || selection.is_some() {
                        selection = Some((page.doc, chrome::Selection { from: p0, to: p1, all: false }));
                        runs_cache.0 = runs_cache.0.min(u64::MAX - 1);
                        dirty = true;
                    }
                }
            } else if !down {
                sel_press = None;
            }
            // Right click: the context menu for what's under the mouse.
            let page_menu = right && !was_right && my >= bar_h() as f32 && {
                // The page's scripts see contextmenu first and may show their own menu instead.
                let z = page.zoom();
                match (lay.hit_at(mx, view_y, scroll), page.js.as_mut()) {
                    (Some(n), Some(js)) => clicks::contextmenu(js, n, mx / z, view_y / z),
                    _ => true,
                }
            };
            if page_menu {
                ctx_menu = Some(context_menu(&page, &lay, &runs_cache.1, fonts, selection.map(|s| s.1), mx, my, view_y, scroll, !back.is_empty(), !forward.is_empty(), w as f32, h as f32));
                dl_panel = None;
                dirty = true;
            }
            for (open, m) in [(ctx_menu.is_some(), 0), (dl_panel.is_some(), 1)] {
                if !open || !(mouse_moved || down) {
                    continue;
                }
                let row = if m == 0 { ctx_menu.as_ref().unwrap().row_at(mx, my) } else { dl_panel.as_ref().unwrap().row_at(mx, my) };
                let cur = if m == 0 { &mut ctx_menu.as_mut().unwrap().hover } else { &mut dl_panel.as_mut().unwrap().hover };
                if row != *cur {
                    *cur = row;
                    dirty = true;
                }
            }
            let btn = button_at(mx, my);
            // :hover — restyle when the element under the mouse changes.
            if page.hover_rules {
                let over = if my >= bar_h() as f32 { lay.hit_at(mx, view_y, scroll) } else { None };
                if over != hovered {
                    hovered = over;
                    style::set_hover(over);
                    restyle = true;
                    reflow = true;
                }
            }
            // :active — the element the left button is held down on.
            if style::uses_active() {
                let now = if down && my >= bar_h() as f32 { lay.hit_at(mx, view_y, scroll) } else { None };
                if now != active_node {
                    active_node = now;
                    style::set_active(now);
                    restyle = true;
                    reflow = true;
                }
            }
            if btn != hover {
                hover = btn;
                dirty = true;
            }
            // The page's own `cursor` property, for what's under the mouse (nearest
            // ancestor with one set, since the property is inherited).
            let page_cursor = if my >= bar_h() as f32 && hit.is_none() && btn.is_none() {
                lay.hit_at(mx, view_y, scroll).and_then(|n| page.styles.get(n)).map(|s| s.cursor).unwrap_or(0)
            } else {
                0
            };
            window.set_cursor_style(if hit.is_some() || btn.is_some() || page_cursor == 1 {
                CursorStyle::ClosedHand
            } else if page_cursor == 2 {
                CursorStyle::Ibeam
            } else if page_cursor == 5 {
                CursorStyle::ResizeAll
            } else if my >= TAB_H && (my as usize) < TOOL_BOTTOM && mx >= FIELD_X {
                CursorStyle::Ibeam
            } else {
                CursorStyle::Arrow
            });
            // An open bookmarks folder: hover, then a click opens a bookmark or closes the menu.
            if let Some(menu) = folder_menu.as_mut() {
                let row = menu.row_at(&bm_list, w as f32, mx, my);
                if row != menu.hover {
                    menu.hover = row;
                    dirty = true;
                }
                if (mid && !was_mid) || (down && !was_down) {
                    let entries: Vec<String> = menu.entries(&bm_list).iter().map(|b| b.url.clone()).collect();
                    let on_chip = bm_hover.is_some_and(|i| bar_items(&bm_list).get(i) == Some(&BarItem::Folder(menu.folder.clone())));
                    if let Some(url) = row.and_then(|r| entries.get(r).cloned()) {
                        if mid || ctrl_down {
                            tab_cmd = Some(TabCmd::Open(url, true));
                        } else {
                            navigate = Some(Go::To(url));
                        }
                    }
                    folder_menu = None;
                    dirty = true;
                    if row.is_some() || on_chip {
                        // The click was for the menu (or closed it from its chip): nothing else.
                        was_down = down;
                    }
                }
            }
            // Highlight the option under the mouse in an open list.
            if let Some(sg) = suggest.as_mut() {
                let row = sg.row_at(mx, my, sg.y - scroll + bar_h() as f32);
                if row != sg.hover {
                    sg.hover = row;
                    dirty = true;
                }
            }
            if let Some(sp) = save_prompt.as_mut() {
                dirty |= sp.hover_at(mx, my, w as f32, bar_h() as f32);
            }
            if let Some(p) = picker.as_mut() {
                if p.hover != Some((mx, my)) {
                    p.hover = Some((mx, my));
                    dirty = true;
                }
            }
            if let Some(dd) = dropdown.as_mut() {
                let row = dd.row_at(mx, my - bar_h() as f32 + scroll);
                if row != dd.hover {
                    dd.hover = row;
                    dirty = true;
                }
            }
            // A click on a permission question: Allow or Block (remembered for the site).
            if down && !was_down && perm_queue.first().is_some_and(|p| p.contains(mx, my)) {
                if let Some(allow) = perm_queue[0].button_at(mx, my) {
                    let p = perm_queue.remove(0);
                    permissions::set(&p.origin, p.kind, allow);
                    if let Some(js) = page.js.as_mut() {
                        js.permission_done(&p.ids, allow);
                    }
                    if js::take_dirty() {
                        page.restyle();
                        lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
                        page_version += 1;
                    }
                }
                was_down = down;
                dirty = true;
            }
            // A click with a menu open: do the item, or just close the menu.
            if down && !was_down && (ctx_menu.is_some() || dl_panel.is_some()) {
                let on_chip = my >= TAB_H && (my as usize) < TOOL_BOTTOM && mx >= w as f32 - 8.0 - STAR_W - DL_W && mx < w as f32 - 8.0 - STAR_W;
                if let Some(m) = ctx_menu.take() {
                    if let Some(a) = m.row_at(mx, my).and_then(|r| m.action(r)) {
                        ui_action = Some(a);
                    }
                    was_down = down;
                } else if let Some(m) = dl_panel.take() {
                    if let Some(a) = m.row_at(mx, my).and_then(|r| m.action(r)) {
                        match a {
                            DlAct::Open(id) => drop(downloads::page(&format!("open={id}"))),
                            DlAct::Folder => drop(downloads::page("folder")),
                            DlAct::ShowAll => tab_cmd = Some(TabCmd::Open("about:downloads".into(), false)),
                        }
                    }
                    if m.contains(mx, my) || on_chip {
                        was_down = down;
                    }
                }
                dirty = true;
            }
            // A click in an open date/time/color picker; a click elsewhere closes it and goes on.
            let mut picked = false;
            if down && !was_down {
                if let Some(hit) = save_prompt.as_ref().and_then(|sp| sp.hit(mx, my, w as f32, bar_h() as f32)) {
                    picked = true;
                    if let Some(choice) = hit {
                        let offer = save_prompt.take().unwrap().offer;
                        match choice {
                            formui::SaveChoice::Save => autofill::save_login(offer.login),
                            formui::SaveChoice::Never => autofill::never_for(&offer.login.origin),
                            formui::SaveChoice::NotNow => {}
                        }
                        dirty = true;
                    }
                } else if let Some(sg) = suggest.take() {
                    dirty = true;
                    if let Some(i) = sg.row_at(mx, my, sg.y - scroll + bar_h() as f32) {
                        picked = true;
                        let it = &sg.items[i];
                        for (node, value) in &it.fill {
                            js::with_dom_mut(|d| forms::set_value(d, *node, value));
                            if focused == Some(*node) {
                                fedit = Some(textedit::TextEdit::new(value));
                            }
                            fire(&mut page, *node, "input");
                            fire(&mut page, *node, "change");
                        }
                        if let Some((origin, user)) = &it.login {
                            autofill::mark_used(origin, user);
                        }
                        form_changed = true;
                    }
                }
            }
            if down && !was_down && !picked {
                if let Some(p) = picker.as_mut() {
                    let top = p.y - scroll + bar_h() as f32;
                    match p.click(mx, my, top) {
                        formui::Pick::Outside => picker = None,
                        formui::Pick::Nothing => picked = true,
                        formui::Pick::Set { value, close } => {
                            let node = p.node;
                            js::with_dom_mut(|d| forms::set_value(d, node, &value));
                            if focused == Some(node) {
                                fedit = Some(textedit::TextEdit::new(&value));
                            }
                            fire(&mut page, node, "input");
                            fire(&mut page, node, "change");
                            if close {
                                picker = None;
                                suggest = None;
                            }
                            form_changed = true;
                            picked = true;
                        }
                    }
                    dirty = true;
                }
            }
            if picked {
            } else if down && !was_down && dropdown.is_some() {
                // A click while a list is open picks an option or closes the list.
                let dd = dropdown.take().unwrap();
                if let Some(i) = dd.row_at(mx, my - bar_h() as f32 + scroll) {
                    let opt = dd.options[i].0;
                    js::with_dom_mut(|d| forms::select_option(d, dd.select, opt));
                    fire(&mut page, dd.select, "input");
                    fire(&mut page, dd.select, "change");
                }
                form_changed = true;
            } else if down && !was_down && editing && !suggestions.is_empty() && my >= TOOL_BOTTOM as f32 && mx >= FIELD_X && my < TOOL_BOTTOM as f32 + suggestions.len() as f32 * SUGGEST_H {
                // A click on an address suggestion opens it.
                let i = ((my - TOOL_BOTTOM as f32) / SUGGEST_H) as usize;
                if let Some(sg) = suggestions.get(i) {
                    navigate = Some(Go::To(sg.url.clone()));
                }
                editing = false;
                suggestions.clear();
                dirty = true;
            } else if down && !was_down && my >= TAB_H && (my as usize) < TOOL_BOTTOM && blocklist::blocking_on(&page.url).is_some() && {
                let sx = shield_x(w as f32, !downloads::list().is_empty());
                mx >= sx && mx < sx + SHIELD_W
            } {
                let sx = shield_x(w as f32, !downloads::list().is_empty());
                ctx_menu = Some(shield_menu(&page.url, if rtl::on() { w as f32 - sx - 300.0 } else { sx + SHIELD_W - 300.0 }, TOOL_BOTTOM as f32 + 2.0, w as f32, h as f32));
                dirty = true;
            } else if down && !was_down && my >= TAB_H && (my as usize) < TOOL_BOTTOM && !downloads::list().is_empty() && mx >= w as f32 - 8.0 - STAR_W - DL_W && mx < w as f32 - 8.0 - STAR_W {
                let px = w as f32 - 8.0 - STAR_W - 330.0;
                dl_panel = Some(downloads_panel(if rtl::on() { w as f32 - px - 330.0 } else { px }, TOOL_BOTTOM as f32 + 2.0, w as f32, h as f32));
                dirty = true;
            } else if down && !was_down && my >= TAB_H && (my as usize) < TOOL_BOTTOM && mx >= w as f32 - 8.0 - STAR_W {
                library::toggle_bookmark(&page.url, &page.title);
                dirty = true;
            } else if down && !was_down && my < TAB_H {
                if matches!(th, Some(TabHit::Tab(_))) {
                    tab_drag = Some((mx, false));
                }
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
                } else if (my as usize) < TOOL_BOTTOM && mx >= FIELD_X {
                    let double = last_click.0.elapsed().as_millis() < 450 && (mx - last_click.1).abs() < 5.0 && (my - last_click.2).abs() < 5.0;
                    if !editing {
                        // First click selects the whole address, ready to type over.
                        editing = true;
                        aedit = textedit::TextEdit::select_all(&address);
                    } else {
                        let full = fonts.width(font::REGULAR, &aedit.text, 14.0);
                        let i = char_index_at(fonts, font::REGULAR, 14.0, &aedit.text, rtl::field_offset(FIELD_X + 8.0, mx, full));
                        if double {
                            aedit.select_word_at(i);
                        } else {
                            aedit.move_to(i, shift_down);
                        }
                    }
                    dirty = true;
                } else if (my as usize) < bar_h() {
                    // Bookmarks bar: open the bookmark (Ctrl: in a background tab), or "»" for all.
                    editing = false;
                    let items = bar_items(&bm_list);
                    match bm_hover {
                        Some(usize::MAX) => navigate = Some(Go::To("about:bookmarks".into())),
                        Some(i) => match items.get(i) {
                            Some(BarItem::Link(j)) => {
                                let url = bm_list[*j].url.clone();
                                if ctrl_down {
                                    tab_cmd = Some(TabCmd::Open(url, true));
                                } else {
                                    navigate = Some(Go::To(url));
                                }
                            }
                            Some(BarItem::Folder(f)) => {
                                let x = bookmark_rects(fonts, w as f32, &bm_list).get(i).map_or(8.0, |r| r.0);
                                folder_menu = Some(FolderMenu { folder: f.clone(), x, hover: None });
                                dirty = true;
                            }
                            None => {}
                        },
                        None => {}
                    }
                } else if let Some((k, _, _)) = &over_frame {
                    // A press inside a live iframe went to the frame (above); it takes the keyboard.
                    if frame_focus.as_ref() != Some(k) {
                        if let Some(old) = frame_focus.replace(k.clone()) {
                            frames::blur(page.doc, &old);
                        }
                    }
                    if let Some(old) = focused.take() {
                        fire(&mut page, old, "blur");
                        form_changed = true;
                    }
                } else if (my as usize) >= bar_h() {
                    if let Some(k) = frame_focus.take() {
                        frames::blur(page.doc, &k);
                    }
                    // A press on the page may start a text selection (and clears the old one).
                    sel_press = Some((mx, view_y + scroll));
                    if selection.take().is_some() {
                        dirty = true;
                    }
                    if editing {
                        editing = false;
                        address = page.url.clone();
                        suggestions.clear();
                        dirty = true;
                    }
                    // The press focuses and places the caret; the click itself (the page's click
                    // handlers, then following a link, checking a box, submitting…) comes on release.
                    let node = lay.hit_at(mx, view_y, scroll);
                    // The focusable element clicked (a link, a tabindex element) takes focus, so
                    // keyboard users keep their place after activating it. Found before the
                    // page's click handlers run, since they may replace the node clicked.
                    let clicked_focusable = {
                        node.and_then(|n| {
                            js::with_dom(|d| {
                                let mut cur = Some(n);
                                while let Some(x) = cur {
                                    if a11y::is_focusable(d, x) {
                                        return Some(x);
                                    }
                                    cur = d.nodes[x].parent;
                                }
                                None
                            })
                        })
                    };
                    // Form controls: focus, check, open a list, submit or reset.
                    let ctrl = node.and_then(|n| js::with_dom(|d| control_for(d, n)));
                    let ctrl = ctrl.filter(|&c| !js::with_dom(|d| forms::is_disabled(d, c)));
                    let focus_to = ctrl.or(clicked_focusable);
                    if focus_to != focused && !mousedown_cancelled {
                        if let Some(old) = focused {
                            fire(&mut page, old, "change");
                            fire(&mut page, old, "blur");
                        }
                        focused = focus_to;
                        if let Some(n) = focus_to {
                            fire(&mut page, n, "focus");
                        }
                        form_changed = true;
                    }
                    // A click in a contenteditable element puts the caret there; elsewhere ends editing.
                    // (Clicks on a plain block hit no node: find the editable box under the mouse.)
                    let host = node.and_then(|n| js::with_dom(|d| editing::host_of(d, n))).or_else(|| {
                        js::with_dom(|d| {
                            d.descendants(dom::Dom::ROOT).into_iter().rev().find(|&n| {
                                editing::host_of(d, n) == Some(n) && lay.boxes.get(&n).is_some_and(|(r, fixed)| {
                                    let py = if *fixed { view_y } else { view_y + scroll };
                                    mx >= r.x && mx < r.x + r.w && py >= r.y && py < r.y + r.h
                                })
                            })
                        })
                    });
                    let host = host.filter(|_| ctrl.is_none() && !mousedown_cancelled);
                    match host {
                        Some(hst) => {
                            if ce.map(|c| c.host) != Some(hst) {
                                // Lay out once with the host known, so its words' positions are recorded.
                                editing::set_host(Some(hst));
                                lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
                                fire(&mut page, hst, "focus");
                            }
                            let z = page.scale();
                            ce = Some(js::with_dom_mut(|d| editing::caret_at_point(d, fonts, hst, mx / z, (view_y + scroll) / z)));
                            form_changed = true;
                            dirty = true;
                        }
                        None if ce.is_some() => {
                            if let Some(c) = ce {
                                fire(&mut page, c.host, "blur");
                            }
                            stop_editing(&mut ce);
                            dirty = true;
                        }
                        None => {}
                    }
                    // A click inside a single-line text field puts the caret where you clicked
                    // (double-click selects a word).
                    if let Some(c) = ctrl.filter(|&c| js::with_dom(|d| forms::is_editable(d, c) && d.tag(c) == Some("input"))) {
                        if let Some(r) = lay.hits.iter().rev().find(|(_, n)| *n == c).map(|(r, _)| *r) {
                            let st = page.styles[c].clone();
                            let fnt = fonts.resolve_weight(st.family.as_deref(), st.font_weight(), st.italic, st.mono);
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
                    if let (false, Some(c)) = (mousedown_cancelled, ctrl) {
                        suggest = open_suggest(&page, &lay, c);
                    }
                    page_press = Some(clicks::Press { node, mx, my, view_y, hit: hit.clone(), synthetic: synthetic.is_some(), doc: page.doc });
                }
            } else if !down && was_down && page_press.is_some() {
                // The release after a press on the page: a click at the element both were over.
                let press = page_press.take().unwrap();
                let up = if press.synthetic { press.node } else if my >= bar_h() as f32 { lay.hit_at(mx, view_y, scroll) } else { None };
                let node = js::with_dom(|d| clicks::target(d, press.node, up));
                if press.doc == page.doc && (press.node.is_none() || node.is_some()) {
                    let hit = if node == press.node { press.hit.clone() } else { None };
                    let (mx, my, view_y) = (press.mx, press.my, press.view_y);
                    // Let the page's JavaScript see the click first.
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
                        restyle = true;
                        reflow = true;
                    }
                    if let Some(t) = js::take_navigation() {
                        let t = net::resolve(&page.url, &t);
                        if security::page_may_navigate(&page.url, &t) {
                            navigate = Some(Go::To(t));
                        }
                    }
                    // Form controls: check, open a list, submit or reset.
                    let ctrl = node.and_then(|n| js::with_dom(|d| control_for(d, n)));
                    let ctrl = ctrl.filter(|&c| !js::with_dom(|d| forms::is_disabled(d, c)));
                    // A second click in the same place is a double click.
                    if last_page_click.record(mx, my) {
                        let z = page.zoom();
                        if let (Some(n), Some(js)) = (node, page.js.as_mut()) {
                            clicks::dblclick(js, n, mx / z, view_y / z);
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
                            Some(forms::Control::Other) if file_pick.is_none() && js::with_dom(|d| forms::is_file(d, c)) => match open_file_picker(&page, c) {
                                Ok(p) => file_pick = Some(p),
                                Err(e) => {
                                    toast = Some((e, Instant::now()));
                                    dirty = true;
                                }
                            },
                            Some(forms::Control::Button { reset: true, .. }) => {
                                if let Some(form) = js::with_dom(|d| forms::form_of(d, c)) {
                                    if fire(&mut page, form, "reset") {
                                        let defaults = page.form_defaults.clone();
                                        js::with_dom_mut(|d| forms::reset(d, form, &defaults));
                                        form_changed = true;
                                    }
                                }
                            }
                            // Dates, times and colors: a picker under the field (a second click closes it).
                            _ => {
                                let kind = js::with_dom(|d| formui::PickerKind::of(d, c));
                                let rect = lay.hits.iter().rev().find(|(_, n)| *n == c).map(|(r, _)| *r);
                                if let (Some(kind), Some(r)) = (kind, rect) {
                                    let mut p = js::with_dom(|d| formui::Picker::new(d, c, kind, r.x, r.y + r.h));
                                    p.x = p.left(w as f32);
                                    picker = Some(p);
                                    dirty = true;
                                }
                            }
                        }
                    }
                    if let (true, None, Some(href)) = (allowed, ctrl, &hit) {
                        let target = net::resolve(&page.url, href);
                        if !target.starts_with("mailto:") && !target.starts_with("javascript:") && security::page_may_navigate(&page.url, &target) {
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
                        let u = net::resolve(&page.url, &u);
                        if security::page_may_navigate(&page.url, &u) {
                            tab_cmd = Some(TabCmd::Open(u, false));
                        }
                    }
                }
            }
        }
        if form_changed {
            let editable = focused.is_some_and(|f| js::with_dom(|d| forms::control(d, f).is_some() || a11y::is_focusable(d, f)));
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
                restyle = true;
            }
            reflow = true;
        }
        if reflow && navigate.is_none() {
            if restyle || js::take_dirty() {
                page.restyle();
            }
            lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
            page_version += 1;
            dirty = true;
        }
        if down && !was_down {
            if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) {
                last_click = (Instant::now(), mx, my);
            }
        }
        was_down = down;
        was_mid = mid;
        was_right = right;
        // Menu items and shortcuts.
        if let Some(a) = ui_action.take() {
            match a {
                Ctx::Back => navigate = Some(Go::Back),
                Ctx::Forward => navigate = Some(Go::Forward),
                Ctx::Reload => navigate = Some(Go::Reload),
                Ctx::SiteBlocking(on) => {
                    blocklist::set_blocking_on(&page.url, on);
                    navigate = Some(Go::Reload);
                }
                Ctx::OpenSettings => tab_cmd = Some(TabCmd::Open("about:settings".into(), true)),
                Ctx::OpenNewTab(u) => tab_cmd = Some(TabCmd::Open(u, true)),
                Ctx::Copy(text) => {
                    if let Some(c) = clipboard.as_mut() {
                        let _ = c.set_text(text);
                    }
                }
                Ctx::CopySelection => {
                    if let Some((doc, sel)) = selection.filter(|s| s.0 == page.doc) {
                        let _ = doc;
                        if runs_cache.0 != page_version {
                            runs_cache = (page_version, chrome::text_runs(&lay));
                        }
                        let text = sel.text(fonts, &runs_cache.1);
                        if let (false, Some(c)) = (text.is_empty(), clipboard.as_mut()) {
                            let _ = c.set_text(text);
                        }
                    }
                }
                Ctx::SelectAll => {
                    selection = Some((page.doc, chrome::Selection { all: true, ..Default::default() }));
                    runs_cache.0 = u64::MAX;
                }
                Ctx::Find => {
                    let q = find.as_ref().map(|f| f.query.clone()).unwrap_or_default();
                    find = Some(chrome::Find { query: q.clone(), ..Default::default() });
                    find_edit = textedit::TextEdit::select_all(&q);
                    runs_cache.0 = u64::MAX;
                }
                Ctx::FindNext | Ctx::FindPrev => {
                    if let Some(f) = find.as_mut() {
                        f.next(matches!(a, Ctx::FindPrev));
                        if let Some((y, false)) = f.current_y(&runs_cache.1) {
                            let view = h.saturating_sub(bar_h()) as f32 - chrome::FIND_H;
                            if y < scroll + 20.0 || y > scroll + view - 40.0 {
                                scroll = (y - view / 3.0).max(0.0);
                            }
                        }
                    }
                }
                Ctx::ViewSource => {
                    if page.url.starts_with("http") || page.url.starts_with("file:") {
                        tab_cmd = Some(TabCmd::Open(format!("view-source:{}", page.url), false));
                    }
                }
                Ctx::Inspect => {
                    let (scripts, errors) = page.js.as_ref().map_or((0, 0), |j| (j.stats.scripts, j.stats.errors));
                    js::with_dom(|d| devtools::capture(d, &page.url, &page.title, page.loaded_at, scripts, errors));
                    devtools::set_inspected(page.doc);
                    tab_cmd = Some(TabCmd::Open("about:devtools".into(), false));
                }
                Ctx::SavePdf | Ctx::Print => {
                    let sheets = print_pages(&page, fonts);
                    let name: String = page.label().chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' }).take(80).collect();
                    let printed = matches!(a, Ctx::Print) && {
                        let tmp = std::env::temp_dir().join(format!("ironswift-print-{}.pdf", std::process::id()));
                        pdf::write(&tmp, &sheets).is_ok() && std::process::Command::new("lp").arg(&tmp).output().is_ok_and(|o| o.status.success())
                    };
                    if printed {
                        toast = Some((i18n::tf("toast-printed", &[("n", &sheets.len().to_string())]), Instant::now()));
                    } else {
                        let path = downloads::free_path(&format!("{}.pdf", name.trim()));
                        match pdf::write(&path, &sheets) {
                            Ok(size) => {
                                downloads::add_done(&page.url, &path, size);
                                let id = if matches!(a, Ctx::Print) { "toast-pdf-no-printer" } else { "toast-pdf-saved" };
                                let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                                toast = Some((i18n::tf(id, &[("file", &file), ("n", &sheets.len().to_string())]), Instant::now()));
                            }
                            Err(e) => toast = Some((i18n::tf("toast-pdf-failed", &[("error", &e.to_string())]), Instant::now())),
                        }
                    }
                }
            }
            dirty = true;
        }
        if toast.as_ref().is_some_and(|t| t.1.elapsed().as_secs_f32() > 3.0) {
            toast = None;
            dirty = true;
        }
        if selection.is_some_and(|s| s.0 != page.doc) {
            selection = None;
        }

        // The reload button (and Esc) stop a load in progress.
        if matches!(navigate, Some(Go::Reload)) && loading.is_some() || (stop_key && loading.is_some()) {
            navigate = None;
            loading = None;
            address = page.url.clone();
            window.set_title(&format!("{} — Ironswift{}", if page.title.is_empty() { &page.url } else { &page.title }, if is_private() { " (private)" } else { "" }));
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
                    crashlog::set_url(&url);
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
        // The developer console's Run button: run the code in the inspected tab, then show
        // the developer tools again with the result.
        if let Some(code) = match &navigate {
            Some(Go::To(t)) => devtools::run_request(t),
            _ => None,
        } {
            let r = if page.doc == devtools::inspected() {
                page.js.as_mut().map_or(Err("this page doesn't run scripts".to_string()), |j| j.run(&code))
            } else {
                run_in_background_tab(&mut tabs, devtools::inspected(), &code)
            };
            devtools::record(&code, r);
            navigate = Some(Go::To(format!("about:devtools?run={}#console", devtools::runs())));
            replace_entry = true;
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
                    scroll = y.clamp(0.0, (lay.height - h.saturating_sub(bar_h()) as f32).max(0.0));
                }
            }
            address = page.url.clone();
            editing = false;
            if js::take_dirty() {
                page.restyle();
                lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
                page_version += 1;
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
                let ptx = tx.clone();
                let _ = tx.send((g, fetch_page_with(&t, post.as_ref(), &mut |p| {
                    let _ = ptx.send((g, p));
                })));
            });
            window.set_title(&i18n::tf("window-loading", &[("target", &target.to_string())]));
            address = target;
            editing = false;
            loading = Some(Loading { generation, kind, started: Instant::now() });
            dirty = true;
        }
        // A finished load: build the page here, where JavaScript lives.
        while let Ok((g, prepared)) = rx.try_recv() {
            if prepared.partial {
                // The first part of a slow page: show it (if it's still the one wanted), keep
                // the loading bar going; the finished page replaces it.
                if loading.as_ref().is_some_and(|l| l.generation == g) {
                    page = build_page(prepared);
                    focused = None;
                    dropdown = None;
                    picker = None;
                    suggest = None;
                    stop_editing(&mut ce);
                    layout::set_focus(None);
                    lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
                    page_version += 1;
                    if preview_of != Some(g) {
                        scroll = 0.0;
                    }
                    scroll = scroll.clamp(0.0, (lay.height - h.saturating_sub(bar_h()) as f32).max(0.0));
                    preview_of = Some(g);
                    dirty = true;
                }
                continue;
            }
            if prepared.download {
                // Nothing to show: stop the loading bar and stay on the current page.
                if loading.take_if(|l| l.generation == g).is_some() {
                    if !editing {
                        address = page.url.clone();
                    }
                } else {
                    if let Some(slot) = tabs.iter_mut().flatten().find(|s| s.loading.as_ref().is_some_and(|l| l.generation == g)) {
                        slot.loading = None;
                    }
                }
                dirty = true;
                continue;
            }
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
            crashlog::set_url(&page.url);
            library::visit(&page.url, &page.title);
            current = Entry { url: page.url.clone(), doc: page.doc, state };
            focused = None;
            dropdown = None;
            picker = None;
            suggest = None;
            stop_editing(&mut ce);
            layout::set_focus(None);
            release_free_memory();
            lay = page.layout(fonts, w as f32, h.saturating_sub(bar_h()) as f32);
            page_version += 1;
            // A URL with a #fragment opens scrolled to it; a page shown while it loaded keeps
            // where the reader scrolled.
            if preview_of != Some(g) {
                scroll = fragment_target(&page.url).unwrap_or(0.0);
            }
            preview_of = None;
            scroll = scroll.clamp(0.0, (lay.height - h.saturating_sub(bar_h()) as f32).max(0.0));
            address = page.url.clone();
            window.set_title(&format!("{} — Ironswift{}", page.label(), if is_private() { " (private)" } else { "" }));
            dirty = true;
        }
        // While downloads run, keep their progress on screen (the chip, and about:downloads).
        if (downloads::active() > 0 || downloads_were_active) && last_download_tick.elapsed().as_millis() >= 500 {
            last_download_tick = Instant::now();
            downloads_were_active = downloads::active() > 0;
            dirty = true;
            if page.url == "about:downloads" && loading.is_none() && pending_nav.is_none() {
                pending_nav = Some(Go::Reload);
            }
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
                    log!("[tabs] put background tabs to sleep");
                    release_free_memory();
                    dirty = true;
                }
            }
            // Short of memory: put the background tab unseen the longest to sleep (one a second).
            if memory::under_pressure() {
                let oldest = tabs
                    .iter_mut()
                    .flatten()
                    .filter(|s| s.saved.is_some() && s.loading.is_none() && s.pending.is_none())
                    .min_by_key(|s| s.last_seen);
                if let Some(slot) = oldest {
                    let sv = slot.saved.take().expect("saved");
                    if let Some(j) = sv.page.js.as_ref() {
                        js::forget(j);
                    }
                    slot.url = sv.current.url.clone();
                    slot.back = sv.back;
                    slot.forward = sv.forward;
                    drop(sv.page);
                    log!("[tabs] low on memory ({} MB used): put a background tab to sleep", memory::rss_mb());
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
        let busy = dirty || animating || smooth.moving() || any_loading || js::network_busy() || window.get_mouse_down(MouseButton::Left) || !window.get_keys().is_empty();
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
            let icons: Vec<Option<favicon::Icon>> = tabs
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let p = match s {
                        None if i == active => &page,
                        Some(Slot { saved: Some(sv), .. }) => &sv.page,
                        _ => return None,
                    };
                    p.icon.as_deref().and_then(|u| favicon::get(u, &p.url))
                })
                .collect();
            let ui = Toolbar { address: &address, editing, can_back: !back.is_empty(), can_forward: !forward.is_empty(), hover, loading: spin, tabs: &labels, icons: &icons, active, tab_hover, caret: editing.then_some((aedit.caret, aedit.anchor)), bookmarked: library::is_bookmarked(&page.url), bookmarks: if BOOKMARKS_BAR.load(std::sync::atomic::Ordering::Relaxed) { &bm_list } else { &[] }, bm_hover, downloads: (!downloads::list().is_empty()).then(|| (downloads::active(), downloads::progress())), zoom: page.zoom(), shield: blocklist::blocking_on(&page.url).map(|on| (blocklist::blocked_count(&page.url), on)) };
            if let Some(bridge) = a11y.as_mut() {
                if window.is_active() != window_active {
                    window_active = window.is_active();
                    bridge.window_focused(window_active);
                }
                if bridge.active() {
                    let names: Vec<String> = labels.iter().map(|l| l.0.clone()).collect();
                    let rects = tab_rects(w as f32, tabs.len());
                    let chrome = a11y_bridge::Chrome {
                        tabs: &names,
                        active_tab: active,
                        address: &address,
                        editing_address: editing,
                        can_back: !back.is_empty(),
                        can_forward: !forward.is_empty(),
                        win_w: w as f32,
                        win_h: h as f32,
                        page_top: bar_h() as f32,
                        buttons: [BUTTONS[0].0, BUTTONS[1].0, BUTTONS[2].0],
                        button_w: BTN_W,
                        field_x: FIELD_X,
                        tab_h: TAB_H,
                        tool_h: TOOL_H,
                        tab_rects: &rects,
                    };
                    let title = page.title.clone();
                    let url = page.url.clone();
                    js::with_dom(|d| {
                        let pg = a11y_bridge::Page { dom: d, styles: &page.styles, layout: &lay, focused, scroll, url: &url };
                        bridge.update(&title, &chrome, &pg);
                    });
                }
            }
            let t0 = Instant::now();
            #[cfg(feature = "gpu")]
            let popups = folder_menu.is_some() || dropdown.is_some() || bubble.is_some() || picker.is_some() || suggest.is_some() || save_prompt.is_some() || ce.is_some() || (editing && !suggestions.is_empty()) || find.is_some() || selection.is_some() || status_link.is_some() || ctx_menu.is_some() || dl_panel.is_some() || toast.is_some() || !perm_queue.is_empty();
            #[cfg(feature = "gpu")]
            let gpu_tiled = gpu.is_some() && use_tiles && !popups && page_tiles.sync(page_version, w, &lay, &page.images);
            #[cfg(not(feature = "gpu"))]
            let gpu_tiled = false;
            if gpu_tiled {
                // The GPU draws the page from tiles: make sure the visible ones are painted.
                let range = tiles::PageTiles::visible(scroll.round().max(0.0), h.saturating_sub(bar_h()));
                for idx in range.clone() {
                    page_tiles.tile(idx, fonts, &page.images, &lay);
                }
                page_tiles.evict(&range);
                draw(&mut canvas, fonts, &page.images, &lay, scroll, &ui, PagePaint::Skip);
            } else {
                draw(&mut canvas, fonts, &page.images, &lay, scroll, &ui, if use_tiles { PagePaint::Tiles(&mut page_tiles, page_version) } else { PagePaint::Paint });
            }
            if frame_stats {
                draw_times.push(t0.elapsed().as_secs_f64() * 1000.0);
                if draw_times.len() == 60 {
                    let mut v = std::mem::take(&mut draw_times);
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    log!("[frames] draw avg {:.2} ms, median {:.2} ms, worst {:.2} ms (tiles painted {}, reused {}, kept {})", v.iter().sum::<f64>() / 60.0, v[30], v[59], page_tiles.stats.painted, page_tiles.stats.reused, page_tiles.stats.kept);
                }
            }
            // High contrast: the toolbars now, and whatever is drawn over the page from here on.
            let contrast_before = contrast::on().then(|| {
                contrast::recolor_rect(&mut canvas, 0, 0, w, bar_h());
                canvas.buf.clone()
            });
            if let Some(i) = ui_focus {
                let targets = ui_targets(fonts, w as f32, tabs.len(), if BOOKMARKS_BAR.load(std::sync::atomic::Ordering::Relaxed) { &bm_list } else { &[] }, !downloads::list().is_empty(), perm_queue.first());
                if let Some(&t) = targets.get(i.min(targets.len().saturating_sub(1))) {
                    uikeys::draw_ring(&mut canvas, t);
                }
            }
            if let Some(menu) = &folder_menu {
                rtl::begin(&mut canvas);
                draw_folder_menu(&mut canvas, fonts, menu, &bm_list);
                rtl::end(&mut canvas);
            }
            if let Some(dd) = &dropdown {
                draw_dropdown(&mut canvas, fonts, dd, scroll);
            }
            if let (true, Some(f)) = (focus_visible, focused) {
                if let Some((r, fixed)) = js::with_dom(|d| keynav::element_rect(&lay, d, f)) {
                    keynav::draw_ring(&mut canvas, r, fixed, bar_h() as f32, scroll.round(), h as f32, settings::get().high_contrast);
                }
            }
            if let Some(sg) = &suggest {
                sg.draw(&mut canvas, fonts, sg.y - scroll + bar_h() as f32);
            }
            if let Some(sp) = &save_prompt {
                sp.draw(&mut canvas, fonts, bar_h() as f32);
            }
            if let Some(c) = &ce {
                draw_edit_caret(&mut canvas, fonts, &page, &lay, c, scroll);
            }
            if let Some(p) = &picker {
                p.draw(&mut canvas, fonts, p.y - scroll + bar_h() as f32);
            }
            if let Some(b) = &bubble {
                if let Some(r) = lay.hits.iter().rev().find(|(_, n)| *n == b.node).map(|(r, _)| *r) {
                    b.draw(&mut canvas, fonts, r.x, r.y + r.h - scroll + bar_h() as f32);
                }
            }
            if editing && !suggestions.is_empty() {
                rtl::begin(&mut canvas);
                draw_suggestions(&mut canvas, fonts, &suggestions, sugg_sel.or(sugg_hover));
                rtl::end(&mut canvas);
            }
            // Browser UI over the page.
            if (find.is_some() || selection.is_some()) && runs_cache.0 != page_version {
                runs_cache = (page_version, chrome::text_runs(&lay));
                if let Some(f) = find.as_mut() {
                    f.search(&runs_cache.1);
                }
            }
            let view_bottom = h as f32 - if find.is_some() { chrome::FIND_H } else { 0.0 };
            if let Some((_, sel)) = &selection {
                sel.draw(&mut canvas, fonts, &runs_cache.1, bar_h() as f32, scroll.round(), view_bottom);
            }
            if let Some(f) = &find {
                f.draw_highlights(&mut canvas, fonts, &runs_cache.1, bar_h() as f32, scroll.round(), view_bottom);
                rtl::begin(&mut canvas);
                f.draw_bar(&mut canvas, fonts, Some(find_edit.caret));
                rtl::end(&mut canvas);
            } else if let Some(s) = &status_link {
                rtl::begin(&mut canvas);
                chrome::draw_status(&mut canvas, fonts, s);
                rtl::end(&mut canvas);
            }
            rtl::begin(&mut canvas);
            if let Some((t, _)) = &toast {
                chrome::draw_toast(&mut canvas, fonts, TOOL_BOTTOM as f32, t);
            }
            if let Some(p) = perm_queue.first() {
                p.draw(&mut canvas, fonts);
            }
            rtl::end(&mut canvas);
            // Menus open where they were asked to; right to left, their rows are mirrored inside.
            if let Some(m) = dl_panel.as_mut() {
                m.rows = downloads_panel(0.0, 0.0, w as f32, h as f32).rows;
                rtl::begin_in(&mut canvas, m.x, m.w);
                m.draw(&mut canvas, fonts);
                rtl::end(&mut canvas);
            }
            if let Some(m) = &ctx_menu {
                rtl::begin_in(&mut canvas, m.x, m.w);
                m.draw(&mut canvas, fonts);
                rtl::end(&mut canvas);
            }
            if let Some(before) = &contrast_before {
                contrast::recolor_changed(&mut canvas, before);
            }
            #[cfg(feature = "gpu")]
            let presented = match gpu.as_mut() {
                Some(g) => {
                    let top = if gpu_tiled { bar_h() as u32 } else { h as u32 };
                    g.present(&canvas.buf, w as u32, h as u32, top, gpu_tiled.then_some((&page_tiles, page_version, scroll.round().max(0.0))));
                    window.update();
                    true
                }
                None => false,
            };
            #[cfg(not(feature = "gpu"))]
            let presented = false;
            if !presented {
                let _ = window.update_with_buffer(&canvas.buf, w, h);
            }
            if !first_frame_shown {
                first_frame_shown = true;
                let ms = STARTED.get().map_or(0, |t| t.elapsed().as_millis());
                log!("[startup] first frame on screen after {ms} ms");
                // For measuring startup (tools/startup.sh): quit once the first frame is up.
                if std::env::var("IRONSWIFT_EXIT_AFTER_FIRST_FRAME").is_ok_and(|v| v == "1") {
                    return;
                }
            }
            dirty = false;
        } else {
            window.update();
        }
    }
}

/// What a right-click menu item or a shortcut asks for.
#[derive(Clone, Debug)]
enum Ctx {
    Back,
    Forward,
    Reload,
    OpenNewTab(String),
    Copy(String),
    CopySelection,
    SelectAll,
    Find,
    FindNext,
    FindPrev,
    SavePdf,
    Print,
    ViewSource,
    Inspect,
    /// Turn content blocking on (true) or off for this page's site, then reload.
    SiteBlocking(bool),
    OpenSettings,
}

/// The shield's menu: what was blocked, and a switch for the site.
fn shield_menu(url: &str, x: f32, y: f32, win_w: f32, win_h: f32) -> chrome::Menu<Ctx> {
    use chrome::MenuRow as R;
    let site = blocklist::site_for(url).unwrap_or_default();
    let on = blocklist::blocking_on(url).unwrap_or(false);
    let n = blocklist::blocked_count(url);
    let mut rows = Vec::new();
    if on {
        rows.push(R::disabled(&i18n::tf("shield-blocked", &[("count", &n.to_string())])));
        rows.push(R::separator());
        rows.push(R::item(&i18n::tf("shield-turn-off", &[("site", &site)]), Ctx::SiteBlocking(false)));
    } else {
        rows.push(R::disabled(&i18n::tf("shield-off", &[("site", &site)])));
        rows.push(R::separator());
        rows.push(R::item(&i18n::tf("shield-turn-on", &[("site", &site)]), Ctx::SiteBlocking(true)));
    }
    rows.push(R::item(&i18n::t("shield-settings"), Ctx::OpenSettings));
    chrome::Menu::new(x.max(4.0), y, 300.0, rows, win_w, win_h)
}

/// The downloads panel's actions.
#[derive(Clone, Debug)]
enum DlAct {
    Open(u32),
    Folder,
    ShowAll,
}

/// The right-click menu: link and image items when the mouse is on one, Copy when text is
/// selected, then page items.
#[allow(clippy::too_many_arguments)]
fn context_menu(page: &Page, lay: &layout::Layout, runs: &[chrome::TextRun], fonts: &Fonts, sel: Option<chrome::Selection>, mx: f32, my: f32, view_y: f32, scroll: f32, can_back: bool, can_forward: bool, win_w: f32, win_h: f32) -> chrome::Menu<Ctx> {
    use chrome::MenuRow as R;
    let mut rows: Vec<R<Ctx>> = Vec::new();
    if let Some(href) = lay.link_at(mx, view_y, scroll) {
        let url = net::resolve(&page.url, href);
        if !url.starts_with("javascript:") {
            rows.push(R::item(&i18n::t("menu-open-link-new-tab"), Ctx::OpenNewTab(url.clone())));
            rows.push(R::item(&i18n::t("menu-copy-link"), Ctx::Copy(url)));
            rows.push(R::separator());
        }
    }
    let y = view_y + scroll;
    let image = lay.items.iter().rev().find_map(|it| match it {
        paint::Item::Image { x, y: iy, w, h, src, .. } if mx >= *x && mx < x + w && y >= *iy && y < iy + h && !src.starts_with("iframe:") => Some(src.clone()),
        _ => None,
    });
    if let Some(src) = image {
        let url = net::resolve(&page.url, &src);
        rows.push(R::item(&i18n::t("menu-open-image-new-tab"), Ctx::OpenNewTab(url.clone())));
        rows.push(R::item(&i18n::t("menu-copy-image"), Ctx::Copy(url)));
        rows.push(R::separator());
    }
    if let Some(s) = sel {
        if !s.text(fonts, runs).is_empty() {
            rows.push(R::item(&i18n::t("menu-copy"), Ctx::CopySelection));
            rows.push(R::separator());
        }
    }
    rows.push(if can_back { R::item(&i18n::t("menu-back"), Ctx::Back) } else { R::disabled(&i18n::t("menu-back")) });
    rows.push(if can_forward { R::item(&i18n::t("menu-forward"), Ctx::Forward) } else { R::disabled(&i18n::t("menu-forward")) });
    rows.push(R::item(&i18n::t("menu-reload"), Ctx::Reload));
    rows.push(R::separator());
    rows.push(R::item(&i18n::t("menu-select-all"), Ctx::SelectAll));
    rows.push(R::item(&format!("{}   Ctrl+F", i18n::t("menu-find")), Ctx::Find));
    rows.push(R::separator());
    rows.push(R::item(&format!("{}   Ctrl+S", i18n::t("menu-save-pdf")), Ctx::SavePdf));
    rows.push(R::item(&format!("{}   Ctrl+P", i18n::t("menu-print")), Ctx::Print));
    if page.url.starts_with("http") || page.url.starts_with("file:") {
        rows.push(R::item(&format!("{}   Ctrl+U", i18n::t("menu-view-source")), Ctx::ViewSource));
    }
    rows.push(R::item(&format!("{}   F12", i18n::t("menu-inspect")), Ctx::Inspect));
    chrome::Menu::new(mx, my, 240.0, rows, win_w, win_h)
}

/// The downloads panel under the toolbar's ⬇ chip: the latest downloads, then links.
fn downloads_panel(x: f32, y: f32, win_w: f32, win_h: f32) -> chrome::Menu<DlAct> {
    use chrome::MenuRow as R;
    let human = |b: u64| if b >= 1 << 20 { format!("{:.1} MB", b as f64 / 1048576.0) } else { format!("{:.0} KB", (b as f64 / 1024.0).ceil()) };
    let mut rows: Vec<R<DlAct>> = Vec::new();
    for d in downloads::list().iter().rev().take(6) {
        let name = d.path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let (detail, action) = match &d.state {
            downloads::State::Active => (
                match d.total {
                    Some(t) => i18n::tf("dl-progress", &[("pct", &format!("{:.0}", d.received as f64 * 100.0 / t.max(1) as f64)), ("got", &human(d.received)), ("total", &human(t))]),
                    None => i18n::tf("dl-so-far", &[("got", &human(d.received))]),
                },
                None,
            ),
            downloads::State::Done => (i18n::tf("dl-done", &[("size", &human(d.received))]), Some(DlAct::Open(d.id))),
            downloads::State::Failed(e) => (i18n::tf("dl-failed", &[("error", &e.to_string())]), None),
            downloads::State::Cancelled => (i18n::t("dl-cancelled"), None),
        };
        rows.push(R { label: name, detail: Some(detail), action, separator: false });
    }
    rows.push(R::separator());
    rows.push(R::item(&i18n::t("dl-show-all"), DlAct::ShowAll));
    rows.push(R::item(&i18n::t("dl-open-folder"), DlAct::Folder));
    chrome::Menu::new(x, y, 330.0, rows, win_w, win_h)
}

/// Lay the page out for paper (7.5 × 10 inch printable area, drawn at twice the resolution)
/// and draw it sheet by sheet.
fn print_pages(page: &Page, fonts: &Fonts) -> Vec<image::RgbImage> {
    let (cw, ch) = (pdf::CONTENT_W_PX, pdf::CONTENT_H_PX);
    let lay = js::with_dom(|d| match d.composed() {
        Some(c) => layout::layout(&c, &page.styles, fonts, &page.images, cw, ch),
        None => layout::layout(d, &page.styles, fonts, &page.images, cw, ch),
    })
    .scaled(2.0);
    let (w, h) = ((cw * 2.0) as usize, (ch * 2.0) as usize);
    let sheets = ((lay.height / h as f32).ceil() as usize).clamp(1, 60);
    (0..sheets)
        .map(|i| {
            let mut canvas = Canvas::new(w, h);
            canvas.clear(lay.page_bg);
            canvas.paint(fonts, &page.images, &lay.items, 0.0, (i * h) as f32);
            image::RgbImage::from_fn(w as u32, h as u32, |x, y| {
                let p = canvas.buf[y as usize * w + x as usize];
                image::Rgb([(p >> 16) as u8, (p >> 8) as u8, p as u8])
            })
        })
        .collect()
}

struct Toolbar<'a> {
    tabs: &'a [(String, bool)],
    /// Each tab's site icon, once loaded.
    icons: &'a [Option<favicon::Icon>],
    active: usize,
    tab_hover: Option<TabHit>,
    /// While editing the address: (caret, selection anchor).
    caret: Option<(usize, usize)>,
    bookmarked: bool,
    /// The bookmarks bar (empty when hidden) and the chip under the mouse.
    bookmarks: &'a [library::Bookmark],
    bm_hover: Option<usize>,
    /// Downloads running: how many, and overall progress when sizes are known.
    downloads: Option<(usize, Option<f32>)>,
    address: &'a str,
    editing: bool,
    can_back: bool,
    can_forward: bool,
    hover: Option<usize>,
    /// Seconds since the current load started, if a page is loading.
    loading: Option<f32>,
    /// The page's zoom (a "125%" note in the address field when it isn't 100%).
    zoom: f32,
    /// The shield: requests blocked on this page, and whether blocking is on for its site
    /// (None on pages blocking doesn't apply to).
    shield: Option<(usize, bool)>,
}

struct Loading {
    generation: u64,
    kind: Go,
    started: Instant,
}

/// How `draw` puts the page in the window: painted whole, copied from stored tiles, or left
/// out (the GPU draws the tiles itself).
enum PagePaint<'a> {
    Paint,
    Tiles(&'a mut tiles::PageTiles, u64),
    Skip,
}

fn draw(canvas: &mut Canvas, fonts: &Fonts, images: &images::Images, lay: &layout::Layout, scroll: f32, ui: &Toolbar, page: PagePaint) {
    // Whole pixels: text stays sharp, and tiles line up exactly.
    let scroll = scroll.round();
    // The page: from stored tiles when it looks the same at every scroll position,
    // otherwise painted whole.
    let tiled = match page {
        PagePaint::Tiles(t, version) => {
            let ok = t.sync(version, canvas.w, lay, images);
            if ok {
                t.compose(canvas, fonts, images, lay, bar_h(), scroll);
            }
            ok
        }
        PagePaint::Skip => true,
        PagePaint::Paint => false,
    };
    if !tiled {
        canvas.clear(lay.page_bg);
        canvas.paint(fonts, images, &lay.items, bar_h() as f32, scroll);
    }
    // Right to left: the browser UI below is drawn mirrored.
    rtl::begin(canvas);
    let w = canvas.w as f32;
    let bar = TOOL_BOTTOM as f32;
    let top = TAB_H;
    // Tab strip
    // A private window's tab strip is purple.
    let private = is_private();
    canvas.fill_rect(0.0, 0.0, w, top, if private { Color::rgb(91, 61, 160) } else { Color::rgb(222, 222, 228) });
    let toolbar_bg = Color::rgb(236, 236, 240);
    for (i, ((label, loading), &(tx, tw))) in ui.tabs.iter().zip(tab_rects(w, ui.tabs.len()).iter()).enumerate() {
        let is_active = i == ui.active;
        let hovered = matches!(ui.tab_hover, Some(TabHit::Tab(j) | TabHit::Close(j)) if j == i);
        let bg = if is_active { toolbar_bg } else if hovered { Color::rgb(232, 232, 238) } else if private { Color::rgb(201, 189, 236) } else { Color::rgb(222, 222, 228) };
        canvas.fill_rect(tx, 5.0, tw - 2.0, top - 5.0, bg);
        if !is_active && i + 1 != ui.active && i + 1 < ui.tabs.len() {
            canvas.fill_rect(tx + tw - 2.0, 11.0, 1.0, top - 18.0, Color::rgb(190, 190, 198));
        }
        // A small orange dot while the tab loads, else the site's icon.
        let mut text_x = tx + 10.0;
        if *loading {
            canvas.fill_rect(tx + 9.0, 17.0, 6.0, 6.0, Color::rgb(249, 115, 22));
            text_x += 12.0;
        } else if let (Some(Some(icon)), true) = (ui.icons.get(i), tw > 70.0) {
            canvas.draw_argb((tx + 9.0) as i32, 12, favicon::SIZE, icon);
            text_x += 22.0;
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
    canvas.draw_text(fonts, end + 4.0 + (NEW_TAB_W - fonts.width(font::REGULAR, "+", 18.0)) / 2.0, 26.0, "+", font::REGULAR, 18.0, if private { Color::rgb(255, 255, 255) } else { Color::rgb(60, 60, 70) });
    if private {
        let label = &i18n::t("private-badge");
        let lw = fonts.width(font::BOLD, label, 12.0);
        canvas.draw_text(fonts, w - lw - 12.0, 22.0, label, font::BOLD, 12.0, Color::rgb(255, 255, 255));
    }

    // Toolbar, and the bookmarks bar under it
    canvas.fill_rect(0.0, top, w, bar_h() as f32 - top, toolbar_bg);
    canvas.fill_rect(0.0, bar_h() as f32 - 1.0, w, 1.0, Color::rgb(200, 200, 205));
    if !ui.bookmarks.is_empty() {
        let rects = bookmark_rects(fonts, w, ui.bookmarks);
        for (i, &(x, cw)) in rects.iter().enumerate() {
            if ui.bm_hover == Some(i) {
                canvas.fill_rect(x, bar + 2.0, cw, BOOK_H - 6.0, Color::rgb(214, 214, 222));
            }
            let label = bar_label(fonts, ui.bookmarks, &bar_items(ui.bookmarks)[i]);
            canvas.draw_text(fonts, x + 10.0, bar + 17.0, &label, font::REGULAR, 12.5, Color::rgb(45, 45, 55));
        }
        if rects.len() < bar_items(ui.bookmarks).len() {
            let x = w - 8.0 - BOOK_MORE_W;
            if ui.bm_hover == Some(usize::MAX) {
                canvas.fill_rect(x, bar + 2.0, BOOK_MORE_W, BOOK_H - 6.0, Color::rgb(214, 214, 222));
            }
            canvas.draw_text(fonts, x + 9.0, bar + 18.0, rtl::more_label(), font::REGULAR, 15.0, Color::rgb(60, 60, 70));
        }
    }

    // Back, forward, reload
    let enabled = [ui.can_back, ui.can_forward, true];
    for (i, (bx, label)) in BUTTONS.iter().enumerate() {
        if ui.hover == Some(i) && enabled[i] {
            canvas.fill_rect(*bx, top + 4.0, BTN_W, TOOL_H - 8.0, Color::rgb(214, 214, 222));
        }
        let ink = if enabled[i] { Color::rgb(40, 40, 50) } else { Color::rgb(170, 170, 178) };
        let size = 18.0;
        // While loading, reload becomes stop.
        let label = if i == 2 && ui.loading.is_some() { &"✕" } else { &rtl::nav_label(i, label) };
        let tw = fonts.width(font::BOLD, label, size);
        canvas.draw_text(fonts, bx + (BTN_W - tw) / 2.0, top + 24.0, label, font::BOLD, size, ink);
    }

    // Address field
    let field = if ui.editing { Color::rgb(255, 255, 255) } else { Color::rgb(248, 248, 250) };
    canvas.fill_rect(FIELD_X, top + 5.0, w - FIELD_X - 8.0, TOOL_H - 10.0, field);
    // Downloads chip: ⬇ and progress, left of the star.
    if let Some((n, progress)) = ui.downloads {
        let x = w - 8.0 - STAR_W - DL_W;
        canvas.fill_rect(x + 2.0, top + 8.0, DL_W - 6.0, TOOL_H - 16.0, Color::rgb(224, 231, 255));
        if let Some(p) = progress {
            canvas.fill_rect(x + 2.0, top + TOOL_H - 10.0, (DL_W - 6.0) * p.clamp(0.0, 1.0), 2.0, Color::rgb(79, 70, 229));
        }
        let label = match progress {
            Some(p) => format!("⬇ {:.0}%", p * 100.0),
            None if n > 1 => format!("⬇ {n}"),
            None => "⬇".to_string(),
        };
        canvas.draw_text(fonts, x + 9.0, top + 23.0, &label, font::REGULAR, 13.0, Color::rgb(55, 48, 163));
    }
    // The shield: a green count while blocking is on, a grey "off" when it's off for the site.
    if let Some((n, on)) = ui.shield {
        let x = shield_x(w, ui.downloads.is_some());
        let (bg, ink) = if on { (Color::rgb(220, 252, 231), Color::rgb(21, 128, 61)) } else { (Color::rgb(235, 235, 240), Color::rgb(110, 110, 120)) };
        canvas.fill_rect(x + 2.0, top + 8.0, SHIELD_W - 6.0, TOOL_H - 16.0, bg);
        let label = if !on { "off".to_string() } else if n > 999 { "999+".to_string() } else { n.to_string() };
        let lw = fonts.width(font::BOLD, &label, 12.5);
        canvas.draw_text(fonts, x + 2.0 + (SHIELD_W - 6.0 - lw) / 2.0, top + 22.0, &label, font::BOLD, 12.5, ink);
    }
    // Bookmark star
    let (star, star_ink) = if ui.bookmarked { ("★", Color::rgb(234, 179, 8)) } else { ("☆", Color::rgb(110, 110, 120)) };
    canvas.draw_text(fonts, w - 8.0 - STAR_W + 7.0, top + 24.0, star, font::REGULAR, 17.0, star_ink);
    let tx = FIELD_X + 8.0;
    if let Some((caret, anchor)) = ui.caret {
        let upto = |i: usize| fonts.width(font::REGULAR, &ui.address.chars().take(i).collect::<String>(), 14.0);
        // Right to left, the address sits at the field's far end but still reads left to right.
        let full = fonts.width(font::REGULAR, ui.address, 14.0);
        if caret != anchor {
            let (a, b) = (rtl::field_x(tx, upto(caret.min(anchor)), full), rtl::field_x(tx, upto(caret.max(anchor)), full));
            canvas.fill_rect(a.min(b), top + 9.0, (b - a).abs(), TOOL_H - 18.0, Color::rgb(180, 213, 254));
        }
        let cx = rtl::field_x(tx, upto(caret), full) - if rtl::on() { 1.0 } else { 0.0 };
        canvas.fill_rect(cx.round(), top + 9.0, 1.0, TOOL_H - 18.0, Color::rgb(30, 30, 40));
    }
    canvas.draw_text(fonts, tx, top + 23.0, ui.address, font::REGULAR, 14.0, Color::rgb(40, 40, 50));
    if (ui.zoom - 1.0).abs() > 1e-3 && !ui.editing {
        let label = format!("{:.0}%", ui.zoom * 100.0);
        let right = w - 8.0 - STAR_W - if ui.downloads.is_some() { DL_W } else { 0.0 } - if ui.shield.is_some() { SHIELD_W } else { 0.0 };
        let lw = fonts.width(font::REGULAR, &label, 12.5) + 12.0;
        canvas.fill_rect(right - lw - 4.0, top + 9.0, lw, TOOL_H - 18.0, Color::rgb(230, 230, 236));
        canvas.draw_text(fonts, right - lw + 2.0, top + 22.0, &label, font::REGULAR, 12.5, Color::rgb(60, 60, 70));
    }
    // Loading bar: a segment sliding along the bottom of the toolbar.
    if let Some(t) = ui.loading {
        let span = w * 0.25;
        let pos = (t * 0.8).fract() * (w + span) - span;
        canvas.fill_rect(pos.max(0.0), bar - 3.0, (pos + span).min(w) - pos.max(0.0), 2.0, Color::rgb(249, 115, 22));
    }
    rtl::end(canvas);
}

/// Address bar suggestions: a panel under the field, one row per page (title, then address).
fn draw_suggestions(canvas: &mut Canvas, fonts: &Fonts, list: &[library::Suggestion], hot: Option<usize>) {
    let (x, y0) = (FIELD_X, TOOL_BOTTOM as f32);
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
    let top = dd.y - scroll + bar_h() as f32;
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
        ("tests/canvas.html", 860),
        ("tests/webgl.html", 1024),
        ("tests/dotted.html", 400),
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
        ("tests/images-modern.html", 1024),
        ("tests/gradients-layers.html", 1024),
        ("tests/text-shaping.html", 900),
        ("tests/vertical-align.html", 800),
        ("tests/margins.html", 760),
        ("tests/percent-heights.html", 800),
        ("tests/columns.html", 800),
        ("tests/text-wrapping.html", 600),
        ("tests/pseudo.html", 900),
        ("tests/cascade.html", 1024),
        ("tests/shorthands.html", 700),
        ("tests/grid-extras.html", 600),
        ("tests/overflow-height.html", 500),
        ("tests/emoji-and-small-text.html", 700),
        ("tests/boxtree.html", 700),
        ("tests/inline-svg.html", 700),
        ("tests/real-world/react.html", 1024),
        ("tests/real-world/vue.html", 1024),
        ("tests/real-world/jquery.html", 1024),
        ("tests/real-world/bootstrap.html", 1024),
        ("tests/media.html", 1024),
        ("tests/vertical-text.html", 1024),
    ];

    fn read_png(path: &str) -> Option<(usize, usize, Vec<u8>)> {
        let img = image::open(path).ok()?.to_rgb8();
        Some((img.width() as usize, img.height() as usize, img.into_raw()))
    }

    #[test]
    fn pages_match_reference_images() {
        // Keep test runs out of the real profile (cookies, localStorage), starting fresh each time.
        let profile = std::env::temp_dir().join(format!("ironswift-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        // SAFETY: set before any other thread starts in this test.
        unsafe { std::env::set_var("IRONSWIFT_PROFILE", &profile) };
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
                log!("approved {reference}");
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
