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
use crate::window::Key;
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
    /// The frame's box changed size (a top-level page in a site process follows the window).
    /// The page's new size in device pixels, and the window's screen scale.
    Resize { w: u32, h: u32, scale: f32 },
    /// Scroll the page to this y (find in page jumping to a match).
    ScrollTo(f32),
    /// The window asks the page something: "devtools" (a snapshot for the developer tools),
    /// "run" (console code in `arg`) or "print" (the printed sheets, as PNGs: see print_sheets).
    /// Answered with Update::Answer.
    Ask { what: String, arg: String },
    /// A screen reader is listening (true) or stopped (false): send Update::A11y while it is.
    A11y(bool),
    /// A screen reader's request for a page element: 0 focus, 1 click, 2 scroll into view,
    /// 3 set the value of a text field to `value`.
    A11yAction { kind: u8, node: u32, value: String },
    /// The user answered permission requests `ids` (Update::Permission).
    PermDone { ids: Vec<u64>, granted: bool },
    /// Fill these fields (a saved login or an earlier entry picked under a field).
    Fill(Vec<(u32, String)>),
    /// An option was picked in the window's list for the <select> `id` (see Update::Select).
    SelectChoose { id: u32, index: u32 },
    /// Back or Forward moved within the page's own history entries (a top-level page in a site
    /// process): its new address and state, and whether to fire popstate.
    HistoryMoved { url: String, state: String, popstate: bool },
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
    /// A form posted to the window (a top-level page in a site process): the address, the
    /// body's content type, and the body.
    NavigatePost(String, String, Vec<u8>),
    /// The page's title changed (a top-level page in a site process).
    Title(String),
    /// The text on the page and where it's drawn (page coordinates), for find in page and
    /// text selection in the window (a top-level page in a site process).
    Text(Vec<crate::chrome::TextRun>),
    /// The page's accessibility tree (a11y_remote.rs), when it changed and a screen reader is
    /// listening (a top-level page in a site process).
    A11y(Vec<u8>),
    /// The page's script asks for a permission (a top-level page in a site process): the
    /// request's id, what for, and the page's address. Answered with Event::PermDone.
    Permission { id: u64, kind: crate::permissions::Kind, url: String },
    /// A text field got the focus (a top-level page in a site process): what autofill needs to
    /// suggest for it, and the field's box (x, bottom, width; frame pixels after scrolling).
    Field { id: u32, x: f32, y: f32, w: f32, info: crate::autofill::FieldInfo },
    /// A form is being sent (a top-level page in a site process): its entries and login, for the
    /// window to remember or offer to save.
    Submitted(crate::autofill::Submitted),
    /// The answer to Event::Ask.
    Answer { what: String, data: Vec<u8> },
    /// A right click the page didn't cancel (a top-level page in a site process): the link and
    /// the image under the mouse (absolute addresses), for the window's menu.
    ContextMenu { link: Option<String>, image: Option<String> },
    /// A <select> was clicked (a top-level page in a site process): the window shows its list
    /// of options under the box at (x, y, w) (frame pixels, after scrolling) and answers with
    /// Event::SelectChoose.
    Select { id: u32, x: f32, y: f32, w: f32, options: Vec<String> },
    /// The page's script changed its history: pushState, replaceState, location.hash, back…
    /// (a top-level page in a site process).
    History(js::HistoryOp),
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

pub(crate) fn put(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

pub(crate) fn get(r: &mut impl std::io::Read) -> std::io::Result<Vec<u8>> {
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

pub(crate) fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub(crate) fn read_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
}

impl Event {
    pub(crate) fn encode(&self) -> Vec<u8> {
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
            Event::Resize { w, h, scale } => {
                put(&mut m, b"resize");
                put(&mut m, &f32s(&[*w as f32, *h as f32, *scale]));
            }
            Event::ScrollTo(y) => {
                put(&mut m, b"scrollto");
                put(&mut m, &f32s(&[*y]));
            }
            Event::PermDone { ids, granted } => {
                put(&mut m, b"permdone");
                put(&mut m, &[*granted as u8]);
                put(&mut m, serde_json::json!(ids).to_string().as_bytes());
            }
            Event::Fill(fields) => {
                put(&mut m, b"fill");
                put(&mut m, serde_json::json!(fields).to_string().as_bytes());
            }
            Event::A11y(on) => {
                put(&mut m, b"a11y");
                put(&mut m, &[*on as u8]);
            }
            Event::A11yAction { kind, node, value } => {
                put(&mut m, b"a11yact");
                put(&mut m, &f32s(&[*kind as f32, *node as f32]));
                put(&mut m, value.as_bytes());
            }
            Event::Ask { what, arg } => {
                put(&mut m, b"ask");
                put(&mut m, what.as_bytes());
                put(&mut m, arg.as_bytes());
            }
            Event::SelectChoose { id, index } => {
                put(&mut m, b"choose");
                put(&mut m, &f32s(&[*id as f32, *index as f32]));
            }
            Event::HistoryMoved { url, state, popstate } => {
                put(&mut m, b"histmoved");
                put(&mut m, url.as_bytes());
                put(&mut m, state.as_bytes());
                put(&mut m, &[*popstate as u8]);
            }
        }
        m
    }

    pub(crate) fn decode(r: &mut impl std::io::Read) -> std::io::Result<Event> {
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
            b"resize" => {
                let v = read_f32s(&get(r)?);
                if v.len() != 3 {
                    return Err(std::io::Error::other("bad resize event"));
                }
                Event::Resize { w: (v[0] as u32).clamp(1, 8000), h: (v[1] as u32).clamp(1, 8000), scale: v[2] }
            }
            b"scrollto" => match read_f32s(&get(r)?)[..] {
                [y] if y.is_finite() => Event::ScrollTo(y.max(0.0)),
                _ => return Err(std::io::Error::other("bad scroll")),
            },
            b"permdone" => {
                let granted = get(r)? == [1];
                let ids: Vec<u64> = serde_json::from_slice(&get(r)?).map_err(|_| std::io::Error::other("bad answer"))?;
                Event::PermDone { ids: ids.into_iter().take(16).collect(), granted }
            }
            b"fill" => Event::Fill(serde_json::from_slice::<Vec<(u32, String)>>(&get(r)?).map_err(|_| std::io::Error::other("bad fill"))?.into_iter().take(20).collect()),
            b"a11y" => Event::A11y(get(r)? == [1]),
            b"a11yact" => match read_f32s(&get(r)?)[..] {
                [kind, node] if (0.0..=3.0).contains(&kind) && node >= 0.0 => Event::A11yAction { kind: kind as u8, node: node as u32, value: String::from_utf8_lossy(&get(r)?).to_string() },
                _ => return Err(std::io::Error::other("bad a11y action")),
            },
            b"ask" => {
                let what = String::from_utf8_lossy(&get(r)?).to_string();
                Event::Ask { what, arg: String::from_utf8_lossy(&get(r)?).to_string() }
            }
            b"choose" => match read_f32s(&get(r)?)[..] {
                [id, index] if id >= 0.0 && index >= 0.0 => Event::SelectChoose { id: id as u32, index: index as u32 },
                _ => return Err(std::io::Error::other("bad choice")),
            },
            b"histmoved" => {
                let url = String::from_utf8_lossy(&get(r)?).to_string();
                let state = String::from_utf8_lossy(&get(r)?).to_string();
                Event::HistoryMoved { url, state, popstate: get(r)? == [1] }
            }
            _ => return Err(std::io::Error::other("unknown event")),
        })
    }
}

impl Update {
    pub(crate) fn encode(&self) -> Vec<u8> {
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
            Update::NavigatePost(u, ct, body) => {
                put(&mut m, b"post");
                put(&mut m, u.as_bytes());
                put(&mut m, ct.as_bytes());
                put(&mut m, body);
            }
            Update::Title(t) => {
                put(&mut m, b"title");
                put(&mut m, t.as_bytes());
            }
            Update::Permission { id, kind, url } => {
                put(&mut m, b"perm");
                put(&mut m, id.to_string().as_bytes());
                put(&mut m, kind.key().as_bytes());
                put(&mut m, url.as_bytes());
            }
            Update::Field { id, x, y, w, info } => {
                put(&mut m, b"field");
                put(&mut m, &f32s(&[*id as f32, *x, *y, *w]));
                let login = info.login.map(|(u, p)| (u.map(|u| u as u32), p as u32));
                put(&mut m, serde_json::json!({ "k": info.kind, "l": login, "t": info.typed }).to_string().as_bytes());
            }
            Update::Submitted(sub) => {
                put(&mut m, b"submitted");
                put(&mut m, serde_json::json!({ "e": sub.entries, "l": sub.login }).to_string().as_bytes());
            }
            Update::A11y(b) => {
                put(&mut m, b"a11ytree");
                put(&mut m, b);
            }
            Update::Answer { what, data } => {
                put(&mut m, b"answer");
                put(&mut m, what.as_bytes());
                put(&mut m, data);
            }
            Update::ContextMenu { link, image } => {
                put(&mut m, b"menu");
                put(&mut m, link.as_deref().unwrap_or("").as_bytes());
                put(&mut m, image.as_deref().unwrap_or("").as_bytes());
            }
            Update::Select { id, x, y, w, options } => {
                put(&mut m, b"select");
                put(&mut m, &f32s(&[*id as f32, *x, *y, *w, options.len() as f32]));
                for o in options {
                    put(&mut m, o.as_bytes());
                }
            }
            Update::History(op) => {
                put(&mut m, b"history");
                let (kind, url, state) = match op {
                    js::HistoryOp::Push(u, s) => ("push", u.clone(), s.clone()),
                    js::HistoryOp::Replace(u, s) => ("replace", u.clone(), s.clone()),
                    js::HistoryOp::Hash(u, false) => ("hash", u.clone(), String::new()),
                    js::HistoryOp::Hash(u, true) => ("hash-replace", u.clone(), String::new()),
                    js::HistoryOp::Go(n) => ("go", n.to_string(), String::new()),
                };
                put(&mut m, kind.as_bytes());
                put(&mut m, url.as_bytes());
                put(&mut m, state.as_bytes());
            }
            Update::Text(runs) => {
                put(&mut m, b"text");
                put(&mut m, &f32s(&[runs.len() as f32]));
                for r in runs {
                    put(&mut m, &f32s(&[r.x, r.y, r.size, r.font as f32, r.fixed as u8 as f32]));
                    put(&mut m, r.text.as_bytes());
                }
            }
        }
        m
    }

    pub(crate) fn decode(r: &mut impl std::io::Read) -> std::io::Result<Update> {
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
            b"post" => {
                let u = String::from_utf8_lossy(&get(r)?).to_string();
                let ct = String::from_utf8_lossy(&get(r)?).to_string();
                Update::NavigatePost(u, ct, get(r)?)
            }
            b"title" => Update::Title(String::from_utf8_lossy(&get(r)?).to_string()),
            b"perm" => {
                let id = String::from_utf8_lossy(&get(r)?).parse::<u64>().map_err(|_| std::io::Error::other("bad permission"))?;
                let kind = crate::permissions::Kind::parse(&String::from_utf8_lossy(&get(r)?)).ok_or_else(|| std::io::Error::other("bad permission"))?;
                Update::Permission { id, kind, url: String::from_utf8_lossy(&get(r)?).to_string() }
            }
            b"field" => {
                let [id, x, y, w] = read_f32s(&get(r)?)[..] else { return Err(std::io::Error::other("bad field")) };
                let v: serde_json::Value = serde_json::from_slice(&get(r)?).map_err(|_| std::io::Error::other("bad field"))?;
                let login = v.get("l").and_then(|l| serde_json::from_value::<Option<(Option<u32>, u32)>>(l.clone()).ok()).flatten();
                let b = Update::Permission { id: 12, kind: crate::permissions::Kind::Location, url: "https://a.com/".into() }.encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::Permission { id: 12, kind: crate::permissions::Kind::Location, url } if url == "https://a.com/"));
        let info = crate::autofill::FieldInfo {
                    kind: v.get("k").and_then(|k| k.as_str()).map(String::from),
                    login: login.map(|(u, p)| (u.map(|u| u as NodeId), p as NodeId)),
                    typed: v.get("t").and_then(|t| t.as_str()).unwrap_or("").chars().take(500).collect(),
                };
                Update::Field { id: id.max(0.0) as u32, x, y, w, info }
            }
            b"submitted" => {
                let v: serde_json::Value = serde_json::from_slice(&get(r)?).map_err(|_| std::io::Error::other("bad form"))?;
                Update::Submitted(crate::autofill::Submitted {
                    entries: v.get("e").and_then(|e| serde_json::from_value(e.clone()).ok()).unwrap_or_default(),
                    login: v.get("l").and_then(|l| serde_json::from_value(l.clone()).ok()).flatten(),
                })
            }
            b"a11ytree" => Update::A11y(get(r)?),
            b"answer" => {
                let what = String::from_utf8_lossy(&get(r)?).to_string();
                Update::Answer { what, data: get(r)? }
            }
            b"menu" => {
                let some = |b: Vec<u8>| Some(String::from_utf8_lossy(&b).to_string()).filter(|s| !s.is_empty());
                let link = some(get(r)?);
                Update::ContextMenu { link, image: some(get(r)?) }
            }
            b"select" => {
                let [id, x, y, w, n] = read_f32s(&get(r)?)[..] else { return Err(std::io::Error::other("bad select")) };
                if !(0.0..=10_000.0).contains(&n) || id < 0.0 {
                    return Err(std::io::Error::other("bad select"));
                }
                let mut options = Vec::with_capacity(n as usize);
                for _ in 0..n as usize {
                    options.push(String::from_utf8_lossy(&get(r)?).to_string());
                }
                Update::Select { id: id as u32, x, y, w, options }
            }
            b"history" => {
                let kind = get(r)?;
                let url = String::from_utf8_lossy(&get(r)?).to_string();
                let state = String::from_utf8_lossy(&get(r)?).to_string();
                Update::History(match kind.as_slice() {
                    b"push" => js::HistoryOp::Push(url, state),
                    b"replace" => js::HistoryOp::Replace(url, state),
                    b"hash" => js::HistoryOp::Hash(url, false),
                    b"hash-replace" => js::HistoryOp::Hash(url, true),
                    b"go" => js::HistoryOp::Go(url.parse::<i32>().map_err(|_| std::io::Error::other("bad history"))?.clamp(-100, 100)),
                    _ => return Err(std::io::Error::other("bad history")),
                })
            }
            b"text" => {
                let n = match read_f32s(&get(r)?)[..] {
                    [n] if (0.0..=200_000.0).contains(&n) => n as usize,
                    _ => return Err(std::io::Error::other("bad text")),
                };
                let mut runs = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    let v = read_f32s(&get(r)?);
                    let text = String::from_utf8_lossy(&get(r)?).to_string();
                    let [x, y, size, font, fixed] = v[..] else { return Err(std::io::Error::other("bad text run")) };
                    runs.push(crate::chrome::TextRun { x, y, text, font: font as u8, size, fixed: fixed > 0.5 });
                }
                Update::Text(runs)
            }
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
    last_right: bool,
    /// Updates produced away from an input handler (a top-level page's navigations).
    outbox: Vec<Update>,
    /// The title last reported (top-level pages).
    title: String,
    /// The page text last sent to the window (a top-level page in a site process).
    sent_runs: Vec<crate::chrome::TextRun>,
    /// A screen reader is listening in the window, and the tree last sent to it.
    a11y_on: bool,
    sent_a11y: Vec<u8>,
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
        // A top-level page (in a site process): no frame identity, no framing rules. The first
        // load may be a form post the window handed over.
        Some((u, post)) if job.top_level => {
            let first = (url.is_none()).then(|| crate::process::take_post(&u)).flatten();
            let p = crate::fetch_page(&u, post.or(first.as_ref()));
            Some((crate::build_page(p), None))
        }
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
        // A top-level page tells the window its text (find in page, selection), when it changes.
        if self.job.top_level {
            let runs = crate::chrome::text_runs(&self.lay);
            if runs != self.sent_runs {
                self.sent_runs = runs.clone();
                self.outbox.push(Update::Text(runs));
            }
            self.send_a11y();
        }
    }

    /// Send the accessibility tree if a screen reader is listening and it changed.
    fn send_a11y(&mut self) {
        if !self.a11y_on || !self.job.top_level {
            return;
        }
        let tree = js::with_dom(|d| {
            let pg = crate::a11y_bridge::Page { dom: d, styles: &self.page.styles, layout: &self.lay, focused: self.focused, scroll: 0.0, url: &self.page.url };
            crate::a11y_remote::encode(&crate::a11y_bridge::page_tree(&pg))
        });
        if tree != self.sent_a11y {
            self.sent_a11y = tree.clone();
            self.outbox.push(Update::A11y(tree));
        }
    }

    /// A screen reader's request (Event::A11yAction).
    fn a11y_action(&mut self, kind: u8, node: u32, value: String, out: &mut dyn FnMut(Update)) {
        let n = node as NodeId;
        if !js::with_dom(|d| n < d.nodes.len()) {
            return;
        }
        let rect = js::with_dom(|d| crate::a11y_bridge::node_rect(d, &self.lay, n));
        let reveal = |host: &mut Self| {
            if let Some(r) = rect {
                if r.y < host.scroll || r.y + r.h > host.scroll + host.h as f32 {
                    host.scroll = (r.y - 40.0).max(0.0);
                }
            }
        };
        match kind {
            0 => {
                reveal(self);
                self.change_focus(Some(n));
            }
            1 => {
                reveal(self);
                if let Some(r) = rect {
                    self.click(r.x + r.w / 2.0, r.y + r.h / 2.0 - self.scroll, out);
                }
            }
            2 => reveal(self),
            _ => {
                if js::with_dom(|d| forms::is_editable(d, n)) {
                    js::with_dom_mut(|d| forms::set_value(d, n, &value));
                    self.fire(n, "input");
                    self.fire(n, "change");
                }
            }
        }
        self.relayout();
    }

    /// Open another document in the frame (a link, a form, a script setting location).
    fn navigate(&mut self, url: &str, post: Option<&forms::PostBody>) {
        if !security::page_may_navigate(&self.page.url, url) || url.starts_with("javascript:") {
            return;
        }
        // A top-level page doesn't navigate itself: the window does (its address bar and
        // history, and the right site's process for the new page).
        if self.job.top_level {
            self.outbox.push(match post {
                Some(p) => Update::NavigatePost(url.to_string(), p.content_type.clone(), p.bytes.clone()),
                None => Update::NavigateTop(url.to_string()),
            });
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
            // A text field on a top-level page: the window may suggest saved logins or entries.
            if self.job.top_level && js::with_dom(|d| matches!(forms::control(d, n), Some(forms::Control::Text { .. }))) {
                if let Some(r) = self.lay.hits.iter().rev().find(|(_, h)| *h == n).map(|(r, _)| *r) {
                    let info = js::with_dom(|d| crate::autofill::field_info(d, n));
                    self.outbox.push(Update::Field { id: n as u32, x: r.x, y: r.y + r.h - self.scroll + 2.0, w: r.w.max(220.0), info });
                }
            }
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
                Some(forms::Control::Select) if self.job.top_level => {
                    // A top-level page: the window shows the list (Event::SelectChoose answers).
                    let r = self.lay.hits.iter().rev().find(|(_, n)| *n == c).map(|(r, _)| *r);
                    let options = js::with_dom(|d| forms::options(d, c).into_iter().map(|o| forms::option_label(d, o)).collect());
                    if let Some(r) = r {
                        out(Update::Select { id: c as u32, x: r.x, y: r.y + r.h - self.scroll, w: r.w.max(120.0), options });
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
        let go = crate::submit_form(&mut self.page, form, submitter, true);
        // The window remembers its entries and offers to save its login (the saved ones never
        // come into this process).
        if go.is_some() && self.job.top_level {
            let sub = js::with_dom(|d| crate::autofill::submitted_fields(d, form));
            if sub != crate::autofill::Submitted::default() {
                self.outbox.push(Update::Submitted(sub));
            }
        }
        match go {
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
                // A right press: contextmenu for the page, then (top-level, not cancelled) the
                // window's menu for what's under the mouse.
                let right = down[2] && !self.last_right;
                self.last_right = down[2];
                if right {
                    let shown = match (node, self.page.js.as_mut()) {
                        (Some(n), Some(js)) => crate::clicks::contextmenu(js, n, x, y),
                        _ => true,
                    };
                    if shown && self.job.top_level {
                        let url = &self.page.url;
                        let link = self.lay.link_at(x, y, self.scroll).map(|h| net::resolve(url, h)).filter(|u| !u.starts_with("javascript:"));
                        let py = y + self.scroll;
                        let image = self.lay.items.iter().rev().find_map(|it| match it {
                            crate::paint::Item::Image { x: ix, y: iy, w, h, src, .. } if x >= *ix && x < ix + w && py >= *iy && py < iy + h && !src.starts_with("iframe:") => Some(net::resolve(url, src)),
                            _ => None,
                        });
                        out(Update::ContextMenu { link, image });
                    }
                }
            }
            Event::Leave => {
                // A button released outside the frame never reached it.
                self.last_down = false;
                self.last_right = false;
                if let Some(js) = self.page.js.as_mut() {
                    self.pointer.left(js);
                }
            }
            Event::Wheel { x, y, dy } => self.wheel(x, y, dy),
            Event::Keys { keys, text } => self.keys(&keys, &text),
            Event::Blur => self.change_focus(None),
            Event::Resize { w, h, scale } => {
                // The window may have moved to a screen with another scale.
                let rescaled = (scale - crate::zoom::device_scale()).abs() > 1e-3;
                crate::zoom::set_render_scale(scale);
                if (w, h) != (self.w, self.h) || rescaled {
                    (self.w, self.h) = (w, h);
                    self.relayout();
                }
            }
            Event::ScrollTo(y) => self.scroll = y,
            Event::PermDone { ids, granted } => {
                if let Some(js) = self.page.js.as_mut() {
                    js.permission_done(&ids, granted);
                }
            }
            Event::Fill(fields) => {
                for (id, value) in fields {
                    let n = id as NodeId;
                    if js::with_dom(|d| n < d.nodes.len() && forms::is_editable(d, n)) {
                        js::with_dom_mut(|d| forms::set_value(d, n, &value));
                        if self.focused == Some(n) {
                            let mut e = crate::textedit::TextEdit::new(&value);
                            let len = e.len();
                            e.move_to(len, false);
                            self.edit = Some(e);
                        }
                        self.fire(n, "input");
                        self.fire(n, "change");
                    }
                }
                self.relayout();
            }
            Event::A11y(on) => {
                self.a11y_on = on;
                self.sent_a11y.clear();
                self.send_a11y();
            }
            Event::A11yAction { kind, node, value } => self.a11y_action(kind, node, value, out),
            Event::Ask { what, arg } => {
                let data: Vec<u8> = match what.as_str() {
                    "print" => {
                        let mut b = Vec::new();
                        for sheet in crate::print_pages(&self.page, self.fonts) {
                            let mut png = Vec::new();
                            if image::DynamicImage::ImageRgb8(sheet).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
                                put(&mut b, &png);
                            }
                        }
                        b
                    }
                    "devtools" => {
                        let (scripts, errors) = self.page.js.as_ref().map_or((0, 0), |j| (j.stats.scripts, j.stats.errors));
                        let p = &self.page;
                        js::with_dom(|d| crate::devtools::capture(d, &p.url, &p.title, p.loaded_at, scripts, errors));
                        crate::devtools::snapshot().into_bytes()
                    }
                    "run" => match self.page.js.as_mut().map(|j| j.run(&arg)) {
                        Some(Ok(v)) => format!("ok:{v}"),
                        Some(Err(e)) => format!("err:{e}"),
                        None => "err:this page doesn't run scripts".into(),
                    }
                    .into_bytes(),
                    _ => Vec::new(),
                };
                if what == "run" && js::take_dirty() {
                    self.page.restyle();
                    self.relayout();
                }
                out(Update::Answer { what, data });
            }
            Event::SelectChoose { id, index } => {
                let c = id as NodeId;
                let opt = js::with_dom(|d| (c < d.nodes.len() && matches!(forms::control(d, c), Some(forms::Control::Select))).then(|| forms::options(d, c).get(index as usize).copied()).flatten());
                if let Some(o) = opt {
                    js::with_dom_mut(|d| forms::select_option(d, c, o));
                    self.fire(c, "input");
                    self.fire(c, "change");
                }
            }
            Event::HistoryMoved { url, state, popstate } => {
                if crate::net::origin_of(&url) == crate::net::origin_of(&self.page.url) {
                    let old = std::mem::replace(&mut self.page.url, url.clone());
                    if let Some(js) = self.page.js.as_mut() {
                        js.history_moved(&url, &state, popstate);
                    }
                    // A #fragment link (or Back to one): show its target.
                    if !popstate || old.split('#').nth(1) != url.split('#').nth(1) {
                        if let Some(y) = crate::fragment_target(&url) {
                            self.scroll = y;
                        }
                    }
                }
            }
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
        last_right: false,
        outbox: Vec::new(),
        title: String::new(),
        sent_runs: Vec::new(),
        a11y_on: false,
        sent_a11y: Vec::new(),
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
                let before = (host.scroll, host.page.url.clone(), host.w, host.h);
                host.handle(ev, &mut |u| sink.push(u));
                // More input waiting: take it all before drawing.
                while let Ok(ev) = events.try_recv() {
                    host.handle(ev, &mut |u| sink.push(u));
                }
                changed |= before != (host.scroll, host.page.url.clone(), host.w, host.h);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        for u in sink.drain(..) {
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
        // Permission requests: a top-level page's go to the window's prompt; a frame can't ask
        // (permissions::can_ask), so its are refused at once rather than left waiting.
        for (id, kind, url) in js::take_permission_requests() {
            if host.job.top_level {
                sink.push(Update::Permission { id, kind, url });
            } else if let Some(js) = host.page.js.as_mut() {
                js.permission_done(&[id], false);
            }
        }
        // History changes the page's scripts made: a top-level page's go to the window (its
        // address bar and Back/Forward); a frame's have nowhere to go yet.
        for op in js::take_history() {
            if host.job.top_level {
                if let js::HistoryOp::Push(u, _) | js::HistoryOp::Replace(u, _) = &op {
                    host.page.url = u.clone();
                }
                sink.push(Update::History(op));
            }
        }
        sink.extend(host.outbox.drain(..));
        if host.job.top_level && host.page.title != host.title {
            host.title = host.page.title.clone();
            sink.push(Update::Title(host.title.clone()));
        }
        for u in sink.drain(..) {
            if !out(u) {
                return;
            }
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
    /// A top-level page running in its site's process (process.rs).
    Site(u32),
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
    /// The iframe's sandbox (None = not sandboxed), checked here as well: a compromised
    /// frame process can't be trusted to apply it.
    sandbox: Option<security::Sandbox>,
    top_level: bool,
    /// The page's text (a top-level page in a site process) and how often it changed.
    runs: Vec<crate::chrome::TextRun>,
    runs_version: u64,
    /// Updates that came in while the window waited for an answer (site_ask), still to pump.
    held: Vec<Update>,
    /// Its accessibility tree (a top-level page in a site process), how often it changed,
    /// and whether it was asked to send it.
    a11y: Option<crate::a11y_bridge::PageTree>,
    a11y_version: u64,
    a11y_on: bool,
}

impl Drop for Live {
    fn drop(&mut self) {
        match self.sink {
            Sink::Process(id) => isolated::close(id),
            Sink::Site(id) => crate::process::close(id),
            Sink::Thread(_) => {}
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
    let (sandbox, top_level) = (job.sandbox.clone(), job.top_level);
    let (utx, urx) = channel::<Update>();
    let sink = if job.top_level {
        Sink::Site(crate::process::open(&job, utx)?)
    } else if isolated::should_isolate(&job) {
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
    let mut live = Live { key: key.to_string(), owner: owner.to_string(), doc: None, born: Instant::now(), seen: Instant::now(), w, h, sink, updates: urx, scroll: 0.0, max: 0.0, inner: false, sandbox, top_level, runs: Vec::new(), runs_version: 0, held: Vec::new(), a11y: None, a11y_version: 0, a11y_on: false };
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
    /// A form a top-level page posted: (address, body).
    pub post: Option<(String, forms::PostBody)>,
    /// A top-level page's new title.
    pub title: Option<String>,
    /// Permission requests from a top-level page in a site process: (id, what, page address).
    pub permissions: Vec<(u64, crate::permissions::Kind, String)>,
    /// A text field got the focus on a top-level page in a site process: (id, x, bottom, width,
    /// what to suggest for it).
    pub field: Option<(u32, f32, f32, f32, crate::autofill::FieldInfo)>,
    /// A form it is sending (its entries and login, to remember or offer to save).
    pub submitted: Option<crate::autofill::Submitted>,
    /// A top-level page in a site process asked for the right-click menu: (link, image).
    pub context: Option<(Option<String>, Option<String>)>,
    /// A top-level page in a site process opened a <select>: its id, where to show the list
    /// (window page coordinates: x, top, width) and the options.
    pub select: Option<(u32, f32, f32, f32, Vec<String>)>,
    /// History changes a top-level page in a site process made (still to be checked).
    pub history: Vec<js::HistoryOp>,
}

/// May this frame move the whole tab? (Its own sandbox, checked on the window's side.)
fn may_navigate_top(f: &Live) -> bool {
    f.top_level || f.sandbox.as_ref().is_none_or(|s| s.top_navigation)
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
        let held: Vec<Update> = std::mem::take(&mut f.held);
        let mut fresh = Vec::new();
        while let Ok(u) = f.updates.try_recv() {
            fresh.push(u);
        }
        for u in held.into_iter().chain(fresh) {
            match u {
                Update::Picture { img, scroll, max, inner } => {
                    (f.scroll, f.max, f.inner) = (scroll, max, inner);
                    latest = Some(img);
                }
                Update::NavigateTop(t) if may_navigate_top(f) => out.navigate = Some(t),
                Update::OpenTab(t) if f.top_level || f.sandbox.as_ref().is_none_or(|s| s.popups) => out.open_tab = Some(t),
                Update::NavigatePost(u, content_type, bytes) if may_navigate_top(f) => out.post = Some((u, forms::PostBody { content_type, bytes })),
                Update::NavigateTop(_) | Update::OpenTab(_) | Update::NavigatePost(..) => log!("[frames] {}: blocked by its sandbox", f.key),
                Update::Title(t) => out.title = Some(t),
                Update::Text(runs) if f.top_level => {
                    f.runs = runs;
                    f.runs_version += 1;
                    out.repaint = true;
                }
                Update::Text(_) => {}
                Update::History(op) if f.top_level => out.history.push(op),
                Update::Select { id, x, y, w, options } if f.top_level => out.select = Some((id, x, y, w, options)),
                Update::Select { .. } => {}
                Update::ContextMenu { link, image } if f.top_level => out.context = Some((link, image)),
                Update::ContextMenu { .. } => {}
                // Too late: whoever asked stopped waiting.
                Update::Answer { .. } => {}
                Update::A11y(b) if f.top_level => {
                    if let Some(t) = crate::a11y_remote::decode(&b) {
                        f.a11y = Some(t);
                        f.a11y_version += 1;
                    }
                }
                Update::A11y(_) => {}
                Update::Field { id, x, y, w, info } if f.top_level => out.field = Some((id, x, y, w, info)),
                Update::Submitted(sub) if f.top_level => out.submitted = Some(sub),
                Update::Field { .. } | Update::Submitted(_) => {}
                Update::Permission { id, kind, url } if f.top_level => out.permissions.push((id, kind, url)),
                Update::Permission { .. } => {}
                Update::History(_) => {}
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
            Sink::Site(id) => crate::process::event(*id, &ev),
        }
    }
}

/// The page `doc` is a top-level page running in a site process: the key of its frame (which
/// gets the keyboard whenever the address bar doesn't have it).
pub fn site_frame(doc: u64) -> Option<String> {
    let live = LIVE.lock().ok()?;
    live.iter().find(|f| f.doc == Some(doc) && matches!(f.sink, Sink::Site(_))).map(|f| f.key.clone())
}

/// Where the site-process page of `doc` is drawn in the window's layout, with its frame.
fn site_box<'a>(live: &'a [Live], doc: u64, lay: &layout::Layout) -> Option<(&'a Live, f32, f32)> {
    let f = live.iter().find(|f| f.doc == Some(doc) && matches!(f.sink, Sink::Site(_)))?;
    let (x, y) = lay.items.iter().find_map(|it| match it {
        crate::paint::Item::Image { x, y, src, .. } if *src == f.key => Some((*x, *y)),
        _ => None,
    })?;
    Some((f, x, y))
}

/// The text of the site-process page shown for `doc`, placed where the window draws it (its
/// own scrolling applied): find in page and selection work on it like on a local page.
pub fn site_runs(doc: u64, lay: &layout::Layout) -> Option<Vec<crate::chrome::TextRun>> {
    let live = LIVE.lock().ok()?;
    let (f, x0, y0) = site_box(&live, doc, lay)?;
    Some(
        f.runs
            .iter()
            .map(|r| crate::chrome::TextRun { x: r.x + x0, y: r.y + y0 - if r.fixed { 0.0 } else { f.scroll }, fixed: false, ..r.clone() })
            .collect(),
    )
}

/// Where the site-process page of `doc` is drawn in the window's layout (top-left).
pub fn site_origin(doc: u64, lay: &layout::Layout) -> Option<(f32, f32)> {
    let live = LIVE.lock().ok()?;
    site_box(&live, doc, lay).map(|(_, x, y)| (x, y))
}

/// Ask the site-process page of `doc` (shown or in a background tab) something and wait for the
/// answer (Event::Ask): None if it isn't in a site process or doesn't answer in time.
pub fn site_ask(doc: u64, what: &str, arg: &str, wait: Duration) -> Option<Vec<u8>> {
    {
        let live = LIVE.lock().ok()?;
        let f = live.iter().find(|f| f.doc == Some(doc) && matches!(f.sink, Sink::Site(_)))?;
        let Sink::Site(id) = f.sink else { return None };
        crate::process::event(id, &Event::Ask { what: what.to_string(), arg: arg.to_string() });
    }
    let end = Instant::now() + wait;
    while Instant::now() < end {
        {
            let mut live = LIVE.lock().ok()?;
            let f = live.iter_mut().find(|f| f.doc == Some(doc) && matches!(f.sink, Sink::Site(_)))?;
            while let Ok(u) = f.updates.try_recv() {
                match u {
                    Update::Answer { what: w, data } if w == what => return Some(data),
                    other => f.held.push(other),
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// The printed sheets of the site-process page of `doc` (None if it isn't in one or didn't
/// answer).
pub fn print_sheets(doc: u64) -> Option<Vec<image::RgbImage>> {
    let data = site_ask(doc, "print", "", Duration::from_secs(30))?;
    let mut r = data.as_slice();
    let mut sheets = Vec::new();
    while !r.is_empty() {
        let png = get(&mut r).ok()?;
        sheets.push(image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()?.to_rgb8());
    }
    Some(sheets).filter(|s| !s.is_empty())
}

/// The accessibility tree of the site-process page shown for `doc`, placed where the window
/// shows it, with a version that changes whenever it or its scrolling does. `listening`: a
/// screen reader is there (the page sends its tree only then).
pub fn site_a11y(doc: u64, lay: &layout::Layout, listening: bool) -> Option<(crate::a11y_bridge::PageTree, u64)> {
    let mut live = LIVE.lock().ok()?;
    let (x0, y0) = {
        let (_, x, y) = site_box(&live, doc, lay)?;
        (x, y)
    };
    let f = live.iter_mut().find(|f| f.doc == Some(doc) && matches!(f.sink, Sink::Site(_)))?;
    let Sink::Site(id) = f.sink else { return None };
    if f.a11y_on != listening {
        f.a11y_on = listening;
        crate::process::event(id, &Event::A11y(listening));
    }
    if !listening {
        return None;
    }
    let mut t = f.a11y.clone()?;
    t.scroll = f.scroll;
    t.origin = (x0, y0);
    let version = f.a11y_version.wrapping_mul(0x1_0000_0001) ^ f.scroll.to_bits() as u64 ^ ((x0.to_bits() as u64) << 20) ^ ((y0.to_bits() as u64) << 40);
    Some((t, version))
}

/// A screen reader's request for an element of the site-process page shown for `doc`
/// (kind: 0 focus, 1 click, 2 scroll into view, 3 set value).
pub fn site_a11y_action(doc: u64, kind: u8, node: usize, value: &str) {
    if let Some(key) = site_frame(doc) {
        send(doc, &key, Event::A11yAction { kind, node: node as u32, value: value.to_string() });
    }
}

/// The answer to permission requests `ids` of the site-process page shown for `doc`.
pub fn site_permission_done(doc: u64, ids: Vec<u64>, granted: bool) {
    if let Some(key) = site_frame(doc) {
        send(doc, &key, Event::PermDone { ids, granted });
    }
}

/// Fill fields of the site-process page shown for `doc` (a picked autofill suggestion).
pub fn site_fill(doc: u64, fields: Vec<(usize, String)>) {
    if let Some(key) = site_frame(doc) {
        send(doc, &key, Event::Fill(fields.into_iter().map(|(n, v)| (n as u32, v)).collect()));
    }
}

/// Pass a picked <select> option to the site-process page shown for `doc`.
pub fn site_select(doc: u64, id: u32, index: u32) {
    if let Some(key) = site_frame(doc) {
        send(doc, &key, Event::SelectChoose { id, index });
    }
}

/// Back/Forward moved within the history entries of the site-process page shown for `doc`:
/// tell it (popstate, hashchange). False if the page isn't in a site process.
pub fn site_history_moved(doc: u64, url: &str, state: &str, popstate: bool) -> bool {
    let Some(key) = site_frame(doc) else { return false };
    send(doc, &key, Event::HistoryMoved { url: url.to_string(), state: state.to_string(), popstate });
    true
}

/// Changes whenever site_runs would give something else (new text, or scrolled).
pub fn runs_version(doc: u64) -> u64 {
    let Ok(live) = LIVE.lock() else { return 0 };
    live.iter()
        .find(|f| f.doc == Some(doc) && matches!(f.sink, Sink::Site(_)))
        .map(|f| f.runs_version.wrapping_mul(0x1_0000_0001) ^ f.scroll.to_bits() as u64)
        .unwrap_or(0)
}

/// Scroll the site-process page shown for `doc` so the window y `y` (from site_runs) comes into
/// a `view` px tall view. False if the page isn't in a site process.
pub fn site_reveal(doc: u64, lay: &layout::Layout, y: f32, view: f32) -> bool {
    let Ok(live) = LIVE.lock() else { return false };
    let Some((f, _, y0)) = site_box(&live, doc, lay) else { return false };
    let Sink::Site(id) = f.sink else { return false };
    let shown = y - y0;
    if shown < 20.0 || shown > view - 40.0 {
        let page_y = shown + f.scroll;
        crate::process::event(id, &Event::ScrollTo((page_y - view / 3.0).max(0.0)));
    }
    true
}

/// Keep site-process pages the size of their box (the window, less the toolbar).
pub fn fit(doc: u64, lay: &layout::Layout) {
    let Ok(mut live) = LIVE.lock() else { return };
    for f in live.iter_mut().filter(|f| f.doc == Some(doc)) {
        let Sink::Site(id) = f.sink else { continue };
        let size = lay.items.iter().find_map(|it| match it {
            crate::paint::Item::Image { w, h, src, .. } if *src == f.key => Some((w.round().max(1.0) as u32, h.round().max(1.0) as u32)),
            _ => None,
        });
        if let Some((w, h)) = size.filter(|&s| s != (f.w, f.h)) {
            (f.w, f.h) = (w, h);
            crate::process::event(id, &Event::Resize { w, h, scale: crate::zoom::device_scale() });
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
        /// The frame sites it runs: one, or more once the pool is full.
        sites: Vec<String>,
    }

    struct Route {
        /// Its process.
        key: u32,
        /// Its top-level site (what the process may ask the browser about for it).
        top: Option<String>,
        tx: Sender<Update>,
    }

    /// Frame processes, one per frame site, by a number of their own.
    static PROCS: Mutex<Option<HashMap<u32, Proc>>> = Mutex::new(None);
    static NEXT_PROC: AtomicU32 = AtomicU32::new(1);
    /// Each open frame.
    static ROUTES: Mutex<Option<HashMap<u32, Route>>> = Mutex::new(None);
    static NEXT_ID: AtomicU32 = AtomicU32::new(1);
    /// The process drawing a snapshot frame right now (0 = none), so a stuck one can be stopped.
    static RENDERING: AtomicU32 = AtomicU32::new(0);

    /// The most frame processes at once; past it, an idle one is ended or the least busy one
    /// takes the new site too. IRONSWIFT_FRAME_PROCESS_MAX overrides it.
    fn limit() -> usize {
        std::env::var("IRONSWIFT_FRAME_PROCESS_MAX").ok().and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(6).max(1)
    }

    /// What process `key` may ask the browser for: only the top-level sites of its own frames.
    fn auth(key: u32) -> crate::process::Auth {
        let mut tops: Vec<String> = ROUTES.lock().ok().and_then(|r| r.as_ref().map(|r| r.values().filter(|x| x.key == key).filter_map(|x| x.top.clone()).collect())).unwrap_or_default();
        tops.sort();
        tops.dedup();
        crate::process::Auth { sites: Vec::new(), tops, storage: false, extensions: false }
    }

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

    /// How many frames each process has open.
    fn counts() -> HashMap<u32, usize> {
        ROUTES.lock().ok().and_then(|r| r.as_ref().map(|r| r.values().fold(HashMap::new(), |mut m, x| {
            *m.entry(x.key).or_insert(0) += 1;
            m
        }))).unwrap_or_default()
    }

    /// The process for frames of `site`, started (with its reader thread) if need be.
    fn ensure(site: &str) -> Option<u32> {
        let counts = counts();
        let mut g = PROCS.lock().ok()?;
        let procs = g.get_or_insert_with(HashMap::new);
        procs.retain(|_, p| p.child.try_wait().ok().flatten().is_none());
        if let Some((k, _)) = procs.iter().find(|(_, p)| p.sites.iter().any(|s| s == site)) {
            return Some(*k);
        }
        if procs.len() >= limit() {
            // An idle process makes room; otherwise the least busy one takes this site too.
            let idle = procs.keys().find(|k| counts.get(k).copied().unwrap_or(0) == 0).copied();
            if let Some(k) = idle {
                if let Some(mut p) = procs.remove(&k) {
                    let _ = p.child.kill();
                    let _ = p.child.wait();
                }
            } else {
                let k = procs.keys().min_by_key(|k| (counts.get(k).copied().unwrap_or(0), **k)).copied()?;
                let p = procs.get_mut(&k)?;
                let mut m = Vec::new();
                put(&mut m, b"allow");
                put(&mut m, &0u32.to_le_bytes());
                put(&mut m, site.as_bytes());
                p.stdin.write_all(&m).and_then(|_| p.stdin.flush()).ok()?;
                p.sites.push(site.to_string());
                log!("[frames] frame processes full: {site} shares process {k} with {}", p.sites[..p.sites.len() - 1].join(", "));
                return Some(k);
            }
        }
        let exe = std::env::current_exe().ok()?;
        let mut child = std::process::Command::new(exe)
            .arg("--frame-renderer")
            .arg(site)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| log!("[frames] couldn't start the frame process for {site}: {e}"))
            .ok()?;
        if let Some(err) = child.stderr.take() {
            crate::process::relay_stderr(err, site);
        }
        let stdin = child.stdin.take()?;
        let mut stdout = child.stdout.take()?;
        let key = NEXT_PROC.fetch_add(1, Ordering::SeqCst);
        let in_flight = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::Builder::new()
            .name(format!("frames-{site}"))
            .spawn(move || {
                // Route each update to its frame until the process ends; answer its requests
                // (network, document.cookie) as process.rs does for site processes.
                loop {
                    let r = (|| -> std::io::Result<Option<(u32, Update)>> {
                        let kind = get(&mut stdout)?;
                        match kind.as_slice() {
                            b"upd" => {
                                let id = crate::process::get_u32(&mut stdout)?;
                                Ok(Some((id, Update::decode(&mut stdout)?)))
                            }
                            b"req" => {
                                let rid = crate::process::get_u32(&mut stdout)?;
                                let parts = crate::process::get_list(&mut stdout)?;
                                crate::process::serve_async(auth(key), parts, in_flight.clone(), move |answer| {
                                    write(key, &crate::process::response(rid, &answer));
                                });
                                Ok(None)
                            }
                            _ => Err(std::io::Error::other("unknown message")),
                        }
                    })();
                    match r {
                        Ok(None) => {}
                        Ok(Some((id, u))) => {
                            if let Ok(routes) = ROUTES.lock() {
                                // Only its own frames: a process can't speak for another's.
                                if let Some(x) = routes.as_ref().and_then(|r| r.get(&id)).filter(|x| x.key == key) {
                                    let _ = x.tx.send(u);
                                }
                            }
                        }
                        Err(e) => {
                            if e.kind() != std::io::ErrorKind::UnexpectedEof {
                                log!("[frames] frame process {key}: {e}");
                            }
                            // Its frames stop (they keep their last picture); the next frame
                            // of its sites starts a fresh process.
                            if let Ok(mut r) = ROUTES.lock() {
                                if let Some(r) = r.as_mut() {
                                    r.retain(|_, x| x.key != key);
                                }
                            }
                            let gone = PROCS.lock().ok().and_then(|mut g| g.as_mut()?.remove(&key));
                            if let Some(mut p) = gone {
                                let _ = p.child.kill();
                                let _ = p.child.wait();
                            }
                            let _ = RENDERING.compare_exchange(key, 0, Ordering::SeqCst, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            })
            .ok()?;
        log!("[frames] started frame process {key} for {site} (pid {})", child.id());
        procs.insert(key, Proc { child, stdin, sites: vec![site.to_string()] });
        Some(key)
    }

    fn write(key: u32, msg: &[u8]) -> bool {
        let Ok(mut g) = PROCS.lock() else { return false };
        let Some(p) = g.as_mut().and_then(|g| g.get_mut(&key)) else { return false };
        p.stdin.write_all(msg).and_then(|_| p.stdin.flush()).is_ok()
    }

    fn key_of(id: u32) -> Option<u32> {
        ROUTES.lock().ok()?.as_ref()?.get(&id).map(|x| x.key)
    }

    /// Open a frame in its site's process; its updates go to `updates`. None = no process.
    pub fn open(job: &FrameJob, updates: Sender<Update>) -> Option<u32> {
        let FrameSrc::Url(url) = &job.src else { return None };
        let key = ensure(&site(url)?)?;
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        ROUTES.lock().ok()?.get_or_insert_with(HashMap::new).insert(id, Route { key, top: crate::process::site_of(&job.top), tx: updates });
        let mut m = Vec::new();
        put(&mut m, b"open");
        put(&mut m, &id.to_le_bytes());
        m.extend(encode_job(job));
        if !write(key, &m) {
            close(id);
            return None;
        }
        Some(id)
    }

    pub fn event(id: u32, ev: &Event) {
        let Some(key) = key_of(id) else { return };
        let mut m = Vec::new();
        put(&mut m, b"event");
        put(&mut m, &id.to_le_bytes());
        m.extend(ev.encode());
        write(key, &m);
    }

    pub fn close(id: u32) {
        let key = ROUTES.lock().ok().and_then(|mut r| r.as_mut()?.remove(&id)).map(|x| x.key);
        let Some(key) = key else { return };
        let mut m = Vec::new();
        put(&mut m, b"close");
        put(&mut m, &id.to_le_bytes());
        write(key, &m);
    }

    /// Draw a frame once in its site's process (snapshots, frames that can't be live).
    /// None = no process is available (the caller draws it here instead).
    pub fn render(job: &FrameJob) -> Option<Option<image::RgbaImage>> {
        let (tx, rx) = channel();
        let id = open(job, tx)?;
        if let Some(k) = key_of(id) {
            RENDERING.store(k, Ordering::SeqCst);
        }
        let r = loop {
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Update::Picture { img, .. }) => break Some(img),
                Ok(_) => continue,
                Err(_) => break None,
            }
        };
        RENDERING.store(0, Ordering::SeqCst);
        close(id);
        Some(r)
    }

    /// Stop the frame process drawing a snapshot that takes too long (that site's next frame
    /// starts a fresh one; other sites' processes go on).
    pub fn stop_stuck() {
        let key = RENDERING.swap(0, Ordering::SeqCst);
        if key == 0 {
            return;
        }
        let pid = PROCS.lock().ok().and_then(|g| g.as_ref()?.get(&key).map(|p| p.child.id()));
        if let Some(pid) = pid {
            crate::platform::kill(pid);
        }
    }

    #[cfg(test)]
    #[test]
    fn each_frame_process_speaks_only_for_its_own_frames() {
        let (tx, _rx) = channel();
        {
            let mut r = ROUTES.lock().unwrap();
            let r = r.get_or_insert_with(HashMap::new);
            r.insert(9_000_001, Route { key: 901, top: Some("news.org".into()), tx: tx.clone() });
            r.insert(9_000_002, Route { key: 902, top: Some("bank.com".into()), tx: tx.clone() });
            r.insert(9_000_003, Route { key: 901, top: Some("shop.net".into()), tx });
        }
        let a = auth(901);
        assert_eq!(a.tops, ["news.org", "shop.net"]);
        assert!(!a.storage && !a.extensions);
        assert_eq!(auth(902).tops, ["bank.com"]);
        // Another process's frame can't be closed or reached through this one's key.
        assert_eq!(key_of(9_000_002), Some(902));
        for id in [9_000_001, 9_000_002, 9_000_003] {
            ROUTES.lock().unwrap().as_mut().unwrap().remove(&id);
        }
        assert!(auth(901).tops.is_empty());
    }

    /// Frame processes running right now (tests).
    #[allow(dead_code)]
    pub fn running() -> usize {
        PROCS.lock().map(|g| g.as_ref().map_or(0, |p| p.len())).unwrap_or(0)
    }

    /// `ironswift --frame-renderer`: run the frames the browser opens, one thread each, until
    /// stdin closes.
    pub fn serve(own: &str) {
        if own.is_empty() {
            log!("[frames] refusing to run without a site");
            return;
        }
        // The frame sites this process may run: its own, and any the browser adds when the
        // pool is full.
        let mut sites = vec![own.to_string()];
        if !sandbox::protect_frame_renderer() {
            log!("[frames] refusing to run without a sandbox");
            return;
        }
        if crate::load_fonts().is_none() {
            return;
        }
        js::use_memory_storage();
        // Requests and cookies go through the browser; this process can't read the jar.
        crate::process::become_renderer();
        let mut frames: HashMap<u32, Sender<Event>> = HashMap::new();
        let mut input = std::io::stdin().lock();
        loop {
            let r = (|| -> std::io::Result<()> {
                let kind = get(&mut input)?;
                let id = u32::from_le_bytes(get(&mut input)?.try_into().unwrap_or([0; 4]));
                match kind.as_slice() {
                    b"allow" => {
                        let s = String::from_utf8_lossy(&get(&mut input)?).to_string();
                        if !s.is_empty() && !sites.contains(&s) {
                            sites.push(s);
                        }
                    }
                    b"open" => {
                        let url = String::from_utf8_lossy(&get(&mut input)?).to_string();
                        let parent = String::from_utf8_lossy(&get(&mut input)?).to_string();
                        let top = String::from_utf8_lossy(&get(&mut input)?).to_string();
                        let sandbox = sandbox_from(&get(&mut input)?);
                        let w = u32::from_le_bytes(get(&mut input)?.try_into().unwrap_or([0; 4])).clamp(1, 2000);
                        let h = u32::from_le_bytes(get(&mut input)?.try_into().unwrap_or([0; 4])).clamp(1, 2000);
                        let (reply, _) = channel();
                        if !site(&url).is_some_and(|s| sites.contains(&s)) {
                            log!("[frames] {}: refused to open {url}", sites.join(", "));
                            return Ok(());
                        }
                        let job = FrameJob { src: FrameSrc::Url(url), w, h, parent, top, sandbox, reply, top_level: false };
                        let (etx, erx) = channel();
                        frames.insert(id, etx);
                        let _ = std::thread::Builder::new().name("frame".into()).stack_size(crate::MAIN_STACK).spawn(move || {
                            let send = |u: Update| {
                                let mut m = Vec::new();
                                put(&mut m, b"upd");
                                put(&mut m, &id.to_le_bytes());
                                m.extend(u.encode());
                                crate::process::send_raw(&m)
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
                    b"resp" => {
                        // `id` is the request number here.
                        let parts = crate::process::get_list(&mut input)?;
                        crate::process::deliver(id, parts);
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
            Event::ScrollTo(120.0),
            Event::SelectChoose { id: 42, index: 3 },
            Event::Ask { what: "run".into(), arg: "1 + 1".into() },
            Event::Fill(vec![(3, "john".into()), (5, "pä55".into())]),
            Event::PermDone { ids: vec![4, 9], granted: true },
            Event::A11y(true),
            Event::A11yAction { kind: 3, node: 9, value: "typed".into() },
            Event::HistoryMoved { url: "https://a.com/x#y".into(), state: "{\"n\":1}".into(), popstate: true },
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
        for op in [js::HistoryOp::Push("https://a.com/p".into(), "{}".into()), js::HistoryOp::Replace("https://a.com/r".into(), "null".into()), js::HistoryOp::Hash("https://a.com/#h".into(), true), js::HistoryOp::Go(-1)] {
            let b = Update::History(op.clone()).encode();
            assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::History(o) if o == op));
        }
        let b = Update::Select { id: 7, x: 1.0, y: 20.0, w: 120.0, options: vec!["Apple".into(), "Bänana".into()] }.encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::Select { id: 7, options, .. } if options == ["Apple", "Bänana"]));
        let b = Update::Answer { what: "run".into(), data: b"ok:2".to_vec() }.encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::Answer { what, data } if what == "run" && data == b"ok:2"));
        let info = crate::autofill::FieldInfo { kind: Some("email".into()), login: Some((Some(3), 5)), typed: "jo".into() };
        let b = Update::Field { id: 3, x: 1.0, y: 2.0, w: 220.0, info: info.clone() }.encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::Field { id: 3, info: i, .. } if i == info));
        let sub = crate::autofill::Submitted { entries: vec![("email".into(), "a@b.c".into())], login: Some(("john".into(), "pw".into())) };
        let b = Update::Submitted(sub.clone()).encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::Submitted(x) if x == sub));
        let b = Update::ContextMenu { link: Some("https://a.com/l".into()), image: None }.encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::ContextMenu { link: Some(l), image: None } if l == "https://a.com/l"));
        let runs = vec![crate::chrome::TextRun { x: 4.0, y: 20.0, text: "Find me".into(), font: 2, size: 16.0, fixed: true }];
        let b = Update::Text(runs.clone()).encode();
        assert!(matches!(Update::decode(&mut b.as_slice()).unwrap(), Update::Text(r) if r == runs));
        // Garbage is refused, not misread.
        assert!(Event::decode(&mut &b"\x03\0\0\0zzz"[..]).is_err());
    }

    #[test]
    fn a_site_process_page_sends_its_accessibility_tree_and_takes_screen_reader_actions() {
        let (etx, erx) = channel();
        let (utx, urx) = channel();
        let html = "<body><button onclick=\"document.title='pressed'\">Press me</button><a href='/x'>Go</a></body>".to_string();
        let job = FrameJob { src: FrameSrc::Doc { html, base: "about:blank".into() }, w: 400, h: 300, parent: "about:blank".into(), top: "about:blank".into(), sandbox: None, reply: channel().0, top_level: true };
        std::thread::spawn(move || run_host(job, erx, move |u| utx.send(u).is_ok()));
        let deadline = Instant::now() + Duration::from_secs(20);
        // Nothing is sent until a screen reader listens.
        etx.send(Event::A11y(true)).unwrap();
        let mut tree = None;
        while tree.is_none() && Instant::now() < deadline {
            if let Ok(Update::A11y(b)) = urx.recv_timeout(Duration::from_millis(200)) {
                tree = crate::a11y_remote::decode(&b);
            }
        }
        let tree = tree.expect("no tree from the page");
        let button = tree.tree.nodes.iter().position(|n| n.role == "button" && n.name == "Press me").expect("the button");
        assert!(tree.rects[button].is_some(), "where it's drawn");
        assert!(tree.tree.nodes.iter().any(|n| n.role == "link" && n.name == "Go"));
        etx.send(Event::A11yAction { kind: 1, node: tree.tree.nodes[button].dom as u32, value: String::new() }).unwrap();
        let mut pressed = false;
        while !pressed && Instant::now() < deadline {
            if let Ok(Update::Title(t)) = urx.recv_timeout(Duration::from_millis(200)) {
                pressed = t == "pressed";
            }
        }
        assert!(pressed, "the screen reader's click pressed the button");
    }

    #[test]
    fn site_process_pages_ask_the_window_for_permissions() {
        // SAFETY: a scratch profile, so no decision is saved in yours.
        unsafe {
            if std::env::var_os("IRONSWIFT_PROFILE").is_none() {
                std::env::set_var("IRONSWIFT_PROFILE", std::env::temp_dir().join(format!("ironswift-jsapi-{}", std::process::id())));
            }
        }
        crate::permissions::forget("http://127.0.0.1:9");
        let html = "<script>Notification.requestPermission().then(r => document.title = 'p:' + r)</script>".to_string();
        let run = |top_level: bool| {
            let (etx, erx) = channel();
            let (utx, urx) = channel();
            let base = "http://127.0.0.1:9/".to_string();
            let job = FrameJob { src: FrameSrc::Doc { html: html.clone(), base: base.clone() }, w: 300, h: 200, parent: base.clone(), top: base, sandbox: None, reply: channel().0, top_level };
            std::thread::spawn(move || run_host(job, erx, move |u| utx.send(u).is_ok()));
            (etx, urx)
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        // A top-level page's request reaches the window, and its answer reaches the page.
        let (etx, urx) = run(true);
        let mut asked = None;
        while asked.is_none() && Instant::now() < deadline {
            if let Ok(Update::Permission { id, kind, .. }) = urx.recv_timeout(Duration::from_millis(200)) {
                assert_eq!(kind, crate::permissions::Kind::Notifications);
                asked = Some(id);
            }
        }
        let id = asked.expect("the request reached the window");
        etx.send(Event::PermDone { ids: vec![id], granted: true }).unwrap();
        let mut title = String::new();
        while title.is_empty() && Instant::now() < deadline {
            if let Ok(Update::Title(t)) = urx.recv_timeout(Duration::from_millis(200)) {
                title = t;
            }
        }
        assert_eq!(title, "p:granted");
    }

    #[test]
    fn a_site_process_page_sends_its_text_and_scrolls_on_request() {
        // The window finds and selects text on pages in site processes from what they send.
        let (etx, erx) = channel();
        let (utx, urx) = channel();
        let html = format!("<body style='margin:0'><div style='height:2000px'>Top line</div><p>Far down</p></body>");
        let job = FrameJob { src: FrameSrc::Doc { html, base: "about:blank".into() }, w: 400, h: 300, parent: "about:blank".into(), top: "about:blank".into(), sandbox: None, reply: channel().0, top_level: true };
        std::thread::spawn(move || run_host(job, erx, move |u| utx.send(u).is_ok()));
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut runs = None;
        while runs.is_none() && Instant::now() < deadline {
            if let Ok(Update::Text(r)) = urx.recv_timeout(Duration::from_millis(200)) {
                runs = Some(r);
            }
        }
        let runs = runs.expect("no text from the page");
        let far = runs.iter().find(|r| r.text.contains("Far down")).expect("the far line");
        assert!(far.y > 2000.0, "page coordinates: {}", far.y);
        etx.send(Event::ScrollTo(1900.0)).unwrap();
        let mut scrolled = false;
        while !scrolled && Instant::now() < deadline {
            if let Ok(Update::Picture { scroll, .. }) = urx.recv_timeout(Duration::from_millis(200)) {
                scrolled = scroll > 1000.0;
            }
        }
        assert!(scrolled, "the page scrolled to the match");
    }

    #[test]
    fn the_window_enforces_iframe_sandboxes() {
        let live = |sandbox: Option<security::Sandbox>, top_level: bool| Live {
            key: "f".into(),
            owner: "https://a.com/".into(),
            doc: None,
            born: Instant::now(),
            seen: Instant::now(),
            w: 1,
            h: 1,
            sink: Sink::Thread(channel().0),
            updates: channel().1,
            scroll: 0.0,
            max: 0.0,
            inner: false,
            sandbox,
            top_level,
            runs: Vec::new(),
            runs_version: 0,
            held: Vec::new(),
            a11y: None,
            a11y_version: 0,
            a11y_on: false,
        };
        let locked = security::Sandbox { scripts: true, same_origin: false, forms: false, popups: false, top_navigation: false, modals: false };
        assert!(!may_navigate_top(&live(Some(locked.clone()), false)), "a sandboxed frame can't move the tab");
        assert!(may_navigate_top(&live(Some(security::Sandbox { top_navigation: true, ..locked }), false)));
        assert!(may_navigate_top(&live(None, false)), "unsandboxed frames may (as in other browsers)");
        assert!(may_navigate_top(&live(None, true)), "a site process's own top-level page");
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
