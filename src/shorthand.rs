//! Shorthand and logical properties, expanded into the longhands `style::apply` knows.
//!
//! Shorthands that `apply` already handles as a whole (`margin`, `padding`, `border`,
//! `background`, `font`, `flex`, `grid-*`, `inset`, `columns`, `gap`, `list-style`,
//! `overflow`, `text-decoration`, `transition`, `animation`) aren't listed here. This covers
//! the rest: the logical properties (`margin-inline`, `inset-block-start`,
//! `border-inline-end-color`, `inline-size`…), per-side and per-part border shorthands,
//! `place-*`, and multi-value `border-style`/`border-color`.

/// The longhands a property stands for, or None if `apply` handles it directly.
/// `rtl`: the element's writing direction (inline start is the right side when true).
pub fn expand(name: &str, value: &str, rtl: bool) -> Option<Vec<(String, String)>> {
    let v = value.trim();
    let (start, end) = if rtl { ("right", "left") } else { ("left", "right") };
    let one = |n: &str| Some(vec![(n.to_string(), v.to_string())]);
    let words: Vec<String> = crate::css::split_top(v, ' ').into_iter().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
    // `x y` → (x, y); `x` → (x, x).
    let pair = || -> (String, String) {
        match words.as_slice() {
            [a] => (a.clone(), a.clone()),
            [a, b, ..] => (a.clone(), b.clone()),
            [] => (v.to_string(), v.to_string()),
        }
    };
    let two = |a: String, av: String, b: String, bv: String| Some(vec![(a, av), (b, bv)]);
    match name {
        // Sizes.
        "inline-size" => one("width"),
        "block-size" => one("height"),
        "min-inline-size" => one("min-width"),
        "max-inline-size" => one("max-width"),
        "min-block-size" => one("min-height"),
        "max-block-size" => one("max-height"),
        // margin / padding / inset, logical.
        "margin-inline" | "padding-inline" | "inset-inline" => {
            let (a, b) = pair();
            let base = base_of(name);
            two(side(base, start), a, side(base, end), b)
        }
        "margin-block" | "padding-block" | "inset-block" => {
            let (a, b) = pair();
            let base = base_of(name);
            two(side(base, "top"), a, side(base, "bottom"), b)
        }
        "margin-inline-start" | "padding-inline-start" | "inset-inline-start" => one(&side(base_of(name), start)),
        "margin-inline-end" | "padding-inline-end" | "inset-inline-end" => one(&side(base_of(name), end)),
        "margin-block-start" | "padding-block-start" | "inset-block-start" => one(&side(base_of(name), "top")),
        "margin-block-end" | "padding-block-end" | "inset-block-end" => one(&side(base_of(name), "bottom")),
        // Borders, logical: border-inline[-start|-end][-width|-color|-style], border-block…
        n if n.starts_with("border-inline") || n.starts_with("border-block") => {
            let rest = n.trim_start_matches("border-");
            let (axis, rest) = rest.split_once('-').map(|(a, r)| (a, r)).unwrap_or((rest, ""));
            let (which, part) = match rest {
                "" => ("both", ""),
                "start" | "end" => (rest, ""),
                r => match r.split_once('-') {
                    Some((w @ ("start" | "end"), p)) => (w, p),
                    _ => ("both", r),
                },
            };
            let sides: Vec<&str> = match (axis, which) {
                ("inline", "start") => vec![start],
                ("inline", "end") => vec![end],
                ("inline", _) => vec![start, end],
                (_, "start") => vec!["top"],
                (_, "end") => vec!["bottom"],
                _ => vec!["top", "bottom"],
            };
            // `border-inline-width: 1px 2px` gives start and end their own values.
            let vals: Vec<String> = if which == "both" && !part.is_empty() && words.len() == 2 { words.clone() } else { vec![v.to_string(); sides.len()] };
            Some(
                sides
                    .iter()
                    .zip(vals)
                    .map(|(s, val)| (if part.is_empty() { format!("border-{s}") } else { format!("border-{s}-{part}") }, val))
                    .collect(),
            )
        }
        // border-style with 1–4 values; border-<side>-style.
        "border-style" if words.len() > 1 => {
            let f = four(&words);
            Some(["top", "right", "bottom", "left"].iter().zip(f).map(|(s, w)| (format!("border-{s}-style"), w)).collect())
        }
        "border-color" if words.len() > 1 => {
            let f = four(&words);
            Some(["top", "right", "bottom", "left"].iter().zip(f).map(|(s, w)| (format!("border-{s}-color"), w)).collect())
        }
        // Logical radius corners.
        "border-start-start-radius" => one(&format!("border-top-{start}-radius")),
        "border-start-end-radius" => one(&format!("border-top-{end}-radius")),
        "border-end-start-radius" => one(&format!("border-bottom-{start}-radius")),
        "border-end-end-radius" => one(&format!("border-bottom-{end}-radius")),
        // place-*: align then justify.
        "place-content" => {
            let (a, b) = pair();
            two("align-content".into(), a, "justify-content".into(), b)
        }
        "place-self" => {
            let (a, b) = pair();
            two("align-self".into(), a, "justify-self".into(), b)
        }
        "place-items" => {
            let (a, b) = pair();
            two("align-items".into(), a, "justify-items".into(), b)
        }
        // Old names.
        "word-wrap" => one("overflow-wrap"),
        "grid-row-gap" => one("row-gap"),
        "grid-column-gap" => one("column-gap"),
        "grid-gap" => one("gap"),
        _ => None,
    }
}

fn base_of(name: &str) -> &str {
    name.split('-').next().unwrap_or(name)
}

/// `margin` + `left` → `margin-left`; `inset` + `left` → `left`.
fn side(base: &str, s: &str) -> String {
    if base == "inset" { s.to_string() } else { format!("{base}-{s}") }
}

/// 1–4 values to top, right, bottom, left.
fn four(w: &[String]) -> Vec<String> {
    match w {
        [a] => vec![a.clone(); 4],
        [a, b] => vec![a.clone(), b.clone(), a.clone(), b.clone()],
        [a, b, c] => vec![a.clone(), b.clone(), c.clone(), b.clone()],
        [a, b, c, d, ..] => vec![a.clone(), b.clone(), c.clone(), d.clone()],
        [] => vec![String::new(); 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(n: &str, v: &str, rtl: bool) -> Vec<(String, String)> {
        expand(n, v, rtl).unwrap()
    }

    #[test]
    fn logical_and_shorthands() {
        assert_eq!(ex("margin-inline", "1px 2px", false), vec![("margin-left".into(), "1px".into()), ("margin-right".into(), "2px".into())]);
        assert_eq!(ex("margin-inline-start", "3px", true), vec![("margin-right".into(), "3px".into())]);
        assert_eq!(ex("padding-block", "4px", false), vec![("padding-top".into(), "4px".into()), ("padding-bottom".into(), "4px".into())]);
        assert_eq!(ex("inset-inline-end", "0", false), vec![("right".into(), "0".into())]);
        assert_eq!(ex("border-inline-start", "1px solid red", false), vec![("border-left".into(), "1px solid red".into())]);
        assert_eq!(ex("border-block-end-color", "red", false), vec![("border-bottom-color".into(), "red".into())]);
        assert_eq!(ex("border-inline-width", "1px 2px", false), vec![("border-left-width".into(), "1px".into()), ("border-right-width".into(), "2px".into())]);
        assert_eq!(ex("border-style", "solid none", false)[1], ("border-right-style".into(), "none".into()));
        assert_eq!(ex("place-self", "center end", false), vec![("align-self".into(), "center".into()), ("justify-self".into(), "end".into())]);
        assert_eq!(ex("inline-size", "50%", false), vec![("width".into(), "50%".into())]);
        assert!(expand("margin", "0", false).is_none());
    }
}
