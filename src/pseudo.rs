//! Generated content: `::before` and `::after` boxes, the `content` property and CSS
//! counters.
//!
//! Styles for pseudo-elements are computed with their element (see `style::Pseudos`).
//! Before layout, `expand` makes a copy of the tree with an extra element at the start
//! (`::before`) and end (`::after`) of each element that has one, holding the generated
//! text. The extra nodes get ids after the real ones, so everything else keeps its id;
//! layout maps hits on them back to their element.

use crate::dom::{Dom, NodeData, NodeId};
use crate::style::{Display, Style};
use std::rc::Rc;

/// Tag names of the generated elements.
pub const BEFORE: &str = "::before";
pub const AFTER: &str = "::after";

/// Elements whose content is replaced (images, form controls): they get no ::before/::after.
fn replaced(tag: &str) -> bool {
    matches!(tag, "img" | "input" | "textarea" | "select" | "iframe" | "video" | "audio" | "canvas" | "embed" | "object" | "br" | "hr" | "meter" | "progress")
}

/// The tree with generated boxes added, and styles for every node of it. None if no
/// element has a ::before or ::after with content.
pub fn expand(dom: &Dom, styles: &[Rc<Style>]) -> Option<(Dom, Vec<Rc<Style>>)> {
    if styles.len() < dom.nodes.len() || !styles.iter().any(|s| s.pseudo.as_ref().is_some_and(|p| p.before.is_some() || p.after.is_some())) {
        return None;
    }
    let mut out = dom.clone();
    let mut st: Vec<Rc<Style>> = styles[..dom.nodes.len()].to_vec();
    let mut counters = Counters::default();
    let mut added = false;
    walk(dom, styles, Dom::ROOT, &mut out, &mut st, &mut counters, &mut added, 0);
    added.then_some((out, st))
}

#[allow(clippy::too_many_arguments)]
fn walk(dom: &Dom, styles: &[Rc<Style>], id: NodeId, out: &mut Dom, st: &mut Vec<Rc<Style>>, counters: &mut Counters, added: &mut bool, depth: usize) {
    let mark = counters.list.len();
    let mut quote_depth = 0usize;
    for &c in &dom.nodes[id].children {
        let NodeData::Element { tag, .. } = &dom.nodes[c].data else { continue };
        let s = &styles[c];
        if s.display == Display::None {
            continue;
        }
        counters.apply(s, dom, c);
        let pseudo = s.pseudo.clone().filter(|_| !replaced(tag));
        if let Some(b) = pseudo.as_ref().and_then(|p| p.before.clone()) {
            if let Some(text) = resolve(&b, dom, c, counters, &mut quote_depth) {
                let first = out.nodes[c].children.first().copied();
                add_box(out, st, c, BEFORE, b, text, first);
                *added = true;
            }
        }
        if depth < 1024 {
            walk(dom, styles, c, out, st, counters, added, depth + 1);
        }
        if let Some(a) = pseudo.as_ref().and_then(|p| p.after.clone()) {
            if let Some(text) = resolve(&a, dom, c, counters, &mut quote_depth) {
                add_box(out, st, c, AFTER, a, text, None);
                *added = true;
            }
        }
    }
    // Counters made by children reach their later siblings, not past the parent.
    counters.list.truncate(mark);
}

fn add_box(out: &mut Dom, st: &mut Vec<Rc<Style>>, parent: NodeId, tag: &str, style: Rc<Style>, text: String, before: Option<NodeId>) {
    let el = out.create(NodeData::Element { tag: tag.to_string(), attrs: Vec::new() });
    out.insert(parent, el, before);
    st.push(style.clone());
    if !text.is_empty() {
        let t = out.create(NodeData::Text(text));
        out.insert(el, t, None);
        st.push(style);
    }
}

/// The text a ::before/::after box shows, or None if it has no box
/// (`content: none`/`normal`, or `display: none`).
fn resolve(s: &Style, dom: &Dom, el: NodeId, counters: &Counters, quotes: &mut usize) -> Option<String> {
    if s.display == Display::None {
        return None;
    }
    let content = s.content.as_deref()?;
    content_text(content, dom, el, counters, quotes)
}

/// Evaluate a `content` value: strings, `attr()`, `counter()`, `counters()` and quotes.
/// None for `none` and `normal`.
pub fn content_text(content: &str, dom: &Dom, el: NodeId, counters: &Counters, quotes: &mut usize) -> Option<String> {
    // `content: "x" / "alt text"`: the part before the slash is shown.
    let main = top_level_slash(content);
    let t = main.trim();
    if t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("normal") || t.is_empty() {
        return None;
    }
    let mut out = String::new();
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '"' || c == '\'' {
            let (s, n) = string(&chars, i);
            out.push_str(&s);
            i = n;
        } else {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_') {
                i += 1;
            }
            let name = chars[start..i].iter().collect::<String>().to_ascii_lowercase();
            if i < chars.len() && chars[i] == '(' {
                let mut depth = 0;
                let arg_start = i + 1;
                while i < chars.len() {
                    if chars[i] == '"' || chars[i] == '\'' {
                        i = string(&chars, i).1;
                        continue;
                    }
                    if chars[i] == '(' {
                        depth += 1;
                    } else if chars[i] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                let arg: String = chars[arg_start..i.min(chars.len())].iter().collect();
                i += 1;
                let args: Vec<String> = crate::css::split_top(&arg, ',').into_iter().map(|a| a.trim().to_string()).collect();
                match name.as_str() {
                    "attr" => {
                        let a = args.first().map(|a| a.split_whitespace().next().unwrap_or("").to_ascii_lowercase()).unwrap_or_default();
                        let fallback = args.get(1).map(|f| unquote(f)).unwrap_or_default();
                        out.push_str(dom.attr(el, &a).map(str::to_string).unwrap_or(fallback).as_str());
                    }
                    "counter" => {
                        let n = args.first().map(String::as_str).unwrap_or("");
                        let style = args.get(1).map(|s| s.to_ascii_lowercase()).unwrap_or_else(|| "decimal".into());
                        out.push_str(&format_counter(counters.get(n), &style));
                    }
                    "counters" => {
                        let n = args.first().map(String::as_str).unwrap_or("");
                        let sep = args.get(1).map(|s| unquote(s)).unwrap_or_default();
                        let style = args.get(2).map(|s| s.to_ascii_lowercase()).unwrap_or_else(|| "decimal".into());
                        let all: Vec<String> = counters.all(n).into_iter().map(|v| format_counter(v, &style)).collect();
                        out.push_str(&all.join(&sep));
                    }
                    // url() images and anything else aren't drawn.
                    _ => {}
                }
            } else {
                match name.as_str() {
                    "open-quote" => {
                        out.push(if *quotes % 2 == 0 { '\u{201C}' } else { '\u{2018}' });
                        *quotes += 1;
                    }
                    "close-quote" => {
                        if *quotes > 0 {
                            *quotes -= 1;
                            out.push(if *quotes % 2 == 0 { '\u{201D}' } else { '\u{2019}' });
                        }
                    }
                    "no-open-quote" => *quotes += 1,
                    "no-close-quote" => *quotes = quotes.saturating_sub(1),
                    _ => {}
                }
                if i == start {
                    i += 1;
                }
            }
        }
    }
    Some(out)
}

fn top_level_slash(v: &str) -> &str {
    let mut quote = None;
    let mut depth = 0;
    for (i, c) in v.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, '/') if depth == 0 => return &v[..i],
            _ => {}
        }
    }
    v
}

fn unquote(s: &str) -> String {
    let chars: Vec<char> = s.trim().chars().collect();
    if chars.first().is_some_and(|&c| c == '"' || c == '\'') {
        string(&chars, 0).0
    } else {
        s.trim().to_string()
    }
}

/// A CSS string starting at `chars[i]` (a quote), with escapes resolved.
fn string(chars: &[char], i: usize) -> (String, usize) {
    let q = chars[i];
    let mut out = String::new();
    let mut j = i + 1;
    while j < chars.len() && chars[j] != q {
        if chars[j] == '\\' && j + 1 < chars.len() {
            j += 1;
            let hex: String = chars[j..].iter().take(6).take_while(|c| c.is_ascii_hexdigit()).collect();
            if !hex.is_empty() {
                j += hex.len();
                if j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
                out.push(char::from_u32(cp).filter(|&c| c != '\0').unwrap_or('\u{FFFD}'));
                continue;
            }
            if chars[j] != '\n' {
                out.push(chars[j]);
            }
            j += 1;
            continue;
        }
        out.push(chars[j]);
        j += 1;
    }
    (out, j + 1)
}

/// The counters in scope while walking the tree: (name, value), innermost last.
#[derive(Default, Debug)]
pub struct Counters {
    pub list: Vec<(String, i32)>,
}

impl Counters {
    /// An element's counter-reset, counter-set and counter-increment (and list items'
    /// implicit `list-item` counter).
    pub fn apply(&mut self, s: &Style, dom: &Dom, id: NodeId) {
        let tag = dom.tag(id).unwrap_or("");
        if matches!(tag, "ol" | "ul" | "menu") {
            let start = dom.attr(id, "start").and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(1);
            self.list.push(("list-item".into(), start - 1));
        }
        for kind in [0u8, 1, 2] {
            for (k, v) in s.counters.as_deref().unwrap_or(&[]) {
                if *k != kind {
                    continue;
                }
                let words: Vec<&str> = v.split_whitespace().collect();
                let mut i = 0;
                while i < words.len() {
                    let name = words[i];
                    let num = words.get(i + 1).and_then(|n| n.parse::<i32>().ok());
                    i += if num.is_some() { 2 } else { 1 };
                    match kind {
                        0 => self.list.push((name.to_string(), num.unwrap_or(0))),
                        1 => self.set(name, num.unwrap_or(0), false),
                        _ => self.set(name, num.unwrap_or(1), true),
                    }
                }
            }
        }
        if s.display == Display::ListItem && !s.counters.as_deref().unwrap_or(&[]).iter().any(|(k, v)| *k == 2 && v.contains("list-item")) {
            let n = dom.attr(id, "value").filter(|_| tag == "li").and_then(|v| v.trim().parse::<i32>().ok());
            match n {
                Some(n) => self.set("list-item", n, false),
                None => self.set("list-item", 1, true),
            }
        }
    }

    fn set(&mut self, name: &str, v: i32, add: bool) {
        match self.list.iter_mut().rev().find(|(n, _)| n == name) {
            Some((_, cur)) => *cur = if add { cur.saturating_add(v) } else { v },
            // Using a counter that doesn't exist creates it.
            None => self.list.push((name.to_string(), v)),
        }
    }

    pub fn get(&self, name: &str) -> i32 {
        self.list.iter().rev().find(|(n, _)| n == name).map_or(0, |c| c.1)
    }

    pub fn all(&self, name: &str) -> Vec<i32> {
        let v: Vec<i32> = self.list.iter().filter(|(n, _)| n == name).map(|c| c.1).collect();
        if v.is_empty() { vec![0] } else { v }
    }
}

/// A counter value in a list style: decimal, roman numerals, letters, bullets.
pub fn format_counter(n: i32, style: &str) -> String {
    match style {
        "none" => String::new(),
        "disc" => "\u{2022}".into(),
        "circle" => "\u{25E6}".into(),
        "square" => "\u{25AA}".into(),
        "decimal-leading-zero" => if (0..10).contains(&n) { format!("0{n}") } else { n.to_string() },
        "lower-roman" | "upper-roman" if (1..4000).contains(&n) => {
            let r = roman(n as u32);
            if style == "upper-roman" { r } else { r.to_lowercase() }
        }
        "lower-alpha" | "lower-latin" | "upper-alpha" | "upper-latin" if n > 0 => {
            let mut v = n as u32;
            let mut s = Vec::new();
            while v > 0 {
                v -= 1;
                s.push((b'a' + (v % 26) as u8) as char);
                v /= 26;
            }
            let s: String = s.into_iter().rev().collect();
            if style.starts_with("upper") { s.to_uppercase() } else { s }
        }
        "lower-greek" if n > 0 && n <= 24 => char::from_u32(0x3B1 + n as u32 - 1 + if n > 17 { 1 } else { 0 }).map(String::from).unwrap_or_default(),
        _ => n.to_string(),
    }
}

fn roman(mut n: u32) -> String {
    let table = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content: &str) -> Option<String> {
        let mut dom = Dom::new();
        let el = dom.add(NodeData::Element { tag: "a".into(), attrs: vec![("href".into(), "/x".into())] }, Dom::ROOT);
        let mut c = Counters::default();
        c.list.push(("sec".into(), 2));
        c.list.push(("sec".into(), 5));
        content_text(content, &dom, el, &c, &mut 0)
    }

    #[test]
    fn content_values() {
        assert_eq!(text("none"), None);
        assert_eq!(text("normal"), None);
        assert_eq!(text("\"\""), Some(String::new()));
        assert_eq!(text("\"(\" attr(href) \")\""), Some("(/x)".into()));
        assert_eq!(text("'\\201C' 'a\\'b'"), Some("\u{201C}a'b".into()));
        assert_eq!(text("counter(sec) '.'"), Some("5.".into()));
        assert_eq!(text("counters(sec, '.') ' '"), Some("2.5 ".into()));
        assert_eq!(text("counter(sec, upper-roman)"), Some("V".into()));
        assert_eq!(text("open-quote 'x' close-quote"), Some("\u{201C}x\u{201D}".into()));
        assert_eq!(text("\"\\2192\" / \"\""), Some("\u{2192}".into()));
    }

    #[test]
    fn counter_styles() {
        assert_eq!(format_counter(4, "lower-roman"), "iv");
        assert_eq!(format_counter(28, "upper-alpha"), "AB");
        assert_eq!(format_counter(7, "decimal-leading-zero"), "07");
    }
}
