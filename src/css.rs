//! CSS parser: stylesheets -> rules -> selectors + declarations.

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub decls: Vec<Decl>,
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
}

#[derive(Debug, Clone, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    /// [attr] or [attr=value]
    pub attrs: Vec<(String, Option<String>)>,
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
    parse_block(&clean, &mut rules);
    Stylesheet { rules }
}

/// Parse a declaration list like the `style=""` attribute.
pub fn parse_declarations(src: &str) -> Vec<Decl> {
    split_top(src, ';')
        .into_iter()
        .filter_map(|d| {
            let (name, value) = d.split_once(':')?;
            let name = name.trim();
            // Custom property names are case-sensitive; everything else isn't.
            let name = if name.starts_with("--") { name.to_string() } else { name.to_ascii_lowercase() };
            let mut value = value.trim().to_string();
            let important = value.to_ascii_lowercase().ends_with("!important");
            if important {
                value.truncate(value.len() - "!important".len());
                value = value.trim().to_string();
            }
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some(Decl { name, value, important })
        })
        .collect()
}

fn parse_block(src: &str, rules: &mut Vec<Rule>) {
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
            if lower.starts_with("media") && media_matches(&lower[5..]) {
                parse_block(body, rules);
            } else if lower.starts_with("supports") || lower.starts_with("layer") {
                parse_block(body, rules);
            }
            continue;
        }
        let selectors: Vec<Selector> = split_top(prelude, ',').iter().filter_map(|s| parse_selector(s)).collect();
        if selectors.is_empty() {
            continue;
        }
        rules.push(Rule { selectors, decls: parse_declarations(body) });
    }
}

/// Pretend to be a desktop screen ~1024px wide.
fn media_matches(q: &str) -> bool {
    if q.contains("print") && !q.contains("screen") {
        return false;
    }
    if let Some(pos) = q.find("max-width") {
        if let Some(px) = number_after(&q[pos..]) {
            return px >= 1024.0;
        }
    }
    if let Some(pos) = q.find("min-width") {
        if let Some(px) = number_after(&q[pos..]) {
            return px <= 1024.0;
        }
    }
    !q.contains("prefers-color-scheme: dark") && !q.contains("prefers-color-scheme:dark")
}

fn number_after(s: &str) -> Option<f32> {
    let s = s.split_once(':')?.1.trim_start();
    let num: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let v: f32 = num.parse().ok()?;
    Some(if s[num.len()..].starts_with("em") { v * 16.0 } else { v })
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
            return None; // sibling combinators not supported yet
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
                    if k.ends_with(['~', '|', '^', '$', '*']) {
                        return None; // fancy attribute matchers not supported yet
                    }
                    let v = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
                    c.attrs.push((k.trim().to_ascii_lowercase(), Some(v)));
                } else {
                    c.attrs.push((inner.trim().to_ascii_lowercase(), None));
                }
                i = end + 1;
            }
            ':' => {
                let mut j = i + 1;
                if j < chars.len() && chars[j] == ':' {
                    j += 1;
                }
                let (s, mut n) = ident(chars, j);
                if n < chars.len() && chars[n] == '(' {
                    // skip functional pseudo-class arguments
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
                    c.pseudo.push(format!("{}()", s.to_ascii_lowercase()));
                } else {
                    c.pseudo.push(s.to_ascii_lowercase());
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
