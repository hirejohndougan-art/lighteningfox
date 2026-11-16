//! LighteningFox — a web browser engine written from scratch in Rust.
//!
//! Pipeline: fetch -> HTML parse (DOM) -> CSS parse -> style cascade -> layout -> paint.
//!
//! Usage:
//!   lighteningfox [URL or file]                  open a window
//!   lighteningfox URL --png out.png [--width N]  render to an image (no window needed)

mod css;
mod dom;
mod font;
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
    styles: Vec<style::Style>,
    images: images::Images,
    js: js::Js,
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

    fn layout(&self, fonts: &Fonts, width: f32) -> layout::Layout {
        js::with_dom(|d| layout::layout(d, &self.styles, fonts, &self.images, width))
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
fn fetch_page(url: &str) -> Prepared {
    let (final_url, body) = if url == "about:home" || url.is_empty() {
        ("about:home".to_string(), START_PAGE.to_string())
    } else {
        match net::fetch(url) {
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
                    let (_, text) = net::fetch(u).ok()?;
                    let mut sheet = css::parse(&text);
                    css::absolutize_urls(&mut sheet, u);
                    Some(sheet)
                }),
            })
            .collect();
        let script_jobs: Vec<_> =
            script_urls.iter().map(|u| scope.spawn(move || net::fetch(u).ok().map(|(_, b)| (u.clone(), b)))).collect();
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

fn background_urls(styles: &[style::Style]) -> Vec<String> {
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
    js::set_dom(p.dom);
    let mut js = js::Js::new(&p.url);
    js.run_scripts(&p.url, &p.scripts);
    js::take_dirty();
    let mut page = Page { url: p.url, title: String::new(), sheets: p.sheets, styles: Vec::new(), images: p.images, js };
    page.restyle();
    page
}

fn load(url: &str) -> Page {
    build_page(fetch_page(url))
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
    let fonts = match Fonts::load() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    match png {
        Some(out) => render_png(&fonts, &url, width, &out),
        None => run_window(&fonts, &url, width),
    }
}

/// Load a page and paint the whole of it (up to 16,000 px tall) into a canvas.
fn render_page(fonts: &Fonts, url: &str, width: usize) -> (Page, layout::Layout, Canvas) {
    let page = load(url);
    let lay = page.layout(fonts, width as f32);
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
    let st = &page.js.stats;
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

fn run_window(fonts: &Fonts, start: &str, width: usize) {
    let mut window = Window::new("LighteningFox", width, 768, WindowOptions { resize: true, ..WindowOptions::default() })
        .unwrap_or_else(|e| {
            eprintln!("Could not open a window: {e}");
            std::process::exit(1);
        });
    window.set_target_fps(60);
    let typed = Rc::new(RefCell::new(Vec::new()));
    window.set_input_callback(Box::new(Typed(typed.clone())));

    let mut back: Vec<String> = Vec::new();
    let mut forward: Vec<String> = Vec::new();
    let mut hover: Option<usize> = None;
    // Pages load on worker threads; results come back tagged with a generation number
    // so a stopped or superseded load is ignored when it arrives.
    let (tx, rx) = mpsc::channel::<(u64, Prepared)>();
    let mut generation = 0u64;
    let mut loading: Option<Loading> = None;
    let mut last_spin = Instant::now();
    let mut page = load(start);
    let (mut w, mut h) = window.get_size();
    let mut lay = page.layout(fonts, w as f32);
    let mut canvas = Canvas::new(w, h);
    let mut scroll = 0.0f32;
    let mut editing = false;
    let mut address = page.url.clone();
    let mut dirty = true;
    let mut was_down = false;
    window.set_title(&format!("{} — LighteningFox", if page.title.is_empty() { &page.url } else { &page.title }));

    while window.is_open() {
        let mut navigate: Option<Go> = None;
        let mut reflow = false;
        let mut stop_key = false;

        // Resize -> relayout.
        let (nw, nh) = window.get_size();
        if (nw, nh) != (w, h) && nw > 0 && nh > 0 {
            if nw != w {
                lay = page.layout(fonts, nw as f32);
            }
            (w, h) = (nw, nh);
            canvas = Canvas::new(w, h);
            dirty = true;
        }
        // JavaScript: run due timers; restyle and reflow if the DOM changed.
        page.js.tick();
        if js::take_dirty() {
            page.restyle();
            lay = page.layout(fonts, w as f32);
            dirty = true;
        }
        if let Some(target) = js::take_navigation() {
            navigate = Some(Go::To(net::resolve(&page.url, &target)));
        }
        let view_h = h.saturating_sub(BAR_H) as f32;
        let max_scroll = (lay.height - view_h).max(0.0);

        // Typing in the address bar.
        let chars: Vec<char> = typed.borrow_mut().drain(..).collect();
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
            let doc_y = my - BAR_H as f32 + scroll;
            let hit = if my >= BAR_H as f32 { lay.links.iter().find(|(r, _)| r.contains(mx, doc_y)) } else { None };
            let btn = button_at(mx, my);
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
            if down && !was_down {
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
                    let node = lay.hits.iter().find(|(r, _)| r.contains(mx, doc_y)).map(|(_, n)| *n);
                    let allowed = node.map_or(true, |n| page.js.click(n));
                    if js::take_dirty() {
                        page.restyle();
                        reflow = true;
                    }
                    if let Some(t) = js::take_navigation() {
                        navigate = Some(Go::To(net::resolve(&page.url, &t)));
                    }
                    if let (true, Some((_, href))) = (allowed, hit) {
                    let target = net::resolve(&page.url, href);
                    let same_page = target.split('#').next() == page.url.split('#').next() && target.contains('#');
                    if !same_page && !target.starts_with("mailto:") && !target.starts_with("javascript:") {
                        navigate = Some(Go::To(target));
                    }
                    }
                }
            }
        }
        if reflow && navigate.is_none() {
            lay = page.layout(fonts, w as f32);
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
            Some(Go::To(t)) => Some(t.clone()),
            Some(Go::Back) => back.last().cloned(),
            Some(Go::Forward) => forward.last().cloned(),
            Some(Go::Reload) => Some(page.url.clone()),
            None => None,
        };
        if let (Some(target), Some(kind)) = (target, navigate) {
            generation += 1;
            let (g, t, tx) = (generation, target.clone(), tx.clone());
            std::thread::spawn(move || {
                let _ = tx.send((g, fetch_page(&t)));
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
                Go::To(_) => {
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
            lay = page.layout(fonts, w as f32);
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

        if dirty {
            let spin = loading.as_ref().map(|l| l.started.elapsed().as_secs_f32());
            let ui = Toolbar { address: &address, editing, can_back: !back.is_empty(), can_forward: !forward.is_empty(), hover, loading: spin };
            draw(&mut canvas, fonts, &page.images, &lay, scroll, &ui);
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
    canvas.paint(fonts, images, &lay.items, 0.0, BAR_H as f32 - scroll);
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
