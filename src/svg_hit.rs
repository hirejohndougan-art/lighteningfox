//! Hit areas for the shapes inside an inline `<svg>`, so clicks, hover and `:hover` reach the
//! `<path>` or `<circle>` under the pointer rather than the whole `<svg>`.
//!
//! Layout keeps hit areas as rectangles. Each shape's painted area (its fill when it has one,
//! its stroke outline when it has a stroke, as `pointer-events: visiblePainted` says) is cut
//! into horizontal strips a couple of pixels high, so a circle's corners don't count. They go
//! after the `<svg>`'s own hit, in painting order, so the topmost shape wins.

use crate::dom::NodeId;
use crate::layout::Rect;
use resvg::usvg::tiny_skia_path::{Path, PathSegment, Transform};
use resvg::usvg::{self, FillRule, Node};
use std::collections::HashMap;

/// Strip height in page px.
const STEP: f32 = 2.0;
/// Past this many strips for one `<svg>`, shapes are hit by their bounding boxes.
const MAX_RECTS: usize = 6000;

/// Page placement of the drawing: page = tree coordinates × scale + offset.
#[derive(Clone, Copy)]
pub struct Place {
    pub sx: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

/// The hit areas of the shapes in `tree` (drawn for the `<svg>` at `root`), clipped to `clip`.
pub fn shape_hits(tree: &usvg::Tree, ids: &HashMap<String, NodeId>, root: NodeId, place: Place, clip: Rect) -> Vec<(Rect, NodeId)> {
    let mut w = Walker { ids, place, clip, out: Vec::new(), boxes_only: false };
    w.group(tree.root(), root);
    if w.out.len() > MAX_RECTS {
        w.out.clear();
        w.boxes_only = true;
        w.group(tree.root(), root);
    }
    w.out
}

struct Walker<'a> {
    ids: &'a HashMap<String, NodeId>,
    place: Place,
    clip: Rect,
    out: Vec<(Rect, NodeId)>,
    boxes_only: bool,
}

impl Walker<'_> {
    fn target(&self, id: &str, outer: NodeId) -> NodeId {
        self.ids.get(id).copied().unwrap_or(outer)
    }

    fn to_page(&self, t: Transform) -> Transform {
        t.post_concat(Transform::from_row(self.place.sx, 0.0, 0.0, self.place.sy, self.place.tx, self.place.ty))
    }

    fn push(&mut self, r: Rect, node: NodeId) {
        let c = self.clip;
        let (x0, y0) = (r.x.max(c.x), r.y.max(c.y));
        let (x1, y1) = ((r.x + r.w).min(c.x + c.w), (r.y + r.h).min(c.y + c.h));
        if x1 > x0 && y1 > y0 {
            // A strip just under an equally wide one of the same shape grows that one (a
            // rectangle is one hit, not a stack of strips).
            if let Some((last, n)) = self.out.last_mut() {
                let same = |a: f32, b: f32| (a - b).abs() < 0.01;
                if *n == node && same(last.x, x0) && same(last.w, x1 - x0) && same(last.y + last.h, y0) {
                    last.h = y1 - last.y;
                    return;
                }
            }
            self.out.push((Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, node));
        }
    }

    fn bbox(&mut self, r: Option<usvg::Rect>, node: NodeId) {
        let Some(r) = r else { return };
        let p = self.place;
        self.push(Rect { x: r.x() * p.sx + p.tx, y: r.y() * p.sy + p.ty, w: r.width() * p.sx, h: r.height() * p.sy }, node);
    }

    fn group(&mut self, g: &usvg::Group, target: NodeId) {
        let target = self.target(g.id(), target);
        for n in g.children() {
            match n {
                Node::Group(g) => self.group(g, target),
                Node::Path(p) => self.path(p, target),
                Node::Image(i) => {
                    if i.is_visible() {
                        let t = self.target(i.id(), target);
                        self.bbox(Some(i.abs_bounding_box()), t);
                    }
                }
                Node::Text(t) => {
                    let to = self.target(t.id(), target);
                    // Text is hit anywhere in its glyphs' box, as browsers do for text.
                    self.bbox(Some(t.abs_bounding_box()), to);
                }
            }
        }
    }

    fn path(&mut self, p: &usvg::Path, target: NodeId) {
        if !p.is_visible() || (p.fill().is_none() && p.stroke().is_none()) {
            return;
        }
        let t = self.target(p.id(), target);
        if self.boxes_only {
            let b = if p.stroke().is_some() { p.abs_stroke_bounding_box() } else { p.abs_bounding_box() };
            self.bbox(b, t);
            return;
        }
        let ts = self.to_page(p.abs_transform());
        if let Some(f) = p.fill() {
            if let Some(path) = p.data().clone().transform(ts) {
                self.spans(&path, f.rule() == FillRule::EvenOdd, t);
            }
        }
        if let Some(s) = p.stroke() {
            if let Some(path) = p.data().stroke(&s.to_tiny_skia(), 1.0).and_then(|o| o.transform(ts)) {
                self.spans(&path, false, t);
            }
        }
    }

    /// Cut a filled path (page coordinates) into strips.
    fn spans(&mut self, path: &Path, even_odd: bool, node: NodeId) {
        let edges = edges(path);
        let b = path.bounds();
        // Too thin for a strip to cross its middle: its box.
        if b.height() < STEP || b.width() < 1.0 {
            self.push(Rect { x: b.x(), y: b.y(), w: b.width().max(1.0), h: b.height().max(1.0) }, node);
            return;
        }
        // Strips line up with the box's top, so a box moved as a whole keeps the same strips.
        let top = self.clip.y;
        let mut y = top + ((b.y() - top) / STEP).floor() * STEP;
        let mut xs: Vec<(f32, i32)> = Vec::new();
        while y < b.bottom() {
            let mid = y + STEP / 2.0;
            xs.clear();
            for &(x0, y0, x1, y1) in &edges {
                let (lo, hi, dir) = if y0 < y1 { (y0, y1, 1) } else { (y1, y0, -1) };
                if mid >= lo && mid < hi {
                    xs.push((x0 + (mid - y0) * (x1 - x0) / (y1 - y0), dir));
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            let mut start = 0.0;
            for &(x, d) in &xs {
                let was = if even_odd { wind % 2 != 0 } else { wind != 0 };
                wind += d;
                let now = if even_odd { wind % 2 != 0 } else { wind != 0 };
                if !was && now {
                    start = x;
                } else if was && !now && x > start {
                    self.push(Rect { x: start, y, w: x - start, h: STEP }, node);
                }
            }
            y += STEP;
        }
    }
}

/// A path's outline as straight edges (curves split into short lines), closing each contour.
fn edges(path: &Path) -> Vec<(f32, f32, f32, f32)> {
    let mut out = Vec::new();
    let (mut sx, mut sy, mut x, mut y) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    const N: usize = 8;
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                if (x, y) != (sx, sy) {
                    out.push((x, y, sx, sy));
                }
                (sx, sy, x, y) = (p.x, p.y, p.x, p.y);
            }
            PathSegment::LineTo(p) => {
                out.push((x, y, p.x, p.y));
                (x, y) = (p.x, p.y);
            }
            PathSegment::QuadTo(c, p) => {
                let (mut px, mut py) = (x, y);
                for i in 1..=N {
                    let t = i as f32 / N as f32;
                    let u = 1.0 - t;
                    let nx = u * u * x + 2.0 * u * t * c.x + t * t * p.x;
                    let ny = u * u * y + 2.0 * u * t * c.y + t * t * p.y;
                    out.push((px, py, nx, ny));
                    (px, py) = (nx, ny);
                }
                (x, y) = (p.x, p.y);
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let (mut px, mut py) = (x, y);
                for i in 1..=N {
                    let t = i as f32 / N as f32;
                    let u = 1.0 - t;
                    let nx = u * u * u * x + 3.0 * u * u * t * c1.x + 3.0 * u * t * t * c2.x + t * t * t * p.x;
                    let ny = u * u * u * y + 3.0 * u * u * t * c1.y + 3.0 * u * t * t * c2.y + t * t * t * p.y;
                    out.push((px, py, nx, ny));
                    (px, py) = (nx, ny);
                }
                (x, y) = (p.x, p.y);
            }
            PathSegment::Close => {
                if (x, y) != (sx, sy) {
                    out.push((x, y, sx, sy));
                }
                (x, y) = (sx, sy);
            }
        }
    }
    if (x, y) != (sx, sy) {
        out.push((x, y, sx, sy));
    }
    out.retain(|e| e.1 != e.3);
    out
}

#[cfg(test)]
mod tests {
    /// Points over a shape reach that shape; a circle's box corner and empty space reach the
    /// `<svg>`; a later shape over an earlier one wins; `fill="none"` shapes are hit on the stroke.
    #[test]
    fn points_reach_the_shape_under_them() {
        let fonts = crate::load_fonts().expect("fonts");
        crate::relayout::clear();
        let html = r#"<body style="margin:0"><svg id="s" width="200" height="100" viewBox="0 0 100 50" style="display:block">
            <circle id="c" cx="25" cy="25" r="20" fill="red"/>
            <rect id="r" x="20" y="20" width="10" height="10" fill="blue"/>
            <rect id="o" x="60" y="5" width="30" height="40" fill="none" stroke="black" stroke-width="4"/>
            <g id="g"><path id="p" d="M60 0 L64 0 L64 2 Z" fill="green"/></g></svg>"#;
        let dom = crate::html::parse(html);
        let styles = crate::style::compute(&dom, &[]);
        let mut images = crate::images::Images::default();
        crate::svg_inline::publish(&dom, &styles, &mut images);
        let lay = crate::layout::layout(&dom, &styles, fonts, &images, 800.0, 600.0);
        let id = |s: &str| (0..dom.nodes.len()).find(|&n| dom.attr(n, "id") == Some(s));
        let at = |x: f32, y: f32| lay.hit_at(x, y, 0.0).and_then(|n| dom.attr(n, "id").map(str::to_string));
        assert!(id("c").is_some());
        assert_eq!(at(50.0, 15.0).as_deref(), Some("c")); // circle top, (25, 7.5) in the drawing
        assert_eq!(at(50.0, 50.0).as_deref(), Some("r")); // the rect drawn over the circle
        assert_eq!(at(14.0, 14.0).as_deref(), Some("s")); // the circle's box corner
        assert_eq!(at(150.0, 50.0).as_deref(), Some("s")); // inside the unfilled rect
        assert_eq!(at(121.0, 50.0).as_deref(), Some("o")); // on its stroke (x = 60.5)
        assert_eq!(at(195.0, 95.0).as_deref(), Some("s"));
    }

    /// A `<foreignObject>`'s HTML is laid out in the drawing's user units, scaled with it,
    /// and gets clicks over the shapes under it.
    #[test]
    fn foreign_object_content_is_laid_out_and_hit() {
        let fonts = crate::load_fonts().expect("fonts");
        crate::relayout::clear();
        let html = r##"<body style="margin:0"><svg id="s" width="200" height="100" viewBox="0 0 100 50" style="display:block">
            <rect id="bg" width="100" height="50" fill="#eee"/>
            <foreignObject id="fo" x="10" y="5" width="40" height="20"><div id="inner" style="height:10px;font:6px/10px sans-serif;background:red">x</div></foreignObject></svg>"##;
        let dom = crate::html::parse(html);
        let styles = crate::style::compute(&dom, &[]);
        let mut images = crate::images::Images::default();
        crate::svg_inline::publish(&dom, &styles, &mut images);
        let lay = crate::layout::layout(&dom, &styles, fonts, &images, 800.0, 600.0);
        let at = |x: f32, y: f32| lay.hit_at(x, y, 0.0).and_then(|n| dom.attr(n, "id").map(str::to_string));
        // Text hits name the text node: count its element.
        let at = |x: f32, y: f32| at(x, y).or_else(|| lay.hit_at(x, y, 0.0).and_then(|n| dom.nodes[n].parent).and_then(|p| dom.attr(p, "id").map(str::to_string)));
        // (10, 5) in the drawing is (20, 10) on the page; the 10-unit-high div ends at y = 30.
        assert_eq!(at(25.0, 15.0).as_deref(), Some("inner"));
        assert_eq!(at(25.0, 35.0).as_deref(), Some("bg"));
        assert_eq!(at(15.0, 15.0).as_deref(), Some("bg"));
        let red = lay.items.iter().any(|it| matches!(it, crate::paint::Item::Box { .. } | crate::paint::Item::Rect { .. }) && format!("{it:?}").contains("r: 255, g: 0, b: 0"));
        assert!(red, "the div's background is drawn");
    }
}
