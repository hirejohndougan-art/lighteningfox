//! Visual effects drawn on whole groups of paint items: CSS `filter`, `backdrop-filter` and
//! blurred `text-shadow`.
//!
//! Layout wraps an element's paint items in `Item::EffectPush` / `Item::EffectPop`. At paint
//! time the items in between are drawn off-screen (over black and over white, which gives each
//! pixel's color and transparency exactly), the filters run on that copy, and the result is
//! blended onto the page. `backdrop-filter` instead filters what is already on the page behind
//! the element's border box before the element itself is drawn.

use crate::font::Fonts;
use crate::images::Images;
use crate::paint::{Canvas, Item};
use crate::style::{parse_color, parse_len, Color};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filter {
    /// Gaussian blur with this standard deviation (px).
    Blur(f32),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    Sepia(f32),
    Saturate(f32),
    /// Degrees.
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    DropShadow { x: f32, y: f32, blur: f32, color: Color },
}

/// One `text-shadow`. `color: None` means the text's own color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextShadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub color: Option<Color>,
}

/// What an `Item::EffectPush` carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    /// The element's border box (where `backdrop-filter` applies) and its corner radius.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    /// Everything the wrapped items draw lies inside this box (before filtering).
    pub area: (f32, f32, f32, f32),
    pub filter: Vec<Filter>,
    pub backdrop: Vec<Filter>,
}

impl Effect {
    pub fn shift(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
        self.area.0 += dx;
        self.area.1 += dy;
    }

    /// The same effect at `z` times the size (page zoom).
    pub fn scaled(&self, z: f32) -> Effect {
        let sc = |f: &Filter| match *f {
            Filter::Blur(r) => Filter::Blur(r * z),
            Filter::DropShadow { x, y, blur, color } => Filter::DropShadow { x: x * z, y: y * z, blur: blur * z, color },
            other => other,
        };
        Effect {
            x: self.x * z,
            y: self.y * z,
            w: self.w * z,
            h: self.h * z,
            radius: self.radius * z,
            area: (self.area.0 * z, self.area.1 * z, self.area.2 * z, self.area.3 * z),
            filter: self.filter.iter().map(sc).collect(),
            backdrop: self.backdrop.iter().map(sc).collect(),
        }
    }

    /// How far the filters can spread drawing beyond `area` (blur and shadow offsets).
    pub fn margin(&self) -> f32 {
        margin(&self.filter)
    }
}

fn margin(filters: &[Filter]) -> f32 {
    let mut m = 0.0f32;
    for f in filters {
        m += match *f {
            Filter::Blur(s) => 3.0 * s,
            Filter::DropShadow { x, y, blur, .. } => x.abs().max(y.abs()) + 3.0 * blur,
            _ => 0.0,
        };
    }
    m.ceil()
}

// ------------------------------------------------------------------------------- parsing

fn amount(arg: &str, default: f32) -> Option<f32> {
    let a = arg.trim();
    if a.is_empty() {
        return Some(default);
    }
    if let Some(p) = a.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|v| v / 100.0);
    }
    a.parse::<f32>().ok()
}

fn angle(arg: &str) -> Option<f32> {
    let a = arg.trim();
    if a.is_empty() {
        return Some(0.0);
    }
    let num_end = a.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))).unwrap_or(a.len());
    let n: f32 = a[..num_end].parse().ok()?;
    Some(match &a[num_end..] {
        "" | "deg" => n,
        "rad" => n.to_degrees(),
        "turn" => n * 360.0,
        "grad" => n * 0.9,
        _ => return None,
    })
}

fn px(t: &str, fs: f32) -> Option<f32> {
    match parse_len(t, fs)? {
        crate::style::Len::Px(v) => Some(v),
        _ => None,
    }
}

/// A shadow's lengths and color: `x y [blur] [color]` in any order of color vs lengths.
fn shadow_parts(s: &str, fs: f32) -> Option<(Vec<f32>, Option<Color>)> {
    let mut nums = Vec::new();
    let mut color = None;
    for t in crate::css::split_top(s, ' ') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(v) = px(t, fs) {
            nums.push(v);
        } else if t.eq_ignore_ascii_case("currentcolor") {
            color = None;
        } else {
            color = Some(parse_color(t)?);
        }
    }
    (nums.len() >= 2 && nums.len() <= 3).then_some((nums, color))
}

/// `filter` / `backdrop-filter`: a list of filter functions, or `none`. Invalid lists give
/// `None` (the declaration is ignored, as in other browsers).
pub fn parse_filters(value: &str, fs: f32) -> Option<Vec<Filter>> {
    let v = value.trim().to_ascii_lowercase();
    if v == "none" {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut rest = v.as_str();
    while !rest.trim().is_empty() {
        let r = rest.trim_start();
        let open = r.find('(')?;
        let name = r[..open].trim();
        // Find the matching parenthesis.
        let mut depth = 0;
        let mut close = None;
        for (i, c) in r.char_indices().skip(open) {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let arg = &r[open + 1..close];
        out.push(match name {
            "blur" => Filter::Blur(if arg.trim().is_empty() { 0.0 } else { px(arg, fs)?.max(0.0) }),
            "brightness" => Filter::Brightness(amount(arg, 1.0)?.max(0.0)),
            "contrast" => Filter::Contrast(amount(arg, 1.0)?.max(0.0)),
            "grayscale" => Filter::Grayscale(amount(arg, 1.0)?.clamp(0.0, 1.0)),
            "sepia" => Filter::Sepia(amount(arg, 1.0)?.clamp(0.0, 1.0)),
            "saturate" => Filter::Saturate(amount(arg, 1.0)?.max(0.0)),
            "invert" => Filter::Invert(amount(arg, 1.0)?.clamp(0.0, 1.0)),
            "opacity" => Filter::Opacity(amount(arg, 1.0)?.clamp(0.0, 1.0)),
            "hue-rotate" => Filter::HueRotate(angle(arg)?),
            "drop-shadow" => {
                let (n, color) = shadow_parts(arg, fs)?;
                Filter::DropShadow { x: n[0], y: n[1], blur: n.get(2).copied().unwrap_or(0.0).max(0.0) / 2.0, color: color.unwrap_or(Color::BLACK) }
            }
            // url(#svg-filter) and unknown functions: skip the whole declaration.
            _ => return None,
        });
        rest = &r[close + 1..];
    }
    Some(out)
}

/// `text-shadow`: a comma-separated list of `x y [blur] [color]`, or `none`.
pub fn parse_text_shadow(value: &str, fs: f32) -> Option<Vec<TextShadow>> {
    let v = value.trim().to_ascii_lowercase();
    if v == "none" {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for part in crate::css::split_top(&v, ',') {
        let (n, color) = shadow_parts(&part, fs)?;
        out.push(TextShadow { x: n[0], y: n[1], blur: n.get(2).copied().unwrap_or(0.0).max(0.0), color });
    }
    Some(out)
}

/// Paint items for a text item's shadows (drawn under it, the first shadow on top).
pub fn text_shadow_items(text: &Item, shadows: &[TextShadow]) -> Vec<Item> {
    let Item::Text { x, y, text, font, size, color } = text else { return Vec::new() };
    let mut out = Vec::new();
    for s in shadows.iter().rev() {
        let mut c = s.color.unwrap_or(*color);
        // The text's own fade (opacity) carries over to its shadow.
        if s.color.is_some() && color.a < 255 {
            c.a = (c.a as f32 * color.a as f32 / 255.0).round() as u8;
        }
        if c.a == 0 {
            continue;
        }
        let item = Item::Text { x: x + s.x, y: y + s.y, text: text.clone(), font: *font, size: *size, color: c };
        if s.blur > 0.5 {
            // The blur radius is twice the Gaussian's standard deviation.
            let sigma = s.blur / 2.0;
            let (ax, ay, aw, ah) = (x + s.x - size, y + s.y - size * 1.3, size * (text.chars().count() as f32 + 2.0), size * 2.0);
            out.push(Item::EffectPush(Box::new(Effect { x: ax, y: ay, w: aw, h: ah, radius: 0.0, area: (ax, ay, aw, ah), filter: vec![Filter::Blur(sigma)], backdrop: Vec::new() })));
            out.push(item);
            out.push(Item::EffectPop);
        } else {
            out.push(item);
        }
    }
    out
}

// ------------------------------------------------------------------------------- pixels

/// Premultiplied RGBA, 0..1.
type Px = [f32; 4];

struct Layer {
    w: usize,
    h: usize,
    px: Vec<Px>,
}

/// Box sizes whose three passes approximate a Gaussian with standard deviation `sigma`.
fn box_sizes(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round() as i32;
    let mut out = [0usize; 3];
    for (i, o) in out.iter_mut().enumerate() {
        *o = if (i as i32) < m { wl.max(1) as usize } else { wu.max(1) as usize };
    }
    out
}

/// One box-blur pass of radius `r` along rows (`horizontal`) or columns. Outside is transparent.
fn box_pass(l: &mut Layer, r: usize, horizontal: bool, tmp: &mut Vec<Px>) {
    if r == 0 {
        return;
    }
    let (len, lines) = if horizontal { (l.w, l.h) } else { (l.h, l.w) };
    let idx = |line: usize, i: usize| if horizontal { line * l.w + i } else { i * l.w + line };
    let norm = 1.0 / (2 * r + 1) as f32;
    tmp.resize(len, [0.0; 4]);
    for line in 0..lines {
        let mut acc = [0.0f32; 4];
        // Window [i - r, i + r]; start with [−r, r] for i = 0.
        for i in 0..=r.min(len.saturating_sub(1)) {
            let p = l.px[idx(line, i)];
            for k in 0..4 {
                acc[k] += p[k];
            }
        }
        for i in 0..len {
            for k in 0..4 {
                tmp[i][k] = acc[k] * norm;
            }
            let add = i + r + 1;
            if add < len {
                let p = l.px[idx(line, add)];
                for k in 0..4 {
                    acc[k] += p[k];
                }
            }
            if i >= r {
                let p = l.px[idx(line, i - r)];
                for k in 0..4 {
                    acc[k] -= p[k];
                }
            }
        }
        for i in 0..len {
            l.px[idx(line, i)] = tmp[i];
        }
    }
}

fn blur(l: &mut Layer, sigma: f32) {
    if sigma < 0.3 {
        return;
    }
    let mut tmp = Vec::new();
    for b in box_sizes(sigma) {
        let r = (b - 1) / 2;
        box_pass(l, r, true, &mut tmp);
        box_pass(l, r, false, &mut tmp);
    }
}

/// Apply a 3×3 color matrix (on unpremultiplied colors) plus an offset.
fn color_op(l: &mut Layer, f: impl Fn([f32; 3]) -> [f32; 3]) {
    for p in l.px.iter_mut() {
        let a = p[3];
        if a <= 0.0 {
            continue;
        }
        let c = f([p[0] / a, p[1] / a, p[2] / a]);
        for k in 0..3 {
            p[k] = c[k].clamp(0.0, 1.0) * a;
        }
    }
}

fn matrix(m: [[f32; 3]; 3]) -> impl Fn([f32; 3]) -> [f32; 3] {
    move |c| {
        let mut o = [0.0; 3];
        for (i, row) in m.iter().enumerate() {
            o[i] = row[0] * c[0] + row[1] * c[1] + row[2] * c[2];
        }
        o
    }
}

fn saturate_m(s: f32) -> [[f32; 3]; 3] {
    [
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
    ]
}

fn apply_filter(l: &mut Layer, f: &Filter) {
    match *f {
        Filter::Blur(s) => blur(l, s),
        Filter::Brightness(b) => color_op(l, |c| [c[0] * b, c[1] * b, c[2] * b]),
        Filter::Contrast(k) => color_op(l, |c| [(c[0] - 0.5) * k + 0.5, (c[1] - 0.5) * k + 0.5, (c[2] - 0.5) * k + 0.5]),
        Filter::Invert(v) => color_op(l, |c| [c[0] * (1.0 - 2.0 * v) + v, c[1] * (1.0 - 2.0 * v) + v, c[2] * (1.0 - 2.0 * v) + v]),
        Filter::Saturate(s) => color_op(l, matrix(saturate_m(s))),
        Filter::Grayscale(g) => color_op(l, matrix(saturate_m(1.0 - g))),
        Filter::Sepia(s) => {
            let t = 1.0 - s;
            color_op(
                l,
                matrix([
                    [0.393 + 0.607 * t, 0.769 - 0.769 * t, 0.189 - 0.189 * t],
                    [0.349 - 0.349 * t, 0.686 + 0.314 * t, 0.168 - 0.168 * t],
                    [0.272 - 0.272 * t, 0.534 - 0.534 * t, 0.131 + 0.869 * t],
                ]),
            )
        }
        Filter::HueRotate(deg) => {
            let (s, c) = deg.to_radians().sin_cos();
            color_op(
                l,
                matrix([
                    [0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928],
                    [0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283],
                    [0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072],
                ]),
            )
        }
        Filter::Opacity(o) => l.px.iter_mut().for_each(|p| p.iter_mut().for_each(|v| *v *= o)),
        Filter::DropShadow { x, y, blur: sigma, color } => {
            let ca = color.a as f32 / 255.0;
            let col = [color.r as f32 / 255.0 * ca, color.g as f32 / 255.0 * ca, color.b as f32 / 255.0 * ca, ca];
            let mut sh = Layer { w: l.w, h: l.h, px: vec![[0.0; 4]; l.px.len()] };
            let (ox, oy) = (x.round() as i64, y.round() as i64);
            for yy in 0..l.h as i64 {
                let sy = yy - oy;
                if sy < 0 || sy >= l.h as i64 {
                    continue;
                }
                for xx in 0..l.w as i64 {
                    let sx = xx - ox;
                    if sx < 0 || sx >= l.w as i64 {
                        continue;
                    }
                    let a = l.px[(sy as usize) * l.w + sx as usize][3];
                    sh.px[yy as usize * l.w + xx as usize] = [col[0] * a, col[1] * a, col[2] * a, col[3] * a];
                }
            }
            blur(&mut sh, sigma);
            // The element over its shadow.
            for (p, s) in l.px.iter_mut().zip(sh.px) {
                let k = 1.0 - p[3];
                for i in 0..4 {
                    p[i] += s[i] * k;
                }
            }
        }
    }
}

/// Draw `items` (already shifted by dx, dy in page → canvas terms) into an off-screen layer
/// covering canvas pixels [rx, rx + w) × [ry, ry + h), as premultiplied RGBA.
#[allow(clippy::too_many_arguments)]
fn render_layer(canvas_paint: &dyn Fn(&mut Canvas, f32, f32), w: usize, h: usize, rx: i32, ry: i32) -> Layer {
    let mut b = Canvas::new(w, h);
    let mut wh = Canvas::new(w, h);
    b.clear(Color::rgb(0, 0, 0));
    wh.clear(Color::rgb(255, 255, 255));
    canvas_paint(&mut b, -rx as f32, -ry as f32);
    canvas_paint(&mut wh, -rx as f32, -ry as f32);
    let ch = |p: u32, s: u32| ((p >> s) & 255) as f32 / 255.0;
    let px = b
        .buf
        .iter()
        .zip(&wh.buf)
        .map(|(&pb, &pw)| {
            let a = 1.0 - ((ch(pw, 16) - ch(pb, 16)) + (ch(pw, 8) - ch(pb, 8)) + (ch(pw, 0) - ch(pb, 0))) / 3.0;
            let a = a.clamp(0.0, 1.0);
            [ch(pb, 16).min(a), ch(pb, 8).min(a), ch(pb, 0).min(a), a]
        })
        .collect();
    Layer { w, h, px }
}

/// Integer pixel box of (x, y, w, h) grown by `m`, cut to the canvas grown by `m`.
fn region(c: &Canvas, x: f32, y: f32, w: f32, h: f32, m: f32) -> Option<(i32, i32, usize, usize)> {
    let mi = m.ceil() as i32;
    let x0 = ((x - m).floor() as i32).max(-mi);
    let y0 = ((y - m).floor() as i32).max(-mi);
    let x1 = ((x + w + m).ceil() as i32).min(c.w as i32 + mi);
    let y1 = ((y + h + m).ceil() as i32).min(c.h as i32 + mi);
    (x1 > x0 && y1 > y0 && (x1 - x0) as i64 * ((y1 - y0) as i64) < 64_000_000).then(|| (x0, y0, (x1 - x0) as usize, (y1 - y0) as usize))
}

impl Canvas {
    /// Draw an effect group: `inner` are the items between its push and pop, page coordinates
    /// shifted by (dx, dy) to reach this canvas.
    pub(crate) fn draw_effect(&mut self, fonts: &Fonts, images: &Images, inner: &[Item], fx: &Effect, dx: f32, dy: f32) {
        if !fx.backdrop.is_empty() {
            self.apply_backdrop(fx, dx, dy);
        }
        if fx.filter.is_empty() {
            self.paint_group(fonts, images, inner, dx, dy);
            return;
        }
        let m = fx.margin();
        let (ax, ay, aw, ah) = fx.area;
        let Some((rx, ry, w, h)) = region(self, ax + dx, ay + dy, aw, ah, m) else { return };
        let paint = |c: &mut Canvas, ox: f32, oy: f32| c.paint_group(fonts, images, inner, dx + ox, dy + oy);
        let mut layer = render_layer(&paint, w, h, rx, ry);
        for f in &fx.filter {
            apply_filter(&mut layer, f);
        }
        self.composite(&layer, rx, ry);
    }

    /// Blend a premultiplied layer onto the canvas at (rx, ry).
    fn composite(&mut self, l: &Layer, rx: i32, ry: i32) {
        for yy in 0..l.h {
            let cy = ry + yy as i32;
            if cy < 0 || cy >= self.h as i32 {
                continue;
            }
            for xx in 0..l.w {
                let cx = rx + xx as i32;
                if cx < 0 || cx >= self.w as i32 {
                    continue;
                }
                let p = l.px[yy * l.w + xx];
                let a = p[3];
                if a <= 0.002 {
                    continue;
                }
                let un = |v: f32| ((v / a) * 255.0).round().clamp(0.0, 255.0) as u8;
                self.blend_px(cx, cy, Color { r: un(p[0]), g: un(p[1]), b: un(p[2]), a: 255 }, a.min(1.0));
            }
        }
    }

    /// Filter what's already drawn behind the border box (backdrop-filter).
    fn apply_backdrop(&mut self, fx: &Effect, dx: f32, dy: f32) {
        let m = margin(&fx.backdrop);
        let (bx, by) = (fx.x + dx, fx.y + dy);
        // Read a little around the box so blurs near its edges see real pixels.
        let x0 = ((bx - m).floor() as i32).max(0);
        let y0 = ((by - m).floor() as i32).max(0);
        let x1 = ((bx + fx.w + m).ceil() as i32).min(self.w as i32);
        let y1 = ((by + fx.h + m).ceil() as i32).min(self.h as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut px = Vec::with_capacity(w * h);
        for yy in y0..y1 {
            for xx in x0..x1 {
                let p = self.buf[yy as usize * self.w + xx as usize];
                px.push([((p >> 16) & 255) as f32 / 255.0, ((p >> 8) & 255) as f32 / 255.0, (p & 255) as f32 / 255.0, 1.0]);
            }
        }
        let mut layer = Layer { w, h, px };
        for f in &fx.backdrop {
            apply_filter(&mut layer, f);
        }
        // Blurring treats outside as transparent; scale edge pixels back to full strength.
        for yy in 0..h {
            let cy = y0 + yy as i32;
            for xx in 0..w {
                let cx = x0 + xx as i32;
                let (fx_, fy_) = (cx as f32 + 0.5, cy as f32 + 0.5);
                let cov = crate::paint::rounded_cov(fx_, fy_, bx, by, fx.w, fx.h, fx.radius);
                if cov <= 0.0 {
                    continue;
                }
                let p = layer.px[yy * w + xx];
                let a = p[3];
                if a <= 0.002 {
                    continue;
                }
                let un = |v: f32| ((v / a) * 255.0).round().clamp(0.0, 255.0) as u8;
                self.blend_px(cx, cy, Color { r: un(p[0]), g: un(p[1]), b: un(p[2]), a: 255 }, cov * a.min(1.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filter_lists() {
        let f = parse_filters("blur(4px) grayscale(50%) hue-rotate(0.5turn) drop-shadow(2px 3px 4px red)", 16.0).unwrap();
        assert_eq!(f[0], Filter::Blur(4.0));
        assert_eq!(f[1], Filter::Grayscale(0.5));
        assert_eq!(f[2], Filter::HueRotate(180.0));
        assert!(matches!(f[3], Filter::DropShadow { x: 2.0, y: 3.0, blur: 2.0, color: Color { r: 255, .. } }));
        assert_eq!(parse_filters("none", 16.0), Some(vec![]));
        assert_eq!(parse_filters("url(#f)", 16.0), None);
        assert_eq!(parse_filters("brightness(1.5) contrast()", 16.0), Some(vec![Filter::Brightness(1.5), Filter::Contrast(1.0)]));
    }

    #[test]
    fn parses_text_shadows() {
        let s = parse_text_shadow("1px 2px 3px #000, red 0 0 1em", 10.0).unwrap();
        assert_eq!(s[0], TextShadow { x: 1.0, y: 2.0, blur: 3.0, color: Some(Color::BLACK) });
        assert_eq!(s[1], TextShadow { x: 0.0, y: 0.0, blur: 10.0, color: Some(Color::rgb(255, 0, 0)) });
        assert_eq!(parse_text_shadow("1px 1px", 10.0).unwrap()[0].color, None);
        assert!(parse_text_shadow("1px", 10.0).is_none());
    }

    #[test]
    fn blur_keeps_total_and_spreads() {
        let mut l = Layer { w: 41, h: 1, px: vec![[0.0; 4]; 41] };
        l.px[20] = [1.0; 4];
        let mut tmp = Vec::new();
        for b in box_sizes(3.0) {
            box_pass(&mut l, (b - 1) / 2, true, &mut tmp);
        }
        let sum: f32 = l.px.iter().map(|p| p[3]).sum();
        assert!((sum - 1.0).abs() < 1e-3, "{sum}");
        assert!(l.px[20][3] < 0.2 && l.px[17][3] > 0.02);
    }

    #[test]
    fn grayscale_makes_gray() {
        let mut l = Layer { w: 1, h: 1, px: vec![[1.0, 0.0, 0.0, 1.0]] };
        apply_filter(&mut l, &Filter::Grayscale(1.0));
        let p = l.px[0];
        assert!((p[0] - p[1]).abs() < 1e-4 && (p[1] - p[2]).abs() < 1e-4);
        let mut l = Layer { w: 1, h: 1, px: vec![[0.2, 0.4, 0.6, 1.0]] };
        apply_filter(&mut l, &Filter::Invert(1.0));
        assert!((l.px[0][0] - 0.8).abs() < 1e-4);
    }
}
