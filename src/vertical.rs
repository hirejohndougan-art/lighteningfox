//! Vertical text (CSS Writing Modes 4): `writing-mode: vertical-rl | vertical-lr`,
//! `text-orientation` and `text-combine-upright`.
//!
//! A box whose writing mode turns vertical inside horizontal content (a "vertical root") is
//! laid out by the ordinary horizontal code in *logical* coordinates: its inline size (the
//! line length) is its physical height, and its lines stack downward. [`convert`] then moves
//! what that produced to where it really goes:
//!
//! - lines become columns: right to left for `vertical-rl`, left to right for `vertical-lr`;
//! - boxes, borders, backgrounds and element rectangles turn a quarter turn (exactly, so
//!   nothing is resampled);
//! - characters that stand upright in vertical text (CJK and other "upright" scripts under
//!   `text-orientation: mixed`, everything under `upright`, combined runs) are placed one by
//!   one, centred in their column;
//! - other text (Latin words and the like) is drawn turned 90° clockwise, through the same
//!   off-screen transform `transform: rotate()` uses, one group per run of such text.

use crate::font::Fonts;
use crate::paint::Item;

/// The writing-mode properties of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vertical {
    /// writing-mode: 0 horizontal-tb, 1 vertical-rl (and sideways-rl), 2 vertical-lr
    /// (and sideways-lr).
    pub mode: u8,
    /// text-orientation: 0 mixed, 1 upright, 2 sideways (sideways-* writing modes force 2).
    pub orient: u8,
    /// text-combine-upright: all (its text sits upright in a single 1em cell).
    pub combine: bool,
}

/// Put in front of a combined (`text-combine-upright`) run's text item, so [`convert`] knows
/// to squeeze it into one upright cell. A Unicode noncharacter: never in real text, and
/// `Fonts::width` gives these no width.
pub const COMBINE_MARK: char = '\u{FDD0}';

/// Put in front of other text items in vertical text: the text's own `text-orientation`
/// (this plus 0 mixed, 1 upright, 2 sideways), which may differ from the vertical root's.
pub const ORIENT_MARK: u32 = 0xFDD1;

/// Marks (never painted) around the items of a horizontal box inside vertical text: laid
/// out the right way up already, [`convert`] only moves them. The opening one holds the
/// box's place in the vertical root's logical layout.
pub const FENCE_OPEN: usize = usize::MAX;
pub const FENCE_CLOSE: usize = usize::MAX - 1;

/// The marker a text item in vertical text starts with (None in horizontal text).
pub fn mark(v: Vertical) -> Option<char> {
    if v.combines() {
        Some(COMBINE_MARK)
    } else if v.is_vertical() {
        char::from_u32(ORIENT_MARK + v.orient.min(2) as u32)
    } else {
        None
    }
}

impl Vertical {
    pub fn is_vertical(&self) -> bool {
        self.mode != 0
    }

    /// What a child inherits: all three properties inherit.
    pub fn inherit(&self) -> Self {
        *self
    }

    /// Whether a combined run's text should be marked (it only applies in vertical text).
    pub fn combines(&self) -> bool {
        self.combine && self.is_vertical()
    }

    /// Reads one of the three properties; false if `name` isn't one of them.
    pub fn apply(&mut self, name: &str, value: &str) -> bool {
        let v = value.trim();
        match name {
            "writing-mode" | "-webkit-writing-mode" => {
                match v {
                    "horizontal-tb" | "lr" | "lr-tb" | "rl" | "rl-tb" => {
                        self.mode = 0;
                    }
                    "vertical-rl" | "tb-rl" | "tb" => self.mode = 1,
                    "vertical-lr" | "tb-lr" => self.mode = 2,
                    "sideways-rl" => {
                        self.mode = 1;
                        self.orient = 2;
                    }
                    "sideways-lr" => {
                        self.mode = 2;
                        self.orient = 2;
                    }
                    _ => {}
                }
                true
            }
            "text-orientation" | "-webkit-text-orientation" => {
                match v {
                    "mixed" => self.orient = 0,
                    "upright" => self.orient = 1,
                    "sideways" | "sideways-right" => self.orient = 2,
                    _ => {}
                }
                true
            }
            "text-combine-upright" | "-webkit-text-combine" => {
                self.combine = matches!(v, "all" | "horizontal");
                true
            }
            _ => false,
        }
    }

    /// Computed values, for getComputedStyle.
    pub fn computed(&self) -> [(String, String); 3] {
        [
            ("writing-mode".into(), ["horizontal-tb", "vertical-rl", "vertical-lr"][self.mode.min(2) as usize].into()),
            ("text-orientation".into(), ["mixed", "upright", "sideways"][self.orient.min(2) as usize].into()),
            ("text-combine-upright".into(), if self.combine { "all" } else { "none" }.into()),
        ]
    }

    /// Whether character `c` stands upright in this box's vertical text.
    pub fn upright(&self, c: char) -> bool {
        match self.orient {
            1 => !c.is_whitespace(),
            2 => false,
            _ => upright_in_mixed(c),
        }
    }
}

/// Characters drawn upright under `text-orientation: mixed`: the ranges Unicode's Vertical
/// Orientation property (UAX #50) marks U or Tu, approximated by block.
pub fn upright_in_mixed(c: char) -> bool {
    matches!(c as u32,
        0x00A7 | 0x00A9 | 0x00AE | 0x00B1 | 0x00BC..=0x00BE | 0x00D7 | 0x00F7
        | 0x02EA..=0x02EB
        | 0x1100..=0x11FF            // Hangul Jamo
        | 0x1401..=0x167F            // Canadian syllabics
        | 0x18B0..=0x18FF
        | 0x2016 | 0x2020..=0x2021 | 0x2030..=0x2031 | 0x203B..=0x203C | 0x2042 | 0x2047..=0x2049 | 0x2051
        | 0x2100..=0x214F            // letterlike symbols
        | 0x2150..=0x218F            // number forms
        | 0x221E | 0x2234..=0x2235
        | 0x2300..=0x2307 | 0x230C..=0x231F | 0x2324..=0x2328 | 0x232B | 0x237D..=0x239A | 0x23BE..=0x23CD | 0x23CF | 0x23D1..=0x23DB | 0x23E2..=0x23FF
        | 0x2400..=0x245F            // control pictures, OCR
        | 0x2460..=0x24FF            // enclosed alphanumerics
        | 0x25A0..=0x25FF            // geometric shapes
        | 0x2600..=0x2619 | 0x2620..=0x2767 | 0x2776..=0x2793
        | 0x2B12..=0x2B2F | 0x2B50..=0x2B59 | 0x2BB8..=0x2BFF
        | 0x2E80..=0x2FFF            // CJK radicals, Kangxi, description characters
        | 0x3000..=0x3007            // CJK space, 、。〃 etc. (brackets 3008–3011 turn)
        | 0x3012..=0x3013 | 0x3020..=0x302F | 0x3031..=0x303F
        | 0x3040..=0x309F            // Hiragana
        | 0x30A0..=0x30FB | 0x30FD..=0x30FF // Katakana (ー 30FC turns)
        | 0x3100..=0x312F | 0x3130..=0x318F | 0x3190..=0x31FF
        | 0x3200..=0x33FF            // enclosed CJK, compatibility
        | 0x3400..=0x4DBF            // CJK extension A
        | 0x4DC0..=0x4DFF
        | 0x4E00..=0x9FFF            // CJK unified ideographs
        | 0xA000..=0xA4CF            // Yi
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7FF            // Hangul syllables
        | 0xE000..=0xF8FF            // private use
        | 0xF900..=0xFAFF            // CJK compatibility ideographs
        | 0xFE10..=0xFE1F | 0xFE30..=0xFE48 | 0xFE50..=0xFE57 | 0xFE5F..=0xFE62 | 0xFE67..=0xFE6F
        | 0xFF01..=0xFF07 | 0xFF0A | 0xFF0C | 0xFF0E..=0xFF19 | 0xFF1F..=0xFF3A | 0xFF3C | 0xFF3E | 0xFF40..=0xFF5A
        | 0xFFE0..=0xFFE2 | 0xFFE4..=0xFFE7
        | 0x1F000..=0x1FAFF          // mahjong, cards, emoji and pictographs
        | 0x20000..=0x3FFFD          // CJK extensions B on
    )
}

/// The vertical presentation form of punctuation that stays upright but sits differently
/// in a column (the ideographic comma and full stop go to the top right of their cell).
/// Brackets and the long vowel mark turn with sideways text instead.
pub fn vertical_form(c: char) -> char {
    match c {
        '、' => '\u{FE11}',
        '。' => '\u{FE12}',
        '，' => '\u{FE10}',
        '：' => '\u{FE13}',
        '；' => '\u{FE14}',
        '！' => '\u{FE15}',
        '？' => '\u{FE16}',
        '…' => '\u{FE19}',
        '‥' => '\u{FE30}',
        c => c,
    }
}

/// The quarter turn from a vertical root's logical layout to the page.
#[derive(Debug, Clone, Copy)]
pub struct Turn {
    /// writing-mode (1 rl, 2 lr).
    pub mode: u8,
    /// Content box origin: where logical (0, 0) and the physical box both start.
    pub cx: f32,
    pub cy: f32,
    /// Physical content width (the logical block size the columns fill).
    pub w: f32,
}

impl Turn {
    /// A logical point (page coordinates) on the page.
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        let (lx, ly) = (x - self.cx, y - self.cy);
        match self.mode {
            2 => (self.cx + ly, self.cy + lx),
            _ => (self.cx + self.w - ly, self.cy + lx),
        }
    }

    /// A logical rectangle on the page.
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let (lx, ly) = (x - self.cx, y - self.cy);
        match self.mode {
            2 => (self.cx + ly, self.cy + lx, h, w),
            _ => (self.cx + self.w - ly - h, self.cy + lx, h, w),
        }
    }

    /// The map as a matrix [a b c d e f] (px = a·x + c·y + e, py = b·x + d·y + f).
    fn matrix(&self) -> [f32; 6] {
        match self.mode {
            2 => [0.0, 1.0, 1.0, 0.0, self.cx - self.cy, self.cy - self.cx],
            _ => [0.0, 1.0, -1.0, 0.0, self.cx + self.w + self.cy, self.cy - self.cx],
        }
    }

    /// The quarter turn clockwise (what sideways text is drawn through): the map itself for
    /// vertical-rl; for vertical-lr, where the map is a mirror image, the clockwise turn
    /// that puts a line's centre where the map does once the line is mirrored (see `mirror`).
    fn rotation(&self) -> [f32; 6] {
        Turn { mode: 1, ..*self }.matrix()
    }

    /// For vertical-lr: where a logical baseline goes so that, drawn through `rotation`, its
    /// line's centre (`mid` above the baseline) lands where `point` puts it.
    fn mirror(&self, y: f32, mid: f32) -> f32 {
        match self.mode {
            2 => self.cy + self.w - (y - self.cy) + 2.0 * mid,
            _ => y,
        }
    }
}

/// Whether a transform is the exact quarter turn sideways text in vertical writing is drawn
/// through (CSS `rotate(90deg)` never comes out exact in floating point).
pub fn is_text_turn(m: &[f32; 6]) -> bool {
    m[0] == 0.0 && m[3] == 0.0 && m[1] == 1.0 && m[2] == -1.0
}

/// How far a piece of vertical text reaches down its column: a full em per upright
/// character, the horizontal advance for sideways ones. Layout and [`convert`] both use it.
pub fn advance(fonts: &Fonts, v: Vertical, font: u8, size: f32, text: &str) -> f32 {
    text.chars().map(|c| if v.upright(c) { size } else { fonts.advance(font, c, size) }).sum()
}

/// a·b for 2D affine matrices.
fn mul(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    crate::transform::mul(a, b)
}

/// The inverse of a quarter turn (or mirror): both are their own transposes in the linear part.
fn invert(m: [f32; 6]) -> [f32; 6] {
    let [a, b, c, d, e, f] = m;
    let det = a * d - b * c;
    let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
    [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]
}

/// Vertical metrics used to centre glyphs in a column: the em box's middle sits `mid`
/// above the alphabetic baseline.
fn mid(fonts: &Fonts, font: u8, size: f32) -> f32 {
    (fonts.ascent(font, size) - fonts.descent(font, size)) / 2.0
}

/// One upright glyph run (a character, or a combined run) centred at physical (px, py).
fn upright(out: &mut Vec<Item>, fonts: &Fonts, text: &str, font: u8, size: f32, color: crate::style::Color, px: f32, py: f32, squeeze: bool) {
    let mut size = size;
    let mut tw = fonts.width(font, text, size);
    if squeeze && tw > size * 0.95 && tw > 0.0 {
        // A combined run squeezed into one em.
        let s = size * size * 0.95 / tw;
        size = s;
        tw = fonts.width(font, text, size);
    }
    let m = mid(fonts, font, size);
    out.push(Item::Text { x: px - tw / 2.0, y: py + m, text: text.to_string(), font, size, color });
}

/// Moves the display items from `from` on, laid out in logical coordinates, to the page.
/// `v` is the vertical root's writing mode; its `text-orientation` applies to all the text
/// inside (a descendant's own text-orientation isn't looked at yet).
/// While `measuring` (intrinsic sizes), sideways text becomes an empty box where it will be,
/// so size measurements see it in its column rather than at its unturned place.
pub fn convert(items: &mut Vec<Item>, from: usize, turn: Turn, v: Vertical, fonts: &Fonts, measuring: bool) {
    let tail: Vec<Item> = items.drain(from..).collect();
    let r = turn.matrix();
    let rot = turn.rotation();
    // The open group of sideways text (index of its TransformPush in `items`, and its area).
    let mut group: Option<(usize, (f32, f32, f32, f32))> = None;
    let close = |items: &mut Vec<Item>, group: &mut Option<(usize, (f32, f32, f32, f32))>| {
        if let Some((at, (x0, y0, x1, y1))) = group.take() {
            if let Item::TransformPush { x, y, w, h, .. } = &mut items[at] {
                *x = x0;
                *y = y0;
                *w = x1 - x0;
                *h = y1 - y0;
            }
            items.push(Item::TransformPop);
        }
    };
    // Inside a fence: how deep, and how far its items move.
    let mut fence: (u32, f32, f32) = (0, 0.0, 0.0);
    for mut it in tail {
        match &it {
            Item::Mark { id: FENCE_OPEN, x, y, w, h } => {
                close(items, &mut group);
                if fence.0 == 0 {
                    let (px, py, _, _) = turn.rect(*x, *y, *w, *h);
                    fence = (1, px - x, py - y);
                } else {
                    fence.0 += 1;
                }
                continue;
            }
            Item::Mark { id: FENCE_CLOSE, .. } => {
                fence.0 = fence.0.saturating_sub(1);
                continue;
            }
            _ if fence.0 > 0 => {
                crate::layout::shift_item(&mut it, fence.1, fence.2);
                items.push(it);
                continue;
            }
            _ => {}
        }
        match it {
            Item::Text { x, y, text, font, size, color } => {
                let m = mid(fonts, font, size);
                if let Some(rest) = text.strip_prefix(COMBINE_MARK) {
                    close(items, &mut group);
                    let (px, py) = turn.point(x + size / 2.0, y - m);
                    upright(items, fonts, rest, font, size, color, px, py, true);
                    continue;
                }
                // The text's own orientation, if marked.
                let first = text.chars().next().map_or(0, |c| c as u32);
                let (v, text) = if (ORIENT_MARK..ORIENT_MARK + 3).contains(&first) {
                    (Vertical { orient: (first - ORIENT_MARK) as u8, ..v }, text[3..].to_string())
                } else {
                    (v, text)
                };
                // Split into runs of upright characters and runs of sideways text.
                let mut lx = x;
                let mut run = String::new();
                let mut run_x = x;
                let flush = |items: &mut Vec<Item>, group: &mut Option<(usize, (f32, f32, f32, f32))>, run: &mut String, run_x: f32| {
                    if run.trim().is_empty() {
                        run.clear();
                        return;
                    }
                    let tw = fonts.width(font, run, size);
                    if measuring {
                        let (x, y, w, h) = turn.rect(run_x, y - m - size / 2.0, tw, size);
                        items.push(Item::Box { x, y, w, h, radius: 0.0, bg: Box::new(Default::default()) });
                        run.clear();
                        return;
                    }
                    let by = turn.mirror(y, m);
                    let area = (run_x, by - size * 1.2, run_x + tw, by + size * 0.5);
                    match group {
                        Some((_, a)) => {
                            a.0 = a.0.min(area.0);
                            a.1 = a.1.min(area.1);
                            a.2 = a.2.max(area.2);
                            a.3 = a.3.max(area.3);
                        }
                        None => {
                            *group = Some((items.len(), area));
                            items.push(Item::TransformPush { m: rot, x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
                        }
                    }
                    items.push(Item::Text { x: run_x, y: by, text: std::mem::take(run), font, size, color });
                };
                for c in text.chars() {
                    if v.upright(c) {
                        flush(items, &mut group, &mut run, run_x);
                        let s = vertical_form(c).to_string();
                        let a = size;
                        if !c.is_whitespace() {
                            close(items, &mut group);
                            let (px, py) = turn.point(lx + a / 2.0, y - m);
                            upright(items, fonts, &s, font, size, color, px, py, false);
                        }
                        lx += a;
                        run_x = lx;
                    } else {
                        if run.is_empty() {
                            run_x = lx;
                        }
                        run.push(c);
                        lx += fonts.advance(font, c, size);
                    }
                }
                flush(items, &mut group, &mut run, run_x);
            }
            other => {
                close(items, &mut group);
                items.push(turn_item(other, &turn, r));
            }
        }
    }
    close(items, &mut group);
}

/// A non-text item moved to the page.
fn turn_item(it: Item, t: &Turn, r: [f32; 6]) -> Item {
    let rect = |x: f32, y: f32, w: f32, h: f32| t.rect(x, y, w, h);
    match it {
        Item::Rect { x, y, w, h, color } => {
            let (x, y, w, h) = rect(x, y, w, h);
            Item::Rect { x, y, w, h, color }
        }
        Item::Image { x, y, w, h, src, view } => {
            // Pictures stay upright: same size, centred where the turned box is.
            let (px, py, pw, ph) = rect(x, y, w, h);
            Item::Image { x: px + (pw - w) / 2.0, y: py + (ph - h) / 2.0, w, h, src, view }
        }
        Item::Box { x, y, w, h, radius, bg } => {
            let (x, y, w, h) = rect(x, y, w, h);
            Item::Box { x, y, w, h, radius, bg }
        }
        Item::Ring { x, y, w, h, radius, width, color } => {
            let (x, y, w, h) = rect(x, y, w, h);
            Item::Ring { x, y, w, h, radius, width, color }
        }
        Item::Shadow { x, y, w, h, radius, blur, color } => {
            let (x, y, w, h) = rect(x, y, w, h);
            Item::Shadow { x, y, w, h, radius, blur, color }
        }
        Item::ClipPush { x, y, w, h, radius } => {
            let (x, y, w, h) = rect(x, y, w, h);
            Item::ClipPush { x, y, w, h, radius }
        }
        Item::Mark { id, x, y, w, h } => {
            let (x, y, w, h) = rect(x, y, w, h);
            Item::Mark { id, x, y, w, h }
        }
        Item::ScrollArea { id, x, y, w, h, max } => {
            let (x, y, w, h) = rect(x, y, w, h);
            Item::ScrollArea { id, x, y, w, h, max }
        }
        Item::StickyPush { id, y, h, top, limit } => {
            let (_, py, _, ph) = rect(0.0, y, 0.0, h);
            Item::StickyPush { id, y: py, h: ph, top, limit }
        }
        Item::TransformPush { m, x, y, w, h } => {
            // Its content is turned too, so turn the matrix: r·m·r⁻¹.
            let m = mul(r, mul(m, invert(r)));
            let (x, y, w, h) = rect(x, y, w, h);
            Item::TransformPush { m, x, y, w, h }
        }
        Item::EffectPush(mut fx) => {
            let (x, y, w, h) = rect(fx.x, fx.y, fx.w, fx.h);
            fx.x = x;
            fx.y = y;
            fx.w = w;
            fx.h = h;
            let (a, b, c, d) = fx.area;
            fx.area = rect(a, b, c, d);
            Item::EffectPush(fx)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_orientation() {
        let v = Vertical { mode: 1, orient: 0, combine: false };
        assert!(v.upright('漢') && v.upright('あ') && v.upright('カ') && v.upright('한'));
        assert!(!v.upright('A') && !v.upright('1') && !v.upright('ー') && !v.upright('「'));
        let up = Vertical { orient: 1, ..v };
        assert!(up.upright('A') && !up.upright(' '));
        let side = Vertical { orient: 2, ..v };
        assert!(!side.upright('漢'));
        assert_eq!(vertical_form('。'), '\u{FE12}');
        assert_eq!(vertical_form('漢'), '漢');
    }

    #[test]
    fn turns() {
        let rl = Turn { mode: 1, cx: 10.0, cy: 20.0, w: 100.0 };
        // The first line (top of the logical box) is the rightmost column.
        assert_eq!(rl.rect(10.0, 20.0, 50.0, 16.0), (94.0, 20.0, 16.0, 50.0));
        assert_eq!(rl.point(10.0, 20.0), (110.0, 20.0));
        let lr = Turn { mode: 2, ..rl };
        assert_eq!(lr.rect(10.0, 20.0, 50.0, 16.0), (10.0, 20.0, 16.0, 50.0));
        // The matrix agrees with point().
        for t in [rl, lr] {
            let m = t.matrix();
            let (x, y) = (37.0, 51.0);
            let p = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
            assert_eq!(p, t.point(x, y));
            let back = mul(m, invert(m));
            assert!(is_text_turn(&t.rotation()));
            assert!((back[0] - 1.0).abs() < 1e-6 && back[4].abs() < 1e-4);
        }
    }

    #[test]
    fn properties() {
        let mut v = Vertical::default();
        assert!(v.apply("writing-mode", "vertical-rl"));
        assert_eq!(v.mode, 1);
        v.apply("text-combine-upright", "all");
        assert!(v.combines());
        assert!(v.inherit().combine);
        v.apply("writing-mode", "sideways-lr");
        assert_eq!((v.mode, v.orient), (2, 2));
        assert!(!v.apply("color", "red"));
        assert_eq!(mark(Vertical::default()), None);
        assert_eq!(mark(Vertical { mode: 1, orient: 1, combine: false }), Some('\u{FDD2}'));
        assert_eq!(mark(Vertical { mode: 1, orient: 0, combine: true }), Some(COMBINE_MARK));
        assert_eq!('\u{FDD1}'.len_utf8(), 3);
    }
}

