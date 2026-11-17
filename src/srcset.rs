//! Responsive images: which picture an `<img>` shows, from `srcset`, `sizes` and an enclosing
//! `<picture>` with `<source>` elements (HTML standard, "selecting an image source").
//!
//! The choice depends on the window width (for `sizes` and `media`), so it's made again
//! whenever images are loaded or the page is laid out.

use crate::dom::{Dom, NodeId};

/// Device pixels per CSS pixel. Ironswift draws at 1:1 (HiDPI comes with batch E).
const DPR: f32 = 1.0;

/// Image types we can show (for `<source type=…>`).
fn supported_type(t: &str) -> bool {
    let t = t.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    t.is_empty()
        || matches!(
            t.as_str(),
            "image/png" | "image/apng" | "image/jpeg" | "image/jpg" | "image/pjpeg" | "image/gif" | "image/webp" | "image/avif" | "image/svg+xml" | "image/bmp" | "image/x-icon" | "image/vnd.microsoft.icon"
        )
}

/// The URL an `<img>` shows right now (as written in the page; resolve against the base),
/// or None if it has none.
pub fn current_src(dom: &Dom, img: NodeId) -> Option<String> {
    current(dom, img).map(|(u, _)| u)
}

/// The URL and its density (image pixels per CSS pixel): a 1280w picture in a 640px slot has
/// density 2 and is shown at half its pixel size.
pub fn current(dom: &Dom, img: NodeId) -> Option<(String, f32)> {
    // A <picture> parent: the first <source> before the <img> whose media and type fit.
    if let Some(parent) = dom.nodes[img].parent.filter(|&p| dom.tag(p) == Some("picture")) {
        for &s in &dom.nodes[parent].children {
            if s == img {
                break;
            }
            if dom.tag(s) != Some("source") {
                continue;
            }
            let Some(srcset) = dom.attr(s, "srcset").filter(|v| !v.trim().is_empty()) else { continue };
            if dom.attr(s, "media").is_some_and(|m| !m.trim().is_empty() && !crate::css::media_matches(m)) {
                continue;
            }
            if dom.attr(s, "type").is_some_and(|t| !supported_type(t)) {
                continue;
            }
            if let Some(u) = choose(srcset, dom.attr(s, "sizes"), None) {
                return Some(u);
            }
        }
    }
    let src = dom.attr(img, "src").map(str::trim).filter(|s| !s.is_empty());
    match dom.attr(img, "srcset").filter(|v| !v.trim().is_empty()) {
        Some(srcset) => choose(srcset, dom.attr(img, "sizes"), src).or_else(|| src.map(|s| (s.to_string(), 1.0))),
        None => src.map(|s| (s.to_string(), 1.0)),
    }
}

/// One `srcset` entry: a URL with a width (`400w`) or density (`2x`) descriptor.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    url: String,
    w: Option<f32>,
    x: Option<f32>,
}

/// Parse a srcset attribute. URLs end at whitespace (so data: URLs with commas work); a
/// comma directly after a URL ends the entry.
fn parse_srcset(v: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let b: Vec<char> = v.chars().collect();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_whitespace() || b[i] == ',') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len() && !b[i].is_whitespace() {
            i += 1;
        }
        let mut url: String = b[start..i].iter().collect();
        let mut descs = String::new();
        if url.ends_with(',') {
            url = url.trim_end_matches(',').to_string();
        } else {
            // Descriptors run to the next comma outside parentheses.
            let mut depth = 0;
            while i < b.len() {
                match b[i] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth <= 0 => break,
                    _ => {}
                }
                descs.push(b[i]);
                i += 1;
            }
        }
        let mut c = Candidate { url, w: None, x: None };
        let mut bad = false;
        for d in descs.split_whitespace() {
            let (num, unit) = d.split_at(d.len().saturating_sub(1));
            match (unit, num.parse::<f32>()) {
                ("w", Ok(n)) if n > 0.0 && c.w.is_none() && c.x.is_none() => c.w = Some(n),
                ("x", Ok(n)) if n > 0.0 && c.x.is_none() && c.w.is_none() => c.x = Some(n),
                ("h", Ok(_)) => {}
                _ => bad = true,
            }
        }
        if !bad && !c.url.is_empty() {
            out.push(c);
        }
    }
    out
}

/// A `sizes` length in CSS px: px, em/rem (16px), vw/vh, or calc() of those.
fn size_len(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let (vw, vh) = crate::css::viewport();
    if let Some(inner) = v.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) {
        // calc(100vw - 2em), calc(50vw + 10px): left to right, + and - only.
        let mut total = 0.0;
        let mut sign = 1.0;
        for tok in inner.split_whitespace() {
            match tok {
                "+" => sign = 1.0,
                "-" => sign = -1.0,
                t => total += sign * size_len(t)?,
            }
        }
        return Some(total);
    }
    let num = |suffix: &str| v.strip_suffix(suffix).and_then(|n| n.trim().parse::<f32>().ok());
    num("px")
        .or_else(|| num("rem").map(|n| n * 16.0))
        .or_else(|| num("em").map(|n| n * 16.0))
        .or_else(|| num("vw").map(|n| n * vw / 100.0))
        .or_else(|| num("vh").map(|n| n * vh / 100.0))
        .or_else(|| (v == "0").then_some(0.0))
}

/// The slot width `sizes` gives: the first entry whose media condition matches.
fn slot_width(sizes: Option<&str>) -> f32 {
    let vw = crate::css::viewport().0;
    let Some(sizes) = sizes else { return vw };
    for entry in crate::css::split_top(sizes, ',') {
        let e = entry.trim();
        if e.is_empty() {
            continue;
        }
        // "(max-width: 600px) 480px": the length is after the last ')'.
        let (cond, len) = match e.rfind(')') {
            Some(i) if e.starts_with('(') || e.to_ascii_lowercase().starts_with("not") || e.to_ascii_lowercase().starts_with("screen") => (Some(&e[..=i]), &e[i + 1..]),
            _ => (None, e),
        };
        if len.trim() == "auto" {
            continue;
        }
        if cond.is_none_or(crate::css::media_matches) {
            if let Some(px) = size_len(len) {
                return px.max(0.0);
            }
        }
    }
    vw
}

/// Pick the candidate with the smallest density that's still sharp at our pixel ratio;
/// the densest one if none is. Returns its URL and density.
fn choose(srcset: &str, sizes: Option<&str>, fallback_src: Option<&str>) -> Option<(String, f32)> {
    let mut cands = parse_srcset(srcset);
    if cands.is_empty() {
        return None;
    }
    let slot = slot_width(sizes).max(1.0);
    // A plain src counts as the 1x candidate when srcset uses densities and has no 1x.
    if let Some(src) = fallback_src {
        if cands.iter().all(|c| c.w.is_none() && c.x.unwrap_or(1.0) != 1.0) {
            cands.push(Candidate { url: src.to_string(), w: None, x: Some(1.0) });
        }
    }
    let density = |c: &Candidate| match (c.w, c.x) {
        (Some(w), _) => w / slot,
        (_, Some(x)) => x,
        _ => 1.0,
    };
    let mut best: Option<(&Candidate, f32)> = None;
    for c in &cands {
        let d = density(c);
        best = match best {
            None => Some((c, d)),
            Some((_, bd)) if bd < DPR && d > bd => Some((c, d)),
            Some((_, bd)) if d >= DPR && d < bd => Some((c, d)),
            other => other,
        };
    }
    best.map(|(c, d)| (c.url.clone(), d))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The viewport is global: tests that change it take turns.
    static VIEW: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn with_viewport<T>(w: f32, f: impl FnOnce() -> T) -> T {
        let _g = VIEW.lock().unwrap_or_else(|e| e.into_inner());
        crate::css::set_viewport(w, 800.0);
        f()
    }

    #[test]
    fn parses_srcset() {
        let c = parse_srcset("a.jpg 400w, b.jpg 800w,c.jpg 1x , data:image/png;base64,iVBO,xyz 2x");
        assert_eq!(c.len(), 4);
        assert_eq!(c[0], Candidate { url: "a.jpg".into(), w: Some(400.0), x: None });
        assert_eq!(c[2].x, Some(1.0));
        assert_eq!(c[3].url, "data:image/png;base64,iVBO,xyz");
        assert_eq!(c[3].x, Some(2.0));
        // Bad descriptors drop the entry.
        assert!(parse_srcset("a.jpg 4q").is_empty());
    }

    #[test]
    fn widths_follow_the_slot() {
        with_viewport(1000.0, || {
            let s = "s.jpg 320w, m.jpg 640w, l.jpg 1280w";
            assert_eq!(choose(s, None, None).unwrap().0, "l.jpg");
            assert_eq!(choose(s, Some("(max-width: 2000px) 300px, 100vw"), None).unwrap().0, "s.jpg");
            assert_eq!(choose(s, Some("(max-width: 500px) 300px, 50vw"), None).unwrap().0, "m.jpg");
            assert_eq!(choose(s, Some("calc(50vw - 200px)"), None).unwrap().0, "s.jpg");
        });
    }

    #[test]
    fn densities_prefer_1x() {
        with_viewport(1000.0, || {
            assert_eq!(choose("a.png 1x, b.png 2x", None, None).unwrap().0, "a.png");
            assert_eq!(choose("b.png 2x", None, Some("a.png")).unwrap().0, "a.png");
            assert_eq!(choose("b.png 2x, c.png 3x", None, None).unwrap().0, "b.png");
        });
    }

    #[test]
    fn picture_sources_by_media_and_type() {
        with_viewport(1000.0, || {
            let dom = crate::html::parse(
                r#"<picture>
                  <source srcset="huge.jxl" type="image/jxl">
                  <source srcset="narrow.webp" media="(max-width: 600px)">
                  <source srcset="wide.avif" type="image/avif" media="(min-width: 601px)">
                  <img id=i src="fallback.jpg">
                </picture><img id=j src="plain.png">"#,
            );
            let imgs = dom.find_all("img");
            assert_eq!(current_src(&dom, imgs[0]).unwrap(), "wide.avif");
            assert_eq!(current_src(&dom, imgs[1]).unwrap(), "plain.png");
        });
        with_viewport(400.0, || {
            let dom = crate::html::parse(r#"<picture><source srcset="narrow.webp" media="(max-width: 600px)"><img src="f.jpg"></picture>"#);
            assert_eq!(current_src(&dom, dom.find_all("img")[0]).unwrap(), "narrow.webp");
        });
    }
}
