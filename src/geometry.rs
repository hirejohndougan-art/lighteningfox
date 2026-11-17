//! Where things are on screen, for scripts: element boxes, the window size and the
//! scroll position. When a script changed the page since the last layout, asking for a
//! box lays the page out again first (as real browsers do).

use crate::dom::{Dom, NodeId};
use crate::layout::{Layout, Rect};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

#[derive(Default)]
struct Geometry {
    boxes: HashMap<NodeId, (Rect, bool)>,
    hits: Vec<(Rect, NodeId)>,
    scrollers: HashMap<NodeId, (Rect, f32)>,
    view_w: f32,
    view_h: f32,
    scroll: f32,
    doc_h: f32,
    /// DOM generation this layout was made from (`u64::MAX` = none yet).
    version: u64,
}

pub type Provider = Box<dyn Fn(f32, f32) -> Layout>;
/// Computes the page's styles on demand (getComputedStyle).
pub type Styler = Box<dyn Fn() -> Vec<std::rc::Rc<crate::style::Style>>>;

/// How to compute styles on demand (set per page, alongside the layout provider).
pub fn set_styler(s: Option<Styler>) {
    STYLER.with(|c| *c.borrow_mut() = s);
    STYLES.with(|c| *c.borrow_mut() = (u64::MAX, Vec::new()));
}

/// The computed style of `id` as JSON (see `style::computed_json`), restyling first if
/// the DOM changed since last time. None when no page styles are available.
pub fn computed_style(dom: &Dom, id: NodeId) -> Option<String> {
    let version = GEN.with(Cell::get);
    let stale = STYLES.with(|c| c.borrow().0 != version);
    if stale {
        let styles = STYLER.with(|s| s.borrow().as_ref().map(|f| f()))?;
        STYLES.with(|c| *c.borrow_mut() = (version, styles));
    }
    let st = STYLES.with(|c| c.borrow().1.get(id).cloned())?;
    let size = page_rect(dom, id).map(|(r, _)| (r.w, r.h));
    Some(crate::style::computed_json(&st, size))
}

thread_local! {
    static GEOM: RefCell<Geometry> = RefCell::new(Geometry { view_w: 1024.0, view_h: 768.0, version: u64::MAX, ..Default::default() });
    static GEN: Cell<u64> = const { Cell::new(0) };
    static PROVIDER: RefCell<Option<Provider>> = const { RefCell::new(None) };
    static STYLER: RefCell<Option<Styler>> = const { RefCell::new(None) };
    static STYLES: RefCell<(u64, Vec<std::rc::Rc<crate::style::Style>>)> = const { RefCell::new((u64::MAX, Vec::new())) };
    static SCROLL_REQUEST: Cell<Option<f32>> = const { Cell::new(None) };
}

/// The DOM changed: any stored boxes are out of date.
pub fn dom_changed() {
    GEN.with(|g| g.set(g.get().wrapping_add(1)));
}

/// How to lay the page out on demand (set per page; captures its stylesheets and images).
pub fn set_provider(p: Option<Provider>) {
    PROVIDER.with(|c| *c.borrow_mut() = p);
    GEOM.with(|g| {
        let mut g = g.borrow_mut();
        g.boxes.clear();
        g.hits.clear();
        g.version = u64::MAX;
        g.scroll = 0.0;
    });
}

/// Take the on-demand layout function out (the page's tab is going to the background).
pub fn take_provider() -> Option<Provider> {
    PROVIDER.with(|c| c.borrow_mut().take())
}

/// Record a finished layout (the window calls this whenever it lays out).
pub fn publish(lay: &Layout, view_w: f32, view_h: f32) {
    let version = GEN.with(Cell::get);
    GEOM.with(|g| {
        let mut g = g.borrow_mut();
        g.boxes = lay.boxes.clone();
        g.hits = lay.hits.clone();
        g.scrollers = lay.scrollers.iter().map(|&(id, r, max)| (id, (r, max))).collect();
        g.view_w = view_w;
        g.view_h = view_h;
        g.doc_h = lay.height;
        g.version = version;
    });
}

pub fn set_viewport(w: f32, h: f32) {
    GEOM.with(|g| {
        let mut g = g.borrow_mut();
        g.view_w = w;
        g.view_h = h;
    });
}

pub fn set_scroll(y: f32) {
    GEOM.with(|g| g.borrow_mut().scroll = y);
}

/// A scroll a script asked for (window.scrollTo, element.scrollIntoView), in page pixels.
pub fn take_scroll_request() -> Option<f32> {
    SCROLL_REQUEST.with(Cell::take)
}

pub fn request_scroll(y: f32) {
    let (_, view_h, _, doc_h) = viewport();
    let max = (doc_h - view_h).max(0.0);
    let y = y.clamp(0.0, max);
    SCROLL_REQUEST.with(|c| c.set(Some(y)));
    // Scripts see the new position right away.
    set_scroll(y);
}

/// Lay out again if the DOM changed since the stored layout.
fn ensure_fresh() {
    let version = GEN.with(Cell::get);
    let (stale, w, h) = GEOM.with(|g| {
        let g = g.borrow();
        (g.version != version, g.view_w, g.view_h)
    });
    if !stale {
        return;
    }
    let lay = PROVIDER.with(|p| p.borrow().as_ref().map(|f| f(w, h)));
    if let Some(lay) = lay {
        publish(&lay, w, h);
    }
}

/// (window width, window height, scroll y, document height)
pub fn viewport() -> (f32, f32, f32, f32) {
    ensure_fresh();
    GEOM.with(|g| {
        let g = g.borrow();
        (g.view_w, g.view_h, g.scroll, g.doc_h)
    })
}

/// An element's border box in page coordinates, and whether it's position: fixed.
/// Inline elements (spans, links) get the union of the text and boxes inside them.
pub fn page_rect(dom: &Dom, id: NodeId) -> Option<(Rect, bool)> {
    ensure_fresh();
    GEOM.with(|g| {
        let g = g.borrow();
        if let Some(&b) = g.boxes.get(&id) {
            return Some(b);
        }
        let inside = |mut n: NodeId| loop {
            if n == id {
                return true;
            }
            match dom.nodes.get(n).and_then(|x| x.parent) {
                Some(p) => n = p,
                None => return false,
            }
        };
        let mut acc: Option<Rect> = None;
        let mut add = |r: Rect| {
            acc = Some(match acc {
                None => r,
                Some(a) => {
                    let (x0, y0) = (a.x.min(r.x), a.y.min(r.y));
                    let (x1, y1) = ((a.x + a.w).max(r.x + r.w), (a.y + a.h).max(r.y + r.h));
                    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
                }
            })
        };
        for &(r, n) in &g.hits {
            if inside(n) {
                add(r);
            }
        }
        for (&n, &(r, _)) in &g.boxes {
            if n != id && inside(n) {
                add(r);
            }
        }
        acc.map(|r| (r, false))
    })
}

/// A scrollable box's (scrollTop, how far it can scroll, visible height), if `id` is one.
pub fn element_scroll_info(id: NodeId) -> Option<(f32, f32, f32)> {
    ensure_fresh();
    GEOM.with(|g| g.borrow().scrollers.get(&id).copied()).map(|(r, max)| (crate::layout::element_scroll(id).min(max), max, r.h))
}

/// getBoundingClientRect: the box relative to the window's top-left.
pub fn client_rect(dom: &Dom, id: NodeId) -> Rect {
    let scroll = viewport().2;
    match page_rect(dom, id) {
        Some((r, true)) => r,
        Some((r, false)) => Rect { y: r.y - scroll, ..r },
        None => Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
    }
}
