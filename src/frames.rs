//! Live iframes: each frame's document and scripts stay running, and the frame takes mouse
//! clicks, wheel scrolling and keyboard input like the page around it.
//!
//! Each frame is run by a *host*: its own thread (same-site frames), or a thread inside the
//! sandboxed `ironswift --frame-renderer` process (cross-site frames, see `isolated`). A host
//! owns the frame's `Page` (DOM, styles, script engine) and its scroll position, focus and
//! text editing. It sends a new picture of the frame whenever the frame changes; the window
//! shows that picture in the iframe's box (`pump`). Input over an iframe is routed to its host
//! (`mouse`, `wheel`, `keys`) instead of the page.
//!
//! Frames nested inside frames are still drawn once (as before), and a sandboxed `srcdoc`
//! frame stays a still picture: it shares its parent's address, which the per-address
//! security registry can't tell apart from the parent while both are alive.

use crate::{dom::NodeId, forms, js, layout, net, security, userinput, FrameJob, FrameSrc, Go, KeyPress, Page};
use minifb::Key;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- messages

/// Input for a frame, in the frame's own CSS pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The mouse moved or a button changed (left, middle, right).
    Mouse { x: f32, y: f32, down: [bool; 3] },
    /// The mouse left the frame.
    Leave,
    Wheel { x: f32, y: f32, dy: f32 },
    /// Key presses (with ctrl, shift, alt) and typed text, while the frame has focus.
    Keys { keys: Vec<(Key, bool, bool, bool)>, text: String },
    /// Focus moved out of the frame.
    Blur,
}

/// What a frame tells the window.
#[derive(Debug)]
pub enum Update {
    /// A new picture of the frame, its scroll position and how far it can scroll, and
    /// whether it has scrollable boxes inside.
    Picture { img: image::RgbaImage, scroll: f32, max: f32, inner: bool },
    /// A link with target=_top (or _parent) was followed.
    NavigateTop(String),
    /// A link with target=_blank, or window.open during a click.
    OpenTab(String),
}

/// Keys a frame may receive, by name (so they cross the process boundary).
const KEYS: &[(Key, &str)] = &[
    (Key::A, "A"), (Key::B, "B"), (Key::C, "C"), (Key::D, "D"), (Key::E, "E"), (Key::F, "F"), (Key::G, "G"), (Key::H, "H"), (Key::I, "I"),
    (Key::J, "J"), (Key::K, "K"), (Key::L, "L"), (Key::M, "M"), (Key::N, "N"), (Key::O, "O"), (Key::P, "P"), (Key::Q, "Q"), (Key::R, "R"),
    (Key::S, "S"), (Key::T, "T"), (Key::U, "U"), (Key::V, "V"), (Key::W, "W"), (Key::X, "X"), (Key::Y, "Y"), (Key::Z, "Z"),
    (Key::Key0, "0"), (Key::Key1, "1"), (Key::Key2, "2"), (Key::Key3, "3"), (Key::Key4, "4"), (Key::Key5, "5"), (Key::Key6, "6"),
    (Key::Key7, "7"), (Key::Key8, "8"), (Key::Key9, "9"), (Key::Space, "Space"), (Key::Enter, "Enter"), (Key::NumPadEnter, "NumPadEnter"),
    (Key::Backspace, "Backspace"), (Key::Delete, "Delete"), (Key::Tab, "Tab"), (Key::Escape, "Escape"), (Key::Left, "Left"),
    (Key::Right, "Right"), (Key::Up, "Up"), (Key::Down, "Down"), (Key::Home, "Home"), (Key::End, "End"), (Key::PageUp, "PageUp"),
    (Key::PageDown, "PageDown"),
];

fn key_name(k: Key) -> Option<&'static str> {
    KEYS.iter().find(|(key, _)| *key == k).map(|(_, n)| *n)
}

fn key_from(name: &str) -> Option<Key> {
    KEYS.iter().find(|(_, n)| *n == name).map(|(k, _)| *k)
}

/// Can this key go to a frame? Browser shortcuts (Ctrl+T, Ctrl+L, F5…) stay with the window;
/// editing shortcuts go to the frame.
pub fn for_frame(k: Key, ctrl: bool) -> bool {
    key_name(k).is_some() && (!ctrl || matches!(k, Key::A | Key::Z | Key::Y | Key::Left | Key::Right | Key::Home | Key::End | Key::Backspace | Key::Delete))
}

fn put(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

fn get(r: &mut impl std::io::Read) -> std::io::Result<Vec<u8>> {
    let mut n = [0u8; 4];
    r.read_exact(&mut n)?;
    let n = u32::from_le_bytes(n) as usize;
    if n > 64 << 20 {
        return Err(std::io::Error::other("message too large"));
    }
    let mut b = vec![0u8; n];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn read_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
}

impl Event {
    fn encode(&self) -> Vec<u8> {
        let mut m = Vec::new();
        match self {
            Event::Mouse { x, y, down } => {
                put(&mut m, b"mouse");
                put(&mut m, &f32s(&[*x, *y, down[0] as u8 as f32, down[1] as u8 as f32, down[2] as u8 as f32]));
            }
            Event::Leave => put(&mut m, b"leave"),
            Event::Wheel { x, y, dy } => {
                put(&mut m, b"wheel");
                put(&mut m, &f32s(&[*x, *y, *dy]));
            }
            Event::Keys { keys, text } => {
                put(&mut m, b"keys");
                let names: Vec<String> = keys.iter().filter_map(|(k, c, s, a)| key_name(*k).map(|n| format!("{n}:{}{}{}", *c as u8, *s as u8, *a as u8))).collect();
                put(&mut m, names.join(" ").as_bytes());
                put(&mut m, text.as_bytes());
            }
            Event::Blur => put(&mut m, b"blur"),
        }
        m
    }

    fn decode(r: &mut impl std::io::Read) -> std::io::Result<Event> {
        let kind = get(r)?;
        Ok(match kind.as_slice() {
            b"mouse" => {
                let v = read_f32s(&get(r)?);
                if v.len() != 5 {
                    return Err(std::io::Error::other("bad mouse event"));
                }
                Event::Mouse { x: v[0], y: v[1], down: [v[2] > 0.5, v[3] > 0.5, v[4] > 0.5] }
            }
            b"leave" => Event::Leave,
            b"wheel" => {
                let v = read_f32s(&get(r)?);
                if v.len() != 3 {
                    return Err(std::io::Error::other("bad wheel event"));
                }
                Event::Wheel { x: v[0], y: v[1], dy: v[2] }
            }
            b"keys" => {
                let names = String::from_utf8_lossy(&get(r)?).to_string();
                let text = String::from_utf8_lossy(&get(r)?).to_string();
                let keys = names
                    .split_whitespace()
                    .filter_map(|w| {
                        let (n, m) = w.split_once(':')?;
                        let m = m.as_bytes();
                        (m.len() == 3).then(|| key_from(n).map(|k| (k, m[0] == b'1', m[1] == b'1', m[2] == b'1')))?
                    })
                    .collect();
                Event::Keys { keys, text }
            }
            b"blur" => Event::Blur,
            _ => return Err(std::io::Error::other("unknown event")),
        })
    }
}

impl Update {
    fn encode(&self) -> Vec<u8> {
        let mut m = Vec::new();
        match self {
            Update::Picture { img, scroll, max, inner } => {
                put(&mut m, b"picture");
                put(&mut m, &f32s(&[img.width() as f32, img.height() as f32, *scroll, *max, *inner as u8 as f32]));
                put(&mut m, img.as_raw());
            }
            Update::NavigateTop(u) => {
                put(&mut m, b"top");
                put(&mut m, u.as_bytes());
            }
            Update::OpenTab(u) => {
                put(&mut m, b"tab");
                put(&mut m, u.as_bytes());
            }
        }
        m
    }

    fn decode(r: &mut impl std::io::Read) -> std::io::Result<Update> {
        let kind = get(r)?;
        Ok(match kind.as_slice() {
            b"picture" => {
                let v = read_f32s(&get(r)?);
                let px = get(r)?;
                if v.len() != 5 {
                    return Err(std::io::Error::other("bad picture"));
                }
                let img = image::RgbaImage::from_raw(v[0] as u32, v[1] as u32, px).ok_or_else(|| std::io::Error::other("bad picture size"))?;
                Update::Picture { img, scroll: v[2], max: v[3], inner: v[4] > 0.5 }
            }
            b"top" => Update::NavigateTop(String::from_utf8_lossy(&get(r)?).to_string()),
            b"tab" => Update::OpenTab(String::from_utf8_lossy(&get(r)?).to_string()),
            _ => return Err(std::io::Error::other("unknown update")),
        })
    }
}

// ---------------------------------------------------------------- the host

/// A frame being run: its page and what the user is doing in it.
struct Host {
    page: Page,
    lay: layout::Layout,
    fonts: &'static crate::Fonts,
    w: u32,
    h: u32,
    scroll: f32,
    pointer: userinput::Pointer,
    focused: Option<NodeId>,
    edit: Option<crate::textedit::TextEdit>,
    job: FrameJob,
    /// The frame's own address, registered with the security code while it runs.
    registered: Option<String>,
    shown: bool,
    /// The left button as last reported.
    last_down: bool,
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(u) = self.registered.take() {
            security::clear_frame(&u);
        }
        if let Some(js) = self.page.js.as_ref() {
            js::forget(js);
        }
    }
}

/// Load what the job names (or a new address the frame navigated to). None = it may not be
/// shown here (X-Frame-Options / frame-ancestors): the caller shows the refusal picture.
fn load_page(job: &FrameJob, url: Option<(&str, Option<&forms::PostBody>)>) -> Option<(Page, Option<String>)> {
    let info = security::FrameInfo { top: job.top.clone(), sandbox: job.sandbox.clone() };
    let src = match url {
        Some((u, post)) => Some((u.to_string(), post)),
        None => match &job.src {
            FrameSrc::Url(u) => Some((u.clone(), None)),
            FrameSrc::Doc { .. } => None,
        },
    };
    match src {
        Some((u, post)) => {
            security::set_frame(&u, info.clone());
            let p = crate::fetch_page(&u, post);
            security::clear_frame(&u);
            if !p.policy.may_be_framed_by(&p.url, &job.parent, &job.top) {
                security::clear_frame(&p.url);
                return None;
            }
            // The document keeps its frame identity while it lives (storage, cookies, sandbox).
            security::set_frame(&p.url, info);
            let url = p.url.clone();
            Some((crate::build_page(p), Some(url)))
        }
        None => {
            let FrameSrc::Doc { html, base } = &job.src else { unreachable!() };
            let p = crate::prepare(base.clone(), html.clone(), true, false, &[]);
            Some((crate::build_page(p), None))
        }
    }
}

impl Host {
    fn picture(&mut self) -> Update {
        let (w, h) = (self.w as usize, self.h as usize);
        let max = (self.lay.height - self.h as f32).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
        let mut canvas = crate::Canvas::new(w, h);
        canvas.clear(self.lay.page_bg);
        canvas.paint(self.fonts, &self.page.images, &self.lay.items, 0.0, self.scroll.round());
        let img = image::RgbaImage::from_fn(self.w, self.h, |x, y| {
            let p = canvas.buf[y as usize * w + x as usize];
            image::Rgba([(p >> 16) as u8, (p >> 8) as u8, p as u8, 255])
        });
        Update::Picture { img, scroll: self.scroll, max, inner: !self.lay.scrollers.is_empty() }
    }

    fn relayout(&mut self) {
        crate::geometry::set_viewport(self.w as f32, self.h as f32);
        layout::set_focus(self.focused.map(|f| (f, js::with_dom(|d| forms::is_editable(d, f)))));
        if let Some(e) = &self.edit {
            layout::set_edit(e.caret, e.anchor);
        }
        self.lay = self.page.layout(self.fonts, self.w as f32, self.h as f32);
    }

    /// Open another document in the frame (a link, a form, a script setting location).
    fn navigate(&mut self, url: &str, post: Option<&forms::PostBody>) {
        if !security::page_may_navigate(&self.page.url, url) || url.starts_with("javascript:") {
            return;
        }
        if let Some(old) = self.registered.take() {
            security::clear_frame(&old);
        }
        layout::reset_element_scrolls();
        self.pointer.reset();
        self.focused = None;
        self.edit = None;
        self.scroll = 0.0;
        match load_page(&self.job, Some((url, post))) {
            Some((page, reg)) => {
                self.page = page;
                self.registered = reg;
                self.settle();
            }
            None => {
                self.page = Page::empty();
                self.lay = layout::Layout::default();
                self.shown = false;
            }
        }
        self.relayout();
    }

    /// Let a new document's scripts and quick timers fill it in before it's first shown.
    fn settle(&mut self) {
        if let Some(js) = self.page.js.as_mut() {
            js::activate(js);
            js.settle(Duration::from_millis(1500));
            if js::take_dirty() {
                self.page.restyle();
            }
        }
    }

    fn fire(&mut self, node: NodeId, kind: &str) -> bool {
        crate::fire(&mut self.page, node, kind)
    }

    fn change_focus(&mut self, to: Option<NodeId>) {
        if to == self.focused {
            return;
        }
        if let Some(old) = self.focused {
            self.fire(old, "change");
            self.fire(old, "blur");
        }
        self.focused = to;
        self.edit = None;
        if let Some(n) = to {
            self.fire(n, "focus");
            if js::with_dom(|d| forms::is_editable(d, n)) {
                let v = js::with_dom(|d| forms::value(d, n));
                let mut e = crate::textedit::TextEdit::new(&v);
                let len = e.len();
                e.move_to(len, false);
                self.edit = Some(e);
            }
        }
    }

    /// A press of the left button at (x, y): what a click does on the page, inside the frame.
    fn click(&mut self, x: f32, y: f32, out: &mut dyn FnMut(Update)) {
        let node = self.lay.hit_at(x, y, self.scroll);
        let href = self.lay.link_at(x, y, self.scroll).cloned();
        let ctrl = node.and_then(|n| js::with_dom(|d| crate::control_for(d, n))).filter(|&c| !js::with_dom(|d| forms::is_disabled(d, c)));
        let focusable = node.and_then(|n| {
            js::with_dom(|d| {
                let mut cur = Some(n);
                while let Some(x) = cur {
                    if crate::a11y::is_focusable(d, x) {
                        return Some(x);
                    }
                    cur = d.nodes[x].parent;
                }
                None
            })
        });
        let toggled = ctrl.filter(|&c| js::with_dom(|d| matches!(forms::control(d, c), Some(forms::Control::Checkbox | forms::Control::Radio))));
        if let Some(c) = toggled {
            js::with_dom_mut(|d| forms::toggle(d, c));
        }
        let allowed = match (node, self.page.js.as_mut()) {
            (Some(n), Some(js)) => js.click(n),
            _ => true,
        };
        if let Some(c) = toggled {
            if allowed {
                self.fire(c, "input");
                self.fire(c, "change");
            } else {
                js::with_dom_mut(|d| forms::toggle(d, c));
            }
        }
        self.change_focus(ctrl.or(focusable));
        // window.open during the click opens a tab (if the sandbox allows pop-ups).
        for u in js::take_open_requests() {
            if self.job.sandbox.as_ref().is_none_or(|s| s.popups) {
                out(Update::OpenTab(net::resolve(&self.page.url, &u)));
            }
        }
        if !allowed {
            return;
        }
        if let Some(c) = ctrl {
            match js::with_dom(|d| forms::control(d, c)) {
                Some(forms::Control::Button { submit: true, .. }) => {
                    if let Some(form) = js::with_dom(|d| forms::form_of(d, c)) {
                        self.submit(form, Some(c));
                    }
                }
                Some(forms::Control::Button { reset: true, .. }) => {
                    if let Some(form) = js::with_dom(|d| forms::form_of(d, c)) {
                        if self.fire(form, "reset") {
                            let defaults = self.page.form_defaults.clone();
                            js::with_dom_mut(|d| forms::reset(d, form, &defaults));
                        }
                    }
                }
                Some(forms::Control::Select) => {
                    // No drop-down list inside a frame yet: a click picks the next option.
                    let next = js::with_dom(|d| {
                        let opts = forms::options(d, c);
                        let cur = forms::selected_option(d, c).and_then(|o| opts.iter().position(|&x| x == o)).unwrap_or(0);
                        opts.get((cur + 1) % opts.len().max(1)).copied()
                    });
                    if let Some(o) = next {
                        js::with_dom_mut(|d| forms::select_option(d, c, o));
                        self.fire(c, "input");
                        self.fire(c, "change");
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(href) = href {
            self.follow(&href, node, out);
        }
    }

    /// Follow a link: in the frame, the window (target=_top/_parent), or a new tab (_blank).
    fn follow(&mut self, href: &str, node: Option<NodeId>, out: &mut dyn FnMut(Update)) {
        let target = net::resolve(&self.page.url, href);
        if target.starts_with("javascript:") || target.starts_with("mailto:") || !security::page_may_navigate(&self.page.url, &target) {
            return;
        }
        let name = node
            .and_then(|n| {
                js::with_dom(|d| {
                    let mut cur = Some(n);
                    while let Some(x) = cur {
                        if d.tag(x) == Some("a") || d.tag(x) == Some("area") {
                            return d.attr(x, "target").map(|t| t.trim().to_ascii_lowercase());
                        }
                        cur = d.nodes[x].parent;
                    }
                    None
                })
            })
            .unwrap_or_default();
        let sandbox = self.job.sandbox.clone();
        match name.as_str() {
            "_top" | "_parent" => {
                if sandbox.as_ref().is_none_or(|s| s.top_navigation) {
                    out(Update::NavigateTop(target));
                }
            }
            "_blank" => {
                if sandbox.as_ref().is_none_or(|s| s.popups) {
                    out(Update::OpenTab(target));
                }
            }
            _ => self.navigate(&target, None),
        }
    }

    fn submit(&mut self, form: NodeId, submitter: Option<NodeId>) {
        match crate::submit_form(&mut self.page, form, submitter, true) {
            Some(Go::To(u)) => self.navigate(&u, None),
            Some(Go::Post(u, body)) => self.navigate(&u, Some(&body)),
            _ => {}
        }
    }

    fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        let node = self.lay.hit_at(x, y, self.scroll);
        let wanted = match self.page.js.as_mut() {
            Some(js) => userinput::wheel(js, node, x, y, 0.0, dy, userinput::Mods::default()),
            None => true,
        };
        if !wanted {
            return;
        }
        match self.lay.scroller_at(x, y, self.scroll, dy) {
            Some((id, max)) => {
                layout::set_element_scroll(id, (layout::element_scroll(id) + dy).clamp(0.0, max));
                if let Some(js) = self.page.js.as_mut() {
                    userinput::scrolled(js, id);
                }
                self.relayout();
            }
            None => self.scroll += dy,
        }
    }

    fn keys(&mut self, keys: &[(Key, bool, bool, bool)], text: &str) {
        let mut presses: Vec<KeyPress> = Vec::new();
        for &(k, ctrl, shift, alt) in keys {
            let ok = match self.page.js.as_mut() {
                Some(js) => {
                    let mods = userinput::Mods { ctrl, shift, alt };
                    let ok = userinput::key(js, k, false, false, mods);
                    userinput::key(js, k, true, false, mods);
                    ok
                }
                None => true,
            };
            if ok {
                presses.push(KeyPress { key: k, ctrl, shift, alt });
            }
        }
        let chars: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
        let editing = self.focused.filter(|&f| js::with_dom(|d| forms::is_editable(d, f)));
        if let Some(f) = editing {
            let multiline = js::with_dom(|d| d.tag(f) == Some("textarea"));
            let value = js::with_dom(|d| forms::value(d, f));
            let ed = self.edit.get_or_insert_with(|| crate::textedit::TextEdit::new(&value));
            ed.sync(&value);
            let ctrl_now = keys.iter().any(|k| k.1);
            let r = crate::edit_keys(ed, &presses, &chars, ctrl_now, multiline, &mut None);
            let text = ed.text.clone();
            if r.changed {
                js::with_dom_mut(|d| forms::set_value(d, f, &text));
                self.fire(f, "input");
            }
            if r.enter && !multiline {
                if let Some(form) = js::with_dom(|d| forms::form_of(d, f)) {
                    let submitter = js::with_dom(|d| forms::default_button(d, form));
                    self.submit(form, submitter);
                    return;
                }
            }
        }
        for p in &presses {
            match p.key {
                Key::Tab => {
                    let order = js::with_dom(|d| crate::a11y::tab_order(d, Some(&self.page.styles)));
                    if !order.is_empty() {
                        let i = self.focused.and_then(|f| order.iter().position(|&x| x == f));
                        let next = match (i, p.shift) {
                            (Some(i), false) => order.get(i + 1).copied(),
                            (Some(i), true) => i.checked_sub(1).and_then(|i| order.get(i).copied()),
                            (None, false) => order.first().copied(),
                            (None, true) => order.last().copied(),
                        };
                        self.change_focus(next);
                    }
                }
                _ if editing.is_some() => {}
                Key::Enter | Key::NumPadEnter => {
                    // Enter on a focused link follows it.
                    let href = self.focused.and_then(|f| js::with_dom(|d| (d.tag(f) == Some("a")).then(|| d.attr(f, "href").map(String::from)).flatten()));
                    if let Some(h) = href {
                        let f = self.focused;
                        let mut sink = |_u: Update| {};
                        self.follow(&h, f, &mut sink);
                    }
                }
                Key::Down => self.scroll += 40.0,
                Key::Up => self.scroll -= 40.0,
                Key::PageDown | Key::Space => self.scroll += self.h as f32 * 0.9,
                Key::PageUp => self.scroll -= self.h as f32 * 0.9,
                Key::Home => self.scroll = 0.0,
                Key::End => self.scroll = f32::MAX,
                _ => {}
            }
        }
    }

    fn handle(&mut self, ev: Event, out: &mut dyn FnMut(Update)) {
        match ev {
            Event::Mouse { x, y, down } => {
                let was = self.last_down;
                let node = self.lay.hit_at(x, y, self.scroll);
                let cancelled = match self.page.js.as_mut() {
                    Some(js) => self.pointer.update(js, node, x, y, down, userinput::Mods::default()),
                    None => false,
                };
                self.last_down = down[0];
                if down[0] && !was && !cancelled {
                    self.click(x, y, out);
                }
            }
            Event::Leave => {
                // A button released outside the frame never reached it.
                self.last_down = false;
                if let Some(js) = self.page.js.as_mut() {
                    self.pointer.left(js);
                }
            }
            Event::Wheel { x, y, dy } => self.wheel(x, y, dy),
            Event::Keys { keys, text } => self.keys(&keys, &text),
            Event::Blur => self.change_focus(None),
        }
    }
}

/// Run a frame until its window side goes away. `out` sends updates; it returns false once
/// nobody is listening.
pub(crate) fn run_host(job: FrameJob, events: Receiver<Event>, mut out: impl FnMut(Update) -> bool) {
    crate::FRAME_DEPTH.with(|d| d.set(1));
    let Some(fonts) = crate::load_fonts() else { return };
    let (w, h) = (job.w, job.h);
    crate::geometry::set_viewport(w as f32, h as f32);
    let (page, registered) = match load_page(&job, None) {
        Some(x) => x,
        None => {
            // Refused (X-Frame-Options / frame-ancestors): the grey box, and nothing more.
            out(Update::Picture { img: crate::refused_frame(w, h), scroll: 0.0, max: 0.0, inner: false });
            while events.recv().is_ok() {}
            return;
        }
    };
    let mut host = Host {
        page,
        lay: layout::Layout::default(),
        fonts,
        w,
        h,
        scroll: 0.0,
        pointer: userinput::Pointer::default(),
        focused: None,
        edit: None,
        job,
        registered,
        shown: true,
        last_down: false,
    };
    host.settle();
    host.relayout();
    let first = host.picture();
    if !out(first) {
        return;
    }
    let mut last_paint = Instant::now();
    let mut pending = false;
    loop {
        let busy = host.page.js.as_ref().is_some() && (js::network_busy() || pending);
        let wait = if busy { Duration::from_millis(16) } else { Duration::from_millis(50) };
        let mut changed = false;
        let mut sink: Vec<Update> = Vec::new();
        match events.recv_timeout(wait) {
            Ok(ev) => {
                let before = (host.scroll, host.page.url.clone());
                host.handle(ev, &mut |u| sink.push(u));
                // More input waiting: take it all before drawing.
                while let Ok(ev) = events.try_recv() {
                    host.handle(ev, &mut |u| sink.push(u));
                }
                changed |= before != (host.scroll, host.page.url.clone());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        for u in sink {
            if !out(u) {
                return;
            }
        }
        // Scripts: timers, network results, animation frames.
        let mut animating = false;
        if let Some(js) = host.page.js.as_mut() {
            js.tick();
            if js.wants_frame() {
                js.run_frame();
                animating = true;
            }
        }
        if let Some(t) = js::take_navigation() {
            let t = net::resolve(&host.page.url, &t);
            host.navigate(&t, None);
            changed = true;
        }
        if js::take_dirty() {
            host.page.restyle();
            changed = true;
        }
        let it = host.page.images_tick(&host.lay, host.scroll, host.h as f32);
        changed |= it.relayout || it.repaint;
        pending |= changed || animating;
        // At most 30 pictures a second.
        if pending && last_paint.elapsed() >= Duration::from_millis(33) && host.shown {
            host.relayout();
            let p = host.picture();
            if !out(p) {
                return;
            }
            last_paint = Instant::now();
            pending = animating;
        }
    }
}

// ---------------------------------------------------------------- the window side

/// How a live frame is reached.
enum Sink {
    Thread(Sender<Event>),
    Process(u32),
}

struct Live {
    key: String,
    /// The page that loaded it (claimed by that page's document when it's shown).
    owner: String,
    doc: Option<u64>,
    born: Instant,
    seen: Instant,
    w: u32,
    h: u32,
    sink: Sink,
    updates: Receiver<Update>,
    scroll: f32,
    max: f32,
    inner: bool,
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Sink::Process(id) = self.sink {
            isolated::close(id);
        }
    }
}

static LIVE_ON: AtomicBool = AtomicBool::new(false);
static LIVE: Mutex<Vec<Live>> = Mutex::new(Vec::new());

/// Frames stay live in the window (snapshots and tests draw them once instead).
pub fn enable() {
    LIVE_ON.store(true, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    LIVE_ON.load(Ordering::Relaxed) && !std::env::var("IRONSWIFT_LIVE_FRAMES").is_ok_and(|v| v == "0")
}

/// Start a live frame for the iframe `key` on the page `owner`, and wait (until `deadline`)
/// for its first picture.
pub fn start(job: FrameJob, key: &str, owner: &str, deadline: Instant) -> Option<image::RgbaImage> {
    let (w, h) = (job.w, job.h);
    let (utx, urx) = channel::<Update>();
    let sink = if isolated::should_isolate(&job) {
        Sink::Process(isolated::open(&job, utx)?)
    } else {
        let (etx, erx) = channel::<Event>();
        std::thread::Builder::new()
            .name("frame".into())
            .stack_size(crate::MAIN_STACK)
            .spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_host(job, erx, |u| utx.send(u).is_ok())));
            })
            .ok()?;
        Sink::Thread(etx)
    };
    let mut live = Live { key: key.to_string(), owner: owner.to_string(), doc: None, born: Instant::now(), seen: Instant::now(), w, h, sink, updates: urx, scroll: 0.0, max: 0.0, inner: false };
    let first = loop {
        match live.updates.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Update::Picture { img, scroll, max, inner }) => {
                (live.scroll, live.max, live.inner) = (scroll, max, inner);
                break Some(img);
            }
            Ok(_) => continue,
            Err(_) => break None,
        }
    };
    if first.is_some() {
        if let Ok(mut l) = LIVE.lock() {
            l.push(live);
        }
    }
    first
}

/// What the window should do after `pump`.
#[derive(Default, Debug)]
pub struct Pumped {
    pub repaint: bool,
    pub navigate: Option<String>,
    pub open_tab: Option<String>,
}

/// Called every frame for the page in the window: claim its new frames, put their latest
/// pictures in place, and pass on navigation requests. Frames of pages gone for a while are
/// shut down.
pub fn pump(doc: u64, url: &str, images: &mut crate::images::Images) -> Pumped {
    let mut out = Pumped::default();
    let Ok(mut live) = LIVE.lock() else { return out };
    let now = Instant::now();
    for f in live.iter_mut() {
        if f.doc.is_none() && f.owner == url {
            f.doc = Some(doc);
        }
        if f.doc != Some(doc) {
            continue;
        }
        f.seen = now;
        let mut latest = None;
        while let Ok(u) = f.updates.try_recv() {
            match u {
                Update::Picture { img, scroll, max, inner } => {
                    (f.scroll, f.max, f.inner) = (scroll, max, inner);
                    latest = Some(img);
                }
                Update::NavigateTop(t) => out.navigate = Some(t),
                Update::OpenTab(t) => out.open_tab = Some(t),
            }
        }
        if let Some(img) = latest {
            images.insert(&f.key, img);
            out.repaint = true;
        }
    }
    // Never shown (a page loaded but not displayed), or not shown for three minutes.
    live.retain(|f| match f.doc {
        None => now - f.born < Duration::from_secs(60),
        Some(_) => now - f.seen < Duration::from_secs(180),
    });
    out
}

/// The live frame under the window point (x, page_y) (page coordinates of the shown
/// layout): its key and the point in the frame's own pixels.
pub fn frame_at(doc: u64, lay: &layout::Layout, x: f32, page_y: f32) -> Option<(String, f32, f32)> {
    let live = LIVE.lock().ok()?;
    lay.items.iter().rev().find_map(|it| match it {
        crate::paint::Item::Image { x: ix, y: iy, w, h, src, .. } if src.starts_with("iframe:") && x >= *ix && x < ix + w && page_y >= *iy && page_y < iy + h => {
            let f = live.iter().find(|f| f.doc == Some(doc) && f.key == *src)?;
            Some((src.clone(), (x - ix) * f.w as f32 / w.max(1.0), (page_y - iy) * f.h as f32 / h.max(1.0)))
        }
        _ => None,
    })
}

fn send(doc: u64, key: &str, ev: Event) {
    let Ok(live) = LIVE.lock() else { return };
    if let Some(f) = live.iter().find(|f| f.doc == Some(doc) && f.key == key) {
        match &f.sink {
            Sink::Thread(tx) => {
                let _ = tx.send(ev);
            }
            Sink::Process(id) => isolated::event(*id, &ev),
        }
    }
}

/// Is the frame `key` live for the page `doc`?
pub fn has(doc: u64, key: &str) -> bool {
    LIVE.lock().map(|l| l.iter().any(|f| f.doc == Some(doc) && f.key == key)).unwrap_or(false)
}

pub fn mouse(doc: u64, key: &str, x: f32, y: f32, down: [bool; 3]) {
    send(doc, key, Event::Mouse { x, y, down });
}

pub fn leave(doc: u64, key: &str) {
    send(doc, key, Event::Leave);
}

pub fn blur(doc: u64, key: &str) {
    send(doc, key, Event::Blur);
}

pub fn keys(doc: u64, key: &str, keys: Vec<(Key, bool, bool, bool)>, text: String) {
    send(doc, key, Event::Keys { keys, text });
}

/// Turn the wheel over a frame. False if the frame can't scroll that way (the page
/// scrolls instead).
pub fn wheel(doc: u64, key: &str, x: f32, y: f32, dy: f32) -> bool {
    let takes = {
        let Ok(live) = LIVE.lock() else { return false };
        let Some(f) = live.iter().find(|f| f.doc == Some(doc) && f.key == key) else { return false };
        f.inner || (dy > 0.0 && f.scroll < f.max - 0.5) || (dy < 0.0 && f.scroll > 0.5)
    };
    if takes {
        send(doc, key, Event::Wheel { x, y, dy });
    }
    takes
}

// ---------------------------------------------------------------- cross-site frames

/// Cross-site iframes run in their own sandboxed process (`ironswift --frame-renderer`): an
/// embedded ad or widget runs its scripts away from the browser, under a filter that keeps it
/// from writing files or starting programs (see sandbox.rs). One process serves all of them;
/// each frame is a thread in it.
///
/// Browser → process: `open <id> <url> <parent> <top> <sandbox> <w> <h>`, `event <id> …`,
/// `close <id>`. Process → browser: `<id> <update>`.
pub mod isolated {
    use super::*;
    use crate::{cookies, sandbox};
    use std::collections::HashMap;
    use std::io::Write;

    struct Proc {
        child: std::process::Child,
        stdin: std::process::ChildStdin,
    }

    static PROC: Mutex<Option<Proc>> = Mutex::new(None);
    /// Where each frame's updates go.
    static ROUTES: Mutex<Option<HashMap<u32, Sender<Update>>>> = Mutex::new(None);
    static NEXT_ID: AtomicU32 = AtomicU32::new(1);
    /// The renderer's process id (0 = none), so a stuck frame can be stopped from outside.
    static PID: AtomicU32 = AtomicU32::new(0);

    fn site(url: &str) -> Option<String> {
        url::Url::parse(url).ok()?.host_str().map(cookies::site_of)
    }

    /// Frames from another site than the top-level page.
    pub fn should_isolate(job: &FrameJob) -> bool {
        if std::env::var("IRONSWIFT_FRAME_PROCESS").is_ok_and(|v| v == "0") {
            return false;
        }
        match &job.src {
            FrameSrc::Url(u) if u.starts_with("http://") || u.starts_with("https://") => site(u) != site(&job.top),
            _ => false,
        }
    }

    fn sandbox_bytes(s: &Option<security::Sandbox>) -> Vec<u8> {
        match s {
            None => vec![],
            Some(s) => vec![s.scripts as u8, s.same_origin as u8, s.forms as u8, s.popups as u8, s.top_navigation as u8, s.modals as u8],
        }
    }

    fn sandbox_from(b: &[u8]) -> Option<security::Sandbox> {
        (b.len() == 6).then(|| security::Sandbox { scripts: b[0] == 1, same_origin: b[1] == 1, forms: b[2] == 1, popups: b[3] == 1, top_navigation: b[4] == 1, modals: b[5] == 1 })
    }

    fn encode_job(job: &FrameJob) -> Vec<u8> {
        let mut m = Vec::new();
        let url = match &job.src {
            FrameSrc::Url(u) => u.clone(),
            FrameSrc::Doc { .. } => String::new(),
        };
        put(&mut m, url.as_bytes());
        put(&mut m, job.parent.as_bytes());
        put(&mut m, job.top.as_bytes());
        put(&mut m, &sandbox_bytes(&job.sandbox));
        put(&mut m, &job.w.to_le_bytes());
        put(&mut m, &job.h.to_le_bytes());
        m
    }

    /// Start the process (if it isn't running) and its reader thread.
    fn ensure(g: &mut Option<Proc>) -> Option<()> {
        if g.is_some() {
            return Some(());
        }
        let exe = std::env::current_exe().ok()?;
        let mut child = std::process::Command::new(exe)
            .arg("--frame-renderer")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| log!("[frames] couldn't start the frame process: {e}"))
            .ok()?;
        PID.store(child.id(), Ordering::SeqCst);
        let stdin = child.stdin.take()?;
        let mut stdout = child.stdout.take()?;
        let pid = child.id();
        std::thread::Builder::new()
            .name("frame-process".into())
            .spawn(move || {
                // Route each update to its frame until the process ends.
                loop {
                    let r = (|| -> std::io::Result<(u32, Update)> {
                        let id = u32::from_le_bytes(get(&mut stdout)?.try_into().unwrap_or([0; 4]));
                        Ok((id, Update::decode(&mut stdout)?))
                    })();
                    match r {
                        Ok((id, u)) => {
                            if let Ok(routes) = ROUTES.lock() {
                                if let Some(tx) = routes.as_ref().and_then(|r| r.get(&id)) {
                                    let _ = tx.send(u);
                                }
                            }
                        }
                        Err(e) => {
                            log!("[frames] frame process: {e}");
                            // Its frames stop (they keep their last picture); the next frame
                            // starts a fresh process.
                            if let Ok(mut r) = ROUTES.lock() {
                                *r = None;
                            }
                            if let Ok(mut p) = PROC.lock() {
                                if p.as_ref().is_some_and(|p| p.child.id() == pid) {
                                    if let Some(mut p) = p.take() {
                                        let _ = p.child.kill();
                                        let _ = p.child.wait();
                                    }
                                }
                            }
                            let _ = PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            })
            .ok()?;
        *g = Some(Proc { child, stdin });
        Some(())
    }

    fn write(msg: &[u8]) -> bool {
        let Ok(mut g) = PROC.lock() else { return false };
        let Some(p) = g.as_mut() else { return false };
        p.stdin.write_all(msg).and_then(|_| p.stdin.flush()).is_ok()
    }

    /// Open a frame in the process; its updates go to `updates`. None = no process.
    pub fn open(job: &FrameJob, updates: Sender<Update>) -> Option<u32> {
        {
            let mut g = PROC.lock().ok()?;
            ensure(&mut g)?;
        }
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        ROUTES.lock().ok()?.get_or_insert_with(HashMap::new).insert(id, updates);
        let mut m = Vec::new();
        put(&mut m, b"open");
        put(&mut m, &id.to_le_bytes());
        m.extend(encode_job(job));
        if !write(&m) {
            close(id);
            return None;
        }
        Some(id)
    }

    pub fn event(id: u32, ev: &Event) {
        let mut m = Vec::new();
        put(&mut m, b"event");
        put(&mut m, &id.to_le_bytes());
        m.extend(ev.encode());
        write(&m);
    }

    pub fn close(id: u32) {
        if let Ok(mut r) = ROUTES.lock() {
            if let Some(r) = r.as_mut() {
                r.remove(&id);
            }
        }
        let mut m = Vec::new();
        put(&mut m, b"close");
        put(&mut m, &id.to_le_bytes());
        write(&m);
    }

    /// Draw a frame once in the frame process (snapshots, frames that can't be live).
    /// None = the process isn't available (the caller draws it here instead).
    pub fn render(job: &FrameJob) -> Option<Option<image::RgbaImage>> {
        let (tx, rx) = channel();
        let id = open(job, tx)?;
        let r = loop {
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Update::Picture { img, .. }) => break Some(img),
                Ok(_) => continue,
                Err(_) => break None,
            }
        };
        close(id);
        Some(r)
    }

    /// Stop a frame that's taking too long (its process is restarted for the next frame).
    pub fn stop_stuck() {
        let pid = PID.swap(0, Ordering::SeqCst);
        if pid != 0 {
            // SAFETY: sending a signal to our own child process.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }

    /// `ironswift --frame-renderer`: run the frames the browser opens, one thread each, until
    /// stdin closes.
    pub fn serve() {
        if !sandbox::protect_frame_renderer() {
            log!("[frames] refusing to run without a sandbox");
            return;
        }
        if crate::load_fonts().is_none() {
            return;
        }
        js::use_memory_storage();
        let out = std::sync::Arc::new(Mutex::new(std::io::stdout()));
        let mut frames: HashMap<u32, Sender<Event>> = HashMap::new();
        let mut input = std::io::stdin().lock();
        loop {
            let r = (|| -> std::io::Result<()> {
                let kind = get(&mut input)?;
                let id = u32::from_le_bytes(get(&mut input)?.try_into().unwrap_or([0; 4]));
                match kind.as_slice() {
                    b"open" => {
                        let url = String::from_utf8_lossy(&get(&mut input)?).to_string();
                        let parent = String::from_utf8_lossy(&get(&mut input)?).to_string();
                        let top = String::from_utf8_lossy(&get(&mut input)?).to_string();
                        let sandbox = sandbox_from(&get(&mut input)?);
                        let w = u32::from_le_bytes(get(&mut input)?.try_into().unwrap_or([0; 4])).clamp(1, 2000);
                        let h = u32::from_le_bytes(get(&mut input)?.try_into().unwrap_or([0; 4])).clamp(1, 2000);
                        let (reply, _) = channel();
                        let job = FrameJob { src: FrameSrc::Url(url), w, h, parent, top, sandbox, reply };
                        let (etx, erx) = channel();
                        frames.insert(id, etx);
                        let out = out.clone();
                        let _ = std::thread::Builder::new().name("frame".into()).stack_size(crate::MAIN_STACK).spawn(move || {
                            let send = |u: Update| {
                                let mut m = Vec::new();
                                put(&mut m, &id.to_le_bytes());
                                m.extend(u.encode());
                                out.lock().map(|mut o| o.write_all(&m).and_then(|_| o.flush()).is_ok()).unwrap_or(false)
                            };
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_host(job, erx, send)));
                        });
                    }
                    b"event" => {
                        let ev = Event::decode(&mut input)?;
                        if let Some(tx) = frames.get(&id) {
                            let _ = tx.send(ev);
                        }
                    }
                    b"close" => {
                        frames.remove(&id);
                    }
                    _ => return Err(std::io::Error::other("unknown message")),
                }
                Ok(())
            })();
            if r.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_and_updates_round_trip() {
        let evs = vec![
            Event::Mouse { x: 1.5, y: 2.0, down: [true, false, true] },
            Event::Leave,
            Event::Wheel { x: 3.0, y: 4.0, dy: -45.0 },
            Event::Keys { keys: vec![(Key::A, true, false, false), (Key::Backspace, false, true, false)], text: "héllo".into() },
            Event::Blur,
        ];
        for e in evs {
            let b = e.encode();
            assert_eq!(Event::decode(&mut b.as_slice()).unwrap(), e);
        }
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let b = Update::Picture { img: img.clone(), scroll: 10.0, max: 99.0, inner: true }.encode();
        match Update::decode(&mut b.as_slice()).unwrap() {
            Update::Picture { img: i, scroll, max, inner } => assert!(i == img && scroll == 10.0 && max == 99.0 && inner),
            _ => panic!(),
        }
        let b = Update::NavigateTop("https://x.org/".into()).encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::NavigateTop(u) if u == "https://x.org/"));
        // Garbage is refused, not misread.
        assert!(Event::decode(&mut &b"\x03\0\0\0zzz"[..]).is_err());
    }

    #[test]
    fn browser_shortcuts_stay_with_the_window() {
        assert!(for_frame(Key::A, false));
        assert!(for_frame(Key::A, true));
        assert!(!for_frame(Key::T, true));
        assert!(!for_frame(Key::L, true));
        assert!(!for_frame(Key::F5, false));
    }
}
