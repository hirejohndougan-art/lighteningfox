//! The `clip` property (CSS 2.1 §11.1.2): `rect(top, right, bottom, left)` cuts an absolutely
//! positioned box down to a rectangle measured from its border box's top-left corner; `auto`
//! is that edge of the border box. Deprecated in favour of `clip-path`, but the screen-reader
//! "visually hidden" pattern still everywhere uses it (`clip: rect(0 0 0 0)` with a 1px box
//! and `overflow: hidden`): without it NYT's "Skip to content" link showed its first letters.

use crate::style::{parse_len, Len};

/// The four offsets in px (top, right, bottom, left); None is `auto`.
pub type ClipRect = [Option<f32>; 4];

/// Parse a `clip` value: Some(rect) for `rect(…)`, None for `auto` or anything invalid.
pub fn parse(value: &str, font_size: f32) -> Option<ClipRect> {
    let v = value.trim().to_ascii_lowercase();
    let inner = v.strip_prefix("rect(")?.strip_suffix(')')?;
    // Both the comma form and the older space-separated form are allowed.
    let parts: Vec<&str> = if inner.contains(',') { inner.split(',').map(str::trim).collect() } else { inner.split_whitespace().collect() };
    if parts.len() != 4 {
        return None;
    }
    let mut out = [None; 4];
    for (slot, p) in out.iter_mut().zip(parts) {
        *slot = match parse_len(p, font_size)? {
            Len::Auto => None,
            Len::Px(px) => Some(px),
            // Percentages aren't allowed in rect().
            _ => return None,
        };
    }
    Some(out)
}

/// The clipping rectangle (x, y, w, h) for a border box at (x, y) of size (w, h).
pub fn area(r: &ClipRect, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let top = r[0].unwrap_or(0.0);
    let right = r[1].unwrap_or(w);
    let bottom = r[2].unwrap_or(h);
    let left = r[3].unwrap_or(0.0);
    (x + left, y + top, (right - left).max(0.0), (bottom - top).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_forms_and_auto() {
        assert_eq!(parse("rect(0 0 0 0)", 16.0), Some([Some(0.0); 4]));
        assert_eq!(parse("rect(1px, auto, 2em, auto)", 10.0), Some([Some(1.0), None, Some(20.0), None]));
        assert_eq!(parse("RECT(0px,0px,0px,0px)", 16.0), Some([Some(0.0); 4]));
        assert_eq!(parse("auto", 16.0), None);
        assert_eq!(parse("rect(0 0 0)", 16.0), None);
        assert_eq!(parse("rect(10% 0 0 0)", 16.0), None);
    }

    #[test]
    fn auto_edges_are_the_border_box() {
        let r = [Some(5.0), None, Some(15.0), None];
        assert_eq!(area(&r, 100.0, 200.0, 40.0, 30.0), (100.0, 205.0, 40.0, 10.0));
        assert_eq!(area(&[Some(0.0); 4], 1.0, 2.0, 40.0, 30.0).2, 0.0);
    }

    /// tests/sites/clip-rect.html: the visually-hidden link is cut to nothing (and can't be
    /// clicked); `rect(0, 50px, auto, 0)` keeps the left half of a box.
    #[test]
    fn absolute_boxes_are_clipped() {
        use crate::paint::Item;
        let fonts = crate::load_fonts().expect("fonts");
        let src = std::fs::read_to_string("tests/sites/clip-rect.html").unwrap();
        let dom = crate::html::parse(&src);
        let sheets = vec![crate::css::parse(&dom.text_content(dom.find_all("style")[0]))];
        let styles = crate::style::compute(&dom, &sheets);
        let lay = crate::layout::layout(&dom, &styles, fonts, &crate::images::Images::default(), 800.0, 600.0);
        let clips: Vec<(f32, f32, f32, f32)> = lay.items.iter().filter_map(|it| if let Item::ClipPush { x, y, w, h, .. } = it { Some((*x, *y, *w, *h)) } else { None }).collect();
        assert!(clips.iter().any(|c| c.2 == 0.0 && c.3 == 0.0), "the skip link is clipped away: {clips:?}");
        assert!(clips.contains(&(10.0, 40.0, 50.0, 40.0)), "half the box is kept: {clips:?}");
        let skip = (0..dom.nodes.len()).find(|&n| dom.attr(n, "id") == Some("skip")).unwrap();
        assert!(lay.hits.iter().filter(|(_, n)| *n == skip).all(|(r, _)| r.w * r.h == 0.0), "no click area left");
    }
}
