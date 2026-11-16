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
        let (images, url) = (&mut self.images, &self.url);
        js::with_dom(|d| images.load_missing(d, url));
        self.styles = js::with_dom(|d| style::compute(d, &self.sheets));
        self.title = js::with_dom(|d| d.find_all("title").first().map(|&t| d.text_content(t).trim().to_string()).unwrap_or_default());
    }

    fn layout(&self, fonts: &Fonts, width: f32) -> layout::Layout {
        js::with_dom(|d| layout::layout(d, &self.styles, fonts, &self.images, width))
    }
}

fn load(url: &str) -> Page {
    let (final_url, body) = if url == "about:home" || url.is_empty() {
        ("about:home".to_string(), START_PAGE.to_string())
    } else {
        match net::fetch(url) {
            Ok(r) => r,
            Err(e) => (url.to_string(), error_page(url, &e)),
        }
    };
    let dom = html::parse(&body);
    let mut sheets = Vec::new();
    for id in dom.descendants(dom::Dom::ROOT) {
        match dom.tag(id) {
            Some("style") => sheets.push(css::parse(&dom.text_content(id))),
            Some("link") => {
                let rel = dom.attr(id, "rel").unwrap_or("").to_ascii_lowercase();
                if let (true, Some(href)) = (rel.split_whitespace().any(|r| r == "stylesheet"), dom.attr(id, "href")) {
                    let media = dom.attr(id, "media").unwrap_or("all");
                    if media.contains("print") && !media.contains("screen") {
                        continue;
                    }
                    if let Ok((_, text)) = net::fetch(&net::resolve(&final_url, href)) {
                        sheets.push(css::parse(&text));
                    }
                }
            }
            _ => {}
        }
    }
    js::set_dom(dom);
    let mut js = js::Js::new(&final_url);
    js.run_scripts(&final_url);
    js::take_dirty();
    let mut page = Page { url: final_url, title: String::new(), sheets, styles: Vec::new(), images: images::Images::default(), js };
    page.restyle();
    page
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

fn render_png(fonts: &Fonts, url: &str, width: usize, out: &str) {
    let t = std::time::Instant::now();
    let page = load(url);
    let lay = page.layout(fonts, width as f32);
    let height = (lay.height.ceil() as usize).clamp(100, 16000);
    let mut canvas = Canvas::new(width, height);
    canvas.clear(lay.page_bg);
    canvas.paint(fonts, &page.images, &lay.items, 0.0, 0.0);
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = canvas.buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
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

        // Resolve the navigation against the back/forward history.
        let target = match navigate {
            Some(Go::To(t)) => {
                if t != page.url {
                    back.push(page.url.clone());
                    forward.clear();
                }
                Some(t)
            }
            Some(Go::Back) => back.pop().inspect(|_| forward.push(page.url.clone())),
            Some(Go::Forward) => forward.pop().inspect(|_| back.push(page.url.clone())),
            Some(Go::Reload) => Some(page.url.clone()),
            None => None,
        };
        if let Some(target) = target {
            window.set_title(&format!("Loading {target} — LighteningFox"));
            let ui = Toolbar { address: &format!("Loading {target}…"), editing: false, can_back: !back.is_empty(), can_forward: !forward.is_empty(), hover: None };
            draw(&mut canvas, fonts, &page.images, &lay, scroll, &ui);
            let _ = window.update_with_buffer(&canvas.buf, w, h);
            page = load(&target);
            lay = page.layout(fonts, w as f32);
            scroll = 0.0;
            address = page.url.clone();
            window.set_title(&format!("{} — LighteningFox", if page.title.is_empty() { &page.url } else { &page.title }));
            dirty = true;
        }

        if dirty {
            let ui = Toolbar { address: &address, editing, can_back: !back.is_empty(), can_forward: !forward.is_empty(), hover };
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
        let tw = fonts.width(font::BOLD, label, size);
        canvas.draw_text(fonts, bx + (BTN_W - tw) / 2.0, 24.0, label, font::BOLD, size, ink);
    }

    // Address field
    let field = if ui.editing { Color::rgb(255, 255, 255) } else { Color::rgb(248, 248, 250) };
    canvas.fill_rect(FIELD_X, 5.0, w - FIELD_X - 8.0, bar - 10.0, field);
    let text = if ui.editing { format!("{}|", ui.address) } else { ui.address.to_string() };
    canvas.draw_text(fonts, FIELD_X + 8.0, 23.0, &text, font::REGULAR, 14.0, Color::rgb(40, 40, 50));
}
