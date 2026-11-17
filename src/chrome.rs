//! Browser UI drawn over the page: the link status bar, the zoom badge, the downloads panel,
//! find in page, text selection, and the right-click menu. The window loop owns the state;
//! this module draws it and works out what the mouse is on.

use crate::font::{self, Fonts};
use crate::layout::Layout;
use crate::paint::{Canvas, Item};
use crate::style::Color;

// ---------------------------------------------------------------- page text

/// One run of text on the page, where it's drawn (page coordinates, before scrolling).
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub x: f32,
    /// Baseline.
    pub y: f32,
    pub text: String,
    pub font: u8,
    pub size: f32,
    /// Drawn in a position: fixed box (stays put when the page scrolls).
    pub fixed: bool,
}

impl TextRun {
    fn top(&self) -> f32 {
        self.y - self.size * 0.95
    }
    fn bottom(&self) -> f32 {
        self.y + self.size * 0.3
    }
    /// x of the character boundary before character `i`.
    pub fn x_at(&self, fonts: &Fonts, i: usize) -> f32 {
        let prefix: String = self.text.chars().take(i).collect();
        self.x + fonts.width(self.font, &prefix, self.size)
    }
    /// The character boundary nearest to `x`.
    pub fn index_at(&self, fonts: &Fonts, x: f32) -> usize {
        let mut acc = self.x;
        for (i, c) in self.text.chars().enumerate() {
            let w = fonts.advance(self.font, c, self.size);
            if x < acc + w / 2.0 {
                return i;
            }
            acc += w;
        }
        self.text.chars().count()
    }
    /// Where it is on screen for this scroll position (y of the top).
    fn screen_top(&self, top_bar: f32, scroll: f32) -> f32 {
        top_bar + self.top() - if self.fixed { 0.0 } else { scroll }
    }
}

/// The page's text in reading order (top to bottom, then left to right).
pub fn text_runs(lay: &Layout) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut fixed = 0;
    for it in &lay.items {
        match it {
            Item::FixedPush => fixed += 1,
            Item::FixedPop => fixed -= 1,
            Item::Text { x, y, text, font, size, .. } if !text.trim().is_empty() => {
                out.push(TextRun { x: *x, y: *y, text: text.clone(), font: *font, size: *size, fixed: fixed > 0 });
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| (a.y.round() as i64, a.x as i64).cmp(&(b.y.round() as i64, b.x as i64)));
    out
}

// ---------------------------------------------------------------- status bar and zoom badge

/// The link under the mouse, shown in the bottom-left corner of the window.
pub fn draw_status(canvas: &mut Canvas, fonts: &Fonts, text: &str) {
    let size = 12.5;
    let max_w = (canvas.w as f32 * 0.6).max(120.0);
    let mut shown = text.to_string();
    if fonts.width(font::REGULAR, &shown, size) > max_w - 16.0 {
        while !shown.is_empty() && fonts.width(font::REGULAR, &format!("{shown}…"), size) > max_w - 16.0 {
            shown.pop();
        }
        shown.push('…');
    }
    let w = fonts.width(font::REGULAR, &shown, size) + 16.0;
    let h = 22.0;
    let y = canvas.h as f32 - h;
    canvas.fill_rect(0.0, y, w, h, Color::rgb(244, 244, 247));
    canvas.fill_rect(0.0, y, w, 1.0, Color::rgb(200, 200, 208));
    canvas.fill_rect(w - 1.0, y, 1.0, h, Color::rgb(200, 200, 208));
    canvas.draw_text(fonts, 8.0, y + 15.5, &shown, font::REGULAR, size, Color::rgb(50, 50, 60));
}

/// A short note under the toolbar's right end ("Zoom 125%", "Saved page.pdf to Downloads").
pub fn draw_toast(canvas: &mut Canvas, fonts: &Fonts, top: f32, label: &str) {
    let w = fonts.width(font::REGULAR, &label, 13.0) + 24.0;
    let x = canvas.w as f32 - w - 12.0;
    let y = top + 8.0;
    canvas.fill_rect(x, y, w, 30.0, Color::rgb(255, 255, 255));
    canvas.fill_rect(x, y, w, 1.0, Color::rgb(200, 200, 208));
    canvas.fill_rect(x, y + 29.0, w, 1.0, Color::rgb(200, 200, 208));
    canvas.fill_rect(x, y, 1.0, 30.0, Color::rgb(200, 200, 208));
    canvas.fill_rect(x + w - 1.0, y, 1.0, 30.0, Color::rgb(200, 200, 208));
    canvas.draw_text(fonts, x + 12.0, y + 20.0, &label, font::REGULAR, 13.0, Color::rgb(40, 40, 50));
}

// ---------------------------------------------------------------- simple menus

/// A list of rows drawn as a floating panel (the right-click menu and the downloads panel).
pub struct Menu<A> {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub rows: Vec<MenuRow<A>>,
    pub hover: Option<usize>,
}

pub struct MenuRow<A> {
    pub label: String,
    /// Smaller second line (download status), if any.
    pub detail: Option<String>,
    /// None = a separator or a disabled row.
    pub action: Option<A>,
    pub separator: bool,
}

impl<A> MenuRow<A> {
    pub fn item(label: &str, action: A) -> Self {
        MenuRow { label: label.to_string(), detail: None, action: Some(action), separator: false }
    }
    pub fn disabled(label: &str) -> Self {
        MenuRow { label: label.to_string(), detail: None, action: None, separator: false }
    }
    pub fn separator() -> Self {
        MenuRow { label: String::new(), detail: None, action: None, separator: true }
    }
}

const ROW_H: f32 = 28.0;
const DETAIL_H: f32 = 44.0;
const SEP_H: f32 = 9.0;

impl<A: Clone> Menu<A> {
    /// Place the menu at (x, y), kept inside the window.
    pub fn new(x: f32, y: f32, w: f32, rows: Vec<MenuRow<A>>, win_w: f32, win_h: f32) -> Self {
        let mut m = Menu { x, y, w, rows, hover: None };
        let h = m.height();
        m.x = m.x.min(win_w - w - 4.0).max(4.0);
        if m.y + h > win_h - 4.0 {
            m.y = (m.y - h).max(4.0);
        }
        m
    }

    fn row_h(r: &MenuRow<A>) -> f32 {
        if r.separator {
            SEP_H
        } else if r.detail.is_some() {
            DETAIL_H
        } else {
            ROW_H
        }
    }

    pub fn height(&self) -> f32 {
        self.rows.iter().map(Self::row_h).sum::<f32>() + 8.0
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.height()
    }

    /// The row under the mouse (only rows that do something).
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.contains(x, y) {
            return None;
        }
        let mut top = self.y + 4.0;
        for (i, r) in self.rows.iter().enumerate() {
            let h = Self::row_h(r);
            if y >= top && y < top + h {
                return r.action.is_some().then_some(i);
            }
            top += h;
        }
        None
    }

    /// Which rows do something (for moving through the menu with the keyboard).
    pub fn actionable(&self) -> Vec<bool> {
        self.rows.iter().map(|r| r.action.is_some()).collect()
    }

    /// The middle of a row, in window coordinates.
    pub fn row_center(&self, row: usize) -> Option<(f32, f32)> {
        let mut top = self.y + 4.0;
        for (i, r) in self.rows.iter().enumerate() {
            let h = Self::row_h(r);
            if i == row {
                return Some((self.x + self.w / 2.0, top + h / 2.0));
            }
            top += h;
        }
        None
    }

    pub fn action(&self, row: usize) -> Option<A> {
        self.rows.get(row).and_then(|r| r.action.clone())
    }

    pub fn draw(&self, canvas: &mut Canvas, fonts: &Fonts) {
        let h = self.height();
        // A soft shadow, then the panel.
        canvas.fill_rect(self.x + 2.0, self.y + 3.0, self.w, h, Color { r: 0, g: 0, b: 0, a: 40 });
        canvas.fill_rect(self.x, self.y, self.w, h, Color::rgb(255, 255, 255));
        let border = Color::rgb(196, 196, 204);
        canvas.fill_rect(self.x, self.y, self.w, 1.0, border);
        canvas.fill_rect(self.x, self.y + h - 1.0, self.w, 1.0, border);
        canvas.fill_rect(self.x, self.y, 1.0, h, border);
        canvas.fill_rect(self.x + self.w - 1.0, self.y, 1.0, h, border);
        let mut top = self.y + 4.0;
        for (i, r) in self.rows.iter().enumerate() {
            let rh = Self::row_h(r);
            if r.separator {
                canvas.fill_rect(self.x + 8.0, top + 4.0, self.w - 16.0, 1.0, Color::rgb(222, 222, 228));
            } else {
                if self.hover == Some(i) && r.action.is_some() {
                    canvas.fill_rect(self.x + 4.0, top, self.w - 8.0, rh, Color::rgb(226, 232, 255));
                }
                let ink = if r.action.is_some() { Color::rgb(30, 30, 40) } else { Color::rgb(150, 150, 160) };
                let label = fit(fonts, &r.label, 13.0, self.w - 28.0);
                canvas.draw_text(fonts, self.x + 14.0, top + 19.0, &label, font::REGULAR, 13.0, ink);
                if let Some(d) = &r.detail {
                    let d = fit(fonts, d, 11.5, self.w - 28.0);
                    canvas.draw_text(fonts, self.x + 14.0, top + 36.0, &d, font::REGULAR, 11.5, Color::rgb(100, 100, 115));
                }
            }
            top += rh;
        }
    }
}

pub fn fit(fonts: &Fonts, s: &str, size: f32, max: f32) -> String {
    if fonts.width(font::REGULAR, s, size) <= max {
        return s.to_string();
    }
    let mut t: String = s.to_string();
    while !t.is_empty() && fonts.width(font::REGULAR, &format!("{t}…"), size) > max {
        t.pop();
    }
    t + "…"
}

// ---------------------------------------------------------------- find in page

pub const FIND_H: f32 = 40.0;

/// Ctrl+F: the find bar along the bottom of the window, and the matches it highlights.
#[derive(Default)]
pub struct Find {
    pub query: String,
    /// (run index, first char, char count) for each match, in reading order.
    pub matches: Vec<(usize, usize, usize)>,
    pub current: usize,
    pub case_sensitive: bool,
}

impl Find {
    /// Search the page's text again (after the query or the page changed).
    pub fn search(&mut self, runs: &[TextRun]) {
        self.matches.clear();
        if self.query.is_empty() {
            return;
        }
        let fold = |s: &str| if self.case_sensitive { s.to_string() } else { s.to_lowercase() };
        let q: Vec<char> = fold(&self.query).chars().collect();
        for (ri, r) in runs.iter().enumerate() {
            let t: Vec<char> = fold(&r.text).chars().collect();
            if t.len() < q.len() {
                continue;
            }
            let mut i = 0;
            while i + q.len() <= t.len() {
                if t[i..i + q.len()] == q[..] {
                    self.matches.push((ri, i, q.len()));
                    i += q.len();
                } else {
                    i += 1;
                }
            }
        }
        if self.current >= self.matches.len() {
            self.current = 0;
        }
    }

    /// The page y (top) of the current match, to scroll it into view.
    pub fn current_y(&self, runs: &[TextRun]) -> Option<(f32, bool)> {
        let &(ri, _, _) = self.matches.get(self.current)?;
        runs.get(ri).map(|r| (r.top(), r.fixed))
    }

    pub fn next(&mut self, back: bool) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.current = if back { (self.current + n - 1) % n } else { (self.current + 1) % n };
    }

    /// Highlight the matches on the page (yellow; the current one orange).
    pub fn draw_highlights(&self, canvas: &mut Canvas, fonts: &Fonts, runs: &[TextRun], top_bar: f32, scroll: f32, view_bottom: f32) {
        for (k, &(ri, start, len)) in self.matches.iter().enumerate() {
            let Some(r) = runs.get(ri) else { continue };
            let y = r.screen_top(top_bar, scroll);
            let h = r.bottom() - r.top();
            if y + h < top_bar || y > view_bottom {
                continue;
            }
            let (x0, x1) = (r.x_at(fonts, start), r.x_at(fonts, start + len));
            let c = if k == self.current { Color { r: 255, g: 150, b: 50, a: 150 } } else { Color { r: 255, g: 230, b: 0, a: 120 } };
            let top = y.max(top_bar);
            canvas.fill_rect(x0, top, x1 - x0, (y + h).min(view_bottom) - top, c);
        }
    }

    pub fn draw_bar(&self, canvas: &mut Canvas, fonts: &Fonts, caret: Option<usize>) {
        let (w, h) = (canvas.w as f32, canvas.h as f32);
        let y = h - FIND_H;
        canvas.fill_rect(0.0, y, w, FIND_H, Color::rgb(236, 236, 240));
        canvas.fill_rect(0.0, y, w, 1.0, Color::rgb(200, 200, 205));
        canvas.draw_text(fonts, 12.0, y + 25.0, &crate::i18n::t("find-label"), font::REGULAR, 13.0, Color::rgb(70, 70, 80));
        let fx = 52.0;
        let fw = (w * 0.4).clamp(180.0, 420.0);
        let none = !self.query.is_empty() && self.matches.is_empty();
        canvas.fill_rect(fx, y + 7.0, fw, FIND_H - 14.0, if none { Color::rgb(255, 228, 228) } else { Color::rgb(255, 255, 255) });
        canvas.draw_text(fonts, fx + 8.0, y + 25.0, &self.query, font::REGULAR, 14.0, Color::rgb(30, 30, 40));
        if let Some(c) = caret {
            let prefix: String = self.query.chars().take(c).collect();
            let full = fonts.width(font::REGULAR, &self.query, 14.0);
            let cx = crate::rtl::field_x(fx + 8.0, fonts.width(font::REGULAR, &prefix, 14.0), full) - if crate::rtl::on() { 1.0 } else { 0.0 };
            canvas.fill_rect(cx.round(), y + 11.0, 1.0, FIND_H - 22.0, Color::rgb(30, 30, 40));
        }
        let status = if self.query.is_empty() {
            String::new()
        } else if self.matches.is_empty() {
            crate::i18n::t("find-none")
        } else {
            crate::i18n::tf("find-count", &[("current", &(self.current + 1).to_string()), ("total", &self.matches.len().to_string())])
        };
        let sx = fx + fw + 12.0;
        canvas.draw_text(fonts, sx, y + 25.0, &status, font::REGULAR, 13.0, Color::rgb(70, 70, 80));
        let hint = &crate::i18n::t("find-hint");
        let hw = fonts.width(font::REGULAR, hint, 12.0);
        if sx + 110.0 + hw < w {
            canvas.draw_text(fonts, w - hw - 12.0, y + 25.0, hint, font::REGULAR, 12.0, Color::rgb(120, 120, 130));
        }
    }
}

// ---------------------------------------------------------------- text selection

/// A text selection made by dragging over the page: two points in page coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub from: (f32, f32),
    pub to: (f32, f32),
    /// Ctrl+A: the whole page.
    pub all: bool,
}

impl Selection {
    /// For each text run the selection touches: (run index, first char, end char).
    pub fn ranges(&self, fonts: &Fonts, runs: &[TextRun]) -> Vec<(usize, usize, usize)> {
        if self.all {
            return runs.iter().enumerate().map(|(i, r)| (i, 0, r.text.chars().count())).collect();
        }
        // Order the two ends in reading order.
        let line = |p: (f32, f32)| runs.iter().position(|r| p.1 >= r.top() && p.1 <= r.bottom()).map(|i| runs[i].y.round());
        let key = |p: (f32, f32)| (line(p).unwrap_or(p.1), p.0);
        let (a, b) = if key(self.from) <= key(self.to) { (self.from, self.to) } else { (self.to, self.from) };
        let mut out = Vec::new();
        for (i, r) in runs.iter().enumerate() {
            let n = r.text.chars().count();
            let on = |p: (f32, f32)| p.1 >= r.top() && p.1 <= r.bottom();
            if r.bottom() < a.1 && !on(a) {
                continue;
            }
            if r.top() > b.1 && !on(b) {
                continue;
            }
            let start = if on(a) { r.index_at(fonts, a.0) } else { 0 };
            let end = if on(b) { r.index_at(fonts, b.0) } else { n };
            // Runs on the first line wholly left of the start, or on the last line wholly
            // right of the end, aren't selected.
            if start < end {
                out.push((i, start, end));
            }
        }
        out
    }

    /// The selected text, with line breaks between lines.
    pub fn text(&self, fonts: &Fonts, runs: &[TextRun]) -> String {
        let mut out = String::new();
        let mut last: Option<&TextRun> = None;
        for (i, s, e) in self.ranges(fonts, runs) {
            let r = &runs[i];
            if let Some(prev) = last {
                if (prev.y - r.y).abs() > 1.0 {
                    out.push('\n');
                } else if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            out.extend(r.text.chars().skip(s).take(e - s));
            last = Some(r);
        }
        out.trim().to_string()
    }

    pub fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, runs: &[TextRun], top_bar: f32, scroll: f32, view_bottom: f32) {
        // (the previous selected run: its line and where its highlight ended, if it ran to its end)
        let mut prev: Option<(f32, f32)> = None;
        for (i, s, e) in self.ranges(fonts, runs) {
            let r = &runs[i];
            let y = r.screen_top(top_bar, scroll);
            let h = r.bottom() - r.top();
            let n = r.text.chars().count();
            let (mut x0, x1) = (r.x_at(fonts, s), r.x_at(fonts, e));
            // Close the gap to the previous piece on the same line (words of a link, say).
            if let Some((py, px)) = prev {
                if s == 0 && (py - r.y).abs() < 1.0 && px < x0 && x0 - px < r.size * 1.5 {
                    x0 = px;
                }
            }
            prev = (e == n).then_some((r.y, x1));
            if y + h < top_bar || y > view_bottom {
                continue;
            }
            let top = y.max(top_bar);
            canvas.fill_rect(x0, top, x1 - x0, (y + h).min(view_bottom) - top, Color { r: 60, g: 130, b: 250, a: 90 });
        }
    }
}
