//! Painting: turns the display list into pixels in a framebuffer.

use crate::font::Fonts;
use crate::images::Images;
use crate::style::Color;

#[derive(Debug, Clone)]
pub enum Item {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Color },
    /// `y` is the text baseline.
    Text { x: f32, y: f32, text: String, font: u8, size: f32, color: Color },
    Image { x: f32, y: f32, w: f32, h: f32, src: String },
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub buf: Vec<u32>, // 0RGB
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, buf: vec![0xFFFFFF; w * h] }
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
        let a = coverage * c.a as f32 / 255.0;
        if a <= 0.0 {
            return;
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
        let mut pen = x;
        for ch in text.chars() {
            if ch == ' ' || ch == '\u{a0}' {
                pen += fonts.advance(font, ' ', size);
                continue;
            }
            let adv = fonts.with_glyph(font, ch, size, |m, bmp| {
                let gx = pen.round() as i32 + m.xmin;
                let gy = baseline.round() as i32 - m.height as i32 - m.ymin;
                for row in 0..m.height {
                    for col in 0..m.width {
                        let cov = bmp[row * m.width + col];
                        if cov > 0 {
                            self.blend(gx + col as i32, gy + row as i32, c, cov as f32 / 255.0);
                        }
                    }
                }
                m.advance_width
            });
            pen += adv;
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

    /// Paint the display list, shifted by (dx, dy).
    pub fn paint(&mut self, fonts: &Fonts, images: &Images, items: &[Item], dx: f32, dy: f32) {
        let h = self.h as f32;
        for item in items {
            match item {
                Item::Rect { x, y, w, h: rh, color } => {
                    if y + dy + rh >= 0.0 && y + dy <= h {
                        self.fill_rect(x + dx, y + dy, *w, *rh, *color)
                    }
                }
                Item::Text { x, y, text, font, size, color } => {
                    self.draw_text(fonts, x + dx, y + dy, text, *font, *size, *color)
                }
                Item::Image { x, y, w, h: ih, src } => {
                    if y + dy + ih >= 0.0 && y + dy <= h {
                        if let Some(img) = images.scaled(src, w.round() as u32, ih.round() as u32) {
                            self.draw_image(&img, x + dx, y + dy);
                        }
                    }
                }
            }
        }
    }
}

pub fn pack(c: Color) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}
