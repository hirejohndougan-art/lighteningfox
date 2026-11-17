//! How tall the page scrolls: its content can overflow the root box. With the common
//! `html, body { height: 100% }` the root box is one viewport tall while the page's content
//! runs far below it; browsers still let the page scroll to the bottom of that content (the
//! New York Times rendered one screen tall, 2026-11). Boxes clipped by an `overflow` ancestor
//! and fixed boxes don't count. `styles` here are after viewport_overflow, so <html> and <body> don't clip.

use crate::dom::{Dom, NodeId};
use crate::layout::Rect;
use crate::style::Style;
use std::collections::HashMap;
use std::rc::Rc;

/// The lowest bottom edge of the boxes that make the page scroll, or 0.
pub fn bottom(dom: &Dom, styles: &[Rc<Style>], boxes: &HashMap<NodeId, (Rect, bool)>) -> f32 {
    let mut clipped: HashMap<NodeId, bool> = HashMap::new();
    let mut is_clipped = |id: NodeId| -> bool {
        let mut chain = Vec::new();
        let mut cur = dom.nodes[id].parent;
        let mut result = false;
        while let Some(p) = cur {
            if let Some(&c) = clipped.get(&p) {
                result = c;
                break;
            }
            chain.push(p);
            if styles.get(p).is_some_and(|s| s.clip) {
                result = true;
                break;
            }
            cur = dom.nodes[p].parent;
        }
        for p in chain {
            clipped.insert(p, result);
        }
        result
    };
    let mut max = 0.0f32;
    for (&id, &(r, fixed)) in boxes {
        let b = r.y + r.h;
        if fixed || b <= max || !b.is_finite() || r.w <= 0.0 || r.h <= 0.0 {
            continue;
        }
        if !is_clipped(id) {
            max = b;
        }
    }
    max
}

thread_local! {
    /// The last (original, adjusted) style pair for <html> and for <body>, so an unchanged
    /// style gives the same adjusted Rc each pass and relayout.rs can still reuse boxes.
    static ADJUSTED: std::cell::RefCell<[Option<(Rc<Style>, Rc<Style>)>; 2]> = const { std::cell::RefCell::new([None, None]) };
}

/// The styles to lay out with, after `overflow` moves to the window as in browsers: <html>'s
/// always, and <body>'s when <html>'s is `visible`. Neither then clips the page (with
/// `html, body { height: 100% }` and `body { overflow-x: hidden }`, the New York Times' page
/// was cut off below the first screen, 2026-11).
pub fn viewport_overflow<'a>(dom: &Dom, styles: &'a [Rc<Style>]) -> std::borrow::Cow<'a, [Rc<Style>]> {
    let html = dom.find_all("html").first().copied().filter(|&h| dom.nodes[h].parent == Some(Dom::ROOT));
    let Some(html) = html.filter(|&h| h < styles.len()) else { return std::borrow::Cow::Borrowed(styles) };
    let body = dom.nodes[html].children.iter().copied().find(|&c| dom.tag(c) == Some("body")).filter(|&b| b < styles.len());
    let mut fix = Vec::new();
    if styles[html].clip {
        fix.push((0, html));
    } else if let Some(b) = body.filter(|&b| styles[b].clip) {
        fix.push((1, b));
    }
    if fix.is_empty() {
        return std::borrow::Cow::Borrowed(styles);
    }
    let mut out = styles.to_vec();
    ADJUSTED.with(|a| {
        let mut a = a.borrow_mut();
        for (slot, id) in fix {
            let orig = &styles[id];
            let adjusted = match &a[slot] {
                Some((o, adj)) if Rc::ptr_eq(o, orig) => adj.clone(),
                _ => {
                    let mut s = (**orig).clone();
                    s.clip = false;
                    s.scrolls = false;
                    let adj = Rc::new(s);
                    a[slot] = Some((orig.clone(), adj.clone()));
                    adj
                }
            };
            out[id] = adjusted;
        }
    });
    std::borrow::Cow::Owned(out)
}
