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
