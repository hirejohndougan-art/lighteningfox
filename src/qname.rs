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

/// The natives behind `createElementNS`, the `…AttributeNS` methods, `getElementsByTagNameNS`
/// and the namespace lookups in `ns.js`.
pub mod script {
    use crate::dom::{NodeData, NodeId, Ns, NsName};
    use crate::js::{arg_str, with_dom, with_dom_mut};
    use boa_engine::{object::builtins::JsArray, Context, JsResult, JsString, JsValue};

    pub type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

    pub const NATIVES: &[(&str, usize, Native)] = &[
        ("__createNS", 2, n_create_ns),
        ("__validName", 1, n_valid_name),
        ("__qname", 3, n_qname),
        ("__attrsNS", 1, n_attrs_ns),
        ("__markAttrNS", 3, n_mark_attr_ns),
        ("__byTagNS", 3, n_by_tag_ns),
    ];

    fn opt_str(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<Option<String>> {
        match a.get(i) {
            Some(v) if !v.is_null_or_undefined() => Ok(Some(v.to_string(ctx)?.to_std_string_escaped())),
            _ => Ok(None),
        }
    }

    fn node(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<Option<NodeId>> {
        let n = a.get(i).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(-1.0);
        Ok((n >= 0.0).then_some(n as NodeId).filter(|&n| with_dom(|d| n < d.nodes.len())))
    }

    fn js_opt(s: Option<String>) -> JsValue {
        s.map_or(JsValue::null(), |s| JsValue::from(JsString::from(s)))
    }

    /// `__createNS(namespace or null, qualifiedName)`: the element's id, or the name of the
    /// DOMException to throw ("validate and extract" failed).
    fn n_create_ns(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let ns = opt_str(a, 0, ctx)?;
        let qualified = arg_str(a, 1, ctx)?;
        let q = match super::validate_and_extract(ns.as_deref(), &qualified, false) {
            Ok(q) => q,
            Err(e) => return Ok(JsValue::from(JsString::from(e))),
        };
        let kind = match q.ns.as_deref() {
            Some("http://www.w3.org/1999/xhtml") => Ns::Html,
            Some("http://www.w3.org/2000/svg") => Ns::Svg,
            Some("http://www.w3.org/1998/Math/MathML") => Ns::MathMl,
            _ => Ns::Other,
        };
        let id = with_dom_mut(|d| {
            let id = d.create(NodeData::Element { tag: q.local, attrs: Vec::new() });
            if kind != Ns::Html {
                d.ns.insert(id, kind);
            }
            if kind == Ns::Other || q.prefix.is_some() {
                d.ns_names.insert(id, NsName { uri: if kind == Ns::Other { q.ns } else { None }, prefix: q.prefix });
            }
            id
        });
        Ok(JsValue::from(id as f64))
    }

    /// `__validName(name)`: whether `createElement` accepts it as an element's local name.
    fn n_valid_name(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        Ok(JsValue::from(super::valid_element_local_name(&arg_str(a, 0, ctx)?)))
    }

    /// `__qname(namespace, qualifiedName, forAttribute)`: [namespace, prefix, localName], or
    /// the DOMException name.
    fn n_qname(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let ns = opt_str(a, 0, ctx)?;
        let qualified = arg_str(a, 1, ctx)?;
        let attribute = a.get(2).is_some_and(|v| v.to_boolean());
        Ok(match super::validate_and_extract(ns.as_deref(), &qualified, attribute) {
            Ok(q) => JsArray::from_iter([js_opt(q.ns), js_opt(q.prefix), JsValue::from(JsString::from(q.local))], ctx).into(),
            Err(e) => JsValue::from(JsString::from(e)),
        })
    }

    /// `__attrsNS(id)`: [[qualifiedName, namespace or null, value], …] in order.
    fn n_attrs_ns(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = node(a, 0, ctx)? else { return Ok(JsArray::new(ctx)?.into()) };
        let list: Vec<(String, Option<String>, String)> = with_dom(|d| match &d.nodes[id].data {
            NodeData::Element { attrs, .. } => {
                let marks = d.attr_ns.get(&id);
                attrs
                    .iter()
                    .map(|(k, v)| (k.clone(), marks.and_then(|m| m.iter().find(|(q, _)| q == k)).map(|(_, n)| n.clone()), v.clone()))
                    .collect()
            }
            _ => Vec::new(),
        });
        let rows: Vec<JsValue> = list
            .into_iter()
            .map(|(k, n, v)| JsArray::from_iter([JsValue::from(JsString::from(k)), js_opt(n), JsValue::from(JsString::from(v))], ctx).into())
            .collect();
        Ok(JsArray::from_iter(rows, ctx).into())
    }

    /// `__markAttrNS(id, qualifiedName, namespace or null)`: record (or clear) an attribute's
    /// namespace.
    fn n_mark_attr_ns(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = node(a, 0, ctx)? else { return Ok(JsValue::undefined()) };
        let qualified = arg_str(a, 1, ctx)?;
        let ns = opt_str(a, 2, ctx)?;
        if ns.is_none() && with_dom(|d| d.attr_ns.get(&id).is_none_or(|m| m.iter().all(|(q, _)| *q != qualified))) {
            return Ok(JsValue::undefined());
        }
        with_dom_mut(|d| {
            let list = d.attr_ns.entry(id).or_default();
            list.retain(|(q, _)| *q != qualified);
            if let Some(ns) = ns {
                list.push((qualified, ns));
            }
            if list.is_empty() {
                d.attr_ns.remove(&id);
            }
        });
        Ok(JsValue::undefined())
    }

    /// `__byTagNS(root, namespace or null or "*", localName or "*")`: descendant elements, in
    /// tree order (getElementsByTagNameNS).
    fn n_by_tag_ns(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(root) = node(a, 0, ctx)? else { return Ok(JsArray::new(ctx)?.into()) };
        let ns = opt_str(a, 1, ctx)?.filter(|n| !n.is_empty());
        let local = arg_str(a, 2, ctx)?;
        let ids = with_dom(|d| {
            let mut out = Vec::new();
            let mut stack: Vec<NodeId> = d.nodes[root].children.iter().rev().copied().collect();
            while let Some(n) = stack.pop() {
                if let NodeData::Element { tag, .. } = &d.nodes[n].data {
                    let ns_ok = ns.as_deref() == Some("*") || d.namespace_uri(n) == ns;
                    if ns_ok && (local == "*" || *tag == local) {
                        out.push(JsValue::from(n as f64));
                    }
                }
                stack.extend(d.nodes[n].children.iter().rev());
            }
            out
        });
        Ok(JsArray::from_iter(ids, ctx).into())
    }
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
