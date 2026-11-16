//! Fonts: load system TrueType fonts, measure text and rasterize glyphs (cached).
//!
//! Glyph outlines are read from the font file only when a glyph is first drawn
//! (ab_glyph), so loading fonts costs about the size of the font files — not
//! the ~90 MB a pre-computing rasterizer needs for DejaVu's thousands of glyphs.

use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont as _};
use std::cell::RefCell;
use std::collections::HashMap;

pub const REGULAR: u8 = 0;
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const BOLD_ITALIC: u8 = 3;
pub const MONO: u8 = 4;
pub const MONO_BOLD: u8 = 5;

const DIRS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu",
    "/usr/share/fonts/TTF",
    "/usr/share/fonts/dejavu",
    "/usr/share/fonts/dejavu-sans-fonts",
    "/usr/share/fonts/dejavu-sans-mono-fonts",
    "/usr/local/share/fonts",
];
const FILES: [&str; 6] = [
    "DejaVuSans.ttf",
    "DejaVuSans-Bold.ttf",
    "DejaVuSans-Oblique.ttf",
    "DejaVuSans-BoldOblique.ttf",
    "DejaVuSansMono.ttf",
    "DejaVuSansMono-Bold.ttf",
];

/// Where a rasterized glyph sits relative to the pen position and baseline.
/// (Same meaning as the fields painting already uses.)
pub struct Metrics {
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub advance_width: f32,
}

type Key = (u8, char, u16);

pub struct Fonts {
    /// Index into `files` for each style slot (missing faces reuse the regular one).
    slots: [usize; 6],
    files: Vec<FontVec>,
    glyphs: RefCell<HashMap<Key, (Metrics, Vec<u8>)>>,
    advances: RefCell<HashMap<Key, f32>>,
}

/// fontdue-compatible pixel size: ab_glyph scales by ascent-minus-descent height,
/// so convert from the em size CSS uses.
fn scale_for(font: &FontVec, size: f32) -> PxScale {
    let upem = font.units_per_em().unwrap_or(1000.0);
    let height = font.height_unscaled();
    PxScale::from(size * height / upem)
}

impl Fonts {
    pub fn load() -> Result<Fonts, String> {
        let mut files: Vec<FontVec> = Vec::new();
        let mut slots = [0usize; 6];
        for (i, file) in FILES.iter().enumerate() {
            let path = DIRS.iter().map(|d| format!("{d}/{file}")).find(|p| std::path::Path::new(p).exists());
            match path {
                Some(p) => {
                    let bytes = std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?;
                    let font = FontVec::try_from_vec(bytes).map_err(|e| format!("{p}: {e}"))?;
                    slots[i] = files.len();
                    files.push(font);
                }
                None if i > 0 => slots[i] = slots[0],
                None => return Err("DejaVu Sans not found. Install it (e.g. `sudo apt install fonts-dejavu-core`).".into()),
            }
        }
        Ok(Fonts { slots, files, glyphs: RefCell::default(), advances: RefCell::default() })
    }

    fn font(&self, font: u8) -> &FontVec {
        &self.files[self.slots[font as usize]]
    }

    pub fn pick(bold: bool, italic: bool, mono: bool) -> u8 {
        match (mono, bold, italic) {
            (true, true, _) => MONO_BOLD,
            (true, false, _) => MONO,
            (false, true, true) => BOLD_ITALIC,
            (false, true, false) => BOLD,
            (false, false, true) => ITALIC,
            _ => REGULAR,
        }
    }

    fn key(font: u8, c: char, size: f32) -> Key {
        (font, c, (size * 4.0).round() as u16)
    }

    pub fn advance(&self, font: u8, c: char, size: f32) -> f32 {
        let k = Self::key(font, c, size);
        if let Some(&a) = self.advances.borrow().get(&k) {
            return a;
        }
        let f = self.font(font);
        let sf = f.as_scaled(scale_for(f, size));
        let a = sf.h_advance(f.glyph_id(c));
        self.advances.borrow_mut().insert(k, a);
        a
    }

    pub fn width(&self, font: u8, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.advance(font, c, size)).sum()
    }

    pub fn ascent(&self, font: u8, size: f32) -> f32 {
        let f = self.font(font);
        f.as_scaled(scale_for(f, size)).ascent()
    }

    /// Calls `f` with the glyph's metrics and 8-bit coverage bitmap.
    pub fn with_glyph<R>(&self, font: u8, c: char, size: f32, f: impl FnOnce(&Metrics, &[u8]) -> R) -> R {
        let k = Self::key(font, c, size);
        let mut cache = self.glyphs.borrow_mut();
        if !cache.contains_key(&k) {
            let entry = self.rasterize(font, c, size);
            // Keep the cache bounded: text in many sizes and scripts can add up.
            if cache.len() > 20_000 {
                cache.clear();
            }
            cache.insert(k, entry);
        }
        let entry = &cache[&k];
        f(&entry.0, &entry.1)
    }

    fn rasterize(&self, font: u8, c: char, size: f32) -> (Metrics, Vec<u8>) {
        let fnt = self.font(font);
        let scale = scale_for(fnt, size);
        let sf = fnt.as_scaled(scale);
        let id = fnt.glyph_id(c);
        let advance_width = sf.h_advance(id);
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        match fnt.outline_glyph(glyph) {
            Some(og) => {
                let b = og.px_bounds();
                let (width, height) = (b.width() as usize, b.height() as usize);
                let mut bmp = vec![0u8; width * height];
                og.draw(|x, y, cov| {
                    let i = y as usize * width + x as usize;
                    if i < bmp.len() {
                        bmp[i] = (cov.clamp(0.0, 1.0) * 255.0) as u8;
                    }
                });
                // Painting places the bitmap's top at baseline - height - ymin.
                let ymin = -(b.min.y as i32) - height as i32;
                (Metrics { xmin: b.min.x as i32, ymin, width, height, advance_width }, bmp)
            }
            None => (Metrics { xmin: 0, ymin: 0, width: 0, height: 0, advance_width }, Vec::new()),
        }
    }
}
