//! HTML parsing entry points. The work is done by `tokenizer` and `treebuilder`, which follow
//! the HTML standard's parsing algorithm.

use crate::dom::Dom;

pub fn parse(src: &str) -> Dom {
    crate::treebuilder::TreeBuilder::parse_document(src, true)
}

/// Parse markup as the contents of a `<body>` (innerHTML and friends): the result's root
/// holds the parsed nodes, with no html/head/body around them.
pub fn parse_fragment(src: &str) -> Dom {
    crate::treebuilder::TreeBuilder::parse_fragment(src, "body", crate::dom::Ns::Html, true)
}

pub use crate::treebuilder::dump;

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
        assert_eq!(body("a &amp; b &lt;c&gt; &quot; &#169; &#x263A; &bogus; & alone"), "| \"a & b <c> \" © ☺ &bogus; & alone\"");
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
    fn comments_and_doctype_are_kept() {
        assert_eq!(tree("<!DOCTYPE html>a<!-- note -->b"), "| <!DOCTYPE html>\n| <html>\n|   <head>\n|   <body>\n|     \"a\"\n|     <!--  note  -->\n|     \"b\"\n");
    }

    #[test]
    fn fragment_has_no_skeleton() {
        assert_eq!(dump(&parse_fragment("<b>x</b>")), "| <b>\n|   \"x\"\n");
    }
}
