//! Conformance runners for the official test suites (run from the command line):
//!
//!   swiftiron --conformance-html5lib <dir with *.dat>
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
            let end = if name == "document" { rest.len() } else { rest.find("\n#").map_or(rest.len(), |e| e + 1) };
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

/// Expected trees list doctype and comment nodes, which Swiftiron doesn't keep yet;
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
    let (mut total, mut pass, mut pass_structure, mut skipped) = (0usize, 0usize, 0usize, 0usize);
    let mut rows = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let (mut t, mut p, mut ps) = (0, 0, 0);
        for case in parse_dat(&text) {
            if case.script_off {
                skipped += 1;
                continue;
            }
            let dom = match &case.fragment {
                Some(_) => html::parse_fragment(&case.data),
                None => html::parse(&case.data),
            };
            let got = html::dump(&dom).trim_end_matches('\n').to_string();
            t += 1;
            if got == case.expected {
                p += 1;
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
