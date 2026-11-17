//! Page tiles: the page is painted once into horizontal strips that are kept until the
//! page changes. Scrolling copies the strips into place instead of painting the page again,
//! which is how real browsers scroll smoothly. The GPU presenter uploads the same strips as
//! textures.
//!
//! Pages with fixed or sticky elements paint differently at every scroll position, so they
//! are drawn the old way (painted whole each frame).

use crate::font::Fonts;
use crate::images::Images;
use crate::layout::Layout;
use crate::paint::{Canvas, Item};
use crate::style::Color;
use std::collections::HashMap;

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
    /// Tiles painted since the last call to `take_painted` (for the GPU uploader).
    painted: Vec<usize>,
    pub stats: TileStats,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct TileStats {
    pub painted: u64,
    pub reused: u64,
}

fn pack(c: Color) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// Whether the page looks the same at every scroll position (no fixed or sticky parts).
pub fn scroll_invariant(items: &[Item]) -> bool {
    !items.iter().any(|it| matches!(it, Item::FixedPush | Item::StickyPush { .. }))
}

impl PageTiles {
    /// Forget everything if the page, its layout or the window width changed.
    pub fn sync(&mut self, version: u64, width: usize, lay: &Layout) -> bool {
        let bg = pack(lay.page_bg);
        if version != self.version || width != self.width || bg != self.bg || (self.tiles.is_empty() && self.version == 0) {
            self.tiles.clear();
            self.painted.clear();
            self.version = version;
            self.width = width;
            self.bg = bg;
            self.cacheable = scroll_invariant(&lay.items);
        }
        self.cacheable
    }

    pub fn tile(&mut self, idx: usize, fonts: &Fonts, images: &Images, lay: &Layout) -> &[u32] {
        if !self.tiles.contains_key(&idx) {
            let mut c = Canvas::new(self.width, TILE_H);
            c.clear(lay.page_bg);
            c.paint(fonts, images, &lay.items, 0.0, (idx * TILE_H) as f32);
            self.tiles.insert(idx, c.buf);
            self.painted.push(idx);
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

    /// Strips painted since the last call (the GPU presenter uploads these).
    pub fn take_painted(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.painted)
    }

    pub fn get(&self, idx: usize) -> Option<&[u32]> {
        self.tiles.get(&idx).map(|v| v.as_slice())
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn memory_bytes(&self) -> usize {
        self.tiles.len() * self.width * TILE_H * 4
    }
}
