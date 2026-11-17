//! HTML tree construction (§13.2.6 of the HTML standard): turns tokens into a DOM, with every
//! insertion mode, the adoption agency algorithm for misnested formatting tags, foster
//! parenting for stray content in tables, `<template>`, and SVG/MathML inside HTML.

use crate::dom::{Dom, NodeData, NodeId, Ns, Quirks};
use crate::tokenizer::{Doctype, State, Tag, Token, Tokenizer};

/// How deeply the parser nests elements; deeper ones are added as siblings (Chrome uses 512).
pub const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

enum Afe {
    Marker,
    El(NodeId, Tag),
}

pub struct TreeBuilder {
    tk: Tokenizer,
    pub dom: Dom,
    open: Vec<NodeId>,
    afe: Vec<Afe>,
    mode: Mode,
    orig_mode: Mode,
    tmpl_modes: Vec<Mode>,
    head: Option<NodeId>,
    form: Option<NodeId>,
    frameset_ok: bool,
    foster: bool,
    pub scripting: bool,
    pending_text: String,
    context: Option<NodeId>,
    skip_lf: bool,
    stopped: bool,
    /// Pause at each `</script>` so the caller can run it (a live page with JavaScript).
    pub pause_at_scripts: bool,
    pending_script: Option<NodeId>,
    /// Where document.write puts text while a parser-inserted script runs.
    write_point: Option<usize>,
    /// `<template shadowrootmode>` makes a shadow root (whole documents, not innerHTML).
    pub allow_declarative_shadow: bool,
}

const SPECIAL_HTML: &[&str] = &[
    "address", "applet", "area", "article", "aside", "base", "basefont", "bgsound", "blockquote", "body", "br", "button", "caption",
    "center", "col", "colgroup", "dd", "details", "dir", "div", "dl", "dt", "embed", "fieldset", "figcaption", "figure", "footer", "form",
    "frame", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hgroup", "hr", "html", "iframe", "img", "input", "keygen",
    "li", "link", "listing", "main", "marquee", "menu", "meta", "nav", "noembed", "noframes", "noscript", "object", "ol", "p", "param",
    "plaintext", "pre", "script", "search", "section", "select", "source", "style", "summary", "table", "tbody", "td", "template",
    "textarea", "tfoot", "th", "thead", "title", "tr", "track", "ul", "wbr", "xmp",
];

const FORMATTING: &[&str] = &["a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u"];
const HEADINGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];
const IMPLIED_END: &[&str] = &["dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc"];
const IMPLIED_END_ALL: &[&str] = &[
    "caption", "colgroup", "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc", "tbody", "td", "tfoot", "th", "thead", "tr",
];
const HEAD_TAGS: &[&str] = &["base", "basefont", "bgsound", "link", "meta", "noframes", "script", "style", "template", "title"];

fn is_ws_str(s: &str) -> bool {
    s.chars().all(|c| matches!(c, '\t' | '\n' | '\u{0C}' | '\r' | ' '))
}

/// Split a character run into pieces that are all whitespace, all NUL, or neither, since
/// several modes treat those differently.
fn split_text(s: &str) -> Vec<String> {
    let kind = |c: char| if c == '\0' { 0 } else if matches!(c, '\t' | '\n' | '\u{0C}' | '\r' | ' ') { 1 } else { 2 };
    let mut out: Vec<String> = Vec::new();
    let mut last = -1;
    for c in s.chars() {
        let k = kind(c);
        if k == last {
            out.last_mut().expect("run").push(c);
        } else {
            out.push(c.to_string());
            last = k;
        }
    }
    out
}

const SVG_TAGS: &[&str] = &[
    "altGlyph", "altGlyphDef", "altGlyphItem", "animateColor", "animateMotion", "animateTransform", "clipPath", "feBlend", "feColorMatrix",
    "feComponentTransfer", "feComposite", "feConvolveMatrix", "feDiffuseLighting", "feDisplacementMap", "feDistantLight", "feDropShadow",
    "feFlood", "feFuncA", "feFuncB", "feFuncG", "feFuncR", "feGaussianBlur", "feImage", "feMerge", "feMergeNode", "feMorphology", "feOffset",
    "fePointLight", "feSpecularLighting", "feSpotLight", "feTile", "feTurbulence", "foreignObject", "glyphRef", "linearGradient",
    "radialGradient", "textPath",
];

const SVG_ATTRS: &[&str] = &[
    "attributeName", "attributeType", "baseFrequency", "baseProfile", "calcMode", "clipPathUnits", "diffuseConstant", "edgeMode",
    "filterUnits", "glyphRef", "gradientTransform", "gradientUnits", "kernelMatrix", "kernelUnitLength", "keyPoints", "keySplines",
    "keyTimes", "lengthAdjust", "limitingConeAngle", "markerHeight", "markerUnits", "markerWidth", "maskContentUnits", "maskUnits",
    "numOctaves", "pathLength", "patternContentUnits", "patternTransform", "patternUnits", "pointsAtX", "pointsAtY", "pointsAtZ",
    "preserveAlpha", "preserveAspectRatio", "primitiveUnits", "refX", "refY", "repeatCount", "repeatDur", "requiredExtensions",
    "requiredFeatures", "specularConstant", "specularExponent", "spreadMethod", "startOffset", "stdDeviation", "stitchTiles",
    "surfaceScale", "systemLanguage", "tableValues", "targetX", "targetY", "textLength", "viewBox", "viewTarget", "xChannelSelector",
    "yChannelSelector", "zoomAndPan",
];

/// Attributes that live in the XLink, XML or XMLNS namespaces on foreign elements. They keep
/// their `prefix:local` names here; the namespace is implied by the prefix.
pub const FOREIGN_ATTRS: &[&str] = &[
    "xlink:actuate", "xlink:arcrole", "xlink:href", "xlink:role", "xlink:show", "xlink:title", "xlink:type", "xml:lang", "xml:space",
    "xmlns", "xmlns:xlink",
];

fn adjust_svg_tag(name: &str) -> String {
    SVG_TAGS.iter().find(|t| t.eq_ignore_ascii_case(name)).map_or_else(|| name.to_string(), |t| t.to_string())
}

fn adjust_attrs(tag: &mut Tag, ns: Ns) {
    for (k, _) in tag.attrs.iter_mut() {
        if ns == Ns::Svg {
            if let Some(a) = SVG_ATTRS.iter().find(|a| a.to_ascii_lowercase() == *k) {
                *k = a.to_string();
            }
        }
        if ns == Ns::MathMl && k == "definitionurl" {
            *k = "definitionURL".into();
        }
    }
}

const QUIRKY_PUBLIC_PREFIXES: &[&str] = &[
    "+//silmaril//dtd html pro v0r11 19970101//", "-//as//dtd html 3.0 aswedit + extensions//",
    "-//advasoft ltd//dtd html 3.0 aswedit + extensions//", "-//ietf//dtd html 2.0 level 1//", "-//ietf//dtd html 2.0 level 2//",
    "-//ietf//dtd html 2.0 strict level 1//", "-//ietf//dtd html 2.0 strict level 2//", "-//ietf//dtd html 2.0 strict//",
    "-//ietf//dtd html 2.0//", "-//ietf//dtd html 2.1e//", "-//ietf//dtd html 3.0//", "-//ietf//dtd html 3.2 final//",
    "-//ietf//dtd html 3.2//", "-//ietf//dtd html 3//", "-//ietf//dtd html level 0//", "-//ietf//dtd html level 1//",
    "-//ietf//dtd html level 2//", "-//ietf//dtd html level 3//", "-//ietf//dtd html strict level 0//",
    "-//ietf//dtd html strict level 1//", "-//ietf//dtd html strict level 2//", "-//ietf//dtd html strict level 3//",
    "-//ietf//dtd html strict//", "-//ietf//dtd html//", "-//metrius//dtd metrius presentational//",
    "-//microsoft//dtd internet explorer 2.0 html strict//", "-//microsoft//dtd internet explorer 2.0 html//",
    "-//microsoft//dtd internet explorer 2.0 tables//", "-//microsoft//dtd internet explorer 3.0 html strict//",
    "-//microsoft//dtd internet explorer 3.0 html//", "-//microsoft//dtd internet explorer 3.0 tables//",
    "-//netscape comm. corp.//dtd html//", "-//netscape comm. corp.//dtd strict html//", "-//o'reilly and associates//dtd html 2.0//",
    "-//o'reilly and associates//dtd html extended 1.0//", "-//o'reilly and associates//dtd html extended relaxed 1.0//",
    "-//sq//dtd html 2.0 hotmetal + extensions//",
    "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
    "-//softquad//dtd hotmetal pro 4.0::19970916::extensions to html 4.0//", "-//spyglass//dtd html 2.0 extended//",
    "-//sun microsystems corp.//dtd hotjava html//", "-//sun microsystems corp.//dtd hotjava strict html//",
    "-//w3c//dtd html 3 1995-03-24//", "-//w3c//dtd html 3.2 draft//", "-//w3c//dtd html 3.2 final//", "-//w3c//dtd html 3.2//",
    "-//w3c//dtd html 3.2s draft//", "-//w3c//dtd html 4.0 frameset//", "-//w3c//dtd html 4.0 transitional//",
    "-//w3c//dtd html experimental 19960712//", "-//w3c//dtd html experimental 970421//", "-//w3c//dtd w3 html//",
    "-//w3o//dtd w3 html 3.0//", "-//webtechs//dtd mozilla html 2.0//", "-//webtechs//dtd mozilla html//",
];

fn doctype_quirks(d: &Doctype) -> Quirks {
    let public = d.public_id.as_deref().map(str::to_ascii_lowercase);
    let system = d.system_id.as_deref().map(str::to_ascii_lowercase);
    let p = public.as_deref().unwrap_or("");
    let html401 = p.starts_with("-//w3c//dtd html 4.01 frameset//") || p.starts_with("-//w3c//dtd html 4.01 transitional//");
    if d.force_quirks
        || d.name.as_deref() != Some("html")
        || matches!(p, "-//w3o//dtd w3 html strict 3.0//en//" | "-/w3c/dtd html 4.0 transitional/en" | "html")
        || system.as_deref() == Some("http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd")
        || QUIRKY_PUBLIC_PREFIXES.iter().any(|q| p.starts_with(q))
        || (system.is_none() && html401)
    {
        Quirks::Quirks
    } else if p.starts_with("-//w3c//dtd xhtml 1.0 frameset//") || p.starts_with("-//w3c//dtd xhtml 1.0 transitional//") || (system.is_some() && html401) {
        Quirks::Limited
    } else {
        Quirks::NoQuirks
    }
}

impl TreeBuilder {
    pub fn new(src: &str) -> TreeBuilder {
        TreeBuilder {
            tk: Tokenizer::new(src),
            dom: Dom::new(),
            open: Vec::new(),
            afe: Vec::new(),
            mode: Mode::Initial,
            orig_mode: Mode::Initial,
            tmpl_modes: Vec::new(),
            head: None,
            form: None,
            frameset_ok: true,
            foster: false,
            scripting: true,
            pending_text: String::new(),
            context: None,
            skip_lf: false,
            stopped: false,
            pause_at_scripts: false,
            pending_script: None,
            write_point: None,
            allow_declarative_shadow: true,
        }
    }

    /// Parse a whole document.
    pub fn parse_document(src: &str, scripting: bool) -> Dom {
        let mut tb = TreeBuilder::new(src);
        tb.scripting = scripting;
        tb.run();
        fill_selectedcontent(&mut tb.dom);
        tb.dom
    }

    /// The fragment parsing algorithm (innerHTML): `context` is the element the markup goes
    /// into. Returns a DOM whose root holds the parsed nodes.
    pub fn parse_fragment(src: &str, context: &str, context_ns: Ns, scripting: bool) -> Dom {
        let mut tb = TreeBuilder::new(src);
        tb.scripting = scripting;
        tb.allow_declarative_shadow = false;
        let ctx = tb.dom.create(NodeData::Element { tag: context.to_string(), attrs: Vec::new() });
        if context_ns != Ns::Html {
            tb.dom.ns.insert(ctx, context_ns);
        }
        tb.context = Some(ctx);
        if context_ns == Ns::Html {
            tb.tk.state = match context {
                "title" | "textarea" => State::Rcdata,
                "style" | "xmp" | "iframe" | "noembed" | "noframes" => State::Rawtext,
                "noscript" if scripting => State::Rawtext,
                "script" => State::ScriptData,
                "plaintext" => State::Plaintext,
                _ => State::Data,
            };
        }
        let html = tb.dom.add(NodeData::Element { tag: "html".into(), attrs: Vec::new() }, Dom::ROOT);
        tb.open.push(html);
        if context == "template" && context_ns == Ns::Html {
            tb.tmpl_modes.push(Mode::InTemplate);
        }
        tb.reset_mode();
        tb.run();
        // Move the results from the html element up to the root.
        let mut dom = tb.dom;
        let kids = std::mem::take(&mut dom.nodes[html].children);
        dom.nodes[Dom::ROOT].children.clear();
        for k in kids {
            dom.nodes[k].parent = Some(Dom::ROOT);
            dom.nodes[Dom::ROOT].children.push(k);
        }
        dom.nodes[html].parent = None;
        fill_selectedcontent(&mut dom);
        dom
    }

    fn run(&mut self) {
        while self.pump().is_some() {}
    }

    /// Parse until a script needs to run (returns it; call again after running it) or the
    /// input ends (returns None).
    pub fn pump(&mut self) -> Option<NodeId> {
        if self.stopped {
            return None;
        }
        loop {
            self.tk.allow_cdata = self.adjusted_current().is_some_and(|n| self.dom.ns_of(n) != Ns::Html);
            let t = self.tk.next_token();
            let eof = t == Token::Eof;
            match t {
                Token::Text(s) => {
                    let mut s = s;
                    if self.skip_lf {
                        self.skip_lf = false;
                        if s.starts_with('\n') {
                            s.remove(0);
                        }
                    }
                    for piece in split_text(&s) {
                        self.dispatch(Token::Text(piece));
                    }
                }
                other => {
                    self.skip_lf = false;
                    self.dispatch(other);
                }
            }
            if eof || self.stopped {
                self.stopped = true;
                return None;
            }
            if let Some(s) = self.pending_script.take() {
                return Some(s);
            }
        }
    }

    /// A script is about to run: document.write output goes where the parser stopped.
    pub fn begin_script(&mut self) {
        self.write_point = Some(self.tk.position());
    }

    pub fn end_script(&mut self) {
        self.write_point = None;
    }

    /// document.write while a parser-inserted script runs. Returns false if there's no
    /// insertion point (the caller then treats it as a write after loading).
    pub fn write(&mut self, text: &str) -> bool {
        match self.write_point {
            Some(at) => {
                self.write_point = Some(self.tk.insert(at, text));
                true
            }
            None => false,
        }
    }

    /// The finished document.
    pub fn finish(mut self) -> Dom {
        fill_selectedcontent(&mut self.dom);
        self.dom
    }

    // ------------------------------------------------------------ helpers: the tree

    fn name(&self, id: NodeId) -> &str {
        self.dom.tag(id).unwrap_or("")
    }

    fn is(&self, id: NodeId, name: &str) -> bool {
        self.dom.ns_of(id) == Ns::Html && self.name(id) == name
    }

    fn is_in(&self, id: NodeId, names: &[&str]) -> bool {
        self.dom.ns_of(id) == Ns::Html && names.contains(&self.name(id))
    }

    fn current(&self) -> Option<NodeId> {
        self.open.last().copied()
    }

    fn cur_is(&self, name: &str) -> bool {
        self.current().is_some_and(|c| self.is(c, name))
    }

    fn adjusted_current(&self) -> Option<NodeId> {
        if self.context.is_some() && self.open.len() == 1 { self.context } else { self.current() }
    }

    fn is_special(&self, id: NodeId) -> bool {
        match self.dom.ns_of(id) {
            Ns::Html => SPECIAL_HTML.contains(&self.name(id)),
            Ns::MathMl => matches!(self.name(id), "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"),
            Ns::Svg => matches!(self.name(id), "foreignObject" | "desc" | "title"),
            Ns::Other => false,
        }
    }

    fn mathml_text_point(&self, id: NodeId) -> bool {
        self.dom.ns_of(id) == Ns::MathMl && matches!(self.name(id), "mi" | "mo" | "mn" | "ms" | "mtext")
    }

    fn html_integration_point(&self, id: NodeId) -> bool {
        match self.dom.ns_of(id) {
            Ns::MathMl => {
                self.name(id) == "annotation-xml"
                    && self.dom.attr(id, "encoding").is_some_and(|e| e.eq_ignore_ascii_case("text/html") || e.eq_ignore_ascii_case("application/xhtml+xml"))
            }
            Ns::Svg => matches!(self.name(id), "foreignObject" | "desc" | "title"),
            Ns::Html | Ns::Other => false,
        }
    }

    /// "Has an element in scope", with the extra boundary elements for list item, button
    /// and table scope.
    fn in_scope_with(&self, target: &[&str], extra: &[&str], table: bool) -> bool {
        for &n in self.open.iter().rev() {
            if self.is_in(n, target) {
                return true;
            }
            let ns = self.dom.ns_of(n);
            let name = self.name(n);
            let boundary = if table {
                ns == Ns::Html && matches!(name, "html" | "table" | "template")
            } else {
                match ns {
                    Ns::Html => matches!(name, "applet" | "caption" | "html" | "table" | "td" | "th" | "marquee" | "object" | "template") || extra.contains(&name),
                    Ns::MathMl => matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"),
                    Ns::Svg => matches!(name, "foreignObject" | "desc" | "title"),
                    Ns::Other => false,
                }
            };
            if boundary {
                return false;
            }
        }
        false
    }

    fn in_scope(&self, name: &str) -> bool {
        self.in_scope_with(&[name], &[], false)
    }
    fn in_button_scope(&self, name: &str) -> bool {
        self.in_scope_with(&[name], &["button"], false)
    }
    fn in_list_scope(&self, name: &str) -> bool {
        self.in_scope_with(&[name], &["ol", "ul"], false)
    }
    fn in_table_scope(&self, name: &str) -> bool {
        self.in_scope_with(&[name], &[], true)
    }

    fn has_open(&self, name: &str) -> bool {
        self.open.iter().any(|&n| self.is(n, name))
    }

    /// A template on the stack, or parsing a template's contents (innerHTML on a template).
    fn in_template_ctx(&self) -> bool {
        self.has_open("template") || self.context.is_some_and(|c| self.is(c, "template"))
    }

    fn pop(&mut self) -> Option<NodeId> {
        self.open.pop()
    }

    fn pop_until(&mut self, names: &[&str]) {
        while let Some(n) = self.open.pop() {
            if self.is_in(n, names) {
                break;
            }
        }
    }

    fn generate_implied(&mut self, except: Option<&str>) {
        while let Some(c) = self.current() {
            if self.is_in(c, IMPLIED_END) && Some(self.name(c)) != except {
                self.pop();
            } else {
                break;
            }
        }
    }

    fn generate_implied_all(&mut self) {
        while let Some(c) = self.current() {
            if self.is_in(c, IMPLIED_END_ALL) {
                self.pop();
            } else {
                break;
            }
        }
    }

    fn close_p(&mut self) {
        self.generate_implied(Some("p"));
        self.pop_until(&["p"]);
    }

    fn close_p_if_open(&mut self) {
        if self.in_button_scope("p") {
            self.close_p();
        }
    }

    /// Where a new node goes (§13.2.6.1), including foster parenting out of tables.
    fn insert_place(&self, override_target: Option<NodeId>) -> (NodeId, Option<NodeId>) {
        let target = override_target.or(self.current()).unwrap_or(Dom::ROOT);
        let (parent, before) = if self.foster && self.is_in(target, &["table", "tbody", "tfoot", "thead", "tr"]) {
            let last_template = self.open.iter().rposition(|&n| self.is(n, "template"));
            let last_table = self.open.iter().rposition(|&n| self.is(n, "table"));
            match (last_template, last_table) {
                (Some(t), tb) if tb.is_none_or(|tb| t > tb) => (self.open[t], None),
                (_, None) => (self.open[0], None),
                (_, Some(tb)) => {
                    let table = self.open[tb];
                    match self.dom.nodes[table].parent {
                        Some(p) => (p, Some(table)),
                        None => (self.open[tb - 1], None),
                    }
                }
            }
        } else if self.open.len() > MAX_DEPTH {
            // Past the depth limit, new nodes become siblings instead of children (as in
            // Chrome), so no page can build a tree deep enough to exhaust the stack later.
            (self.dom.nodes[target].parent.unwrap_or(target), None)
        } else {
            (target, None)
        };
        // Inside a template, content goes into its contents fragment.
        if self.is(parent, "template") {
            if let Some(&c) = self.dom.template_contents.get(&parent) {
                return (c, None);
            }
        }
        (parent, before)
    }

    fn create_element(&mut self, tag: &Tag, ns: Ns) -> NodeId {
        let id = self.dom.create(NodeData::Element { tag: tag.name.clone(), attrs: tag.attrs.clone() });
        if ns != Ns::Html {
            self.dom.ns.insert(id, ns);
        }
        if ns == Ns::Html && tag.name == "template" {
            let frag = self.dom.create(NodeData::Fragment);
            self.dom.template_contents.insert(id, frag);
        }
        id
    }

    fn insert_element(&mut self, tag: &Tag, ns: Ns) -> NodeId {
        let (parent, before) = self.insert_place(None);
        let id = self.create_element(tag, ns);
        // A document can't hold a second element (a script may have moved one there);
        // the element is still opened, just not attached.
        let doc_full = matches!(self.dom.nodes[parent].data, NodeData::Document)
            && self.dom.nodes[parent].children.iter().any(|&c| matches!(self.dom.nodes[c].data, NodeData::Element { .. }));
        if !doc_full {
            self.dom.insert(parent, id, before);
        }
        self.open.push(id);
        id
    }

    fn insert_html(&mut self, tag: &Tag) -> NodeId {
        self.insert_element(tag, Ns::Html)
    }

    fn insert_fake(&mut self, name: &str) -> NodeId {
        self.insert_html(&Tag { name: name.into(), attrs: Vec::new(), self_closing: false })
    }

    fn insert_text(&mut self, s: &str) {
        let (parent, before) = self.insert_place(None);
        if matches!(self.dom.nodes[parent].data, NodeData::Document) {
            return;
        }
        let kids = &self.dom.nodes[parent].children;
        let prev = match before {
            Some(b) => kids.iter().position(|&k| k == b).and_then(|i| i.checked_sub(1)).map(|i| kids[i]),
            None => kids.last().copied(),
        };
        if let Some(p) = prev {
            if let NodeData::Text(t) = &mut self.dom.nodes[p].data {
                t.push_str(s);
                return;
            }
        }
        let id = self.dom.create(NodeData::Text(s.to_string()));
        self.dom.insert(parent, id, before);
    }

    fn insert_node_at(&mut self, data: NodeData, parent: Option<NodeId>) {
        let (parent, before) = match parent {
            Some(p) => (p, None),
            None => self.insert_place(None),
        };
        let id = self.dom.create(data);
        self.dom.insert(parent, id, before);
    }

    fn insert_comment(&mut self, t: &Token, parent: Option<NodeId>) {
        let data = match t {
            Token::Comment(c) => NodeData::Comment(c.clone()),
            Token::ProcessingInstruction { target, data } => NodeData::ProcessingInstruction { target: target.clone(), data: data.clone() },
            _ => return,
        };
        self.insert_node_at(data, parent);
    }

    // ------------------------------------------------------------ active formatting elements

    fn afe_index(&self, node: NodeId) -> Option<usize> {
        self.afe.iter().position(|e| matches!(e, Afe::El(n, _) if *n == node))
    }

    fn push_afe(&mut self, node: NodeId, tag: &Tag) {
        // "Noah's Ark": at most three identical entries after the last marker.
        let mut same = Vec::new();
        for (i, e) in self.afe.iter().enumerate().rev() {
            match e {
                Afe::Marker => break,
                Afe::El(_, t) => {
                    if t.name == tag.name && t.attrs.len() == tag.attrs.len() && t.attrs.iter().all(|a| tag.attrs.contains(a)) {
                        same.push(i);
                    }
                }
            }
        }
        if same.len() >= 3 {
            self.afe.remove(*same.last().expect("earliest"));
        }
        self.afe.push(Afe::El(node, tag.clone()));
    }

    fn reconstruct_afe(&mut self) {
        let Some(last) = self.afe.last() else { return };
        let in_open = |e: &Afe| match e {
            Afe::Marker => true,
            Afe::El(n, _) => self.open.contains(n),
        };
        if in_open(last) {
            return;
        }
        let mut i = self.afe.len() - 1;
        while i > 0 {
            if in_open(&self.afe[i - 1]) {
                break;
            }
            i -= 1;
        }
        for j in i..self.afe.len() {
            let tag = match &self.afe[j] {
                Afe::El(_, t) => t.clone(),
                Afe::Marker => continue,
            };
            let n = self.insert_html(&tag);
            self.afe[j] = Afe::El(n, tag);
        }
    }

    fn clear_afe_to_marker(&mut self) {
        while let Some(e) = self.afe.pop() {
            if matches!(e, Afe::Marker) {
                break;
            }
        }
    }

    /// The adoption agency algorithm, for misnested formatting end tags like `<b><i></b></i>`.
    /// Returns false if the caller should handle the token as "any other end tag".
    fn adoption_agency(&mut self, subject: &str) -> bool {
        if let Some(c) = self.current() {
            if self.is(c, subject) && self.afe_index(c).is_none() {
                self.pop();
                return true;
            }
        }
        for _ in 0..8 {
            // The formatting element: the last one with this name after the last marker.
            let mut fe_afe = None;
            for (i, e) in self.afe.iter().enumerate().rev() {
                match e {
                    Afe::Marker => break,
                    Afe::El(n, t) if t.name == subject => {
                        fe_afe = Some((i, *n));
                        break;
                    }
                    _ => {}
                }
            }
            let Some((fe_i, fe)) = fe_afe else { return false };
            let Some(fe_open) = self.open.iter().position(|&n| n == fe) else {
                self.afe.remove(fe_i);
                return true;
            };
            // Scope check against the element itself (another element with the same name
            // could be the one in scope).
            let mut in_scope = false;
            for &n in self.open.iter().rev() {
                if n == fe {
                    in_scope = true;
                    break;
                }
                let ns = self.dom.ns_of(n);
                let name = self.name(n);
                let boundary = match ns {
                    Ns::Html => matches!(name, "applet" | "caption" | "html" | "table" | "td" | "th" | "marquee" | "object" | "template" | "select"),
                    Ns::MathMl => matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"),
                    Ns::Svg => matches!(name, "foreignObject" | "desc" | "title"),
                    Ns::Other => false,
                };
                if boundary {
                    break;
                }
            }
            if !in_scope {
                return true;
            }
            let fb_open = (fe_open + 1..self.open.len()).find(|&i| self.is_special(self.open[i]));
            let Some(fb_open) = fb_open else {
                self.open.truncate(fe_open);
                self.afe.remove(fe_i);
                return true;
            };
            let furthest = self.open[fb_open];
            let common = self.open[fe_open - 1];
            let mut bookmark_after: Option<NodeId> = None;
            let mut node_i = fb_open;
            let mut last_node = furthest;
            let mut inner = 0;
            loop {
                inner += 1;
                node_i -= 1;
                let node = self.open[node_i];
                if node == fe {
                    break;
                }
                if inner > 3 {
                    if let Some(ai) = self.afe_index(node) {
                        self.afe.remove(ai);
                    }
                }
                let Some(ai) = self.afe_index(node) else {
                    self.open.remove(node_i);
                    continue;
                };
                let tag = match &self.afe[ai] {
                    Afe::El(_, t) => t.clone(),
                    Afe::Marker => unreachable!(),
                };
                let new = self.create_element(&tag, Ns::Html);
                self.afe[ai] = Afe::El(new, tag);
                self.open[node_i] = new;
                if last_node == furthest {
                    bookmark_after = Some(new);
                }
                self.dom.insert(new, last_node, None);
                last_node = new;
            }
            let (parent, before) = self.insert_place(Some(common));
            self.dom.insert(parent, last_node, before);
            let tag = match &self.afe[self.afe_index(fe).expect("fe in afe")] {
                Afe::El(_, t) => t.clone(),
                Afe::Marker => unreachable!(),
            };
            let new = self.create_element(&tag, Ns::Html);
            for c in std::mem::take(&mut self.dom.nodes[furthest].children) {
                self.dom.nodes[c].parent = None;
                self.dom.insert(new, c, None);
            }
            self.dom.insert(furthest, new, None);
            let fe_i = self.afe_index(fe).expect("fe in afe");
            self.afe.remove(fe_i);
            let at = match bookmark_after.and_then(|b| self.afe_index(b)) {
                Some(i) => i + 1,
                None => fe_i,
            };
            self.afe.insert(at.min(self.afe.len()), Afe::El(new, tag));
            self.open.retain(|&n| n != fe);
            let fb = self.open.iter().position(|&n| n == furthest).expect("furthest open");
            self.open.insert(fb + 1, new);
        }
        true
    }

    // ------------------------------------------------------------ modes

    fn reset_mode(&mut self) {
        for i in (0..self.open.len()).rev() {
            let last = i == 0;
            let node = if last { self.context.unwrap_or(self.open[0]) } else { self.open[i] };
            if self.dom.ns_of(node) != Ns::Html {
                if last {
                    self.mode = Mode::InBody;
                    return;
                }
                continue;
            }
            let m = match self.name(node) {
                "td" | "th" if !last => Mode::InCell,
                "tr" => Mode::InRow,
                "tbody" | "thead" | "tfoot" => Mode::InTableBody,
                "caption" => Mode::InCaption,
                "colgroup" => Mode::InColumnGroup,
                "table" => Mode::InTable,
                "template" => *self.tmpl_modes.last().unwrap_or(&Mode::InTemplate),
                "head" if !last => Mode::InHead,
                "body" => Mode::InBody,
                "frameset" => Mode::InFrameset,
                "html" => {
                    if self.head.is_none() {
                        Mode::BeforeHead
                    } else {
                        Mode::AfterHead
                    }
                }
                _ if last => Mode::InBody,
                _ => continue,
            };
            self.mode = m;
            return;
        }
        self.mode = Mode::InBody;
    }

    fn generic_text(&mut self, tag: &Tag, state: State) {
        self.insert_html(tag);
        self.tk.state = state;
        self.orig_mode = self.mode;
        self.mode = Mode::Text;
    }

    /// The tree construction dispatcher: HTML rules, or the rules for foreign content.
    fn dispatch(&mut self, t: Token) {
        let use_html = match self.adjusted_current() {
            None => true,
            Some(n) => {
                self.dom.ns_of(n) == Ns::Html
                    || (self.mathml_text_point(n) && matches!(&t, Token::StartTag(tg) if tg.name != "mglyph" && tg.name != "malignmark"))
                    || (self.mathml_text_point(n) && matches!(t, Token::Text(_)))
                    || (self.dom.ns_of(n) == Ns::MathMl && self.name(n) == "annotation-xml" && matches!(&t, Token::StartTag(tg) if tg.name == "svg"))
                    || (self.html_integration_point(n) && matches!(t, Token::StartTag(_) | Token::Text(_)))
                    || t == Token::Eof
            }
        };
        if use_html {
            self.step(self.mode, t);
        } else {
            self.foreign(t);
        }
    }

    fn step(&mut self, mode: Mode, t: Token) {
        match mode {
            Mode::Initial => self.initial(t),
            Mode::BeforeHtml => self.before_html(t),
            Mode::BeforeHead => self.before_head(t),
            Mode::InHead => self.in_head(t),
            Mode::InHeadNoscript => self.in_head_noscript(t),
            Mode::AfterHead => self.after_head(t),
            Mode::InBody => self.in_body(t),
            Mode::Text => self.text(t),
            Mode::InTable => self.in_table(t),
            Mode::InTableText => self.in_table_text(t),
            Mode::InCaption => self.in_caption(t),
            Mode::InColumnGroup => self.in_column_group(t),
            Mode::InTableBody => self.in_table_body(t),
            Mode::InRow => self.in_row(t),
            Mode::InCell => self.in_cell(t),
            Mode::InTemplate => self.in_template(t),
            Mode::AfterBody => self.after_body(t),
            Mode::InFrameset => self.in_frameset(t),
            Mode::AfterFrameset => self.after_frameset(t),
            Mode::AfterAfterBody => self.after_after_body(t),
            Mode::AfterAfterFrameset => self.after_after_frameset(t),
        }
    }

    fn reprocess(&mut self, t: Token) {
        self.step(self.mode, t);
    }

    fn initial(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => {}
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, Some(Dom::ROOT)),
            Token::Doctype(d) => {
                self.dom.add(
                    NodeData::Doctype {
                        name: d.name.clone().unwrap_or_default(),
                        public_id: d.public_id.clone(),
                        system_id: d.system_id.clone(),
                    },
                    Dom::ROOT,
                );
                self.dom.quirks = doctype_quirks(&d);
                self.mode = Mode::BeforeHtml;
            }
            t => {
                self.dom.quirks = Quirks::Quirks;
                self.mode = Mode::BeforeHtml;
                self.reprocess(t);
            }
        }
    }

    fn before_html(&mut self, t: Token) {
        match t {
            Token::Doctype(_) => {}
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, Some(Dom::ROOT)),
            Token::Text(ref s) if is_ws_str(s) => {}
            Token::StartTag(ref tag) if tag.name == "html" => {
                let id = self.create_element(tag, Ns::Html);
                self.dom.insert(Dom::ROOT, id, None);
                self.open.push(id);
                self.mode = Mode::BeforeHead;
            }
            Token::EndTag(ref tag) if !matches!(tag.name.as_str(), "head" | "body" | "html" | "br") => {}
            t => {
                let id = self.create_element(&Tag { name: "html".into(), attrs: Vec::new(), self_closing: false }, Ns::Html);
                self.dom.insert(Dom::ROOT, id, None);
                self.open.push(id);
                self.mode = Mode::BeforeHead;
                self.reprocess(t);
            }
        }
    }

    fn before_head(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => {}
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::Doctype(_) => {}
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::StartTag(ref tag) if tag.name == "head" => {
                let h = self.insert_html(tag);
                self.head = Some(h);
                self.mode = Mode::InHead;
            }
            Token::EndTag(ref tag) if !matches!(tag.name.as_str(), "head" | "body" | "html" | "br") => {}
            t => {
                let h = self.insert_fake("head");
                self.head = Some(h);
                self.mode = Mode::InHead;
                self.reprocess(t);
            }
        }
    }

    fn in_head(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => self.insert_text(s),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::Doctype(_) => {}
            Token::StartTag(ref tag) => match tag.name.as_str() {
                "html" => self.in_body(t),
                "base" | "basefont" | "bgsound" | "link" | "meta" => {
                    self.insert_html(tag);
                    self.pop();
                }
                "title" => self.generic_text(tag, State::Rcdata),
                "noscript" if self.scripting => self.generic_text(tag, State::Rawtext),
                "noframes" | "style" => self.generic_text(tag, State::Rawtext),
                "noscript" => {
                    self.insert_html(tag);
                    self.mode = Mode::InHeadNoscript;
                }
                "script" => self.generic_text(tag, State::ScriptData),
                "template" => {
                    // Declarative shadow DOM: the template's contents become the shadow root
                    // of the element it's in, and the template itself isn't kept.
                    let mode = tag.attrs.iter().find(|(k, _)| k == "shadowrootmode").map(|(_, v)| v.to_ascii_lowercase());
                    let host = self.adjusted_current();
                    let declarative = self.allow_declarative_shadow
                        && matches!(mode.as_deref(), Some("open" | "closed"))
                        && host.is_some_and(|h| Some(h) != self.open.first().copied() && !self.dom.shadow_roots.contains_key(&h));
                    if let (true, Some(host)) = (declarative, host) {
                        let t = self.create_element(tag, Ns::Html);
                        let root = self.dom.attach_shadow(host, mode.as_deref() == Some("open"));
                        self.dom.template_contents.insert(t, root);
                        self.open.push(t);
                    } else {
                        self.insert_html(tag);
                    }
                    self.afe.push(Afe::Marker);
                    self.mode = Mode::InTemplate;
                    self.tmpl_modes.push(Mode::InTemplate);
                }
                "head" => {}
                _ => self.leave_head(t),
            },
            Token::EndTag(ref tag) => match tag.name.as_str() {
                "head" => {
                    self.pop();
                    self.mode = Mode::AfterHead;
                }
                "body" | "html" | "br" => self.leave_head(t),
                "template" => {
                    if !self.has_open("template") {
                        return;
                    }
                    self.generate_implied_all();
                    self.pop_until(&["template"]);
                    self.clear_afe_to_marker();
                    self.tmpl_modes.pop();
                    self.reset_mode();
                }
                _ => {}
            },
            t => self.leave_head(t),
        }
    }

    fn leave_head(&mut self, t: Token) {
        self.pop();
        self.mode = Mode::AfterHead;
        self.reprocess(t);
    }

    fn in_head_noscript(&mut self, t: Token) {
        match t {
            Token::Doctype(_) => {}
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::EndTag(ref tag) if tag.name == "noscript" => {
                self.pop();
                self.mode = Mode::InHead;
            }
            Token::Text(ref s) if is_ws_str(s) => self.in_head(t),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.in_head(t),
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style") => self.in_head(t),
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "head" | "noscript") => {}
            Token::EndTag(ref tag) if tag.name != "br" => {}
            t => {
                self.pop();
                self.mode = Mode::InHead;
                self.reprocess(t);
            }
        }
    }

    fn after_head(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => self.insert_text(s),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::Doctype(_) => {}
            Token::StartTag(ref tag) => match tag.name.as_str() {
                "html" => self.in_body(t),
                "body" => {
                    self.insert_html(tag);
                    self.frameset_ok = false;
                    self.mode = Mode::InBody;
                }
                "frameset" => {
                    self.insert_html(tag);
                    self.mode = Mode::InFrameset;
                }
                n if HEAD_TAGS.contains(&n) => {
                    let head = self.head.expect("head");
                    self.open.push(head);
                    self.in_head(t);
                    self.open.retain(|&x| x != head);
                }
                "head" => {}
                _ => self.leave_after_head(t),
            },
            Token::EndTag(ref tag) => match tag.name.as_str() {
                "template" => self.in_head(t),
                "body" | "html" | "br" => self.leave_after_head(t),
                _ => {}
            },
            t => self.leave_after_head(t),
        }
    }

    fn leave_after_head(&mut self, t: Token) {
        self.insert_fake("body");
        self.mode = Mode::InBody;
        self.reprocess(t);
    }

    fn add_missing_attrs(&mut self, node: NodeId, tag: &Tag) {
        if let NodeData::Element { attrs, .. } = &mut self.dom.nodes[node].data {
            for (k, v) in &tag.attrs {
                if !attrs.iter().any(|(a, _)| a == k) {
                    attrs.push((k.clone(), v.clone()));
                }
            }
        }
    }

    fn in_body(&mut self, t: Token) {
        match t {
            Token::Text(ref s) => {
                if s.starts_with('\0') {
                    return;
                }
                self.reconstruct_afe();
                self.insert_text(s);
                // Text inside a template's contents isn't in the document, so it doesn't
                // rule out a frameset.
                if !is_ws_str(s) && !self.has_open("template") {
                    self.frameset_ok = false;
                }
            }
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::Doctype(_) => {}
            Token::StartTag(tag) => self.in_body_start(tag),
            Token::EndTag(tag) => self.in_body_end(tag),
            Token::Eof => {
                if !self.tmpl_modes.is_empty() {
                    self.in_template(Token::Eof);
                } else {
                    self.stopped = true;
                }
            }
        }
    }

    fn in_body_start(&mut self, tag: Tag) {
        let name = tag.name.clone();
        match name.as_str() {
            "html" => {
                if !self.has_open("template") {
                    let h = self.open[0];
                    self.add_missing_attrs(h, &tag);
                }
            }
            "template" => {
                self.frameset_ok = false;
                self.in_head(Token::StartTag(tag));
            }
            n if HEAD_TAGS.contains(&n) => self.in_head(Token::StartTag(tag)),
            "body" => {
                if self.open.len() == 1 || !self.is(self.open[1], "body") || self.has_open("template") {
                    return;
                }
                self.frameset_ok = false;
                let b = self.open[1];
                self.add_missing_attrs(b, &tag);
            }
            "frameset" => {
                if self.open.len() == 1 || !self.is(self.open[1], "body") || !self.frameset_ok {
                    return;
                }
                let b = self.open[1];
                self.dom.detach(b);
                self.open.truncate(1);
                self.insert_html(&tag);
                self.mode = Mode::InFrameset;
            }
            "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption"
            | "figure" | "footer" | "header" | "hgroup" | "main" | "menu" | "nav" | "ol" | "p" | "search" | "section" | "summary" | "ul" => {
                self.close_p_if_open();
                self.insert_html(&tag);
            }
            n if HEADINGS.contains(&n) => {
                self.close_p_if_open();
                if self.current().is_some_and(|c| self.is_in(c, HEADINGS)) {
                    self.pop();
                }
                self.insert_html(&tag);
            }
            "pre" | "listing" => {
                self.close_p_if_open();
                self.insert_html(&tag);
                self.skip_lf = true;
                self.frameset_ok = false;
            }
            "form" => {
                let tmpl = self.in_template_ctx();
                if self.form.is_some() && !tmpl {
                    return;
                }
                self.close_p_if_open();
                let f = self.insert_html(&tag);
                if !tmpl {
                    self.form = Some(f);
                }
            }
            "li" | "dd" | "dt" => {
                self.frameset_ok = false;
                let stops: &[&str] = if name == "li" { &["li"] } else { &["dd", "dt"] };
                for i in (0..self.open.len()).rev() {
                    let node = self.open[i];
                    if self.is_in(node, stops) {
                        let nm = self.name(node).to_string();
                        self.generate_implied(Some(&nm));
                        self.pop_until(&[nm.as_str()]);
                        break;
                    }
                    if self.is_special(node) && !self.is_in(node, &["address", "div", "p"]) {
                        break;
                    }
                }
                self.close_p_if_open();
                self.insert_html(&tag);
            }
            "plaintext" => {
                self.close_p_if_open();
                self.insert_html(&tag);
                self.tk.state = State::Plaintext;
            }
            "button" => {
                if self.in_scope("button") {
                    self.generate_implied(None);
                    self.pop_until(&["button"]);
                }
                self.reconstruct_afe();
                self.insert_html(&tag);
                self.frameset_ok = false;
            }
            "a" => {
                let mut existing = None;
                for e in self.afe.iter().rev() {
                    match e {
                        Afe::Marker => break,
                        Afe::El(n, t) if t.name == "a" => {
                            existing = Some(*n);
                            break;
                        }
                        _ => {}
                    }
                }
                if let Some(a) = existing {
                    if !self.adoption_agency("a") {
                        self.any_other_end("a");
                    }
                    if let Some(i) = self.afe_index(a) {
                        self.afe.remove(i);
                    }
                    self.open.retain(|&n| n != a);
                }
                self.reconstruct_afe();
                let n = self.insert_html(&tag);
                self.push_afe(n, &tag);
            }
            "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small" | "strike" | "strong" | "tt" | "u" => {
                self.reconstruct_afe();
                let n = self.insert_html(&tag);
                self.push_afe(n, &tag);
            }
            "nobr" => {
                self.reconstruct_afe();
                if self.in_scope("nobr") {
                    if !self.adoption_agency("nobr") {
                        self.any_other_end("nobr");
                    }
                    self.reconstruct_afe();
                }
                let n = self.insert_html(&tag);
                self.push_afe(n, &tag);
            }
            "applet" | "marquee" | "object" => {
                self.reconstruct_afe();
                self.insert_html(&tag);
                self.afe.push(Afe::Marker);
                self.frameset_ok = false;
            }
            "table" => {
                if self.dom.quirks != Quirks::Quirks {
                    self.close_p_if_open();
                }
                self.insert_html(&tag);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
                self.reconstruct_afe();
                self.insert_html(&tag);
                self.pop();
                self.frameset_ok = false;
            }
            "input" => {
                if self.context.is_some_and(|c| self.is(c, "select")) {
                    return;
                }
                if self.in_scope("select") {
                    self.pop_until(&["select"]);
                }
                self.reconstruct_afe();
                self.insert_html(&tag);
                self.pop();
                let hidden = tag.attrs.iter().any(|(k, v)| k == "type" && v.eq_ignore_ascii_case("hidden"));
                if !hidden {
                    self.frameset_ok = false;
                }
            }
            "param" | "source" | "track" => {
                self.insert_html(&tag);
                self.pop();
            }
            "hr" => {
                self.close_p_if_open();
                if self.in_scope("select") {
                    self.generate_implied(None);
                }
                self.insert_html(&tag);
                self.pop();
                self.frameset_ok = false;
            }
            "image" => {
                let mut t = tag;
                t.name = "img".into();
                self.in_body_start(t);
            }
            "textarea" => {
                self.insert_html(&tag);
                self.skip_lf = true;
                self.tk.state = State::Rcdata;
                self.orig_mode = self.mode;
                self.frameset_ok = false;
                self.mode = Mode::Text;
            }
            "xmp" => {
                self.close_p_if_open();
                self.reconstruct_afe();
                self.frameset_ok = false;
                self.generic_text(&tag, State::Rawtext);
            }
            "iframe" => {
                self.frameset_ok = false;
                self.generic_text(&tag, State::Rawtext);
            }
            "noembed" => self.generic_text(&tag, State::Rawtext),
            "noscript" if self.scripting => self.generic_text(&tag, State::Rawtext),
            "select" => {
                if self.context.is_some_and(|c| self.is(c, "select")) {
                    return;
                }
                if self.in_scope("select") {
                    self.pop_until(&["select"]);
                    return;
                }
                self.reconstruct_afe();
                self.insert_html(&tag);
                self.frameset_ok = false;
            }
            "option" => {
                if self.in_scope("select") {
                    self.generate_implied(Some("optgroup"));
                } else if self.cur_is("option") {
                    self.pop();
                }
                self.reconstruct_afe();
                self.insert_html(&tag);
            }
            "optgroup" => {
                if self.in_scope("select") {
                    self.generate_implied(None);
                } else if self.cur_is("option") {
                    self.pop();
                }
                self.reconstruct_afe();
                self.insert_html(&tag);
            }
            "rb" | "rtc" => {
                if self.in_scope("ruby") {
                    self.generate_implied(None);
                }
                self.insert_html(&tag);
            }
            "rp" | "rt" => {
                if self.in_scope("ruby") {
                    self.generate_implied(Some("rtc"));
                }
                self.insert_html(&tag);
            }
            "math" | "svg" => {
                let ns = if name == "math" { Ns::MathMl } else { Ns::Svg };
                self.reconstruct_afe();
                let mut t = tag;
                adjust_attrs(&mut t, ns);
                self.insert_element(&t, ns);
                if t.self_closing {
                    self.pop();
                }
            }
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr" => {}
            _ => {
                self.reconstruct_afe();
                self.insert_html(&tag);
            }
        }
    }

    fn in_body_end(&mut self, tag: Tag) {
        let name = tag.name.clone();
        match name.as_str() {
            "template" => self.in_head(Token::EndTag(tag)),
            "body" | "html" => {
                if !self.in_scope("body") {
                    return;
                }
                self.mode = Mode::AfterBody;
                if name == "html" {
                    self.reprocess(Token::EndTag(tag));
                }
            }
            "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details" | "dialog" | "dir" | "div" | "dl" | "fieldset"
            | "figcaption" | "figure" | "footer" | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol" | "pre" | "search"
            | "section" | "select" | "summary" | "ul" => {
                if !self.in_scope(&name) {
                    return;
                }
                self.generate_implied(None);
                self.pop_until(&[name.as_str()]);
            }
            "form" => {
                if !self.in_template_ctx() {
                    let node = self.form.take();
                    let Some(node) = node else { return };
                    // In scope as that particular element.
                    if !self.in_scope("form") || !self.open.contains(&node) {
                        return;
                    }
                    self.generate_implied(None);
                    self.open.retain(|&n| n != node);
                } else {
                    if !self.in_scope("form") {
                        return;
                    }
                    self.generate_implied(None);
                    self.pop_until(&["form"]);
                }
            }
            "p" => {
                if !self.in_button_scope("p") {
                    self.insert_fake("p");
                }
                self.close_p();
            }
            "li" => {
                if !self.in_list_scope("li") {
                    return;
                }
                self.generate_implied(Some("li"));
                self.pop_until(&["li"]);
            }
            "dd" | "dt" => {
                if !self.in_scope(&name) {
                    return;
                }
                self.generate_implied(Some(&name));
                self.pop_until(&[name.as_str()]);
            }
            n if HEADINGS.contains(&n) => {
                if !self.in_scope_with(HEADINGS, &[], false) {
                    return;
                }
                self.generate_implied(None);
                self.pop_until(HEADINGS);
            }
            n if FORMATTING.contains(&n) => {
                if !self.adoption_agency(n) {
                    self.any_other_end(&name);
                }
            }
            "applet" | "marquee" | "object" => {
                if !self.in_scope(&name) {
                    return;
                }
                self.generate_implied(None);
                self.pop_until(&[name.as_str()]);
                self.clear_afe_to_marker();
            }
            "br" => self.in_body_start(Tag { name: "br".into(), attrs: Vec::new(), self_closing: false }),
            _ => self.any_other_end(&name),
        }
    }

    fn any_other_end(&mut self, name: &str) {
        for i in (0..self.open.len()).rev() {
            let node = self.open[i];
            if self.is(node, name) {
                self.generate_implied(Some(name));
                self.open.truncate(i);
                return;
            }
            if self.is_special(node) {
                return;
            }
        }
    }

    fn text(&mut self, t: Token) {
        match t {
            Token::Text(s) => self.insert_text(&s),
            Token::EndTag(ref tag) if tag.name == "script" => {
                let script = self.pop();
                self.mode = self.orig_mode;
                if self.pause_at_scripts {
                    self.pending_script = script;
                }
            }
            Token::Eof => {
                self.pop();
                self.mode = self.orig_mode;
                self.reprocess(Token::Eof);
            }
            _ => {
                self.pop();
                self.mode = self.orig_mode;
            }
        }
    }

    fn clear_to(&mut self, names: &[&str]) {
        while let Some(c) = self.current() {
            if self.is_in(c, names) {
                break;
            }
            self.pop();
        }
    }

    fn in_table(&mut self, t: Token) {
        match t {
            Token::Text(_) if self.current().is_some_and(|c| self.is_in(c, &["table", "tbody", "template", "tfoot", "thead", "tr"])) => {
                self.pending_text.clear();
                self.orig_mode = self.mode;
                self.mode = Mode::InTableText;
                self.reprocess(t);
            }
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::Doctype(_) => {}
            Token::StartTag(ref tag) => match tag.name.as_str() {
                "caption" => {
                    self.clear_to(&["table", "template", "html"]);
                    self.afe.push(Afe::Marker);
                    self.insert_html(tag);
                    self.mode = Mode::InCaption;
                }
                "colgroup" => {
                    self.clear_to(&["table", "template", "html"]);
                    self.insert_html(tag);
                    self.mode = Mode::InColumnGroup;
                }
                "col" => {
                    self.clear_to(&["table", "template", "html"]);
                    self.insert_fake("colgroup");
                    self.mode = Mode::InColumnGroup;
                    self.reprocess(t);
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_to(&["table", "template", "html"]);
                    self.insert_html(tag);
                    self.mode = Mode::InTableBody;
                }
                "td" | "th" | "tr" => {
                    self.clear_to(&["table", "template", "html"]);
                    self.insert_fake("tbody");
                    self.mode = Mode::InTableBody;
                    self.reprocess(t);
                }
                "table" => {
                    if !self.in_table_scope("table") {
                        return;
                    }
                    self.pop_until(&["table"]);
                    self.reset_mode();
                    self.reprocess(t);
                }
                "style" | "script" | "template" => self.in_head(t),
                "input" if tag.attrs.iter().any(|(k, v)| k == "type" && v.eq_ignore_ascii_case("hidden")) => {
                    self.insert_html(tag);
                    self.pop();
                }
                "form" => {
                    let tmpl = self.in_template_ctx();
                    if self.form.is_some() && !tmpl {
                        return;
                    }
                    let f = self.insert_html(tag);
                    if !tmpl {
                        self.form = Some(f);
                    }
                    self.pop();
                }
                _ => self.table_anything_else(t),
            },
            Token::EndTag(ref tag) => match tag.name.as_str() {
                "table" => {
                    if !self.in_table_scope("table") {
                        return;
                    }
                    self.pop_until(&["table"]);
                    self.reset_mode();
                }
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr" => {}
                "template" => self.in_head(t),
                _ => self.table_anything_else(t),
            },
            Token::Eof => self.in_body(t),
            t => self.table_anything_else(t),
        }
    }

    fn table_anything_else(&mut self, t: Token) {
        self.foster = true;
        self.in_body(t);
        self.foster = false;
    }

    fn in_table_text(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if s.starts_with('\0') => {}
            Token::Text(s) => self.pending_text.push_str(&s),
            t => {
                let pending = std::mem::take(&mut self.pending_text);
                if !pending.is_empty() {
                    if !is_ws_str(&pending) {
                        self.foster = true;
                        self.reconstruct_afe();
                        self.insert_text(&pending);
                        self.frameset_ok = false;
                        self.foster = false;
                    } else {
                        self.insert_text(&pending);
                    }
                }
                self.mode = self.orig_mode;
                self.reprocess(t);
            }
        }
    }

    fn close_caption(&mut self) -> bool {
        if !self.in_table_scope("caption") {
            return false;
        }
        self.generate_implied(None);
        self.pop_until(&["caption"]);
        self.clear_afe_to_marker();
        self.mode = Mode::InTable;
        true
    }

    fn in_caption(&mut self, t: Token) {
        match t {
            Token::EndTag(ref tag) if tag.name == "caption" => {
                self.close_caption();
            }
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr") => {
                if self.close_caption() {
                    self.reprocess(t);
                }
            }
            Token::EndTag(ref tag) if tag.name == "table" => {
                if self.close_caption() {
                    self.reprocess(t);
                }
            }
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "body" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr") => {}
            t => self.in_body(t),
        }
    }

    fn in_column_group(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => self.insert_text(s),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::Doctype(_) => {}
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::StartTag(ref tag) if tag.name == "col" => {
                self.insert_html(tag);
                self.pop();
            }
            Token::EndTag(ref tag) if tag.name == "colgroup" => {
                if self.cur_is("colgroup") {
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Token::EndTag(ref tag) if tag.name == "col" => {}
            Token::StartTag(ref tag) if tag.name == "template" => self.in_head(t),
            Token::EndTag(ref tag) if tag.name == "template" => self.in_head(t),
            Token::Eof => self.in_body(t),
            t => {
                if self.cur_is("colgroup") {
                    self.pop();
                    self.mode = Mode::InTable;
                    self.reprocess(t);
                }
            }
        }
    }

    fn in_table_body(&mut self, t: Token) {
        const CTX: &[&str] = &["tbody", "tfoot", "thead", "template", "html"];
        match t {
            Token::StartTag(ref tag) if tag.name == "tr" => {
                self.clear_to(CTX);
                self.insert_html(tag);
                self.mode = Mode::InRow;
            }
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "th" | "td") => {
                self.clear_to(CTX);
                self.insert_fake("tr");
                self.mode = Mode::InRow;
                self.reprocess(t);
            }
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "tbody" | "tfoot" | "thead") => {
                if !self.in_table_scope(&tag.name) {
                    return;
                }
                self.clear_to(CTX);
                self.pop();
                self.mode = Mode::InTable;
            }
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead") => self.leave_table_body(t),
            Token::EndTag(ref tag) if tag.name == "table" => self.leave_table_body(t),
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr") => {}
            t => self.in_table(t),
        }
    }

    fn leave_table_body(&mut self, t: Token) {
        if !(self.in_table_scope("tbody") || self.in_table_scope("thead") || self.in_table_scope("tfoot")) {
            return;
        }
        self.clear_to(&["tbody", "tfoot", "thead", "template", "html"]);
        self.pop();
        self.mode = Mode::InTable;
        self.reprocess(t);
    }

    fn close_row(&mut self) -> bool {
        if !self.in_table_scope("tr") {
            return false;
        }
        self.clear_to(&["tr", "template", "html"]);
        self.pop();
        self.mode = Mode::InTableBody;
        true
    }

    fn in_row(&mut self, t: Token) {
        match t {
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "th" | "td") => {
                self.clear_to(&["tr", "template", "html"]);
                self.insert_html(tag);
                self.mode = Mode::InCell;
                self.afe.push(Afe::Marker);
            }
            Token::EndTag(ref tag) if tag.name == "tr" => {
                self.close_row();
            }
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr") => {
                if self.close_row() {
                    self.reprocess(t);
                }
            }
            Token::EndTag(ref tag) if tag.name == "table" => {
                if self.close_row() {
                    self.reprocess(t);
                }
            }
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "tbody" | "tfoot" | "thead") => {
                if !self.in_table_scope(&tag.name) {
                    return;
                }
                if self.close_row() {
                    self.reprocess(t);
                }
            }
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th") => {}
            t => self.in_table(t),
        }
    }

    fn close_cell(&mut self) {
        self.generate_implied(None);
        self.pop_until(&["td", "th"]);
        self.clear_afe_to_marker();
        self.mode = Mode::InRow;
    }

    fn in_cell(&mut self, t: Token) {
        match t {
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "td" | "th") => {
                if !self.in_table_scope(&tag.name) {
                    return;
                }
                self.generate_implied(None);
                self.pop_until(&[tag.name.as_str()]);
                self.clear_afe_to_marker();
                self.mode = Mode::InRow;
            }
            Token::StartTag(ref tag) if matches!(tag.name.as_str(), "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr") => {
                if !(self.in_table_scope("td") || self.in_table_scope("th")) {
                    return;
                }
                self.close_cell();
                self.reprocess(t);
            }
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "body" | "caption" | "col" | "colgroup" | "html") => {}
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") => {
                if !self.in_table_scope(&tag.name) {
                    return;
                }
                self.close_cell();
                self.reprocess(t);
            }
            t => self.in_body(t),
        }
    }

    fn in_template(&mut self, t: Token) {
        match t {
            Token::Text(_) | Token::Comment(_) | Token::ProcessingInstruction { .. } | Token::Doctype(_) => self.in_body(t),
            Token::StartTag(ref tag) if HEAD_TAGS.contains(&tag.name.as_str()) => self.in_head(t),
            Token::EndTag(ref tag) if tag.name == "template" => self.in_head(t),
            Token::StartTag(ref tag) => {
                let m = match tag.name.as_str() {
                    "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => Mode::InTable,
                    "col" => Mode::InColumnGroup,
                    "tr" => Mode::InTableBody,
                    "td" | "th" => Mode::InRow,
                    _ => Mode::InBody,
                };
                self.tmpl_modes.pop();
                self.tmpl_modes.push(m);
                self.mode = m;
                self.reprocess(t);
            }
            Token::EndTag(_) => {}
            Token::Eof => {
                if !self.has_open("template") {
                    self.stopped = true;
                    return;
                }
                self.pop_until(&["template"]);
                self.clear_afe_to_marker();
                self.tmpl_modes.pop();
                self.reset_mode();
                self.reprocess(Token::Eof);
            }
        }
    }

    fn after_body(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => self.in_body(t),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => {
                let html = self.open[0];
                self.insert_comment(&t, Some(html));
            }
            Token::Doctype(_) => {}
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::EndTag(ref tag) if tag.name == "html" => {
                if self.context.is_none() {
                    self.mode = Mode::AfterAfterBody;
                }
            }
            Token::Eof => self.stopped = true,
            t => {
                self.mode = Mode::InBody;
                self.reprocess(t);
            }
        }
    }

    fn in_frameset(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => self.insert_text(s),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::StartTag(ref tag) if tag.name == "frameset" => {
                self.insert_html(tag);
            }
            Token::EndTag(ref tag) if tag.name == "frameset" => {
                if self.open.len() == 1 {
                    return;
                }
                self.pop();
                if self.context.is_none() && !self.cur_is("frameset") {
                    self.mode = Mode::AfterFrameset;
                }
            }
            Token::StartTag(ref tag) if tag.name == "frame" => {
                self.insert_html(tag);
                self.pop();
            }
            Token::StartTag(ref tag) if tag.name == "noframes" => self.in_head(t),
            Token::Eof => self.stopped = true,
            _ => {}
        }
    }

    fn after_frameset(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if is_ws_str(s) => self.insert_text(s),
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::EndTag(ref tag) if tag.name == "html" => self.mode = Mode::AfterAfterFrameset,
            Token::StartTag(ref tag) if tag.name == "noframes" => self.in_head(t),
            Token::Eof => self.stopped = true,
            _ => {}
        }
    }

    fn after_after_body(&mut self, t: Token) {
        match t {
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, Some(Dom::ROOT)),
            Token::Doctype(_) => self.in_body(t),
            Token::Text(ref s) if is_ws_str(s) => self.in_body(t),
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::Eof => self.stopped = true,
            t => {
                self.mode = Mode::InBody;
                self.reprocess(t);
            }
        }
    }

    fn after_after_frameset(&mut self, t: Token) {
        match t {
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, Some(Dom::ROOT)),
            Token::Doctype(_) => self.in_body(t),
            Token::Text(ref s) if is_ws_str(s) => self.in_body(t),
            Token::StartTag(ref tag) if tag.name == "html" => self.in_body(t),
            Token::Eof => self.stopped = true,
            Token::StartTag(ref tag) if tag.name == "noframes" => self.in_head(t),
            _ => {}
        }
    }

    /// The rules for parsing tokens in foreign content (inside `<svg>` or `<math>`).
    fn foreign(&mut self, t: Token) {
        match t {
            Token::Text(ref s) if s.starts_with('\0') => self.insert_text(&"\u{FFFD}".repeat(s.chars().count())),
            Token::Text(ref s) => {
                self.insert_text(s);
                if !is_ws_str(s) {
                    self.frameset_ok = false;
                }
            }
            Token::Comment(_) | Token::ProcessingInstruction { .. } => self.insert_comment(&t, None),
            Token::Doctype(_) => {}
            Token::StartTag(ref tag)
                if matches!(
                    tag.name.as_str(),
                    "b" | "big" | "blockquote" | "body" | "br" | "center" | "code" | "dd" | "div" | "dl" | "dt" | "em" | "embed" | "h1" | "h2" | "h3"
                        | "h4" | "h5" | "h6" | "head" | "hr" | "i" | "img" | "li" | "listing" | "menu" | "meta" | "nobr" | "ol" | "p" | "pre"
                        | "ruby" | "s" | "small" | "span" | "strong" | "strike" | "sub" | "sup" | "table" | "tt" | "u" | "ul" | "var"
                ) || (tag.name == "font" && tag.attrs.iter().any(|(k, _)| matches!(k.as_str(), "color" | "face" | "size"))) =>
            {
                self.foreign_breakout(t)
            }
            Token::EndTag(ref tag) if matches!(tag.name.as_str(), "br" | "p") => self.foreign_breakout(t),
            Token::StartTag(mut tag) => {
                let acn = self.adjusted_current().expect("adjusted current");
                let ns = self.dom.ns_of(acn);
                if ns == Ns::Svg {
                    tag.name = adjust_svg_tag(&tag.name);
                }
                adjust_attrs(&mut tag, ns);
                self.insert_element(&tag, ns);
                if tag.self_closing {
                    self.pop();
                }
            }
            Token::EndTag(ref tag) => {
                let mut i = self.open.len() - 1;
                loop {
                    if i == 0 {
                        return;
                    }
                    let node = self.open[i];
                    if self.name(node).to_ascii_lowercase() == tag.name {
                        self.open.truncate(i);
                        return;
                    }
                    i -= 1;
                    if self.dom.ns_of(self.open[i]) == Ns::Html {
                        self.step(self.mode, t);
                        return;
                    }
                }
            }
            Token::Eof => self.step(self.mode, t),
        }
    }

    fn foreign_breakout(&mut self, t: Token) {
        while let Some(c) = self.current() {
            if self.mathml_text_point(c) || self.html_integration_point(c) || self.dom.ns_of(c) == Ns::Html {
                break;
            }
            self.pop();
        }
        // Reprocessed with the HTML rules of the current insertion mode.
        self.step(self.mode, t);
    }
}

fn deep_clone(dom: &mut Dom, src: NodeId) -> NodeId {
    let id = dom.create(dom.nodes[src].data.clone());
    if let Some(&ns) = dom.ns.get(&src) {
        dom.ns.insert(id, ns);
    }
    for c in dom.nodes[src].children.clone() {
        let k = deep_clone(dom, c);
        dom.insert(id, k, None);
    }
    id
}

/// A customizable `<select>`'s `<selectedcontent>` shows a copy of the selected option (the
/// first one with `selected`, else the first option).
fn fill_selectedcontent(dom: &mut Dom) {
    for select in dom.find_all("select") {
        let all = dom.descendants(select);
        let html = |dom: &Dom, n: NodeId, t: &str| dom.ns_of(n) == Ns::Html && dom.tag(n) == Some(t);
        let Some(&sc) = all.iter().find(|&&n| html(dom, n, "selectedcontent")) else { continue };
        let options: Vec<NodeId> = all.iter().copied().filter(|&n| html(dom, n, "option")).collect();
        let Some(opt) = options.iter().copied().find(|&o| dom.attr(o, "selected").is_some()).or(options.first().copied()) else { continue };
        dom.remove_children(sc);
        for c in dom.nodes[opt].children.clone() {
            let k = deep_clone(dom, c);
            dom.insert(sc, k, None);
        }
    }
}

/// The tree in the html5lib-tests text format (`| <html>`, `|   "text"`).
pub fn dump(dom: &Dom) -> String {
    fn walk(dom: &Dom, id: NodeId, depth: usize, out: &mut String) {
        for &c in &dom.nodes[id].children {
            let pad = "  ".repeat(depth);
            match &dom.nodes[c].data {
                NodeData::Element { tag, attrs } => {
                    let ns = dom.ns_of(c);
                    let prefix = match ns {
                        Ns::Html => "",
                        Ns::Svg => "svg ",
                        Ns::MathMl => "math ",
                        Ns::Other => "other ",
                    };
                    out.push_str(&format!("| {pad}<{prefix}{tag}>\n"));
                    let mut a: Vec<(String, &String)> = attrs
                        .iter()
                        .map(|(k, v)| {
                            let k = if ns != Ns::Html && FOREIGN_ATTRS.contains(&k.as_str()) { k.replacen(':', " ", 1) } else { k.clone() };
                            (k, v)
                        })
                        .collect();
                    a.sort();
                    for (k, v) in a {
                        out.push_str(&format!("| {pad}  {k}=\"{v}\"\n"));
                    }
                    if let Some(&frag) = dom.template_contents.get(&c) {
                        out.push_str(&format!("| {pad}  content\n"));
                        walk(dom, frag, depth + 2, out);
                    }
                    walk(dom, c, depth + 1, out);
                }
                NodeData::Text(t) => out.push_str(&format!("| {pad}\"{t}\"\n")),
                NodeData::Comment(t) => out.push_str(&format!("| {pad}<!-- {t} -->\n")),
                NodeData::ProcessingInstruction { target, data } => out.push_str(&format!("| {pad}<?{target} {data}?>\n")),
                NodeData::Doctype { name, public_id, system_id } => {
                    if public_id.is_none() && system_id.is_none() {
                        out.push_str(&format!("| {pad}<!DOCTYPE {name}>\n"));
                    } else {
                        let (p, s) = (public_id.as_deref().unwrap_or(""), system_id.as_deref().unwrap_or(""));
                        out.push_str(&format!("| {pad}<!DOCTYPE {name} \"{p}\" \"{s}\">\n"));
                    }
                }
                NodeData::Document | NodeData::Fragment => {}
            }
        }
    }
    let mut out = String::new();
    walk(dom, Dom::ROOT, 0, &mut out);
    out
}

/// Parse an XHTML/XML document (served as application/xhtml+xml, or a .xhtml file). XML
/// has no implied tags or error recovery: `<div/>` is an empty element, end tags close
/// what they name, and namespaces come from `xmlns` attributes.
pub fn parse_xml(src: &str) -> Dom {
    let mut tk = Tokenizer::new(src);
    let mut dom = Dom::new();
    let mut open: Vec<(NodeId, Ns)> = Vec::new();
    let ns_for = |attrs: &[(String, String)], inherited: Ns| match attrs.iter().find(|(k, _)| k == "xmlns").map(|(_, v)| v.as_str()) {
        Some("http://www.w3.org/2000/svg") => Ns::Svg,
        Some("http://www.w3.org/1998/Math/MathML") => Ns::MathMl,
        Some(_) => Ns::Html,
        None => inherited,
    };
    loop {
        tk.allow_cdata = true;
        let parent = open.last().map_or(Dom::ROOT, |o| o.0);
        match tk.next_token() {
            Token::Eof => break,
            Token::Doctype(d) => {
                dom.add(NodeData::Doctype { name: d.name.unwrap_or_default(), public_id: d.public_id, system_id: d.system_id }, parent);
            }
            Token::Comment(c) => {
                dom.add(NodeData::Comment(c), parent);
            }
            // The XML declaration (<?xml …?>) isn't a node.
            Token::ProcessingInstruction { target, data } => {
                if !target.eq_ignore_ascii_case("xml") {
                    dom.add(NodeData::ProcessingInstruction { target, data }, parent);
                }
            }
            Token::Text(t) => {
                if parent == Dom::ROOT {
                    continue;
                }
                if let Some(&last) = dom.nodes[parent].children.last() {
                    if let NodeData::Text(s) = &mut dom.nodes[last].data {
                        s.push_str(&t);
                        continue;
                    }
                }
                dom.add(NodeData::Text(t), parent);
            }
            Token::StartTag(tag) => {
                let ns = ns_for(&tag.attrs, open.last().map_or(Ns::Html, |o| o.1));
                let name = if ns == Ns::Svg { adjust_svg_tag(&tag.name) } else { tag.name.clone() };
                let id = dom.add(NodeData::Element { tag: name, attrs: tag.attrs.clone() }, parent);
                if ns != Ns::Html {
                    dom.ns.insert(id, ns);
                }
                if !tag.self_closing {
                    open.push((id, ns));
                    // Raw text elements still hold text in XHTML, not markup, only when
                    // they're written that way; XML has no raw-text states, so stay in data.
                }
            }
            Token::EndTag(tag) => {
                if let Some(i) = open.iter().rposition(|&(n, _)| dom.tag(n).is_some_and(|t| t.eq_ignore_ascii_case(&tag.name))) {
                    open.truncate(i);
                }
            }
        }
    }
    dom
}

#[cfg(test)]
mod depth_tests {
    use super::*;

    #[test]
    fn nesting_is_capped_so_deep_pages_cannot_exhaust_the_stack() {
        let src = format!("{}deep{}", "<div>".repeat(20_000), "</div>".repeat(20_000));
        let dom = TreeBuilder::parse_document(&src, true);
        let mut max = 0;
        for id in 0..dom.nodes.len() {
            let (mut d, mut n) = (0, id);
            while let Some(p) = dom.nodes[n].parent {
                d += 1;
                n = p;
            }
            max = max.max(d);
        }
        assert!(max <= MAX_DEPTH + 2, "depth {max}");
        assert!(dom.text_content(Dom::ROOT).contains("deep"));
    }
}
