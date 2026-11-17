//! The HTML tokenizer, following the HTML standard's state machine (§13.2.5): it turns text
//! into start tags, end tags, comments, doctypes and character runs. The tree builder pulls
//! tokens one at a time and switches the tokenizer's state for <title>, <style>, <script>,
//! <plaintext> and foreign content, as the standard says.
//!
//! Character tokens come out as runs (`Token::Text`) rather than one per character; the tree
//! builder splits them where a mode cares about whitespace.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    pub name: String,
    /// In source order; duplicates already removed (the first one wins).
    pub attrs: Vec<(String, String)>,
    pub self_closing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Doctype {
    pub name: Option<String>,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
    pub force_quirks: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Doctype(Doctype),
    StartTag(Tag),
    EndTag(Tag),
    Comment(String),
    /// `<?target data>`.
    ProcessingInstruction { target: String, data: String },
    Text(String),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    TagOpen,
    EndTagOpen,
    TagName,
    RcdataLessThan,
    RcdataEndTagOpen,
    RcdataEndTagName,
    RawtextLessThan,
    RawtextEndTagOpen,
    RawtextEndTagName,
    ScriptDataLessThan,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThan,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThan,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThan,
    CommentLessThanBang,
    CommentLessThanBangDash,
    CommentLessThanBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicId,
    DoctypePublicIdDoubleQuoted,
    DoctypePublicIdSingleQuoted,
    AfterDoctypePublicId,
    BetweenDoctypePublicAndSystemIds,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemId,
    DoctypeSystemIdDoubleQuoted,
    DoctypeSystemIdSingleQuoted,
    AfterDoctypeSystemId,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    PiOpen,
    PiTarget,
    AfterPiTarget,
    PiData,
    PiQuestionable,
}

fn entities() -> &'static HashMap<&'static str, &'static str> {
    static M: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    M.get_or_init(|| crate::entities::ENTITIES.iter().copied().collect())
}

/// The longest entity name (`CounterClockwiseContourIntegral;`).
const MAX_ENTITY: usize = 33;

/// Numeric references in 0x80–0x9F mean the Windows-1252 characters there.
fn c1_replacement(n: u32) -> Option<char> {
    const TABLE: [(u32, char); 27] = [
        (0x80, '\u{20AC}'), (0x82, '\u{201A}'), (0x83, '\u{0192}'), (0x84, '\u{201E}'), (0x85, '\u{2026}'), (0x86, '\u{2020}'),
        (0x87, '\u{2021}'), (0x88, '\u{02C6}'), (0x89, '\u{2030}'), (0x8A, '\u{0160}'), (0x8B, '\u{2039}'), (0x8C, '\u{0152}'),
        (0x8E, '\u{017D}'), (0x91, '\u{2018}'), (0x92, '\u{2019}'), (0x93, '\u{201C}'), (0x94, '\u{201D}'), (0x95, '\u{2022}'),
        (0x96, '\u{2013}'), (0x97, '\u{2014}'), (0x98, '\u{02DC}'), (0x99, '\u{2122}'), (0x9A, '\u{0161}'), (0x9B, '\u{203A}'),
        (0x9C, '\u{0153}'), (0x9E, '\u{017E}'), (0x9F, '\u{0178}'),
    ];
    TABLE.iter().find(|(k, _)| *k == n).map(|(_, c)| *c)
}

fn is_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0C}' | ' ')
}

pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    pub state: State,
    /// The last start tag emitted (for "appropriate end tag" checks).
    pub last_start_tag: String,
    /// Set by the tree builder: CDATA sections are allowed (the adjusted current node isn't HTML).
    pub allow_cdata: bool,
    out: std::collections::VecDeque<Token>,
    text: String,
    tag: Tag,
    is_end: bool,
    attr_name: String,
    attr_value: String,
    comment: String,
    doctype: Doctype,
    temp: String,
    pi_target: String,
    done: bool,
}

impl Tokenizer {
    pub fn new(src: &str) -> Tokenizer {
        // Input stream preprocessing: CR LF and lone CR become LF.
        let mut input = Vec::with_capacity(src.len());
        let mut it = src.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\r' {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                input.push('\n');
            } else {
                input.push(c);
            }
        }
        Tokenizer {
            input,
            pos: 0,
            state: State::Data,
            last_start_tag: String::new(),
            allow_cdata: false,
            out: Default::default(),
            text: String::new(),
            tag: Tag { name: String::new(), attrs: Vec::new(), self_closing: false },
            is_end: false,
            attr_name: String::new(),
            attr_value: String::new(),
            comment: String::new(),
            doctype: Doctype { name: None, public_id: None, system_id: None, force_quirks: false },
            temp: String::new(),
            pi_target: String::new(),
            done: false,
        }
    }

    // ------------------------------------------------------------ input

    fn next(&mut self) -> Option<char> {
        let c = self.input.get(self.pos).copied();
        self.pos += 1;
        c
    }

    fn reconsume(&mut self) {
        self.pos -= 1;
    }

    fn peek_is(&self, s: &str, ci: bool) -> bool {
        let n = s.chars().count();
        if self.pos + n > self.input.len() {
            return false;
        }
        self.input[self.pos..self.pos + n].iter().zip(s.chars()).all(|(&a, b)| if ci { a.eq_ignore_ascii_case(&b) } else { a == b })
    }

    // ------------------------------------------------------------ output

    fn emit_char(&mut self, c: char) {
        self.text.push(c);
    }

    fn emit_str(&mut self, s: &str) {
        self.text.push_str(s);
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.out.push_back(Token::Text(std::mem::take(&mut self.text)));
        }
    }

    fn emit(&mut self, t: Token) {
        self.flush_text();
        self.out.push_back(t);
    }

    fn new_tag(&mut self, end: bool) {
        self.tag = Tag { name: String::new(), attrs: Vec::new(), self_closing: false };
        self.is_end = end;
        self.attr_name.clear();
        self.attr_value.clear();
    }

    fn start_attr(&mut self) {
        self.finish_attr();
        self.attr_name.clear();
        self.attr_value.clear();
    }

    fn finish_attr(&mut self) {
        if !self.attr_name.is_empty() {
            let name = std::mem::take(&mut self.attr_name);
            let value = std::mem::take(&mut self.attr_value);
            // A repeated attribute is dropped; the first one wins.
            if !self.tag.attrs.iter().any(|(k, _)| *k == name) {
                self.tag.attrs.push((name, value));
            }
        }
    }

    fn emit_tag(&mut self) {
        self.finish_attr();
        let tag = std::mem::replace(&mut self.tag, Tag { name: String::new(), attrs: Vec::new(), self_closing: false });
        if self.is_end {
            self.emit(Token::EndTag(tag));
        } else {
            self.last_start_tag = tag.name.clone();
            self.emit(Token::StartTag(tag));
        }
    }

    fn appropriate_end(&self) -> bool {
        self.is_end && !self.last_start_tag.is_empty() && self.tag.name == self.last_start_tag
    }

    fn emit_comment(&mut self) {
        let c = std::mem::take(&mut self.comment);
        self.emit(Token::Comment(c));
    }

    fn emit_doctype(&mut self) {
        let d = std::mem::replace(&mut self.doctype, Doctype { name: None, public_id: None, system_id: None, force_quirks: false });
        self.emit(Token::Doctype(d));
    }

    fn eof(&mut self) {
        self.flush_text();
        self.out.push_back(Token::Eof);
        self.done = true;
    }

    /// The next token. After `Eof`, keeps returning `Eof`.
    pub fn next_token(&mut self) -> Token {
        while self.out.is_empty() {
            if self.done {
                return Token::Eof;
            }
            self.step();
            // Long character runs come out in pieces, so the tree builder sees text in order
            // with tags without waiting for the whole document.
            if self.text.len() > 4096 && matches!(self.state, State::Data | State::Rcdata | State::Rawtext | State::ScriptData | State::Plaintext) {
                self.flush_text();
            }
        }
        self.out.pop_front().expect("token")
    }

    // ------------------------------------------------------------ character references

    /// After `&`: try a named or numeric reference. Returns what to emit (or add to the
    /// attribute value); `&` alone if nothing matches.
    fn char_ref(&mut self, in_attr: bool) -> String {
        match self.input.get(self.pos).copied() {
            Some(c) if c.is_ascii_alphanumeric() => self.named_ref(in_attr),
            Some('#') => {
                self.pos += 1;
                self.numeric_ref()
            }
            _ => "&".into(),
        }
    }

    fn named_ref(&mut self, in_attr: bool) -> String {
        let start = self.pos;
        let mut end = start;
        while end < self.input.len() && end - start < MAX_ENTITY && (self.input[end].is_ascii_alphanumeric() || self.input[end] == ';') {
            let semi = self.input[end] == ';';
            end += 1;
            if semi {
                break;
            }
        }
        let cand: String = self.input[start..end].iter().collect();
        // The longest prefix that is an entity name.
        let table = entities();
        let mut best: Option<(usize, &str)> = None;
        for len in (1..=cand.len()).rev() {
            if let Some(v) = cand.get(..len).and_then(|n| table.get(n)) {
                best = Some((len, v));
                break;
            }
        }
        match best {
            Some((len, value)) => {
                let name = &cand[..len];
                let next = self.input.get(start + len).copied();
                // In attributes, a legacy reference without `;` followed by `=` or an
                // alphanumeric is left alone (it's probably part of a URL query: `&copy=1`).
                if in_attr && !name.ends_with(';') && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric()) {
                    self.pos = start + len;
                    return format!("&{name}");
                }
                self.pos = start + len;
                value.to_string()
            }
            None => {
                // Not a reference: the `&` and the run of alphanumerics stay as they are.
                let mut n = start;
                while n < self.input.len() && self.input[n].is_ascii_alphanumeric() {
                    n += 1;
                }
                let s: String = self.input[start..n].iter().collect();
                self.pos = n;
                format!("&{s}")
            }
        }
    }

    fn numeric_ref(&mut self) -> String {
        let hex = matches!(self.input.get(self.pos), Some('x') | Some('X'));
        let x_char = if hex { self.input.get(self.pos).copied() } else { None };
        if hex {
            self.pos += 1;
        }
        let start = self.pos;
        let mut value: u32 = 0;
        let mut overflow = false;
        while let Some(&c) = self.input.get(self.pos) {
            let d = if hex { c.to_digit(16) } else { c.to_digit(10) };
            let Some(d) = d else { break };
            value = value.checked_mul(if hex { 16 } else { 10 }).and_then(|v| v.checked_add(d)).unwrap_or_else(|| {
                overflow = true;
                0x110000
            });
            if overflow {
                value = 0x110000;
            }
            self.pos += 1;
        }
        if self.pos == start {
            // `&#` or `&#x` with no digits: not a reference.
            return match x_char {
                Some(x) => format!("&#{x}"),
                None => "&#".into(),
            };
        }
        if self.input.get(self.pos) == Some(&';') {
            self.pos += 1;
        }
        let c = match value {
            0 => '\u{FFFD}',
            v if v > 0x10FFFF => '\u{FFFD}',
            v if (0xD800..=0xDFFF).contains(&v) => '\u{FFFD}',
            v => c1_replacement(v).or_else(|| char::from_u32(v)).unwrap_or('\u{FFFD}'),
        };
        c.to_string()
    }

    // ------------------------------------------------------------ the state machine

    fn step(&mut self) {
        use State::*;
        match self.state {
            Data => match self.next() {
                Some('&') => {
                    let s = self.char_ref(false);
                    self.emit_str(&s);
                }
                Some('<') => self.state = TagOpen,
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            Rcdata => match self.next() {
                Some('&') => {
                    let s = self.char_ref(false);
                    self.emit_str(&s);
                }
                Some('<') => self.state = RcdataLessThan,
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            Rawtext => match self.next() {
                Some('<') => self.state = RawtextLessThan,
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            ScriptData => match self.next() {
                Some('<') => self.state = ScriptDataLessThan,
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            Plaintext => match self.next() {
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            TagOpen => match self.next() {
                Some('!') => self.state = MarkupDeclarationOpen,
                Some('/') => self.state = EndTagOpen,
                Some(c) if c.is_ascii_alphabetic() => {
                    self.new_tag(false);
                    self.reconsume();
                    self.state = TagName;
                }
                Some('?') => {
                    self.temp.clear();
                    self.state = PiOpen;
                }
                None => {
                    self.emit_char('<');
                    self.eof();
                }
                Some(_) => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = Data;
                }
            },
            EndTagOpen => match self.next() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.new_tag(true);
                    self.reconsume();
                    self.state = TagName;
                }
                Some('>') => self.state = Data,
                None => {
                    self.emit_str("</");
                    self.eof();
                }
                Some(_) => {
                    self.comment.clear();
                    self.reconsume();
                    self.state = BogusComment;
                }
            },
            TagName => match self.next() {
                Some(c) if is_ws(c) => self.state = BeforeAttributeName,
                Some('/') => self.state = SelfClosingStartTag,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                Some('\0') => self.tag.name.push('\u{FFFD}'),
                Some(c) => self.tag.name.push(c.to_ascii_lowercase()),
                None => self.eof(),
            },
            RcdataLessThan => {
                if self.next() == Some('/') {
                    self.temp.clear();
                    self.state = RcdataEndTagOpen;
                } else {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = Rcdata;
                }
            }
            RcdataEndTagOpen => self.raw_end_tag_open(Rcdata, RcdataEndTagName),
            RcdataEndTagName => self.raw_end_tag_name(Rcdata),
            RawtextLessThan => {
                if self.next() == Some('/') {
                    self.temp.clear();
                    self.state = RawtextEndTagOpen;
                } else {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = Rawtext;
                }
            }
            RawtextEndTagOpen => self.raw_end_tag_open(Rawtext, RawtextEndTagName),
            RawtextEndTagName => self.raw_end_tag_name(Rawtext),
            ScriptDataLessThan => match self.next() {
                Some('/') => {
                    self.temp.clear();
                    self.state = ScriptDataEndTagOpen;
                }
                Some('!') => {
                    self.state = ScriptDataEscapeStart;
                    self.emit_str("<!");
                }
                _ => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = ScriptData;
                }
            },
            ScriptDataEndTagOpen => self.raw_end_tag_open(ScriptData, ScriptDataEndTagName),
            ScriptDataEndTagName => self.raw_end_tag_name(ScriptData),
            ScriptDataEscapeStart => {
                if self.next() == Some('-') {
                    self.state = ScriptDataEscapeStartDash;
                    self.emit_char('-');
                } else {
                    self.reconsume();
                    self.state = ScriptData;
                }
            }
            ScriptDataEscapeStartDash => {
                if self.next() == Some('-') {
                    self.state = ScriptDataEscapedDashDash;
                    self.emit_char('-');
                } else {
                    self.reconsume();
                    self.state = ScriptData;
                }
            }
            ScriptDataEscaped => match self.next() {
                Some('-') => {
                    self.state = ScriptDataEscapedDash;
                    self.emit_char('-');
                }
                Some('<') => self.state = ScriptDataEscapedLessThan,
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            ScriptDataEscapedDash => match self.next() {
                Some('-') => {
                    self.state = ScriptDataEscapedDashDash;
                    self.emit_char('-');
                }
                Some('<') => self.state = ScriptDataEscapedLessThan,
                Some('\0') => {
                    self.state = ScriptDataEscaped;
                    self.emit_char('\u{FFFD}');
                }
                Some(c) => {
                    self.state = ScriptDataEscaped;
                    self.emit_char(c);
                }
                None => self.eof(),
            },
            ScriptDataEscapedDashDash => match self.next() {
                Some('-') => self.emit_char('-'),
                Some('<') => self.state = ScriptDataEscapedLessThan,
                Some('>') => {
                    self.state = ScriptData;
                    self.emit_char('>');
                }
                Some('\0') => {
                    self.state = ScriptDataEscaped;
                    self.emit_char('\u{FFFD}');
                }
                Some(c) => {
                    self.state = ScriptDataEscaped;
                    self.emit_char(c);
                }
                None => self.eof(),
            },
            ScriptDataEscapedLessThan => match self.next() {
                Some('/') => {
                    self.temp.clear();
                    self.state = ScriptDataEscapedEndTagOpen;
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.temp.clear();
                    self.emit_char('<');
                    self.reconsume();
                    self.state = ScriptDataDoubleEscapeStart;
                }
                _ => {
                    self.emit_char('<');
                    self.reconsume();
                    self.state = ScriptDataEscaped;
                }
            },
            ScriptDataEscapedEndTagOpen => self.raw_end_tag_open(ScriptDataEscaped, ScriptDataEscapedEndTagName),
            ScriptDataEscapedEndTagName => self.raw_end_tag_name(ScriptDataEscaped),
            ScriptDataDoubleEscapeStart => match self.next() {
                Some(c) if is_ws(c) || c == '/' || c == '>' => {
                    self.state = if self.temp == "script" { ScriptDataDoubleEscaped } else { ScriptDataEscaped };
                    self.emit_char(c);
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.temp.push(c.to_ascii_lowercase());
                    self.emit_char(c);
                }
                _ => {
                    self.reconsume();
                    self.state = ScriptDataEscaped;
                }
            },
            ScriptDataDoubleEscaped => match self.next() {
                Some('-') => {
                    self.state = ScriptDataDoubleEscapedDash;
                    self.emit_char('-');
                }
                Some('<') => {
                    self.state = ScriptDataDoubleEscapedLessThan;
                    self.emit_char('<');
                }
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            ScriptDataDoubleEscapedDash => match self.next() {
                Some('-') => {
                    self.state = ScriptDataDoubleEscapedDashDash;
                    self.emit_char('-');
                }
                Some('<') => {
                    self.state = ScriptDataDoubleEscapedLessThan;
                    self.emit_char('<');
                }
                Some('\0') => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char('\u{FFFD}');
                }
                Some(c) => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char(c);
                }
                None => self.eof(),
            },
            ScriptDataDoubleEscapedDashDash => match self.next() {
                Some('-') => self.emit_char('-'),
                Some('<') => {
                    self.state = ScriptDataDoubleEscapedLessThan;
                    self.emit_char('<');
                }
                Some('>') => {
                    self.state = ScriptData;
                    self.emit_char('>');
                }
                Some('\0') => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char('\u{FFFD}');
                }
                Some(c) => {
                    self.state = ScriptDataDoubleEscaped;
                    self.emit_char(c);
                }
                None => self.eof(),
            },
            ScriptDataDoubleEscapedLessThan => {
                if self.next() == Some('/') {
                    self.temp.clear();
                    self.state = ScriptDataDoubleEscapeEnd;
                    self.emit_char('/');
                } else {
                    self.reconsume();
                    self.state = ScriptDataDoubleEscaped;
                }
            }
            ScriptDataDoubleEscapeEnd => match self.next() {
                Some(c) if is_ws(c) || c == '/' || c == '>' => {
                    self.state = if self.temp == "script" { ScriptDataEscaped } else { ScriptDataDoubleEscaped };
                    self.emit_char(c);
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.temp.push(c.to_ascii_lowercase());
                    self.emit_char(c);
                }
                _ => {
                    self.reconsume();
                    self.state = ScriptDataDoubleEscaped;
                }
            },
            BeforeAttributeName => match self.next() {
                Some(c) if is_ws(c) => {}
                Some('/') | Some('>') | None => {
                    if self.pos <= self.input.len() {
                        self.reconsume();
                    }
                    self.state = AfterAttributeName;
                }
                Some('=') => {
                    self.start_attr();
                    self.attr_name.push('=');
                    self.state = AttributeName;
                }
                Some(_) => {
                    self.start_attr();
                    self.reconsume();
                    self.state = AttributeName;
                }
            },
            AttributeName => match self.next() {
                Some(c) if is_ws(c) || c == '/' || c == '>' => {
                    self.reconsume();
                    self.state = AfterAttributeName;
                }
                None => {
                    self.reconsume();
                    self.state = AfterAttributeName;
                }
                Some('=') => self.state = BeforeAttributeValue,
                Some('\0') => self.attr_name.push('\u{FFFD}'),
                Some(c) => self.attr_name.push(c.to_ascii_lowercase()),
            },
            AfterAttributeName => match self.next() {
                Some(c) if is_ws(c) => {}
                Some('/') => self.state = SelfClosingStartTag,
                Some('=') => self.state = BeforeAttributeValue,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                None => self.eof(),
                Some(_) => {
                    self.start_attr();
                    self.reconsume();
                    self.state = AttributeName;
                }
            },
            BeforeAttributeValue => match self.next() {
                Some(c) if is_ws(c) => {}
                Some('"') => self.state = AttributeValueDoubleQuoted,
                Some('\'') => self.state = AttributeValueSingleQuoted,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                _ => {
                    self.reconsume();
                    self.state = AttributeValueUnquoted;
                }
            },
            AttributeValueDoubleQuoted => match self.next() {
                Some('"') => self.state = AfterAttributeValueQuoted,
                Some('&') => {
                    let s = self.char_ref(true);
                    self.attr_value.push_str(&s);
                }
                Some('\0') => self.attr_value.push('\u{FFFD}'),
                Some(c) => self.attr_value.push(c),
                None => self.eof(),
            },
            AttributeValueSingleQuoted => match self.next() {
                Some('\'') => self.state = AfterAttributeValueQuoted,
                Some('&') => {
                    let s = self.char_ref(true);
                    self.attr_value.push_str(&s);
                }
                Some('\0') => self.attr_value.push('\u{FFFD}'),
                Some(c) => self.attr_value.push(c),
                None => self.eof(),
            },
            AttributeValueUnquoted => match self.next() {
                Some(c) if is_ws(c) => self.state = BeforeAttributeName,
                Some('&') => {
                    let s = self.char_ref(true);
                    self.attr_value.push_str(&s);
                }
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                Some('\0') => self.attr_value.push('\u{FFFD}'),
                Some(c) => self.attr_value.push(c),
                None => self.eof(),
            },
            AfterAttributeValueQuoted => match self.next() {
                Some(c) if is_ws(c) => self.state = BeforeAttributeName,
                Some('/') => self.state = SelfClosingStartTag,
                Some('>') => {
                    self.state = Data;
                    self.emit_tag();
                }
                None => self.eof(),
                Some(_) => {
                    self.reconsume();
                    self.state = BeforeAttributeName;
                }
            },
            SelfClosingStartTag => match self.next() {
                Some('>') => {
                    self.tag.self_closing = true;
                    self.state = Data;
                    self.emit_tag();
                }
                None => self.eof(),
                Some(_) => {
                    self.reconsume();
                    self.state = BeforeAttributeName;
                }
            },
            BogusComment => match self.next() {
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                None => {
                    self.emit_comment();
                    self.eof();
                }
                Some('\0') => self.comment.push('\u{FFFD}'),
                Some(c) => self.comment.push(c),
            },
            MarkupDeclarationOpen => {
                if self.peek_is("--", false) {
                    self.pos += 2;
                    self.comment.clear();
                    self.state = CommentStart;
                } else if self.peek_is("doctype", true) {
                    self.pos += 7;
                    self.state = Doctype;
                } else if self.peek_is("[CDATA[", false) {
                    self.pos += 7;
                    if self.allow_cdata {
                        self.state = CdataSection;
                    } else {
                        self.comment = "[CDATA[".into();
                        self.state = BogusComment;
                    }
                } else {
                    self.comment.clear();
                    self.state = BogusComment;
                }
            }
            CommentStart => match self.next() {
                Some('-') => self.state = CommentStartDash,
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                _ => {
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentStartDash => match self.next() {
                Some('-') => self.state = CommentEnd,
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                None => {
                    self.emit_comment();
                    self.eof();
                }
                Some(_) => {
                    self.comment.push('-');
                    self.reconsume();
                    self.state = Comment;
                }
            },
            Comment => match self.next() {
                Some('<') => {
                    self.comment.push('<');
                    self.state = CommentLessThan;
                }
                Some('-') => self.state = CommentEndDash,
                Some('\0') => self.comment.push('\u{FFFD}'),
                Some(c) => self.comment.push(c),
                None => {
                    self.emit_comment();
                    self.eof();
                }
            },
            CommentLessThan => match self.next() {
                Some('!') => {
                    self.comment.push('!');
                    self.state = CommentLessThanBang;
                }
                Some('<') => self.comment.push('<'),
                _ => {
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentLessThanBang => {
                if self.next() == Some('-') {
                    self.state = CommentLessThanBangDash;
                } else {
                    self.reconsume();
                    self.state = Comment;
                }
            }
            CommentLessThanBangDash => {
                if self.next() == Some('-') {
                    self.state = CommentLessThanBangDashDash;
                } else {
                    self.reconsume();
                    self.state = CommentEndDash;
                }
            }
            CommentLessThanBangDashDash => {
                self.next();
                self.reconsume();
                self.state = CommentEnd;
            }
            CommentEndDash => match self.next() {
                Some('-') => self.state = CommentEnd,
                None => {
                    self.emit_comment();
                    self.eof();
                }
                Some(_) => {
                    self.comment.push('-');
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentEnd => match self.next() {
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                Some('!') => self.state = CommentEndBang,
                Some('-') => self.comment.push('-'),
                None => {
                    self.emit_comment();
                    self.eof();
                }
                Some(_) => {
                    self.comment.push_str("--");
                    self.reconsume();
                    self.state = Comment;
                }
            },
            CommentEndBang => match self.next() {
                Some('-') => {
                    self.comment.push_str("--!");
                    self.state = CommentEndDash;
                }
                Some('>') => {
                    self.state = Data;
                    self.emit_comment();
                }
                None => {
                    self.emit_comment();
                    self.eof();
                }
                Some(_) => {
                    self.comment.push_str("--!");
                    self.reconsume();
                    self.state = Comment;
                }
            },
            Doctype => match self.next() {
                Some(c) if is_ws(c) => self.state = BeforeDoctypeName,
                Some('>') => {
                    self.reconsume();
                    self.state = BeforeDoctypeName;
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.eof();
                }
                Some(_) => {
                    self.reconsume();
                    self.state = BeforeDoctypeName;
                }
            },
            BeforeDoctypeName => match self.next() {
                Some(c) if is_ws(c) => {}
                Some('\0') => {
                    self.doctype.name = Some("\u{FFFD}".into());
                    self.state = DoctypeName;
                }
                Some('>') => {
                    self.doctype.force_quirks = true;
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.eof();
                }
                Some(c) => {
                    self.doctype.name = Some(c.to_ascii_lowercase().to_string());
                    self.state = DoctypeName;
                }
            },
            DoctypeName => match self.next() {
                Some(c) if is_ws(c) => self.state = AfterDoctypeName,
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                Some('\0') => self.doctype.name.get_or_insert_with(String::new).push('\u{FFFD}'),
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.eof();
                }
                Some(c) => self.doctype.name.get_or_insert_with(String::new).push(c.to_ascii_lowercase()),
            },
            AfterDoctypeName => match self.next() {
                Some(c) if is_ws(c) => {}
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.eof();
                }
                Some(_) => {
                    self.reconsume();
                    if self.peek_is("public", true) {
                        self.pos += 6;
                        self.state = AfterDoctypePublicKeyword;
                    } else if self.peek_is("system", true) {
                        self.pos += 6;
                        self.state = AfterDoctypeSystemKeyword;
                    } else {
                        self.pos += 1;
                        self.doctype.force_quirks = true;
                        self.state = BogusDoctype;
                    }
                }
            },
            AfterDoctypePublicKeyword | AfterDoctypeSystemKeyword => {
                let public = self.state == AfterDoctypePublicKeyword;
                match self.next() {
                    Some(c) if is_ws(c) => self.state = if public { BeforeDoctypePublicId } else { BeforeDoctypeSystemId },
                    Some('"') => {
                        self.set_id(public, String::new());
                        self.state = if public { DoctypePublicIdDoubleQuoted } else { DoctypeSystemIdDoubleQuoted };
                    }
                    Some('\'') => {
                        self.set_id(public, String::new());
                        self.state = if public { DoctypePublicIdSingleQuoted } else { DoctypeSystemIdSingleQuoted };
                    }
                    Some('>') => {
                        self.doctype.force_quirks = true;
                        self.state = Data;
                        self.emit_doctype();
                    }
                    None => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.eof();
                    }
                    Some(_) => {
                        self.doctype.force_quirks = true;
                        self.reconsume();
                        self.state = BogusDoctype;
                    }
                }
            }
            BeforeDoctypePublicId | BeforeDoctypeSystemId => {
                let public = self.state == BeforeDoctypePublicId;
                match self.next() {
                    Some(c) if is_ws(c) => {}
                    Some('"') => {
                        self.set_id(public, String::new());
                        self.state = if public { DoctypePublicIdDoubleQuoted } else { DoctypeSystemIdDoubleQuoted };
                    }
                    Some('\'') => {
                        self.set_id(public, String::new());
                        self.state = if public { DoctypePublicIdSingleQuoted } else { DoctypeSystemIdSingleQuoted };
                    }
                    Some('>') => {
                        self.doctype.force_quirks = true;
                        self.state = Data;
                        self.emit_doctype();
                    }
                    None => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.eof();
                    }
                    Some(_) => {
                        self.doctype.force_quirks = true;
                        self.reconsume();
                        self.state = BogusDoctype;
                    }
                }
            }
            DoctypePublicIdDoubleQuoted | DoctypePublicIdSingleQuoted | DoctypeSystemIdDoubleQuoted | DoctypeSystemIdSingleQuoted => {
                let public = matches!(self.state, DoctypePublicIdDoubleQuoted | DoctypePublicIdSingleQuoted);
                let quote = if matches!(self.state, DoctypePublicIdDoubleQuoted | DoctypeSystemIdDoubleQuoted) { '"' } else { '\'' };
                match self.next() {
                    Some(c) if c == quote => self.state = if public { AfterDoctypePublicId } else { AfterDoctypeSystemId },
                    Some('\0') => self.push_id(public, '\u{FFFD}'),
                    Some('>') => {
                        self.doctype.force_quirks = true;
                        self.state = Data;
                        self.emit_doctype();
                    }
                    None => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.eof();
                    }
                    Some(c) => self.push_id(public, c),
                }
            }
            AfterDoctypePublicId => match self.next() {
                Some(c) if is_ws(c) => self.state = BetweenDoctypePublicAndSystemIds,
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                Some('"') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdSingleQuoted;
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            BetweenDoctypePublicAndSystemIds => match self.next() {
                Some(c) if is_ws(c) => {}
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                Some('"') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdDoubleQuoted;
                }
                Some('\'') => {
                    self.doctype.system_id = Some(String::new());
                    self.state = DoctypeSystemIdSingleQuoted;
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.eof();
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            AfterDoctypeSystemId => match self.next() {
                Some(c) if is_ws(c) => {}
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.eof();
                }
                Some(_) => {
                    // Not quirks: just junk after the system id.
                    self.reconsume();
                    self.state = BogusDoctype;
                }
            },
            BogusDoctype => match self.next() {
                Some('>') => {
                    self.state = Data;
                    self.emit_doctype();
                }
                None => {
                    self.emit_doctype();
                    self.eof();
                }
                Some(_) => {}
            },
            PiOpen => match self.next() {
                None => self.eof(),
                Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                    self.reconsume();
                    self.state = PiTarget;
                }
                Some(_) => self.pi_to_comment(),
            },
            PiTarget => match self.next() {
                Some(c) if is_ws(c) || c == '?' || c == '>' => {
                    if self.temp.eq_ignore_ascii_case("xml") || self.temp.eq_ignore_ascii_case("xml-stylesheet") {
                        self.pi_to_comment();
                    } else {
                        self.pi_target = std::mem::take(&mut self.temp);
                        self.comment.clear();
                        self.reconsume();
                        self.state = AfterPiTarget;
                    }
                }
                Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => self.temp.push(c),
                None => self.eof(),
                Some(_) => self.pi_to_comment(),
            },
            AfterPiTarget => match self.next() {
                Some(c) if is_ws(c) => {}
                _ => {
                    self.reconsume();
                    self.state = PiData;
                }
            },
            PiData => match self.next() {
                Some('>') => {
                    self.state = Data;
                    self.emit_pi();
                }
                Some('?') => self.state = PiQuestionable,
                None => self.eof(),
                Some('\0') => self.comment.push('\u{FFFD}'),
                Some(c) => self.comment.push(c),
            },
            PiQuestionable => match self.next() {
                Some('>') => {
                    self.state = Data;
                    self.emit_pi();
                }
                None => self.eof(),
                Some(_) => {
                    self.comment.push('?');
                    self.reconsume();
                    self.state = PiData;
                }
            },
            CdataSection => match self.next() {
                Some(']') => self.state = CdataSectionBracket,
                Some(c) => self.emit_char(c),
                None => self.eof(),
            },
            CdataSectionBracket => {
                if self.next() == Some(']') {
                    self.state = CdataSectionEnd;
                } else {
                    self.emit_char(']');
                    self.reconsume();
                    self.state = CdataSection;
                }
            }
            CdataSectionEnd => match self.next() {
                Some(']') => self.emit_char(']'),
                Some('>') => self.state = Data,
                _ => {
                    self.emit_str("]]");
                    self.reconsume();
                    self.state = CdataSection;
                }
            },
        }
    }

    /// An invalid processing instruction: `?` plus what was read becomes a bogus comment.
    fn pi_to_comment(&mut self) {
        self.comment = format!("?{}", self.temp);
        self.temp.clear();
        if self.pos <= self.input.len() {
            self.reconsume();
        }
        self.state = State::BogusComment;
    }

    fn emit_pi(&mut self) {
        let target = std::mem::take(&mut self.pi_target);
        let data = std::mem::take(&mut self.comment);
        self.emit(Token::ProcessingInstruction { target, data });
    }

    fn set_id(&mut self, public: bool, v: String) {
        if public {
            self.doctype.public_id = Some(v);
        } else {
            self.doctype.system_id = Some(v);
        }
    }

    fn push_id(&mut self, public: bool, c: char) {
        let id = if public { &mut self.doctype.public_id } else { &mut self.doctype.system_id };
        id.get_or_insert_with(String::new).push(c);
    }

    /// `</` inside RCDATA/RAWTEXT/script data: an end tag name may follow.
    fn raw_end_tag_open(&mut self, back: State, name_state: State) {
        match self.next() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.new_tag(true);
                self.reconsume();
                self.state = name_state;
            }
            _ => {
                self.emit_str("</");
                self.reconsume();
                self.state = back;
            }
        }
    }

    /// The end tag name inside RCDATA/RAWTEXT/script data: only the matching end tag
    /// (`</title>` after `<title>`) ends it; anything else is text.
    fn raw_end_tag_name(&mut self, back: State) {
        let c = self.next();
        match c {
            Some(c) if (is_ws(c) || c == '/' || c == '>') && self.appropriate_end() => match c {
                '/' => self.state = State::SelfClosingStartTag,
                '>' => {
                    self.state = State::Data;
                    self.emit_tag();
                }
                _ => self.state = State::BeforeAttributeName,
            },
            Some(c) if c.is_ascii_alphabetic() => {
                self.tag.name.push(c.to_ascii_lowercase());
                self.temp.push(c);
            }
            _ => {
                let t = format!("</{}", self.temp);
                self.emit_str(&t);
                if c.is_some() || self.pos <= self.input.len() {
                    self.reconsume();
                }
                self.state = back;
            }
        }
    }
}
