//! Style: selector matching, the cascade and computed values.

use crate::css::{self, Combinator, Compound, Decl, Selector, Stylesheet};
use crate::dom::{Dom, NodeData, NodeId};

pub const VIEWPORT_W: f32 = 1024.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 255 };
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    None,
    Block,
    Inline,
    ListItem,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Len {
    Px(f32),
    Pct(f32),
    Auto,
}

impl Len {
    pub fn resolve(self, base: f32) -> f32 {
        match self {
            Len::Px(v) => v,
            Len::Pct(p) => base * p / 100.0,
            Len::Auto => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ListStyle {
    Disc,
    Decimal,
    None,
}

#[derive(Debug, Clone)]
pub struct Style {
    pub display: Display,
    // inherited
    pub color: Color,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub underline: bool,
    pub text_align: Align,
    pub line_height: f32,
    pub pre: bool,
    pub list_style: ListStyle,
    pub hidden: bool,
    // not inherited
    pub background: Color,
    pub margin: [Len; 4], // top right bottom left
    pub padding: [Len; 4],
    pub border: [f32; 4],
    pub border_color: Color,
    pub width: Len,
    pub max_width: Len,
    pub height: Len,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            display: Display::Inline,
            color: Color::BLACK,
            font_size: 16.0,
            bold: false,
            italic: false,
            mono: false,
            underline: false,
            text_align: Align::Left,
            line_height: 1.25,
            pre: false,
            list_style: ListStyle::Disc,
            hidden: false,
            background: Color::TRANSPARENT,
            margin: [Len::Px(0.0); 4],
            padding: [Len::Px(0.0); 4],
            border: [0.0; 4],
            border_color: Color::BLACK,
            width: Len::Auto,
            max_width: Len::Auto,
            height: Len::Auto,
        }
    }
}

impl Style {
    fn inherit(parent: &Style) -> Style {
        Style {
            color: parent.color,
            font_size: parent.font_size,
            bold: parent.bold,
            italic: parent.italic,
            mono: parent.mono,
            underline: parent.underline,
            text_align: parent.text_align,
            line_height: parent.line_height,
            pre: parent.pre,
            list_style: parent.list_style,
            hidden: parent.hidden,
            ..Style::default()
        }
    }
}

/// The browser's built-in default styles (the "user agent stylesheet").
pub const UA_CSS: &str = r#"
html, body, div, p, h1, h2, h3, h4, h5, h6, ul, ol, dl, dt, dd, pre, blockquote, form, fieldset,
header, footer, main, nav, section, article, aside, figure, figcaption, address, hr, table, tr,
thead, tbody, tfoot, caption, center, details, summary, menu, legend, hgroup, search, dialog { display: block; }
td, th { display: block; }
li { display: list-item; }
head, script, style, title, meta, link, noscript, template, base, datalist, dialog, svg { display: none; }
[hidden] { display: none; }
body { margin: 8px; }
p, dl, blockquote, figure { margin-top: 1em; margin-bottom: 1em; }
h1 { font-size: 2em; margin-top: 0.67em; margin-bottom: 0.67em; font-weight: bold; }
h2 { font-size: 1.5em; margin-top: 0.83em; margin-bottom: 0.83em; font-weight: bold; }
h3 { font-size: 1.17em; margin-top: 1em; margin-bottom: 1em; font-weight: bold; }
h4 { margin-top: 1.33em; margin-bottom: 1.33em; font-weight: bold; }
h5 { font-size: 0.83em; font-weight: bold; }
h6 { font-size: 0.67em; font-weight: bold; }
ul, ol, menu { margin-top: 1em; margin-bottom: 1em; padding-left: 40px; }
ol { list-style-type: decimal; }
li ul, li ol { margin-top: 0; margin-bottom: 0; }
dd { margin-left: 40px; }
blockquote, figure { margin-left: 40px; margin-right: 40px; }
b, strong, th, dt { font-weight: bold; }
i, em, cite, var, dfn, address { font-style: italic; }
code, kbd, samp, tt, pre { font-family: monospace; }
pre { white-space: pre; margin-top: 1em; margin-bottom: 1em; }
a:link { color: #0000ee; text-decoration: underline; }
u, ins { text-decoration: underline; }
small { font-size: 0.83em; }
big { font-size: 1.2em; }
center { text-align: center; }
hr { border: 1px solid #888; margin-top: 0.5em; margin-bottom: 0.5em; }
th, td { padding: 1px; }
button, input, select, textarea { display: inline; }
"#;

struct Matched<'a> {
    important: bool,
    origin: u8, // 0 = UA, 1 = author, 2 = inline
    spec: (u32, u32, u32),
    order: usize,
    decl: &'a Decl,
}

/// Compute a style for every node (text nodes share their parent's inherited style).
pub fn compute(dom: &Dom, sheets: &[Stylesheet]) -> Vec<Style> {
    let ua = css::parse(UA_CSS);
    let mut styles: Vec<Style> = vec![Style { display: Display::None, ..Style::default() }; dom.nodes.len()];
    styles[0].display = Display::Block;
    // Walk the tree parent-first (scripts can move nodes, so ids aren't in tree order).
    let mut stack: Vec<NodeId> = dom.nodes[0].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        stack.extend(dom.nodes[id].children.iter().rev().copied());
        let parent = dom.nodes[id].parent.unwrap_or(0);
        let parent_style = styles[parent].clone();
        let mut st = Style::inherit(&parent_style);
        if let NodeData::Element { .. } = dom.nodes[id].data {
            let mut matched: Vec<Matched> = Vec::new();
            let mut order = 0;
            for (origin, sheet) in std::iter::once(&ua).chain(sheets.iter()).enumerate() {
                let origin = if origin == 0 { 0 } else { 1 };
                for rule in &sheet.rules {
                    let best = rule
                        .selectors
                        .iter()
                        .filter(|s| matches(dom, id, s))
                        .map(|s| s.specificity())
                        .max();
                    if let Some(spec) = best {
                        for d in &rule.decls {
                            matched.push(Matched { important: d.important, origin, spec, order, decl: d });
                            order += 1;
                        }
                    }
                }
            }
            let inline = dom.attr(id, "style").map(css::parse_declarations).unwrap_or_default();
            for d in &inline {
                matched.push(Matched { important: d.important, origin: 2, spec: (9, 9, 9), order, decl: d });
                order += 1;
            }
            matched.sort_by_key(|m| (m.important, m.origin, m.spec, m.order));
            presentational_hints(dom, id, &mut st, &parent_style);
            for m in matched {
                apply(&mut st, &parent_style, &m.decl.name, &m.decl.value);
            }
        } else {
            st.display = Display::Inline;
        }
        styles[id] = st;
    }
    styles
}

/// Old-fashioned HTML attributes that act like CSS (bgcolor, align, <font color>...).
fn presentational_hints(dom: &Dom, id: NodeId, st: &mut Style, parent: &Style) {
    if let Some(c) = dom.attr(id, "bgcolor").and_then(parse_color) {
        st.background = c;
    }
    if dom.tag(id) == Some("font") {
        if let Some(c) = dom.attr(id, "color").and_then(parse_color) {
            st.color = c;
        }
    }
    if let Some(a) = dom.attr(id, "align") {
        apply(st, parent, "text-align", a);
    }
    if let Some(w) = dom.attr(id, "width") {
        if dom.tag(id) != Some("img") {
            let v = if w.chars().all(|c| c.is_ascii_digit()) { format!("{}px", w) } else { w.to_string() };
            apply(st, parent, "width", &v);
        }
    }
}

pub fn matches(dom: &Dom, id: NodeId, sel: &Selector) -> bool {
    match_from(dom, id, sel, sel.parts.len() - 1)
}

fn match_from(dom: &Dom, id: NodeId, sel: &Selector, idx: usize) -> bool {
    if !compound_matches(dom, id, &sel.parts[idx]) {
        return false;
    }
    if idx == 0 {
        return true;
    }
    let mut cur = dom.nodes[id].parent;
    match sel.combinators[idx - 1] {
        Combinator::Child => cur.map_or(false, |p| match_from(dom, p, sel, idx - 1)),
        Combinator::Descendant => {
            while let Some(p) = cur {
                if match_from(dom, p, sel, idx - 1) {
                    return true;
                }
                cur = dom.nodes[p].parent;
            }
            false
        }
    }
}

fn compound_matches(dom: &Dom, id: NodeId, c: &Compound) -> bool {
    let Some(tag) = dom.tag(id) else { return false };
    if let Some(t) = &c.tag {
        if t != tag {
            return false;
        }
    }
    if let Some(want) = &c.id {
        if dom.attr(id, "id") != Some(want.as_str()) {
            return false;
        }
    }
    for cls in &c.classes {
        if !dom.classes(id).any(|x| x == cls) {
            return false;
        }
    }
    for (k, v) in &c.attrs {
        match (dom.attr(id, k), v) {
            (None, _) => return false,
            (Some(actual), Some(want)) if actual != want => return false,
            _ => {}
        }
    }
    for p in &c.pseudo {
        let ok = match p.as_str() {
            "link" | "any-link" => tag == "a" && dom.attr(id, "href").is_some(),
            "visited" => false,
            "root" => tag == "html",
            "first-child" | "last-child" => {
                let parent = dom.nodes[id].parent.unwrap_or(0);
                let elems: Vec<NodeId> = dom.nodes[parent].children.iter().copied().filter(|&c| dom.tag(c).is_some()).collect();
                if p == "first-child" { elems.first() == Some(&id) } else { elems.last() == Some(&id) }
            }
            _ => false, // :hover, :focus, ::before... not supported yet
        };
        if !ok {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------- values

pub fn parse_len(v: &str, font_size: f32) -> Option<Len> {
    let v = v.trim().to_ascii_lowercase();
    if v == "auto" {
        return Some(Len::Auto);
    }
    let num_end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(v.len());
    let num: f32 = v[..num_end].parse().ok()?;
    let unit = &v[num_end..];
    Some(match unit {
        "" | "px" => Len::Px(num),
        "em" => Len::Px(num * font_size),
        "rem" => Len::Px(num * 16.0),
        "%" => Len::Pct(num),
        "pt" => Len::Px(num * 4.0 / 3.0),
        "vw" => Len::Px(num * VIEWPORT_W / 100.0),
        "vh" => Len::Px(num * 7.68),
        "ch" | "ex" => Len::Px(num * font_size * 0.5),
        _ => return None,
    })
}

pub fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let h = |s: &str| u8::from_str_radix(s, 16).ok();
        let d = |c: char| h(&format!("{c}{c}"));
        let c: Vec<char> = hex.chars().collect();
        return match c.len() {
            3 | 4 => Some(Color { r: d(c[0])?, g: d(c[1])?, b: d(c[2])?, a: if c.len() == 4 { d(c[3])? } else { 255 } }),
            6 | 8 => Some(Color {
                r: h(&hex[0..2])?,
                g: h(&hex[2..4])?,
                b: h(&hex[4..6])?,
                a: if c.len() == 8 { h(&hex[6..8])? } else { 255 },
            }),
            _ => None,
        };
    }
    if v.starts_with("rgb") {
        let inner = v.split_once('(')?.1.trim_end_matches(')');
        let parts: Vec<&str> = inner.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
        let comp = |s: &str| -> Option<u8> {
            Some(if let Some(p) = s.strip_suffix('%') { (p.parse::<f32>().ok()? * 2.55) as u8 } else { s.parse::<f32>().ok()?.clamp(0.0, 255.0) as u8 })
        };
        let a = match parts.get(3) {
            Some(s) => {
                if let Some(p) = s.strip_suffix('%') { (p.parse::<f32>().ok()? * 2.55) as u8 } else { (s.parse::<f32>().ok()? * 255.0) as u8 }
            }
            None => 255,
        };
        return Some(Color { r: comp(parts.first()?)?, g: comp(parts.get(1)?)?, b: comp(parts.get(2)?)?, a });
    }
    Some(match v.as_str() {
        "transparent" => Color::TRANSPARENT,
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "orange" => Color::rgb(255, 165, 0),
        "purple" => Color::rgb(128, 0, 128),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "lightgray" | "lightgrey" => Color::rgb(211, 211, 211),
        "darkgray" | "darkgrey" => Color::rgb(169, 169, 169),
        "maroon" => Color::rgb(128, 0, 0),
        "navy" => Color::rgb(0, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "olive" => Color::rgb(128, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "aqua" | "cyan" => Color::rgb(0, 255, 255),
        "fuchsia" | "magenta" => Color::rgb(255, 0, 255),
        "brown" => Color::rgb(165, 42, 42),
        "pink" => Color::rgb(255, 192, 203),
        "gold" => Color::rgb(255, 215, 0),
        "beige" => Color::rgb(245, 245, 220),
        "ivory" => Color::rgb(255, 255, 240),
        "whitesmoke" => Color::rgb(245, 245, 245),
        "darkblue" => Color::rgb(0, 0, 139),
        "darkgreen" => Color::rgb(0, 100, 0),
        "darkred" => Color::rgb(139, 0, 0),
        "steelblue" => Color::rgb(70, 130, 180),
        "slategray" | "slategrey" => Color::rgb(112, 128, 144),
        "dimgray" | "dimgrey" => Color::rgb(105, 105, 105),
        _ => return None,
    })
}

fn font_size_value(v: &str, parent: f32) -> Option<f32> {
    Some(match v.trim() {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "smaller" => parent * 0.83,
        "larger" => parent * 1.2,
        other => parse_len(other, parent)?.resolve(parent),
    })
}

/// Expand 1–4 values into [top, right, bottom, left].
fn four<T: Copy>(vals: &[T]) -> Option<[T; 4]> {
    Some(match vals.len() {
        1 => [vals[0]; 4],
        2 => [vals[0], vals[1], vals[0], vals[1]],
        3 => [vals[0], vals[1], vals[2], vals[1]],
        4 => [vals[0], vals[1], vals[2], vals[3]],
        _ => return None,
    })
}

fn side_index(s: &str) -> Option<usize> {
    ["top", "right", "bottom", "left"].iter().position(|x| *x == s)
}

fn border_width_word(t: &str, fs: f32) -> Option<f32> {
    match t {
        "thin" => Some(1.0),
        "medium" => Some(3.0),
        "thick" => Some(5.0),
        _ => parse_len(t, fs).map(|l| l.resolve(0.0)),
    }
}

/// Parse a `border` shorthand: returns (width, color).
fn border_shorthand(v: &str, fs: f32) -> (f32, Option<Color>) {
    let mut width = 3.0;
    let mut color = None;
    let mut has_style = false;
    for t in css::split_top(v, ' ') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if ["none", "hidden"].contains(&t) {
            return (0.0, None);
        }
        if ["solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset"].contains(&t) {
            has_style = true;
        } else if let Some(w) = border_width_word(t, fs) {
            width = w;
        } else if let Some(c) = parse_color(t) {
            color = Some(c);
        }
    }
    (if has_style { width } else { 0.0 }, color)
}

pub fn apply(st: &mut Style, parent: &Style, name: &str, value: &str) {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if lower == "inherit" {
        return; // inherited props already inherited; good enough for now
    }
    let fs = st.font_size;
    let lens = |s: &str| -> Option<Vec<Len>> { s.split_whitespace().map(|t| parse_len(t, fs)).collect() };
    match name {
        "display" => {
            st.display = match lower.as_str() {
                "none" => Display::None,
                "inline" | "inline-block" | "inline-flex" | "inline-grid" | "contents" | "inline-table" => Display::Inline,
                "list-item" => Display::ListItem,
                _ => Display::Block,
            }
        }
        "visibility" => st.hidden = lower == "hidden" || lower == "collapse",
        "color" => {
            if let Some(c) = parse_color(&lower) {
                st.color = c
            }
        }
        "background-color" => {
            if let Some(c) = parse_color(&lower) {
                st.background = c
            }
        }
        "background" => {
            if lower == "none" {
                st.background = Color::TRANSPARENT;
            }
            for t in css::split_top(&lower, ' ') {
                if let Some(c) = parse_color(&t) {
                    st.background = c;
                }
            }
        }
        "font-size" => {
            if let Some(px) = font_size_value(&lower, parent.font_size) {
                st.font_size = px.max(1.0);
            }
        }
        "font-weight" => {
            st.bold = match lower.as_str() {
                "bold" | "bolder" => true,
                "normal" | "lighter" => false,
                n => n.parse::<u32>().map(|w| w >= 600).unwrap_or(st.bold),
            }
        }
        "font-style" => st.italic = lower == "italic" || lower.starts_with("oblique"),
        "font-family" => st.mono = lower.contains("mono") || lower.contains("courier") || lower.contains("consolas"),
        "font" => {
            // e.g. "italic bold 16px/1.5 Georgia, serif"
            let mut rest = lower.as_str();
            st.italic = false;
            st.bold = false;
            while let Some((tok, after)) = rest.split_once(' ') {
                if tok.chars().next().map_or(false, |c| c.is_ascii_digit() || c == '.') && !tok.chars().all(|c| c.is_ascii_digit()) {
                    break;
                }
                match tok {
                    "italic" | "oblique" => st.italic = true,
                    "bold" | "bolder" | "600" | "700" | "800" | "900" => st.bold = true,
                    _ => {}
                }
                rest = after.trim_start();
            }
            let (size_tok, families) = rest.split_once(' ').unwrap_or((rest, ""));
            let (size, lh) = size_tok.split_once('/').map(|(a, b)| (a, Some(b))).unwrap_or((size_tok, None));
            if let Some(px) = font_size_value(size, parent.font_size) {
                st.font_size = px;
            }
            if let Some(lh) = lh {
                apply(st, parent, "line-height", lh);
            }
            st.mono = families.contains("mono") || families.contains("courier");
        }
        "line-height" => {
            st.line_height = if lower == "normal" {
                1.25
            } else if let Ok(n) = lower.parse::<f32>() {
                n
            } else {
                match parse_len(&lower, fs) {
                    Some(Len::Px(px)) => px / fs.max(1.0),
                    Some(Len::Pct(p)) => p / 100.0,
                    _ => st.line_height,
                }
            }
        }
        "text-align" => {
            st.text_align = match lower.as_str() {
                "center" | "-webkit-center" | "middle" => Align::Center,
                "right" | "end" => Align::Right,
                _ => Align::Left,
            }
        }
        "white-space" => st.pre = lower.starts_with("pre") && lower != "pre-line",
        "text-decoration" | "text-decoration-line" => st.underline = lower.contains("underline"),
        "list-style" | "list-style-type" => {
            if lower.contains("none") {
                st.list_style = ListStyle::None;
            } else if lower.contains("decimal") || lower.contains("roman") || lower.contains("alpha") {
                st.list_style = ListStyle::Decimal;
            } else if lower.contains("disc") || lower.contains("circle") || lower.contains("square") {
                st.list_style = ListStyle::Disc;
            }
        }
        "margin" => {
            if let Some(m) = lens(&lower).and_then(|l| four(&l)) {
                st.margin = m
            }
        }
        "padding" => {
            if let Some(p) = lens(&lower).and_then(|l| four(&l)) {
                st.padding = p
            }
        }
        "width" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.width = l
            }
        }
        "max-width" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.max_width = l
            } else if lower == "none" {
                st.max_width = Len::Auto
            }
        }
        "height" | "min-height" => {
            if let Some(l @ Len::Px(_)) = parse_len(&lower, fs) {
                st.height = l
            }
        }
        "border" => {
            let (w, c) = border_shorthand(&lower, fs);
            st.border = [w; 4];
            if let Some(c) = c {
                st.border_color = c;
            }
        }
        "border-width" => {
            let ws: Option<Vec<f32>> = lower.split_whitespace().map(|t| border_width_word(t, fs)).collect();
            if let Some(w) = ws.and_then(|w| four(&w)) {
                st.border = w;
            }
        }
        "border-color" => {
            if let Some(c) = lower.split_whitespace().next().and_then(parse_color) {
                st.border_color = c;
            }
        }
        "border-style" if lower == "none" || lower == "hidden" => st.border = [0.0; 4],
        _ => {
            if let Some(side) = name.strip_prefix("margin-").and_then(side_index) {
                if let Some(l) = parse_len(&lower, fs) {
                    st.margin[side] = l;
                }
            } else if let Some(side) = name.strip_prefix("padding-").and_then(side_index) {
                if let Some(l) = parse_len(&lower, fs) {
                    st.padding[side] = l;
                }
            } else if let Some(rest) = name.strip_prefix("border-") {
                let (side, prop) = rest.split_once('-').unwrap_or((rest, ""));
                if let Some(i) = side_index(side) {
                    match prop {
                        "" => {
                            let (w, c) = border_shorthand(&lower, fs);
                            st.border[i] = w;
                            if let Some(c) = c {
                                st.border_color = c;
                            }
                        }
                        "width" => {
                            if let Some(w) = border_width_word(&lower, fs) {
                                st.border[i] = w;
                            }
                        }
                        "color" => {
                            if let Some(c) = parse_color(&lower) {
                                st.border_color = c;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
