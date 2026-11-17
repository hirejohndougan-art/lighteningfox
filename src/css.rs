//! CSS parser: stylesheets -> rules -> selectors + declarations.

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub font_faces: Vec<FontFace>,
    /// `@keyframes` rules, for CSS animations.
    pub keyframes: Vec<crate::anim::Keyframes>,
}

/// An `@font-face` rule: a downloadable font for a family name.
#[derive(Debug, Clone)]
pub struct FontFace {
    /// Lower-case, without quotes.
    pub family: String,
    /// The raw `src` list: `url(…) format("woff2"), url(…) format("woff"), local(…)`.
    pub src: String,
    /// font-weight range it covers (a variable font covers e.g. 100–900).
    pub weight: (u16, u16),
    pub italic: bool,
}

impl FontFace {
    /// The URL to download: the first WOFF2, else WOFF, else TrueType/OpenType source.
    pub fn best_url(&self) -> Option<String> {
        let mut found: Vec<(u8, String)> = Vec::new();
        for part in split_top(&self.src, ',') {
            let p = part.trim();
            let Some(start) = p.find("url(") else { continue };
            let rest = &p[start + 4..];
            let Some(end) = rest.find(')') else { continue };
            let url = rest[..end].trim().trim_matches(|c| c == '"' || c == '\'').to_string();
            let lower = p.to_ascii_lowercase();
            let fmt = lower.split("format(").nth(1).unwrap_or("");
            let rank = if fmt.contains("woff2") || (fmt.is_empty() && url.to_ascii_lowercase().split('?').next().unwrap_or("").ends_with(".woff2")) {
                0
            } else if fmt.contains("woff") || fmt.is_empty() && url.to_ascii_lowercase().split('?').next().unwrap_or("").ends_with(".woff") {
                1
            } else if fmt.contains("truetype") || fmt.contains("opentype") || fmt.is_empty() {
                2
            } else {
                continue; // svg, embedded-opentype: not supported
            };
            found.push((rank, url));
        }
        found.sort_by_key(|f| f.0);
        found.into_iter().next().map(|f| f.1)
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub decls: Vec<Decl>,
    /// The @media condition the rule sits in (nested ones joined with " && "), checked
    /// against the window each time styles are computed.
    pub media: Option<std::sync::Arc<str>>,
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub name: String,
    pub value: String,
    pub important: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Combinator {
    Descendant,
    Child,
    /// `a + b`: b directly after a
    Adjacent,
    /// `a ~ b`: b anywhere after a, same parent
    Sibling,
}

#[derive(Debug, Clone, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    /// [attr], [attr=value], [attr^=value] ... as (name, operator, value)
    pub attrs: Vec<(String, char, Option<String>)>,
    pub pseudo: Vec<String>,
}

/// Rightmost compound is last. `combinators[i]` joins parts[i] and parts[i+1].
#[derive(Debug, Clone)]
pub struct Selector {
    pub parts: Vec<Compound>,
    pub combinators: Vec<Combinator>,
}

impl Selector {
    /// (id count, class/attr/pseudo count, tag count)
    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut s = (0, 0, 0);
        for p in &self.parts {
            s.0 += p.id.is_some() as u32;
            s.1 += (p.classes.len() + p.attrs.len() + p.pseudo.len()) as u32;
            s.2 += p.tag.is_some() as u32;
        }
        s
    }
}

pub fn parse(src: &str) -> Stylesheet {
    let clean = strip_comments(src);
    let mut rules = Vec::new();
    let mut font_faces = Vec::new();
    parse_block(&clean, &mut rules, &mut font_faces, None);
    let keyframes = if clean.contains("keyframes") { crate::anim::find_keyframes(&clean) } else { Vec::new() };
    Stylesheet { rules, font_faces, keyframes }
}

/// Parse a declaration list like the `style=""` attribute.
pub fn parse_declarations(src: &str) -> Vec<Decl> {
    // Inline style attributes can carry comments too.
    let cleaned;
    let src = if src.contains("/*") {
        cleaned = strip_comments(src);
        cleaned.as_str()
    } else {
        src
    };
    split_top(src, ';')
        .into_iter()
        .filter_map(|d| {
            let (name, value) = d.split_once(':')?;
            let name = name.trim();
            // Custom property names are case-sensitive; everything else isn't.
            let name = if name.starts_with("--") { name.to_string() } else { name.to_ascii_lowercase() };
            let mut value = value.trim().to_string();
            // `!important`, with or without space after the `!`.
            let lower = value.to_ascii_lowercase();
            let bang = lower.rfind('!').filter(|&p| lower[p + 1..].trim() == "important");
            let important = bang.is_some();
            if let Some(p) = bang {
                value.truncate(p);
                value = value.trim().to_string();
            }
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some(Decl { name, value, important })
        })
        .collect()
}

fn parse_block(src: &str, rules: &mut Vec<Rule>, faces: &mut Vec<FontFace>, media: Option<&std::sync::Arc<str>>) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        // Find the end of the prelude: '{' or ';' (for statement at-rules like @import).
        let start = i;
        while i < b.len() && b[i] != b'{' && b[i] != b';' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let prelude = src[start..i].trim();
        if b[i] == b';' {
            i += 1;
            continue;
        }
        let body_start = i + 1;
        let body_end = matching_brace(b, i);
        let body = &src[body_start..body_end.min(src.len())];
        i = body_end + 1;

        if let Some(at) = prelude.strip_prefix('@') {
            let lower = at.to_ascii_lowercase();
            if lower.starts_with("media") {
                let cond = lower[5..].trim();
                // Print-only rules can never apply on screen: drop them now.
                if !media_possible(cond) {
                    continue;
                }
                let joined: std::sync::Arc<str> = match media {
                    Some(outer) => format!("{outer} && {cond}").into(),
                    None => cond.into(),
                };
                parse_block(body, rules, faces, Some(&joined));
            } else if lower.starts_with("supports") || lower.starts_with("layer") {
                parse_block(body, rules, faces, media);
            } else if lower.starts_with("font-face") {
                let decls = parse_declarations(body);
                let get = |n: &str| decls.iter().rev().find(|d| d.name == n).map(|d| d.value.clone());
                if let (Some(family), Some(src)) = (get("font-family"), get("src")) {
                    let num = |w: &str| match w.trim() {
                        "normal" => Some(400),
                        "bold" => Some(700),
                        n => n.parse::<f32>().ok().map(|v| v as u16),
                    };
                    let wv = get("font-weight").unwrap_or_else(|| "400".into()).to_ascii_lowercase();
                    let mut ws = wv.split_whitespace().filter_map(num);
                    let lo = ws.next().unwrap_or(400);
                    let hi = ws.next().unwrap_or(lo).max(lo);
                    let st = get("font-style").unwrap_or_default().to_ascii_lowercase();
                    faces.push(FontFace {
                        family: family.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_ascii_lowercase(),
                        src,
                        weight: (lo, hi),
                        italic: st.starts_with("italic") || st.starts_with("oblique"),
                    });
                }
            }
            continue;
        }
        let selectors: Vec<Selector> = split_top(prelude, ',').iter().filter_map(|s| parse_selector(s)).collect();
        if selectors.is_empty() {
            continue;
        }
        rules.push(Rule { selectors, decls: parse_declarations(body), media: media.cloned() });
    }
}

// ---------------------------------------------------------------- media queries

/// The window media queries are checked against: (width, height) in CSS pixels, and the
/// reader's preferences.
static VIEWPORT: std::sync::Mutex<(f32, f32)> = std::sync::Mutex::new((1024.0, 768.0));

pub fn set_viewport(w: f32, h: f32) {
    if let Ok(mut v) = VIEWPORT.lock() {
        *v = (w, h);
    }
}

pub fn viewport() -> (f32, f32) {
    VIEWPORT.lock().map(|v| *v).unwrap_or((1024.0, 768.0))
}

/// A query that can't match a screen at any size (print-only): its rules are dropped.
fn media_possible(q: &str) -> bool {
    q.split(',').any(|part| {
        let p = part.trim();
        !(p == "print" || p.starts_with("print ") || p == "speech" || p.starts_with("only print"))
    })
}

/// Does `q` (a media query list, or several joined with " && ") match the window now?
pub fn media_matches(q: &str) -> bool {
    let (w, h) = viewport();
    q.split(" && ").all(|list| query_list(list, w, h))
}

fn query_list(list: &str, w: f32, h: f32) -> bool {
    let list = list.trim();
    list.is_empty() || list.split(',').any(|q| one_query(q.trim(), w, h))
}

fn one_query(q: &str, w: f32, h: f32) -> bool {
    let (negate, q) = match q.strip_prefix("not ") {
        Some(rest) => (true, rest.trim()),
        None => (false, q.strip_prefix("only ").unwrap_or(q).trim()),
    };
    let mut ok = true;
    for part in q.split(" and ") {
        let part = part.trim();
        let matched = match part {
            "" | "all" | "screen" => true,
            "print" | "speech" | "tv" | "handheld" | "projection" | "braille" | "embossed" | "aural" | "tty" => false,
            p if p.starts_with('(') => feature(p.trim_start_matches('(').trim_end_matches(')').trim(), w, h),
            // Unknown media types don't match.
            _ => false,
        };
        ok &= matched;
    }
    ok != negate
}

/// One media feature: `min-width: 600px`, `orientation: landscape`, `width >= 600px`,
/// `400px <= width <= 700px`, `prefers-color-scheme: dark`…
fn feature(f: &str, w: f32, h: f32) -> bool {
    let px = |v: &str| -> Option<f32> {
        let v = v.trim();
        let num: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
        let n: f32 = num.parse().ok()?;
        Some(match v[num.len()..].trim() {
            "em" | "rem" => n * 16.0,
            "" | "px" => n,
            "dppx" | "x" => n,
            "dpi" => n / 96.0,
            _ => n,
        })
    };
    // Range syntax.
    for op in [">=", "<=", ">", "<", "="] {
        if f.contains(op) && !f.contains(':') {
            let parts: Vec<&str> = f.split(['<', '>', '=']).map(str::trim).filter(|p| !p.is_empty()).collect();
            let ops: Vec<&str> = f.split(|c: char| !matches!(c, '<' | '>' | '=')).filter(|p| !p.is_empty()).collect();
            let val = |name: &str| match name {
                "width" => Some(w),
                "height" => Some(h),
                "aspect-ratio" => Some(w / h.max(1.0)),
                _ => None,
            };
            let cmp = |a: f32, op: &str, b: f32| match op {
                ">=" => a >= b,
                "<=" => a <= b,
                ">" => a > b,
                "<" => a < b,
                _ => (a - b).abs() < 0.5,
            };
            return match (parts.as_slice(), ops.as_slice()) {
                ([name, v], [op]) => match (val(name), px(v)) {
                    (Some(a), Some(b)) => cmp(a, op, b),
                    _ => match (px(name), val(v)) {
                        (Some(a), Some(b)) => cmp(a, op, b),
                        _ => false,
                    },
                },
                ([lo, name, hi], [op1, op2]) => match (px(lo), val(name), px(hi)) {
                    (Some(a), Some(x), Some(b)) => cmp(a, op1, x) && cmp(x, op2, b),
                    _ => false,
                },
                _ => false,
            };
        }
    }
    let (name, value) = match f.split_once(':') {
        Some((n, v)) => (n.trim(), Some(v.trim())),
        None => (f.trim(), None),
    };
    let reduced = crate::settings::get().reduce_motion;
    let contrast = crate::settings::get().high_contrast;
    match (name, value) {
        ("min-width", Some(v)) => px(v).is_some_and(|v| w >= v),
        ("max-width", Some(v)) => px(v).is_some_and(|v| w <= v),
        ("width", Some(v)) => px(v).is_some_and(|v| (w - v).abs() < 0.5),
        ("min-height", Some(v)) => px(v).is_some_and(|v| h >= v),
        ("max-height", Some(v)) => px(v).is_some_and(|v| h <= v),
        ("min-device-width", Some(v)) => px(v).is_some_and(|v| w >= v),
        ("max-device-width", Some(v)) => px(v).is_some_and(|v| w <= v),
        ("orientation", Some(v)) => (v == "landscape") == (w >= h),
        ("min-aspect-ratio" | "max-aspect-ratio", Some(v)) => {
            let r = v.split_once('/').and_then(|(a, b)| Some(a.trim().parse::<f32>().ok()? / b.trim().parse::<f32>().ok()?.max(1e-6))).or_else(|| v.parse().ok());
            r.is_some_and(|r| if name.starts_with("min") { w / h.max(1.0) >= r } else { w / h.max(1.0) <= r })
        }
        ("prefers-color-scheme", Some(v)) => v == "light",
        ("prefers-reduced-motion", Some(v)) => (v == "reduce") == reduced,
        ("prefers-reduced-motion", None) => reduced,
        ("prefers-contrast", Some(v)) => if contrast { v == "more" || v == "custom" } else { v == "no-preference" },
        ("prefers-contrast", None) => contrast,
        ("forced-colors", None) => contrast,
        ("prefers-reduced-transparency", Some(v)) => v == "no-preference",
        ("forced-colors", Some(v)) => v == if contrast { "active" } else { "none" },
        ("hover" | "any-hover", Some(v)) => v == "hover",
        ("pointer" | "any-pointer", Some(v)) => v == "fine",
        ("hover" | "any-hover" | "pointer" | "any-pointer" | "color" | "width" | "height" | "orientation", None) => true,
        ("min-resolution" | "min-device-pixel-ratio" | "-webkit-min-device-pixel-ratio", Some(v)) => px(v).is_some_and(|v| 1.0 >= v),
        ("max-resolution" | "max-device-pixel-ratio" | "-webkit-max-device-pixel-ratio", Some(v)) => px(v).is_some_and(|v| 1.0 <= v),
        ("color", Some(_)) | ("min-color", Some(_)) => true,
        ("display-mode", Some(v)) => v == "browser",
        ("scripting", Some(v)) => v == "enabled",
        ("update", Some(v)) => v == "fast",
        _ => false,
    }
}

fn matching_brace(b: &[u8], open: usize) -> usize {
    let mut depth = 0;
    let mut i = open;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    b.len()
}

/// Split on `sep`, ignoring separators inside (), [] and quotes.
pub fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            cur.push(c);
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        if c == sep && depth == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find("/*") {
        out.push_str(&rest[..p]);
        rest = match rest[p + 2..].find("*/") {
            Some(e) => &rest[p + 2 + e + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Parse a comma-separated selector list (used by querySelector).
pub fn parse_selector_list(s: &str) -> Vec<Selector> {
    split_top(s, ',').iter().filter_map(|s| parse_selector(s)).collect()
}

fn parse_selector(s: &str) -> Option<Selector> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let mut combinators = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut pending: Option<Combinator> = None;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            if !parts.is_empty() && pending.is_none() {
                pending = Some(Combinator::Descendant);
            }
            i += 1;
            continue;
        }
        if c == '>' {
            pending = Some(Combinator::Child);
            i += 1;
            continue;
        }
        if c == '+' || c == '~' {
            pending = Some(if c == '+' { Combinator::Adjacent } else { Combinator::Sibling });
            i += 1;
            continue;
        }
        let (compound, next) = parse_compound(&chars, i)?;
        i = next;
        if !parts.is_empty() {
            combinators.push(pending.take().unwrap_or(Combinator::Descendant));
        }
        pending = None;
        parts.push(compound);
    }
    if parts.is_empty() {
        return None;
    }
    Some(Selector { parts, combinators })
}

fn ident(chars: &[char], mut i: usize) -> (String, usize) {
    let mut s = String::new();
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_' || chars[i] == '\\') {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
        }
        s.push(chars[i]);
        i += 1;
    }
    (s, i)
}

fn parse_compound(chars: &[char], mut i: usize) -> Option<(Compound, usize)> {
    let mut c = Compound::default();
    let start = i;
    while i < chars.len() {
        match chars[i] {
            '*' => i += 1,
            '#' => {
                let (s, n) = ident(chars, i + 1);
                c.id = Some(s);
                i = n;
            }
            '.' => {
                let (s, n) = ident(chars, i + 1);
                c.classes.push(s);
                i = n;
            }
            '[' => {
                let end = chars[i..].iter().position(|&x| x == ']')? + i;
                let inner: String = chars[i + 1..end].iter().collect();
                if let Some((k, v)) = inner.split_once('=') {
                    let (name, op) = match k.trim_end().chars().last() {
                        Some(o @ ('~' | '|' | '^' | '$' | '*')) => (&k.trim_end()[..k.trim_end().len() - 1], o),
                        _ => (k, '='),
                    };
                    // Drop a trailing case flag: [type="a" i]
                    let v = v.trim();
                    let v = v.strip_suffix(" i").or_else(|| v.strip_suffix(" s")).unwrap_or(v);
                    let v = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
                    c.attrs.push((name.trim().to_ascii_lowercase(), op, Some(v)));
                } else {
                    c.attrs.push((inner.trim().to_ascii_lowercase(), '=', None));
                }
                i = end + 1;
            }
            ':' => {
                let mut j = i + 1;
                let element = j < chars.len() && chars[j] == ':';
                if element {
                    j += 1;
                }
                let (s, mut n) = ident(chars, j);
                if n < chars.len() && chars[n] == '(' {
                    // Functional pseudo-class: keep its argument, e.g. "not(.a)", "nth-child(2n+1)".
                    let arg_start = n + 1;
                    let mut depth = 0;
                    while n < chars.len() {
                        if chars[n] == '(' {
                            depth += 1;
                        } else if chars[n] == ')' {
                            depth -= 1;
                            if depth == 0 {
                                n += 1;
                                break;
                            }
                        }
                        n += 1;
                    }
                    let arg: String = chars[arg_start..n.saturating_sub(1).max(arg_start)].iter().collect();
                    c.pseudo.push(format!("{}({})", s.to_ascii_lowercase(), arg.trim()));
                } else {
                    let name = s.to_ascii_lowercase();
                    // ::before/::after (and legacy :before/:after) are pseudo-elements.
                    let is_element = element || matches!(name.as_str(), "before" | "after" | "first-line" | "first-letter");
                    c.pseudo.push(if is_element { format!("::{name}") } else { name });
                }
                i = n;
            }
            ch if ch.is_alphanumeric() || ch == '-' || ch == '_' => {
                let (s, n) = ident(chars, i);
                c.tag = Some(s.to_ascii_lowercase());
                i = n;
            }
            _ => break,
        }
    }
    if i == start {
        return None;
    }
    Some((c, i))
}

/// Make every `url(...)` in a stylesheet absolute, relative to the sheet's own URL.
pub fn absolutize_urls(sheet: &mut Stylesheet, base: &str) {
    for f in &mut sheet.font_faces {
        f.src = rewrite_urls(&f.src, base);
    }
    for rule in &mut sheet.rules {
        for d in &mut rule.decls {
            if d.value.contains("url(") {
                d.value = rewrite_urls(&d.value, base);
            }
        }
    }
}

fn rewrite_urls(v: &str, base: &str) -> String {
    let mut out = String::new();
    let mut rest = v;
    while let Some(i) = rest.find("url(") {
        out.push_str(&rest[..i + 4]);
        rest = &rest[i + 4..];
        let Some(end) = rest.find(')') else { break };
        let raw = rest[..end].trim().trim_matches(|c| c == '"' || c == '\'');
        if raw.starts_with("data:") {
            out.push_str(&rest[..end]);
        } else {
            out.push('"');
            out.push_str(&crate::net::resolve(base, raw));
            out.push('"');
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(src: &str) -> Vec<(String, String, bool)> {
        parse_declarations(src).into_iter().map(|d| (d.name, d.value, d.important)).collect()
    }

    #[test]
    fn rules_and_declarations() {
        let s = parse("h1, .a > b { color: red; margin : 0 auto }  p{}");
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.rules[0].selectors.len(), 2);
        assert_eq!(s.rules[0].decls.len(), 2);
        assert_eq!(s.rules[0].decls[1].name, "margin");
        assert_eq!(s.rules[0].decls[1].value, "0 auto");
    }

    #[test]
    fn important_and_case() {
        assert_eq!(decls("COLOR: Red !important; width:1px ! important"), vec![
            ("color".into(), "Red".into(), true),
            ("width".into(), "1px".into(), true),
        ]);
    }

    #[test]
    fn comments_are_ignored() {
        let s = parse("/* a { color: red } */ b { /* x */ color: blue; /* y */ }");
        assert_eq!(s.rules.len(), 1);
        assert_eq!(decls("/*c*/color:/*d*/blue")[0].1.trim(), "blue");
    }

    #[test]
    fn values_with_semicolons_and_braces_in_strings() {
        let d = decls(r#"content: "a;b}"; background: url(data:image/png;base64,AAA=) no-repeat"#);
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(d[0].1, r#""a;b}""#);
        assert!(d[1].1.starts_with("url(data:image/png;base64,AAA=)"), "{d:?}");
    }

    #[test]
    fn custom_properties_keep_their_case_and_value() {
        let d = decls("--Main-Color: #ABC; --gap: calc(1px + 2px)");
        assert_eq!(d[0].0, "--Main-Color");
        assert_eq!(d[1].1, "calc(1px + 2px)");
    }

    #[test]
    fn broken_declarations_are_skipped() {
        let d = decls("color red; width: 2px; : nothing; height:");
        assert!(d.iter().any(|x| x.0 == "width" && x.1 == "2px"), "{d:?}");
        assert!(!d.iter().any(|x| x.0 == "color"), "{d:?}");
    }

    #[test]
    fn selectors_parts_and_combinators() {
        let s = parse_selector_list("div#main.a.b[href^=\"http\"]:hover > p + span ~ em i");
        assert_eq!(s.len(), 1);
        let sel = &s[0];
        assert_eq!(sel.parts.len(), 5);
        assert_eq!(sel.parts[0].tag.as_deref(), Some("div"));
        assert_eq!(sel.parts[0].id.as_deref(), Some("main"));
        assert_eq!(sel.parts[0].classes, vec!["a", "b"]);
        assert_eq!(sel.parts[0].attrs[0].0, "href");
        assert_eq!(sel.parts[0].attrs[0].1, '^');
        assert_eq!(sel.parts[0].pseudo, vec!["hover"]);
        assert_eq!(sel.combinators, vec![Combinator::Child, Combinator::Adjacent, Combinator::Sibling, Combinator::Descendant]);
    }

    #[test]
    fn specificity() {
        let sp = |s: &str| parse_selector_list(s)[0].specificity();
        assert_eq!(sp("*"), (0, 0, 0));
        assert_eq!(sp("li"), (0, 0, 1));
        assert_eq!(sp("ul li.x"), (0, 1, 2));
        assert_eq!(sp("#a .b:hover"), (1, 2, 0));
        assert_eq!(sp("a[href]"), (0, 1, 1));
    }

    #[test]
    fn invalid_selectors_drop_only_their_rule() {
        let s = parse("a<<b { color: red } p { color: blue }");
        assert!(s.rules.iter().any(|r| r.decls.iter().any(|d| d.value == "blue")));
        assert!(!s.rules.iter().any(|r| r.decls.iter().any(|d| d.value == "red")));
    }

    #[test]
    fn media_queries_follow_the_window() {
        let s = parse("@media screen and (min-width: 768px) { a { x: 1 } } @media print { b { x: 2 } } @media (max-width: 600px) { c { x: 3 } }");
        // Print-only rules are dropped; the others keep their condition.
        assert_eq!(s.rules.len(), 2);
        assert!(one_query("screen and (min-width: 768px)", 1024.0, 700.0));
        assert!(!one_query("screen and (min-width: 768px)", 500.0, 700.0));
        assert!(one_query("(max-width: 600px)", 500.0, 700.0));
        assert!(one_query("not print", 500.0, 700.0));
        assert!(query_list("print, (orientation: portrait)", 500.0, 700.0));
        assert!(feature("width >= 600px", 800.0, 600.0));
        assert!(!feature("400px <= width <= 700px", 800.0, 600.0));
        assert!(feature("400px <= width <= 700px", 500.0, 600.0));
        assert!(feature("min-width: 40em", 700.0, 600.0));
        assert!(!feature("prefers-color-scheme: dark", 700.0, 600.0));
    }

    #[test]
    fn supports_and_layer_blocks_are_used() {
        let s = parse("@supports (display: grid) { a { x: 1 } } @layer base { b { x: 2 } }");
        assert_eq!(s.rules.len(), 2);
    }

    #[test]
    fn font_face() {
        let s = parse("@font-face { font-family: 'My Font'; src: url(a.woff2) format('woff2'), url(a.ttf); font-weight: 100 900; font-style: italic }");
        assert_eq!(s.font_faces.len(), 1);
        let f = &s.font_faces[0];
        assert_eq!(f.family, "my font");
        assert_eq!(f.weight, (100, 900));
        assert!(f.italic);
        assert_eq!(f.best_url().as_deref(), Some("a.woff2"));
    }

    #[test]
    fn unknown_at_rules_are_skipped_whole() {
        let s = parse("@keyframes spin { from { a: 1 } to { a: 2 } } p { color: red }");
        assert_eq!(s.rules.len(), 1);
        assert_eq!(s.rules[0].selectors[0].parts[0].tag.as_deref(), Some("p"));
    }

    #[test]
    fn split_top_respects_nesting() {
        assert_eq!(split_top("a, rgb(1, 2, 3), \"x,y\"", ','), vec!["a", " rgb(1, 2, 3)", " \"x,y\""]);
    }
}
