//! CSS transforms: parse `transform` and `transform-origin` into a 2D affine matrix.
//!
//! A matrix `[a, b, c, d, e, f]` maps (x, y) to (a·x + c·y + e, b·x + d·y + f).

use crate::css;

pub type Matrix = [f32; 6];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m · n`: apply `n` first, then `m`.
pub fn mul(m: Matrix, n: Matrix) -> Matrix {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

pub fn translate(x: f32, y: f32) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

pub fn apply(m: &Matrix, x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

pub fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-9 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

/// Only moves things (no scale, rotation or skew)?
pub fn is_translation(m: &Matrix) -> bool {
    (m[0] - 1.0).abs() < 1e-6 && m[1].abs() < 1e-6 && m[2].abs() < 1e-6 && (m[3] - 1.0).abs() < 1e-6
}

/// Bounding box of a rectangle after transforming it: (x, y, w, h).
pub fn bounds(m: &Matrix, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let pts = [apply(m, x, y), apply(m, x + w, y), apply(m, x, y + h), apply(m, x + w, y + h)];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (px, py) in pts {
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    (x0, y0, x1 - x0, y1 - y0)
}

fn angle(v: &str) -> Option<f32> {
    let v = v.trim();
    let num = |s: &str| s.trim().parse::<f32>().ok();
    Some(if let Some(n) = v.strip_suffix("deg") {
        num(n)?.to_radians()
    } else if let Some(n) = v.strip_suffix("grad") {
        num(n)? * std::f32::consts::PI / 200.0
    } else if let Some(n) = v.strip_suffix("rad") {
        num(n)?
    } else if let Some(n) = v.strip_suffix("turn") {
        num(n)? * std::f32::consts::TAU
    } else {
        // Unitless 0 is allowed.
        let n = num(v)?;
        if n != 0.0 {
            return None;
        }
        0.0
    })
}

/// A length or percentage of `basis`.
fn length(v: &str, basis: f32, fs: f32) -> Option<f32> {
    let v = v.trim();
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|p| p / 100.0 * basis);
    }
    match crate::style::parse_len(v, fs)? {
        crate::style::Len::Auto => None,
        l => Some(l.resolve(basis)),
    }
}

/// The element's transform for a border box `w`×`h`, in its own coordinates
/// (origin at the box's top-left), with `transform-origin` applied.
pub fn matrix(spec: &str, origin: &str, w: f32, h: f32, fs: f32) -> Option<Matrix> {
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("none") {
        return None;
    }
    let mut m = IDENTITY;
    let mut rest = spec;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().to_ascii_lowercase();
        let close = rest[open..].find(')')? + open;
        let args: Vec<String> = css::split_top(&rest[open + 1..close], ',').iter().map(|a| a.trim().to_string()).collect();
        let arg = |i: usize| args.get(i).map(String::as_str);
        let num = |i: usize| arg(i).and_then(|a| a.parse::<f32>().ok());
        let f: Matrix = match name.as_str() {
            "translate" | "translate3d" => translate(length(arg(0)?, w, fs)?, arg(1).and_then(|a| length(a, h, fs)).unwrap_or(0.0)),
            "translatex" => translate(length(arg(0)?, w, fs)?, 0.0),
            "translatey" => translate(0.0, length(arg(0)?, h, fs)?),
            "scale" | "scale3d" => {
                let sx = num(0)?;
                [sx, 0.0, 0.0, num(1).unwrap_or(sx), 0.0, 0.0]
            }
            "scalex" => [num(0)?, 0.0, 0.0, 1.0, 0.0, 0.0],
            "scaley" => [1.0, 0.0, 0.0, num(0)?, 0.0, 0.0],
            "rotate" | "rotatez" => {
                let a = angle(arg(0)?)?;
                let (s, c) = a.sin_cos();
                [c, s, -s, c, 0.0, 0.0]
            }
            "skew" => [1.0, angle(arg(1).unwrap_or("0"))?.tan(), angle(arg(0)?)?.tan(), 1.0, 0.0, 0.0],
            "skewx" => [1.0, 0.0, angle(arg(0)?)?.tan(), 1.0, 0.0, 0.0],
            "skewy" => [1.0, angle(arg(0)?)?.tan(), 0.0, 1.0, 0.0, 0.0],
            "matrix" => [num(0)?, num(1)?, num(2)?, num(3)?, num(4)?, num(5)?],
            // 3D-only functions have no visible effect in a flat rendering.
            _ => IDENTITY,
        };
        m = mul(m, f);
        rest = &rest[close + 1..];
    }
    // transform-origin: defaults to the center of the box.
    let mut parts = origin.split_whitespace().filter(|p| !p.is_empty());
    let pos = |v: Option<&str>, basis: f32, vertical: bool| -> f32 {
        match v {
            None | Some("center") => basis / 2.0,
            Some("left") if !vertical => 0.0,
            Some("right") if !vertical => basis,
            Some("top") if vertical => 0.0,
            Some("bottom") if vertical => basis,
            Some(v) => length(v, basis, fs).unwrap_or(basis / 2.0),
        }
    };
    let (mut first, mut second) = (parts.next(), parts.next());
    // "top left" style keywords may come in either order.
    if matches!(first, Some("top" | "bottom")) || matches!(second, Some("left" | "right")) {
        std::mem::swap(&mut first, &mut second);
    }
    let (ox, oy) = (pos(first, w, false), pos(second, h, true));
    Some(mul(translate(ox, oy), mul(m, translate(-ox, -oy))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transforms() {
        let m = matrix("translate(-50%, -50%)", "", 200.0, 100.0, 16.0).unwrap();
        assert!(is_translation(&m) && (m[4] + 100.0).abs() < 1e-4 && (m[5] + 50.0).abs() < 1e-4);
        // Rotating 90° about the center maps the top-left corner to the top-right.
        let m = matrix("rotate(90deg)", "", 100.0, 100.0, 16.0).unwrap();
        let (x, y) = apply(&m, 0.0, 0.0);
        assert!((x - 100.0).abs() < 1e-3 && y.abs() < 1e-3);
        let m = matrix("scale(2)", "top left", 50.0, 50.0, 16.0).unwrap();
        assert_eq!(apply(&m, 10.0, 10.0), (20.0, 20.0));
        let inv = invert(&matrix("rotate(30deg) scale(1.5)", "", 80.0, 40.0, 16.0).unwrap()).unwrap();
        let fwd = matrix("rotate(30deg) scale(1.5)", "", 80.0, 40.0, 16.0).unwrap();
        let (x, y) = apply(&inv, apply(&fwd, 7.0, 9.0).0, apply(&fwd, 7.0, 9.0).1);
        assert!((x - 7.0).abs() < 1e-3 && (y - 9.0).abs() < 1e-3);
    }
}
