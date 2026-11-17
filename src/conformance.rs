//! Conformance runners for the official test suites (run from the command line):
//!
//!   ironswift --conformance-html5lib <dir with *.dat>
//!
//! The html5lib tree-construction tests are the HTML-parsing part of the Web Platform Tests
//! (wpt/html/syntax/parsing/resources/*.dat). Each test gives markup and the exact tree a
//! browser must build from it.

use crate::html;

struct Case {
    data: String,
    fragment: Option<String>,
    expected: String,
    script_off: bool,
}

fn parse_dat(text: &str) -> Vec<Case> {
    let mut out = Vec::new();
    let text = text.replace("\r\n", "\n");
    for chunk in text.split("\n#data\n").map(|c| c.strip_prefix("#data\n").unwrap_or(c)) {
        if chunk.trim().is_empty() {
            continue;
        }
        let section = |name: &str| -> Option<String> {
            let marker = format!("\n#{name}\n");
            let start = if chunk.starts_with(&marker[1..]) { marker.len() - 1 } else { chunk.find(&marker)? + marker.len() };
            let rest = &chunk[start..];
            // A section runs until the next "#name" line (the document section runs to the end).
            let end = rest.find("\n#").map_or(rest.len(), |e| e + 1);
            Some(rest[..end].to_string())
        };
        let data = match chunk.find("\n#errors\n") {
            Some(e) => chunk[..e].to_string(),
            None => continue,
        };
        let Some(expected) = section("document") else { continue };
        out.push(Case {
            data,
            fragment: section("document-fragment").map(|f| f.trim().to_string()),
            expected: expected.trim_end_matches('\n').to_string(),
            script_off: chunk.contains("\n#script-off\n"),
        });
    }
    out
}

/// Expected trees list doctype and comment nodes, which Ironswift doesn't keep yet;
/// this removes them to measure the element/text structure alone.
fn without_doctype_and_comments(tree: &str) -> String {
    tree.lines().filter(|l| !l.trim_start_matches('|').trim_start().starts_with("<!")).collect::<Vec<_>>().join("\n")
}

pub fn html5lib(dir: &str) -> i32 {
    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "dat")).collect(),
        Err(e) => {
            eprintln!("{dir}: {e}");
            return 2;
        }
    };
    files.sort();
    let (mut total, mut pass, mut pass_structure, skipped) = (0usize, 0usize, 0usize, 0usize);
    let mut rows = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let (mut t, mut p, mut ps) = (0, 0, 0);
        for case in parse_dat(&text) {
            let scripting = !case.script_off;
            if std::env::var("IRONSWIFT_TRACE").is_ok() {
                eprintln!("CASE {:?} {:?}", case.data, case.fragment);
            }
            let dom = match &case.fragment {
                Some(ctx) => {
                    let (ns, name) = match ctx.split_once(' ') {
                        Some(("svg", n)) => (crate::dom::Ns::Svg, n),
                        Some(("math", n)) => (crate::dom::Ns::MathMl, n),
                        _ => (crate::dom::Ns::Html, ctx.as_str()),
                    };
                    crate::treebuilder::TreeBuilder::parse_fragment(&case.data, name, ns, scripting)
                }
                // Tests with scripts run them while parsing, as a browser does.
                None if case.data.contains("<script") && f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("scripted")) => {
                    let mut js = crate::js::Js::new("about:blank");
                    js.parse_and_run(&case.data, "about:blank", &Default::default());
                    crate::js::with_dom(|d| d.clone())
                }
                None => crate::treebuilder::TreeBuilder::parse_document(&case.data, scripting),
            };
            let got = html::dump(&dom).trim_end_matches('\n').to_string();
            t += 1;
            if got == case.expected {
                p += 1;
            } else if std::env::var("IRONSWIFT_SHOW_FAILS").is_ok() {
                println!("--- {}\n{}\n--- expected\n{}\n--- got\n{}\n", f.display(), case.data, case.expected, got);
            }
            if got == without_doctype_and_comments(&case.expected) || got == case.expected {
                ps += 1;
            }
        }
        rows.push((f.file_name().unwrap_or_default().to_string_lossy().to_string(), t, p, ps));
        total += t;
        pass += p;
        pass_structure += ps;
    }
    for (name, t, p, ps) in &rows {
        println!("{name:<45} {p:>4}/{t:<4} exact   {ps:>4}/{t:<4} ignoring doctype/comments");
    }
    let pct = |n: usize| if total == 0 { 0.0 } else { n as f64 * 100.0 / total as f64 };
    println!("\nTOTAL: {pass}/{total} exact ({:.1}%), {pass_structure}/{total} ignoring doctype and comment nodes ({:.1}%); {skipped} scripting-off tests skipped", pct(pass), pct(pass_structure));
    0
}

// ---------------------------------------------------------------- tokenizer tests

/// Just enough JSON for the html5lib test files.
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Obj(v) => v.iter().find(|(kk, _)| kk == k).map(|(_, v)| v),
            _ => None,
        }
    }
    fn str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }
}

fn parse_json(s: &str) -> Option<Json> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    fn ws(c: &[char], i: &mut usize) {
        while *i < c.len() && c[*i].is_whitespace() {
            *i += 1;
        }
    }
    fn val(c: &[char], i: &mut usize) -> Option<Json> {
        ws(c, i);
        match *c.get(*i)? {
            '{' => {
                *i += 1;
                let mut v = Vec::new();
                loop {
                    ws(c, i);
                    if c.get(*i) == Some(&'}') {
                        *i += 1;
                        return Some(Json::Obj(v));
                    }
                    let Json::Str(k) = val(c, i)? else { return None };
                    ws(c, i);
                    *i += 1; // :
                    let x = val(c, i)?;
                    v.push((k, x));
                    ws(c, i);
                    if c.get(*i) == Some(&',') {
                        *i += 1;
                    }
                }
            }
            '[' => {
                *i += 1;
                let mut v = Vec::new();
                loop {
                    ws(c, i);
                    if c.get(*i) == Some(&']') {
                        *i += 1;
                        return Some(Json::Arr(v));
                    }
                    v.push(val(c, i)?);
                    ws(c, i);
                    if c.get(*i) == Some(&',') {
                        *i += 1;
                    }
                }
            }
            '"' => {
                *i += 1;
                let mut out: Vec<u16> = Vec::new();
                let mut text = String::new();
                loop {
                    let ch = *c.get(*i)?;
                    *i += 1;
                    match ch {
                        '"' => break,
                        '\\' => {
                            let e = *c.get(*i)?;
                            *i += 1;
                            match e {
                                'u' => {
                                    let h: String = c.get(*i..*i + 4)?.iter().collect();
                                    *i += 4;
                                    out.push(u16::from_str_radix(&h, 16).ok()?);
                                    continue;
                                }
                                'n' => text.push('\n'),
                                't' => text.push('\t'),
                                'r' => text.push('\r'),
                                'b' => text.push('\u{8}'),
                                'f' => text.push('\u{c}'),
                                other => text.push(other),
                            }
                        }
                        other => {
                            if !out.is_empty() {
                                text.push_str(&String::from_utf16_lossy(&out));
                                out.clear();
                            }
                            text.push(other);
                            continue;
                        }
                    }
                    if !out.is_empty() {
                        let last = text.pop();
                        text.push_str(&String::from_utf16_lossy(&out));
                        out.clear();
                        if let Some(l) = last {
                            text.push(l);
                        }
                    }
                }
                if !out.is_empty() {
                    text.push_str(&String::from_utf16_lossy(&out));
                }
                Some(Json::Str(text))
            }
            't' => {
                *i += 4;
                Some(Json::Bool(true))
            }
            'f' => {
                *i += 5;
                Some(Json::Bool(false))
            }
            'n' => {
                *i += 4;
                Some(Json::Null)
            }
            _ => {
                let start = *i;
                while *i < c.len() && (c[*i].is_ascii_digit() || matches!(c[*i], '-' | '+' | '.' | 'e' | 'E')) {
                    *i += 1;
                }
                c[start..*i].iter().collect::<String>().parse().ok().map(Json::Num)
            }
        }
    }
    val(&c, &mut i)
}

/// Undo the tests' "doubleEscaped" encoding (\uXXXX written out literally).
fn unescape_double(s: &str) -> String {
    let mut units: Vec<u16> = Vec::new();
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\\' && c.get(i + 1) == Some(&'u') && i + 6 <= c.len() {
            if let Ok(v) = u16::from_str_radix(&c[i + 2..i + 6].iter().collect::<String>(), 16) {
                units.push(v);
                i += 6;
                continue;
            }
        }
        let mut b = [0u16; 2];
        units.extend_from_slice(c[i].encode_utf16(&mut b));
        i += 1;
    }
    String::from_utf16_lossy(&units)
}

fn token_json(t: &crate::tokenizer::Token) -> Option<Json> {
    use crate::tokenizer::Token;
    let s = |x: &str| Json::Str(x.to_string());
    Some(match t {
        Token::Doctype(d) => Json::Arr(vec![
            s("DOCTYPE"),
            d.name.as_deref().map_or(Json::Null, s),
            d.public_id.as_deref().map_or(Json::Null, s),
            d.system_id.as_deref().map_or(Json::Null, s),
            Json::Bool(!d.force_quirks),
        ]),
        Token::StartTag(tag) => {
            let mut v = vec![s("StartTag"), s(&tag.name), Json::Obj(tag.attrs.iter().map(|(k, v)| (k.clone(), s(v))).collect())];
            if tag.self_closing {
                v.push(Json::Bool(true));
            }
            Json::Arr(v)
        }
        Token::EndTag(tag) => Json::Arr(vec![s("EndTag"), s(&tag.name)]),
        Token::Comment(c) => Json::Arr(vec![s("Comment"), s(c)]),
        Token::ProcessingInstruction { target, data } => Json::Arr(vec![s("ProcessingInstruction"), s(target), s(data)]),
        Token::Text(t) => Json::Arr(vec![s("Character"), s(t)]),
        Token::Eof => return None,
    })
}

/// Join adjacent Character tokens, and sort attributes, so outputs compare equal.
fn normalize(tokens: Vec<Json>) -> Vec<Json> {
    let mut out: Vec<Json> = Vec::new();
    for t in tokens {
        let t = match t {
            Json::Arr(mut v) if v.first() == Some(&Json::Str("StartTag".into())) => {
                if let Some(Json::Obj(a)) = v.get_mut(2) {
                    a.sort_by(|x, y| x.0.cmp(&y.0));
                }
                Json::Arr(v)
            }
            other => other,
        };
        if let (Some(Json::Arr(prev)), Json::Arr(cur)) = (out.last_mut(), &t) {
            if prev.first() == Some(&Json::Str("Character".into())) && cur.first() == Some(&Json::Str("Character".into())) {
                if let (Some(Json::Str(a)), Some(Json::Str(b))) = (prev.get_mut(1), cur.get(1)) {
                    a.push_str(b);
                    continue;
                }
            }
        }
        out.push(t);
    }
    out
}

pub fn tokenizer_tests(dir: &str) -> i32 {
    use crate::tokenizer::{State, Token, Tokenizer};
    let mut files: Vec<_> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "test")).collect()).unwrap_or_default();
    files.sort();
    let (mut total, mut pass) = (0, 0);
    for f in &files {
        let Some(json) = std::fs::read_to_string(f).ok().and_then(|t| parse_json(&t)) else { continue };
        let Some(Json::Arr(tests)) = json.get("tests").cloned() else { continue };
        let (mut t_total, mut t_pass) = (0, 0);
        let mut first_fail: Option<String> = None;
        for t in tests {
            let double = t.get("doubleEscaped") == Some(&Json::Bool(true));
            let fix = |s: &str| if double { unescape_double(s) } else { s.to_string() };
            let input = fix(t.get("input").and_then(Json::str).unwrap_or(""));
            let states: Vec<String> = match t.get("initialStates") {
                Some(Json::Arr(v)) => v.iter().filter_map(|x| x.str().map(str::to_string)).collect(),
                _ => vec!["Data state".into()],
            };
            let expected = match t.get("output") {
                Some(Json::Arr(v)) => normalize(
                    v.iter()
                        .map(|tok| match tok {
                            Json::Arr(parts) => Json::Arr(parts.iter().map(|p| match p {
                                Json::Str(s) => Json::Str(fix(s)),
                                Json::Obj(a) => Json::Obj(a.iter().map(|(k, v)| (fix(k), match v { Json::Str(s) => Json::Str(fix(s)), o => o.clone() })).collect()),
                                o => o.clone(),
                            }).collect()),
                            o => o.clone(),
                        })
                        .collect(),
                ),
                _ => Vec::new(),
            };
            for st in states {
                let state = match st.as_str() {
                    "Data state" => State::Data,
                    "PLAINTEXT state" => State::Plaintext,
                    "RCDATA state" => State::Rcdata,
                    "RAWTEXT state" => State::Rawtext,
                    "Script data state" => State::ScriptData,
                    "CDATA section state" => State::CdataSection,
                    _ => continue,
                };
                let mut tk = Tokenizer::new(&input);
                tk.state = state;
                if let Some(last) = t.get("lastStartTag").and_then(Json::str) {
                    tk.last_start_tag = last.to_string();
                }
                let mut got = Vec::new();
                loop {
                    let tok = tk.next_token();
                    if tok == Token::Eof {
                        break;
                    }
                    if let Some(j) = token_json(&tok) {
                        got.push(j);
                    }
                }
                let got = normalize(got);
                t_total += 1;
                if got == expected {
                    t_pass += 1;
                } else if first_fail.is_none() {
                    first_fail = Some(format!("{:?}\n      expected {:?}\n      got      {:?}", input, expected, got));
                }
            }
        }
        println!("{:<34} {t_pass:>5}/{t_total}", f.file_name().unwrap_or_default().to_string_lossy());
        if let Some(ff) = first_fail {
            println!("    first failure: {}", ff.chars().take(600).collect::<String>());
        }
        total += t_total;
        pass += t_pass;
    }
    println!("\nTOTAL: {pass}/{total} ({:.1}%)", pass as f64 * 100.0 / total.max(1) as f64);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dat_format() {
        let dat = "#data\n<p>One<p>Two\n#errors\n(1,3): x\n#document\n| <html>\n|   <head>\n|   <body>\n\n#data\nx\n#errors\n#document-fragment\ndiv\n#document\n| \"x\"\n";
        let cases = parse_dat(dat);
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].data, "<p>One<p>Two");
        assert_eq!(cases[0].expected, "| <html>\n|   <head>\n|   <body>");
        assert_eq!(cases[1].fragment.as_deref(), Some("div"));
        assert_eq!(cases[1].expected, "| \"x\"");
    }
}
