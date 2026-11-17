//! Page tiles: the page is painted once into horizontal strips that are kept until the
//! page changes. Scrolling copies the strips into place instead of painting the page again,
//! which is how real browsers scroll smoothly. The GPU presenter uploads the same strips as
//! textures.
//!
//! When the page changes (a script edit, an animation frame), only strips whose content
//! changed are painted again: each strip keeps a fingerprint of the paint items that touch
//! it, and strips whose fingerprint is unchanged are kept.
//!
//! Pages with fixed or sticky elements paint differently at every scroll position, so they
//! are drawn the old way (painted whole each frame).

use crate::font::Fonts;
use crate::images::Images;
use crate::layout::Layout;
use crate::paint::{Canvas, Item};
use crate::style::Color;
use std::collections::HashMap;
use std::hash::Hasher;

pub const TILE_H: usize = 256;

/// Strips kept above and below the visible ones, so short scrolls don't wait on painting.
const KEEP_MARGIN: usize = 2;

#[derive(Default)]
pub struct PageTiles {
    version: u64,
    width: usize,
    bg: u32,
    cacheable: bool,
    tiles: HashMap<usize, Vec<u32>>,
    /// Each stored strip's content fingerprint and paint generation (bumped on every repaint,
    /// so the GPU presenter knows which textures to upload again).
    sigs: HashMap<usize, u64>,
    gens: HashMap<usize, u64>,
    next_gen: u64,
    /// The current page's item groups (see `groups`), worked out once per page version.
    groups: Option<Vec<(f32, f32, u64)>>,
    pub stats: TileStats,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct TileStats {
    pub painted: u64,
    pub reused: u64,
    /// Strips kept across a page change because nothing in them changed.
    pub kept: u64,
}

fn pack(c: Color) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// Whether the page looks the same at every scroll position (no fixed or sticky parts), and
/// can be cut into strips (backdrop filters read what's around them, so they can't).
pub fn scroll_invariant(items: &[Item]) -> bool {
    !items.iter().any(|it| match it {
        Item::FixedPush | Item::StickyPush { .. } => true,
        Item::EffectPush(fx) => !fx.backdrop.is_empty(),
        _ => false,
    })
}

/// A Hasher that can be written to with `write!` (hashes an item's Debug text without
/// building a string).
struct HashWriter(std::collections::hash_map::DefaultHasher);

impl std::fmt::Write for HashWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

/// Vertical extent (page rows) an item paints into, or None for items that paint nothing.
fn item_span(it: &Item) -> Option<(f32, f32)> {
    Some(match it {
        Item::Rect { y, h, .. } | Item::Image { y, h, .. } | Item::Box { y, h, .. } | Item::Ring { y, h, .. } | Item::ClipPush { y, h, .. } => (*y, y + h),
        Item::Shadow { y, h, blur, .. } => (y - blur, y + h + blur),
        Item::Text { y, size, .. } => (y - size * 1.6, y + size * 0.8),
        Item::TransformPush { m, x, y, w, h } => {
            let (_, by, _, bh) = crate::transform::bounds(m, *x, *y, *w, *h);
            (by, by + bh)
        }
        Item::EffectPush(fx) => {
            let m = fx.margin();
            (fx.area.1.min(fx.y) - m, (fx.area.1 + fx.area.3).max(fx.y + fx.h) + m)
        }
        _ => return None,
    })
}

/// The page's paint items as top-level groups (a clip, transform or effect with everything
/// inside it counts as one): each group's vertical extent and a fingerprint of its content,
/// including which decoded image it shows.
fn groups(items: &[Item], images: &Images) -> Vec<(f32, f32, u64)> {
    use std::fmt::Write;
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let first = &items[i];
        let Some((y0, mut y1)) = item_span(first) else {
            i += 1;
            continue;
        };
        let mut h = HashWriter(Default::default());
        // Find the end of a push..pop group.
        let opens = |it: &Item| matches!(it, Item::ClipPush { .. } | Item::TransformPush { .. } | Item::EffectPush(_) | Item::StickyPush { .. } | Item::FixedPush);
        let closes = |it: &Item| matches!(it, Item::ClipPop | Item::TransformPop | Item::EffectPop | Item::StickyPop | Item::FixedPop);
        let mut j = i + 1;
        if opens(first) {
            let mut depth = 1;
            while j < items.len() && depth > 0 {
                if opens(&items[j]) {
                    depth += 1;
                } else if closes(&items[j]) {
                    depth -= 1;
                }
                j += 1;
            }
        }
        for it in &items[i..j] {
            if matches!(it, Item::Mark { .. } | Item::ScrollArea { .. }) {
                continue;
            }
            let _ = write!(h, "{it:?}");
            let src = match it {
                Item::Image { src, .. } => Some(src.as_str()),
                Item::Box { bg, .. } => match &bg.image {
                    Some(crate::style::BgImage::Url(u)) => Some(u.as_str()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(src) = src {
                h.0.write_usize(images.get(src).map_or(0, |a| std::sync::Arc::as_ptr(a) as usize));
            }
            // Inside a clip, items can't reach past it; inside transforms and effects the
            // group's own extent already covers them.
            if j > i + 1 && matches!(first, Item::ClipPush { .. }) {
                continue;
            }
            if let Some((_, b)) = item_span(it).filter(|_| !matches!(first, Item::TransformPush { .. } | Item::EffectPush(_))) {
                y1 = y1.max(b);
            }
        }
        out.push((y0, y1, h.0.finish()));
        i = j;
    }
    out
}

/// Fingerprint of everything painted into page rows [top, top + TILE_H).
fn band_signature(groups: &[(f32, f32, u64)], idx: usize) -> u64 {
    let (top, bottom) = ((idx * TILE_H) as f32, ((idx + 1) * TILE_H) as f32);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for &(y0, y1, g) in groups {
        // A pixel of slack for anti-aliased edges.
        if y1 >= top - 1.0 && y0 <= bottom + 1.0 {
            h.write_u64(g);
        }
    }
    h.finish()
}

impl PageTiles {
    /// Bring the stored strips up to date with the page: forget all of them if the window
    /// width or page background changed, otherwise only those whose content changed.
    pub fn sync(&mut self, version: u64, width: usize, lay: &Layout, images: &Images) -> bool {
        let bg = pack(lay.page_bg);
        if width != self.width || bg != self.bg || (self.tiles.is_empty() && self.version == 0) {
            self.tiles.clear();
            self.sigs.clear();
            self.groups = None;
            self.version = version;
            self.width = width;
            self.bg = bg;
            self.cacheable = scroll_invariant(&lay.items);
        } else if version != self.version {
            self.version = version;
            self.cacheable = scroll_invariant(&lay.items);
            self.groups = None;
            if !self.cacheable || self.tiles.is_empty() {
                self.tiles.clear();
                self.sigs.clear();
            } else {
                let g = groups(&lay.items, images);
                let sigs = &self.sigs;
                self.tiles.retain(|i, _| sigs.get(i) == Some(&band_signature(&g, *i)));
                self.sigs.retain(|i, _| self.tiles.contains_key(i));
                self.stats.kept += self.tiles.len() as u64;
                self.groups = Some(g);
            }
        }
        self.cacheable
    }

    /// Which paint of strip `idx` is stored (changes whenever it's painted again).
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    pub fn generation(&self, idx: usize) -> Option<u64> {
        self.gens.get(&idx).copied().filter(|_| self.tiles.contains_key(&idx))
    }

    pub fn tile(&mut self, idx: usize, fonts: &Fonts, images: &Images, lay: &Layout) -> &[u32] {
        if !self.tiles.contains_key(&idx) {
            let mut c = Canvas::new(self.width, TILE_H);
            c.clear(lay.page_bg);
            c.paint(fonts, images, &lay.items, 0.0, (idx * TILE_H) as f32);
            self.tiles.insert(idx, c.buf);
            // Remember what went into it, to tell later whether it needs painting again.
            let g = self.groups.get_or_insert_with(|| groups(&lay.items, images));
            let sig = band_signature(g, idx);
            self.sigs.insert(idx, sig);
            self.next_gen += 1;
            self.gens.insert(idx, self.next_gen);
            self.stats.painted += 1;
        } else {
            self.stats.reused += 1;
        }
        &self.tiles[&idx]
    }

    /// The strips covering page rows [top, top + h).
    pub fn visible(top: f32, h: usize) -> std::ops::RangeInclusive<usize> {
        let first = (top.max(0.0) as usize) / TILE_H;
        let last = ((top.max(0.0) as usize) + h.max(1) - 1) / TILE_H;
        first..=last
    }

    /// Drop strips far from the visible ones, to keep memory small.
    pub fn evict(&mut self, visible: &std::ops::RangeInclusive<usize>) {
        let (lo, hi) = (visible.start().saturating_sub(KEEP_MARGIN), visible.end() + KEEP_MARGIN);
        self.tiles.retain(|&i, _| i >= lo && i <= hi);
        self.sigs.retain(|&i, _| i >= lo && i <= hi);
    }

    /// Software presenter: copy the visible strips into the canvas below the toolbar.
    pub fn compose(&mut self, canvas: &mut Canvas, fonts: &Fonts, images: &Images, lay: &Layout, origin_y: usize, scroll: f32) {
        let view_h = canvas.h.saturating_sub(origin_y);
        let scroll_px = scroll.max(0.0).round() as usize;
        let range = Self::visible(scroll_px as f32, view_h);
        let w = canvas.w.min(self.width);
        for idx in range.clone() {
            let tile_top = idx * TILE_H;
            let buf = self.tile(idx, fonts, images, lay);
            // Rows of this strip that are on screen.
            let from = scroll_px.max(tile_top);
            let to = (scroll_px + view_h).min(tile_top + TILE_H);
            for y in from..to {
                let src = (y - tile_top) * w;
                let dst = (origin_y + y - scroll_px) * canvas.w;
                canvas.buf[dst..dst + w].copy_from_slice(&buf[src..src + w]);
            }
        }
        self.evict(&range);
    }

    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    pub fn get(&self, idx: usize) -> Option<&[u32]> {
        self.tiles.get(&idx).map(|v| v.as_slice())
    }

    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    pub fn width(&self) -> usize {
        self.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(y: f32, r: u8) -> Item {
        Item::Rect { x: 0.0, y, w: 50.0, h: 20.0, color: Color::rgb(r, 0, 0) }
    }

    #[test]
    fn only_changed_strips_are_painted_again() {
        let Ok(fonts) = Fonts::load() else { return };
        let images = Images::default();
        let mut lay = Layout { items: vec![rect(10.0, 1), rect(300.0, 2), rect(600.0, 3)], height: 800.0, ..Default::default() };
        let mut t = PageTiles::default();
        assert!(t.sync(1, 100, &lay, &images));
        for i in 0..3 {
            t.tile(i, &fonts, &images, &lay);
        }
        let g0 = t.generation(0);
        // Change only the rectangle in strip 1 (rows 256..512).
        lay.items[1] = rect(300.0, 9);
        t.sync(2, 100, &lay, &images);
        assert!(t.get(0).is_some() && t.get(2).is_some(), "untouched strips are kept");
        assert!(t.get(1).is_none(), "the changed strip is painted again");
        assert_eq!(t.generation(0), g0);
        // Moving something from strip 2 to strip 1 repaints both and keeps strip 0.
        t.tile(1, &fonts, &images, &lay);
        lay.items[2] = rect(400.0, 3);
        t.sync(3, 100, &lay, &images);
        assert!(t.get(0).is_some() && t.get(1).is_none() && t.get(2).is_none());
        // A new window width forgets everything.
        t.sync(3, 120, &lay, &images);
        assert!(t.get(0).is_none());
    }
}
