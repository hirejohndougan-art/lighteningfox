//! Lantern — a web browser engine written from scratch in Rust.
//!
//! Pipeline: fetch -> HTML parse (DOM) -> CSS parse -> style cascade -> layout -> paint.
//!
//! Usage:
//!   lantern [URL or file]                  open a window
//!   lantern URL --png out.png [--width N]  render to an image (no window needed)

mod css;
mod dom;
mod font;
mod html;
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
const START_PAGE: &str = include_str!("start.html");

struct Page {
    url: String,
    title: String,
    dom: dom::Dom,
    styles: Vec<style::Style>,
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
    for id in 0..dom.nodes.len() {
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
    let styles = style::compute(&dom, &sheets);
    let title = dom.find_all("title").first().map(|&t| dom.text_content(t).trim().to_string()).unwrap_or_default();
    Page { url: final_url, title, dom, styles }
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
    let lay = layout::layout(&page.dom, &page.styles, fonts, width as f32);
    let height = (lay.height.ceil() as usize).clamp(100, 16000);
    let mut canvas = Canvas::new(width, height);
    canvas.clear(lay.page_bg);
    canvas.paint(fonts, &lay.items, 0.0, 0.0);
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = canvas.buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    eprintln!(
        "{}: {} nodes, {} display items, {}x{} px, {:?}",
        page.url,
        page.dom.nodes.len(),
        lay.items.len(),
        width,
        height,
        t.elapsed()
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
    let mut window = Window::new("Lantern", width, 768, WindowOptions { resize: true, ..WindowOptions::default() })
        .unwrap_or_else(|e| {
            eprintln!("Could not open a window: {e}");
            std::process::exit(1);
        });
    window.set_target_fps(60);
    let typed = Rc::new(RefCell::new(Vec::new()));
    window.set_input_callback(Box::new(Typed(typed.clone())));

    let mut history: Vec<String> = Vec::new();
    let mut page = load(start);
    let (mut w, mut h) = window.get_size();
    let mut lay = layout::layout(&page.dom, &page.styles, fonts, w as f32);
    let mut canvas = Canvas::new(w, h);
    let mut scroll = 0.0f32;
    let mut editing = false;
    let mut address = page.url.clone();
    let mut dirty = true;
    let mut was_down = false;
    window.set_title(&format!("{} — Lantern", if page.title.is_empty() { &page.url } else { &page.title }));

    while window.is_open() {
        let mut navigate: Option<String> = None;

        // Resize -> relayout.
        let (nw, nh) = window.get_size();
        if (nw, nh) != (w, h) && nw > 0 && nh > 0 {
            if nw != w {
                lay = layout::layout(&page.dom, &page.styles, fonts, nw as f32);
            }
            (w, h) = (nw, nh);
            canvas = Canvas::new(w, h);
            dirty = true;
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
                        navigate = Some(net::normalize_input(&address));
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
                    Key::F5 | Key::R if key == Key::F5 || ctrl => navigate = Some(page.url.clone()),
                    Key::Backspace => navigate = history.pop().map(|u| format!("\u{0}{u}")),
                    Key::Left if alt => navigate = history.pop().map(|u| format!("\u{0}{u}")),
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
            window.set_cursor_style(if hit.is_some() {
                CursorStyle::ClosedHand
            } else if (my as usize) < BAR_H {
                CursorStyle::Ibeam
            } else {
                CursorStyle::Arrow
            });
            if down && !was_down {
                if (my as usize) < BAR_H {
                    editing = true;
                    address.clear();
                    dirty = true;
                } else if let Some((_, href)) = hit {
                    let target = net::resolve(&page.url, href);
                    let same_page = target.split('#').next() == page.url.split('#').next() && target.contains('#');
                    if !same_page && !target.starts_with("mailto:") && !target.starts_with("javascript:") {
                        navigate = Some(target);
                    }
                }
            }
        }
        was_down = down;

        if let Some(target) = navigate {
            // A leading NUL marks "going back" so we don't push to history.
            let (target, back) = match target.strip_prefix('\u{0}') {
                Some(t) => (t.to_string(), true),
                None => (target, false),
            };
            if !back && target != page.url {
                history.push(page.url.clone());
            }
            window.set_title(&format!("Loading {target} — Lantern"));
            draw(&mut canvas, fonts, &lay, scroll, &format!("Loading {target}…"), false);
            let _ = window.update_with_buffer(&canvas.buf, w, h);
            page = load(&target);
            lay = layout::layout(&page.dom, &page.styles, fonts, w as f32);
            scroll = 0.0;
            address = page.url.clone();
            window.set_title(&format!("{} — Lantern", if page.title.is_empty() { &page.url } else { &page.title }));
            dirty = true;
        }

        if dirty {
            draw(&mut canvas, fonts, &lay, scroll, &address, editing);
            let _ = window.update_with_buffer(&canvas.buf, w, h);
            dirty = false;
        } else {
            window.update();
        }
    }
}

fn draw(canvas: &mut Canvas, fonts: &Fonts, lay: &layout::Layout, scroll: f32, address: &str, editing: bool) {
    canvas.clear(lay.page_bg);
    canvas.paint(fonts, &lay.items, 0.0, BAR_H as f32 - scroll);
    // Address bar
    let w = canvas.w as f32;
    canvas.fill_rect(0.0, 0.0, w, BAR_H as f32, Color::rgb(236, 236, 240));
    canvas.fill_rect(0.0, BAR_H as f32 - 1.0, w, 1.0, Color::rgb(200, 200, 205));
    let field = if editing { Color::rgb(255, 255, 255) } else { Color::rgb(248, 248, 250) };
    canvas.fill_rect(8.0, 5.0, w - 16.0, BAR_H as f32 - 10.0, field);
    let text = if editing { format!("{address}|") } else { address.to_string() };
    canvas.draw_text(fonts, 16.0, 23.0, &text, font::REGULAR, 14.0, Color::rgb(40, 40, 50));
}
