//! Painting order for block children that paint in the positioned layer.
//!
//! CSS 2 Appendix E paints a stacking context's in-flow, non-positioned blocks, then floats,
//! then inline content, and only then (step 8) its positioned descendants with `z-index: auto`
//! or `0` and the boxes that form a stacking context of their own (a transform, a filter,
//! opacity below 1), in tree order. Layout writes items in tree order, so without this a later
//! sibling's background covers a relatively positioned or transformed box that overflows into
//! it (NYT's masthead: "Today's Paper" sits in a zero-height, transformed bar and was hidden
//! under the logo row after it).
//!
//! The reordering is done among the children of one block container, inside that container's
//! own output, so the container's clip, scroll offset and any later shift still apply to the
//! moved items. (Boxes with a non-zero z-index are lifted into `Ctx::layers` instead.)

use crate::dom::NodeId;
use crate::style::Position;

/// Where one child's output went in the items, links and hits lists.
pub(super) type Span = [(usize, usize); 3];

impl super::Ctx<'_> {
    /// Whether a block child paints in step 8 rather than with the normal flow.
    pub(super) fn paints_positioned(&self, id: NodeId) -> bool {
        if self.measuring {
            return false;
        }
        let st = &self.styles[id];
        let positioned = matches!(st.position, Position::Relative | Position::Sticky);
        let context = st.transform.is_some() || st.filter.is_some() || st.opacity < 1.0;
        // A positioned box with a z-index is already in its own layer.
        (positioned || context) && !(st.position != Position::Static && st.z_index != 0)
    }

    pub(super) fn span_start(&self) -> (usize, usize, usize) {
        (self.items.len(), self.links.len(), self.hits.len())
    }

    pub(super) fn span_since(&self, s: (usize, usize, usize)) -> Span {
        [(s.0, self.items.len()), (s.1, self.links.len()), (s.2, self.hits.len())]
    }

    /// Move the output of `later` (children's spans, in tree order) after everything else
    /// written since the first of them. `fixed_before` is `self.fixed.len()` from before the
    /// children: fixed boxes placed in between record link and hit ranges, so those lists are
    /// left alone then.
    pub(super) fn paint_positioned_last(&mut self, later: &[Span], fixed_before: usize) {
        let Some(first) = later.first() else { return };
        // Nothing after the moved boxes: already in order.
        if later.last().is_some_and(|l| l[0].1 == self.items.len()) && later.windows(2).all(|w| w[0][0].1 == w[1][0].0) {
            return;
        }
        move_last(&mut self.items, later.iter().map(|s| s[0]));
        if self.fixed.len() == fixed_before {
            move_last(&mut self.links, later.iter().map(|s| s[1]));
            move_last(&mut self.hits, later.iter().map(|s| s[2]));
        }
        // Cached children placed in the moved region are no longer where they were recorded:
        // the container copies their output instead of sharing it (relayout.rs).
        let cut = first[0].0;
        self.placed.retain(|p| p.at[0].0 < cut);
    }
}

/// Reorder `v` from the first range's start on: what isn't in `ranges` first, then the ranges
/// in order. The ranges are sorted and don't overlap.
fn move_last<T>(v: &mut Vec<T>, ranges: impl Iterator<Item = (usize, usize)>) {
    let ranges: Vec<(usize, usize)> = ranges.filter(|(a, b)| b > a).collect();
    let Some(&(start, _)) = ranges.first() else { return };
    let tail: Vec<T> = v.drain(start..).collect();
    let mut rest = Vec::new();
    let mut moved = Vec::new();
    let mut r = ranges.iter().map(|&(a, b)| (a - start, b - start)).peekable();
    for (i, it) in tail.into_iter().enumerate() {
        while r.peek().is_some_and(|&(_, b)| i >= b) {
            r.next();
        }
        match r.peek() {
            Some(&(a, b)) if i >= a && i < b => moved.push(it),
            _ => rest.push(it),
        }
    }
    v.append(&mut rest);
    v.append(&mut moved);
}

#[cfg(test)]
mod tests {
    #[test]
    fn move_last_keeps_order_within_groups() {
        let mut v: Vec<u32> = (0..10).collect();
        super::move_last(&mut v, [(2, 4), (6, 7)].into_iter());
        assert_eq!(v, vec![0, 1, 4, 5, 7, 8, 9, 2, 3, 6]);
        let mut w: Vec<u32> = (0..3).collect();
        super::move_last(&mut w, [(1, 1)].into_iter());
        assert_eq!(w, vec![0, 1, 2]);
    }

    /// The index in the display list of the first background with this red value.
    fn rect_at(lay: &crate::layout::Layout, red: u8) -> usize {
        lay.items
            .iter()
            .position(|it| match it {
                crate::paint::Item::Rect { color, .. } => color.r == red && color.a > 0,
                crate::paint::Item::Box { bg, .. } => bg.color.r == red && bg.color.a > 0,
                _ => false,
            })
            .unwrap_or_else(|| panic!("no rect with red {red}"))
    }

    /// tests/sites/paint-order.html: relative, transformed and translucent boxes paint over
    /// a later in-flow sibling's background; a plain box doesn't.
    #[test]
    fn positioned_siblings_paint_over_later_backgrounds() {
        let fonts = crate::load_fonts().expect("fonts");
        let src = std::fs::read_to_string("tests/sites/paint-order.html").unwrap();
        let dom = crate::html::parse(&src);
        let sheets = vec![crate::css::parse(&dom.text_content(dom.find_all("style")[0]))];
        let styles = crate::style::compute(&dom, &sheets);
        let lay = crate::layout::layout(&dom, &styles, fonts, &crate::images::Images::default(), 800.0, 600.0);
        let after = rect_at(&lay, 200); // the later sibling's background
        for (red, what) in [(1, "relative"), (2, "transformed"), (3, "opacity"), (4, "zero-height transformed bar")] {
            assert!(rect_at(&lay, red) > after, "{what} box paints after the later sibling");
        }
        assert!(rect_at(&lay, 5) < after, "a static box keeps tree order");
    }
}
