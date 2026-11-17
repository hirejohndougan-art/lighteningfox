//! Browser-drawn helpers for form controls, shown over the page: the bubble that explains why
//! a form wasn't sent (validation.rs), and the pickers for `<input type=date|month|week|time|
//! datetime-local|color>`.

use std::time::{Duration, Instant};

use crate::dom::NodeId;
use crate::font::{self, Fonts};
use crate::paint::Canvas;
use crate::style::Color;

const INK: Color = Color { r: 40, g: 40, b: 50, a: 255 };
const EDGE: Color = Color { r: 190, g: 190, b: 200, a: 255 };
const ACCENT: Color = Color { r: 11, g: 107, b: 211, a: 255 };
const WHITE: Color = Color { r: 255, g: 255, b: 255, a: 255 };

fn panel(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    canvas.fill_rect(x + 2.0, y + 3.0, w, h, Color { r: 0, g: 0, b: 0, a: 35 });
    canvas.fill_rect(x, y, w, h, EDGE);
    canvas.fill_rect(x + 1.0, y + 1.0, w - 2.0, h - 2.0, WHITE);
}

// ---------------------------------------------------------------- validation bubble

/// "Please fill out this field." under the control that stopped a submission.
pub struct Bubble {
    pub node: NodeId,
    pub text: String,
    pub until: Instant,
}

impl Bubble {
    pub fn new(node: NodeId, text: String) -> Bubble {
        Bubble { node, text, until: Instant::now() + Duration::from_secs(6) }
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    /// Draw below the control's box (`x`, `bottom` in window coordinates), kept inside the window.
    pub fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, x: f32, bottom: f32) {
        let size = 13.0;
        let lines = wrap(fonts, &self.text, size, 360.0);
        let w = lines.iter().map(|l| fonts.width(font::REGULAR, l, size)).fold(0.0, f32::max) + 40.0;
        let h = lines.len() as f32 * 18.0 + 14.0;
        let x = x.min(canvas.w as f32 - w - 6.0).max(6.0);
        let y = (bottom + 8.0).min(canvas.h as f32 - h - 6.0);
        panel(canvas, x, y, w, h);
        // The little arrow pointing up at the control.
        for i in 0..6 {
            let fi = i as f32;
            canvas.fill_rect(x + 16.0 - fi, y - 6.0 + fi, 1.0 + 2.0 * fi, 1.0, EDGE);
            canvas.fill_rect(x + 16.0 - fi + 1.0, y - 5.0 + fi, (2.0 * fi - 1.0).max(0.0), 1.0, WHITE);
        }
        // A warning badge.
        canvas.fill_rect(x + 10.0, y + 9.0, 16.0, 16.0, Color { r: 230, g: 140, b: 20, a: 255 });
        canvas.draw_text(fonts, x + 16.0, y + 22.0, "!", font::BOLD, 13.0, WHITE);
        for (i, l) in lines.iter().enumerate() {
            canvas.draw_text(fonts, x + 32.0, y + 21.0 + i as f32 * 18.0, l, font::REGULAR, size, INK);
        }
    }
}

/// Break text into lines no wider than `max` (at spaces, and at explicit newlines).
fn wrap(fonts: &Fonts, text: &str, size: f32, max: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let trial = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && fonts.width(font::REGULAR, &trial, size) > max {
                out.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = trial;
            }
        }
        out.push(line);
    }
    out.truncate(6);
    out
}

// ---------------------------------------------------------------- pickers

use crate::dom::Dom;
use crate::validation::{civil_from_days, days_from_civil, parse_date, parse_month, parse_time, Numeric};

/// Which picker an input gets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickerKind {
    Date,
    Month,
    Week,
    Time,
    DateTime,
    Color,
}

impl PickerKind {
    pub fn of(dom: &Dom, id: NodeId) -> Option<PickerKind> {
        if dom.tag(id) != Some("input") || crate::forms::is_disabled(dom, id) || dom.attr(id, "readonly").is_some() {
            return None;
        }
        Some(match dom.attr(id, "type")?.trim().to_ascii_lowercase().as_str() {
            "date" => PickerKind::Date,
            "month" => PickerKind::Month,
            "week" => PickerKind::Week,
            "time" => PickerKind::Time,
            "datetime-local" => PickerKind::DateTime,
            "color" => PickerKind::Color,
            _ => return None,
        })
    }
}

/// What a click in a picker did.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    /// Inside the panel, nothing to change (or just turned a page).
    Nothing,
    /// Set the input's value (and close the picker if `close`).
    Set { value: String, close: bool },
    /// The click was outside: close the picker and let the page have the click.
    Outside,
}

/// A calendar, month grid, time grid or color palette shown under an input.
pub struct Picker {
    pub node: NodeId,
    pub doc: u64,
    pub kind: PickerKind,
    /// Top-left corner in page coordinates.
    pub x: f32,
    pub y: f32,
    /// The month (or year, for the month grid) being shown.
    view: (i64, u32),
    /// The input's value when the picker was opened (and as changed since).
    value: String,
    min: Option<f64>,
    max: Option<f64>,
    pub hover: Option<(f32, f32)>,
}

const CELL: f32 = 32.0;
const ROW: f32 = 28.0;
const PAD: f32 = 8.0;
const HEAD: f32 = 34.0;
const FOOT: f32 = 32.0;
const SWATCH: f32 = 28.0;

/// The palette offered for <input type=color>.
const PALETTE: [u32; 48] = [
    0x000000, 0x434343, 0x666666, 0x999999, 0xb7b7b7, 0xcccccc, 0xefefef, 0xffffff, //
    0x980000, 0xff0000, 0xff9900, 0xffff00, 0x00ff00, 0x00ffff, 0x4a86e8, 0x0000ff, //
    0x9900ff, 0xff00ff, 0xe6b8af, 0xf4cccc, 0xfce5cd, 0xfff2cc, 0xd9ead3, 0xd0e0e3, //
    0xc9daf8, 0xcfe2f3, 0xd9d2e9, 0xead1dc, 0xdd7e6b, 0xea9999, 0xf9cb9c, 0xffe599, //
    0xb6d7a8, 0xa2c4c9, 0xa4c2f4, 0x9fc5e8, 0xb4a7d6, 0xd5a6bd, 0xcc4125, 0xe06666, //
    0xf6b26b, 0xffd966, 0x93c47d, 0x76a5af, 0x6d9eeb, 0x6fa8dc, 0x8e7cc3, 0xc27ba0,
];

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn today_days() -> i64 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    secs.div_euclid(86_400)
}

fn days_in(y: i64, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u32
}

/// The ISO week ("2024-W05") a day falls in.
fn iso_week(days: i64) -> String {
    let weekday = (days + 3).rem_euclid(7); // 0 = Monday
    let thursday = days - weekday + 3;
    let (y, _, _) = civil_from_days(thursday);
    let week = (thursday - days_from_civil(y, 1, 1)) / 7 + 1;
    format!("{y:04}-W{week:02}")
}

impl Picker {
    /// A picker for `node`, whose box is at (x, y, w, h) in page coordinates.
    pub fn new(dom: &Dom, node: NodeId, doc: u64, kind: PickerKind, x: f32, bottom: f32) -> Picker {
        let value = crate::forms::value(dom, node);
        let attr = |a: &str| Numeric::of(dom.attr(node, "type").unwrap_or("").trim().to_ascii_lowercase().as_str()).and_then(|k| dom.attr(node, a).and_then(|v| k.parse(v.trim())));
        let (min, max) = (attr("min"), attr("max"));
        let today = civil_from_days(today_days());
        let shown = match kind {
            PickerKind::Date | PickerKind::DateTime => parse_date(value.split('T').next().unwrap_or("")).map(|(y, m, _)| (y, m)),
            PickerKind::Week => Numeric::Week.parse(&value).map(|ms| civil_from_days((ms / 86_400_000.0) as i64)).map(|(y, m, _)| (y, m)),
            PickerKind::Month => parse_month(&value),
            _ => None,
        };
        Picker { node, doc, kind, x, y: bottom + 2.0, view: shown.unwrap_or((today.0, today.1)), value, min, max, hover: None }
    }

    pub fn size(&self) -> (f32, f32) {
        match self.kind {
            PickerKind::Date | PickerKind::Week | PickerKind::DateTime => (7.0 * CELL + 2.0 * PAD, HEAD + 20.0 + 6.0 * ROW + FOOT + PAD),
            PickerKind::Month => (4.0 * 56.0 + 2.0 * PAD, HEAD + 3.0 * 40.0 + FOOT + PAD),
            PickerKind::Time => (6.0 * 38.0 + 2.0 * PAD, 20.0 + 4.0 * 26.0 + 20.0 + 2.0 * 26.0 + FOOT + PAD + 4.0),
            PickerKind::Color => (8.0 * SWATCH + 2.0 * PAD, 6.0 * SWATCH + 2.0 * PAD + FOOT),
        }
    }

    /// The selected day (days since 1970), if the value names one.
    fn selected_day(&self) -> Option<i64> {
        match self.kind {
            PickerKind::Date | PickerKind::DateTime => parse_date(self.value.split('T').next()?).map(|(y, m, d)| days_from_civil(y, m, d)),
            PickerKind::Week => Numeric::Week.parse(&self.value).map(|ms| (ms / 86_400_000.0).round() as i64),
            _ => None,
        }
    }

    fn allowed(&self, ms: f64) -> bool {
        self.min.is_none_or(|m| ms >= m) && self.max.is_none_or(|m| ms <= m)
    }

    /// The first day shown in the calendar grid (a Sunday on or before the 1st).
    fn grid_start(&self) -> i64 {
        let first = days_from_civil(self.view.0, self.view.1, 1);
        first - (first + 4).rem_euclid(7) // 0 = Sunday
    }

    /// The value for a day picked in the calendar.
    fn value_for_day(&self, day: i64) -> String {
        let (y, m, d) = civil_from_days(day);
        match self.kind {
            PickerKind::Week => iso_week(day),
            PickerKind::DateTime => {
                let time = self.value.split_once('T').map(|(_, t)| t.to_string()).filter(|t| parse_time(t).is_some()).unwrap_or_else(|| "12:00".into());
                format!("{y:04}-{m:02}-{d:02}T{time}")
            }
            _ => format!("{y:04}-{m:02}-{d:02}"),
        }
    }

    fn turn(&mut self, by: i32) {
        if self.kind == PickerKind::Month {
            self.view.0 += by as i64;
            return;
        }
        let total = self.view.0 * 12 + self.view.1 as i64 - 1 + by as i64;
        self.view = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
    }

    /// A click at (mx, my) in window coordinates; `top` is where the panel's top is drawn.
    pub fn click(&mut self, mx: f32, my: f32, top: f32) -> Pick {
        let (w, h) = self.size();
        let (lx, ly) = (mx - self.x, my - top);
        if lx < 0.0 || ly < 0.0 || lx > w || ly > h {
            return Pick::Outside;
        }
        let footer = ly >= h - FOOT - PAD / 2.0 && self.kind != PickerKind::Color;
        if footer {
            // Left button: Today / This month / Now; right button: Clear.
            if lx < w / 2.0 {
                let today = today_days();
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                let (hh, mm) = ((now.rem_euclid(86_400) / 3600), (now.rem_euclid(3600) / 60));
                let (y, m, _) = civil_from_days(today);
                let value = match self.kind {
                    PickerKind::Month => format!("{y:04}-{m:02}"),
                    PickerKind::Time => format!("{hh:02}:{mm:02}"),
                    _ => self.value_for_day(today),
                };
                self.value = value.clone();
                return Pick::Set { value, close: true };
            }
            self.value.clear();
            return Pick::Set { value: String::new(), close: true };
        }
        match self.kind {
            PickerKind::Color => {
                let (c, r) = (((lx - PAD) / SWATCH).floor(), ((ly - PAD) / SWATCH).floor());
                if (0.0..8.0).contains(&c) && (0.0..6.0).contains(&r) {
                    let rgb = PALETTE[(r * 8.0 + c) as usize];
                    return Pick::Set { value: format!("#{rgb:06x}"), close: true };
                }
                Pick::Nothing
            }
            PickerKind::Time => {
                let (hh, mm) = parse_time(&self.value).map_or((None, None), |ms| {
                    let mins = (ms / 60_000.0) as i64;
                    (Some(mins / 60), Some(mins % 60))
                });
                let col = ((lx - PAD) / 38.0).floor();
                if !(0.0..6.0).contains(&col) {
                    return Pick::Nothing;
                }
                let hour_row = ((ly - 20.0) / 26.0).floor();
                let min_row = ((ly - (20.0 + 4.0 * 26.0 + 20.0)) / 26.0).floor();
                let value = if (0.0..4.0).contains(&hour_row) {
                    format!("{:02}:{:02}", (hour_row * 6.0 + col) as i64, mm.unwrap_or(0))
                } else if (0.0..2.0).contains(&min_row) {
                    format!("{:02}:{:02}", hh.unwrap_or(12), ((min_row * 6.0 + col) * 5.0) as i64)
                } else {
                    return Pick::Nothing;
                };
                self.value = value.clone();
                Pick::Set { value, close: false }
            }
            _ if ly < HEAD => {
                if lx < PAD + 28.0 {
                    self.turn(-1);
                } else if lx > w - PAD - 28.0 {
                    self.turn(1);
                }
                Pick::Nothing
            }
            PickerKind::Month => {
                let (c, r) = (((lx - PAD) / 56.0).floor(), ((ly - HEAD) / 40.0).floor());
                if (0.0..4.0).contains(&c) && (0.0..3.0).contains(&r) {
                    let m = (r * 4.0 + c) as u32 + 1;
                    let n = ((self.view.0 - 1970) * 12 + m as i64 - 1) as f64;
                    if self.allowed(n) {
                        let value = format!("{:04}-{m:02}", self.view.0);
                        self.value = value.clone();
                        return Pick::Set { value, close: true };
                    }
                }
                Pick::Nothing
            }
            _ => {
                let (c, r) = (((lx - PAD) / CELL).floor(), ((ly - HEAD - 20.0) / ROW).floor());
                if (0.0..7.0).contains(&c) && (0.0..6.0).contains(&r) {
                    let day = self.grid_start() + (r * 7.0 + c) as i64;
                    if self.allowed(day as f64 * 86_400_000.0) {
                        let value = self.value_for_day(day);
                        self.value = value.clone();
                        return Pick::Set { value, close: true };
                    }
                }
                Pick::Nothing
            }
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, top: f32) {
        let (w, h) = self.size();
        let (x, y) = (self.x.min(canvas.w as f32 - w - 4.0).max(4.0), top);
        panel(canvas, x, y, w, h);
        let faint = Color { r: 150, g: 150, b: 160, a: 255 };
        let hover = |cx: f32, cy: f32, cw: f32, ch: f32| self.hover.is_some_and(|(hx, hy)| hx >= cx && hx < cx + cw && hy >= cy && hy < cy + ch);
        let centered = |canvas: &mut Canvas, cx: f32, cw: f32, base: f32, t: &str, f: u8, c: Color| {
            let tw = fonts.width(f, t, 13.0);
            canvas.draw_text(fonts, (cx + (cw - tw) / 2.0).round(), base, t, f, 13.0, c);
        };
        let cell = |canvas: &mut Canvas, cx: f32, cy: f32, cw: f32, ch: f32, t: &str, selected: bool, marked: bool, ok: bool| {
            if selected {
                canvas.fill_rect(cx + 1.0, cy + 1.0, cw - 2.0, ch - 2.0, ACCENT);
            } else if ok && hover(cx, cy, cw, ch) {
                canvas.fill_rect(cx + 1.0, cy + 1.0, cw - 2.0, ch - 2.0, Color { r: 228, g: 236, b: 248, a: 255 });
            }
            if marked && !selected {
                canvas.fill_rect(cx + 4.0, cy + ch - 4.0, cw - 8.0, 2.0, ACCENT);
            }
            let ink = if selected { WHITE } else if ok { INK } else { faint };
            centered(canvas, cx, cw, cy + ch / 2.0 + 5.0, t, font::REGULAR, ink);
        };
        let footer = |canvas: &mut Canvas, left: &str| {
            let fy = y + h - FOOT - PAD / 2.0;
            canvas.fill_rect(x + 1.0, fy, w - 2.0, 1.0, Color { r: 225, g: 225, b: 232, a: 255 });
            centered(canvas, x, w / 2.0, fy + 21.0, left, font::BOLD, ACCENT);
            centered(canvas, x + w / 2.0, w / 2.0, fy + 21.0, "Clear", font::BOLD, ACCENT);
        };
        match self.kind {
            PickerKind::Color => {
                let current = crate::style::parse_color(&self.value);
                for (i, rgb) in PALETTE.iter().enumerate() {
                    let (cx, cy) = (x + PAD + (i % 8) as f32 * SWATCH, y + PAD + (i / 8) as f32 * SWATCH);
                    let c = Color { r: (rgb >> 16) as u8, g: (rgb >> 8) as u8, b: *rgb as u8, a: 255 };
                    let picked = current.is_some_and(|k| k.r == c.r && k.g == c.g && k.b == c.b);
                    if picked || hover(cx, cy, SWATCH, SWATCH) {
                        canvas.fill_rect(cx, cy, SWATCH, SWATCH, if picked { ACCENT } else { INK });
                    }
                    canvas.fill_rect(cx + 3.0, cy + 3.0, SWATCH - 6.0, SWATCH - 6.0, EDGE);
                    canvas.fill_rect(cx + 4.0, cy + 4.0, SWATCH - 8.0, SWATCH - 8.0, c);
                }
                let fy = y + h - FOOT;
                if let Some(c) = current {
                    canvas.fill_rect(x + PAD + 4.0, fy + 6.0, 20.0, 20.0, EDGE);
                    canvas.fill_rect(x + PAD + 5.0, fy + 7.0, 18.0, 18.0, c);
                }
                canvas.draw_text(fonts, x + PAD + 32.0, fy + 21.0, &self.value, font::REGULAR, 13.0, INK);
            }
            PickerKind::Time => {
                let mins = parse_time(&self.value).map(|ms| (ms / 60_000.0) as i64);
                canvas.draw_text(fonts, x + PAD, y + 15.0, "Hour", font::BOLD, 12.0, faint);
                for i in 0..24 {
                    let (cx, cy) = (x + PAD + (i % 6) as f32 * 38.0, y + 20.0 + (i / 6) as f32 * 26.0);
                    cell(canvas, cx, cy, 38.0, 26.0, &format!("{i:02}"), mins.is_some_and(|m| m / 60 == i), false, true);
                }
                let my = y + 20.0 + 4.0 * 26.0;
                canvas.draw_text(fonts, x + PAD, my + 15.0, "Minute", font::BOLD, 12.0, faint);
                for i in 0..12 {
                    let (cx, cy) = (x + PAD + (i % 6) as f32 * 38.0, my + 20.0 + (i / 6) as f32 * 26.0);
                    cell(canvas, cx, cy, 38.0, 26.0, &format!("{:02}", i * 5), mins.is_some_and(|m| m % 60 == i * 5), false, true);
                }
                footer(canvas, "Now");
            }
            _ => {
                // Header: ‹ March 2025 ›
                let title = if self.kind == PickerKind::Month { format!("{}", self.view.0) } else { format!("{} {}", MONTHS[self.view.1 as usize - 1], self.view.0) };
                centered(canvas, x, w, y + 22.0, &title, font::BOLD, INK);
                for (ax, dir) in [(x + PAD + 6.0, -1.0f32), (x + w - PAD - 14.0, 1.0)] {
                    for i in 0..5 {
                        let fi = i as f32;
                        let dx = if dir < 0.0 { 4.0 - fi } else { fi };
                        canvas.fill_rect(ax + dx, y + 13.0 + fi, 2.0, 1.0, INK);
                        canvas.fill_rect(ax + dx, y + 21.0 - fi, 2.0, 1.0, INK);
                    }
                }
                if self.kind == PickerKind::Month {
                    let sel = parse_month(&self.value);
                    let (ty, tm, _) = civil_from_days(today_days());
                    for (i, name) in MONTHS.iter().enumerate() {
                        let (cx, cy) = (x + PAD + (i % 4) as f32 * 56.0, y + HEAD + (i / 4) as f32 * 40.0);
                        let m = i as u32 + 1;
                        let ok = self.allowed(((self.view.0 - 1970) * 12 + i as i64) as f64);
                        cell(canvas, cx, cy, 56.0, 40.0, &name[..3], sel == Some((self.view.0, m)), (ty, tm) == (self.view.0, m), ok);
                    }
                    footer(canvas, "This month");
                } else {
                    for (i, d) in ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"].iter().enumerate() {
                        centered(canvas, x + PAD + i as f32 * CELL, CELL, y + HEAD + 14.0, d, font::BOLD, faint);
                    }
                    let start = self.grid_start();
                    let sel = self.selected_day();
                    let today = today_days();
                    for i in 0..42 {
                        let day = start + i;
                        let (_, m, d) = civil_from_days(day);
                        let (cx, cy) = (x + PAD + (i % 7) as f32 * CELL, y + HEAD + 20.0 + (i / 7) as f32 * ROW);
                        let selected = match (self.kind, sel) {
                            (PickerKind::Week, Some(s)) => day >= s && day < s + 7,
                            (_, Some(s)) => s == day,
                            _ => false,
                        };
                        let ok = self.allowed(day as f64 * 86_400_000.0);
                        let ink_ok = ok && m == self.view.1;
                        cell(canvas, cx, cy, CELL, ROW, &d.to_string(), selected, day == today, ink_ok);
                    }
                    footer(canvas, if self.kind == PickerKind::Week { "This week" } else { "Today" });
                }
            }
        }
    }

    /// Where the panel's left edge ends up after keeping it inside the window.
    pub fn left(&self, window_w: f32) -> f32 {
        self.x.min(window_w - self.size().0 - 4.0).max(4.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_math() {
        assert_eq!(iso_week(days_from_civil(2024, 1, 1)), "2024-W01");
        assert_eq!(iso_week(days_from_civil(2021, 1, 3)), "2020-W53");
        assert_eq!(days_in(2024, 2), 29);
    }

    #[test]
    fn date_picker_clicks() {
        let dom = crate::html::parse(r#"<input type=date value="2024-03-15" min="2024-03-05">"#);
        let id = dom.find_all("input")[0];
        let mut p = Picker::new(&dom, id, 0, PickerKind::Date, 10.0, 0.0);
        assert_eq!(p.view, (2024, 3));
        // March 2024 starts on a Friday: the grid starts Sunday Feb 25; row 2, Tuesday = March 12.
        let (cx, cy) = (10.0 + PAD + 2.0 * CELL + 5.0, HEAD + 20.0 + 2.0 * ROW + 5.0);
        assert_eq!(p.click(cx, cy + 2.0, 2.0), Pick::Set { value: "2024-03-12".into(), close: true });
        // March 4 is before min: refused.
        let (cx, cy) = (10.0 + PAD + 1.0 * CELL + 5.0, HEAD + 20.0 + 1.0 * ROW + 5.0);
        assert_eq!(p.click(cx, cy + 2.0, 2.0), Pick::Nothing);
        // Next month.
        p.click(10.0 + 7.0 * CELL + PAD, 12.0, 2.0);
        assert_eq!(p.view, (2024, 4));
        assert_eq!(p.click(500.0, 500.0, 2.0), Pick::Outside);
    }

    #[test]
    fn color_and_time() {
        let dom = crate::html::parse(r##"<input type=color value="#ff0000"><input type=time value="09:30">"##);
        let ids = dom.find_all("input");
        let mut c = Picker::new(&dom, ids[0], 0, PickerKind::Color, 0.0, 0.0);
        assert_eq!(c.click(PAD + 1.0 * SWATCH + 3.0, 2.0 + PAD + 3.0, 2.0), Pick::Set { value: "#434343".into(), close: true });
        let mut t = Picker::new(&dom, ids[1], 0, PickerKind::Time, 0.0, 0.0);
        // Hour 14 (row 2, column 2), keeping 30 minutes.
        assert_eq!(t.click(PAD + 2.0 * 38.0 + 3.0, 2.0 + 20.0 + 2.0 * 26.0 + 3.0, 2.0), Pick::Set { value: "14:30".into(), close: false });
    }
}
