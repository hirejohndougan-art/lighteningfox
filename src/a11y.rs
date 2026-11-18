//! The accessibility tree: what a screen reader sees of a page. Built from the DOM, each
//! element's computed style (hidden things are left out) and its ARIA attributes, following
//! the HTML-AAM role mappings and a simplified accessible-name computation (accname 1.2).
//!
//! `build` makes the tree; `dump` prints it (`ironswift --a11y page.html`); `tab_order` is the
//! order keyboard focus moves through a page.

use crate::dom::{Dom, NodeData, NodeId};
use crate::forms::{self, Control};
use crate::style::{Display, Style};
use std::rc::Rc;

/// One node of the accessibility tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    /// The DOM node it comes from (text runs: the text node).
    pub dom: NodeId,
    /// The ARIA role: "link", "button", "heading", "text" (a run of text)…
    pub role: &'static str,
    /// The accessible name ("" if none).
    pub name: String,
    pub description: String,
    /// The current value (text fields, sliders, select boxes).
    pub value: Option<String>,
    pub states: States,
    /// Heading level, or level in a tree.
    pub level: Option<u32>,
    /// Range controls: min, max and current.
    pub range: Option<(f64, f64, f64)>,
    /// Links: where they go.
    pub href: Option<String>,
    /// Indexes into `Tree::nodes`.
    pub children: Vec<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct States {
    pub focusable: bool,
    pub focused: bool,
    pub disabled: bool,
    /// Checkboxes, radios, switches; `Some(None)` is "mixed".
    pub checked: Option<Option<bool>>,
    pub pressed: Option<bool>,
    pub expanded: Option<bool>,
    pub selected: Option<bool>,
    pub required: bool,
    pub readonly: bool,
    pub invalid: bool,
    pub multiline: bool,
    pub password: bool,
    /// aria-live: "polite" or "assertive".
    pub live: Option<&'static str>,
    pub modal: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Tree {
    /// `nodes[0]` is the document.
    pub nodes: Vec<Node>,
}

#[cfg(test)]
impl Tree {
    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }
    /// Every node with a role, depth first.
    pub fn with_role(&self, role: &str) -> Vec<&Node> {
        self.nodes.iter().filter(|n| n.role == role).collect()
    }
}

/// Every role we recognise in a `role` attribute (ARIA 1.2, minus the abstract ones).
const ROLES: &[&str] = &[
    "alert", "alertdialog", "application", "article", "banner", "blockquote", "button", "caption", "cell", "checkbox",
    "code", "columnheader", "combobox", "complementary", "contentinfo", "definition", "deletion", "dialog", "directory",
    "document", "emphasis", "feed", "figure", "form", "generic", "grid", "gridcell", "group", "heading", "img",
    "insertion", "link", "list", "listbox", "listitem", "log", "main", "marquee", "math", "menu", "menubar",
    "menuitem", "menuitemcheckbox", "menuitemradio", "meter", "navigation", "none", "note", "option", "paragraph",
    "presentation", "progressbar", "radio", "radiogroup", "region", "row", "rowgroup", "rowheader", "scrollbar",
    "search", "searchbox", "separator", "slider", "spinbutton", "status", "strong", "subscript", "superscript",
    "switch", "tab", "table", "tablist", "tabpanel", "term", "textbox", "time", "timer", "toolbar", "tooltip", "tree",
    "treegrid", "treeitem",
];

/// The role named `s` as one of ours (a tree sent from a site process): unknown ones are generic.
pub fn intern_role(s: &str) -> &'static str {
    match s {
        "text" => "text",
        "document" => "document",
        _ => ROLES.iter().find(|&&r| r == s).copied().unwrap_or("generic"),
    }
}

/// Roles whose name comes from their contents when nothing else names them.
const NAME_FROM_CONTENT: &[&str] = &[
    "button", "cell", "checkbox", "columnheader", "gridcell", "heading", "link", "menuitem", "menuitemcheckbox",
    "menuitemradio", "option", "radio", "row", "rowheader", "switch", "tab", "tooltip", "treeitem", "caption", "term",
];

/// Elements that never show and never reach the tree.
const NEVER_SHOWN: &[&str] = &["head", "script", "style", "template", "noscript", "title", "meta", "link", "base"];

/// Build the tree. `styles` are the page's computed styles (indexed by node id) when known;
/// without them only the `hidden` attribute and `aria-hidden` hide things.
pub fn build(dom: &Dom, styles: Option<&[Rc<Style>]>, focused: Option<NodeId>) -> Tree {
    let mut b = Builder { dom, styles, focused, tree: Tree::default() };
    let title = dom.find_all("title").first().map(|&t| squash(&dom.text_content(t))).unwrap_or_default();
    b.tree.nodes.push(Node { dom: Dom::ROOT, role: "document", name: title, ..Node::default() });
    let kids = b.children_of(Dom::ROOT);
    b.tree.nodes[0].children = kids;
    b.tree
}

struct Builder<'a> {
    dom: &'a Dom,
    styles: Option<&'a [Rc<Style>]>,
    focused: Option<NodeId>,
    tree: Tree,
}

impl Builder<'_> {
    fn hidden(&self, id: NodeId) -> bool {
        let d = self.dom;
        if let Some(t) = d.tag(id) {
            if NEVER_SHOWN.contains(&t) || d.attr(id, "hidden").is_some() || d.attr(id, "aria-hidden").is_some_and(|v| v.trim() == "true") {
                return true;
            }
            if t == "input" && d.attr(id, "type").is_some_and(|v| v.eq_ignore_ascii_case("hidden")) {
                return true;
            }
        }
        match self.styles.and_then(|s| s.get(id)) {
            Some(st) if d.tag(id).is_some() => st.display == Display::None,
            _ => false,
        }
    }

    /// visibility: hidden removes a box from the tree but its children may come back.
    fn invisible(&self, id: NodeId) -> bool {
        self.styles.and_then(|s| s.get(id)).is_some_and(|st| st.hidden)
    }

    /// The tree nodes for a DOM node's children (generic wrappers are passed through).
    fn children_of(&mut self, id: NodeId) -> Vec<usize> {
        let mut out = Vec::new();
        let kids: Vec<NodeId> = match self.dom.shadow_roots.get(&id) {
            Some(&root) => self.dom.nodes[root].children.clone(),
            None => self.dom.nodes[id].children.clone(),
        };
        for c in kids {
            self.add(c, &mut out);
        }
        out
    }

    fn add(&mut self, id: NodeId, out: &mut Vec<usize>) {
        let d = self.dom;
        match &d.nodes[id].data {
            NodeData::Text(t) => {
                let s = squash(t);
                if !s.is_empty() && !self.invisible(id) && !self.parent_invisible(id) {
                    // Neighbouring text runs (split by generic inline wrappers) join up.
                    if let Some(&last) = out.last() {
                        let prev = &mut self.tree.nodes[last];
                        if prev.role == "text" {
                            prev.name.push(' ');
                            prev.name.push_str(&s);
                            return;
                        }
                    }
                    out.push(self.push(Node { dom: id, role: "text", name: s, ..Node::default() }));
                }
            }
            NodeData::Element { .. } => {
                if self.hidden(id) {
                    return;
                }
                let role = role_of(d, id);
                let generic = matches!(role, "generic" | "none" | "presentation") && !self.is_focusable(id) && !has_global_aria(d, id);
                if generic || self.invisible(id) {
                    let kids = self.children_of(id);
                    for k in kids {
                        // Merge text across the dropped wrapper.
                        if let (Some(&last), true) = (out.last(), self.tree.nodes[k].role == "text") {
                            if self.tree.nodes[last].role == "text" {
                                let s = self.tree.nodes[k].name.clone();
                                let prev = &mut self.tree.nodes[last];
                                prev.name.push(' ');
                                prev.name.push_str(&s);
                                continue;
                            }
                        }
                        out.push(k);
                    }
                    return;
                }
                let node = self.describe(id, role);
                let idx = self.push(node);
                // A <select> lists its options; these other roles present their contents as
                // their name or value, so they are leaves.
                let kids = if d.tag(id) == Some("select") {
                    self.options(id)
                } else if matches!(role, "img" | "textbox" | "searchbox" | "combobox" | "slider" | "spinbutton" | "progressbar" | "meter" | "separator" | "checkbox" | "radio" | "switch") {
                    Vec::new()
                } else {
                    self.children_of(id)
                };
                self.tree.nodes[idx].children = kids;
                out.push(idx);
            }
            _ => {}
        }
    }

    fn parent_invisible(&self, id: NodeId) -> bool {
        self.dom.nodes[id].parent.is_some_and(|p| self.invisible(p))
    }

    fn options(&mut self, select: NodeId) -> Vec<usize> {
        let d = self.dom;
        let cur = forms::selected_option(d, select);
        let opts: Vec<NodeId> = forms::options(d, select).into_iter().filter(|&o| !self.hidden(o)).collect();
        opts.into_iter()
            .map(|o| {
                let mut n = Node { dom: o, role: "option", name: squash(&forms::option_label(d, o)), ..Node::default() };
                n.states.selected = Some(cur == Some(o));
                n.states.disabled = d.attr(o, "disabled").is_some();
                self.push(n)
            })
            .collect()
    }

    fn push(&mut self, n: Node) -> usize {
        self.tree.nodes.push(n);
        self.tree.nodes.len() - 1
    }

    fn is_focusable(&self, id: NodeId) -> bool {
        is_focusable(self.dom, id)
    }

    fn describe(&self, id: NodeId, role: &'static str) -> Node {
        let d = self.dom;
        let tag = d.tag(id).unwrap_or("");
        let attr = |n: &str| d.attr(id, n);
        let aria_bool = |n: &str| attr(n).map(|v| v.trim().eq_ignore_ascii_case("true"));
        let mut n = Node { dom: id, role, ..Node::default() };
        n.name = name_of(d, id, role, self.styles);
        n.description = match attr("aria-describedby") {
            Some(ids) => squash(&ids.split_whitespace().filter_map(|i| by_id(d, i)).map(|e| text_of(d, e, self.styles, 0)).collect::<Vec<_>>().join(" ")),
            None => match (attr("title"), attr("aria-label").is_some() || attr("aria-labelledby").is_some()) {
                // A title that didn't become the name describes the element.
                (Some(t), _) if squash(t) != n.name => squash(t),
                _ => String::new(),
            },
        };
        let s = &mut n.states;
        s.focusable = self.is_focusable(id);
        s.focused = self.focused == Some(id);
        s.disabled = forms::is_disabled(d, id) || aria_bool("aria-disabled") == Some(true);
        s.required = attr("required").is_some() || aria_bool("aria-required") == Some(true);
        s.readonly = attr("readonly").is_some() || aria_bool("aria-readonly") == Some(true);
        s.invalid = attr("aria-invalid").is_some_and(|v| !matches!(v.trim(), "" | "false"));
        s.expanded = aria_bool("aria-expanded");
        s.selected = aria_bool("aria-selected");
        s.modal = aria_bool("aria-modal") == Some(true);
        s.pressed = attr("aria-pressed").map(|v| v.trim() == "true" || v.trim() == "mixed");
        s.live = match attr("aria-live").map(str::trim) {
            Some("polite") => Some("polite"),
            Some("assertive") => Some("assertive"),
            _ => match role {
                "alert" => Some("assertive"),
                "status" | "log" => Some("polite"),
                _ => None,
            },
        };
        if tag == "details" {
            s.expanded = Some(attr("open").is_some());
        }
        if tag == "summary" {
            s.expanded = d.nodes[id].parent.map(|p| d.attr(p, "open").is_some());
        }
        // Checked state: native first, then aria-checked.
        let native = match forms::control(d, id) {
            Some(Control::Checkbox | Control::Radio) if tag == "input" => Some(Some(attr("checked").is_some())),
            _ => None,
        };
        let aria = attr("aria-checked").map(|v| match v.trim() {
            "true" => Some(true),
            "mixed" => None,
            _ => Some(false),
        });
        s.checked = native.or(aria).or(match role {
            "checkbox" | "radio" | "switch" | "menuitemcheckbox" | "menuitemradio" => Some(Some(false)),
            _ => None,
        });
        // Values.
        match forms::control(d, id) {
            Some(Control::Text { password }) => {
                s.password = password;
                n.value = Some(if password { "•".repeat(forms::value(d, id).chars().count()) } else { forms::value(d, id) });
            }
            Some(Control::TextArea) => {
                s.multiline = true;
                n.value = Some(forms::value(d, id));
            }
            Some(Control::Select) => {
                n.value = forms::selected_option(d, id).map(|o| squash(&forms::option_label(d, o)));
            }
            _ => {}
        }
        if attr("contenteditable").is_some_and(|v| !v.eq_ignore_ascii_case("false")) {
            n.value = Some(squash(&d.text_content(id)));
            s.multiline = true;
        }
        if role == "heading" {
            n.level = attr("aria-level")
                .and_then(|v| v.trim().parse().ok())
                .or_else(|| tag.strip_prefix('h').and_then(|l| l.parse().ok()).filter(|l| (1..=6).contains(l)))
                .or(Some(2));
        } else if let Some(l) = attr("aria-level").and_then(|v| v.trim().parse().ok()) {
            n.level = Some(l);
        }
        if matches!(role, "slider" | "spinbutton" | "progressbar" | "meter" | "scrollbar") {
            let num = |a: &str| attr(a).and_then(|v| v.trim().parse::<f64>().ok());
            let (min, max) = match tag {
                "progress" => (0.0, num("max").unwrap_or(1.0)),
                _ => (num("aria-valuemin").or(num("min")).unwrap_or(0.0), num("aria-valuemax").or(num("max")).unwrap_or(100.0)),
            };
            let now = num("aria-valuenow").or(num("value")).unwrap_or(if tag == "input" { (min + max) / 2.0 } else { min });
            n.range = Some((min, max, now));
            if let Some(t) = attr("aria-valuetext") {
                n.value = Some(squash(t));
            }
        }
        if role == "link" {
            n.href = attr("href").map(str::to_string);
        }
        n
    }
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn by_id(d: &Dom, id: &str) -> Option<NodeId> {
    d.descendants(Dom::ROOT).into_iter().find(|&n| d.attr(n, "id") == Some(id))
}

fn has_global_aria(d: &Dom, id: NodeId) -> bool {
    ["aria-label", "aria-labelledby", "aria-describedby", "aria-live"].iter().any(|a| d.attr(id, a).is_some())
}

/// The element's role: an explicit `role` attribute (first one we know), else the native one.
pub fn role_of(d: &Dom, id: NodeId) -> &'static str {
    if let Some(r) = d.attr(id, "role") {
        for tok in r.split_whitespace() {
            let tok = tok.to_ascii_lowercase();
            if let Some(&known) = ROLES.iter().find(|&&k| k == tok) {
                // Focusable elements can't be made presentational (ARIA's conflict rule).
                if matches!(known, "none" | "presentation") && is_focusable(d, id) {
                    break;
                }
                return known;
            }
        }
    }
    native_role(d, id)
}

fn native_role(d: &Dom, id: NodeId) -> &'static str {
    let tag = d.tag(id).unwrap_or("");
    let attr = |n: &str| d.attr(id, n);
    let inside = |tags: &[&str]| {
        let mut p = d.nodes[id].parent;
        while let Some(x) = p {
            if d.tag(x).is_some_and(|t| tags.contains(&t)) {
                return true;
            }
            p = d.nodes[x].parent;
        }
        false
    };
    match tag {
        "a" | "area" if attr("href").is_some() => "link",
        "button" => "button",
        "summary" => "button",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "img" => match attr("alt") {
            Some(a) if a.is_empty() => "presentation",
            _ => "img",
        },
        "svg" => "img",
        "input" => match attr("type").unwrap_or("text").trim().to_ascii_lowercase().as_str() {
            "checkbox" => {
                if attr("switch").is_some() {
                    "switch"
                } else {
                    "checkbox"
                }
            }
            "radio" => "radio",
            "range" => "slider",
            "number" => "spinbutton",
            "search" => {
                if attr("list").is_some() {
                    "combobox"
                } else {
                    "searchbox"
                }
            }
            "submit" | "reset" | "button" | "image" => "button",
            "text" | "email" | "tel" | "url" | "" | "password" => {
                if attr("list").is_some() {
                    "combobox"
                } else {
                    "textbox"
                }
            }
            "hidden" => "none",
            _ => "textbox",
        },
        "textarea" => "textbox",
        "select" => {
            if attr("multiple").is_some() || attr("size").and_then(|s| s.trim().parse::<u32>().ok()).is_some_and(|s| s > 1) {
                "listbox"
            } else {
                "combobox"
            }
        }
        "option" => "option",
        "optgroup" => "group",
        "ul" | "ol" | "menu" => "list",
        "li" => "listitem",
        "dl" => "list",
        "dt" => "term",
        "dd" => "definition",
        "nav" => "navigation",
        "main" => "main",
        "header" if !inside(&["article", "aside", "main", "nav", "section"]) => "banner",
        "footer" if !inside(&["article", "aside", "main", "nav", "section"]) => "contentinfo",
        "aside" => "complementary",
        "search" => "search",
        "form" if attr("aria-label").is_some() || attr("aria-labelledby").is_some() || attr("name").is_some() => "form",
        "section" if attr("aria-label").is_some() || attr("aria-labelledby").is_some() => "region",
        "article" => "article",
        "table" => "table",
        "caption" => "caption",
        "thead" | "tbody" | "tfoot" => "rowgroup",
        "tr" => "row",
        "td" => "cell",
        "th" => {
            if attr("scope").is_some_and(|s| s.eq_ignore_ascii_case("row")) {
                "rowheader"
            } else {
                "columnheader"
            }
        }
        "dialog" => "dialog",
        "p" => "paragraph",
        "hr" => "separator",
        "fieldset" | "details" => "group",
        "progress" => "progressbar",
        "meter" => "meter",
        "figure" => "figure",
        "blockquote" => "blockquote",
        "code" => "code",
        "em" => "emphasis",
        "strong" => "strong",
        "del" | "s" => "deletion",
        "ins" => "insertion",
        "sub" => "subscript",
        "sup" => "superscript",
        "time" => "time",
        "output" => "status",
        "math" => "math",
        "iframe" => "document",
        "html" => "generic",
        _ => "generic",
    }
}

/// Whether keyboard focus can land on an element (by Tab or by click).
pub fn is_focusable(d: &Dom, id: NodeId) -> bool {
    let Some(tag) = d.tag(id) else { return false };
    if d.attr(id, "tabindex").and_then(|v| v.trim().parse::<i32>().ok()).is_some() {
        return !forms::is_disabled(d, id);
    }
    match tag {
        "a" | "area" => d.attr(id, "href").is_some(),
        "input" | "select" | "textarea" | "button" => forms::control(d, id).is_some() && !forms::is_disabled(d, id),
        "summary" => d.nodes[id].parent.is_some_and(|p| d.tag(p) == Some("details")),
        "iframe" => true,
        _ => d.attr(id, "contenteditable").is_some_and(|v| !v.eq_ignore_ascii_case("false")),
    }
}

/// The order Tab moves through: positive `tabindex` values first (ascending, ties in document
/// order), then everything else focusable in document order. `tabindex="-1"` is skipped, as
/// is anything hidden.
pub fn tab_order(d: &Dom, styles: Option<&[Rc<Style>]>) -> Vec<NodeId> {
    let mut pos: Vec<(i32, usize, NodeId)> = Vec::new();
    let mut zero = Vec::new();
    let mut i = 0;
    walk_visible(d, Dom::ROOT, styles, &mut |n| {
        if !is_focusable(d, n) {
            return;
        }
        let ti = d.attr(n, "tabindex").and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0);
        if ti > 0 {
            pos.push((ti, i, n));
        } else if ti == 0 {
            zero.push(n);
        }
        i += 1;
    });
    pos.sort();
    pos.into_iter().map(|p| p.2).chain(zero).collect()
}

fn walk_visible(d: &Dom, id: NodeId, styles: Option<&[Rc<Style>]>, f: &mut impl FnMut(NodeId)) {
    if let Some(t) = d.tag(id) {
        if NEVER_SHOWN.contains(&t) || d.attr(id, "hidden").is_some() {
            return;
        }
        if styles.and_then(|s| s.get(id)).is_some_and(|st| st.display == Display::None) {
            return;
        }
        if !styles.and_then(|s| s.get(id)).is_some_and(|st| st.hidden) {
            f(id);
        }
        // Closed <details>: only its summary can be reached.
        if t == "details" && d.attr(id, "open").is_none() {
            if let Some(&s) = d.nodes[id].children.iter().find(|&&c| d.tag(c) == Some("summary")) {
                walk_visible(d, s, styles, f);
            }
            return;
        }
    }
    let kids = match d.shadow_roots.get(&id) {
        Some(&r) => &d.nodes[r].children,
        None => &d.nodes[id].children,
    };
    for &c in kids {
        walk_visible(d, c, styles, f);
    }
}

// ---------------------------------------------------------------- accessible names

fn name_of(d: &Dom, id: NodeId, role: &str, styles: Option<&[Rc<Style>]>) -> String {
    let attr = |n: &str| d.attr(id, n);
    // 1. aria-labelledby
    if let Some(ids) = attr("aria-labelledby") {
        let s = squash(&ids.split_whitespace().filter_map(|i| by_id(d, i)).map(|e| text_of(d, e, styles, 0)).collect::<Vec<_>>().join(" "));
        if !s.is_empty() {
            return s;
        }
    }
    // 2. aria-label
    if let Some(l) = attr("aria-label").map(squash).filter(|l| !l.is_empty()) {
        return l;
    }
    // 3. Native labelling.
    let tag = d.tag(id).unwrap_or("");
    let native = match tag {
        "input" | "select" | "textarea" | "meter" | "progress" | "output" => {
            let ty = attr("type").unwrap_or("").to_ascii_lowercase();
            match ty.as_str() {
                "submit" | "reset" | "button" => attr("value").map(squash).filter(|v| !v.is_empty()).or_else(|| {
                    Some(match ty.as_str() {
                        "submit" => "Submit".to_string(),
                        "reset" => "Reset".to_string(),
                        _ => String::new(),
                    })
                }),
                "image" => attr("alt").or(attr("value")).map(squash).or(Some("Submit".into())),
                _ => labels_for(d, id, styles).or_else(|| attr("placeholder").map(squash)),
            }
        }
        "img" | "area" => attr("alt").map(squash),
        "fieldset" => d.nodes[id].children.iter().find(|&&c| d.tag(c) == Some("legend")).map(|&l| text_of(d, l, styles, 0)),
        "table" => d.nodes[id].children.iter().find(|&&c| d.tag(c) == Some("caption")).map(|&c| text_of(d, c, styles, 0)),
        "figure" => d.nodes[id].children.iter().find(|&&c| d.tag(c) == Some("figcaption")).map(|&c| text_of(d, c, styles, 0)),
        "details" => d.nodes[id].children.iter().find(|&&c| d.tag(c) == Some("summary")).map(|&c| text_of(d, c, styles, 0)),
        "svg" => d.nodes[id].children.iter().find(|&&c| d.tag(c) == Some("title")).map(|&c| squash(&d.text_content(c))),
        "iframe" => None,
        _ => None,
    };
    if let Some(n) = native.filter(|n| !n.is_empty()) {
        return n;
    }
    // 4. Contents, for roles named by them.
    if NAME_FROM_CONTENT.contains(&role) {
        let s = text_of(d, id, styles, 0);
        if !s.is_empty() {
            return s;
        }
    }
    // 5. title
    attr("title").map(squash).unwrap_or_default()
}

/// The text of <label>s for a control: `<label for=id>` and a wrapping label.
fn labels_for(d: &Dom, id: NodeId, styles: Option<&[Rc<Style>]>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(my) = d.attr(id, "id") {
        for l in d.find_all("label") {
            if d.attr(l, "for") == Some(my) {
                parts.push(label_text(d, l, id, styles));
            }
        }
    }
    let mut p = d.nodes[id].parent;
    while let Some(x) = p {
        if d.tag(x) == Some("label") && d.attr(x, "for").is_none() {
            parts.push(label_text(d, x, id, styles));
            break;
        }
        p = d.nodes[x].parent;
    }
    let s = squash(&parts.join(" "));
    (!s.is_empty()).then_some(s)
}

/// A label's text, leaving out the control itself (whose value would otherwise be read twice).
fn label_text(d: &Dom, label: NodeId, control: NodeId, styles: Option<&[Rc<Style>]>) -> String {
    let mut out = String::new();
    for &c in &d.nodes[label].children {
        if c != control {
            out.push_str(&text_of(d, c, styles, 1));
            out.push(' ');
        }
    }
    squash(&out)
}

/// The text a subtree contributes to a name: text, image alts, embedded controls' values,
/// children's aria-labels. Hidden parts are skipped.
fn text_of(d: &Dom, id: NodeId, styles: Option<&[Rc<Style>]>, depth: usize) -> String {
    if depth > 40 {
        return String::new();
    }
    match &d.nodes[id].data {
        NodeData::Text(t) => t.clone(),
        NodeData::Element { tag, .. } => {
            let hidden = NEVER_SHOWN.contains(&tag.as_str())
                || d.attr(id, "hidden").is_some()
                || d.attr(id, "aria-hidden").is_some_and(|v| v.trim() == "true")
                || styles.and_then(|s| s.get(id)).is_some_and(|st| st.display == Display::None || st.hidden);
            // aria-labelledby may point at hidden text, which still counts (depth 0 only).
            if hidden && depth > 0 {
                return String::new();
            }
            if depth > 0 {
                if let Some(l) = d.attr(id, "aria-label").filter(|l| !l.trim().is_empty()) {
                    return format!(" {} ", l.trim());
                }
                match forms::control(d, id) {
                    Some(Control::Text { .. } | Control::TextArea) => return format!(" {} ", forms::value(d, id)),
                    Some(Control::Select) => return format!(" {} ", forms::selected_option(d, id).map(|o| forms::option_label(d, o)).unwrap_or_default()),
                    _ => {}
                }
            }
            if tag == "img" {
                return format!(" {} ", d.attr(id, "alt").unwrap_or(""));
            }
            let block = !matches!(tag.as_str(), "a" | "span" | "b" | "i" | "em" | "strong" | "code" | "small" | "sub" | "sup" | "abbr" | "label" | "mark" | "q" | "s" | "u" | "time");
            let mut out = String::new();
            if block {
                out.push(' ');
            }
            for &c in &d.nodes[id].children {
                out.push_str(&text_of(d, c, styles, depth + 1));
            }
            if block {
                out.push(' ');
            }
            if depth == 0 {
                squash(&out)
            } else {
                out
            }
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------- printing

/// The tree as indented text, one node per line: `role "name" value=… [states]`.
pub fn dump(t: &Tree) -> String {
    let mut out = String::new();
    dump_node(t, 0, 0, &mut out);
    out
}

fn dump_node(t: &Tree, i: usize, depth: usize, out: &mut String) {
    let n = &t.nodes[i];
    out.push_str(&"  ".repeat(depth));
    out.push_str(n.role);
    if !n.name.is_empty() {
        out.push_str(&format!(" {:?}", n.name));
    }
    if let Some(l) = n.level {
        out.push_str(&format!(" level={l}"));
    }
    if let Some(v) = &n.value {
        out.push_str(&format!(" value={v:?}"));
    }
    if let Some((min, max, now)) = n.range {
        out.push_str(&format!(" range={min}..{max} now={now}"));
    }
    if let Some(h) = &n.href {
        out.push_str(&format!(" href={h:?}"));
    }
    if !n.description.is_empty() {
        out.push_str(&format!(" description={:?}", n.description));
    }
    let s = &n.states;
    let mut flags: Vec<String> = Vec::new();
    let mut flag = |on: bool, name: &str| {
        if on {
            flags.push(name.to_string());
        }
    };
    flag(s.focusable, "focusable");
    flag(s.focused, "focused");
    flag(s.disabled, "disabled");
    flag(s.required, "required");
    flag(s.readonly, "readonly");
    flag(s.invalid, "invalid");
    flag(s.multiline, "multiline");
    flag(s.password, "password");
    flag(s.modal, "modal");
    match s.checked {
        Some(Some(true)) => flags.push("checked".into()),
        Some(Some(false)) => flags.push("unchecked".into()),
        Some(None) => flags.push("mixed".into()),
        None => {}
    }
    match s.expanded {
        Some(true) => flags.push("expanded".into()),
        Some(false) => flags.push("collapsed".into()),
        None => {}
    }
    if let Some(p) = s.pressed {
        flags.push(if p { "pressed" } else { "not-pressed" }.into());
    }
    if s.selected == Some(true) {
        flags.push("selected".into());
    }
    if let Some(l) = s.live {
        flags.push(format!("live={l}"));
    }
    if !flags.is_empty() {
        out.push_str(&format!(" [{}]", flags.join(" ")));
    }
    out.push('\n');
    for &c in &n.children {
        dump_node(t, c, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(html: &str) -> Tree {
        build(&crate::html::parse(html), None, None)
    }

    #[test]
    fn landmarks_headings_links() {
        let t = tree(
            "<title>Home</title><header><nav aria-label=Main><a href=/a>About <b>us</b></a></nav></header>\
             <main><h1>Welcome</h1><h3 aria-level=4>Sub</h3><p>Hello <span>world</span></p></main><footer>c</footer>",
        );
        assert_eq!(t.root().role, "document");
        assert_eq!(t.root().name, "Home");
        let d = dump(&t);
        assert!(d.contains("banner\n"), "{d}");
        assert!(d.contains("navigation \"Main\""), "{d}");
        assert!(d.contains("link \"About us\" href=\"/a\" [focusable]"), "{d}");
        assert!(d.contains("heading \"Welcome\" level=1"), "{d}");
        assert!(d.contains("heading \"Sub\" level=4"), "{d}");
        assert!(d.contains("text \"Hello world\""), "{d}");
        assert!(d.contains("contentinfo"), "{d}");
    }

    #[test]
    fn hidden_things_are_left_out() {
        let t = tree("<p hidden>a</p><div aria-hidden=true><button>b</button></div><script>x</script><button>c</button><input type=hidden>");
        let d = dump(&t);
        assert!(!d.contains("\"a\"") && !d.contains("\"b\"") && !d.contains("\"x\""), "{d}");
        assert!(d.contains("button \"c\""), "{d}");
    }

    #[test]
    fn form_labels_and_states() {
        let t = tree(
            "<label for=e>Email</label><input id=e type=email value=me@x required>\
             <label><input type=checkbox checked> Remember me</label>\
             <input placeholder=Search type=search>\
             <select><option>One<option selected>Two</select>\
             <input type=submit><button disabled>Go</button>\
             <input type=range min=0 max=10 value=3 aria-label=Volume>\
             <div role=checkbox aria-checked=mixed tabindex=0>All</div>",
        );
        let d = dump(&t);
        assert!(d.contains("textbox \"Email\" value=\"me@x\" [focusable required]"), "{d}");
        assert!(d.contains("checkbox \"Remember me\" [focusable checked]"), "{d}");
        assert!(d.contains("searchbox \"Search\""), "{d}");
        assert!(d.contains("combobox value=\"Two\""), "{d}");
        assert!(d.contains("option \"Two\" [selected]"), "{d}");
        assert!(d.contains("button \"Submit\""), "{d}");
        assert!(d.contains("button \"Go\" [disabled]"), "{d}");
        assert!(d.contains("slider \"Volume\" range=0..10 now=3"), "{d}");
        assert!(d.contains("checkbox \"All\" [focusable mixed]"), "{d}");
    }

    #[test]
    fn aria_names_and_descriptions() {
        let t = tree(
            "<span id=l1>Delete</span><span id=l2>file</span><p id=h>Cannot be undone</p>\
             <button aria-labelledby='l1 l2' aria-describedby=h>X</button>\
             <img src=a.png alt='A cat'><img src=b.png alt=''>\
             <div role=alert>Saved</div><button aria-expanded=false aria-pressed=true title=More>⋯</button>",
        );
        let d = dump(&t);
        assert!(d.contains("button \"Delete file\" description=\"Cannot be undone\""), "{d}");
        assert!(d.contains("img \"A cat\""), "{d}");
        assert_eq!(t.with_role("img").len(), 1, "{d}");
        assert!(d.contains("alert [live=assertive]"), "{d}");
        assert!(d.contains("button \"⋯\" description=\"More\" [focusable collapsed pressed]"), "{d}");
    }

    #[test]
    fn tables_lists_details() {
        let t = tree(
            "<table><caption>Prices</caption><tr><th>Item<th>Cost<tr><td>Tea<td>$2</table>\
             <ul><li>One<li>Two</ul><details><summary>More</summary>Inside</details>",
        );
        let d = dump(&t);
        assert!(d.contains("table \"Prices\""), "{d}");
        assert!(d.contains("columnheader \"Item\""), "{d}");
        assert!(d.contains("cell \"Tea\""), "{d}");
        assert!(d.contains("list\n"), "{d}");
        assert_eq!(t.with_role("listitem").len(), 2);
        assert!(d.contains("group \"More\" [collapsed]"), "{d}");
        assert!(d.contains("button \"More\" [focusable collapsed]"), "{d}");
    }

    #[test]
    fn tab_order_follows_tabindex() {
        let dom = crate::html::parse(
            "<a href=#1 id=a>a</a><button id=b tabindex=2>b</button><input id=c tabindex=-1>\
             <span id=d tabindex=0>d</span><button id=e tabindex=1>e</button><button disabled id=f>f</button>\
             <div hidden><a href=#x id=g>g</a></div><details><summary id=h>s</summary><a href=#y id=i>i</a></details>",
        );
        let ids: Vec<&str> = tab_order(&dom, None).iter().map(|&n| dom.attr(n, "id").unwrap_or("?")).collect();
        assert_eq!(ids, ["e", "b", "a", "d", "h"]);
    }

    #[test]
    fn role_attribute_overrides() {
        let t = tree("<div role='bogus tab' aria-selected=true>One</div><ul role=presentation><li>x</li></ul><a href=# role=none>keep</a>");
        let d = dump(&t);
        assert!(d.contains("tab \"One\" [selected]"), "{d}");
        assert!(d.contains("link \"keep\""), "{d}");
    }
}
