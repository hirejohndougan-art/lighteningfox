//! Painting: turns the display list into pixels in a framebuffer.

use crate::font::Fonts;
use crate::images::Images;
use crate::style::{Background, BgImage, BgSize, Color, Len};

#[derive(Debug, Clone)]
pub enum Item {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Color },
    /// `y` is the text baseline.
    Text { x: f32, y: f32, text: String, font: u8, size: f32, color: Color },
    /// An image filling its box. `view`, when set (object-fit), is where the whole picture
    /// sits relative to the box (dx, dy, w, h); only the part inside the box is drawn.
    Image { x: f32, y: f32, w: f32, h: f32, src: String, view: Option<Box<[f32; 4]>> },
    /// A background: color and/or image/gradient, clipped to rounded corners.
    Box { x: f32, y: f32, w: f32, h: f32, radius: f32, bg: Box<Background> },
    /// A uniform border following rounded corners.
    Ring { x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32, color: Color },
    /// A soft drop shadow (box-shadow).
    Shadow { x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, color: Color },
    /// Start clipping to a (rounded) rectangle until the matching ClipPop (overflow: hidden).
    ClipPush { x: f32, y: f32, w: f32, h: f32, radius: f32 },
    ClipPop,
    /// Items between these markers stay put when the page scrolls (position: fixed).
    FixedPush,
    FixedPop,
    /// Where an element's border box ended up (for scripts' getBoundingClientRect).
    /// Never painted; collected into `Layout::boxes` at the end of layout.
    Mark { id: usize, x: f32, y: f32, w: f32, h: f32 },
    /// Start of a position: sticky box whose border box starts at page `y` and is `h` tall.
    /// While scrolling, it moves down by up to `limit` px to stay `top` px below the window top.
    StickyPush { id: usize, y: f32, h: f32, top: f32, limit: f32 },
    StickyPop,
    /// A scrollable box (overflow: auto/scroll): its padding box and how far it can scroll.
    /// Never painted; collected into `Layout::scrollers`.
    ScrollArea { id: usize, x: f32, y: f32, w: f32, h: f32, max: f32 },
    /// Start of a transformed element: everything up to the matching TransformPop is drawn
    /// off-screen over the page area (x, y, w, h), then onto the page through matrix `m`.
    TransformPush { m: [f32; 6], x: f32, y: f32, w: f32, h: f32 },
    TransformPop,
    /// Start of an element with `filter`/`backdrop-filter` (or a blurred text shadow): the
    /// items up to the matching EffectPop are drawn off-screen, filtered, then blended in.
    EffectPush(Box<crate::effects::Effect>),
    EffectPop,
}

/// Index of the EffectPop matching the EffectPush just before `from` (or items.len()).
fn matching_pop(items: &[Item], from: usize) -> usize {
    let mut depth = 1;
    let mut j = from;
    while j < items.len() {
        match items[j] {
            Item::EffectPush(_) => depth += 1,
            Item::EffectPop => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
        j += 1;
    }
    items.len()
}

/// How far a sticky box moves down at this scroll position.
pub fn sticky_offset(y: f32, top: f32, limit: f32, scroll: f32) -> f32 {
    (scroll + top - y).clamp(0.0, limit.max(0.0))
}

/// Signed distance from (px, py) to a rounded rectangle's edge (negative inside).
#[inline]
fn rounded_dist(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let qx = (px - cx).abs() - (w / 2.0 - r);
    let qy = (py - cy).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Coverage (0..1) of pixel center (px, py) inside a rounded rectangle. Anti-aliased edges.
#[inline]
pub(crate) fn rounded_cov(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let dx = ((px - cx).abs() - (w / 2.0 - r)).max(0.0);
    let dy = ((py - cy).abs() - (h / 2.0 - r)).max(0.0);
    let d = if dx > 0.0 && dy > 0.0 {
        (dx * dx + dy * dy).sqrt() - r
    } else {
        // Straight edges: distance outside the plain rectangle.
        ((px - cx).abs() - w / 2.0).max((py - cy).abs() - h / 2.0)
    };
    (0.5 - d).clamp(0.0, 1.0)
}

#[derive(Clone, Copy)]
struct Clip {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub buf: Vec<u32>, // 0RGB
    clips: Vec<Clip>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, buf: vec![0xFFFFFF; w * h], clips: Vec::new() }
    }

    pub fn clear(&mut self, c: Color) {
        let px = pack(c);
        self.buf.iter_mut().for_each(|p| *p = px);
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: Color, coverage: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let mut a = coverage * c.a as f32 / 255.0;
        if a <= 0.0 {
            return;
        }
        for clip in &self.clips {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if fx < clip.x || fy < clip.y || fx > clip.x + clip.w || fy > clip.y + clip.h {
                return;
            }
            if clip.radius > 0.0 {
                a *= rounded_cov(fx, fy, clip.x, clip.y, clip.w, clip.h, clip.radius);
                if a <= 0.0 {
                    return;
                }
            }
        }
        let i = y as usize * self.w + x as usize;
        if a >= 1.0 {
            self.buf[i] = pack(c);
            return;
        }
        let d = self.buf[i];
        let mix = |src: u8, dst: u32| ((src as f32) * a + (dst as f32) * (1.0 - a)) as u32;
        let r = mix(c.r, (d >> 16) & 0xFF);
        let g = mix(c.g, (d >> 8) & 0xFF);
        let b = mix(c.b, d & 0xFF);
        self.buf[i] = (r << 16) | (g << 8) | b;
    }

    /// Blend one pixel (respecting the current clips).
    pub(crate) fn blend_px(&mut self, x: i32, y: i32, c: Color, coverage: f32) {
        self.blend(x, y, c, coverage);
    }

    /// Paint a group of items with page coordinates shifted by (dx, dy), no scrolling.
    pub(crate) fn paint_group(&mut self, fonts: &Fonts, images: &Images, items: &[Item], dx: f32, dy: f32) {
        self.paint_at(fonts, images, items, dx, dy, 0.0);
    }

    /// Draw a small ARGB image (an icon) with its alpha, top-left at (x, y).
    pub fn draw_argb(&mut self, x: i32, y: i32, w: usize, px: &[u32]) {
        for (i, &p) in px.iter().enumerate() {
            let c = Color { r: (p >> 16) as u8, g: (p >> 8) as u8, b: p as u8, a: (p >> 24) as u8 };
            self.blend(x + (i % w) as i32, y + (i / w) as i32, c, 1.0);
        }
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        if c.a == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let x0 = (x.round() as i32).max(0);
        let y0 = (y.round() as i32).max(0);
        let x1 = ((x + w).round() as i32).min(self.w as i32);
        let y1 = ((y + h).round() as i32).min(self.h as i32);
        for yy in y0..y1 {
            for xx in x0..x1 {
                self.blend(xx, yy, c, 1.0);
            }
        }
    }

    pub fn draw_text(&mut self, fonts: &Fonts, x: f32, baseline: f32, text: &str, font: u8, size: f32, c: Color) {
        if baseline + size < 0.0 || baseline - size > self.h as f32 {
            return;
        }
        let shaped = fonts.shape(font, text, size);
        for g in &shaped.glyphs {
            let (gxp, gyp) = (x + g.x, baseline - g.y);
            // Whole pixels, plus a quarter-pixel step drawn into the glyph itself.
            let steps = crate::font::SUBPIXEL_STEPS;
            let q = (gxp * steps).round() / steps;
            let (whole, subpx) = (q.floor(), ((q - q.floor()) * steps).round() as u8 % steps as u8);
            fonts.with_glyph_id(g.face, g.id, size, subpx, |m, bmp| {
                let gx = whole as i32 + m.xmin;
                let gy = gyp.round() as i32 - m.height as i32 - m.ymin;
                if m.color {
                    // A color glyph (emoji): its own colors, faded with the text's opacity.
                    for row in 0..m.height {
                        for col in 0..m.width {
                            let i = (row * m.width + col) * 4;
                            let Some(px) = bmp.get(i..i + 4) else { continue };
                            if px[3] > 0 {
                                let a = (px[3] as f32 * c.a as f32 / 255.0) as u8;
                                self.blend(gx + col as i32, gy + row as i32, Color { r: px[0], g: px[1], b: px[2], a }, 1.0);
                            }
                        }
                    }
                    return;
                }
                for row in 0..m.height {
                    for col in 0..m.width {
                        let cov = bmp[row * m.width + col];
                        if cov > 0 {
                            self.blend(gx + col as i32, gy + row as i32, c, cov as f32 / 255.0);
                        }
                    }
                }
            });
        }
    }

    fn draw_image(&mut self, img: &image::RgbaImage, x: f32, y: f32) {
        let (x0, y0) = (x.round() as i32, y.round() as i32);
        for (ix, iy, px) in img.enumerate_pixels() {
            let [r, g, b, a] = px.0;
            if a > 0 {
                self.blend(x0 + ix as i32, y0 + iy as i32, Color { r, g, b, a }, 1.0);
            }
        }
    }

    /// Draw `img` at (x, y), only where it falls inside `clip` (x, y, w, h).
    fn draw_image_clipped(&mut self, img: &image::RgbaImage, x: f32, y: f32, clip: (f32, f32, f32, f32)) {
        let (x0, y0) = (x.round() as i32, y.round() as i32);
        let (cx0, cy0, cx1, cy1) = self.span(clip.0, clip.1, clip.2, clip.3);
        let (cx0, cy0) = (cx0.max(clip.0.round() as i32), cy0.max(clip.1.round() as i32));
        let (cx1, cy1) = (cx1.min((clip.0 + clip.2).round() as i32), cy1.min((clip.1 + clip.3).round() as i32));
        for py in cy0.max(y0)..cy1.min(y0 + img.height() as i32) {
            for px in cx0.max(x0)..cx1.min(x0 + img.width() as i32) {
                let [r, g, b, a] = img.get_pixel((px - x0) as u32, (py - y0) as u32).0;
                if a > 0 {
                    self.blend(px, py, Color { r, g, b, a }, 1.0);
                }
            }
        }
    }

    /// Pixel range covered by a box, clipped to the canvas.
    fn span(&self, x: f32, y: f32, w: f32, h: f32) -> (i32, i32, i32, i32) {
        (
            (x.floor() as i32).max(0),
            (y.floor() as i32).max(0),
            ((x + w).ceil() as i32).min(self.w as i32),
            ((y + h).ceil() as i32).min(self.h as i32),
        )
    }

    fn fill_background(&mut self, images: &Images, x: f32, y: f32, w: f32, h: f32, radius: f32, bg: &Background) {
        let (x0, y0, x1, y1) = self.span(x, y, w, h);
        let cov = |px: i32, py: i32| {
            if radius > 0.0 {
                rounded_cov(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, radius)
            } else {
                1.0
            }
        };
        if bg.color.a > 0 {
            for py in y0..y1 {
                for px in x0..x1 {
                    let c = cov(px, py);
                    if c > 0.0 {
                        self.blend(px, py, bg.color, c);
                    }
                }
            }
        }
        // Image layers, bottom first (the first layer is drawn on top).
        for layer in bg.layers().iter().rev() {
            self.fill_layer(images, (x, y, w, h), (x0, y0, x1, y1), &cov, layer);
        }
    }

    #[allow(clippy::type_complexity)]
    fn fill_layer(&mut self, images: &Images, (x, y, w, h): (f32, f32, f32, f32), (x0, y0, x1, y1): (i32, i32, i32, i32), cov: &dyn Fn(i32, i32) -> f32, layer: &crate::background::Layer) {
        let (size, pos, repeat) = (layer.size, layer.pos, layer.repeat);
        match layer.image {
            BgImage::Gradient(g) => {
                // A gradient fills the box unless background-size says otherwise; then it
                // tiles like an image.
                let (tw, th) = match size {
                    BgSize::Size(a, b) => {
                        let tw = if a == Len::Auto { w } else { a.resolve(w) };
                        let th = if b == Len::Auto { h } else { b.resolve(h) };
                        (tw.max(1.0), th.max(1.0))
                    }
                    _ => (w, h),
                };
                let off = |l: Len, free: f32| match l {
                    Len::Pct(p) => free * p / 100.0,
                    Len::Mix(px, p) => px + free * p / 100.0,
                    other => other.resolve(0.0),
                };
                let (ox, oy) = (x + off(pos.0, w - tw), y + off(pos.1, h - th));
                let prepared = g.prepare(tw, th);
                for py in y0..y1 {
                    let mut ty = py as f32 + 0.5 - oy;
                    if repeat.1 {
                        ty = ty.rem_euclid(th);
                    } else if ty < 0.0 || ty >= th {
                        continue;
                    }
                    for px in x0..x1 {
                        let mut tx = px as f32 + 0.5 - ox;
                        if repeat.0 {
                            tx = tx.rem_euclid(tw);
                        } else if tx < 0.0 || tx >= tw {
                            continue;
                        }
                        let c = cov(px, py);
                        if c > 0.0 {
                            self.blend(px, py, prepared.at(tx, ty), c);
                        }
                    }
                }
            }
            BgImage::Url(src) => {
                let Some((iw, ih)) = images.size(src) else { return };
                let ratio = iw / ih.max(1.0);
                let (tw, th) = match size {
                    BgSize::Auto => (iw, ih),
                    BgSize::Cover => {
                        let s = (w / iw).max(h / ih);
                        (iw * s, ih * s)
                    }
                    BgSize::Contain => {
                        let s = (w / iw).min(h / ih);
                        (iw * s, ih * s)
                    }
                    BgSize::Size(a, b) => match (a, b) {
                        (Len::Auto, Len::Auto) => (iw, ih),
                        (a, Len::Auto) => (a.resolve(w), a.resolve(w) / ratio),
                        (Len::Auto, b) => (b.resolve(h) * ratio, b.resolve(h)),
                        (a, b) => (a.resolve(w), b.resolve(h)),
                    },
                };
                let (tw, th) = (tw.round().max(1.0), th.round().max(1.0));
                let Some(tile) = images.scaled(src, tw as u32, th as u32) else { return };
                // Percent positions align the same point of image and box.
                let off = |l: Len, free: f32| match l {
                    Len::Pct(p) => free * p / 100.0,
                    Len::Mix(px, p) => px + free * p / 100.0,
                    other => other.resolve(0.0),
                };
                let ox = x + off(pos.0, w - tw);
                let oy = y + off(pos.1, h - th);
                let (twi, thi) = (tile.width() as i32, tile.height() as i32);
                for py in y0..y1 {
                    let mut ty = py - oy.round() as i32;
                    if repeat.1 {
                        ty = ty.rem_euclid(thi);
                    } else if ty < 0 || ty >= thi {
                        continue;
                    }
                    for px in x0..x1 {
                        let mut tx = px - ox.round() as i32;
                        if repeat.0 {
                            tx = tx.rem_euclid(twi);
                        } else if tx < 0 || tx >= twi {
                            continue;
                        }
                        let c = cov(px, py);
                        if c > 0.0 {
                            let [r, g, b, a] = tile.get_pixel(tx as u32, ty as u32).0;
                            self.blend(px, py, Color { r, g, b, a }, c);
                        }
                    }
                }
            }
        }
    }

    fn fill_shadow(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, color: Color) {
        let pad = blur;
        let (x0, y0, x1, y1) = self.span(x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad);
        for py in y0..y1 {
            for px in x0..x1 {
                let d = rounded_dist(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, radius);
                let c = if blur > 0.0 {
                    // Smooth falloff across the blur distance, centered on the edge.
                    let t = (0.5 - d / blur).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                } else {
                    (0.5 - d).clamp(0.0, 1.0)
                };
                if c > 0.0 {
                    self.blend(px, py, color, c);
                }
            }
        }
    }

    fn stroke_ring(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32, color: Color) {
        let (x0, y0, x1, y1) = self.span(x, y, w, h);
        let inner_r = (radius - width).max(0.0);
        for py in y0..y1 {
            for px in x0..x1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let outer = rounded_cov(fx, fy, x, y, w, h, radius);
                if outer <= 0.0 {
                    continue;
                }
                let inner = rounded_cov(fx, fy, x + width, y + width, w - 2.0 * width, h - 2.0 * width, inner_r);
                let c = outer * (1.0 - inner);
                if c > 0.0 {
                    self.blend(px, py, color, c);
                }
            }
        }
    }

    /// Paint the display list with the page's top at `origin_y`, scrolled by `scroll`.
    /// Fixed-position items ignore the scroll.
    pub fn paint(&mut self, fonts: &Fonts, images: &Images, items: &[Item], origin_y: f32, scroll: f32) {
        self.clips.clear();
        self.paint_at(fonts, images, items, 0.0, origin_y, scroll);
        self.clips.clear();
    }

    /// Paint items with page coordinates shifted by (dx, origin_y - scroll).
    fn paint_at(&mut self, fonts: &Fonts, images: &Images, items: &[Item], dx: f32, origin_y: f32, scroll: f32) {
        let h = self.h as f32;
        let mut fixed_depth = 0;
        let mut dy = origin_y - scroll;
        let mut sticky: Vec<f32> = Vec::new();
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            i += 1;
            match item {
                Item::Mark { .. } | Item::ScrollArea { .. } | Item::TransformPop | Item::EffectPop => {}
                Item::EffectPush(fx) => {
                    let j = matching_pop(items, i);
                    let inner = &items[i..j.min(items.len())];
                    i = j + 1;
                    self.draw_effect(fonts, images, inner, fx, dx, dy);
                }
                Item::TransformPush { m, x, y, w, h: th } => {
                    // Find the matching pop.
                    let mut depth = 1;
                    let mut j = i;
                    while j < items.len() {
                        match items[j] {
                            Item::TransformPush { .. } => depth += 1,
                            Item::TransformPop => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    let inner = &items[i..j.min(items.len())];
                    i = j + 1;
                    self.draw_transformed(fonts, images, inner, m, (*x, *y, *w, *th), dx, dy);
                }
                Item::StickyPush { y, top, limit, .. } => {
                    let off = if fixed_depth == 0 { sticky_offset(*y, *top, *limit, scroll) } else { 0.0 };
                    sticky.push(off);
                    dy += off;
                }
                Item::StickyPop => {
                    dy -= sticky.pop().unwrap_or(0.0);
                }
                Item::FixedPush => {
                    fixed_depth += 1;
                    dy = origin_y;
                }
                Item::FixedPop => {
                    fixed_depth -= 1;
                    if fixed_depth == 0 {
                        dy = origin_y - scroll;
                    }
                }
                Item::ClipPush { x, y, w, h: ch, radius } => {
                    self.clips.push(Clip { x: x + dx, y: y + dy, w: *w, h: *ch, radius: *radius });
                }
                Item::ClipPop => {
                    self.clips.pop();
                }
                Item::Shadow { x, y, w, h: sh, radius, blur, color } => {
                    if y + dy + sh + blur >= 0.0 && y + dy - blur <= h {
                        self.fill_shadow(x + dx, y + dy, *w, *sh, *radius, *blur, *color);
                    }
                }
                Item::Rect { x, y, w, h: rh, color } => {
                    if y + dy + rh >= 0.0 && y + dy <= h {
                        self.fill_rect(x + dx, y + dy, *w, *rh, *color)
                    }
                }
                Item::Text { x, y, text, font, size, color } => {
                    self.draw_text(fonts, x + dx, y + dy, text, *font, *size, *color)
                }
                Item::Box { x, y, w, h: bh, radius, bg } => {
                    if y + dy + bh >= 0.0 && y + dy <= h {
                        self.fill_background(images, x + dx, y + dy, *w, *bh, *radius, bg);
                    }
                }
                Item::Ring { x, y, w, h: rh, radius, width, color } => {
                    if y + dy + rh >= 0.0 && y + dy <= h {
                        self.stroke_ring(x + dx, y + dy, *w, *rh, *radius, *width, *color);
                    }
                }
                Item::Image { x, y, w, h: ih, src, view } => {
                    if y + dy + ih >= 0.0 && y + dy <= h {
                        match view {
                            None => {
                                if let Some(img) = images.scaled(src, w.round() as u32, ih.round() as u32) {
                                    self.draw_image(&img, x + dx, y + dy);
                                }
                            }
                            Some(v) => {
                                if let Some(img) = images.scaled(src, v[2].round() as u32, v[3].round() as u32) {
                                    self.draw_image_clipped(&img, x + dx + v[0], y + dy + v[1], (x + dx, y + dy, *w, *ih));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Draw `items` (a transformed element) off-screen, then onto this canvas through `m`.
    /// The off-screen copy is drawn twice, over black and over white, which gives each
    /// pixel's color and transparency exactly.
    #[allow(clippy::too_many_arguments)]
    fn draw_transformed(&mut self, fonts: &Fonts, images: &Images, items: &[Item], m: &[f32; 6], area: (f32, f32, f32, f32), dx: f32, dy: f32) {
        use crate::transform;
        let (ax, ay) = (area.0.floor(), area.1.floor());
        let bw = ((area.2 + area.0 - ax).ceil() as usize + 1).min(4096);
        let bh = ((area.3 + area.1 - ay).ceil() as usize + 1).min(4096);
        if bw == 0 || bh == 0 {
            return;
        }
        // Where it lands on this canvas; skip if off-screen.
        let to_canvas = transform::mul(transform::translate(dx, dy), *m);
        let (qx, qy, qw, qh) = transform::bounds(&to_canvas, ax, ay, bw as f32, bh as f32);
        let (x0, y0) = (qx.floor().max(0.0) as i32, qy.floor().max(0.0) as i32);
        let (x1, y1) = ((qx + qw).ceil().min(self.w as f32) as i32, (qy + qh).ceil().min(self.h as f32) as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let Some(inv) = transform::invert(&to_canvas) else { return };
        let mut layers = [Canvas::new(bw, bh), Canvas::new(bw, bh)];
        layers[0].clear(Color::rgb(0, 0, 0));
        layers[1].clear(Color::rgb(255, 255, 255));
        for l in &mut layers {
            l.paint_at(fonts, images, items, -ax, -ay, 0.0);
            l.clips.clear();
        }
        let sample = |c: &Canvas, x: f32, y: f32| -> [f32; 3] {
            // Bilinear sampling; outside the copy counts as the background.
            let (fx, fy) = (x - 0.5, y - 0.5);
            let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
            let (tx, ty) = (fx - ix as f32, fy - iy as f32);
            let bg = c.buf[0];
            let px = |x: i32, y: i32| if x < 0 || y < 0 || x as usize >= c.w || y as usize >= c.h { bg } else { c.buf[y as usize * c.w + x as usize] };
            let (p00, p10, p01, p11) = (px(ix, iy), px(ix + 1, iy), px(ix, iy + 1), px(ix + 1, iy + 1));
            let ch = |p: u32, s: u32| ((p >> s) & 255) as f32;
            let mut out = [0.0; 3];
            for (k, sh) in [16u32, 8, 0].into_iter().enumerate() {
                let top = ch(p00, sh) * (1.0 - tx) + ch(p10, sh) * tx;
                let bot = ch(p01, sh) * (1.0 - tx) + ch(p11, sh) * tx;
                out[k] = top * (1.0 - ty) + bot * ty;
            }
            out
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let (sx, sy) = transform::apply(&inv, x as f32 + 0.5, y as f32 + 0.5);
                let (lx, ly) = (sx - ax, sy - ay);
                if lx < -1.0 || ly < -1.0 || lx > bw as f32 + 1.0 || ly > bh as f32 + 1.0 {
                    continue;
                }
                let b = sample(&layers[0], lx, ly);
                let w = sample(&layers[1], lx, ly);
                let alpha = 1.0 - ((w[0] - b[0]) + (w[1] - b[1]) + (w[2] - b[2])) / (3.0 * 255.0);
                if alpha <= 0.004 {
                    continue;
                }
                let un = |v: f32| (v / alpha).round().clamp(0.0, 255.0) as u8;
                self.blend(x, y, Color { r: un(b[0]), g: un(b[1]), b: un(b[2]), a: 255 }, alpha.min(1.0));
            }
        }
    }
}

pub fn pack(c: Color) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}
