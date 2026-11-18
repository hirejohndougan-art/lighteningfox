//! Which @font-face faces to download first. Pages often declare many more faces than they
//! use (NYT: every weight from 200 to 900 of several families, upright and italic), and only a
//! limited number are fetched. Faces were taken in stylesheet order, so the ones the text
//! actually needed could be left out (NYT's franklin 500 and 700, which then fell back to the
//! 300 face: no bold anywhere). Now the face that CSS font matching would pick for each
//! (family, weight, style) the page's styles use comes first, then the rest in order.

use crate::css::FontFace;

/// A family (lower case), weight and italic flag some element's style asks for.
pub type Want = (String, u16, bool);

/// How far a face's weight range is from `w`, as CSS Fonts §5.2 orders candidates
/// (smaller is better; 0 is inside the range).
fn weight_rank(range: (u16, u16), w: u16) -> u32 {
    let (lo, hi) = (range.0.min(range.1), range.0.max(range.1));
    if (lo..=hi).contains(&w) {
        return 0;
    }
    let (above, d) = if lo > w { (true, (lo - w) as u32) } else { (false, (w - hi) as u32) };
    // 400–500: first heavier up to 500, then lighter, then heavier than 500.
    // Under 400: lighter first, then heavier. Over 500: heavier first, then lighter.
    match (w, above) {
        (400..=500, true) if lo <= 500 => d,
        (400..=500, false) => 1_000 + d,
        (400..=500, true) => 2_000 + d,
        (0..=399, false) => d,
        (0..=399, true) => 1_000 + d,
        (_, true) => d,
        (_, false) => 1_000 + d,
    }
}

/// The face font matching picks in `faces` for one family, weight and style, if any.
pub fn best<'a>(faces: &[&'a FontFace], want: &Want) -> Option<&'a FontFace> {
    faces
        .iter()
        .copied()
        .filter(|f| f.family == want.0)
        // The right style first, then the nearest weight.
        .min_by_key(|f| ((f.italic != want.2) as u32, weight_rank(f.weight, want.1)))
}

/// `faces` reordered: the best match for each wanted style first (in the order the wants
/// come), then every other face in its original order.
pub fn prioritize<'a>(faces: &[&'a FontFace], wants: &[Want]) -> Vec<&'a FontFace> {
    let mut out: Vec<&FontFace> = Vec::new();
    for w in wants {
        if let Some(f) = best(faces, w) {
            if !out.iter().any(|o| std::ptr::eq(*o, f)) {
                out.push(f);
            }
        }
    }
    for f in faces {
        if !out.iter().any(|o| std::ptr::eq(*o, *f)) {
            out.push(f);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(w: u16, italic: bool) -> FontFace {
        FontFace { family: "f".into(), src: format!("url(f{w}{italic}.woff2)"), weight: (w, w), italic, unicode_range: Vec::new() }
    }

    #[test]
    fn picks_like_css_font_matching() {
        let all: Vec<FontFace> = [300, 500, 700].iter().flat_map(|&w| [face(w, false), face(w, true)]).collect();
        let faces: Vec<&FontFace> = all.iter().collect();
        let pick = |w: u16, i: bool| best(&faces, &("f".into(), w, i)).map(|f| (f.weight.0, f.italic));
        assert_eq!(pick(700, false), Some((700, false)));
        assert_eq!(pick(400, false), Some((500, false)), "400 looks up to 500 first");
        assert_eq!(pick(600, true), Some((700, true)), "over 500 looks heavier first");
        assert_eq!(pick(200, false), Some((300, false)), "nothing lighter: the next heavier");
        assert!(best(&faces, &("other".into(), 400, false)).is_none());
    }

    #[test]
    fn used_faces_come_first() {
        let all: Vec<FontFace> = (2..=9).flat_map(|w| [face(w * 100, true), face(w * 100, false)]).collect();
        let faces: Vec<&FontFace> = all.iter().collect();
        let order = prioritize(&faces, &[("f".into(), 700, false), ("f".into(), 500, false)]);
        assert_eq!((order[0].weight.0, order[0].italic), (700, false));
        assert_eq!((order[1].weight.0, order[1].italic), (500, false));
        assert_eq!(order.len(), faces.len());
        assert_eq!((order[2].weight.0, order[2].italic), (200, true), "then the rest in order");
    }
}
