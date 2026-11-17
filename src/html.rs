//! HTML parser: turns source text into a DOM tree.
//! Forgiving like real browsers: unclosed tags, stray end tags and
//! common implied closes (<p>, <li>, <td>...) are handled.

use crate::dom::{Dom, NodeData, NodeId};

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr",
];
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title", "noscript"];
/// Opening one of these implicitly closes an open <p>.
const CLOSES_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "div", "dl", "fieldset", "footer", "form", "h1", "h2",
    "h3", "h4", "h5", "h6", "header", "hr", "main", "nav", "ol", "p", "pre", "section", "table", "ul",
];

pub fn parse(src: &str) -> Dom {
    let mut dom = parse_fragment(src);
    ensure_skeleton(&mut dom);
    dom
}

/// Parse markup as-is, without adding <html>/<head>/<body> (for innerHTML and friends).
pub fn parse_fragment(src: &str) -> Dom {
    let mut dom = Dom::new();
    let mut stack: Vec<NodeId> = vec![Dom::ROOT];
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut text = String::new();

    let starts = |i: usize, s: &str| -> bool {
        let mut j = i;
        for c in s.chars() {
            if j >= n || !chars[j].eq_ignore_ascii_case(&c) {
                return false;
            }
            j += 1;
        }
        true
    };
    let find = |from: usize, s: &str| -> Option<usize> { (from..n).find(|&j| starts(j, s)) };

    while i < n {
        let c = chars[i];
        if c != '<' {
            text.push(c);
            i += 1;
            continue;
        }
        if starts(i, "<!--") {
            flush(&mut dom, &stack, &mut text);
            i = find(i + 4, "-->").map(|j| j + 3).unwrap_or(n);
        } else if starts(i, "<!") || starts(i, "<?") {
            flush(&mut dom, &stack, &mut text);
            i = find(i, ">").map(|j| j + 1).unwrap_or(n);
        } else if starts(i, "</") {
            flush(&mut dom, &stack, &mut text);
            let mut j = i + 2;
            let mut name = String::new();
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '-') {
                name.push(chars[j].to_ascii_lowercase());
                j += 1;
            }
            i = find(j, ">").map(|j| j + 1).unwrap_or(n);
            if let Some(pos) = stack.iter().rposition(|&id| dom.tag(id) == Some(&name)) {
                stack.truncate(pos.max(1));
            }
        } else if i + 1 < n && chars[i + 1].is_ascii_alphabetic() {
            flush(&mut dom, &stack, &mut text);
            let (tag, attrs, self_closing, next) = parse_tag(&chars, i + 1);
            i = next;
            implied_closes(&dom, &mut stack, &tag);
            let parent = *stack.last().unwrap();
            let id = dom.add(NodeData::Element { tag: tag.clone(), attrs }, parent);
            if RAW_TEXT.contains(&tag.as_str()) {
                let close = format!("</{}", tag);
                let end = find(i, &close).unwrap_or(n);
                let raw: String = chars[i..end].iter().collect();
                let content = if tag == "title" || tag == "textarea" { decode_entities(&raw) } else { raw };
                if !content.is_empty() {
                    dom.add(NodeData::Text(content), id);
                }
                i = find(end, ">").map(|j| j + 1).unwrap_or(n);
            } else if !self_closing && !VOID.contains(&tag.as_str()) {
                stack.push(id);
            }
        } else {
            text.push(c);
            i += 1;
        }
    }
    flush(&mut dom, &stack, &mut text);
    dom
}

/// Every page has <html>, <head> and <body>, as in real browsers, even when the
/// source leaves them out: metadata goes in the head, everything else in the body.
fn ensure_skeleton(dom: &mut Dom) {
    use crate::dom::NodeData;
    let is_ws = |dom: &Dom, n: NodeId| matches!(&dom.nodes[n].data, NodeData::Text(t) if t.trim().is_empty());
    let el = |tag: &str| NodeData::Element { tag: tag.to_string(), attrs: Vec::new() };
    let root_kids = dom.nodes[Dom::ROOT].children.clone();
    let html = match root_kids.iter().copied().find(|&n| dom.tag(n) == Some("html")) {
        Some(h) => h,
        None => {
            // Wrap everything except a leading doctype/comment in a new <html>.
            let h = dom.create(el("html"));
            for n in root_kids {
                if matches!(dom.nodes[n].data, NodeData::Element { .. } | NodeData::Text(_)) {
                    dom.detach(n);
                    dom.insert(h, n, None);
                }
            }
            dom.insert(Dom::ROOT, h, None);
            h
        }
    };
    let kids = dom.nodes[html].children.clone();
    let has_head = kids.iter().any(|&n| dom.tag(n) == Some("head"));
    let body = kids.iter().copied().find(|&n| matches!(dom.tag(n), Some("body" | "frameset")));
    if has_head && body.is_some() {
        return;
    }
    let head = match kids.iter().copied().find(|&n| dom.tag(n) == Some("head")) {
        Some(h) => h,
        None => {
            let h = dom.create(el("head"));
            dom.insert(html, h, kids.first().copied());
            h
        }
    };
    let body = match body {
        Some(b) => b,
        None => {
            let b = dom.create(el("body"));
            dom.insert(html, b, None);
            b
        }
    };
    // Leading metadata belongs in the head; from the first real content on, the body.
    let mut in_body = false;
    for n in kids {
        if n == head || n == body {
            continue;
        }
        let meta = matches!(dom.tag(n), Some("title" | "meta" | "link" | "style" | "script" | "base" | "noscript" | "template"));
        if !in_body && (meta || is_ws(dom, n)) {
            if !is_ws(dom, n) {
                dom.detach(n);
                dom.insert(head, n, None);
            }
            continue;
        }
        in_body = true;
        dom.detach(n);
        dom.insert(body, n, None);
    }
}

fn implied_closes(dom: &Dom, stack: &mut Vec<NodeId>, tag: &str) {
    let close_nearest = |stack: &mut Vec<NodeId>, target: &[&str], barrier: &[&str]| {
        for pos in (1..stack.len()).rev() {
            let t = dom.tag(stack[pos]).unwrap_or("");
            if target.contains(&t) {
                stack.truncate(pos);
                return;
            }
            if barrier.contains(&t) {
                return;
            }
        }
    };
    if CLOSES_P.contains(&tag) {
        close_nearest(stack, &["p"], &["div", "section", "article", "td", "li", "body", "button"]);
    }
    match tag {
        "li" => close_nearest(stack, &["li"], &["ul", "ol"]),
        "dt" | "dd" => close_nearest(stack, &["dt", "dd"], &["dl"]),
        "tr" => close_nearest(stack, &["tr"], &["table", "tbody", "thead", "tfoot"]),
        "td" | "th" => close_nearest(stack, &["td", "th"], &["tr", "table"]),
        "option" => close_nearest(stack, &["option"], &["select"]),
        _ => {}
    }
}

fn flush(dom: &mut Dom, stack: &[NodeId], text: &mut String) {
    if !text.is_empty() {
        let decoded = decode_entities(text);
        let parent = *stack.last().unwrap();
        // Text next to text (split by a comment or a stray end tag) is one text node.
        match dom.nodes[parent].children.last().copied() {
            Some(last) if matches!(dom.nodes[last].data, NodeData::Text(_)) => {
                if let NodeData::Text(t) = &mut dom.nodes[last].data {
                    t.push_str(&decoded);
                }
            }
            _ => {
                dom.add(NodeData::Text(decoded), parent);
            }
        }
        text.clear();
    }
}

/// Parse `tagname attr="v" ...>` starting just after '<'.
fn parse_tag(c: &[char], mut i: usize) -> (String, Vec<(String, String)>, bool, usize) {
    let n = c.len();
    let mut tag = String::new();
    while i < n && !c[i].is_whitespace() && c[i] != '>' && c[i] != '/' {
        tag.push(c[i].to_ascii_lowercase());
        i += 1;
    }
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        while i < n && c[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        if c[i] == '>' {
            i += 1;
            break;
        }
        if c[i] == '/' {
            self_closing = true;
            i += 1;
            continue;
        }
        let mut name = String::new();
        while i < n && !c[i].is_whitespace() && !matches!(c[i], '=' | '>' | '/') {
            name.push(c[i].to_ascii_lowercase());
            i += 1;
        }
        while i < n && c[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < n && c[i] == '=' {
            i += 1;
            while i < n && c[i].is_whitespace() {
                i += 1;
            }
            if i < n && (c[i] == '"' || c[i] == '\'') {
                let q = c[i];
                i += 1;
                while i < n && c[i] != q {
                    value.push(c[i]);
                    i += 1;
                }
                i += 1;
            } else {
                while i < n && !c[i].is_whitespace() && c[i] != '>' {
                    value.push(c[i]);
                    i += 1;
                }
            }
        }
        if !name.is_empty() {
            // The first occurrence of an attribute wins, as in browsers.
            if !attrs.iter().any(|(k, _)| *k == name) {
                attrs.push((name, decode_entities(&value)));
            }
        } else {
            i += 1; // skip junk character
        }
    }
    attrs.shrink_to_fit();
    (tag, attrs, self_closing, i)
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let end = rest[1..].find(|c: char| !(c.is_alphanumeric() || c == '#')).map(|e| e + 1).unwrap_or(rest.len());
        let name = &rest[1..end];
        let decoded = if let Some(num) = name.strip_prefix('#') {
            let v = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                num.parse().ok()
            };
            v.and_then(char::from_u32)
        } else {
            named_entity(name)
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &rest[end..];
                if rest.starts_with(';') {
                    rest = &rest[1..];
                }
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'',
        "nbsp" => '\u{a0}', "copy" => '©', "reg" => '®', "trade" => '™',
        "mdash" => '—', "ndash" => '–', "hellip" => '…', "middot" => '·', "bull" => '•',
        "lsquo" => '‘', "rsquo" => '’', "ldquo" => '“', "rdquo" => '”',
        "laquo" => '«', "raquo" => '»', "times" => '×', "deg" => '°', "larr" => '←', "rarr" => '→',
        "uarr" => '↑', "darr" => '↓', "sect" => '§', "para" => '¶', "euro" => '€', "pound" => '£',
        "cent" => '¢', "yen" => '¥', "frac12" => '½', "eacute" => 'é', "egrave" => 'è', "ntilde" => 'ñ',
        _ => return None,
    })
}


/// The tree in the html5lib-tests text format (`| <html>`, `|   "text"`), used by the parser
/// tests and the conformance runner.
pub fn dump(dom: &Dom) -> String {
    fn walk(dom: &Dom, id: NodeId, depth: usize, out: &mut String) {
        for &c in &dom.nodes[id].children {
            let pad = "  ".repeat(depth);
            match &dom.nodes[c].data {
                NodeData::Element { tag, attrs } => {
                    out.push_str(&format!("| {pad}<{tag}>\n"));
                    let mut a: Vec<&(String, String)> = attrs.iter().collect();
                    a.sort();
                    for (k, v) in a {
                        out.push_str(&format!("| {pad}  {k}=\"{v}\"\n"));
                    }
                    walk(dom, c, depth + 1, out);
                }
                NodeData::Text(t) => out.push_str(&format!("| {pad}\"{t}\"\n")),
                NodeData::Document => {}
            }
        }
    }
    let mut out = String::new();
    walk(dom, Dom::ROOT, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(src: &str) -> String {
        dump(&parse(src))
    }

    fn body(src: &str) -> String {
        // Just the part inside <body>, without the indentation of html/body.
        let t = tree(src);
        let lines: Vec<&str> = t.lines().collect();
        let start = lines.iter().position(|l| l.trim() == "|   <body>").expect("has body") + 1;
        lines[start..].iter().map(|l| format!("| {}", &l[6..])).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn skeleton_is_added() {
        assert_eq!(tree("hi"), "| <html>\n|   <head>\n|   <body>\n|     \"hi\"\n");
        assert_eq!(tree(""), "| <html>\n|   <head>\n|   <body>\n");
    }

    #[test]
    fn head_elements_go_in_head() {
        let t = tree("<title>T</title><p>x");
        assert!(t.contains("|   <head>\n|     <title>\n|       \"T\"\n|   <body>\n|     <p>\n|       \"x\""), "{t}");
    }

    #[test]
    fn attributes_quoted_unquoted_and_bare() {
        let b = body(r#"<a href="x y" id=one class='two' hidden>t</a>"#);
        assert_eq!(b, "| <a>\n|   class=\"two\"\n|   hidden=\"\"\n|   href=\"x y\"\n|   id=\"one\"\n|   \"t\"");
    }

    #[test]
    fn tag_and_attribute_names_are_lower_case() {
        assert_eq!(body("<DIV CLASS=A>x</DIV>"), "| <div>\n|   class=\"A\"\n|   \"x\"");
    }

    #[test]
    fn entities_named_and_numeric() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &quot;"), "a & b <c> \"");
        assert_eq!(decode_entities("&#169; &#x263A; &#X263a;"), "© ☺ ☺");
        assert_eq!(decode_entities("&nbsp;"), "\u{a0}");
        // Unknown names stay as written.
        assert_eq!(decode_entities("&bogus; & alone"), "&bogus; & alone");
        assert_eq!(body("<p title=\"a&amp;b\">&copy;</p>"), "| <p>\n|   title=\"a&b\"\n|   \"©\"");
    }

    #[test]
    fn void_elements_have_no_children() {
        assert_eq!(body("<br>a<img src=x>b<br/>c"), "| <br>\n| \"a\"\n| <img>\n|   src=\"x\"\n| \"b\"\n| <br>\n| \"c\"");
    }

    #[test]
    fn raw_text_is_not_parsed() {
        let t = tree("<script>if (a<b && c>d) x='</p>'</script>");
        assert!(t.contains("\"if (a<b && c>d) x='</p>'\""), "{t}");
        let b = body("<textarea><b>bold</b></textarea>");
        assert_eq!(b, "| <textarea>\n|   \"<b>bold</b>\"");
    }

    #[test]
    fn implied_end_tags() {
        assert_eq!(body("<p>one<p>two"), "| <p>\n|   \"one\"\n| <p>\n|   \"two\"");
        assert_eq!(body("<ul><li>a<li>b</ul>"), "| <ul>\n|   <li>\n|     \"a\"\n|   <li>\n|     \"b\"");
        assert_eq!(body("<p>a<div>b</div>"), "| <p>\n|   \"a\"\n| <div>\n|   \"b\"");
    }

    #[test]
    fn table_cells_and_rows_close_each_other() {
        let b = body("<table><tr><td>1<td>2<tr><td>3</table>");
        assert!(b.matches("<td>").count() == 3 && b.matches("<tr>").count() == 2, "{b}");
        // The second cell is a sibling of the first, not inside it.
        assert!(!b.contains("|       <td>\n|         \"1\"\n|         <td>"), "{b}");
    }

    #[test]
    fn unclosed_and_stray_tags() {
        assert_eq!(body("<div><span>x"), "| <div>\n|   <span>\n|     \"x\"");
        assert_eq!(body("a</span>b"), "| \"ab\"");
    }

    #[test]
    fn comments_and_doctype_are_skipped() {
        assert_eq!(body("<!DOCTYPE html>a<!-- note -->b"), "| \"ab\"");
    }

    #[test]
    fn fragment_has_no_skeleton() {
        assert_eq!(dump(&parse_fragment("<b>x</b>")), "| <b>\n|   \"x\"\n");
    }
}
