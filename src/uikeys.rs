//! Keyboard control of the browser's own UI.
//!
//! - F6 cycles: page → address bar → toolbar → page. In the toolbar, Left/Right (or Tab /
//!   Shift+Tab) move between the permission prompt's buttons, back, forward, reload, the
//!   shield, the downloads chip, the bookmark star, the bookmarks and the tabs; Enter or Space presses the
//!   one with the focus ring; Escape goes back to the page.
//! - Shift+F10 or the Menu key opens the right-click menu for the focused element (or the
//!   page); Up/Down/Home/End move in an open menu or downloads panel, Enter or Space picks.
//!
//! Pressing something is a click at its middle, so the mouse code does the work either way.

use crate::paint::Canvas;
use crate::style::Color;

/// A target in the toolbar: its box in left-to-right UI coordinates (x, y, w, h).
pub type Target = (f32, f32, f32, f32);

/// Move through `n` targets, wrapping; `forward` is the reading direction's "next".
pub fn step(cur: Option<usize>, n: usize, forward: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match cur {
        None => if forward { 0 } else { n - 1 },
        Some(i) => if forward { (i + 1) % n } else { (i + n - 1) % n },
    })
}

/// Where to click to press a target (window coordinates, with the UI mirrored right to left).
pub fn click_point(t: Target, window_w: f32) -> (f32, f32) {
    let x = t.0 + t.2 / 2.0;
    (if crate::rtl::on() { window_w - x } else { x }, t.1 + t.3 / 2.0)
}

/// The next row that does something in a menu (skipping separators and disabled rows).
pub fn menu_step(actionable: &[bool], cur: Option<usize>, delta: i32) -> Option<usize> {
    let n = actionable.len() as i32;
    if n == 0 || !actionable.iter().any(|&a| a) {
        return None;
    }
    let mut i = match cur {
        Some(c) => c as i32,
        None => if delta > 0 { -1 } else { n },
    };
    for _ in 0..n {
        i = (i + delta).rem_euclid(n);
        if actionable[i as usize] {
            return Some(i as usize);
        }
    }
    cur
}

/// The focus ring around a toolbar target (drawn mirrored right to left).
pub fn draw_ring(canvas: &mut Canvas, t: Target) {
    crate::rtl::begin(canvas);
    let c = Color::rgb(26, 115, 232);
    let (x, y, w, h) = (t.0 - 1.0, t.1 - 1.0, t.2 + 2.0, t.3 + 2.0);
    canvas.fill_rect(x, y, w, 2.0, c);
    canvas.fill_rect(x, y + h - 2.0, w, 2.0, c);
    canvas.fill_rect(x, y, 2.0, h, c);
    canvas.fill_rect(x + w - 2.0, y, 2.0, h, c);
    crate::rtl::end(canvas);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_wraps() {
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(None, 3, false), Some(2));
        assert_eq!(step(Some(2), 3, true), Some(0));
        assert_eq!(step(Some(0), 3, false), Some(2));
        assert_eq!(step(Some(0), 0, true), None);
    }

    #[test]
    fn menus_skip_separators_and_disabled_rows() {
        let rows = [false, true, false, true, true];
        assert_eq!(menu_step(&rows, None, 1), Some(1));
        assert_eq!(menu_step(&rows, Some(1), 1), Some(3));
        assert_eq!(menu_step(&rows, Some(4), 1), Some(1));
        assert_eq!(menu_step(&rows, None, -1), Some(4));
        assert_eq!(menu_step(&rows, Some(1), -1), Some(4));
        assert_eq!(menu_step(&[false, false], None, 1), None);
    }
}
