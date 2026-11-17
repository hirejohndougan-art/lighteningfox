//! CSS parser: stylesheets -> rules -> selectors + declarations.

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub font_faces: Vec<FontFace>,
    /// `@keyframes` rules, for CSS animations.
    pub keyframes: Vec<crate::anim::Keyframes>,
    /// `@import` rules: sheets to load ahead of this one's rules (see `load_imports`).
    pub imports: Vec<Import>,
    /// Cascade layer names in the order they were first declared.
    pub layers: Vec<std::sync::Arc<str>>,
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
    /// `unicode-range` as inclusive code point ranges (empty: every character).
    pub unicode_range: Vec<(u32, u32)>,
}

impl FontFace {
    /// Does this face cover the character (its unicode-range, if it has one)?
    pub fn covers(&self, c: char) -> bool {
        let c = c as u32;
        self.unicode_range.is_empty() || self.unicode_range.iter().any(|&(a, b)| a <= c && c <= b)
    }

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
    /// The cascade layer it's in (`@layer a.b`), by full name; None = not in a layer.
    pub layer: Option<std::sync::Arc<str>>,
    /// The `@container` conditions it sits in (nested ones joined with " && ").
    pub container: Option<std::sync::Arc<str>>,
}

/// An `@import` rule: the sheet to load in its place.
#[derive(Debug, Clone)]
pub struct Import {
    pub url: String,
    pub media: Option<std::sync::Arc<str>>,
    pub layer: Option<std::sync::Arc<str>>,
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
            for ps in &p.pseudo {
                if ps.starts_with("where(") {
                    continue;
                }
                // :is(), :not(), :has(): the most specific selector in the list.
                let arg = ["is(", "not(", "has(", "matches(", "-webkit-any("].iter().find_map(|f| ps.strip_prefix(f)).and_then(|a| a.strip_suffix(')'));
                match arg {
                    Some(a) => {
                        let best = parse_selector_list(a.trim_start_matches(['>', '+', '~'])).iter().map(|s| s.specificity()).max().unwrap_or((0, 0, 0));
                        s.0 += best.0;
                        s.1 += best.1;
                        s.2 += best.2;
                    }
                    None if ps.starts_with("::") => s.2 += 1,
                    None => s.1 += 1,
                }
            }
            s.1 += (p.classes.len() + p.attrs.len()) as u32;
            s.2 += p.tag.is_some() as u32;
        }
        s
    }
}

/// The parsing state for one stylesheet.
struct Builder<'a> {
    src: &'a [char],
    rules: Vec<Rule>,
    faces: Vec<FontFace>,
    imports: Vec<Import>,
    /// Layer names in the order they first appear (this decides which layer wins).
    layers: Vec<std::sync::Arc<str>>,
    anon: usize,
}

/// Where a rule sits: inside which @media, @layer, @container and (for nested rules)
/// which parent selectors.
#[derive(Clone, Default)]
struct Scope {
    media: Option<std::sync::Arc<str>>,
    layer: Option<std::sync::Arc<str>>,
    container: Option<std::sync::Arc<str>>,
    parent: Option<std::rc::Rc<Vec<String>>>,
}

pub fn parse(src: &str) -> Stylesheet {
    let chars = crate::csssyntax::preprocess(src);
    let items = crate::csssyntax::parse_stylesheet(&chars);
    let mut b = Builder { src: &chars, rules: Vec::new(), faces: Vec::new(), imports: Vec::new(), layers: Vec::new(), anon: 0 };
    let mut imports_allowed = true;
    for item in items {
        // @import only counts before every other rule (except @charset and @layer statements).
        let is_import = matches!(&item, crate::csssyntax::Item::AtRule(n, _, None) if n == "import");
        if is_import && imports_allowed {
            if let crate::csssyntax::Item::AtRule(_, prelude, _) = &item {
                b.import(prelude);
            }
            continue;
        }
        let keeps = matches!(&item, crate::csssyntax::Item::AtRule(n, _, None) if n == "charset" || n == "layer");
        if !keeps {
            imports_allowed = false;
        }
        b.item(item, &Scope::default());
    }
    let keyframes = if src.contains("keyframes") { crate::anim::find_keyframes(&strip_comments(src)) } else { Vec::new() };
    Stylesheet { rules: b.rules, font_faces: b.faces, keyframes, imports: b.imports, layers: b.layers }
}

/// Parse a declaration list like the `style=""` attribute.
pub fn parse_declarations(src: &str) -> Vec<Decl> {
    let chars = crate::csssyntax::preprocess(src);
    let cvs = crate::csssyntax::Parser::new(&chars).component_values();
    decls_of(&chars, crate::csssyntax::block_contents(cvs))
}

fn decls_of(src: &[char], items: Vec<crate::csssyntax::Item>) -> Vec<Decl> {
    items
        .into_iter()
        .filter_map(|it| match it {
            crate::csssyntax::Item::Decl(name, value, important) => {
                // Custom property names are case-sensitive; everything else isn't.
                let name = if name.starts_with("--") { name } else { name.to_ascii_lowercase() };
                let value = crate::csssyntax::text(src, &value);
                (!value.is_empty() && !name.is_empty()).then_some(Decl { name, value, important })
            }
            _ => None,
        })
        .collect()
}

fn join(outer: &Option<std::sync::Arc<str>>, inner: &str) -> std::sync::Arc<str> {
    match outer {
        Some(o) => format!("{o} && {inner}").into(),
        None => inner.into(),
    }
}

impl Builder<'_> {
    fn text(&self, cvs: &[crate::csssyntax::Cv]) -> String {
        crate::csssyntax::text(self.src, cvs)
    }

    fn layer_name(&mut self, scope: &Scope, name: &str) -> std::sync::Arc<str> {
        let name = name.trim();
        let full: std::sync::Arc<str> = match (&scope.layer, name.is_empty()) {
            (Some(outer), false) => format!("{outer}.{name}").into(),
            (None, false) => name.into(),
            (outer, true) => {
                self.anon += 1;
                match outer {
                    Some(o) => format!("{o}.\u{0}anon{}", self.anon).into(),
                    None => format!("\u{0}anon{}", self.anon).into(),
                }
            }
        };
        // Declaring "a.b" also declares "a" (before it).
        let mut prefix = String::new();
        for part in full.split('.') {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(part);
            if !self.layers.iter().any(|l| **l == *prefix) {
                self.layers.push(prefix.as_str().into());
            }
        }
        full
    }

    fn import(&mut self, prelude: &[crate::csssyntax::Cv]) {
        use crate::csssyntax::{Cv, Token};
        let mut url = None;
        let mut layer = None;
        let mut rest = Vec::new();
        let mut supported = true;
        for cv in prelude {
            match cv {
                Cv::Tok(t) if url.is_none() && matches!(t.t, Token::Url(_) | Token::Str(_)) => {
                    if let Token::Url(u) | Token::Str(u) = &t.t {
                        url = Some(u.clone());
                    }
                }
                Cv::Func(n, args, _) if url.is_none() && n.eq_ignore_ascii_case("url") => {
                    url = args.iter().find_map(|a| match a.token() {
                        Some(Token::Str(u)) => Some(u.clone()),
                        _ => None,
                    });
                }
                Cv::Tok(t) if url.is_some() && rest.is_empty() && matches!(&t.t, Token::Ident(i) if i.eq_ignore_ascii_case("layer")) => {
                    layer = Some(self.layer_name(&Scope::default(), ""));
                }
                Cv::Func(n, args, _) if url.is_some() && rest.is_empty() && n.eq_ignore_ascii_case("layer") => {
                    let name = self.text(args);
                    layer = Some(self.layer_name(&Scope::default(), &name));
                }
                Cv::Func(n, args, _) if url.is_some() && rest.is_empty() && n.eq_ignore_ascii_case("supports") => {
                    let cond = self.text(args);
                    // supports(display: grid) holds a declaration without parentheses.
                    supported = supports(&format!("({cond})")) || supports(&cond);
                }
                Cv::Tok(t) if matches!(t.t, Token::Whitespace) && rest.is_empty() => {}
                _ if url.is_some() => rest.push(cv.clone()),
                _ => {}
            }
        }
        let media = self.text(&rest).to_ascii_lowercase();
        if let (Some(url), true) = (url, supported) {
            if media.is_empty() || media_possible(&media) {
                self.imports.push(Import { url, media: (!media.is_empty()).then(|| media.into()), layer });
            }
        }
    }

    fn item(&mut self, item: crate::csssyntax::Item, scope: &Scope) {
        use crate::csssyntax::Item;
        match item {
            Item::Qualified(prelude, block) => {
                let sel_text = self.text(&prelude);
                let sels: Vec<String> = match &scope.parent {
                    Some(parents) => split_top(&sel_text, ',').iter().map(|s| nest(parents, s.trim())).collect(),
                    None => split_top(&sel_text, ',').iter().map(|s| s.trim().to_string()).collect(),
                };
                let selectors: Vec<Selector> = sels.iter().filter_map(|s| parse_selector(s)).collect();
                // One bad selector drops the whole rule, as in browsers.
                if selectors.is_empty() || selectors.len() != sels.len() {
                    return;
                }
                let contents = crate::csssyntax::block_contents(block);
                let (decls, nested): (Vec<_>, Vec<_>) = contents.into_iter().partition(|i| matches!(i, Item::Decl(..)));
                let decls = decls_of(self.src, decls);
                if !decls.is_empty() || nested.is_empty() {
                    self.rules.push(Rule { selectors, decls, media: scope.media.clone(), layer: scope.layer.clone(), container: scope.container.clone() });
                }
                if !nested.is_empty() {
                    let inner = Scope { parent: Some(std::rc::Rc::new(sels)), ..scope.clone() };
                    for n in nested {
                        self.item(n, &inner);
                    }
                }
            }
            Item::Decl(..) => {
                // Declarations straight inside a nested @media etc.: they belong to the parent selector.
                if let Some(parents) = &scope.parent {
                    let selectors: Vec<Selector> = parents.iter().filter_map(|s| parse_selector(s)).collect();
                    let decls = decls_of(self.src, vec![item]);
                    if !selectors.is_empty() && !decls.is_empty() {
                        self.rules.push(Rule { selectors, decls, media: scope.media.clone(), layer: scope.layer.clone(), container: scope.container.clone() });
                    }
                }
            }
            Item::AtRule(name, prelude, block) => {
                let pre = self.text(&prelude);
                match (name.as_str(), block) {
                    ("media", Some(block)) => {
                        let cond = pre.to_ascii_lowercase();
                        // Print-only rules can never apply on screen: drop them now.
                        if !media_possible(&cond) {
                            return;
                        }
                        let inner = Scope { media: Some(join(&scope.media, &cond)), ..scope.clone() };
                        self.block(block, &inner);
                    }
                    ("supports", Some(block)) => {
                        if supports(&pre) {
                            self.block(block, scope);
                        }
                    }
                    ("layer", Some(block)) => {
                        let layer = self.layer_name(scope, &pre);
                        let inner = Scope { layer: Some(layer), ..scope.clone() };
                        self.block(block, &inner);
                    }
                    ("layer", None) => {
                        for name in pre.split(',') {
                            if !name.trim().is_empty() {
                                self.layer_name(scope, name);
                            }
                        }
                    }
                    ("container", Some(block)) => {
                        let inner = Scope { container: Some(join(&scope.container, pre.trim())), ..scope.clone() };
                        self.block(block, &inner);
                    }
                    ("scope" | "starting-style" | "document" | "-moz-document", Some(_)) => {}
                    ("font-face", Some(block)) => {
                        let decls = decls_of(self.src, crate::csssyntax::block_contents(block));
                        self.font_face(decls);
                    }
                    _ => {}
                }
            }
        }
    }

    /// The contents of a conditional group rule (@media, @supports, @layer, @container).
    fn block(&mut self, block: Vec<crate::csssyntax::Cv>, scope: &Scope) {
        let items = if scope.parent.is_some() { crate::csssyntax::block_contents(block) } else { crate::csssyntax::rule_list(block, false) };
        for it in items {
            self.item(it, scope);
        }
    }

    fn font_face(&mut self, decls: Vec<Decl>) {
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
            let unicode_range = get("unicode-range").map(|r| parse_unicode_range(&r)).unwrap_or_default();
            self.faces.push(FontFace {
                family: family.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_ascii_lowercase(),
                src,
                weight: (lo, hi),
                italic: st.starts_with("italic") || st.starts_with("oblique"),
                unicode_range,
            });
        }
    }
}

/// `U+0000-00FF, U+0131, U+4??` as inclusive ranges (wildcards cover every digit).
pub fn parse_unicode_range(v: &str) -> Vec<(u32, u32)> {
    v.split(',')
        .filter_map(|part| {
            let p = part.trim().to_ascii_uppercase();
            let p = p.strip_prefix("U+")?;
            let hex = |h: &str| u32::from_str_radix(h, 16).ok();
            match p.split_once('-') {
                Some((a, b)) => Some((hex(a)?, hex(b)?)),
                None if p.contains('?') => Some((hex(&p.replace('?', "0"))?, hex(&p.replace('?', "F"))?)),
                None => hex(p).map(|c| (c, c)),
            }
        })
        .collect()
}

/// A nested selector's full form: `&` stands for the parent selectors; without one, it's
/// a descendant of them.
fn nest(parents: &[String], sel: &str) -> String {
    let parent = if parents.len() == 1 { parents[0].clone() } else { format!(":is({})", parents.join(", ")) };
    if sel.contains('&') {
        sel.replace('&', &parent)
    } else {
        format!("{parent} {sel}")
    }
}

// ---------------------------------------------------------------- @supports

/// Does an `@supports` condition hold? `(prop: value)`, `not`, `and`, `or`,
/// `selector(…)`, `font-tech(…)`, `font-format(…)`.
pub fn supports(cond: &str) -> bool {
    let c = cond.trim();
    if c.is_empty() {
        return false;
    }
    let lower = c.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("not ").or_else(|| lower.strip_prefix("not(").map(|_| &lower[3..])) {
        return !supports(rest);
    }
    // Split on top-level `and` / `or`.
    let parts = split_words(c);
    if parts.len() > 1 {
        let mut result = supports(&parts[0]);
        let mut i = 1;
        while i + 1 < parts.len() {
            let op = parts[i].to_ascii_lowercase();
            let v = supports(&parts[i + 1]);
            result = if op == "or" { result || v } else { result && v };
            i += 2;
        }
        return result;
    }
    if let Some(inner) = c.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        let inner = inner.trim();
        if inner.starts_with('(') || inner.to_ascii_lowercase().starts_with("not ") || inner.to_ascii_lowercase().starts_with("selector(") {
            return supports(inner);
        }
        let Some((prop, value)) = inner.split_once(':') else { return false };
        return property_supported(prop.trim(), value.trim());
    }
    if let Some(arg) = lower.strip_prefix("selector(").and_then(|r| r.strip_suffix(')')) {
        let sels = parse_selector_list(arg);
        return !sels.is_empty() && sels.iter().all(|s| s.parts.iter().all(|p| p.pseudo.iter().all(|ps| known_pseudo(ps))));
    }
    if let Some(arg) = lower.strip_prefix("font-format(").and_then(|r| r.strip_suffix(')')) {
        return matches!(arg.trim(), "woff" | "woff2" | "truetype" | "opentype");
    }
    false
}

/// Split a condition into operands and `and`/`or` words at the top level.
fn split_words(c: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    let chars: Vec<char> = c.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
        }
        if depth == 0 && ch.is_whitespace() {
            let rest: String = chars[i + 1..].iter().take(4).collect::<String>().to_ascii_lowercase();
            for op in ["and", "or"] {
                if rest.starts_with(op) && chars.get(i + 1 + op.len()).is_some_and(|c| c.is_whitespace() || *c == '(') {
                    out.push(std::mem::take(&mut cur).trim().to_string());
                    out.push(op.to_string());
                    i += 1 + op.len();
                    break;
                }
            }
        }
        if i < chars.len() {
            cur.push(chars[i]);
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn known_pseudo(p: &str) -> bool {
    let name = p.split('(').next().unwrap_or(p).trim_start_matches(':');
    matches!(
        name,
        "link" | "any-link" | "visited" | "root" | "first-child" | "last-child" | "only-child" | "first-of-type" | "last-of-type" | "only-of-type" | "empty"
            | "checked" | "focus" | "focus-visible" | "focus-within" | "placeholder-shown" | "optional" | "disabled" | "enabled" | "required" | "hover" | "active"
            | "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" | "has" | "not" | "is" | "where" | "before" | "after" | "marker" | "placeholder"
            | "target" | "scope" | "defined" | "read-only" | "read-write" | "valid" | "invalid" | "indeterminate" | "default" | "lang" | "dir"
    )
}

/// Properties Ironswift understands (for `@supports`).
fn property_supported(prop: &str, value: &str) -> bool {
    let p = prop.to_ascii_lowercase();
    let v = value.to_ascii_lowercase();
    if p.starts_with("--") {
        return true;
    }
    let known = KNOWN_PROPERTIES.contains(&p.as_str())
        || ["margin-", "padding-", "border-", "inset-", "grid-", "flex-", "font-", "text-", "background-", "animation", "transition", "column", "list-style", "overflow", "counter-", "scroll-"]
            .iter()
            .any(|pre| p.starts_with(pre));
    if !known {
        return false;
    }
    match p.as_str() {
        "display" => matches!(
            v.as_str(),
            "none" | "block" | "inline" | "inline-block" | "flex" | "inline-flex" | "grid" | "inline-grid" | "table" | "inline-table" | "table-row" | "table-cell" | "list-item" | "contents" | "flow-root" | "table-row-group" | "table-header-group" | "table-footer-group" | "table-caption"
        ) || is_wide_keyword(&v),
        "position" => matches!(v.as_str(), "static" | "relative" | "absolute" | "fixed" | "sticky" | "-webkit-sticky") || is_wide_keyword(&v),
        _ => !v.is_empty(),
    }
}

pub fn is_wide_keyword(v: &str) -> bool {
    matches!(v, "inherit" | "initial" | "unset" | "revert" | "revert-layer")
}

const KNOWN_PROPERTIES: &[&str] = &[
    "display", "position", "top", "right", "bottom", "left", "inset", "float", "clear", "z-index", "width", "height", "min-width", "max-width", "min-height", "max-height",
    "margin", "padding", "border", "border-radius", "box-sizing", "box-shadow", "color", "background", "opacity", "visibility", "overflow", "transform", "transform-origin",
    "translate", "rotate", "scale", "filter", "backdrop-filter", "-webkit-backdrop-filter", "aspect-ratio", "gap", "row-gap", "column-gap", "order", "flex", "justify-content",
    "align-items", "align-self", "align-content", "justify-items", "justify-self", "place-items", "place-content", "place-self", "line-height", "letter-spacing",
    "word-spacing", "white-space", "vertical-align", "cursor", "content", "quotes", "direction", "unicode-bidi", "writing-mode", "object-fit", "object-position",
    "outline", "pointer-events", "user-select", "appearance", "-webkit-appearance", "will-change", "contain", "container", "container-type", "container-name",
    "accent-color", "caret-color", "hyphens", "tab-size", "word-break", "overflow-wrap", "word-wrap", "isolation", "mix-blend-mode", "clip-path", "mask",
    "border-collapse", "border-spacing", "table-layout", "caption-side", "empty-cells", "resize", "touch-action", "color-scheme", "inline-size", "block-size",
    "min-inline-size", "max-inline-size", "min-block-size", "max-block-size", "zoom",
];

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

/// Does a stylesheet's result depend on the window size (@media rules, or vw/vh/vmin/vmax
/// lengths)? Such pages are styled again when the window changes size.
pub fn depends_on_viewport(sheet: &Stylesheet) -> bool {
    sheet.rules.iter().any(|r| r.media.is_some() || r.decls.iter().any(|d| has_viewport_unit(&d.value)))
}

/// The same for `style` attributes.
pub fn inline_viewport_units(dom: &crate::dom::Dom) -> bool {
    dom.nodes.iter().any(|n| match &n.data {
        crate::dom::NodeData::Element { attrs, .. } => attrs.iter().any(|(k, v)| k == "style" && has_viewport_unit(v)),
        _ => false,
    })
}

fn has_viewport_unit(v: &str) -> bool {
    // A number followed by vw, vh, vmin, vmax (or the sv/lv/dv variants).
    let b = v.as_bytes();
    (1..b.len()).any(|i| {
        let rest = &b[i..];
        (b[i - 1].is_ascii_digit() || b[i - 1] == b'.')
            && [&b"vw"[..], b"vh", b"vmin", b"vmax", b"svw", b"svh", b"lvw", b"lvh", b"dvw", b"dvh"].iter().any(|u| rest.starts_with(u))
    })
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
    // Styling asks about the same few queries for every element; answer each once per pass.
    MEDIA_MEMO.with(|m| {
        let mut m = m.borrow_mut();
        if m.0 != (w.to_bits(), h.to_bits()) {
            m.0 = (w.to_bits(), h.to_bits());
            m.1.clear();
        }
        if let Some(&r) = m.1.get(q) {
            return r;
        }
        let r = q.split(" && ").all(|list| query_list(list, w, h));
        if m.1.len() < 4096 {
            m.1.insert(q.to_string(), r);
        }
        r
    })
}

thread_local! {
    /// Media query answers for the current window size, kept until the next styling pass
    /// (settings such as reduced motion can change between passes).
    static MEDIA_MEMO: std::cell::RefCell<((u32, u32), std::collections::HashMap<String, bool>)> = std::cell::RefCell::new(((0, 0), std::collections::HashMap::new()));
}

/// Forget remembered media query answers (called at the start of each styling pass).
pub fn forget_media_answers() {
    MEDIA_MEMO.with(|m| m.borrow_mut().1.clear());
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
    // Settings are read only by the features that need them (copying them is not cheap, and
    // this runs for every media rule on every element).
    let reduced = || crate::settings::get().reduce_motion;
    let contrast = || crate::settings::get().high_contrast;
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
        ("prefers-reduced-motion", Some(v)) => (v == "reduce") == reduced(),
        ("prefers-reduced-motion", None) => reduced(),
        ("prefers-contrast", Some(v)) => if contrast() { v == "more" || v == "custom" } else { v == "no-preference" },
        ("prefers-contrast", None) => contrast(),
        ("forced-colors", None) => contrast(),
        ("prefers-reduced-transparency", Some(v)) => v == "no-preference",
        ("forced-colors", Some(v)) => v == if contrast() { "active" } else { "none" },
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

/// A container query (`(min-width: 400px)`, `(width > 30em) and (height < 200px)`,
/// `not (…)`, `… or …`) against a container `w`×`h` (h is NaN for inline-size containers).
pub fn container_query(q: &str, w: f32, h: f32) -> bool {
    let q = q.trim();
    let parts = split_words(q);
    if parts.len() > 1 {
        let mut result = container_query(&parts[0], w, h);
        let mut i = 1;
        while i + 1 < parts.len() {
            let v = container_query(&parts[i + 1], w, h);
            result = if parts[i] == "or" { result || v } else { result && v };
            i += 2;
        }
        return result;
    }
    if let Some(rest) = q.strip_prefix("not") {
        return !container_query(rest, w, h);
    }
    let Some(inner) = q.strip_prefix('(').and_then(|r| r.strip_suffix(')')) else { return false };
    let inner = inner.trim();
    if inner.starts_with('(') || inner.starts_with("not") {
        return container_query(inner, w, h);
    }
    if h.is_nan() && inner.contains("height") || h.is_nan() && inner.contains("orientation") {
        return false;
    }
    let inner = inner.replace("inline-size", "width").replace("block-size", "height");
    feature(&inner, w, if h.is_nan() { 0.0 } else { h })
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

/// Load a sheet's `@import`s (fetched with `fetch`, at most 5 deep) and put their rules
/// ahead of its own, inside the import's media condition and layer.
pub fn load_imports(sheet: &mut Stylesheet, base: &str, fetch: &dyn Fn(&str) -> Option<String>, depth: u32) {
    if sheet.imports.is_empty() || depth >= 5 {
        return;
    }
    let mut rules = Vec::new();
    let mut faces = Vec::new();
    let mut keyframes = Vec::new();
    let mut layers: Vec<std::sync::Arc<str>> = Vec::new();
    for imp in std::mem::take(&mut sheet.imports) {
        let url = crate::net::resolve(base, &imp.url);
        let Some(text) = fetch(&url) else { continue };
        let mut inner = parse(&text);
        absolutize_urls(&mut inner, &url);
        load_imports(&mut inner, &url, fetch, depth + 1);
        if let Some(m) = &imp.media {
            wrap_media(&mut inner, m);
        }
        if let Some(l) = &imp.layer {
            layers.push(l.clone());
            for r in &mut inner.rules {
                r.layer = Some(match &r.layer {
                    Some(own) => format!("{l}.{own}").into(),
                    None => l.clone(),
                });
            }
            inner.layers = inner.layers.iter().map(|own| format!("{l}.{own}").into()).collect();
        }
        rules.extend(inner.rules);
        faces.extend(inner.font_faces);
        keyframes.extend(inner.keyframes);
        layers.extend(inner.layers);
    }
    rules.append(&mut sheet.rules);
    sheet.rules = rules;
    faces.append(&mut sheet.font_faces);
    sheet.font_faces = faces;
    keyframes.append(&mut sheet.keyframes);
    sheet.keyframes = keyframes;
    for l in std::mem::take(&mut sheet.layers) {
        if !layers.contains(&l) {
            layers.push(l);
        }
    }
    sheet.layers = layers;
}

/// Put every rule of a sheet inside a media condition (`<link media>`, `@import … screen`).
pub fn wrap_media(sheet: &mut Stylesheet, media: &str) {
    let m = media.trim().to_ascii_lowercase();
    if m.is_empty() || m == "all" || m == "screen" {
        return;
    }
    for r in &mut sheet.rules {
        r.media = Some(match &r.media {
            Some(own) => format!("{m} && {own}").into(),
            None => m.as_str().into(),
        });
    }
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

#[cfg(test)]
mod syntax_tests {
    use super::*;

    #[test]
    fn unicode_ranges() {
        assert_eq!(parse_unicode_range("U+0000-00FF, U+0131,u+4??"), vec![(0, 0xff), (0x131, 0x131), (0x400, 0x4ff)]);
        let sheet = parse("@font-face{font-family:X;src:url(a.woff2);unicode-range:U+0400-045F}");
        assert!(!sheet.font_faces[0].covers('a') && sheet.font_faces[0].covers('Ж'));
    }

    #[test]
    fn supports_conditions() {
        assert!(supports("(display: grid)"));
        assert!(!supports("(display: subgrid-ish)"));
        assert!(!supports("(foo: bar)"));
        assert!(supports("not (foo: bar)"));
        assert!(supports("(display: flex) and (position: sticky)"));
        assert!(supports("(foo: bar) or (gap: 1px)"));
        assert!(supports("selector(:has(a))"));
        assert!(!supports("selector(:fancy)"));
    }

    #[test]
    fn nesting_imports_layers_containers() {
        let s = parse("@import 'a.css' layer(x) screen; .a { color: red; & .b { x: 1 } > .c { y: 2 } @media (min-width: 1px) { z: 3 } } @layer l { p { q: 1 } } @container card (min-width: 400px) { i { j: 1 } }");
        assert_eq!(s.imports.len(), 1);
        assert_eq!(s.imports[0].url, "a.css");
        assert_eq!(s.imports[0].layer.as_deref(), Some("x"));
        assert_eq!(s.imports[0].media.as_deref(), Some("screen"));
        let sels: Vec<usize> = s.rules.iter().map(|r| r.selectors[0].parts.len()).collect();
        assert_eq!(sels, vec![1, 2, 2, 1, 1, 1]);
        assert!(s.rules[3].media.is_some());
        assert_eq!(s.rules[4].layer.as_deref(), Some("l"));
        assert_eq!(s.rules[5].container.as_deref(), Some("card (min-width: 400px)"));
        assert!(container_query("(min-width: 400px)", 500.0, f32::NAN));
        assert!(!container_query("(min-height: 10px)", 500.0, f32::NAN));
        assert!(container_query("(width > 100px) and (height < 50px)", 500.0, 20.0));
    }

    #[test]
    fn specificity_of_is_not_where() {
        let sp = |s: &str| parse_selector_list(s)[0].specificity();
        assert_eq!(sp(":is(#a, .b) p"), (1, 0, 1));
        assert_eq!(sp(":where(#a) p"), (0, 0, 1));
        assert_eq!(sp("a:not(.x.y)"), (0, 2, 1));
    }
}
