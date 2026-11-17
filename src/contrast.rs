//! High contrast (Settings → Appearance → High contrast): pages in forced colors, as
//! Windows' and Firefox's high-contrast modes do (CSS `forced-colors: active`). Text is
//! light on black, links yellow, borders keep their shape in the text color; background
//! images, shadows and filters go. The browser's own controls use `PALETTE`.

use crate::dom::{Dom, NodeId};
use crate::style::{Color, Style};
use std::collections::HashMap;
use std::rc::Rc;

pub const CANVAS: Color = Color { r: 0, g: 0, b: 0, a: 255 };
pub const TEXT: Color = Color { r: 255, g: 255, b: 255, a: 255 };
pub const LINK: Color = Color { r: 255, g: 255, b: 0, a: 255 };
/// Buttons and form fields: a dark field with a cyan edge.
pub const BUTTON_FACE: Color = Color { r: 0, g: 0, b: 0, a: 255 };
pub const BUTTON_TEXT: Color = Color { r: 0, g: 255, b: 255, a: 255 };
pub const HIGHLIGHT: Color = Color { r: 26, g: 235, b: 255, a: 255 };

/// Whether the high-contrast setting is on.
pub fn on() -> bool {
    crate::settings::get().high_contrast
}

/// Is `id` a link or inside one?
fn in_link(dom: &Dom, id: NodeId) -> bool {
    let mut cur = Some(id);
    while let Some(n) = cur {
        if matches!(dom.tag(n), Some("a" | "area")) && dom.attr(n, "href").is_some() {
            return true;
        }
        cur = dom.nodes[n].parent;
    }
    false
}

fn is_control(dom: &Dom, id: NodeId) -> bool {
    matches!(dom.tag(id), Some("button" | "input" | "select" | "textarea"))
}

/// Replace every node's colors with the forced palette. Styles shared between nodes stay
/// shared (one new style per old style and kind of node).
pub fn force_colors(dom: &Dom, styles: &mut [Rc<Style>]) {
    let mut done: HashMap<(*const Style, u8), Rc<Style>> = HashMap::new();
    for id in 0..styles.len().min(dom.nodes.len()) {
        let kind = if is_control(dom, id) {
            2
        } else if in_link(dom, id) {
            1
        } else if matches!(dom.tag(id), Some("html")) {
            3
        } else {
            0
        };
        let key = (Rc::as_ptr(&styles[id]), kind);
        if let Some(s) = done.get(&key) {
            styles[id] = s.clone();
            continue;
        }
        let mut st = (*styles[id]).clone();
        let fg = match kind {
            1 => LINK,
            2 => BUTTON_TEXT,
            _ => TEXT,
        };
        st.color = fg;
        st.deco_color = None;
        st.border_color = fg;
        st.border_colors = None;
        // Only boxes that had a background keep one (in the canvas color), so text over an
        // image or a colored band still sits on black.
        let had_bg = st.background.a > 0 || st.bg.color.a > 0 || st.bg.image.is_some();
        let bg = if kind == 2 { BUTTON_FACE } else { CANVAS };
        st.background = if had_bg || kind == 3 { bg } else { Color::TRANSPARENT };
        st.bg.color = st.background;
        // Images that are content (<img>) stay; decorative backgrounds go, as in other
        // browsers' forced-colors mode.
        st.bg.image = None;
        st.shadow = None;
        st.text_shadow = None;
        let st = Rc::new(st);
        done.insert(key, st.clone());
        styles[id] = st;
    }
}

/// One pixel (0RGB) of the browser's own UI in the high-contrast palette: light surfaces
/// become black and dark text white (by inverting brightness), and colored accents
/// (selection, the loading dot, a private window's purple) become yellow.
pub fn recolor_px(p: u32) -> u32 {
    let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max - min > 60 {
        // An accent: yellow, as bright as the accent is strong.
        let k = ((max - min) * 255 / max.max(1)).min(255);
        let y = 255 * k / 255;
        return (y << 16) | (y << 8);
    }
    let lum = (r * 30 + g * 59 + b * 11) / 100;
    let v = 255 - lum.min(255);
    // Push mid greys apart so borders and text stay crisp.
    let v = ((v as i32 - 128) * 3 / 2 + 128).clamp(0, 255) as u32;
    (v << 16) | (v << 8) | v
}

/// Recolor a rectangle of the canvas (the toolbars).
pub fn recolor_rect(canvas: &mut crate::paint::Canvas, x0: usize, y0: usize, x1: usize, y1: usize) {
    let w = canvas.w;
    for y in y0..y1.min(canvas.h) {
        for x in x0..x1.min(w) {
            let i = y * w + x;
            canvas.buf[i] = recolor_px(canvas.buf[i]);
        }
    }
}

/// Recolor every pixel that changed since `before` (menus, panels, prompts and notes drawn
/// over the page after it).
pub fn recolor_changed(canvas: &mut crate::paint::Canvas, before: &[u32]) {
    if before.len() != canvas.buf.len() {
        return;
    }
    for (p, &b) in canvas.buf.iter_mut().zip(before) {
        if *p != b {
            *p = recolor_px(*p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forces_text_link_and_background_colors() {
        let dom = crate::html::parse(r#"<body style="background:#fff;color:#333"><p style="background:url(x.png) #eee">Hi <a href="/">link</a></p><span>plain</span><button>Go</button></body>"#);
        let mut styles = crate::style::compute(&dom, &[]);
        force_colors(&dom, &mut styles);
        let p = dom.find_all("p")[0];
        let a = dom.find_all("a")[0];
        let span = dom.find_all("span")[0];
        let button = dom.find_all("button")[0];
        let body = dom.find_all("body")[0];
        assert_eq!(styles[p].color, TEXT);
        assert_eq!(styles[p].background, CANVAS);
        assert!(styles[p].bg.image.is_none());
        assert_eq!(styles[a].color, LINK);
        assert_eq!(styles[span].background, Color::TRANSPARENT);
        assert_eq!(styles[button].color, BUTTON_TEXT);
        assert_eq!(styles[body].background, CANVAS);
    }

    #[test]
    fn ui_palette() {
        // Light toolbar → black, dark text → white, blue accent → yellow.
        assert_eq!(recolor_px(0xececf0), 0x000000);
        assert_eq!(recolor_px(0x1e1e28), 0xffffff);
        let a = recolor_px(0x1a73e8);
        assert_eq!(a & 0xff, 0);
        assert!((a >> 16) > 200 && ((a >> 8) & 255) > 200);
    }
}
