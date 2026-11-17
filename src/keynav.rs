//! Keyboard navigation of pages: where the focused element is, scrolling it into view, and the
//! focus ring drawn around it. The window loop (main.rs) moves focus with Tab / Shift+Tab
//! (in `a11y::tab_order`) and turns Enter / Space on a focused element into a click.

use crate::dom::{Dom, NodeId};
use crate::layout::{Layout, Rect};
use crate::paint::Canvas;
use crate::style::Color;

/// Where an element is drawn (page coordinates), and whether it's in a fixed box. Inline
/// elements such as links are found by their clickable areas; others by their box.
pub fn element_rect(lay: &Layout, d: &Dom, n: NodeId) -> Option<(Rect, bool)> {
    let parts: Vec<usize> = (0..lay.hits.len()).filter(|&i| lay.hits[i].1 == n || d.is_inside(lay.hits[i].1, n)).collect();
    if let Some(&first) = parts.first() {
        // The union of the fragments on the first line the element starts on.
        let r0 = lay.hits[first].0;
        let mut r = r0;
        for &i in &parts[1..] {
            let h = lay.hits[i].0;
            if (h.y - r0.y).abs() < 2.0 {
                let x2 = (r.x + r.w).max(h.x + h.w);
                r.x = r.x.min(h.x);
                r.w = x2 - r.x;
                r.h = r.h.max(h.h);
            }
        }
        if r.w > 0.0 && r.h > 0.0 {
            return Some((r, lay.is_fixed(first, false)));
        }
    }
    lay.boxes.get(&n).copied().filter(|(r, _)| r.w > 0.0 || r.h > 0.0)
}

/// The new scroll position that brings a rectangle into a `view_h`-tall view, if it's not
/// already fully visible (it lands a third of the way down, like other browsers).
pub fn scroll_into_view(r: Rect, fixed: bool, scroll: f32, view_h: f32, max_scroll: f32) -> Option<f32> {
    if fixed || (r.y >= scroll && r.y + r.h <= scroll + view_h) {
        return None;
    }
    let target = if r.h > view_h * 0.6 { r.y - 16.0 } else { r.y - view_h / 3.0 };
    Some(target.clamp(0.0, max_scroll.max(0.0)))
}

/// Window coordinates of the middle of a rectangle (for a keyboard "click").
pub fn center_on_screen(r: Rect, fixed: bool, top_bar: f32, scroll: f32) -> (f32, f32) {
    let y = r.y + r.h / 2.0 + top_bar - if fixed { 0.0 } else { scroll };
    (r.x + r.w / 2.0, y)
}

/// The focus ring: a 2 px blue outline with a thin white edge, 2 px outside the element, clipped
/// to the page area between `top_bar` and `bottom`.
pub fn draw_ring(canvas: &mut Canvas, r: Rect, fixed: bool, top_bar: f32, scroll: f32, bottom: f32, high_contrast: bool) {
    let y = r.y + top_bar - if fixed { 0.0 } else { scroll };
    let (x0, y0, x1, y1) = (r.x - 3.0, y - 3.0, r.x + r.w + 3.0, y + r.h + 3.0);
    let ring = if high_contrast { Color::rgb(255, 255, 0) } else { Color::rgb(26, 115, 232) };
    let edge = if high_contrast { Color::rgb(0, 0, 0) } else { Color::rgb(255, 255, 255) };
    let mut rect = |x: f32, y: f32, w: f32, h: f32, c: Color| {
        let (top, bot) = (y.max(top_bar), (y + h).min(bottom));
        if bot > top {
            canvas.fill_rect(x, top, w, bot - top, c);
        }
    };
    for (inset, width, c) in [(0.0, 1.0, edge), (1.0, 2.0, ring)] {
        let (ax, ay, bx, by) = (x0 + inset, y0 + inset, x1 - inset, y1 - inset);
        rect(ax, ay, bx - ax, width, c);
        rect(ax, by - width, bx - ax, width, c);
        rect(ax, ay, width, by - ay, c);
        rect(bx - width, ay, width, by - ay, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(y: f32, h: f32) -> Rect {
        Rect { x: 10.0, y, w: 50.0, h }
    }

    #[test]
    fn scrolls_only_when_needed() {
        assert_eq!(scroll_into_view(rect(100.0, 20.0), false, 0.0, 600.0, 5000.0), None);
        assert_eq!(scroll_into_view(rect(1000.0, 20.0), false, 0.0, 600.0, 5000.0), Some(800.0));
        assert_eq!(scroll_into_view(rect(50.0, 20.0), false, 400.0, 600.0, 5000.0), Some(0.0));
        assert_eq!(scroll_into_view(rect(1000.0, 20.0), true, 0.0, 600.0, 5000.0), None);
    }

    #[test]
    fn ring_stays_in_the_page_area() {
        let mut c = Canvas::new(100, 100);
        c.clear(Color::rgb(0, 0, 0));
        draw_ring(&mut c, Rect { x: 20.0, y: 0.0, w: 40.0, h: 20.0 }, false, 30.0, 10.0, 100.0, false);
        // Nothing drawn above the page area (y < 30).
        assert!(c.buf[..30 * 100].iter().all(|&p| p == 0));
        assert!(c.buf.iter().any(|&p| p != 0));
    }
}
