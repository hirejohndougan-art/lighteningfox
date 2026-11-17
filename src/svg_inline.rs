//! SVG written directly in HTML pages (`<svg>…</svg>` inside the body).
//!
//! Each outermost `<svg>` element is drawn like a picture. Before layout, `publish` writes the
//! element's subtree back out as an SVG document and hands it to the page's images as
//! `inline-svg:<node id>`; the SVG image renderer (resvg, as for `<img src="x.svg">`) draws it.
//! The page's CSS for the SVG elements (fill, stroke, colors, fonts, display…, as the cascade
//! decided them, so `.icon:hover path { fill: red }` and `currentColor` work) goes into each
//! element's `style` attribute. Pieces the SVG refers to elsewhere in the page (a sprite sheet's
//! `<symbol>` used with `<use href="#icon">`, a shared gradient) are copied into its `<defs>`.
//! Layout treats the element as a replaced box sized by `natural_size`.

use crate::dom::{Dom, NodeData, NodeId, Ns};
use crate::style::{Display, ObjectFit, Style};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// CSS properties that style SVG content.
const PROPS: &[&str] = &[
    "fill", "fill-opacity", "fill-rule", "stroke", "stroke-width", "stroke-opacity", "stroke-linecap", "stroke-linejoin",
    "stroke-miterlimit", "stroke-dasharray", "stroke-dashoffset", "opacity", "color", "display", "visibility", "stop-color",
    "stop-opacity", "font", "font-family", "font-size", "font-weight", "font-style", "font-variant", "font-stretch",
    "text-anchor", "dominant-baseline", "alignment-baseline", "baseline-shift", "letter-spacing", "word-spacing",
    "text-decoration", "writing-mode", "transform", "transform-origin", "transform-box", "clip-path", "clip-rule", "mask",
    "filter", "marker", "marker-start", "marker-mid", "marker-end", "paint-order", "shape-rendering", "text-rendering",
    "image-rendering", "vector-effect", "mix-blend-mode", "isolation", "flood-color", "flood-opacity", "lighting-color",
    "color-interpolation", "color-interpolation-filters", "overflow",
];

/// Properties of the outermost `<svg>` that the HTML layout handles itself.
const ROOT_SKIP: &[&str] = &["display", "transform", "transform-origin", "opacity", "filter", "overflow", "visibility"];

pub fn is_svg_prop(name: &str) -> bool {
    PROPS.contains(&name)
}

/// The SVG properties among an element's matched declarations (lowest priority first), with
/// `var()` filled in. Called by the cascade for elements in the SVG namespace.
pub fn declared<'a>(decls: impl Iterator<Item = &'a crate::css::Decl>, vars: &HashMap<String, String>) -> Option<Rc<Vec<(String, String)>>> {
    let mut out: Vec<(String, String)> = Vec::new();
    for d in decls.filter(|d| is_svg_prop(&d.name)) {
        let v = if d.value.contains("var(") { crate::style::substitute_vars(&d.value, vars, 0) } else { d.value.clone() };
        out.retain(|(n, _)| *n != d.name);
        out.push((d.name.clone(), v.trim().to_string()));
    }
    (!out.is_empty()).then(|| Rc::new(out))
}

/// Presentation attributes that CSS transitions and animations start from when no rule sets
/// the property (`transform` is left out: its attribute syntax isn't CSS's).
const PRESENTATION: &[&str] = &["fill", "fill-opacity", "stroke", "stroke-width", "stroke-opacity", "stroke-dashoffset", "opacity", "stop-color", "stop-opacity", "color"];

pub fn presentation_attrs(dom: &Dom, id: NodeId) -> Vec<(String, String)> {
    PRESENTATION.iter().filter_map(|&n| dom.attr(id, n).map(|v| (n.to_string(), v.trim().to_string()))).collect()
}

/// An element's SVG declarations with the animated values (anim.rs) on top.
pub fn with_animated(decls: Option<Rc<Vec<(String, String)>>>, animated: Vec<(String, String)>) -> Option<Rc<Vec<(String, String)>>> {
    let mut out: Vec<(String, String)> = decls.map(|d| (*d).clone()).unwrap_or_default();
    for (n, v) in animated {
        out.retain(|(k, _)| *k != n);
        out.push((n, v));
    }
    Some(Rc::new(out))
}

/// An outermost `<svg>`: one in the SVG namespace whose parent isn't.
pub fn is_root(dom: &Dom, id: NodeId) -> bool {
    dom.tag(id) == Some("svg") && dom.ns_of(id) == Ns::Svg && dom.nodes[id].parent.is_none_or(|p| dom.ns_of(p) != Ns::Svg)
}

/// The image key an inline `<svg>` is drawn from.
pub fn key(id: NodeId) -> String {
    format!("inline-svg:{id}")
}

fn view_box(dom: &Dom, id: NodeId) -> Option<[f32; 4]> {
    let v: Vec<f32> = dom.attr(id, "viewBox")?.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    (v.len() == 4 && v[2] > 0.0 && v[3] > 0.0).then(|| [v[0], v[1], v[2], v[3]])
}

/// A width/height attribute in CSS px; percentages of `pct_of` (None: unknown).
fn len_attr(dom: &Dom, id: NodeId, name: &str, pct_of: Option<f32>) -> Option<f32> {
    let v = dom.attr(id, name)?.trim();
    if let Some(p) = v.strip_suffix('%') {
        return Some(p.trim().parse::<f32>().ok()? * pct_of? / 100.0);
    }
    let v = v.strip_suffix("px").unwrap_or(v).trim();
    v.parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0)
}

/// The box an inline `<svg>` takes when CSS doesn't size it: its width and height attributes;
/// a missing one follows the viewBox's aspect ratio (as wide as `avail` when both are missing,
/// as in other browsers); 300×150 without a viewBox.
pub fn natural_size(dom: &Dom, id: NodeId, avail: f32) -> (f32, f32) {
    let w = len_attr(dom, id, "width", Some(avail));
    let h = len_attr(dom, id, "height", None);
    let ratio = view_box(dom, id).map(|v| v[2] / v[3]);
    match (w, h, ratio) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some(r)) => (w, w / r),
        (None, Some(h), Some(r)) => (h * r, h),
        (None, None, Some(r)) => (avail.max(0.0), avail.max(0.0) / r),
        (w, h, None) => (w.unwrap_or(300.0), h.unwrap_or(150.0)),
    }
}

/// How the drawing sits in the element's box (`preserveAspectRatio`): kept in proportion and
/// centred, unless it is `none`.
pub fn fit(dom: &Dom, id: NodeId) -> ObjectFit {
    let par = dom.attr(id, "preserveAspectRatio").unwrap_or("").trim();
    if view_box(dom, id).is_none() || par.starts_with("none") {
        ObjectFit::Fill
    } else if par.ends_with("slice") {
        ObjectFit::Cover
    } else {
        ObjectFit::Contain
    }
}

thread_local! {
    /// What was last handed to the images under each key (a hash of the document).
    static PUBLISHED: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
}

/// Write every shown inline `<svg>` out to the page's images (only the ones that changed).
pub fn publish(dom: &Dom, styles: &[Rc<Style>], images: &mut crate::images::Images) {
    if dom.ns.is_empty() {
        return;
    }
    let mut roots: Vec<NodeId> = dom.ns.iter().filter(|(_, ns)| **ns == Ns::Svg).map(|(id, _)| *id).filter(|&id| id < dom.nodes.len() && is_root(dom, id)).collect();
    roots.sort_unstable();
    let mut by_id: Option<HashMap<&str, NodeId>> = None;
    for id in roots {
        if styles.get(id).is_none_or(|s| s.display == Display::None) {
            continue;
        }
        let doc = document(dom, styles, id, &mut by_id);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            doc.hash(&mut h);
            h.finish()
        };
        let key = key(id);
        let same = PUBLISHED.with(|p| p.borrow().get(&key) == Some(&hash));
        if same && images.get(&key).is_some() {
            continue;
        }
        if !images.insert_svg(&key, doc.as_bytes()) {
            log!("[svg] couldn't draw the <svg> at node {id}");
        }
        PUBLISHED.with(|p| p.borrow_mut().insert(key, hash));
    }
}

fn escape(s: &str, attr: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
}

/// Attribute names XML accepts (frameworks leave names like `@click` or `:class` behind).
fn xml_name(name: &str) -> bool {
    let mut cs = name.chars();
    let first_ok = cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let rest_ok = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'));
    let prefix_ok = match name.split_once(':') {
        None => true,
        Some((p, local)) => matches!(p, "xlink" | "xml") && !local.is_empty() && !local.contains(':'),
    };
    first_ok && rest_ok && prefix_ok && !name.starts_with("xmlns") && !name.starts_with("on")
}

/// The ids an attribute or style value points at: `url(#x)`, or `#x` in an href.
fn refs_in(name: &str, value: &str, out: &mut Vec<String>) {
    let mut rest = value;
    while let Some(p) = rest.find("url(") {
        rest = &rest[p + 4..];
        let inner = rest.split(')').next().unwrap_or("").trim().trim_matches(['"', '\'']);
        if let Some(id) = inner.strip_prefix('#') {
            out.push(id.to_string());
        }
    }
    if name == "href" || name == "xlink:href" {
        if let Some(id) = value.trim().strip_prefix('#') {
            out.push(id.to_string());
        }
    }
}

fn css_color(c: crate::style::Color) -> String {
    format!("rgba({},{},{},{:.3})", c.r, c.g, c.b, c.a as f32 / 255.0)
}

/// One element and its subtree as SVG markup.
fn element(dom: &Dom, styles: &[Rc<Style>], id: NodeId, root: bool, out: &mut String, refs: &mut Vec<String>) {
    let NodeData::Element { tag, attrs } = &dom.nodes[id].data else { return };
    out.push('<');
    out.push_str(tag);
    let mut seen: HashSet<&str> = HashSet::new();
    let mut style = String::new();
    if root {
        out.push_str(" xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
        // The drawing's own size: its viewBox (layout fits it to the box), else its size
        // attributes or 300×150, with a viewBox added so it scales to the box.
        let (w, h) = match view_box(dom, id) {
            Some(v) => (v[2], v[3]),
            None => {
                let (w, h) = natural_size(dom, id, 300.0);
                let (w, h) = (w.max(1.0), h.max(1.0));
                out.push_str(&format!(" viewBox=\"0 0 {w} {h}\""));
                (w, h)
            }
        };
        out.push_str(&format!(" width=\"{w}\" height=\"{h}\""));
        seen.extend(["width", "height"]);
        // What the drawing inherits from the page: its color (for currentColor) and font.
        if let Some(st) = styles.get(id) {
            style.push_str(&format!("color:{};font-size:{}px;", css_color(st.color), st.font_size));
        }
    }
    // Every element gets an id, so shapes in the drawing can be traced back to their element
    // (svg_hit.rs).
    if dom.attr(id, "id").is_none_or(str::is_empty) {
        out.push_str(&format!(" id=\"{}\"", auto_id(id)));
        seen.insert("id");
    }
    for (k, v) in attrs {
        if k == "style" || !xml_name(k) || !seen.insert(k.as_str()) {
            continue;
        }
        refs_in(k, v, refs);
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape(v, true, out);
        out.push('"');
    }
    if let Some(decls) = styles.get(id).and_then(|s| s.svg_decls.as_ref()) {
        for (n, v) in decls.iter() {
            if root && ROOT_SKIP.contains(&n.as_str()) {
                continue;
            }
            refs_in(n, v, refs);
            style.push_str(n);
            style.push(':');
            if n == "transform" {
                style.push_str(&svg_transform(v.trim_end_matches(';')));
            } else {
                style.push_str(v.trim_end_matches(';'));
            }
            style.push(';');
        }
    }
    if !style.is_empty() {
        out.push_str(" style=\"");
        escape(&style, true, out);
        out.push('"');
    }
    let kids: Vec<NodeId> = dom.nodes[id]
        .children
        .iter()
        .copied()
        .filter(|&c| match &dom.nodes[c].data {
            NodeData::Text(_) => true,
            NodeData::Element { tag, .. } => dom.ns_of(c) == Ns::Svg && !matches!(tag.as_str(), "script" | "foreignObject"),
            _ => false,
        })
        .collect();
    if kids.is_empty() && !root {
        out.push_str("/>");
        return;
    }
    out.push('>');
    if root {
        out.push_str("\u{0}DEFS\u{0}");
    }
    for c in kids {
        match &dom.nodes[c].data {
            NodeData::Text(t) => escape(t, false, out),
            _ => element(dom, styles, c, false, out, refs),
        }
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// A CSS `transform` value in the SVG attribute syntax the renderer reads: units dropped
/// (`rotate(45deg)` → `rotate(45)`, `translate(10px, 0)` → `translate(10 0)`) and the
/// one-axis CSS functions spelled out. 3D functions and percentages are left out.
pub fn svg_transform(css: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for f in crate::css::split_top(css.trim(), ' ') {
        let f = f.trim();
        let Some((name, args)) = f.split_once('(') else { continue };
        let args: Vec<f32> = args
            .trim_end_matches(')')
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|a| !a.is_empty())
            .filter_map(|a| {
                let num: String = a.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e')).collect();
                let v: f32 = num.parse().ok()?;
                Some(match &a[num.len()..] {
                    "" | "px" | "deg" => v,
                    "rad" => v.to_degrees(),
                    "turn" => v * 360.0,
                    "grad" => v * 0.9,
                    _ => return None,
                })
            })
            .collect();
        let a = |i: usize, d: f32| args.get(i).copied().unwrap_or(d);
        let part = match name.trim().to_ascii_lowercase().as_str() {
            "translate" if !args.is_empty() => format!("translate({} {})", a(0, 0.0), a(1, 0.0)),
            "translatex" if !args.is_empty() => format!("translate({} 0)", a(0, 0.0)),
            "translatey" if !args.is_empty() => format!("translate(0 {})", a(0, 0.0)),
            "scale" if !args.is_empty() => format!("scale({} {})", a(0, 1.0), a(1, a(0, 1.0))),
            "scalex" if !args.is_empty() => format!("scale({} 1)", a(0, 1.0)),
            "scaley" if !args.is_empty() => format!("scale(1 {})", a(0, 1.0)),
            "rotate" if !args.is_empty() => format!("rotate({})", a(0, 0.0)),
            "skewx" if !args.is_empty() => format!("skewX({})", a(0, 0.0)),
            "skewy" if !args.is_empty() => format!("skewY({})", a(0, 0.0)),
            "skew" if !args.is_empty() => format!("skewX({}) skewY({})", a(0, 0.0), a(1, 0.0)),
            "matrix" if args.len() == 6 => format!("matrix({})", args.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ")),
            _ => continue,
        };
        out.push(part);
    }
    if out.is_empty() { "translate(0 0)".to_string() } else { out.join(" ") }
}

/// The id an element without one has in the published drawing.
fn auto_id(id: NodeId) -> String {
    format!("__is{id}")
}

/// The elements of the `<svg>` at `root` by their id in the published drawing.
pub fn id_map(dom: &Dom, root: NodeId) -> HashMap<String, NodeId> {
    let mut m = HashMap::new();
    m.insert(dom.attr(root, "id").filter(|s| !s.is_empty()).map_or_else(|| auto_id(root), str::to_string), root);
    dom.walk(root, |n| {
        if matches!(dom.nodes[n].data, NodeData::Element { .. }) {
            let i = dom.attr(n, "id").filter(|s| !s.is_empty()).map_or_else(|| auto_id(n), str::to_string);
            m.entry(i).or_insert(n);
        }
        true
    });
    m
}

/// The `<svg>` at `root` as a standalone SVG document.
fn document<'a>(dom: &'a Dom, styles: &[Rc<Style>], root: NodeId, by_id: &mut Option<HashMap<&'a str, NodeId>>) -> String {
    let mut out = String::new();
    let mut refs = Vec::new();
    element(dom, styles, root, true, &mut out, &mut refs);
    // Ids the drawing has itself.
    let mut have: HashSet<String> = HashSet::new();
    dom.walk(root, |n| {
        if let Some(i) = dom.attr(n, "id") {
            have.insert(i.to_string());
        }
        true
    });
    let mut defs = String::new();
    let mut copied = 0;
    while let Some(r) = refs.pop() {
        if have.contains(&r) || copied > 200 {
            continue;
        }
        have.insert(r.clone());
        let map = by_id.get_or_insert_with(|| {
            let mut m = HashMap::new();
            for (&n, ns) in dom.ns.iter() {
                if *ns == Ns::Svg && n < dom.nodes.len() {
                    if let Some(i) = dom.attr(n, "id") {
                        m.entry(i).or_insert(n);
                    }
                }
            }
            m
        });
        if let Some(&n) = map.get(r.as_str()) {
            copied += 1;
            element(dom, styles, n, false, &mut defs, &mut refs);
        }
    }
    let defs = if defs.is_empty() { defs } else { format!("<defs>{defs}</defs>") };
    out.replacen("\u{0}DEFS\u{0}", &defs, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(html: &str) -> (Dom, Vec<Rc<Style>>) {
        let dom = crate::html::parse(html);
        let sheets: Vec<crate::css::Stylesheet> = dom.find_all("style").iter().map(|&s| crate::css::parse(&dom.text_content(s))).collect();
        let styles = crate::style::compute(&dom, &sheets);
        (dom, styles)
    }

    fn root_of(dom: &Dom) -> NodeId {
        (0..dom.nodes.len()).find(|&i| is_root(dom, i)).unwrap()
    }

    #[test]
    fn writes_out_a_document_with_page_css() {
        let (dom, styles) = page(r##"<style>.i path { fill: red } svg:hover path { fill: blue }</style><p style="color:#00f"><svg class="i" viewBox="0 0 10 10" width="20" @click="x"><path d="M0 0H10V10Z"/><circle r="2" fill="currentColor"/></svg>"##);
        let id = root_of(&dom);
        assert_eq!(natural_size(&dom, id, 500.0), (20.0, 20.0));
        let doc = document(&dom, &styles, id, &mut None);
        assert!(doc.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""), "{doc}");
        assert!(doc.contains(" d=\"M0 0H10V10Z\" style=\"fill:red;\"/>"), "{doc}");
        assert!(doc.contains("color:rgba(0,0,255,1.000)"), "{doc}");
        assert!(!doc.contains("@click"), "{doc}");
        assert!(resvg::usvg::Tree::from_str(&doc, &Default::default()).is_ok(), "{doc}");
    }

    #[test]
    fn copies_symbols_used_from_elsewhere() {
        let (dom, styles) = page(r##"<svg style="display:none"><symbol id="star" viewBox="0 0 4 4"><rect width="4" height="4" fill="url(#g)"/></symbol><linearGradient id="g"><stop offset="0" stop-color="red"/></linearGradient></svg><svg width="8" height="8"><use href="#star"/></svg>"##);
        let id = (0..dom.nodes.len()).filter(|&i| is_root(&dom, i)).nth(1).unwrap();
        let doc = document(&dom, &styles, id, &mut None);
        assert!(doc.contains("<defs><symbol id=\"star\""), "{doc}");
        assert!(doc.contains("<linearGradient id=\"g\""), "{doc}");
    }

    #[test]
    fn css_transforms_in_svg_syntax() {
        assert_eq!(svg_transform("rotate(45deg)"), "rotate(45)");
        assert_eq!(svg_transform("translate(50px, 0) scale(2)"), "translate(50 0) scale(2 2)");
        assert_eq!(svg_transform("translateX(5px) rotate(0.5turn)"), "translate(5 0) rotate(180)");
        assert_eq!(svg_transform("none"), "translate(0 0)");
    }

    #[test]
    fn sizes() {
        let (dom, _) = page(r#"<svg viewBox="0 0 20 10"></svg>"#);
        assert_eq!(natural_size(&dom, root_of(&dom), 400.0), (400.0, 200.0));
        let (dom, _) = page(r#"<svg></svg>"#);
        assert_eq!(natural_size(&dom, root_of(&dom), 400.0), (300.0, 150.0));
        let (dom, _) = page(r#"<svg width="50%" height="30"></svg>"#);
        assert_eq!(natural_size(&dom, root_of(&dom), 400.0), (200.0, 30.0));
    }
}
