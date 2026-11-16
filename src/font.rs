//! Fonts: load system TrueType fonts, measure text and rasterize glyphs (cached).

use fontdue::{Font, FontSettings, Metrics};
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

type Key = (u8, char, u16);

pub struct Fonts {
    fonts: Vec<Font>,
    glyphs: RefCell<HashMap<Key, (Metrics, Vec<u8>)>>,
    advances: RefCell<HashMap<Key, f32>>,
}

impl Fonts {
    pub fn load() -> Result<Fonts, String> {
        let mut fonts: Vec<Font> = Vec::new();
        for (i, file) in FILES.iter().enumerate() {
            let path = DIRS.iter().map(|d| format!("{d}/{file}")).find(|p| std::path::Path::new(p).exists());
            let font = match path {
                Some(p) => {
                    let bytes = std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?;
                    Font::from_bytes(bytes, FontSettings::default()).map_err(|e| format!("{p}: {e}"))?
                }
                // Missing variant: fall back to the regular face.
                None if i > 0 => fonts[0usize].clone(),
                None => return Err("DejaVu Sans not found. Install it (e.g. `sudo apt install fonts-dejavu-core`).".into()),
            };
            fonts.push(font);
        }
        Ok(Fonts { fonts, glyphs: RefCell::default(), advances: RefCell::default() })
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
        let a = self.fonts[font as usize].metrics(c, size).advance_width;
        self.advances.borrow_mut().insert(k, a);
        a
    }

    pub fn width(&self, font: u8, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.advance(font, c, size)).sum()
    }

    pub fn ascent(&self, font: u8, size: f32) -> f32 {
        self.fonts[font as usize].horizontal_line_metrics(size).map(|m| m.ascent).unwrap_or(size * 0.8)
    }

    /// Calls `f` with the glyph's metrics and 8-bit coverage bitmap.
    pub fn with_glyph<R>(&self, font: u8, c: char, size: f32, f: impl FnOnce(&Metrics, &[u8]) -> R) -> R {
        let k = Self::key(font, c, size);
        let mut cache = self.glyphs.borrow_mut();
        let entry = cache.entry(k).or_insert_with(|| self.fonts[font as usize].rasterize(c, size));
        f(&entry.0, &entry.1)
    }
}
