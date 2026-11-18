//! The page's stylesheets, kept in step with its `<style>` and `<link rel="stylesheet">`
//! elements while scripts run.
//!
//! Stylesheets are first gathered when the page is fetched (main.rs, in parallel with
//! scripts). After that, pages add and change them all the time: frameworks insert `<style>`
//! and `<link>` elements, CSS-in-JS libraries (Emotion, styled-components) add rules with
//! `sheet.insertRule()`. Each restyle compares a signature of those elements with the last one
//! and, if it changed, gathers the list again, reusing sheets it has already parsed or fetched.
//! Rules added through the CSSOM are kept per element ([`set_cssom`]) and used in place of the
//! element's text for as long as that text doesn't change.

use crate::css::{self, Stylesheet};
use crate::dom::{Dom, NodeData, NodeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    /// CSS text a page's scripts gave a `<style>` element through its CSSOM sheet, with a hash
    /// of the element's text it was based on.
    static CSSOM: RefCell<HashMap<NodeId, (u64, String)>> = RefCell::new(HashMap::new());
    /// The rules of the constructed sheets in `document.adoptedStyleSheets` (and shadow
    /// roots'), which come after the document's own sheets.
    static ADOPTED: RefCell<String> = const { RefCell::new(String::new()) };
    /// The same for each shadow root (by its id), styling only its own tree.
    static ADOPTED_SHADOW: RefCell<HashMap<NodeId, String>> = RefCell::new(HashMap::new());
}

/// A new page: forget the last one's CSSOM rules.
pub fn reset() {
    CSSOM.with(|m| m.borrow_mut().clear());
    ADOPTED.with(|a| a.borrow_mut().clear());
    ADOPTED_SHADOW.with(|a| a.borrow_mut().clear());
}

/// A shadow root's adopted sheets' CSS text (cssom.js); empty removes them.
pub fn set_adopted_shadow(root: NodeId, css: &str) {
    ADOPTED_SHADOW.with(|a| {
        let mut a = a.borrow_mut();
        if css.is_empty() {
            a.remove(&root);
        } else {
            a.insert(root, css.to_string());
        }
    });
}

/// Every shadow root's adopted CSS (style.rs adds it to the root's own sheets).
pub fn adopted_shadow() -> Vec<(NodeId, String)> {
    ADOPTED_SHADOW.with(|a| a.borrow().iter().map(|(k, v)| (*k, v.clone())).collect())
}

/// The CSS text of every adopted constructed stylesheet, in order (cssom.js).
pub fn set_adopted(css: &str) {
    ADOPTED.with(|a| *a.borrow_mut() = css.to_string());
}

/// A `<style>` element's rules as the page's scripts last left them (sheet.insertRule…).
/// `based_on` is the element's text at that time; if the text changes, its rules win again.
pub fn set_cssom(id: NodeId, based_on: &str, css: &str) {
    CSSOM.with(|m| m.borrow_mut().insert(id, (hash(based_on), css.to_string())));
}

fn hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// One stylesheet source in the document.
#[derive(Clone, PartialEq)]
enum Source {
    Inline { text: String, nonce: Option<String> },
    /// Adopted constructed stylesheets: made by script, so not an inline style CSP checks.
    Adopted { text: String },
    External { url: String, media: String, nonce: Option<String> },
}

/// The live stylesheet list of one page.
pub struct Live {
    base: String,
    policy: std::sync::Arc<crate::security::PagePolicy>,
    sig: u64,
    sheets: Rc<Vec<Stylesheet>>,
    inline_cache: HashMap<u64, Stylesheet>,
    external_cache: HashMap<(String, String), Option<Stylesheet>>,
    /// Changed since the page last took the list (scripts' layouts refresh it too).
    unseen: bool,
}

impl Live {
    /// Start from the sheets gathered at load, for the document as it was then.
    pub fn new(base: &str, policy: std::sync::Arc<crate::security::PagePolicy>, dom: &Dom, sheets: Vec<Stylesheet>) -> Live {
        let sig = signature(&sources(dom, base)) ^ hash(&crate::blocklist::hiding_css(base, dom)).rotate_left(1);
        Live { base: base.to_string(), policy, sig, sheets: Rc::new(sheets), inline_cache: HashMap::new(), external_cache: HashMap::new(), unseen: false }
    }

    pub fn sheets(&self) -> Rc<Vec<Stylesheet>> {
        self.sheets.clone()
    }

    /// Gather the stylesheets again if the page's style elements changed. Returns true if the
    /// list changed.
    pub fn refresh(&mut self, dom: &Dom) -> bool {
        let srcs = sources(dom, &self.base);
        // Element hiding depends on which classes, ids and attributes the page has now: an
        // ad a script inserts later gets hidden too.
        let hiding = crate::blocklist::hiding_css(&self.base, dom);
        let sig = signature(&srcs) ^ hash(&hiding).rotate_left(1);
        if sig == self.sig {
            return false;
        }
        self.sig = sig;
        let mut out = Vec::with_capacity(srcs.len() + 2);
        let csp = &self.policy.csp;
        let page = self.base.clone();
        for s in &srcs {
            match s {
                Source::Inline { text, nonce } => {
                    if !csp.allows_inline(crate::security::Inline::Style, text, nonce.as_deref()) {
                        continue;
                    }
                    let key = hash(text);
                    let sheet = self.inline_cache.entry(key).or_insert_with(|| {
                        let mut sheet = css::parse(text);
                        css::absolutize_urls(&mut sheet, &page);
                        css::load_imports(&mut sheet, &page, &|u| crate::net::fetch_sub_nonce(u, &page, crate::security::Dest::Style, None).ok().map(|(_, t)| t), 0);
                        sheet
                    });
                    out.push(sheet.clone());
                }
                Source::Adopted { text } => {
                    let key = hash(text);
                    let sheet = self.inline_cache.entry(key).or_insert_with(|| {
                        let mut sheet = css::parse(text);
                        css::absolutize_urls(&mut sheet, &page);
                        sheet
                    });
                    out.push(sheet.clone());
                }
                Source::External { url, media, nonce } => {
                    let sheet = self.external_cache.entry((url.clone(), media.clone())).or_insert_with(|| {
                        let (_, text) = crate::net::fetch_sub_nonce(url, &page, crate::security::Dest::Style, nonce.as_deref()).ok()?;
                        let mut sheet = css::parse(&text);
                        css::absolutize_urls(&mut sheet, url);
                        css::load_imports(&mut sheet, url, &|v| crate::net::fetch_sub_nonce(v, &page, crate::security::Dest::Style, nonce.as_deref()).ok().map(|(_, t)| t), 0);
                        css::wrap_media(&mut sheet, media);
                        Some(sheet)
                    });
                    if let Some(sheet) = sheet {
                        out.push(sheet.clone());
                    }
                }
            }
        }
        // As at load: what's left of blocked ads is hidden, and extensions add their styles.
        if !hiding.is_empty() {
            let key = hash(&hiding);
            let sheet = self.inline_cache.entry(key).or_insert_with(|| css::parse(&hiding));
            out.push(sheet.clone());
        }
        for text in crate::extensions::content::page_css(&self.base, crate::security::frame_info(&self.base).is_none()) {
            out.push(css::parse(&text));
        }
        self.sheets = Rc::new(out);
        self.unseen = true;
        true
    }

    /// Refresh, and say whether the list changed since the last call to this (whoever
    /// refreshed it in between).
    pub fn refresh_for_page(&mut self, dom: &Dom) -> bool {
        self.refresh(dom);
        std::mem::take(&mut self.unseen)
    }
}

/// The document's style sources in tree order (what main.rs gathers at load, the same way).
fn sources(dom: &Dom, base: &str) -> Vec<Source> {
    let mut out = Vec::new();
    let mut stack = vec![Dom::ROOT];
    while let Some(id) = stack.pop() {
        if let NodeData::Element { tag, .. } = &dom.nodes[id].data {
            match tag.as_str() {
                "template" => continue,
                "style" => {
                    let text = dom.text_content(id);
                    let text = CSSOM.with(|m| match m.borrow().get(&id) {
                        Some((based_on, css)) if *based_on == hash(&text) => css.clone(),
                        _ => text,
                    });
                    out.push(Source::Inline { text, nonce: dom.attr(id, "nonce").map(String::from) });
                }
                "link" => {
                    let rel = dom.attr(id, "rel").unwrap_or("").to_ascii_lowercase();
                    let stylesheet = rel.split_whitespace().any(|r| r == "stylesheet") && !rel.split_whitespace().any(|r| r == "alternate");
                    if let (true, Some(href), false) = (stylesheet, dom.attr(id, "href"), dom.attr(id, "disabled").is_some()) {
                        let media = dom.attr(id, "media").unwrap_or("all").to_string();
                        if !(media.contains("print") && !media.contains("screen")) {
                            out.push(Source::External { url: crate::net::resolve(base, href), media, nonce: dom.attr(id, "nonce").map(String::from) });
                        }
                    }
                }
                _ => {}
            }
        }
        stack.extend(dom.nodes[id].children.iter().rev().copied());
    }
    let adopted = ADOPTED.with(|a| a.borrow().clone());
    if !adopted.is_empty() {
        out.push(Source::Adopted { text: adopted });
    }
    out
}

fn signature(srcs: &[Source]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |s: &str| {
        h ^= hash(s);
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    for s in srcs {
        match s {
            Source::Inline { text, .. } => {
                mix("i");
                mix(text);
            }
            Source::Adopted { text } => {
                mix("a");
                mix(text);
            }
            Source::External { url, media, .. } => {
                mix("e");
                mix(url);
                mix(media);
            }
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_added_and_changed_styles() {
        let mut dom = crate::html::parse("<html><head><style>p{color:red}</style></head><body><p>x</p></body></html>");
        let initial = vec![css::parse("p{color:red}")];
        let mut live = Live::new("file:///t.html", Default::default(), &dom, initial);
        assert!(!live.refresh(&dom), "nothing changed yet");
        // A script adds a <style>.
        let head = dom.find_all("head")[0];
        let st = dom.create(NodeData::Element { tag: "style".into(), attrs: vec![] });
        let t = dom.create(NodeData::Text("p{color:green}".into()));
        dom.insert(st, t, None);
        dom.insert(head, st, None);
        assert!(live.refresh(&dom));
        assert_eq!(live.sheets().len(), 2);
        // Rules added with insertRule replace that element's text while it stays the same.
        set_cssom(st, "p{color:green}", "p{color:green}\n.x{color:blue}");
        assert!(live.refresh(&dom));
        assert_eq!(live.sheets()[1].rules.len(), 2);
        assert!(!live.refresh(&dom));
    }
}
