//! Right-to-left browser UI (Arabic, Hebrew…): the tab strip, toolbar, bookmarks bar, panels
//! and menus are laid out mirrored. Drawing code keeps working in left-to-right coordinates;
//! `Canvas::mirror` flips what it draws, and the window loop flips the mouse's x before testing
//! what it is over. Text keeps its own reading order. Web pages are never mirrored: their
//! direction is their own (`dir`, `direction`).

use crate::paint::Canvas;

/// Whether the browser UI is mirrored: the UI language reads right to left, or
/// IRONSWIFT_RTL=1 / 0 forces it (for testing).
pub fn on() -> bool {
    match std::env::var("IRONSWIFT_RTL").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => crate::i18n::rtl(),
    }
}

/// Mirror the whole window while drawing browser UI (no-op left to right).
pub fn begin(canvas: &mut Canvas) {
    canvas.mirror = on().then_some((0.0, canvas.w as f32));
}

/// Mirror within a panel's own box (a menu that opens where it was asked to).
pub fn begin_in(canvas: &mut Canvas, x: f32, w: f32) {
    canvas.mirror = on().then_some((x, x + w));
}

pub fn end(canvas: &mut Canvas) {
    canvas.mirror = None;
}

/// The x a mouse position has in left-to-right UI coordinates.
pub fn ui_x(x: f32, window_w: f32) -> f32 {
    if on() { window_w - x } else { x }
}

/// Back and forward point the way the reading goes.
pub fn nav_label<'a>(i: usize, label: &'a str) -> &'a str {
    if !on() {
        return label;
    }
    match i {
        0 => "→",
        1 => "←",
        _ => label,
    }
}

/// "»" (more bookmarks) and similar arrows, turned for right to left.
pub fn more_label() -> &'static str {
    if on() { "«" } else { "»" }
}

/// For a text field drawn mirrored (its text right-aligned but read left to right): the
/// left-to-right x where the boundary `before` (the width of the text before it) falls,
/// given where the text starts (`tx`) and its full width.
pub fn field_x(tx: f32, before: f32, full: f32) -> f32 {
    if on() { tx + full - before } else { tx + before }
}

/// The reverse: how far into the text (from its start) a mouse x in left-to-right UI
/// coordinates is.
pub fn field_offset(tx: f32, mx: f32, full: f32) -> f32 {
    if on() { tx + full - mx } else { mx - tx }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Color;

    #[test]
    fn mirrored_canvas_flips_rectangles() {
        let mut c = Canvas::new(10, 1);
        c.clear(Color::rgb(0, 0, 0));
        c.mirror = Some((0.0, 10.0));
        c.fill_rect(0.0, 0.0, 3.0, 1.0, Color::rgb(255, 255, 255));
        c.mirror = None;
        assert_eq!(&c.buf, &[0, 0, 0, 0, 0, 0, 0, 0xffffff, 0xffffff, 0xffffff]);
        // Within a box from x=2 to x=6.
        let mut c = Canvas::new(10, 1);
        c.clear(Color::rgb(0, 0, 0));
        c.mirror = Some((2.0, 6.0));
        c.fill_rect(2.0, 0.0, 1.0, 1.0, Color::rgb(255, 255, 255));
        assert_eq!(c.buf[5], 0xffffff);
    }

    #[test]
    fn text_field_positions_round_trip() {
        // Not mirrored here (no RTL language in tests): plain offsets.
        if !on() {
            assert_eq!(field_x(100.0, 20.0, 80.0), 120.0);
            assert_eq!(field_offset(100.0, 120.0, 80.0), 20.0);
        }
    }
}
