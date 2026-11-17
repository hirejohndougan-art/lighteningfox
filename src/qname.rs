//! Namespaced names for `createElementNS`, `createAttributeNS` and `setAttributeNS`: the DOM
//! spec's "validate and extract" and the name checks behind it (the current, relaxed rules:
//! https://dom.spec.whatwg.org/#validate-and-extract).

pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

fn bad_in_name(c: char, extra: &[char]) -> bool {
    matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' ' | '\0' | '/' | '>') || extra.contains(&c)
}

/// A valid namespace prefix: not empty, no ASCII whitespace, NULL, `/` or `>`.
pub fn valid_prefix(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| bad_in_name(c, &[]))
}

/// A valid attribute local name: not empty, no ASCII whitespace, NULL, `/`, `=` or `>`.
pub fn valid_attribute_local_name(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| bad_in_name(c, &['=']))
}

/// A valid element local name (what `createElement` accepts too).
pub fn valid_element_local_name(s: &str) -> bool {
    let mut cs = s.chars();
    let Some(first) = cs.next() else { return false };
    if first.is_ascii_alphabetic() {
        return !s.chars().any(|c| bad_in_name(c, &[]));
    }
    if !(first == ':' || first == '_' || first as u32 >= 0x80) {
        return false;
    }
    cs.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_') || c as u32 >= 0x80)
}

/// A namespaced name, split and checked.
#[derive(Debug, PartialEq, Eq)]
pub struct QName {
    pub ns: Option<String>,
    pub prefix: Option<String>,
    pub local: String,
}

/// "Validate and extract": `Err` holds the DOMException name to throw.
pub fn validate_and_extract(ns: Option<&str>, qualified: &str, attribute: bool) -> Result<QName, &'static str> {
    let ns = ns.filter(|n| !n.is_empty());
    let (prefix, local) = match qualified.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, qualified),
    };
    if prefix.is_some_and(|p| !valid_prefix(p)) {
        return Err("InvalidCharacterError");
    }
    let ok = if attribute { valid_attribute_local_name(local) } else { valid_element_local_name(local) };
    if !ok {
        return Err("InvalidCharacterError");
    }
    if prefix.is_some() && ns.is_none() {
        return Err("NamespaceError");
    }
    if prefix == Some("xml") && ns != Some(XML_NS) {
        return Err("NamespaceError");
    }
    let xmlns = qualified == "xmlns" || prefix == Some("xmlns");
    if xmlns != (ns == Some(XMLNS_NS)) {
        return Err("NamespaceError");
    }
    Ok(QName { ns: ns.map(str::to_string), prefix: prefix.map(str::to_string), local: local.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(ns: Option<&str>, q: &str) -> Option<&'static str> {
        validate_and_extract(ns, q, false).err()
    }

    #[test]
    fn element_names() {
        // Cases from WPT dom/nodes/Document-createElementNS.js.
        for ok in ["foo", "f1oo", "\u{BC6}foo", "\u{37E}foo", "f}oo", "foo}", "\u{FFFF}foo", "f<oo", "xml", "xmlfoo", "null", "undefined"] {
            assert_eq!(err(None, ok), None, "{ok}");
        }
        for (q, e) in [
            ("1foo", "InvalidCharacterError"),
            ("}foo", "InvalidCharacterError"),
            ("<foo", "InvalidCharacterError"),
            ("foo>", "InvalidCharacterError"),
            ("^^", "InvalidCharacterError"),
            ("fo o", "InvalidCharacterError"),
            ("-foo", "InvalidCharacterError"),
            (".foo", "InvalidCharacterError"),
            (":foo", "InvalidCharacterError"),
            ("foo:", "InvalidCharacterError"),
            (":", "InvalidCharacterError"),
            ("", "InvalidCharacterError"),
            ("f:oo", "NamespaceError"),
            ("f:o:o", "NamespaceError"),
            ("xmlns", "NamespaceError"),
            ("xml:foo", "NamespaceError"),
            ("xmlns:foo", "NamespaceError"),
        ] {
            assert_eq!(err(None, q), Some(e), "{q}");
        }
        assert_eq!(err(Some(XMLNS_NS), "foo:xmlns"), Some("NamespaceError"));
        assert_eq!(err(Some(XMLNS_NS), "xmlns"), None);
        assert_eq!(err(Some(XMLNS_NS), "xmlns:foo"), None);
        assert_eq!(err(Some(XML_NS), "xml:foo"), None);
        assert_eq!(err(Some("http://example.com/"), "xml:foo"), Some("NamespaceError"));
        let q = validate_and_extract(Some("test"), "html:SPAN", false).unwrap();
        assert_eq!((q.ns.as_deref(), q.prefix.as_deref(), q.local.as_str()), (Some("test"), Some("html"), "SPAN"));
        assert_eq!(validate_and_extract(Some(""), "a", false).unwrap().ns, None);
    }

    #[test]
    fn attribute_names() {
        assert!(validate_and_extract(None, "1foo", true).is_ok());
        assert_eq!(validate_and_extract(None, "a=b", true).err(), Some("InvalidCharacterError"));
    }
}
