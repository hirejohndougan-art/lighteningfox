//! `<foreignObject>` in inline `<svg>`: the HTML inside is laid out like any other HTML, in
//! the SVG's user units, at the foreignObject's x/y/width/height, and drawn over the drawing
//! through the same matrix (viewBox and `transform`s) that places the SVG's own shapes.
//! The renderer (resvg) leaves foreignObject out of the drawing itself.

use crate::dom::{Dom, NodeData, NodeId, Ns};
use crate::transform::{mul, Matrix, IDENTITY};

/// The `<foreignObject>`s of the `<svg>` at `root`, in tree order (not ones inside another).
pub fn find(dom: &Dom, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = dom.nodes[root].children.iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        if dom.ns_of(n) != Ns::Svg {
            continue;
        }
        if dom.tag(n) == Some("foreignObject") {
            out.push(n);
            continue;
        }
        if matches!(dom.nodes[n].data, NodeData::Element { .. }) {
            stack.extend(dom.nodes[n].children.iter().rev());
        }
    }
    out
}

fn len(dom: &Dom, id: NodeId, name: &str) -> f32 {
    dom.attr(id, name)
        .map(|v| v.trim())
        .map(|v| v.strip_suffix("px").unwrap_or(v).trim())
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
}

/// The foreignObject's rectangle in its own user units: (x, y, width, height).
pub fn rect(dom: &Dom, id: NodeId) -> (f32, f32, f32, f32) {
    (len(dom, id, "x"), len(dom, id, "y"), len(dom, id, "width").max(0.0), len(dom, id, "height").max(0.0))
}

/// From the foreignObject's user units to the root `<svg>`'s: its own `transform` and those of
/// the elements between it and the root.
pub fn to_root(dom: &Dom, root: NodeId, id: NodeId) -> Matrix {
    let mut m = IDENTITY;
    let mut n = Some(id);
    while let Some(k) = n.filter(|&k| k != root) {
        if let Some(t) = dom.attr(k, "transform") {
            m = mul(parse_transform(t), m);
        }
        n = dom.nodes[k].parent;
    }
    m
}

/// An SVG `transform` attribute (`translate(10 20) rotate(45 5 5) scale(2)`…) as a matrix.
pub fn parse_transform(s: &str) -> Matrix {
    let mut m = IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim().to_ascii_lowercase();
        let Some(close) = rest[open..].find(')') else { break };
        let args: Vec<f32> = rest[open + 1..open + close]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|a| !a.is_empty())
            .filter_map(|a| a.parse().ok())
            .collect();
        rest = &rest[open + close + 1..];
        let a = |i: usize, d: f32| args.get(i).copied().unwrap_or(d);
        let f: Matrix = match name.as_str() {
            "matrix" if args.len() == 6 => [args[0], args[1], args[2], args[3], args[4], args[5]],
            "translate" => [1.0, 0.0, 0.0, 1.0, a(0, 0.0), a(1, 0.0)],
            "scale" => [a(0, 1.0), 0.0, 0.0, a(1, a(0, 1.0)), 0.0, 0.0],
            "rotate" => {
                let (s, c) = a(0, 0.0).to_radians().sin_cos();
                let (cx, cy) = (a(1, 0.0), a(2, 0.0));
                let r = [c, s, -s, c, 0.0, 0.0];
                mul([1.0, 0.0, 0.0, 1.0, cx, cy], mul(r, [1.0, 0.0, 0.0, 1.0, -cx, -cy]))
            }
            "skewx" => [1.0, 0.0, a(0, 0.0).to_radians().tan(), 1.0, 0.0, 0.0],
            "skewy" => [1.0, a(0, 0.0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
            _ => continue,
        };
        m = mul(m, f);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_and_rects() {
        let m = parse_transform("translate(10, 20) scale(2)");
        assert_eq!(crate::transform::apply(&m, 1.0, 1.0), (12.0, 22.0));
        let r = parse_transform("rotate(90)");
        let (x, y) = crate::transform::apply(&r, 1.0, 0.0);
        assert!(x.abs() < 1e-5 && (y - 1.0).abs() < 1e-5);
        let dom = crate::html::parse(r#"<svg viewBox="0 0 100 100"><g transform="translate(5 0)"><foreignObject x="1" y="2" width="30" height="40"><p>hi</p></foreignObject></g></svg>"#);
        let root = (0..dom.nodes.len()).find(|&n| crate::svg_inline::is_root(&dom, n)).unwrap();
        let fo = find(&dom, root);
        assert_eq!(fo.len(), 1);
        assert_eq!(rect(&dom, fo[0]), (1.0, 2.0, 30.0, 40.0));
        assert_eq!(to_root(&dom, root, fo[0]), [1.0, 0.0, 0.0, 1.0, 5.0, 0.0]);
    }
}
