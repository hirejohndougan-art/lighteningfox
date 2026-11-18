//! Site permissions: camera, microphone, location and notifications. A site has to ask, you
//! answer in a prompt under the address bar, and the answer is remembered per site
//! (`<profile>/permissions.tsv`) until you reset it on about:settings.
//!
//! Only top-level pages on secure connections (https, or your own computer) can ask; frames
//! and plain-http pages are refused without a prompt, as in other browsers.

use crate::chrome;
use crate::font::{self, Fonts};
use crate::paint::Canvas;
use crate::style::Color;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Camera,
    Microphone,
    Location,
    Notifications,
    /// Reading what you copied (navigator.clipboard.readText).
    ClipboardRead,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "camera" | "video" => Kind::Camera,
            "microphone" | "audio" => Kind::Microphone,
            "geolocation" | "location" => Kind::Location,
            "notifications" | "notification" => Kind::Notifications,
            "clipboard-read" => Kind::ClipboardRead,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            Kind::Camera => "camera",
            Kind::Microphone => "microphone",
            Kind::Location => "geolocation",
            Kind::Notifications => "notifications",
            Kind::ClipboardRead => "clipboard-read",
        }
    }

    fn asks(self) -> &'static str {
        match self {
            Kind::Camera => "wants to use your camera",
            Kind::Microphone => "wants to use your microphone",
            Kind::Location => "wants to know your location",
            Kind::Notifications => "wants to show notifications",
            Kind::ClipboardRead => "wants to see what you copied",
        }
    }
}

/// (origin, permission, allowed)
static DECISIONS: Mutex<Option<Vec<(String, String, bool)>>> = Mutex::new(None);

fn file() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("permissions.tsv"))
}

fn with<R>(f: impl FnOnce(&mut Vec<(String, String, bool)>) -> R) -> R {
    let mut g = DECISIONS.lock().unwrap_or_else(|e| e.into_inner());
    let list = g.get_or_insert_with(|| {
        let text = file().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let mut p = l.split('\t');
                Some((p.next()?.to_string(), p.next()?.to_string(), p.next()? == "allow"))
            })
            .collect()
    });
    f(list)
}

fn save(list: &[(String, String, bool)]) {
    let Some(path) = file() else { return };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let body: String = list.iter().map(|(o, k, a)| format!("{o}\t{k}\t{}\n", if *a { "allow" } else { "block" })).collect();
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, body).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// A page that may ask at all: https (or your own computer), and not inside a frame.
pub fn can_ask(url: &str, top: &str) -> bool {
    if url != top {
        return false;
    }
    match url::Url::parse(url) {
        Ok(u) if u.scheme() == "https" => true,
        Ok(u) if u.scheme() == "http" => matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        Ok(u) => u.scheme() == "file",
        Err(_) => false,
    }
}

pub fn origin_key(url: &str) -> String {
    if url.starts_with("file:") { "file".into() } else { crate::net::origin_of(url) }
}

/// "granted", "denied" or "prompt" (not decided yet).
pub fn state(url: &str, top: &str, kind: Kind) -> &'static str {
    if !can_ask(url, top) {
        return "denied";
    }
    let origin = origin_key(url);
    match with(|l| l.iter().find(|(o, k, _)| *o == origin && k == kind.key()).map(|e| e.2)) {
        Some(true) => "granted",
        Some(false) => "denied",
        None => "prompt",
    }
}

pub fn set(url: &str, kind: Kind, allow: bool) {
    let origin = origin_key(url);
    // A private window remembers nothing.
    let private = std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1");
    with(|l| {
        l.retain(|(o, k, _)| !(*o == origin && k == kind.key()));
        l.push((origin, kind.key().to_string(), allow));
        if !private {
            save(l);
        }
    });
}

/// (In a site process) a decision the browser made after this process read the file: kept in
/// memory only (the process can't write the profile).
pub fn learned(origin: &str, kind: Kind, allow: bool) {
    with(|l| {
        l.retain(|(o, k, _)| !(o == origin && k == kind.key()));
        l.push((origin.to_string(), kind.key().to_string(), allow));
    });
}

/// (In a site process) the browser reset a site's decisions.
pub fn forgotten(origin: &str) {
    with(|l| l.retain(|(o, _, _)| o != origin));
}

/// Reset every decision for a site (an origin like https://example.com).
pub fn forget(origin: &str) {
    with(|l| {
        l.retain(|(o, _, _)| o != origin);
        save(l);
    });
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The remembered decisions, for about:settings.
pub fn settings_html() -> String {
    let list = with(|l| l.clone());
    if list.is_empty() {
        return "No site has been given or refused a permission yet.".into();
    }
    let mut origins: Vec<&String> = list.iter().map(|(o, _, _)| o).collect();
    origins.dedup();
    origins.sort();
    origins.dedup();
    origins
        .iter()
        .map(|o| {
            let what: Vec<String> = list.iter().filter(|(x, _, _)| x == *o).map(|(_, k, a)| format!("{k} {}", if *a { "allowed" } else { "blocked" })).collect();
            let enc: String = o.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
            format!("<br>{}: {} (<a href=\"about:settings?forget_permission={enc}\">reset</a>)", esc(o), esc(&what.join(", ")))
        })
        .collect()
}

/// Show a desktop notification (for `new Notification(…)` from a site you allowed).
pub fn notify(origin: &str, title: &str, body: &str) {
    let title: String = title.chars().take(120).collect();
    let body: String = body.chars().take(400).collect();
    let host = url::Url::parse(origin).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    let shown = if host.is_empty() { body } else { format!("{body}\n— {host}") };
    let ok = crate::platform::notify(&title, &shown) || std::process::Command::new("notify-send").args(["--app-name=Ironswift", "--", &title, &shown]).spawn().is_ok();
    if !ok {
        log!("[notification] {title}: {shown}");
    }
}

// ---------------------------------------------------------------- the prompt

pub const PROMPT_W: f32 = 400.0;
pub const PROMPT_H: f32 = 84.0;
const BTN_W: f32 = 76.0;

/// A site's question waiting for your answer.
pub struct Prompt {
    pub origin: String,
    pub kind: Kind,
    /// The script requests waiting on it (several calls get one answer).
    pub ids: Vec<u64>,
    /// The page that asked (the prompt goes away when it does).
    pub doc: u64,
    /// The page runs in a site process: the answer goes there (frames::site_permission_done).
    pub remote: bool,
    pub x: f32,
    pub y: f32,
}

impl Prompt {
    fn buttons(&self) -> [(f32, f32, bool); 2] {
        let by = self.y + PROMPT_H - 36.0;
        let right = self.x + PROMPT_W - 14.0;
        [(right - BTN_W, by, true), (right - 2.0 * BTN_W - 10.0, by, false)]
    }

    /// The Allow and Block buttons as (x, y, w, h, allow), Block first (for keyboard focus).
    pub fn button_boxes(&self) -> Vec<(f32, f32, f32, f32, bool)> {
        let mut b: Vec<_> = self.buttons().iter().map(|&(x, y, allow)| (x, y, BTN_W, 26.0, allow)).collect();
        b.reverse();
        b
    }

    pub fn contains(&self, mx: f32, my: f32) -> bool {
        mx >= self.x && mx < self.x + PROMPT_W && my >= self.y && my < self.y + PROMPT_H
    }

    /// Some(true) = Allow, Some(false) = Block.
    pub fn button_at(&self, mx: f32, my: f32) -> Option<bool> {
        self.buttons().iter().find(|(bx, by, _)| mx >= *bx && mx < bx + BTN_W && my >= *by && my < by + 26.0).map(|b| b.2)
    }

    pub fn draw(&self, canvas: &mut Canvas, fonts: &Fonts) {
        let (x, y) = (self.x, self.y);
        let line = Color::rgb(200, 200, 208);
        canvas.fill_rect(x + 2.0, y + 3.0, PROMPT_W, PROMPT_H, Color { r: 0, g: 0, b: 0, a: 30 });
        canvas.fill_rect(x, y, PROMPT_W, PROMPT_H, Color::rgb(255, 255, 255));
        canvas.fill_rect(x, y, PROMPT_W, 1.0, line);
        canvas.fill_rect(x, y + PROMPT_H - 1.0, PROMPT_W, 1.0, line);
        canvas.fill_rect(x, y, 1.0, PROMPT_H, line);
        canvas.fill_rect(x + PROMPT_W - 1.0, y, 1.0, PROMPT_H, line);
        let host = url::Url::parse(&self.origin).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_else(|| "This file".into());
        let text = chrome::fit(fonts, &format!("{host} {}", self.kind.asks()), 13.5, PROMPT_W - 28.0);
        canvas.draw_text(fonts, x + 14.0, y + 24.0, &text, font::BOLD, 13.5, Color::rgb(30, 30, 40));
        for (bx, by, allow) in self.buttons() {
            let (bg, fg) = if allow { (Color::rgb(79, 70, 229), Color::rgb(255, 255, 255)) } else { (Color::rgb(238, 238, 242), Color::rgb(40, 40, 50)) };
            canvas.fill_rect(bx, by, BTN_W, 26.0, bg);
            let label = if allow { "Allow" } else { "Block" };
            let tw = fonts.width(font::REGULAR, label, 13.0);
            canvas.draw_text(fonts, bx + (BTN_W - tw) / 2.0, by + 18.0, label, font::REGULAR, 13.0, fg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_may_ask() {
        assert!(can_ask("https://maps.example/", "https://maps.example/"));
        assert!(can_ask("http://localhost:8000/", "http://localhost:8000/"));
        assert!(!can_ask("http://insecure.example/", "http://insecure.example/"));
        assert!(!can_ask("https://ads.example/frame", "https://news.example/"), "frames can't ask");
        assert_eq!(state("http://insecure.example/", "http://insecure.example/", Kind::Location), "denied");
        assert_eq!(Kind::parse("geolocation"), Some(Kind::Location));
        assert_eq!(Kind::parse("midi"), None);
    }
}
