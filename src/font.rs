//! Fonts: load system TrueType fonts, measure text and rasterize glyphs (cached).
//!
//! Glyph outlines are read from the font file only when a glyph is first drawn
//! (ab_glyph), so loading fonts costs about the size of the font files — not
//! the ~90 MB a pre-computing rasterizer needs for DejaVu's thousands of glyphs.

use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont as _};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const REGULAR: u8 = 0;
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const BOLD_ITALIC: u8 = 3;
pub const MONO: u8 = 4;
pub const MONO_BOLD: u8 = 5;
/// Ids from here on are web fonts.
pub const FIRST_WEB: u8 = 6;

thread_local! {
    /// The current page's @font-face families: name -> faces (weight range, italic, font id).
    static FAMILIES: RefCell<HashMap<String, Vec<(u16, u16, bool, u8)>>> = RefCell::new(HashMap::new());
}

pub fn set_web_families(map: HashMap<String, Vec<(u16, u16, bool, u8)>>) {
    FAMILIES.with(|f| *f.borrow_mut() = map);
    FAMILIES_GEN.with(|g| g.set(g.get().wrapping_add(1)));
}

thread_local! {
    /// Bumped when the page's web fonts change, so cached family choices are made again.
    static FAMILIES_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub fn take_web_families() -> HashMap<String, Vec<(u16, u16, bool, u8)>> {
    FAMILIES.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// Choose a font for a `font-family` list: the first named web font the page loaded,
/// else the first generic family (monospace…), else the default sans-serif.
pub fn resolve(family: Option<&str>, bold: bool, italic: bool, mono: bool) -> u8 {
    let fallback = Fonts::pick(bold, italic, mono);
    let Some(list) = family else { return fallback };
    FAMILIES.with(|fams| {
        let fams = fams.borrow();
        for name in list.split(',') {
            let name = name.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            if let Some(faces) = fams.get(name) {
                let want = if bold { 700 } else { 400 };
                // Prefer the right style, then the closest weight.
                let best = faces.iter().min_by_key(|(lo, hi, it, _)| {
                    let dist = if want < *lo { lo - want } else { want.saturating_sub(*hi) };
                    ((*it != italic) as u32) * 10_000 + dist as u32
                });
                if let Some(&(_, _, _, id)) = best {
                    return id;
                }
            }
            match name {
                "monospace" | "ui-monospace" => return Fonts::pick(bold, italic, true),
                "sans-serif" | "serif" | "system-ui" | "ui-sans-serif" | "-apple-system" | "blinkmacsystemfont" | "cursive" | "fantasy" => return Fonts::pick(bold, italic, false),
                _ => {}
            }
        }
        fallback
    })
}

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
    /// The bitmap is RGBA color (an emoji), not a coverage mask.
    pub color: bool,
}

/// Glyphs are drawn at quarter-pixel horizontal positions (subpixel positioning), so the
/// spacing between letters stays even instead of snapping each glyph to whole pixels.
pub const SUBPIXEL_STEPS: f32 = 4.0;

type Key = (u8, char, u16);

/// Fonts for characters the chosen font lacks (Chinese, Japanese, Korean, Hindi, Thai,
/// Hebrew…): file names searched for under the system font folders, in order of preference.
const FALLBACK_NAMES: &[&str] = &[
    "NotoColorEmoji.ttf",
    "NotoColorEmoji-Regular.ttf",
    "TwemojiMozilla.ttf",
    "NotoSans-Regular.ttf",
    "NotoSansArabic-Regular.ttf",
    "NotoSansHebrew-Regular.ttf",
    "NotoSansDevanagari-Regular.ttf",
    "NotoSansBengali-Regular.ttf",
    "NotoSansTamil-Regular.ttf",
    "NotoSansThai-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "FreeSerif.ttf",
    "FreeSerif.otf",
    "FreeSans.ttf",
    "NotoSansCJK-Regular.ttc",
    "NotoSansCJKsc-Regular.otf",
    "NotoSansCJKjp-Regular.otf",
    "wqy-zenhei.ttc",
    "wqy-microhei.ttc",
    "fonts-japanese-gothic.ttf",
];

/// Face ids: system styles are 0…, web fonts start here, fallback fonts at FALLBACK_BASE.
const WEB_BASE: u16 = 1000;
const FALLBACK_BASE: u16 = 2000;

/// One positioned glyph of shaped text (pixels).
#[derive(Clone, Copy, Debug)]
pub struct ShapedGlyph {
    pub face: u16,
    pub id: u16,
    pub x: f32,
    pub y: f32,
}

/// A run of text after shaping: glyphs in visual order, and its total advance.
#[derive(Debug, Default)]
pub struct Shaped {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
}

fn find_fallbacks() -> Vec<std::path::PathBuf> {
    let mut found: HashMap<String, std::path::PathBuf> = HashMap::new();
    let mut dirs: Vec<std::path::PathBuf> = vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(&home).join(".local/share/fonts"));
        dirs.push(std::path::PathBuf::from(&home).join(".fonts"));
    }
    // A bounded walk: font folders are shallow.
    let mut stack: Vec<(std::path::PathBuf, u8)> = dirs.into_iter().map(|d| (d, 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                if depth < 4 {
                    stack.push((path, depth + 1));
                }
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if FALLBACK_NAMES.contains(&name) {
                    found.entry(name.to_string()).or_insert(path);
                }
            }
        }
    }
    FALLBACK_NAMES.iter().filter_map(|n| found.remove(*n)).collect()
}

/// rustybuzz faces borrow the font bytes; the fonts they come from are never dropped
/// (the Fonts value lives for the whole program), so the borrow is extended to 'static.
fn buzz_face(font: &Rc<FontVec>, index: u32) -> Option<rustybuzz::Face<'static>> {
    let data: &[u8] = font.as_slice();
    // SAFETY: `font` is kept in Fonts (files, web or fallbacks) for the life of the program and
    // its byte buffer is never moved or freed, so the slice stays valid.
    let data: &'static [u8] = unsafe { std::mem::transmute::<&[u8], &'static [u8]>(data) };
    rustybuzz::Face::from_slice(data, index)
}

pub struct Fonts {
    /// Index into `files` for each style slot (missing faces reuse the regular one).
    slots: [usize; 6],
    files: Vec<Rc<FontVec>>,
    /// Web fonts added while browsing (ids 6 and up), each with the system style to use
    /// for characters it doesn't have.
    web: RefCell<Vec<(Rc<FontVec>, u8)>>,
    /// Face index inside the file for each `web` entry (font collections hold several).
    web_index: RefCell<Vec<u32>>,
    /// Installed fonts already loaded: (file, index) -> font id.
    system: RefCell<HashMap<(String, u32), u8>>,
    /// font-family list (+ bold, italic, mono) -> font id, for the current web fonts.
    resolved: RefCell<(u64, HashMap<(String, bool, bool, bool), u8>)>,
    advances: RefCell<HashMap<Key, f32>>,
    /// Fallback font files found on the system, loaded the first time they're needed.
    fallback_paths: Vec<std::path::PathBuf>,
    fallbacks: RefCell<Vec<Option<Option<Rc<FontVec>>>>>,
    /// Which fallback (face id) covers a character missing from the main fonts.
    char_fallback: RefCell<HashMap<char, Option<u16>>>,
    buzz: RefCell<HashMap<u16, Rc<rustybuzz::Face<'static>>>>,
    shaped: RefCell<HashMap<(u8, u16, String), Rc<Shaped>>>,
    glyphs_by_id: RefCell<HashMap<(u16, u16, u16, u8), (Metrics, Vec<u8>)>>,
    /// Glyph rasterizer with TrueType hinting and color glyph support (swash).
    scaler: RefCell<swash::scale::ScaleContext>,
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
        let mut files: Vec<Rc<FontVec>> = Vec::new();
        let mut slots = [0usize; 6];
        for (i, file) in FILES.iter().enumerate() {
            // IRONSWIFT_FONT_DIR (set by the AppImage, which carries its own DejaVu) comes first.
            let extra = std::env::var("IRONSWIFT_FONT_DIR").ok();
            let path = extra.iter().map(String::as_str).chain(DIRS.iter().copied()).map(|d| format!("{d}/{file}")).find(|p| std::path::Path::new(p).exists());
            match path {
                Some(p) => {
                    let bytes = std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?;
                    let font = FontVec::try_from_vec(bytes).map_err(|e| format!("{p}: {e}"))?;
                    slots[i] = files.len();
                    files.push(Rc::new(font));
                }
                None if i > 0 => slots[i] = slots[0],
                None => return Err("DejaVu Sans not found. Install it (e.g. `sudo apt install fonts-dejavu-core`).".into()),
            }
        }
        let fallback_paths = find_fallbacks();
        Ok(Fonts {
            slots,
            files,
            web: RefCell::default(),
            web_index: RefCell::default(),
            system: RefCell::default(),
            resolved: RefCell::default(),
            advances: RefCell::default(),
            fallbacks: RefCell::new(vec![None; fallback_paths.len()]),
            fallback_paths,
            char_fallback: RefCell::default(),
            buzz: RefCell::default(),
            shaped: RefCell::default(),
            glyphs_by_id: RefCell::default(),
            scaler: RefCell::new(swash::scale::ScaleContext::new()),
        })
    }

    /// The font file for an id; for a web font missing `c`, its system fallback.
    fn font_for(&self, font: u8, c: Option<char>) -> Rc<FontVec> {
        if (font as usize) < FIRST_WEB as usize {
            return self.files[self.slots[font as usize]].clone();
        }
        let web = self.web.borrow();
        match web.get((font - FIRST_WEB) as usize) {
            Some((f, fallback)) => {
                if c.is_some_and(|c| f.glyph_id(c).0 == 0) {
                    self.files[self.slots[*fallback as usize]].clone()
                } else {
                    f.clone()
                }
            }
            None => self.files[self.slots[0]].clone(),
        }
    }

    fn font(&self, font: u8) -> Rc<FontVec> {
        self.font_for(font, None)
    }

    /// Add a downloaded font (TrueType/OpenType data). Returns its id, or None if the data
    /// isn't a usable font or there's no room for more.
    pub fn add_web_font(&self, data: Vec<u8>, bold: bool, italic: bool) -> Option<u8> {
        let font = FontVec::try_from_vec(data).ok()?;
        let mut web = self.web.borrow_mut();
        if web.len() >= (255 - FIRST_WEB) as usize {
            return None;
        }
        web.push((Rc::new(font), Self::pick(bold, italic, false)));
        self.web_index.borrow_mut().push(0);
        Some(FIRST_WEB + web.len() as u8 - 1)
    }

    /// An installed font file (face `index` of a collection), loaded once.
    fn system_face(&self, face: &crate::sysfonts::Face, mono: bool) -> Option<u8> {
        let key = (face.path.clone(), face.index);
        if let Some(&id) = self.system.borrow().get(&key) {
            return Some(id);
        }
        let data = std::fs::read(&face.path).ok()?;
        let font = FontVec::try_from_vec_and_index(data, face.index).ok()?;
        let mut web = self.web.borrow_mut();
        if web.len() >= (255 - FIRST_WEB) as usize {
            return None;
        }
        web.push((Rc::new(font), Self::pick(face.weight >= 600, face.italic, mono)));
        self.web_index.borrow_mut().push(face.index);
        let id = FIRST_WEB + web.len() as u8 - 1;
        self.system.borrow_mut().insert(key, id);
        Some(id)
    }

    /// The font for a `font-family` list: the page's web fonts first, then fonts installed on
    /// the computer (by name, or a free look-alike for Arial, Times New Roman…), then the
    /// generic families; DejaVu Sans when nothing matches.
    pub fn resolve(&self, family: Option<&str>, bold: bool, italic: bool, mono: bool) -> u8 {
        let fallback = Self::pick(bold, italic, mono);
        let Some(list) = family else { return fallback };
        let generation = FAMILIES_GEN.with(|g| g.get());
        let key = (list.to_string(), bold, italic, mono);
        {
            let r = self.resolved.borrow();
            if r.0 == generation {
                if let Some(&id) = r.1.get(&key) {
                    return id;
                }
            }
        }
        let id = self.resolve_uncached(list, bold, italic, mono);
        let mut r = self.resolved.borrow_mut();
        if r.0 != generation || r.1.len() > 4096 {
            *r = (generation, HashMap::new());
        }
        r.1.insert(key, id);
        id
    }

    fn resolve_uncached(&self, list: &str, bold: bool, italic: bool, mono: bool) -> u8 {
        let idx = crate::sysfonts::index();
        let strict = crate::settings::get().resist_fingerprinting;
        for name in list.split(',') {
            let name = name.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            if FAMILIES.with(|f| f.borrow().contains_key(name)) {
                return resolve(Some(name), bold, italic, mono);
            }
            match name {
                // The built-in faces: the default sans-serif and monospace.
                "monospace" | "ui-monospace" => return Self::pick(bold, italic, true),
                "sans-serif" | "system-ui" | "ui-sans-serif" | "-apple-system" | "blinkmacsystemfont" | "dejavu sans" => return Self::pick(bold, italic, false),
                "dejavu sans mono" => return Self::pick(bold, italic, true),
                _ => {}
            }
            if let Some(fam) = crate::sysfonts::family(idx, name, strict) {
                let face = idx.families.get(&fam).and_then(|faces| crate::sysfonts::best_face(faces, bold, italic));
                if let Some(id) = face.and_then(|f| self.system_face(f, mono)) {
                    return id;
                }
            }
        }
        Self::pick(bold, italic, mono)
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
        let f = self.font_for(font, Some(c));
        let sf = f.as_scaled(scale_for(&f, size));
        let a = sf.h_advance(f.glyph_id(c));
        let mut cache = self.advances.borrow_mut();
        if cache.len() > 100_000 {
            cache.clear();
        }
        cache.insert(k, a);
        a
    }

    pub fn width(&self, font: u8, text: &str, size: f32) -> f32 {
        // Plain ASCII without kerning pairs is the common case; everything goes through the
        // shaper so kerning, ligatures and fallback fonts measure the way they draw.
        if !text.is_ascii() && text.contains(is_marker) {
            // Layout's internal markers (vertical.rs) take no room.
            let plain: String = text.chars().filter(|&c| !is_marker(c)).collect();
            return self.shape(font, &plain, size).width;
        }
        self.shape(font, text, size).width
    }

    pub fn ascent(&self, font: u8, size: f32) -> f32 {
        let f = self.font(font);
        f.as_scaled(scale_for(&f, size)).ascent()
    }

    /// How far the font reaches below the baseline (a positive distance).
    pub fn descent(&self, font: u8, size: f32) -> f32 {
        let f = self.font(font);
        -f.as_scaled(scale_for(&f, size)).descent()
    }
}

/// The Unicode noncharacters U+FDD0–U+FDEF, which layout uses as internal markers.
fn is_marker(c: char) -> bool {
    ('\u{FDD0}'..='\u{FDEF}').contains(&c)
}

impl Fonts {
    /// The font file behind a face id.
    fn face_file(&self, face: u16) -> Option<Rc<FontVec>> {
        if face >= FALLBACK_BASE {
            let i = (face - FALLBACK_BASE) as usize;
            let mut fb = self.fallbacks.borrow_mut();
            let slot = fb.get_mut(i)?;
            if slot.is_none() {
                let font = std::fs::read(&self.fallback_paths[i]).ok().and_then(|b| FontVec::try_from_vec_and_index(b, 0).ok()).map(Rc::new);
                *slot = Some(font);
            }
            slot.clone().flatten()
        } else if face >= WEB_BASE {
            self.web.borrow().get((face - WEB_BASE) as usize).map(|w| w.0.clone())
        } else {
            self.files.get(face as usize).cloned()
        }
    }

    /// The face a style id draws with first.
    fn primary_face(&self, font: u8) -> u16 {
        if font < FIRST_WEB {
            self.slots[font as usize] as u16
        } else if ((font - FIRST_WEB) as usize) < self.web.borrow().len() {
            WEB_BASE + (font - FIRST_WEB) as u16
        } else {
            self.slots[0] as u16
        }
    }

    fn has_glyph(&self, face: u16, c: char) -> bool {
        self.face_file(face).is_some_and(|f| f.glyph_id(c).0 != 0)
    }

    /// The face to draw `c` with: the font itself, its system style (for web fonts), then
    /// the fallback fonts.
    fn face_for(&self, font: u8, c: char) -> u16 {
        // Emoji are drawn in color from an emoji font when there is one, even where the page's
        // font has a black-and-white version.
        if is_emoji(c) {
            if let Some(&hit) = self.char_fallback.borrow().get(&c) {
                if let Some(h) = hit {
                    return h;
                }
            } else {
                let emoji_face = (0..self.fallback_paths.len())
                    .filter(|&i| self.fallback_paths[i].file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().contains("emoji")))
                    .map(|i| FALLBACK_BASE + i as u16)
                    .find(|&f| self.has_glyph(f, c));
                if let Some(f) = emoji_face {
                    self.char_fallback.borrow_mut().insert(c, Some(f));
                    return f;
                }
            }
        }
        let primary = self.primary_face(font);
        if self.has_glyph(primary, c) {
            return primary;
        }
        if font >= FIRST_WEB {
            let sys = self.web.borrow().get((font - FIRST_WEB) as usize).map_or(0, |w| w.1);
            let f = self.slots[sys as usize] as u16;
            if self.has_glyph(f, c) {
                return f;
            }
        }
        if let Some(&hit) = self.char_fallback.borrow().get(&c) {
            return hit.unwrap_or(primary);
        }
        let mut hit = None;
        for i in 0..self.fallback_paths.len() {
            let face = FALLBACK_BASE + i as u16;
            if self.has_glyph(face, c) {
                hit = Some(face);
                break;
            }
        }
        self.char_fallback.borrow_mut().insert(c, hit);
        hit.unwrap_or(primary)
    }

    fn buzz_for(&self, face: u16) -> Option<Rc<rustybuzz::Face<'static>>> {
        if let Some(f) = self.buzz.borrow().get(&face) {
            return Some(f.clone());
        }
        let file = self.face_file(face)?;
        let index = if (WEB_BASE..FALLBACK_BASE).contains(&face) { self.web_index.borrow().get((face - WEB_BASE) as usize).copied().unwrap_or(0) } else { 0 };
        let f = Rc::new(buzz_face(&file, index)?);
        self.buzz.borrow_mut().insert(face, f.clone());
        Some(f)
    }

    /// Shape `text` (one word or run) in a style: font fallback per character, then
    /// HarfBuzz-style shaping (kerning, ligatures, joining scripts) per run.
    pub fn shape(&self, font: u8, text: &str, size: f32) -> Rc<Shaped> {
        let key = (font, (size * 4.0).round() as u16, text.to_string());
        if let Some(s) = self.shaped.borrow().get(&key) {
            return s.clone();
        }
        let shaped = Rc::new(self.shape_uncached(font, text, size));
        let mut cache = self.shaped.borrow_mut();
        if cache.len() > 30_000 {
            cache.clear();
        }
        cache.insert(key, shaped.clone());
        shaped
    }

    fn shape_uncached(&self, font: u8, text: &str, size: f32) -> Shaped {
        // A right-to-left word with numbers or Latin in it ("ו-123", "مرحبا2024"): the
        // left-to-right pieces keep their own order, and the pieces go right to left.
        if text.chars().any(crate::style::is_rtl_char) && text.chars().any(|c| c.is_ascii_alphanumeric()) {
            let mut pieces: Vec<(bool, usize, usize)> = Vec::new();
            for (i, c) in text.char_indices() {
                let ltr = c.is_ascii_alphanumeric() || (c.is_alphabetic() && !crate::style::is_rtl_char(c));
                // Separators inside a number ("1,000.5") stay with it.
                let joins_number = matches!(c, '.' | ',' | ':' | '/') && pieces.last().is_some_and(|p| p.0)
                    && text[i + c.len_utf8()..].chars().next().is_some_and(|n| n.is_ascii_digit());
                let ltr = ltr || joins_number;
                match pieces.last_mut() {
                    Some(p) if p.0 == ltr => p.2 = i + c.len_utf8(),
                    _ => pieces.push((ltr, i, i + c.len_utf8())),
                }
            }
            let mut out = Shaped::default();
            for &(ltr, a, b) in pieces.iter().rev() {
                let part = self.shape_dir(font, &text[a..b], size, Some(!ltr));
                for g in &part.glyphs {
                    out.glyphs.push(ShapedGlyph { x: g.x + out.width, ..*g });
                }
                out.width += part.width;
            }
            return out;
        }
        self.shape_dir(font, text, size, None)
    }

    /// Shape with a fixed direction (Some(true) = right to left), or guess it from the text.
    fn shape_dir(&self, font: u8, text: &str, size: f32, rtl: Option<bool>) -> Shaped {
        // Split into runs that share a face. Spaces and combining marks stay with the run
        // they're in, so a mark is shaped together with its base letter.
        let mut runs: Vec<(u16, usize, usize)> = Vec::new();
        for (i, c) in text.char_indices() {
            let joins = c.is_whitespace() || is_mark(c) || c == '\u{200d}' || c == '\u{200c}';
            // U+FE0F after a symbol asks for its emoji (color) form.
            if text[i + c.len_utf8()..].starts_with('\u{FE0F}') && !is_emoji(c) {
                if let Some(f) = self.emoji_face_for(c) {
                    match runs.last_mut() {
                        Some(r) if r.0 == f => r.2 = i + c.len_utf8(),
                        _ => runs.push((f, i, i + c.len_utf8())),
                    }
                    continue;
                }
            }
            let face = match runs.last() {
                // A space after emoji goes back to the text font (emoji fonts have wide spaces).
                Some(&(f, _, _)) if c.is_whitespace() && self.is_emoji_face(f) => self.face_for(font, c),
                Some(&(f, _, _)) if joins => f,
                _ => self.face_for(font, c),
            };
            match runs.last_mut() {
                Some(r) if r.0 == face => r.2 = i + c.len_utf8(),
                _ => runs.push((face, i, i + c.len_utf8())),
            }
        }
        let mut out = Shaped::default();
        for (face, a, b) in runs {
            let piece = &text[a..b];
            let Some(bf) = self.buzz_for(face) else {
                // No shaping data: lay out with plain advances.
                for c in piece.chars() {
                    let adv = self.advance(font, c, size);
                    out.glyphs.push(ShapedGlyph { face, id: self.face_file(face).map_or(0, |f| f.glyph_id(c).0), x: out.width, y: 0.0 });
                    out.width += adv;
                }
                continue;
            };
            let mut buf = rustybuzz::UnicodeBuffer::new();
            buf.push_str(piece);
            buf.guess_segment_properties();
            if let Some(r) = rtl {
                buf.set_direction(if r { rustybuzz::Direction::RightToLeft } else { rustybuzz::Direction::LeftToRight });
            }
            let res = rustybuzz::shape(&bf, &[], buf);
            let scale = size / bf.units_per_em() as f32;
            for (info, pos) in res.glyph_infos().iter().zip(res.glyph_positions()) {
                let adv = pos.x_advance as f32 * scale;
                out.glyphs.push(ShapedGlyph {
                    face,
                    id: info.glyph_id as u16,
                    x: out.width + pos.x_offset as f32 * scale,
                    y: pos.y_offset as f32 * scale,
                });
                out.width += adv;
            }
        }
        out
    }

    /// Calls `f` with a glyph's metrics and bitmap (coverage, or RGBA for color glyphs), by
    /// face and glyph id, drawn `subpx` quarter-pixels to the right of a whole pixel.
    pub fn with_glyph_id<R>(&self, face: u16, id: u16, size: f32, subpx: u8, f: impl FnOnce(&Metrics, &[u8]) -> R) -> R {
        let k = (face, id, (size * 4.0).round() as u16, subpx);
        let mut cache = self.glyphs_by_id.borrow_mut();
        if !cache.contains_key(&k) {
            let entry = match self.face_file(face) {
                Some(fnt) => {
                    let index = if (WEB_BASE..FALLBACK_BASE).contains(&face) { self.web_index.borrow().get((face - WEB_BASE) as usize).copied().unwrap_or(0) } else { 0 };
                    self.rasterize_swash(&fnt, index, id, size, subpx as f32 / SUBPIXEL_STEPS)
                        .unwrap_or_else(|| rasterize_id(&fnt, ab_glyph::GlyphId(id), size))
                }
                None => (Metrics { xmin: 0, ymin: 0, width: 0, height: 0, color: false }, Vec::new()),
            };
            if cache.len() > 20_000 {
                cache.clear();
            }
            cache.insert(k, entry);
        }
        let e = &cache[&k];
        f(&e.0, &e.1)
    }
}

impl Fonts {
    /// Rasterize with swash: TrueType hinting (sharper stems on low-resolution screens),
    /// a horizontal subpixel offset, and color glyphs (emoji bitmaps and color outlines).
    fn rasterize_swash(&self, font: &FontVec, index: u32, id: u16, size: f32, dx: f32) -> Option<(Metrics, Vec<u8>)> {
        use swash::scale::{image::Content, Render, Source, StrikeWith};
        let data: &[u8] = font.as_slice();
        let fref = swash::FontRef::from_index(data, index as usize)?;
        let mut ctx = self.scaler.borrow_mut();
        // Hinting helps small text; large text keeps its true shapes.
        let mut scaler = ctx.builder(fref).size(size).hint(size <= 28.0).build();
        let img = Render::new(&[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline])
            .offset(swash::zeno::Vector::new(dx, 0.0))
            .render(&mut scaler, id)?;
        let p = img.placement;
        let (width, height) = (p.width as usize, p.height as usize);
        let color = img.content == Content::Color;
        let data = match img.content {
            Content::Mask => img.data,
            Content::Color => img.data,
            // A subpixel (RGB) mask isn't requested; use its green channel as coverage.
            Content::SubpixelMask => img.data.chunks(4).map(|c| c[1]).collect(),
        };
        Some((Metrics { xmin: p.left, ymin: p.top - height as i32, width, height, color }, data))
    }
}

impl Fonts {
    /// The color emoji font, if it has `c`.
    fn emoji_face_for(&self, c: char) -> Option<u16> {
        (0..self.fallback_paths.len()).map(|i| FALLBACK_BASE + i as u16).find(|&f| self.is_emoji_face(f) && self.has_glyph(f, c))
    }

    fn is_emoji_face(&self, face: u16) -> bool {
        face >= FALLBACK_BASE
            && self.fallback_paths.get((face - FALLBACK_BASE) as usize).and_then(|p| p.file_name()).and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().contains("emoji"))
    }
}

/// Characters drawn as emoji (in color) by default.
fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B50 | 0x2B55 | 0x231A..=0x231B | 0x23E9..=0x23FA)
        && !matches!(c as u32, 0x2600..=0x27BF if !EMOJI_PRESENTATION.contains(&(c as u32)))
}

/// Symbols in the 2600–27BF block that show as emoji even without U+FE0F.
const EMOJI_PRESENTATION: &[u32] = &[
    0x2614, 0x2615, 0x2648, 0x2649, 0x264A, 0x264B, 0x264C, 0x264D, 0x264E, 0x264F, 0x2650, 0x2651, 0x2652, 0x2653, 0x267F, 0x2693, 0x26A1,
    0x26AA, 0x26AB, 0x26BD, 0x26BE, 0x26C4, 0x26C5, 0x26CE, 0x26D4, 0x26EA, 0x26F2, 0x26F3, 0x26F5, 0x26FA, 0x26FD, 0x2705, 0x270A, 0x270B,
    0x2728, 0x274C, 0x274E, 0x2753, 0x2754, 0x2755, 0x2757, 0x2795, 0x2796, 0x2797, 0x27B0, 0x27BF,
];

/// Combining marks (accents, vowel signs) belong with the letter before them.
fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7..=0x06E8 | 0x06EA..=0x06ED
        | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963
        | 0x0981..=0x0983 | 0x09BC..=0x09D7 | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F)
}

fn rasterize_id(fnt: &FontVec, id: ab_glyph::GlyphId, size: f32) -> (Metrics, Vec<u8>) {
    let scale = scale_for(fnt, size);
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
            let ymin = -(b.min.y as i32) - height as i32;
            (Metrics { xmin: b.min.x as i32, ymin, width, height, color: false }, bmp)
        }
        None => (Metrics { xmin: 0, ymin: 0, width: 0, height: 0, color: false }, Vec::new()),
    }
}
