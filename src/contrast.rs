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
}
