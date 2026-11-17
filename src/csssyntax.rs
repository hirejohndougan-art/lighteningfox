//! CSS Syntax Level 3: the tokenizer and the generic parser
//! (https://drafts.csswg.org/css-syntax-3/).
//!
//! The tokenizer turns text into tokens exactly as the standard says (escapes, numbers,
//! `url()`, bad strings and comments included). The parser groups tokens into component
//! values (blocks and functions) and then into rules and declarations, recovering from
//! errors the way browsers do. `css.rs` builds stylesheets from what this produces.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Function(String),
    AtKeyword(String),
    /// value, and whether it's a valid identifier ("id" type)
    Hash(String, bool),
    Str(String),
    BadStr,
    Url(String),
    BadUrl,
    Delim(char),
    /// value, integer?, as written
    Number(f64, bool, String),
    Percentage(f64, bool, String),
    Dimension(f64, bool, String, String),
    Whitespace,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
}

/// A token and where it is in the (preprocessed) input, in chars.
#[derive(Debug, Clone)]
pub struct Tok {
    pub t: Token,
    pub span: Range<usize>,
}

// ---------------------------------------------------------------- tokenizer

/// §3.3: CR, FF and CRLF become LF; NUL and surrogates become U+FFFD.
pub fn preprocess(s: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                out.push('\n');
            }
            '\u{c}' => out.push('\n'),
            '\0' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

pub struct Tokenizer<'a> {
    s: &'a [char],
    i: usize,
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c as u32 >= 0x80
}

fn is_name(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-'
}

fn is_nonprintable(c: char) -> bool {
    matches!(c as u32, 0..=8 | 0xB | 0xE..=0x1F | 0x7F)
}

pub fn tokenize(s: &[char]) -> Vec<Tok> {
    let mut t = Tokenizer { s, i: 0 };
    let mut out = Vec::new();
    while let Some(tok) = t.next_token() {
        out.push(tok);
    }
    out
}

impl Tokenizer<'_> {
    fn at(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn valid_escape_at(&self, k: usize) -> bool {
        self.at(k) == Some('\\') && self.at(k + 1) != Some('\n')
    }

    /// §4.3.9: would the next three chars start an identifier?
    fn starts_ident_at(&self, k: usize) -> bool {
        match self.at(k) {
            Some('-') => self.at(k + 1).is_some_and(|c| is_name_start(c) || c == '-') || self.valid_escape_at(k + 1),
            Some(c) if is_name_start(c) => true,
            Some('\\') => self.valid_escape_at(k),
            _ => false,
        }
    }

    /// §4.3.10: would they start a number?
    fn starts_number_at(&self, k: usize) -> bool {
        match self.at(k) {
            Some('+' | '-') => match self.at(k + 1) {
                Some(c) if c.is_ascii_digit() => true,
                Some('.') => self.at(k + 2).is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            },
            Some('.') => self.at(k + 1).is_some_and(|c| c.is_ascii_digit()),
            Some(c) => c.is_ascii_digit(),
            None => false,
        }
    }

    fn consume_comments(&mut self) {
        while self.at(0) == Some('/') && self.at(1) == Some('*') {
            self.i += 2;
            loop {
                match self.at(0) {
                    None => return,
                    Some('*') if self.at(1) == Some('/') => {
                        self.i += 2;
                        break;
                    }
                    _ => self.i += 1,
                }
            }
        }
    }

    pub fn next_token(&mut self) -> Option<Tok> {
        self.consume_comments();
        let start = self.i;
        let c = self.at(0)?;
        let t = match c {
            c if c.is_whitespace() && matches!(c, ' ' | '\t' | '\n') => {
                while self.at(0).is_some_and(|c| matches!(c, ' ' | '\t' | '\n')) {
                    self.i += 1;
                }
                Token::Whitespace
            }
            '"' | '\'' => {
                self.i += 1;
                self.string(c)
            }
            '#' => {
                if self.at(1).is_some_and(is_name) || self.valid_escape_at(1) {
                    self.i += 1;
                    let id = self.starts_ident_at(0);
                    Token::Hash(self.name(), id)
                } else {
                    self.i += 1;
                    Token::Delim('#')
                }
            }
            '(' => self.one(Token::LParen),
            ')' => self.one(Token::RParen),
            '+' | '.' => {
                if self.starts_number_at(0) {
                    self.numeric()
                } else {
                    self.one(Token::Delim(c))
                }
            }
            ',' => self.one(Token::Comma),
            '-' => {
                if self.starts_number_at(0) {
                    self.numeric()
                } else if self.at(1) == Some('-') && self.at(2) == Some('>') {
                    self.i += 3;
                    Token::Cdc
                } else if self.starts_ident_at(0) {
                    self.ident_like()
                } else {
                    self.one(Token::Delim('-'))
                }
            }
            ':' => self.one(Token::Colon),
            ';' => self.one(Token::Semicolon),
            '<' => {
                if self.at(1) == Some('!') && self.at(2) == Some('-') && self.at(3) == Some('-') {
                    self.i += 4;
                    Token::Cdo
                } else {
                    self.one(Token::Delim('<'))
                }
            }
            '@' => {
                if self.starts_ident_at(1) {
                    self.i += 1;
                    Token::AtKeyword(self.name())
                } else {
                    self.one(Token::Delim('@'))
                }
            }
            '[' => self.one(Token::LBracket),
            '\\' => {
                if self.valid_escape_at(0) {
                    self.ident_like()
                } else {
                    self.one(Token::Delim('\\'))
                }
            }
            ']' => self.one(Token::RBracket),
            '{' => self.one(Token::LBrace),
            '}' => self.one(Token::RBrace),
            c if c.is_ascii_digit() => self.numeric(),
            c if is_name_start(c) => self.ident_like(),
            c => self.one(Token::Delim(c)),
        };
        Some(Tok { t, span: start..self.i })
    }

    fn one(&mut self, t: Token) -> Token {
        self.i += 1;
        t
    }

    /// §4.3.7: after the backslash.
    fn escape(&mut self) -> char {
        let Some(c) = self.at(0) else { return '\u{FFFD}' };
        self.i += 1;
        if c.is_ascii_hexdigit() {
            let mut v = c.to_digit(16).unwrap();
            let mut n = 1;
            while n < 6 && self.at(0).is_some_and(|c| c.is_ascii_hexdigit()) {
                v = v * 16 + self.at(0).unwrap().to_digit(16).unwrap();
                self.i += 1;
                n += 1;
            }
            if self.at(0).is_some_and(|c| matches!(c, ' ' | '\t' | '\n')) {
                self.i += 1;
            }
            if v == 0 || (0xD800..=0xDFFF).contains(&v) || v > 0x10FFFF {
                return '\u{FFFD}';
            }
            char::from_u32(v).unwrap_or('\u{FFFD}')
        } else {
            c
        }
    }

    fn name(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.at(0) {
                Some(c) if is_name(c) => {
                    s.push(c);
                    self.i += 1;
                }
                Some('\\') if self.valid_escape_at(0) => {
                    self.i += 1;
                    s.push(self.escape());
                }
                _ => return s,
            }
        }
    }

    fn string(&mut self, q: char) -> Token {
        let mut s = String::new();
        loop {
            match self.at(0) {
                None => return Token::Str(s),
                Some(c) if c == q => {
                    self.i += 1;
                    return Token::Str(s);
                }
                Some('\n') => return Token::BadStr,
                Some('\\') => match self.at(1) {
                    None => self.i += 1,
                    Some('\n') => self.i += 2,
                    Some(_) => {
                        self.i += 1;
                        s.push(self.escape());
                    }
                },
                Some(c) => {
                    s.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// §4.3.12: the number's text, value and whether it's an integer.
    fn number(&mut self) -> (f64, bool, String) {
        let start = self.i;
        let mut int = true;
        if matches!(self.at(0), Some('+' | '-')) {
            self.i += 1;
        }
        while self.at(0).is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.at(0) == Some('.') && self.at(1).is_some_and(|c| c.is_ascii_digit()) {
            int = false;
            self.i += 1;
            while self.at(0).is_some_and(|c| c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        if matches!(self.at(0), Some('e' | 'E')) {
            let k = if matches!(self.at(1), Some('+' | '-')) { 2 } else { 1 };
            if self.at(k).is_some_and(|c| c.is_ascii_digit()) {
                int = false;
                self.i += k;
                while self.at(0).is_some_and(|c| c.is_ascii_digit()) {
                    self.i += 1;
                }
            }
        }
        let repr: String = self.s[start..self.i].iter().collect();
        let v = repr.parse::<f64>().unwrap_or(0.0);
        (v, int, repr)
    }

    fn numeric(&mut self) -> Token {
        let (v, int, repr) = self.number();
        if self.starts_ident_at(0) {
            let unit = self.name();
            Token::Dimension(v, int, repr, unit)
        } else if self.at(0) == Some('%') {
            self.i += 1;
            Token::Percentage(v, int, repr)
        } else {
            Token::Number(v, int, repr)
        }
    }

    fn ident_like(&mut self) -> Token {
        let name = self.name();
        if name.eq_ignore_ascii_case("url") && self.at(0) == Some('(') {
            self.i += 1;
            // Keep at most one whitespace before a quote: `url( "x")` is a function.
            while self.at(0).is_some_and(|c| matches!(c, ' ' | '\t' | '\n')) && self.at(1).is_some_and(|c| matches!(c, ' ' | '\t' | '\n')) {
                self.i += 1;
            }
            let next = if self.at(0).is_some_and(|c| matches!(c, ' ' | '\t' | '\n')) { self.at(1) } else { self.at(0) };
            if matches!(next, Some('"' | '\'')) {
                return Token::Function(name);
            }
            return self.url();
        }
        if self.at(0) == Some('(') {
            self.i += 1;
            return Token::Function(name);
        }
        Token::Ident(name)
    }

    fn url(&mut self) -> Token {
        let mut s = String::new();
        while self.at(0).is_some_and(|c| matches!(c, ' ' | '\t' | '\n')) {
            self.i += 1;
        }
        loop {
            match self.at(0) {
                None => return Token::Url(s),
                Some(')') => {
                    self.i += 1;
                    return Token::Url(s);
                }
                Some(' ' | '\t' | '\n') => {
                    while self.at(0).is_some_and(|c| matches!(c, ' ' | '\t' | '\n')) {
                        self.i += 1;
                    }
                    match self.at(0) {
                        None => return Token::Url(s),
                        Some(')') => {
                            self.i += 1;
                            return Token::Url(s);
                        }
                        _ => {
                            self.bad_url_rest();
                            return Token::BadUrl;
                        }
                    }
                }
                Some(c) if c == '"' || c == '\'' || c == '(' || is_nonprintable(c) => {
                    self.bad_url_rest();
                    return Token::BadUrl;
                }
                Some('\\') => {
                    if self.valid_escape_at(0) {
                        self.i += 1;
                        s.push(self.escape());
                    } else {
                        self.bad_url_rest();
                        return Token::BadUrl;
                    }
                }
                Some(c) => {
                    s.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn bad_url_rest(&mut self) {
        loop {
            match self.at(0) {
                None => return,
                Some(')') => {
                    self.i += 1;
                    return;
                }
                Some('\\') if self.valid_escape_at(0) => {
                    self.i += 1;
                    self.escape();
                }
                _ => self.i += 1,
            }
        }
    }
}

// ---------------------------------------------------------------- component values

#[derive(Debug, Clone)]
pub enum Cv {
    Tok(Tok),
    /// opening char ('{', '[' or '('), contents, span including the brackets
    Block(char, Vec<Cv>, Range<usize>),
    /// name, arguments, span
    Func(String, Vec<Cv>, Range<usize>),
}

impl Cv {

    pub fn is_ws(&self) -> bool {
        matches!(self, Cv::Tok(Tok { t: Token::Whitespace, .. }))
    }

    pub fn token(&self) -> Option<&Token> {
        match self {
            Cv::Tok(t) => Some(&t.t),
            _ => None,
        }
    }
}

/// A stream of tokens consumed into component values.
pub struct Parser {
    toks: Vec<Tok>,
    i: usize,
}

impl Parser {
    pub fn new(chars: &[char]) -> Parser {
        Parser { toks: tokenize(chars), i: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.i).map(|t| &t.t)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn end(&self) -> usize {
        self.toks.last().map_or(0, |t| t.span.end)
    }

    /// §5.4.8 consume a component value.
    pub fn component_value(&mut self) -> Option<Cv> {
        let t = self.next()?;
        Some(match &t.t {
            Token::LBrace | Token::LBracket | Token::LParen => {
                let (open, close) = match t.t {
                    Token::LBrace => ('{', Token::RBrace),
                    Token::LBracket => ('[', Token::RBracket),
                    _ => ('(', Token::RParen),
                };
                let (kids, end) = self.until(&close);
                Cv::Block(open, kids, t.span.start..end)
            }
            Token::Function(name) => {
                let (kids, end) = self.until(&Token::RParen);
                Cv::Func(name.clone(), kids, t.span.start..end)
            }
            _ => Cv::Tok(t),
        })
    }

    /// Component values up to the matching `close` token (consumed); and where it ends.
    fn until(&mut self, close: &Token) -> (Vec<Cv>, usize) {
        let mut kids = Vec::new();
        loop {
            match self.peek() {
                None => return (kids, self.end()),
                Some(t) if t == close => {
                    let t = self.next().unwrap();
                    return (kids, t.span.end);
                }
                _ => kids.push(self.component_value().unwrap()),
            }
        }
    }

    pub fn component_values(&mut self) -> Vec<Cv> {
        let mut v = Vec::new();
        while let Some(c) = self.component_value() {
            v.push(c);
        }
        v
    }
}

// ---------------------------------------------------------------- rules and declarations

#[derive(Debug, Clone)]
pub enum Item {
    /// name (lower-case), prelude, block contents (None for `@import …;`)
    AtRule(String, Vec<Cv>, Option<Vec<Cv>>),
    /// prelude (selector), block contents
    Qualified(Vec<Cv>, Vec<Cv>),
    /// name, value, !important
    Decl(String, Vec<Cv>, bool),
}

/// §5.4.1 consume a stylesheet's contents (top level: `<!--` and `-->` are skipped).
pub fn rule_list(cvs: Vec<Cv>, top: bool) -> Vec<Item> {
    let mut out = Vec::new();
    let mut it = cvs.into_iter().peekable();
    while let Some(cv) = it.next() {
        match cv.token() {
            Some(Token::Whitespace) => continue,
            Some(Token::Cdo | Token::Cdc) if top => continue,
            Some(Token::AtKeyword(name)) => {
                let name = name.to_ascii_lowercase();
                out.push(at_rule(name, &mut it, false));
            }
            _ => {
                // A qualified rule: prelude up to a {} block.
                let mut prelude = vec![cv];
                let mut block = None;
                for c in it.by_ref() {
                    if let Cv::Block('{', kids, _) = c {
                        block = Some(kids);
                        break;
                    }
                    prelude.push(c);
                }
                if let Some(b) = block {
                    out.push(Item::Qualified(prelude, b));
                }
            }
        }
    }
    out
}

fn at_rule(name: String, it: &mut std::iter::Peekable<impl Iterator<Item = Cv>>, nested: bool) -> Item {
    let mut prelude = Vec::new();
    while let Some(c) = it.peek() {
        match c {
            Cv::Tok(Tok { t: Token::Semicolon, .. }) => {
                it.next();
                return Item::AtRule(name, prelude, None);
            }
            Cv::Tok(Tok { t: Token::RBrace, .. }) if nested => return Item::AtRule(name, prelude, None),
            Cv::Block('{', ..) => {
                let Some(Cv::Block(_, kids, _)) = it.next() else { unreachable!() };
                return Item::AtRule(name, prelude, Some(kids));
            }
            _ => prelude.push(it.next().unwrap()),
        }
    }
    Item::AtRule(name, prelude, None)
}

/// §5.4.4 consume a block's contents: declarations, at-rules and (nested) style rules.
pub fn block_contents(cvs: Vec<Cv>) -> Vec<Item> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < cvs.len() {
        match cvs[i].token() {
            Some(Token::Whitespace | Token::Semicolon) => i += 1,
            Some(Token::AtKeyword(name)) => {
                let name = name.to_ascii_lowercase();
                let mut it = cvs[i + 1..].iter().cloned().peekable();
                let before = it.len();
                let rule = at_rule(name, &mut it, true);
                i += 1 + (before - it.len());
                out.push(rule);
            }
            _ => {
                // Up to the next top-level semicolon: try it as a declaration first.
                let end = cvs[i..].iter().position(|c| matches!(c.token(), Some(Token::Semicolon))).map_or(cvs.len(), |p| i + p);
                if let Some(d) = declaration(&cvs[i..end]) {
                    out.push(d);
                    i = end + 1;
                    continue;
                }
                // Otherwise a nested style rule: prelude up to a {} block.
                let mut j = i;
                while j < cvs.len() && !matches!(cvs[j], Cv::Block('{', ..)) {
                    j += 1;
                }
                if j < cvs.len() {
                    if let Cv::Block(_, kids, _) = &cvs[j] {
                        out.push(Item::Qualified(cvs[i..j].to_vec(), kids.clone()));
                    }
                    i = j + 1;
                } else {
                    // Junk: skip to the semicolon.
                    i = end + 1;
                }
            }
        }
    }
    out
}

/// §5.4.6 consume a declaration: `name : value [!important]`.
pub fn declaration(cvs: &[Cv]) -> Option<Item> {
    let mut i = 0;
    let name = match cvs.first()?.token()? {
        Token::Ident(n) => n.clone(),
        _ => return None,
    };
    i += 1;
    while cvs.get(i).is_some_and(Cv::is_ws) {
        i += 1;
    }
    if !matches!(cvs.get(i)?.token(), Some(Token::Colon)) {
        return None;
    }
    i += 1;
    let mut value: Vec<Cv> = cvs[i..].to_vec();
    while value.first().is_some_and(Cv::is_ws) {
        value.remove(0);
    }
    while value.last().is_some_and(Cv::is_ws) {
        value.pop();
    }
    // `! important` at the end.
    let mut important = false;
    let n = value.len();
    if n >= 2 {
        if let (Some(Token::Delim('!')), Some(Token::Ident(w))) = (value[n - 2].token(), value[n - 1].token()) {
            if w.eq_ignore_ascii_case("important") {
                important = true;
                value.truncate(n - 2);
            }
        }
        if !important && n >= 3 {
            if let (Some(Token::Delim('!')), true, Some(Token::Ident(w))) = (value[n - 3].token(), value[n - 2].is_ws(), value[n - 1].token()) {
                if w.eq_ignore_ascii_case("important") {
                    important = true;
                    value.truncate(n - 3);
                }
            }
        }
        while value.last().is_some_and(Cv::is_ws) {
            value.pop();
        }
    }
    // Only custom properties may hold {} blocks (alongside other values).
    let custom = name.starts_with("--");
    if !custom && value.iter().any(|c| matches!(c, Cv::Block('{', ..))) {
        return None;
    }
    Some(Item::Decl(name, value, important))
}

/// The source text of component values: tokens as written, comments dropped (a space
/// stands in for a comment between two tokens).
pub fn text(src: &[char], cvs: &[Cv]) -> String {
    let mut out = String::new();
    let mut last_end: Option<usize> = None;
    fn walk(src: &[char], cvs: &[Cv], out: &mut String, last_end: &mut Option<usize>) {
        for c in cvs {
            match c {
                Cv::Tok(t) => {
                    if let Some(e) = *last_end {
                        if t.span.start > e && !matches!(t.t, Token::Whitespace) && !out.ends_with(' ') {
                            out.push(' ');
                        }
                    }
                    if matches!(t.t, Token::Whitespace) {
                        if !out.ends_with(' ') {
                            out.push(' ');
                        }
                    } else {
                        out.extend(&src[t.span.clone()]);
                    }
                    *last_end = Some(t.span.end);
                }
                Cv::Block(_, kids, span) | Cv::Func(_, kids, span) => {
                    let open = match c {
                        Cv::Block(o, ..) => *o,
                        _ => '(',
                    };
                    // The opening: "{", "[", "(" or "name(" exactly as written.
                    let head_end = match c {
                        Cv::Func(..) => {
                            let mut k = span.start;
                            while k < span.end && src[k] != '(' {
                                k += 1;
                            }
                            k + 1
                        }
                        _ => span.start + 1,
                    };
                    if let Some(e) = *last_end {
                        if span.start > e && !out.ends_with(' ') {
                            out.push(' ');
                        }
                    }
                    out.extend(&src[span.start..head_end.min(src.len())]);
                    *last_end = Some(head_end);
                    walk(src, kids, out, last_end);
                    let close = match open {
                        '{' => '}',
                        '[' => ']',
                        _ => ')',
                    };
                    if span.end > 0 && src.get(span.end - 1) == Some(&close) {
                        out.push(close);
                    }
                    *last_end = Some(span.end);
                }
            }
        }
    }
    walk(src, cvs, &mut out, &mut last_end);
    out.trim().to_string()
}

/// Parse a whole stylesheet into rules.
pub fn parse_stylesheet(src: &[char]) -> Vec<Item> {
    rule_list(Parser::new(src).component_values(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small JSON reader for the css-parsing-tests files.
    #[derive(Debug, Clone, PartialEq)]
    enum J {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<J>),
        Obj,
    }

    fn json(s: &[char], i: &mut usize) -> J {
        while s[*i].is_whitespace() {
            *i += 1;
        }
        match s[*i] {
            'n' => {
                *i += 4;
                J::Null
            }
            't' => {
                *i += 4;
                J::Bool(true)
            }
            'f' => {
                *i += 5;
                J::Bool(false)
            }
            '"' => {
                *i += 1;
                let mut out = String::new();
                while s[*i] != '"' {
                    if s[*i] == '\\' {
                        *i += 1;
                        match s[*i] {
                            'n' => out.push('\n'),
                            't' => out.push('\t'),
                            'r' => out.push('\r'),
                            'f' => out.push('\u{c}'),
                            'b' => out.push('\u{8}'),
                            'u' => {
                                let h: String = s[*i + 1..*i + 5].iter().collect();
                                let mut cp = u32::from_str_radix(&h, 16).unwrap();
                                *i += 4;
                                if (0xD800..0xDC00).contains(&cp) && s[*i + 1] == '\\' {
                                    let l: String = s[*i + 3..*i + 7].iter().collect();
                                    let lo = u32::from_str_radix(&l, 16).unwrap();
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    *i += 6;
                                }
                                out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                            }
                            c => out.push(c),
                        }
                    } else {
                        out.push(s[*i]);
                    }
                    *i += 1;
                }
                *i += 1;
                J::Str(out)
            }
            '[' => {
                *i += 1;
                let mut v = Vec::new();
                loop {
                    while s[*i].is_whitespace() || s[*i] == ',' {
                        *i += 1;
                    }
                    if s[*i] == ']' {
                        *i += 1;
                        return J::Arr(v);
                    }
                    v.push(json(s, i));
                }
            }
            '{' => {
                let mut d = 0;
                loop {
                    if s[*i] == '{' {
                        d += 1;
                    } else if s[*i] == '}' {
                        d -= 1;
                        if d == 0 {
                            *i += 1;
                            return J::Obj;
                        }
                    }
                    *i += 1;
                }
            }
            _ => {
                let st = *i;
                while *i < s.len() && (s[*i].is_ascii_digit() || matches!(s[*i], '-' | '+' | '.' | 'e' | 'E')) {
                    *i += 1;
                }
                J::Num(s[st..*i].iter().collect::<String>().parse().unwrap())
            }
        }
    }

    fn s(v: &str) -> J {
        J::Str(v.into())
    }

    fn num(kind: &str, v: f64, int: bool, repr: &str) -> Vec<J> {
        vec![s(kind), s(repr), J::Num(v), s(if int { "integer" } else { "number" })]
    }

    /// Our component values in the test files' notation.
    fn to_json(cvs: &[Cv], src: &[char]) -> Vec<J> {
        let mut out = Vec::new();
        for c in cvs {
            // A string or url() cut off by the end of the input is a parse error there.
            let eof = match c {
                Cv::Tok(Tok { t: Token::Str(_), span }) if span.end == src.len() && (span.len() < 2 || src[span.end - 1] != src[span.start] || (span.end >= 2 && src[span.end - 2] == '\\')) => Some("eof-in-string"),
                Cv::Tok(Tok { t: Token::Url(_), span }) if span.end == src.len() && src[span.end - 1] != ')' => Some("eof-in-url"),
                _ => None,
            };
            out.push(match c {
                Cv::Tok(t) => match &t.t {
                    Token::Ident(v) => J::Arr(vec![s("ident"), s(v)]),
                    Token::Function(v) => J::Arr(vec![s("function"), s(v)]),
                    Token::AtKeyword(v) => J::Arr(vec![s("at-keyword"), s(v)]),
                    Token::Hash(v, id) => J::Arr(vec![s("hash"), s(v), s(if *id { "id" } else { "unrestricted" })]),
                    Token::Str(v) => J::Arr(vec![s("string"), s(v)]),
                    Token::BadStr => J::Arr(vec![s("error"), s("bad-string")]),
                    Token::Url(v) => J::Arr(vec![s("url"), s(v)]),
                    Token::BadUrl => J::Arr(vec![s("error"), s("bad-url")]),
                    Token::Delim(c) => s(&c.to_string()),
                    Token::Number(v, i, r) => J::Arr(num("number", *v, *i, r)),
                    Token::Percentage(v, i, r) => J::Arr(num("percentage", *v, *i, r)),
                    Token::Dimension(v, i, r, u) => {
                        let mut a = num("dimension", *v, *i, r);
                        a.push(s(u));
                        J::Arr(a)
                    }
                    Token::Whitespace => s(" "),
                    Token::Cdo => s("<!--"),
                    Token::Cdc => s("-->"),
                    Token::Colon => s(":"),
                    Token::Semicolon => s(";"),
                    Token::Comma => s(","),
                    Token::RBracket => J::Arr(vec![s("error"), s("]")]),
                    Token::RParen => J::Arr(vec![s("error"), s(")")]),
                    Token::RBrace => J::Arr(vec![s("error"), s("}")]),
                    Token::LBracket | Token::LParen | Token::LBrace => unreachable!(),
                },
                Cv::Block(o, kids, _) => {
                    let mut a = vec![s(match o {
                        '{' => "{}",
                        '[' => "[]",
                        _ => "()",
                    })];
                    a.extend(to_json(kids, src));
                    J::Arr(a)
                }
                Cv::Func(n, kids, _) => {
                    let mut a = vec![s("function"), s(n)];
                    a.extend(to_json(kids, src));
                    J::Arr(a)
                }
            });
            if let Some(e) = eof {
                out.push(J::Arr(vec![s("error"), s(e)]));
            }
        }
        out
    }

    /// The test files predate a few spec changes: tokens that no longer exist (`~=`, `||`,
    /// unicode-range) are merged back from their delims for comparison.
    fn legacy(v: Vec<J>) -> Vec<J> {
        let mut out: Vec<J> = Vec::new();
        for j in v {
            let j = match j {
                J::Arr(a) => J::Arr(legacy(a)),
                j => j,
            };
            if let (Some(J::Str(prev)), J::Str(cur)) = (out.last(), &j) {
                let pair = format!("{prev}{cur}");
                if matches!(pair.as_str(), "~=" | "|=" | "^=" | "$=" | "*=" | "||") {
                    out.pop();
                    out.push(J::Str(pair));
                    continue;
                }
            }
            out.push(j);
        }
        out
    }

    fn close(a: &J, b: &J) -> bool {
        match (a, b) {
            (J::Num(x), J::Num(y)) => (x - y).abs() <= 1e-6 * x.abs().max(1.0),
            (J::Arr(x), J::Arr(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| close(p, q)),
            _ => a == b,
        }
    }

    #[test]
    fn css_parsing_tests_component_values() {
        let text = std::fs::read_to_string("tests/css-parsing/component_value_list.json").unwrap();
        let chars: Vec<char> = text.chars().collect();
        let J::Arr(cases) = json(&chars, &mut 0) else { panic!() };
        let (mut pass, mut total, mut skipped) = (0, 0, 0);
        let mut fails = Vec::new();
        for pair in cases.chunks(2) {
            let (J::Str(input), J::Arr(want)) = (&pair[0], &pair[1]) else { continue };
            // unicode-range tokens were removed from the standard.
            if format!("{want:?}").contains("unicode-range") {
                skipped += 1;
                continue;
            }
            total += 1;
            let src = preprocess(input);
            let got = legacy(to_json(&Parser::new(&src).component_values(), &src));
            if close(&J::Arr(got.clone()), &J::Arr(want.clone())) {
                pass += 1;
            } else {
                fails.push(format!("{input:?}\n  got  {got:?}\n  want {want:?}"));
            }
        }
        eprintln!("component_value_list: {pass}/{total} pass ({skipped} unicode-range cases skipped)");
        for f in fails.iter().take(5) {
            eprintln!("{f}");
        }
        assert!(pass * 100 >= total * 95, "{pass}/{total}");
    }

    #[test]
    fn declarations_and_rules() {
        let src = preprocess("a { color: red !important; --x: { a: b }; width: calc(1px + 2px) } @media (x) { b {} } @import url(a.css) screen;");
        let items = parse_stylesheet(&src);
        assert_eq!(items.len(), 3);
        let Item::Qualified(_, block) = &items[0] else { panic!() };
        let decls = block_contents(block.clone());
        assert_eq!(decls.len(), 3);
        let Item::Decl(n, v, imp) = &decls[0] else { panic!() };
        assert_eq!((n.as_str(), text(&src, v).as_str(), *imp), ("color", "red", true));
        let Item::Decl(_, v, _) = &decls[1] else { panic!() };
        assert_eq!(text(&src, v), "{ a: b }");
        let Item::Decl(_, v, _) = &decls[2] else { panic!() };
        assert_eq!(text(&src, v), "calc(1px + 2px)");
        assert!(matches!(&items[2], Item::AtRule(n, _, None) if n == "import"));
    }

    #[test]
    fn nested_rules_in_blocks() {
        let src = preprocess("color: red; &:hover { color: blue } .x & { a: b } @media (y) { c: d }");
        let items = block_contents(Parser::new(&src).component_values());
        assert!(matches!(&items[0], Item::Decl(n, _, _) if n == "color"));
        assert!(matches!(&items[1], Item::Qualified(..)));
        assert!(matches!(&items[2], Item::Qualified(..)));
        assert!(matches!(&items[3], Item::AtRule(n, _, Some(_)) if n == "media"));
    }
}
