//! Text properties beyond fonts, and splitting text into the pieces lines can break between:
//!
//! - `letter-spacing`, `word-spacing`, `white-space: nowrap`
//! - `word-break`, `overflow-wrap` / `word-wrap`, `hyphens` (soft hyphens, and automatic
//!   hyphenation for the page's language)
//! - `text-overflow: ellipsis`
//! - `text-decoration-style` (solid, double, dotted, dashed, wavy), `-thickness`, and
//!   `text-underline-offset`
//!
//! Line-break opportunities follow the Unicode line breaking algorithm (UAX #14), so text in
//! Chinese, Japanese and Thai-style scripts without spaces wraps, and lines may break after
//! hyphens and dashes.

/// Text settings kept on each element's style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    // Inherited:
    /// Extra space after every character (px).
    pub letter_spacing: f32,
    /// Extra space added to every space (px).
    pub word_spacing: f32,
    /// `white-space: nowrap` (or `pre`): lines don't wrap.
    pub nowrap: bool,
    pub word_break: WordBreak,
    /// `overflow-wrap: break-word | anywhere`: a word too long for the line may be split.
    pub break_long_words: bool,
    pub hyphens: Hyphens,
    // Not inherited:
    /// `text-overflow: ellipsis`
    pub ellipsis: bool,
    pub deco_style: DecoStyle,
    /// `text-decoration-thickness` in px (None = from the font size).
    pub deco_thickness: Option<f32>,
    /// `text-underline-offset` in px (None = the font's default).
    pub underline_offset: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordBreak {
    Normal,
    BreakAll,
    KeepAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hyphens {
    None,
    Manual,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecoStyle {
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            letter_spacing: 0.0,
            word_spacing: 0.0,
            nowrap: false,
            word_break: WordBreak::Normal,
            break_long_words: false,
            hyphens: Hyphens::Manual,
            ellipsis: false,
            deco_style: DecoStyle::Solid,
            deco_thickness: None,
            underline_offset: None,
        }
    }
}

impl TextStyle {
    /// What a child starts with: the inherited settings; decoration style comes along with
    /// the decoration itself (it's drawn across descendants).
    pub fn inherit(&self) -> TextStyle {
        TextStyle { ellipsis: false, ..*self }
    }
}

/// A length for letter/word spacing and decoration sizes: px, em, rem, `normal`/`auto`.
fn len_px(v: &str, font_size: f32) -> Option<f32> {
    let v = v.trim();
    if v == "normal" || v == "auto" || v == "from-font" {
        return Some(0.0);
    }
    let num = |s: &str| s.trim().parse::<f32>().ok();
    if let Some(n) = v.strip_suffix("px") {
        num(n)
    } else if let Some(n) = v.strip_suffix("rem") {
        num(n).map(|n| n * 16.0)
    } else if let Some(n) = v.strip_suffix("em") {
        num(n).map(|n| n * font_size)
    } else if let Some(n) = v.strip_suffix("pt") {
        num(n).map(|n| n * 4.0 / 3.0)
    } else if let Some(n) = v.strip_suffix('%') {
        num(n).map(|n| n * font_size / 100.0)
    } else {
        num(v)
    }
}

/// Apply one declaration if it's a text property handled here. Returns true if it was.
pub fn apply(t: &mut TextStyle, name: &str, lower: &str, font_size: f32) -> bool {
    match name {
        "letter-spacing" => {
            if let Some(v) = len_px(lower, font_size) {
                t.letter_spacing = v.clamp(-font_size, font_size * 4.0);
            }
        }
        "word-spacing" => {
            if let Some(v) = len_px(lower, font_size) {
                t.word_spacing = v.clamp(-font_size * 2.0, font_size * 8.0);
            }
        }
        "word-break" => {
            t.word_break = match lower {
                "break-all" => WordBreak::BreakAll,
                "keep-all" => WordBreak::KeepAll,
                // `break-word` (old) acts like overflow-wrap: anywhere.
                "break-word" => {
                    t.break_long_words = true;
                    WordBreak::Normal
                }
                _ => WordBreak::Normal,
            }
        }
        "overflow-wrap" | "word-wrap" => t.break_long_words = matches!(lower, "break-word" | "anywhere"),
        "hyphens" | "-webkit-hyphens" => {
            t.hyphens = match lower {
                "none" => Hyphens::None,
                "auto" => Hyphens::Auto,
                _ => Hyphens::Manual,
            }
        }
        "text-overflow" => t.ellipsis = lower.split_whitespace().last() == Some("ellipsis"),
        "text-decoration-style" => {
            if let Some(s) = deco_style(lower) {
                t.deco_style = s;
            }
        }
        "text-decoration-thickness" => t.deco_thickness = len_px(lower, font_size).filter(|v| *v > 0.0),
        "text-underline-offset" => t.underline_offset = if lower == "auto" { None } else { len_px(lower, font_size) },
        // The shorthand can carry a style and a thickness too ("underline wavy red 2px").
        "text-decoration" => {
            t.deco_style = lower.split_whitespace().find_map(deco_style).unwrap_or(DecoStyle::Solid);
            t.deco_thickness = lower.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit())).and_then(|w| len_px(w, font_size)).filter(|v| *v > 0.0);
            return false; // the line kinds and color are handled with the rest of the style
        }
        "white-space" | "white-space-collapse" | "text-wrap" | "text-wrap-mode" => {
            t.nowrap = matches!(lower, "nowrap" | "pre");
            return name == "text-wrap" || name == "text-wrap-mode";
        }
        _ => return false,
    }
    true
}

fn deco_style(w: &str) -> Option<DecoStyle> {
    Some(match w {
        "solid" => DecoStyle::Solid,
        "double" => DecoStyle::Double,
        "dotted" => DecoStyle::Dotted,
        "dashed" => DecoStyle::Dashed,
        "wavy" => DecoStyle::Wavy,
        _ => return None,
    })
}

// ---------------------------------------------------------------- decoration lines

/// Draw one text-decoration line (underline, overline or line-through) of thickness `th`,
/// `y` being its top, as rectangles in the display list.
pub fn deco_line(items: &mut Vec<crate::paint::Item>, x: f32, y: f32, w: f32, th: f32, color: crate::style::Color, style: DecoStyle) {
    use crate::paint::Item;
    if w <= 0.0 {
        return;
    }
    let th = th.max(1.0);
    match style {
        DecoStyle::Solid => items.push(Item::Rect { x, y, w, h: th, color }),
        DecoStyle::Double => {
            items.push(Item::Rect { x, y, w, h: th, color });
            items.push(Item::Rect { x, y: y + th * 2.0, w, h: th, color });
        }
        DecoStyle::Dotted | DecoStyle::Dashed => {
            // Dots as long as the line is thick; dashes three times that; gaps alike.
            let (on, off) = if style == DecoStyle::Dotted { (th, th) } else { (th * 3.0, th * 2.0) };
            let mut cx = x;
            while cx < x + w {
                items.push(Item::Rect { x: cx, y, w: on.min(x + w - cx), h: th, color });
                cx += on + off;
            }
        }
        DecoStyle::Wavy => {
            // A sine wave, one pixel column at a time.
            let amp = (th * 1.2).max(1.0);
            let period = (th * 6.0).max(6.0);
            let mut cx = 0.0;
            while cx < w {
                let dy = amp * (std::f32::consts::TAU * cx / period).sin();
                items.push(Item::Rect { x: x + cx, y: y + amp + dy, w: 1.0, h: th, color });
                cx += 1.0;
            }
        }
    }
}

// ---------------------------------------------------------------- where lines may break

const SHY: char = '\u{AD}';

/// One piece of a word: lines may break after it. `hyphen`: a line that breaks here shows a
/// hyphen (a soft hyphen, or automatic hyphenation).
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub text: String,
    pub hyphen: bool,
}

/// Split a word (no spaces in it) where lines may break, by UAX #14 and the text settings.
/// `lang`: the element's language, for automatic hyphenation.
pub fn pieces(word: &str, t: &TextStyle, lang: Option<&str>) -> Vec<Piece> {
    if t.nowrap {
        return vec![Piece { text: word.replace(SHY, ""), hyphen: false }];
    }
    let mut out: Vec<Piece> = Vec::new();
    // Break opportunities inside the word (a mandatory break can't occur: no newlines here).
    let mut cuts: Vec<usize> = unicode_linebreak::linebreaks(word).map(|(i, _)| i).filter(|&i| i < word.len()).collect();
    if t.word_break == WordBreak::KeepAll {
        // Only where a non-CJK rule allows it: drop breaks between two ideographs/kana/hangul.
        cuts.retain(|&i| {
            let before = word[..i].chars().next_back();
            let after = word[i..].chars().next();
            !(before.is_some_and(is_cjk) && after.is_some_and(is_cjk))
        });
    }
    if t.word_break == WordBreak::BreakAll {
        // Between any two letters (grapheme clusters, keeping marks with their letter).
        cuts = clusters(word).into_iter().skip(1).collect();
    }
    if t.hyphens == Hyphens::None {
        // Soft hyphens aren't break points then.
        cuts.retain(|&i| !word[..i].ends_with(SHY));
    }
    let mut start = 0;
    for c in cuts.into_iter().chain(std::iter::once(word.len())) {
        if c <= start {
            continue;
        }
        let seg = &word[start..c];
        start = c;
        let parts: Vec<&str> = seg.split(SHY).collect();
        if t.hyphens == Hyphens::None || parts.len() == 1 {
            out.push(Piece { text: parts.concat(), hyphen: false });
            continue;
        }
        // A soft hyphen: break there with a visible hyphen.
        let last = parts.len() - 1;
        for (k, p) in parts.iter().enumerate() {
            if p.is_empty() {
                if k > 0 {
                    if let Some(prev) = out.last_mut() {
                        prev.hyphen = true;
                    }
                }
                continue;
            }
            out.push(Piece { text: p.to_string(), hyphen: k < last });
        }
    }
    if t.hyphens == Hyphens::Auto && !word.contains(SHY) {
        out = out.into_iter().flat_map(|p| hyphenate(p, lang)).collect();
    }
    out.retain(|p| !p.text.is_empty());
    out
}

/// Automatic hyphenation of one piece (words of five letters or more, by language).
fn hyphenate(p: Piece, lang: Option<&str>) -> Vec<Piece> {
    let Some(language) = lang.and_then(hyph_lang) else { return vec![p] };
    // Only plain words: leave numbers, URLs and words with punctuation inside alone.
    let letters: usize = p.text.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 5 || p.text.chars().any(|c| !c.is_alphabetic() && !matches!(c, '-' | '’' | '\'')) {
        return vec![p];
    }
    let syl: Vec<&str> = hypher::hyphenate(&p.text, language).collect();
    if syl.len() < 2 {
        return vec![p];
    }
    let n = syl.len();
    syl.into_iter().enumerate().map(|(i, s)| Piece { text: s.to_string(), hyphen: i + 1 < n || p.hyphen }).collect()
}

fn hyph_lang(lang: &str) -> Option<hypher::Lang> {
    let code = lang.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    let mut chars = code.chars();
    let bytes = [chars.next()? as u8, chars.next()? as u8];
    hypher::Lang::from_iso(bytes)
}

/// Start of each grapheme cluster (a letter and the marks on it).
fn clusters(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, c) in s.char_indices() {
        let mark = matches!(c as u32, 0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05C7 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200C | 0x200D | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F);
        let after_zwj = s[..i].ends_with('\u{200D}');
        if !mark && !after_zwj {
            out.push(i);
        }
    }
    out
}

/// The letters (grapheme clusters) of a text, for drawing letter-spaced text one at a time.
pub fn letters(s: &str) -> Vec<&str> {
    let starts = clusters(s);
    starts.iter().enumerate().map(|(k, &a)| &s[a..starts.get(k + 1).copied().unwrap_or(s.len())]).collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF | 0x20000..=0x2FFFF)
}

/// Split a piece that is wider than `max` into parts that fit (for overflow-wrap). `width`
/// measures text.
pub fn split_to_fit(text: &str, max: f32, width: &dyn Fn(&str) -> f32) -> Vec<String> {
    let starts = clusters(text);
    let mut out = Vec::new();
    let mut from = 0;
    let mut k = 1;
    while from < text.len() {
        // Grow the part one cluster at a time while it fits (at least one cluster per part).
        let mut end = starts.get(k).copied().unwrap_or(text.len());
        while k < starts.len() {
            let next = starts.get(k + 1).copied().unwrap_or(text.len());
            if width(&text[from..next]) > max {
                break;
            }
            end = next;
            k += 1;
        }
        out.push(text[from..end].to_string());
        from = end;
        k += 1;
    }
    out
}

/// The longest start of `text` that fits in `max` with "…" after it.
pub fn ellipsize(text: &str, max: f32, width: &dyn Fn(&str) -> f32) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let starts = clusters(text);
    let mut best = String::from("…");
    for &e in starts.iter().skip(1).chain(std::iter::once(&text.len())) {
        let cand = format!("{}…", text[..e].trim_end());
        if width(&cand) > max {
            break;
        }
        best = cand;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &[Piece]) -> Vec<&str> {
        v.iter().map(|p| p.text.as_str()).collect()
    }

    #[test]
    fn break_opportunities() {
        let t = TextStyle::default();
        assert_eq!(texts(&pieces("hello", &t, None)), ["hello"]);
        assert_eq!(texts(&pieces("well-known", &t, None)), ["well-", "known"]);
        // Chinese and Japanese break between characters.
        assert_eq!(texts(&pieces("中文字", &t, None)), ["中", "文", "字"]);
        // …but not before closing punctuation.
        assert_eq!(texts(&pieces("中文。", &t, None)), ["中", "文。"]);
        let keep = TextStyle { word_break: WordBreak::KeepAll, ..t };
        assert_eq!(texts(&pieces("中文字", &keep, None)), ["中文字"]);
        let all = TextStyle { word_break: WordBreak::BreakAll, ..t };
        assert_eq!(texts(&pieces("abc", &all, None)), ["a", "b", "c"]);
        assert_eq!(texts(&pieces("e\u{301}x", &all, None)), ["e\u{301}", "x"], "accents stay with their letter");
        let nowrap = TextStyle { nowrap: true, ..t };
        assert_eq!(texts(&pieces("well-known", &nowrap, None)), ["well-known"]);
    }

    #[test]
    fn soft_and_auto_hyphens() {
        let t = TextStyle::default();
        let p = pieces("hyph\u{AD}en\u{AD}ate", &t, None);
        assert_eq!(texts(&p), ["hyph", "en", "ate"]);
        assert_eq!(p.iter().map(|p| p.hyphen).collect::<Vec<_>>(), [true, true, false]);
        let none = TextStyle { hyphens: Hyphens::None, ..t };
        assert_eq!(texts(&pieces("hyph\u{AD}en", &none, None)), ["hyphen"]);
        let auto = TextStyle { hyphens: Hyphens::Auto, ..t };
        let p = pieces("extraordinary", &auto, Some("en-US"));
        assert!(p.len() >= 3, "{p:?}");
        assert!(p[0].hyphen && !p.last().unwrap().hyphen);
        assert_eq!(p.iter().map(|p| p.text.as_str()).collect::<String>(), "extraordinary");
        assert_eq!(pieces("extraordinary", &auto, None).len(), 1, "no language: no hyphenation");
        assert_eq!(pieces("Silbentrennung", &auto, Some("de")).len() > 2, true);
    }

    #[test]
    fn properties() {
        let mut t = TextStyle::default();
        assert!(apply(&mut t, "letter-spacing", "0.1em", 20.0));
        assert!((t.letter_spacing - 2.0).abs() < 1e-4);
        assert!(apply(&mut t, "word-spacing", "4px", 20.0));
        assert_eq!(t.word_spacing, 4.0);
        apply(&mut t, "text-overflow", "ellipsis", 16.0);
        assert!(t.ellipsis);
        assert!(!apply(&mut t, "text-decoration", "underline wavy red 2px", 16.0));
        assert_eq!((t.deco_style, t.deco_thickness), (DecoStyle::Wavy, Some(2.0)));
        apply(&mut t, "white-space", "nowrap", 16.0);
        assert!(t.nowrap && !t.inherit().ellipsis);
    }

    #[test]
    fn fitting() {
        let w = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(split_to_fit("abcdefgh", 30.0, &w), ["abc", "def", "gh"]);
        assert_eq!(split_to_fit("ab", 5.0, &w), ["a", "b"], "at least one letter per part");
        assert_eq!(ellipsize("Hello world", 60.0, &w), "Hello…");
        assert_eq!(ellipsize("Hi", 60.0, &w), "Hi");
    }
}
