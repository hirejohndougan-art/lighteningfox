//! CSS gradients: linear, radial and conic, each also repeating (CSS Images Level 3/4).
//!
//! `parse` reads the CSS function; `Gradient::prepare` resolves it for a box, after which
//! `Prepared::at` gives the color at any point. Stop positions may be percentages, lengths
//! (linear and radial) or angles (conic), and a stop may carry two positions (`red 10% 20%`).

use crate::style::{Color, Len};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// CSS angle in degrees: 0 = towards the top, 90 = towards the right.
    Linear { angle: f32 },
    Radial { circle: bool, extent: Extent, at: (Len, Len) },
    /// `from` in degrees, clockwise from the top.
    Conic { from: f32, at: (Len, Len) },
}

/// How big a radial gradient's ending shape is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Extent {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
    /// Explicit radii (x, y); a circle uses x.
    Size(Len, Len),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub kind: Kind,
    /// Colors and positions: Pct for percentages (and conic angles, as a share of a turn),
    /// Px for lengths. None = spread evenly between neighbours.
    pub stops: Vec<(Color, Option<Len>)>,
    pub repeating: bool,
}

fn angle_deg(t: &str) -> Option<f32> {
    let t = t.trim();
    let num = |s: &str| t.strip_suffix(s).and_then(|n| n.trim().parse::<f32>().ok());
    num("deg").or_else(|| num("grad").map(|g| g * 0.9)).or_else(|| num("rad").map(f32::to_degrees)).or_else(|| num("turn").map(|n| n * 360.0)).or_else(|| (t == "0").then_some(0.0))
}

fn position(words: &[&str], fs: f32) -> (Len, Len) {
    let mut x = None;
    let mut y = None;
    for &w in words {
        match w {
            "left" => x = Some(Len::Pct(0.0)),
            "right" => x = Some(Len::Pct(100.0)),
            "top" => y = Some(Len::Pct(0.0)),
            "bottom" => y = Some(Len::Pct(100.0)),
            "center" => {}
            t => {
                if let Some(l) = crate::style::parse_len(t, fs) {
                    if x.is_none() {
                        x = Some(l);
                    } else if y.is_none() {
                        y = Some(l);
                    }
                }
            }
        }
    }
    (x.unwrap_or(Len::Pct(50.0)), y.unwrap_or(Len::Pct(50.0)))
}

/// Parse `linear-gradient(…)`, `radial-gradient(…)`, `conic-gradient(…)` and their
/// `repeating-` (and `-webkit-` linear) forms. `fs` is the font size for em lengths.
pub fn parse(v: &str, fs: f32) -> Option<Gradient> {
    let lower = v.trim().to_ascii_lowercase();
    let open = lower.find('(')?;
    let name = lower[..open].trim_start_matches("-webkit-").trim_start_matches("-moz-");
    let (repeating, name) = match name.strip_prefix("repeating-") {
        Some(n) => (true, n),
        None => (false, name),
    };
    let body = &lower[open + 1..lower.rfind(')')?];
    let mut args: Vec<String> = crate::css::split_top(body, ',').into_iter().map(|a| a.trim().to_string()).collect();
    let first_is_stop = args.first().is_some_and(|a| a.split_whitespace().any(|w| crate::style::parse_color(w).is_some()) && !a.starts_with("to ") && !a.starts_with("from ") && !a.starts_with("at "));
    let prelude = if first_is_stop || args.is_empty() { None } else { Some(args.remove(0)) };
    let words: Vec<&str> = prelude.as_deref().map(|p| p.split_whitespace().collect()).unwrap_or_default();
    let kind = match name {
        "linear-gradient" => {
            let mut angle = 180.0;
            if let Some(p) = prelude.as_deref() {
                if let Some(a) = angle_deg(p) {
                    // -webkit-linear-gradient measured angles the old way (0deg = to the right).
                    angle = if lower.starts_with("-webkit-") { 90.0 - a } else { a };
                } else if let Some(dir) = p.strip_prefix("to ") {
                    let has = |w: &str| dir.split_whitespace().any(|d| d == w);
                    let (h, vtc) = (if has("left") { -1.0 } else if has("right") { 1.0 } else { 0.0 }, if has("top") { -1.0 } else if has("bottom") { 1.0 } else { 0.0 });
                    angle = match (h, vtc) {
                        (0.0, -1.0) => 0.0,
                        (1.0, 0.0) => 90.0,
                        (0.0, 1.0) => 180.0,
                        (-1.0, 0.0) => 270.0,
                        (1.0, -1.0) => 45.0,
                        (1.0, 1.0) => 135.0,
                        (-1.0, 1.0) => 225.0,
                        (-1.0, -1.0) => 315.0,
                        _ => 180.0,
                    };
                } else if lower.starts_with("-webkit-") {
                    // -webkit-linear-gradient(left, …): the side it starts from.
                    angle = match p.trim() {
                        "left" => 90.0,
                        "right" => 270.0,
                        "bottom" => 0.0,
                        _ => 180.0,
                    };
                }
            }
            Kind::Linear { angle }
        }
        "radial-gradient" => {
            let at = words.iter().position(|w| *w == "at");
            let (shape, pos) = match at {
                Some(i) => (&words[..i], position(&words[i + 1..], fs)),
                None => (&words[..], (Len::Pct(50.0), Len::Pct(50.0))),
            };
            let mut circle = shape.contains(&"circle");
            let mut extent = Extent::FarthestCorner;
            let lens: Vec<Len> = shape.iter().filter_map(|w| crate::style::parse_len(w, fs)).collect();
            for w in shape {
                extent = match *w {
                    "closest-side" => Extent::ClosestSide,
                    "farthest-side" => Extent::FarthestSide,
                    "closest-corner" => Extent::ClosestCorner,
                    "farthest-corner" => Extent::FarthestCorner,
                    _ => extent,
                };
            }
            match lens.as_slice() {
                [r] => {
                    circle = !shape.contains(&"ellipse");
                    extent = Extent::Size(*r, *r);
                }
                [a, b, ..] => {
                    circle = false;
                    extent = Extent::Size(*a, *b);
                }
                _ => {}
            }
            Kind::Radial { circle, extent, at: pos }
        }
        "conic-gradient" => {
            let mut from = 0.0;
            let mut at = (Len::Pct(50.0), Len::Pct(50.0));
            if let Some(i) = words.iter().position(|w| *w == "from") {
                from = words.get(i + 1).and_then(|a| angle_deg(a)).unwrap_or(0.0);
            }
            if let Some(i) = words.iter().position(|w| *w == "at") {
                at = position(&words[i + 1..], fs);
            }
            Kind::Conic { from, at }
        }
        _ => return None,
    };
    let conic = matches!(kind, Kind::Conic { .. });
    let mut stops = Vec::new();
    for a in &args {
        let parts = crate::css::split_top(a, ' ');
        let parts: Vec<&str> = parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
        let Some(ci) = parts.iter().position(|p| crate::style::parse_color(p).is_some()) else { continue }; // a bare hint
        let c = crate::style::parse_color(parts[ci])?;
        let pos = |p: &str| -> Option<Len> {
            if conic {
                if let Some(d) = angle_deg(p) {
                    return Some(Len::Pct(d / 3.6));
                }
            }
            crate::style::parse_len(p, fs)
        };
        let ps: Vec<Len> = parts.iter().enumerate().filter(|(i, _)| *i != ci).filter_map(|(_, p)| pos(p)).collect();
        match ps.as_slice() {
            [] => stops.push((c, None)),
            [p] => stops.push((c, Some(*p))),
            [p, q, ..] => {
                stops.push((c, Some(*p)));
                stops.push((c, Some(*q)));
            }
        }
    }
    if stops.is_empty() {
        return None;
    }
    if stops.len() == 1 {
        stops.push(stops[0]);
    }
    Some(Gradient { kind, stops, repeating })
}

/// A gradient resolved for one box: ready to give colors.
pub struct Prepared {
    kind: PKind,
    /// (color, position along the gradient line, 0..1 of its length)
    stops: Vec<(Color, f32)>,
    repeating: bool,
}

enum PKind {
    /// Unit direction and the line's length; t is measured from its start.
    Linear { dx: f32, dy: f32, len: f32, cx: f32, cy: f32 },
    Radial { cx: f32, cy: f32, rx: f32, ry: f32 },
    Conic { cx: f32, cy: f32, from: f32 },
}

impl Gradient {
    /// Resolve for a w×h box. Point coordinates for `at` are relative to the box.
    pub fn prepare(&self, w: f32, h: f32) -> Prepared {
        let (kind, line) = match &self.kind {
            Kind::Linear { angle } => {
                let a = angle.to_radians();
                let (dx, dy) = (a.sin(), -a.cos());
                let len = (w * dx).abs() + (h * dy).abs();
                (PKind::Linear { dx, dy, len: len.max(1e-3), cx: w / 2.0, cy: h / 2.0 }, len)
            }
            Kind::Radial { circle, extent, at } => {
                let (cx, cy) = (at.0.resolve(w), at.1.resolve(h));
                let (l, r, t, b) = (cx.abs(), (w - cx).abs(), cy.abs(), (h - cy).abs());
                let (sx_c, sy_c) = (l.min(r), t.min(b));
                let (sx_f, sy_f) = (l.max(r), t.max(b));
                let (rx, ry) = match (extent, circle) {
                    (Extent::Size(a, _), true) => (a.resolve(w), a.resolve(w)),
                    (Extent::Size(a, b), false) => (a.resolve(w), b.resolve(h)),
                    (Extent::ClosestSide, true) => (sx_c.min(sy_c), sx_c.min(sy_c)),
                    (Extent::FarthestSide, true) => (sx_f.max(sy_f), sx_f.max(sy_f)),
                    (Extent::ClosestSide, false) => (sx_c, sy_c),
                    (Extent::FarthestSide, false) => (sx_f, sy_f),
                    (Extent::ClosestCorner, true) => {
                        let d = sx_c.hypot(sy_c);
                        (d, d)
                    }
                    (Extent::FarthestCorner, true) => {
                        let d = sx_f.hypot(sy_f);
                        (d, d)
                    }
                    // An ellipse through the corner, with the sides' proportions.
                    (Extent::ClosestCorner, false) => (sx_c * std::f32::consts::SQRT_2, sy_c * std::f32::consts::SQRT_2),
                    (Extent::FarthestCorner, false) => (sx_f * std::f32::consts::SQRT_2, sy_f * std::f32::consts::SQRT_2),
                };
                (PKind::Radial { cx, cy, rx: rx.max(1e-3), ry: ry.max(1e-3) }, rx.max(1e-3))
            }
            Kind::Conic { from, at } => (PKind::Conic { cx: at.0.resolve(w), cy: at.1.resolve(h), from: from.to_radians() }, 1.0),
        };
        Prepared { kind, stops: resolve_stops(&self.stops, line), repeating: self.repeating }
    }
}

/// Every stop position as a share of the gradient line (lengths divided by its length),
/// missing ones spread evenly, never going backwards (CSS Images "color stop fixup").
fn resolve_stops(stops: &[(Color, Option<Len>)], line: f32) -> Vec<(Color, f32)> {
    let n = stops.len();
    let mut pos: Vec<Option<f32>> = stops
        .iter()
        .map(|(_, p)| {
            p.map(|l| match l {
                Len::Pct(v) => v / 100.0,
                Len::Px(v) => v / line.max(1e-3),
                Len::Mix(px, pct) => px / line.max(1e-3) + pct / 100.0,
                Len::Auto => 0.0,
            })
        })
        .collect();
    if pos[0].is_none() {
        pos[0] = Some(0.0);
    }
    if pos[n - 1].is_none() {
        pos[n - 1] = Some(pos[..n - 1].iter().flatten().fold(1.0f32, |a, &b| a.max(b)));
    }
    let mut last = f32::MIN;
    for p in pos.iter_mut().flatten() {
        if *p < last {
            *p = last;
        }
        last = *p;
    }
    let mut i = 0;
    while i < n {
        if pos[i].is_none() {
            let start = i - 1;
            let mut end = i;
            while pos[end].is_none() {
                end += 1;
            }
            let (a, b) = (pos[start].unwrap(), pos[end].unwrap());
            for (k, p) in pos.iter_mut().enumerate().take(end).skip(i) {
                *p = Some(a + (b - a) * (k - start) as f32 / (end - start) as f32);
            }
            i = end;
        }
        i += 1;
    }
    stops.iter().zip(pos).map(|((c, _), p)| (*c, p.unwrap())).collect()
}

impl Prepared {
    /// The color at (x, y), relative to the box's top-left corner.
    pub fn at(&self, x: f32, y: f32) -> Color {
        let t = match self.kind {
            PKind::Linear { dx, dy, len, cx, cy } => ((x - cx) * dx + (y - cy) * dy) / len + 0.5,
            PKind::Radial { cx, cy, rx, ry } => ((x - cx) / rx).hypot((y - cy) / ry),
            PKind::Conic { cx, cy, from } => {
                // Clockwise from the top.
                let a = (x - cx).atan2(-(y - cy)) - from;
                a.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU
            }
        };
        self.color(t)
    }

    fn color(&self, t: f32) -> Color {
        let s = &self.stops;
        let t = if self.repeating {
            let (a, b) = (s[0].1, s[s.len() - 1].1);
            if b - a > 1e-4 { a + (t - a).rem_euclid(b - a) } else { t }
        } else {
            t
        };
        if t <= s[0].1 {
            return s[0].0;
        }
        for pair in s.windows(2) {
            let ((c0, p0), (c1, p1)) = (pair[0], pair[1]);
            if t <= p1 {
                let f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
                // Mix in premultiplied space, so fading to `transparent` doesn't go grey.
                let (a0, a1) = (c0.a as f32 / 255.0, c1.a as f32 / 255.0);
                let a = a0 + (a1 - a0) * f;
                let ch = |x: u8, y: u8| {
                    if a <= 0.0 {
                        0
                    } else {
                        ((x as f32 * a0 + (y as f32 * a1 - x as f32 * a0) * f) / a).round().clamp(0.0, 255.0) as u8
                    }
                };
                return Color { r: ch(c0.r, c1.r), g: ch(c0.g, c1.g), b: ch(c0.b, c1.b), a: (a * 255.0).round() as u8 };
            }
        }
        s[s.len() - 1].0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(c: Color) -> (u8, u8, u8) {
        (c.r, c.g, c.b)
    }

    #[test]
    fn linear_directions_and_lengths() {
        let g = parse("linear-gradient(to right, red, blue)", 16.0).unwrap();
        let p = g.prepare(100.0, 10.0);
        assert_eq!(rgb(p.at(0.0, 5.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(100.0, 5.0)), (0, 0, 255));
        let g = parse("linear-gradient(90deg, red 20px, blue 20px)", 16.0).unwrap();
        let p = g.prepare(100.0, 10.0);
        assert_eq!(rgb(p.at(19.0, 5.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(21.0, 5.0)), (0, 0, 255));
    }

    #[test]
    fn repeating_stripes() {
        let g = parse("repeating-linear-gradient(90deg, red 0 10px, white 10px 20px)", 16.0).unwrap();
        assert!(g.repeating);
        let p = g.prepare(200.0, 10.0);
        assert_eq!(rgb(p.at(5.0, 5.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(15.0, 5.0)), (255, 255, 255));
        assert_eq!(rgb(p.at(125.0, 5.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(195.0, 5.0)), (255, 255, 255));
    }

    #[test]
    fn conic_pie() {
        let g = parse("conic-gradient(red 0 25%, lime 25% 50%, blue 50% 75%, black 75%)", 16.0).unwrap();
        let p = g.prepare(100.0, 100.0);
        assert_eq!(rgb(p.at(70.0, 20.0)), (255, 0, 0), "top right");
        assert_eq!(rgb(p.at(70.0, 80.0)), (0, 255, 0), "bottom right");
        assert_eq!(rgb(p.at(30.0, 80.0)), (0, 0, 255), "bottom left");
        assert_eq!(rgb(p.at(30.0, 20.0)), (0, 0, 0), "top left");
        let g = parse("conic-gradient(from 90deg at 50% 50%, red 0deg 90deg, blue 90deg)", 16.0).unwrap();
        let p = g.prepare(100.0, 100.0);
        // Starting at 3 o'clock, the first quarter is the bottom-right one.
        assert_eq!(rgb(p.at(80.0, 55.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(55.0, 80.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(40.0, 90.0)), (0, 0, 255));
        assert_eq!(rgb(p.at(80.0, 45.0)), (0, 0, 255));
    }

    #[test]
    fn radial_shapes() {
        let g = parse("radial-gradient(circle 10px at 20px 20px, red 99%, blue 100%)", 16.0).unwrap();
        let p = g.prepare(100.0, 100.0);
        assert_eq!(rgb(p.at(20.0, 25.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(20.0, 35.0)), (0, 0, 255));
        // The default is an ellipse reaching the farthest corner.
        let g = parse("radial-gradient(red, blue)", 16.0).unwrap();
        let p = g.prepare(200.0, 100.0);
        assert_eq!(rgb(p.at(100.0, 50.0)), (255, 0, 0));
        let edge = p.at(0.0, 0.0);
        assert!(edge.b > 250 && edge.r < 5, "{edge:?}");
        let g = parse("repeating-radial-gradient(circle, red 0 5px, white 5px 10px)", 16.0).unwrap();
        let p = g.prepare(100.0, 100.0);
        assert_eq!(rgb(p.at(52.0, 50.0)), (255, 0, 0));
        assert_eq!(rgb(p.at(57.0, 50.0)), (255, 255, 255));
        assert_eq!(rgb(p.at(72.0, 50.0)), (255, 0, 0));
    }

    #[test]
    fn transparent_fades_without_grey() {
        let g = parse("linear-gradient(90deg, rgb(255,0,0), transparent)", 16.0).unwrap();
        let mid = g.prepare(100.0, 1.0).at(50.0, 0.5);
        assert_eq!((mid.r, mid.g, mid.b), (255, 0, 0));
        assert!(mid.a > 100 && mid.a < 155);
    }
}
