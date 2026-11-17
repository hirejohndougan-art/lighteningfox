//! GLSL ES shaders for WebGL (1.00 for WebGL 1, 3.00 for WebGL 2): the preprocessor, the
//! tokenizer, the parser and the syntax tree. `glsl_gen` compiles the tree for `glsl_vm`.

use std::collections::HashMap;
use std::rc::Rc;

// ---------------------------------------------------------------- tokens

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    /// An integer literal and whether it had a `u` suffix.
    Int(u64, bool),
    Float(f64),
    Punct(&'static str),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub t: Tok,
    pub line: u32,
}

const PUNCTS: &[&str] = &[
    "<<=", ">>=", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "^^", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "##", "(",
    ")", "[", "]", "{", "}", ".", ",", ";", ":", "?", "+", "-", "*", "/", "%", "<", ">", "!", "~", "&", "|", "^", "=", "#",
];

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Split one line into tokens.
fn tokenize_line(s: &str, line: u32, out: &mut Vec<Token>) -> Result<(), String> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if is_ident_start(c) {
            let st = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            out.push(Token { t: Tok::Ident(s[st..i].to_string()), line });
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            let st = i;
            if c == b'0' && i + 1 < b.len() && (b[i + 1] == b'x' || b[i + 1] == b'X') {
                i += 2;
                let hs = i;
                while i < b.len() && b[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let v = u64::from_str_radix(&s[hs..i], 16).map_err(|_| format!("{line}: bad hex literal"))?;
                let u = i < b.len() && (b[i] == b'u' || b[i] == b'U');
                if u {
                    i += 1;
                }
                out.push(Token { t: Tok::Int(v, u), line });
                continue;
            }
            let mut float = false;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i < b.len() && b[i] == b'.' {
                float = true;
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let save = i;
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                if i < b.len() && b[i].is_ascii_digit() {
                    float = true;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let text = &s[st..i];
            if i < b.len() && (b[i] == b'f' || b[i] == b'F') {
                float = true;
                i += 1;
            }
            if float {
                let v: f64 = text.parse().map_err(|_| format!("{line}: bad float literal '{text}'"))?;
                out.push(Token { t: Tok::Float(v), line });
            } else {
                let u = i < b.len() && (b[i] == b'u' || b[i] == b'U');
                if u {
                    i += 1;
                }
                let v = if text.len() > 1 && text.starts_with('0') {
                    u64::from_str_radix(&text[1..], 8).map_err(|_| format!("{line}: bad octal literal"))?
                } else {
                    text.parse::<u64>().map_err(|_| format!("{line}: bad integer literal"))?
                };
                out.push(Token { t: Tok::Int(v, u), line });
            }
            if i < b.len() && is_ident(b[i]) {
                return Err(format!("{line}: invalid character after number"));
            }
            continue;
        }
        let rest = &s[i..];
        match PUNCTS.iter().find(|p| rest.starts_with(**p)) {
            Some(p) => {
                out.push(Token { t: Tok::Punct(p), line });
                i += p.len();
            }
            None => return Err(format!("{line}: invalid character '{}'", rest.chars().next().unwrap_or('?'))),
        }
    }
    Ok(())
}

/// Remove comments (keeping line breaks) and join lines ending in a backslash.
fn strip_comments(src: &str) -> String {
    let src = src.replace("\r\n", "\n").replace('\r', "\n");
    let b = src.as_bytes();
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            out.push(' ');
            while i < b.len() && !(b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/') {
                if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        if b[i] == b'\\' && i + 1 < b.len() && b[i + 1] == b'\n' {
            // A continued line: keep the line count by moving the break after the line.
            i += 2;
            continue;
        }
        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

// ---------------------------------------------------------------- preprocessor

#[derive(Clone)]
struct Macro {
    params: Option<Vec<String>>,
    body: Vec<Token>,
}

pub struct Preprocessed {
    pub tokens: Vec<Token>,
    pub version: u16,
    pub extensions: Vec<String>,
}

/// Run the preprocessor. `supported` lists the extensions this implementation offers
/// (each gets its `GL_…` macro).
pub fn preprocess(src: &str, supported: &[&str]) -> Result<Preprocessed, String> {
    let text = strip_comments(src);
    let mut macros: HashMap<String, Macro> = HashMap::new();
    let one = |line| vec![Token { t: Tok::Int(1, false), line }];
    macros.insert("GL_ES".into(), Macro { params: None, body: one(0) });
    macros.insert("GL_FRAGMENT_PRECISION_HIGH".into(), Macro { params: None, body: one(0) });
    for e in supported {
        macros.insert(format!("GL_{}", e.trim_start_matches("GL_")), Macro { params: None, body: one(0) });
    }
    let mut version = 100u16;
    let mut seen_code = false;
    let mut extensions = Vec::new();
    // (this branch active, some branch taken, parent active)
    let mut conds: Vec<(bool, bool, bool)> = Vec::new();
    let mut out = Vec::new();
    let mut pending: Vec<Token> = Vec::new();
    let active = |c: &Vec<(bool, bool, bool)>| c.last().is_none_or(|x| x.0);
    for (ln, raw) in text.split('\n').enumerate() {
        let line = ln as u32 + 1;
        let t = raw.trim_start();
        if let Some(d) = t.strip_prefix('#') {
            let d = d.trim_start();
            let (name, rest) = match d.find(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                Some(p) => (&d[..p], &d[p..]),
                None => (d, ""),
            };
            if active(&conds) {
                flush(&mut pending, &macros, &mut out)?;
            }
            match name {
                "if" | "ifdef" | "ifndef" => {
                    let parent = active(&conds);
                    let v = if !parent {
                        false
                    } else if name == "if" {
                        eval_if(rest, &macros, line)? != 0
                    } else {
                        let id = rest.trim();
                        if id.is_empty() {
                            return Err(format!("{line}: #{name} needs a name"));
                        }
                        macros.contains_key(id) == (name == "ifdef")
                    };
                    conds.push((v, v, parent));
                }
                "elif" => {
                    let Some(top) = conds.last().copied() else { return Err(format!("{line}: #elif without #if")) };
                    let v = top.2 && !top.1 && eval_if(rest, &macros, line)? != 0;
                    *conds.last_mut().unwrap() = (v, top.1 || v, top.2);
                }
                "else" => {
                    let Some(top) = conds.last().copied() else { return Err(format!("{line}: #else without #if")) };
                    *conds.last_mut().unwrap() = (top.2 && !top.1, true, top.2);
                }
                "endif" => {
                    if conds.pop().is_none() {
                        return Err(format!("{line}: #endif without #if"));
                    }
                }
                _ if !active(&conds) => {}
                "" => {}
                "version" => {
                    if seen_code {
                        return Err(format!("{line}: #version must come first"));
                    }
                    let v: u16 = rest.split_whitespace().next().and_then(|s| s.parse().ok()).ok_or(format!("{line}: bad #version"))?;
                    if v != 100 && v != 300 {
                        return Err(format!("{line}: version {v} is not supported"));
                    }
                    version = v;
                }
                "define" => {
                    let r = rest.trim_start();
                    let end = r.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(r.len());
                    let mname = &r[..end];
                    if mname.is_empty() || !is_ident_start(mname.as_bytes()[0]) {
                        return Err(format!("{line}: bad #define"));
                    }
                    if mname.starts_with("GL_") || mname.contains("__") {
                        return Err(format!("{line}: reserved macro name '{mname}'"));
                    }
                    let after = &r[end..];
                    let (params, body) = if let Some(p) = after.strip_prefix('(') {
                        let close = p.find(')').ok_or(format!("{line}: bad macro parameters"))?;
                        let ps: Vec<String> = p[..close].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        (Some(ps), &p[close + 1..])
                    } else {
                        (None, after)
                    };
                    let mut toks = Vec::new();
                    tokenize_line(body, line, &mut toks)?;
                    macros.insert(mname.to_string(), Macro { params, body: toks });
                }
                "undef" => {
                    macros.remove(rest.trim());
                }
                "extension" => {
                    let parts: Vec<&str> = rest.split(':').map(|s| s.trim()).collect();
                    let ext = parts.first().copied().unwrap_or("");
                    let behavior = parts.get(1).copied().unwrap_or("");
                    let ok = ext == "all" || supported.iter().any(|s| s.trim_start_matches("GL_") == ext.trim_start_matches("GL_"));
                    if !ok && (behavior == "require") {
                        return Err(format!("{line}: extension '{ext}' is not supported"));
                    }
                    if ok && behavior != "disable" {
                        extensions.push(ext.to_string());
                    }
                }
                "pragma" | "line" => {}
                "error" => return Err(format!("{line}: #error {}", rest.trim())),
                other => return Err(format!("{line}: unknown directive #{other}")),
            }
            seen_code = true;
            continue;
        }
        if !t.is_empty() {
            seen_code = true;
        }
        if active(&conds) {
            tokenize_line(raw, line, &mut pending)?;
        }
    }
    if !conds.is_empty() {
        return Err("unterminated #if".into());
    }
    flush(&mut pending, &macros, &mut out)?;
    let ver = Macro { params: None, body: vec![Token { t: Tok::Int(version as u64, false), line: 0 }] };
    // __VERSION__ only matters after the fact for code that uses it as a value.
    for t in out.iter_mut() {
        if t.t == Tok::Ident("__VERSION__".into()) {
            t.t = ver.body[0].t.clone();
        }
    }
    Ok(Preprocessed { tokens: out, version, extensions })
}

fn flush(pending: &mut Vec<Token>, macros: &HashMap<String, Macro>, out: &mut Vec<Token>) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }
    let toks = std::mem::take(pending);
    let mut disabled = Vec::new();
    out.extend(expand(&toks, macros, &mut disabled, 0)?);
    Ok(())
}

fn expand(toks: &[Token], macros: &HashMap<String, Macro>, disabled: &mut Vec<String>, depth: u32) -> Result<Vec<Token>, String> {
    if depth > 64 {
        return Err("macro expansion too deep".into());
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        let tk = &toks[i];
        let Tok::Ident(name) = &tk.t else {
            out.push(tk.clone());
            i += 1;
            continue;
        };
        if name == "__LINE__" {
            out.push(Token { t: Tok::Int(tk.line as u64, false), line: tk.line });
            i += 1;
            continue;
        }
        let Some(m) = macros.get(name).filter(|_| !disabled.contains(name)) else {
            out.push(tk.clone());
            i += 1;
            continue;
        };
        match &m.params {
            None => {
                disabled.push(name.clone());
                let body: Vec<Token> = m.body.iter().map(|b| Token { t: b.t.clone(), line: tk.line }).collect();
                let e = expand(&body, macros, disabled, depth + 1)?;
                disabled.pop();
                out.extend(e);
                i += 1;
            }
            Some(params) => {
                if toks.get(i + 1).map(|t| &t.t) != Some(&Tok::Punct("(")) {
                    out.push(tk.clone());
                    i += 1;
                    continue;
                }
                let mut j = i + 2;
                let mut lvl = 0;
                let mut args: Vec<Vec<Token>> = vec![Vec::new()];
                loop {
                    let Some(t) = toks.get(j) else { return Err(format!("{}: unterminated macro call", tk.line)) };
                    match &t.t {
                        Tok::Punct("(") => {
                            lvl += 1;
                            args.last_mut().unwrap().push(t.clone());
                        }
                        Tok::Punct(")") if lvl == 0 => break,
                        Tok::Punct(")") => {
                            lvl -= 1;
                            args.last_mut().unwrap().push(t.clone());
                        }
                        Tok::Punct(",") if lvl == 0 => args.push(Vec::new()),
                        _ => args.last_mut().unwrap().push(t.clone()),
                    }
                    j += 1;
                }
                if params.is_empty() && args.len() == 1 && args[0].is_empty() {
                    args.clear();
                }
                if args.len() != params.len() {
                    return Err(format!("{}: macro '{name}' takes {} arguments", tk.line, params.len()));
                }
                let mut ex_args = Vec::new();
                for a in &args {
                    ex_args.push(expand(a, macros, disabled, depth + 1)?);
                }
                let mut body = Vec::new();
                let mut k = 0;
                while k < m.body.len() {
                    let b = &m.body[k];
                    // Token pasting: a ## b
                    if m.body.get(k + 1).map(|t| &t.t) == Some(&Tok::Punct("##")) && k + 2 < m.body.len() {
                        let left = subst_raw(b, params, &args);
                        let right = subst_raw(&m.body[k + 2], params, &args);
                        let joined = format!("{}{}", tok_text(&left), tok_text(&right));
                        let mut tt = Vec::new();
                        tokenize_line(&joined, tk.line, &mut tt)?;
                        body.extend(tt);
                        k += 3;
                        continue;
                    }
                    if let Tok::Ident(p) = &b.t {
                        if let Some(pi) = params.iter().position(|x| x == p) {
                            body.extend(ex_args[pi].iter().cloned());
                            k += 1;
                            continue;
                        }
                    }
                    body.push(Token { t: b.t.clone(), line: tk.line });
                    k += 1;
                }
                disabled.push(name.clone());
                let e = expand(&body, macros, disabled, depth + 1)?;
                disabled.pop();
                out.extend(e);
                i = j + 1;
            }
        }
    }
    Ok(out)
}

fn subst_raw(t: &Token, params: &[String], args: &[Vec<Token>]) -> Vec<Token> {
    if let Tok::Ident(p) = &t.t {
        if let Some(pi) = params.iter().position(|x| x == p) {
            return args[pi].clone();
        }
    }
    vec![t.clone()]
}

fn tok_text(ts: &[Token]) -> String {
    ts.iter()
        .map(|t| match &t.t {
            Tok::Ident(s) => s.clone(),
            Tok::Int(v, u) => format!("{v}{}", if *u { "u" } else { "" }),
            Tok::Float(f) => format!("{f:?}"),
            Tok::Punct(p) => p.to_string(),
        })
        .collect()
}

/// Evaluate a `#if` expression.
fn eval_if(rest: &str, macros: &HashMap<String, Macro>, line: u32) -> Result<i64, String> {
    let mut toks = Vec::new();
    tokenize_line(rest, line, &mut toks)?;
    // Resolve defined(X) / defined X before expansion.
    let mut pre = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].t == Tok::Ident("defined".into()) {
            let (name, skip) = match (toks.get(i + 1).map(|t| &t.t), toks.get(i + 2).map(|t| &t.t)) {
                (Some(Tok::Punct("(")), Some(Tok::Ident(n))) => (n.clone(), 4),
                (Some(Tok::Ident(n)), _) => (n.clone(), 2),
                _ => return Err(format!("{line}: bad 'defined'")),
            };
            pre.push(Token { t: Tok::Int(macros.contains_key(&name) as u64, false), line });
            i += skip;
            continue;
        }
        pre.push(toks[i].clone());
        i += 1;
    }
    let mut disabled = Vec::new();
    let ex = expand(&pre, macros, &mut disabled, 0)?;
    let mut p = 0;
    let v = pp_expr(&ex, &mut p, 0, line)?;
    if p != ex.len() {
        return Err(format!("{line}: bad #if expression"));
    }
    Ok(v)
}

fn pp_prec(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "|" => 3,
        "^" => 4,
        "&" => 5,
        "==" | "!=" => 6,
        "<" | ">" | "<=" | ">=" => 7,
        "<<" | ">>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        _ => return None,
    })
}

fn pp_expr(t: &[Token], p: &mut usize, min: u8, line: u32) -> Result<i64, String> {
    let mut lhs = pp_unary(t, p, line)?;
    while let Some(Tok::Punct(op)) = t.get(*p).map(|x| &x.t) {
        let Some(pr) = pp_prec(op) else { break };
        if pr < min.max(1) {
            break;
        }
        *p += 1;
        let rhs = pp_expr(t, p, pr + 1, line)?;
        lhs = match *op {
            "||" => ((lhs != 0) || (rhs != 0)) as i64,
            "&&" => ((lhs != 0) && (rhs != 0)) as i64,
            "|" => lhs | rhs,
            "^" => lhs ^ rhs,
            "&" => lhs & rhs,
            "==" => (lhs == rhs) as i64,
            "!=" => (lhs != rhs) as i64,
            "<" => (lhs < rhs) as i64,
            ">" => (lhs > rhs) as i64,
            "<=" => (lhs <= rhs) as i64,
            ">=" => (lhs >= rhs) as i64,
            "<<" => lhs.wrapping_shl(rhs as u32),
            ">>" => lhs.wrapping_shr(rhs as u32),
            "+" => lhs.wrapping_add(rhs),
            "-" => lhs.wrapping_sub(rhs),
            "*" => lhs.wrapping_mul(rhs),
            "/" | "%" if rhs == 0 => return Err(format!("{line}: division by zero in #if")),
            "/" => lhs / rhs,
            _ => lhs % rhs,
        };
    }
    Ok(lhs)
}

fn pp_unary(t: &[Token], p: &mut usize, line: u32) -> Result<i64, String> {
    let Some(tk) = t.get(*p) else { return Err(format!("{line}: incomplete #if expression")) };
    *p += 1;
    match &tk.t {
        Tok::Int(v, _) => Ok(*v as i64),
        Tok::Ident(_) => Ok(0),
        Tok::Punct("-") => Ok(-pp_unary(t, p, line)?),
        Tok::Punct("+") => pp_unary(t, p, line),
        Tok::Punct("!") => Ok((pp_unary(t, p, line)? == 0) as i64),
        Tok::Punct("~") => Ok(!pp_unary(t, p, line)?),
        Tok::Punct("(") => {
            let v = pp_expr(t, p, 0, line)?;
            if t.get(*p).map(|x| &x.t) != Some(&Tok::Punct(")")) {
                return Err(format!("{line}: missing ')' in #if"));
            }
            *p += 1;
            Ok(v)
        }
        _ => Err(format!("{line}: bad #if expression")),
    }
}

// ---------------------------------------------------------------- types

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Base {
    F,
    I,
    U,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dim {
    D2,
    D3,
    Cube,
    D2Array,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Samp {
    pub dim: Dim,
    pub shadow: bool,
    pub base: Base,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Void,
    Scalar(Base),
    Vec(Base, u8),
    /// Columns, rows.
    Mat(u8, u8),
    Sampler(Samp),
    Struct(Rc<StructTy>),
    Array(Box<Ty>, u32),
}

#[derive(Debug, PartialEq)]
pub struct StructTy {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
}

impl Ty {
    pub const FLOAT: Ty = Ty::Scalar(Base::F);
    pub const INT: Ty = Ty::Scalar(Base::I);
    pub const BOOL: Ty = Ty::Scalar(Base::B);
    pub fn vec(b: Base, n: u8) -> Ty {
        if n == 1 { Ty::Scalar(b) } else { Ty::Vec(b, n) }
    }
    /// Slots (32-bit components) the type takes.
    pub fn size(&self) -> u32 {
        match self {
            Ty::Void => 0,
            Ty::Scalar(_) | Ty::Sampler(_) => 1,
            Ty::Vec(_, n) => *n as u32,
            Ty::Mat(c, r) => (*c as u32) * (*r as u32),
            Ty::Struct(s) => s.fields.iter().map(|f| f.1.size()).sum(),
            Ty::Array(t, n) => t.size() * n,
        }
    }
    pub fn base(&self) -> Option<Base> {
        match self {
            Ty::Scalar(b) | Ty::Vec(b, _) => Some(*b),
            Ty::Mat(..) => Some(Base::F),
            _ => None,
        }
    }
    /// Components of a scalar or vector (or all of a matrix).
    pub fn comps(&self) -> u32 {
        match self {
            Ty::Scalar(_) => 1,
            Ty::Vec(_, n) => *n as u32,
            Ty::Mat(c, r) => (*c * *r) as u32,
            _ => 0,
        }
    }
    pub fn is_scalar(&self) -> bool {
        matches!(self, Ty::Scalar(_))
    }
    pub fn has_sampler(&self) -> bool {
        match self {
            Ty::Sampler(_) => true,
            Ty::Array(t, _) => t.has_sampler(),
            Ty::Struct(s) => s.fields.iter().any(|f| f.1.has_sampler()),
            _ => false,
        }
    }
    pub fn name(&self) -> String {
        match self {
            Ty::Void => "void".into(),
            Ty::Scalar(b) => match b {
                Base::F => "float",
                Base::I => "int",
                Base::U => "uint",
                Base::B => "bool",
            }
            .into(),
            Ty::Vec(b, n) => format!(
                "{}vec{n}",
                match b {
                    Base::F => "",
                    Base::I => "i",
                    Base::U => "u",
                    Base::B => "b",
                }
            ),
            Ty::Mat(c, r) if c == r => format!("mat{c}"),
            Ty::Mat(c, r) => format!("mat{c}x{r}"),
            Ty::Sampler(s) => format!(
                "{}sampler{}{}",
                match s.base {
                    Base::I => "i",
                    Base::U => "u",
                    _ => "",
                },
                match s.dim {
                    Dim::D2 => "2D",
                    Dim::D3 => "3D",
                    Dim::Cube => "Cube",
                    Dim::D2Array => "2DArray",
                },
                if s.shadow { "Shadow" } else { "" }
            ),
            Ty::Struct(s) => s.name.clone(),
            Ty::Array(t, n) => format!("{}[{n}]", t.name()),
        }
    }
}

/// A built-in type name.
pub fn builtin_type(name: &str) -> Option<Ty> {
    use Base::*;
    let samp = |dim, shadow, base| Some(Ty::Sampler(Samp { dim, shadow, base }));
    Some(match name {
        "void" => Ty::Void,
        "float" => Ty::Scalar(F),
        "int" => Ty::Scalar(I),
        "uint" => Ty::Scalar(U),
        "bool" => Ty::Scalar(B),
        "vec2" => Ty::Vec(F, 2),
        "vec3" => Ty::Vec(F, 3),
        "vec4" => Ty::Vec(F, 4),
        "ivec2" => Ty::Vec(I, 2),
        "ivec3" => Ty::Vec(I, 3),
        "ivec4" => Ty::Vec(I, 4),
        "uvec2" => Ty::Vec(U, 2),
        "uvec3" => Ty::Vec(U, 3),
        "uvec4" => Ty::Vec(U, 4),
        "bvec2" => Ty::Vec(B, 2),
        "bvec3" => Ty::Vec(B, 3),
        "bvec4" => Ty::Vec(B, 4),
        "mat2" | "mat2x2" => Ty::Mat(2, 2),
        "mat3" | "mat3x3" => Ty::Mat(3, 3),
        "mat4" | "mat4x4" => Ty::Mat(4, 4),
        "mat2x3" => Ty::Mat(2, 3),
        "mat2x4" => Ty::Mat(2, 4),
        "mat3x2" => Ty::Mat(3, 2),
        "mat3x4" => Ty::Mat(3, 4),
        "mat4x2" => Ty::Mat(4, 2),
        "mat4x3" => Ty::Mat(4, 3),
        "sampler2D" | "samplerExternalOES" => return samp(Dim::D2, false, F),
        "samplerCube" => return samp(Dim::Cube, false, F),
        "sampler3D" => return samp(Dim::D3, false, F),
        "sampler2DArray" => return samp(Dim::D2Array, false, F),
        "sampler2DShadow" => return samp(Dim::D2, true, F),
        "samplerCubeShadow" => return samp(Dim::Cube, true, F),
        "sampler2DArrayShadow" => return samp(Dim::D2Array, true, F),
        "isampler2D" => return samp(Dim::D2, false, I),
        "isampler3D" => return samp(Dim::D3, false, I),
        "isamplerCube" => return samp(Dim::Cube, false, I),
        "isampler2DArray" => return samp(Dim::D2Array, false, I),
        "usampler2D" => return samp(Dim::D2, false, U),
        "usampler3D" => return samp(Dim::D3, false, U),
        "usamplerCube" => return samp(Dim::Cube, false, U),
        "usampler2DArray" => return samp(Dim::D2Array, false, U),
        _ => return None,
    })
}

// ---------------------------------------------------------------- syntax tree

#[derive(Clone, Debug)]
pub enum TyAst {
    Builtin(Ty),
    Named(String),
    Def(Rc<StructAst>),
}

#[derive(Clone, Debug)]
pub struct StructAst {
    pub name: Option<String>,
    pub fields: Vec<(TypeSpec, String, Vec<Option<Expr>>)>,
}

/// A type as written: the type name and any array dimensions after it (`float[3]`).
/// `None` in a dimension means unsized (`float[]`).
#[derive(Clone, Debug)]
pub struct TypeSpec {
    pub ty: TyAst,
    pub dims: Vec<Option<Expr>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    None,
    Const,
    Attribute,
    Varying,
    Uniform,
    In,
    Out,
    InOut,
}

#[derive(Clone, Debug)]
pub struct Quals {
    pub storage: Storage,
    pub flat: bool,
    pub location: Option<i64>,
    pub invariant: bool,
}

#[derive(Clone, Debug)]
pub struct Declarator {
    pub name: String,
    pub dims: Vec<Option<Expr>>,
    pub init: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct Decl {
    pub quals: Quals,
    pub ty: TypeSpec,
    pub vars: Vec<Declarator>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub storage: Storage,
    pub name: String,
    pub fields: Vec<(TypeSpec, String, Vec<Option<Expr>>)>,
    pub instance: Option<(String, Vec<Option<Expr>>)>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub storage: Storage,
    pub ty: TypeSpec,
    pub name: Option<String>,
    pub dims: Vec<Option<Expr>>,
}

#[derive(Clone, Debug)]
pub struct FuncDef {
    pub ret: TypeSpec,
    pub name: String,
    pub params: Vec<Param>,
    pub body: Option<Vec<Stmt>>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub enum Global {
    Decl(Decl),
    Block(Block),
    Func(Rc<FuncDef>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Xor,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
}

#[derive(Clone, Debug)]
pub enum Callee {
    Name(String),
    Type(TypeSpec),
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int(i64),
    UInt(u64),
    Float(f64),
    Bool(bool),
    Var(String),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Un(UnOp, Box<Expr>),
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    /// ++/--: prefix?, +1 or -1.
    Step(bool, i8, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Callee, Vec<Expr>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Length(Box<Expr>),
    Comma(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Decl(Decl),
    Expr(Expr, u32),
    Block(Vec<Stmt>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>, u32),
    For(Option<Box<Stmt>>, Option<Box<Stmt>>, Option<Expr>, Box<Stmt>, u32),
    While(Box<Stmt>, Box<Stmt>, u32),
    Do(Box<Stmt>, Expr, u32),
    Switch(Expr, Vec<Stmt>, u32),
    Case(Option<Expr>, u32),
    Break(u32),
    Continue(u32),
    Return(Option<Expr>, u32),
    Discard(u32),
    Empty,
}

pub struct Unit {
    pub version: u16,
    pub extensions: Vec<String>,
    pub globals: Vec<Global>,
}

// ---------------------------------------------------------------- parser

const RESERVED: &[&str] = &[
    "asm", "class", "union", "enum", "typedef", "template", "this", "packed", "goto", "inline", "noinline", "volatile", "public", "static",
    "extern", "external", "interface", "long", "short", "double", "half", "fixed", "unsigned", "superp", "input", "output", "hvec2", "hvec3",
    "hvec4", "dvec2", "dvec3", "dvec4", "fvec2", "fvec3", "fvec4", "sampler1D", "sampler1DShadow", "sampler2DRect", "sampler3DRect",
    "sampler2DRectShadow", "sizeof", "cast", "namespace", "using",
];

struct Parser {
    t: Vec<Token>,
    p: usize,
    structs: Vec<std::collections::HashSet<String>>,
    version: u16,
}

pub fn parse(src: &str, supported: &[&str]) -> Result<Unit, String> {
    let pp = preprocess(src, supported)?;
    let mut p = Parser { t: pp.tokens, p: 0, structs: vec![Default::default()], version: pp.version };
    let mut globals = Vec::new();
    while p.p < p.t.len() {
        p.global(&mut globals)?;
    }
    Ok(Unit { version: pp.version, extensions: pp.extensions, globals })
}

type R<T> = Result<T, String>;

impl Parser {
    fn line(&self) -> u32 {
        self.t.get(self.p).or(self.t.last()).map_or(0, |t| t.line)
    }
    fn err<T>(&self, msg: impl std::fmt::Display) -> R<T> {
        Err(format!("{}: {msg}", self.line()))
    }
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.p).map(|t| &t.t)
    }
    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.t.get(self.p + k).map(|t| &t.t)
    }
    fn is(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(p)) if *p == s)
    }
    fn is_kw(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(p)) if p == s)
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) || self.is_kw(s) {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> R<()> {
        if self.eat(s) {
            Ok(())
        } else {
            let got = match self.peek() {
                Some(t) => tok_text(&[Token { t: t.clone(), line: 0 }]),
                None => "end of shader".into(),
            };
            self.err(format!("expected '{s}' but found '{got}'"))
        }
    }
    fn ident(&mut self) -> R<String> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.clone();
                if is_keyword(&s) {
                    return self.err(format!("'{s}' is a reserved word"));
                }
                if s.starts_with("gl_") {
                    return self.err(format!("'{s}': names starting with gl_ are reserved"));
                }
                if s.contains("__") && self.version == 100 {
                    // Allowed by the spec in 3.00, reserved in 1.00 for the implementation.
                }
                self.p += 1;
                Ok(s)
            }
            _ => self.err("expected a name"),
        }
    }
    fn is_struct_name(&self, s: &str) -> bool {
        self.structs.iter().rev().any(|m| m.contains(s))
    }
    /// Does a type start here?
    fn at_type(&self) -> bool {
        match self.peek() {
            Some(Tok::Ident(s)) => builtin_type(s).is_some() || s == "struct" || self.is_struct_name(s),
            _ => false,
        }
    }
    fn at_qualifier(&self) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if matches!(s.as_str(),
            "const" | "attribute" | "varying" | "uniform" | "in" | "out" | "inout" | "invariant" | "centroid" | "flat" | "smooth"
            | "highp" | "mediump" | "lowp" | "layout"))
    }

    fn qualifiers(&mut self) -> R<Quals> {
        let mut q = Quals { storage: Storage::None, flat: false, location: None, invariant: false };
        loop {
            let Some(Tok::Ident(s)) = self.peek().cloned() else { break };
            let st = match s.as_str() {
                "const" => Storage::Const,
                "attribute" => Storage::Attribute,
                "varying" => Storage::Varying,
                "uniform" => Storage::Uniform,
                "in" => Storage::In,
                "out" => Storage::Out,
                "inout" => Storage::InOut,
                "invariant" => {
                    q.invariant = true;
                    self.p += 1;
                    continue;
                }
                "flat" => {
                    q.flat = true;
                    self.p += 1;
                    continue;
                }
                "centroid" | "smooth" | "highp" | "mediump" | "lowp" => {
                    self.p += 1;
                    continue;
                }
                "layout" => {
                    self.p += 1;
                    self.expect("(")?;
                    loop {
                        let k = self.ident_any()?;
                        if self.eat("=") {
                            let v = match self.peek() {
                                Some(Tok::Int(v, _)) => *v as i64,
                                _ => return self.err("expected a number in layout()"),
                            };
                            self.p += 1;
                            if k == "location" {
                                q.location = Some(v);
                            }
                        }
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    continue;
                }
                _ => break,
            };
            if q.storage == Storage::Const && st == Storage::In {
                // "const in" on a function parameter.
                self.p += 1;
                continue;
            }
            if q.storage != Storage::None {
                // "centroid in" etc. handled above; two storage qualifiers is an error.
                return self.err("too many storage qualifiers");
            }
            q.storage = st;
            self.p += 1;
        }
        Ok(q)
    }
    fn ident_any(&mut self) -> R<String> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.clone();
                self.p += 1;
                Ok(s)
            }
            _ => self.err("expected a name"),
        }
    }

    fn dims(&mut self) -> R<Vec<Option<Expr>>> {
        let mut d = Vec::new();
        while self.eat("[") {
            if self.eat("]") {
                d.push(None);
            } else {
                d.push(Some(self.expr_assign()?));
                self.expect("]")?;
            }
        }
        Ok(d)
    }

    fn type_spec(&mut self) -> R<TypeSpec> {
        // Precision qualifiers may sit right before the type.
        while self.is_kw("highp") || self.is_kw("mediump") || self.is_kw("lowp") {
            self.p += 1;
        }
        let name = self.ident_any()?;
        let ty = if name == "struct" {
            let sname = if let Some(Tok::Ident(_)) = self.peek() { Some(self.ident()?) } else { None };
            self.expect("{")?;
            let mut fields = Vec::new();
            while !self.eat("}") {
                let _ = self.qualifiers()?;
                let fty = self.type_spec()?;
                loop {
                    let fname = self.ident()?;
                    let d = self.dims()?;
                    fields.push((fty.clone(), fname, d));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(";")?;
            }
            if fields.is_empty() {
                return self.err("a struct needs at least one field");
            }
            if let Some(n) = &sname {
                self.structs.last_mut().unwrap().insert(n.clone());
            }
            TyAst::Def(Rc::new(StructAst { name: sname, fields }))
        } else if let Some(t) = builtin_type(&name) {
            TyAst::Builtin(t)
        } else if self.is_struct_name(&name) {
            TyAst::Named(name)
        } else {
            self.p -= 1;
            return self.err(format!("'{name}' is not a type"));
        };
        let dims = self.dims()?;
        Ok(TypeSpec { ty, dims })
    }

    fn global(&mut self, out: &mut Vec<Global>) -> R<()> {
        let line = self.line();
        if self.eat(";") {
            return Ok(());
        }
        if self.is_kw("precision") {
            self.p += 1;
            self.ident_any()?;
            self.type_spec()?;
            return self.expect(";");
        }
        let quals = self.qualifiers()?;
        // invariant gl_Position; / invariant varName;
        if quals.invariant && quals.storage == Storage::None && !self.at_type() {
            while !self.eat(";") {
                self.p += 1;
                if self.p >= self.t.len() {
                    return self.err("unterminated invariant declaration");
                }
            }
            return Ok(());
        }
        // Interface blocks: uniform Name { ... } inst;
        if quals.storage != Storage::None && matches!(self.peek(), Some(Tok::Ident(s)) if !self.at_type() && builtin_type(s).is_none()) && matches!(self.peek_at(1), Some(Tok::Punct("{"))) {
            let name = self.ident()?;
            self.expect("{")?;
            let mut fields = Vec::new();
            while !self.eat("}") {
                let _ = self.qualifiers()?;
                let fty = self.type_spec()?;
                loop {
                    let fname = self.ident()?;
                    let d = self.dims()?;
                    fields.push((fty.clone(), fname, d));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(";")?;
            }
            let instance = if let Some(Tok::Ident(_)) = self.peek() {
                let n = self.ident()?;
                let d = self.dims()?;
                Some((n, d))
            } else {
                None
            };
            self.expect(";")?;
            out.push(Global::Block(Block { storage: quals.storage, name, fields, instance, line }));
            return Ok(());
        }
        let ty = self.type_spec()?;
        if self.eat(";") {
            out.push(Global::Decl(Decl { quals, ty, vars: Vec::new(), line }));
            return Ok(());
        }
        let name = self.ident()?;
        if self.is("(") {
            let f = self.function(ty, name, line)?;
            out.push(Global::Func(Rc::new(f)));
            return Ok(());
        }
        let d = self.declarators(quals, ty, name, line)?;
        out.push(Global::Decl(d));
        Ok(())
    }

    fn function(&mut self, ret: TypeSpec, name: String, line: u32) -> R<FuncDef> {
        self.expect("(")?;
        let mut params = Vec::new();
        if self.is_kw("void") && matches!(self.peek_at(1), Some(Tok::Punct(")"))) {
            self.p += 1;
        }
        if !self.is(")") {
            loop {
                let q = self.qualifiers()?;
                let ty = self.type_spec()?;
                let (pname, dims) = if let Some(Tok::Ident(_)) = self.peek() {
                    let n = self.ident()?;
                    (Some(n), self.dims()?)
                } else {
                    (None, Vec::new())
                };
                let storage = match q.storage {
                    Storage::Out => Storage::Out,
                    Storage::InOut => Storage::InOut,
                    Storage::Const => Storage::Const,
                    _ => Storage::In,
                };
                params.push(Param { storage, ty, name: pname, dims });
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(")")?;
        if self.eat(";") {
            return Ok(FuncDef { ret, name, params, body: None, line });
        }
        self.expect("{")?;
        self.structs.push(Default::default());
        let mut body = Vec::new();
        while !self.eat("}") {
            if self.p >= self.t.len() {
                return self.err("missing '}' at end of function");
            }
            body.push(self.stmt()?);
        }
        self.structs.pop();
        Ok(FuncDef { ret, name, params, body: Some(body), line })
    }

    fn declarators(&mut self, quals: Quals, ty: TypeSpec, first: String, line: u32) -> R<Decl> {
        let mut vars = Vec::new();
        let mut name = first;
        loop {
            let dims = self.dims()?;
            let init = if self.eat("=") { Some(self.expr_assign()?) } else { None };
            vars.push(Declarator { name, dims, init });
            if !self.eat(",") {
                break;
            }
            name = self.ident()?;
        }
        self.expect(";")?;
        Ok(Decl { quals, ty, vars, line })
    }

    /// Is a declaration (rather than an expression) at the start of this statement?
    fn at_decl(&self) -> bool {
        if self.at_qualifier() {
            return true;
        }
        if !self.at_type() {
            return false;
        }
        if self.is_kw("struct") {
            return true;
        }
        // type name ... | type[N] name
        match self.peek_at(1) {
            Some(Tok::Ident(_)) => true,
            Some(Tok::Punct("[")) => {
                // float[3](…) is a constructor; float[3] a is a declaration.
                let mut k = 1;
                let mut lvl = 0;
                while let Some(t) = self.peek_at(k) {
                    match t {
                        Tok::Punct("[") => lvl += 1,
                        Tok::Punct("]") => {
                            lvl -= 1;
                            if lvl == 0 && !matches!(self.peek_at(k + 1), Some(Tok::Punct("["))) {
                                return matches!(self.peek_at(k + 1), Some(Tok::Ident(_)));
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                false
            }
            _ => false,
        }
    }

    fn local_decl(&mut self) -> R<Decl> {
        let line = self.line();
        let quals = self.qualifiers()?;
        let ty = self.type_spec()?;
        if self.eat(";") {
            return Ok(Decl { quals, ty, vars: Vec::new(), line });
        }
        let name = self.ident()?;
        self.declarators(quals, ty, name, line)
    }

    fn stmt(&mut self) -> R<Stmt> {
        let line = self.line();
        let Some(tok) = self.peek().cloned() else { return self.err("unexpected end of shader") };
        if let Tok::Ident(k) = &tok {
            match k.as_str() {
                "if" => {
                    self.p += 1;
                    self.expect("(")?;
                    let c = self.expr()?;
                    self.expect(")")?;
                    let a = self.stmt_scoped()?;
                    let b = if self.eat("else") { Some(Box::new(self.stmt_scoped()?)) } else { None };
                    return Ok(Stmt::If(c, Box::new(a), b, line));
                }
                "for" => {
                    self.p += 1;
                    self.expect("(")?;
                    self.structs.push(Default::default());
                    let init = if self.eat(";") {
                        None
                    } else if self.at_decl() {
                        Some(Box::new(Stmt::Decl(self.local_decl()?)))
                    } else {
                        let e = self.expr()?;
                        self.expect(";")?;
                        Some(Box::new(Stmt::Expr(e, line)))
                    };
                    let cond = if self.is(";") { None } else { Some(Box::new(self.condition()?)) };
                    self.expect(";")?;
                    let step = if self.is(")") { None } else { Some(self.expr()?) };
                    self.expect(")")?;
                    let body = self.stmt_noscope()?;
                    self.structs.pop();
                    return Ok(Stmt::For(init, cond, step, Box::new(body), line));
                }
                "while" => {
                    self.p += 1;
                    self.expect("(")?;
                    self.structs.push(Default::default());
                    let c = self.condition()?;
                    self.expect(")")?;
                    let body = self.stmt_noscope()?;
                    self.structs.pop();
                    return Ok(Stmt::While(Box::new(c), Box::new(body), line));
                }
                "do" => {
                    self.p += 1;
                    let body = self.stmt_scoped()?;
                    self.expect("while")?;
                    self.expect("(")?;
                    let c = self.expr()?;
                    self.expect(")")?;
                    self.expect(";")?;
                    return Ok(Stmt::Do(Box::new(body), c, line));
                }
                "switch" => {
                    self.p += 1;
                    self.expect("(")?;
                    let c = self.expr()?;
                    self.expect(")")?;
                    self.expect("{")?;
                    self.structs.push(Default::default());
                    let mut body = Vec::new();
                    while !self.eat("}") {
                        if self.p >= self.t.len() {
                            return self.err("missing '}'");
                        }
                        body.push(self.stmt()?);
                    }
                    self.structs.pop();
                    return Ok(Stmt::Switch(c, body, line));
                }
                "case" => {
                    self.p += 1;
                    let e = self.expr()?;
                    self.expect(":")?;
                    return Ok(Stmt::Case(Some(e), line));
                }
                "default" => {
                    self.p += 1;
                    self.expect(":")?;
                    return Ok(Stmt::Case(None, line));
                }
                "break" => {
                    self.p += 1;
                    self.expect(";")?;
                    return Ok(Stmt::Break(line));
                }
                "continue" => {
                    self.p += 1;
                    self.expect(";")?;
                    return Ok(Stmt::Continue(line));
                }
                "discard" => {
                    self.p += 1;
                    self.expect(";")?;
                    return Ok(Stmt::Discard(line));
                }
                "return" => {
                    self.p += 1;
                    if self.eat(";") {
                        return Ok(Stmt::Return(None, line));
                    }
                    let e = self.expr()?;
                    self.expect(";")?;
                    return Ok(Stmt::Return(Some(e), line));
                }
                "precision" => {
                    self.p += 1;
                    self.ident_any()?;
                    self.type_spec()?;
                    self.expect(";")?;
                    return Ok(Stmt::Empty);
                }
                _ => {}
            }
        }
        if self.eat("{") {
            self.structs.push(Default::default());
            let mut body = Vec::new();
            while !self.eat("}") {
                if self.p >= self.t.len() {
                    return self.err("missing '}'");
                }
                body.push(self.stmt()?);
            }
            self.structs.pop();
            return Ok(Stmt::Block(body));
        }
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.at_decl() {
            return Ok(Stmt::Decl(self.local_decl()?));
        }
        let e = self.expr()?;
        self.expect(";")?;
        Ok(Stmt::Expr(e, line))
    }

    /// A sub-statement that gets its own scope (if/else branches).
    fn stmt_scoped(&mut self) -> R<Stmt> {
        self.structs.push(Default::default());
        let s = self.stmt()?;
        self.structs.pop();
        Ok(match s {
            Stmt::Decl(_) => Stmt::Block(vec![s]),
            s => s,
        })
    }
    /// A loop body: a compound body shares the loop's scope.
    fn stmt_noscope(&mut self) -> R<Stmt> {
        if self.eat("{") {
            let mut body = Vec::new();
            while !self.eat("}") {
                if self.p >= self.t.len() {
                    return self.err("missing '}'");
                }
                body.push(self.stmt()?);
            }
            return Ok(Stmt::Block(body));
        }
        self.stmt()
    }

    /// A loop condition: an expression, or `type name = init`.
    fn condition(&mut self) -> R<Stmt> {
        let line = self.line();
        if self.at_decl() {
            let quals = self.qualifiers()?;
            let ty = self.type_spec()?;
            let name = self.ident()?;
            self.expect("=")?;
            let init = self.expr_assign()?;
            return Ok(Stmt::Decl(Decl { quals, ty, vars: vec![Declarator { name, dims: Vec::new(), init: Some(init) }], line }));
        }
        Ok(Stmt::Expr(self.expr()?, line))
    }

    // ---- expressions

    pub fn expr(&mut self) -> R<Expr> {
        let mut e = self.expr_assign()?;
        while self.eat(",") {
            let r = self.expr_assign()?;
            e = Expr::Comma(Box::new(e), Box::new(r));
        }
        Ok(e)
    }

    fn expr_assign(&mut self) -> R<Expr> {
        let lhs = self.expr_cond()?;
        let op = match self.peek() {
            Some(Tok::Punct(p)) => match *p {
                "=" => Some(None),
                "+=" => Some(Some(BinOp::Add)),
                "-=" => Some(Some(BinOp::Sub)),
                "*=" => Some(Some(BinOp::Mul)),
                "/=" => Some(Some(BinOp::Div)),
                "%=" => Some(Some(BinOp::Mod)),
                "<<=" => Some(Some(BinOp::Shl)),
                ">>=" => Some(Some(BinOp::Shr)),
                "&=" => Some(Some(BinOp::BitAnd)),
                "|=" => Some(Some(BinOp::BitOr)),
                "^=" => Some(Some(BinOp::BitXor)),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            self.p += 1;
            let rhs = self.expr_assign()?;
            return Ok(Expr::Assign(op, Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn expr_cond(&mut self) -> R<Expr> {
        let c = self.expr_bin(0)?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr_assign()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn expr_bin(&mut self, min: u8) -> R<Expr> {
        let mut lhs = self.expr_unary()?;
        loop {
            let Some(Tok::Punct(p)) = self.peek() else { break };
            let (prec, op) = match *p {
                "||" => (1, BinOp::Or),
                "^^" => (2, BinOp::Xor),
                "&&" => (3, BinOp::And),
                "|" => (4, BinOp::BitOr),
                "^" => (5, BinOp::BitXor),
                "&" => (6, BinOp::BitAnd),
                "==" => (7, BinOp::Eq),
                "!=" => (7, BinOp::Ne),
                "<" => (8, BinOp::Lt),
                ">" => (8, BinOp::Gt),
                "<=" => (8, BinOp::Le),
                ">=" => (8, BinOp::Ge),
                "<<" => (9, BinOp::Shl),
                ">>" => (9, BinOp::Shr),
                "+" => (10, BinOp::Add),
                "-" => (10, BinOp::Sub),
                "*" => (11, BinOp::Mul),
                "/" => (11, BinOp::Div),
                "%" => (11, BinOp::Mod),
                _ => break,
            };
            if prec <= min {
                break;
            }
            self.p += 1;
            let rhs = self.expr_bin(prec)?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn expr_unary(&mut self) -> R<Expr> {
        if let Some(Tok::Punct(p)) = self.peek() {
            let p = *p;
            let op = match p {
                "-" => Some(UnOp::Neg),
                "+" => Some(UnOp::Plus),
                "!" => Some(UnOp::Not),
                "~" => Some(UnOp::BitNot),
                _ => None,
            };
            if let Some(op) = op {
                self.p += 1;
                let e = self.expr_unary()?;
                return Ok(Expr::Un(op, Box::new(e)));
            }
            if p == "++" || p == "--" {
                self.p += 1;
                let e = self.expr_unary()?;
                return Ok(Expr::Step(true, if p == "++" { 1 } else { -1 }, Box::new(e)));
            }
        }
        self.expr_postfix()
    }

    fn expr_postfix(&mut self) -> R<Expr> {
        let mut e = self.expr_primary()?;
        loop {
            if self.eat("[") {
                let i = self.expr()?;
                self.expect("]")?;
                e = Expr::Index(Box::new(e), Box::new(i));
            } else if self.eat(".") {
                let f = self.ident_any()?;
                if f == "length" && self.is("(") {
                    self.p += 1;
                    self.expect(")")?;
                    e = Expr::Length(Box::new(e));
                } else {
                    e = Expr::Field(Box::new(e), f);
                }
            } else if self.is("++") || self.is("--") {
                let d = if self.is("++") { 1 } else { -1 };
                self.p += 1;
                e = Expr::Step(false, d, Box::new(e));
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn args(&mut self) -> R<Vec<Expr>> {
        self.expect("(")?;
        let mut a = Vec::new();
        if self.is_kw("void") && matches!(self.peek_at(1), Some(Tok::Punct(")"))) {
            self.p += 1;
        }
        if !self.eat(")") {
            loop {
                a.push(self.expr_assign()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        Ok(a)
    }

    fn expr_primary(&mut self) -> R<Expr> {
        let Some(t) = self.peek().cloned() else { return self.err("unexpected end of shader") };
        match t {
            Tok::Int(v, u) => {
                self.p += 1;
                if u {
                    if v > u32::MAX as u64 {
                        return self.err("integer literal too large");
                    }
                    Ok(Expr::UInt(v))
                } else {
                    if v > u32::MAX as u64 {
                        return self.err("integer literal too large");
                    }
                    // In 1.00/3.00 an int literal is 32-bit; hex values above i32::MAX wrap.
                    Ok(Expr::Int(v as u32 as i32 as i64))
                }
            }
            Tok::Float(f) => {
                self.p += 1;
                Ok(Expr::Float(f))
            }
            Tok::Punct("(") => {
                self.p += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(s) => {
                if s == "true" || s == "false" {
                    self.p += 1;
                    return Ok(Expr::Bool(s == "true"));
                }
                if self.at_type() {
                    let ts = self.type_spec()?;
                    let a = self.args()?;
                    return Ok(Expr::Call(Callee::Type(ts), a));
                }
                if is_keyword(&s) && s != "length" {
                    return self.err(format!("unexpected '{s}'"));
                }
                self.p += 1;
                if self.is("(") {
                    let a = self.args()?;
                    return Ok(Expr::Call(Callee::Name(s), a));
                }
                Ok(Expr::Var(s))
            }
            Tok::Punct(p) => self.err(format!("unexpected '{p}'")),
        }
    }
}

pub fn is_keyword(s: &str) -> bool {
    builtin_type(s).is_some()
        || RESERVED.contains(&s)
        || matches!(
            s,
            "attribute" | "const" | "uniform" | "varying" | "break" | "continue" | "do" | "for" | "while" | "if" | "else" | "in" | "out"
                | "inout" | "true" | "false" | "lowp" | "mediump" | "highp" | "precision" | "invariant" | "discard" | "return" | "struct"
                | "switch" | "case" | "default" | "layout" | "centroid" | "flat" | "smooth"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocessor_macros_and_conditionals() {
        let src = "#define N 3\n#define SQ(x) ((x)*(x))\n#if N > 2 && defined(GL_ES)\nfloat a = SQ(N);\n#else\nint b;\n#endif\n";
        let pp = preprocess(src, &[]).unwrap();
        let text = tok_text(&pp.tokens);
        assert_eq!(text, "floata=((3)*(3));");
    }

    #[test]
    fn parses_a_shader() {
        let src = "precision mediump float;\nuniform vec4 u_color;\nvarying vec2 v_uv;\nstruct L { vec3 dir; float k; };\nuniform L lights[2];\nfloat f(in float x, out float y) { y = x * 2.0; return x + 1.0; }\nvoid main() {\n  float y;\n  for (int i = 0; i < 2; i++) { if (lights[i].k > 0.5) continue; }\n  gl_FragColor = u_color * vec4(v_uv, f(1.0, y), 1.0);\n}\n";
        let u = parse(src, &[]).unwrap();
        assert_eq!(u.globals.len(), 6);
    }

    #[test]
    fn reports_syntax_errors() {
        assert!(parse("void main() { float x = ; }", &[]).is_err());
        assert!(parse("void main() { gl_FragColor = vec4(1.0) }", &[]).is_err());
        assert!(parse("#version 300 es\nout vec4 c; void main() { c = vec4(1.0); }", &[]).is_ok());
        // three.js writes `const in` on parameters.
        assert!(parse("float f(const in float x) { return x; } void main() {}", &[]).is_ok());
    }
}
