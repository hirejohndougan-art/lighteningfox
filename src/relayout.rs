//! Incremental relayout: only redo the boxes whose subtree changed.
//!
//! Every box in the box tree gets a fingerprint of its whole subtree: the node itself (tag
//! and attributes, or text; a file input's picked files), its computed style (by identity: restyling keeps the same
//! `Rc` for an element whose style didn't change), list marker text, scroll offset, and its
//! children's fingerprints. When layout lays a block box out, it remembers what came out
//! (display items, hits, links, its height and bottom margin), keyed by that fingerprint
//! and everything the parent handed in (available width, collapsing margins, imposed flex
//! sizes, the percentage base). Next time, a box with the same key is put back moved to
//! its new position instead of being laid out again — so a change deep in a page redoes
//! only the boxes on the path to it, and a pass where nothing changed redoes nothing.
//!
//! A box is only remembered when its layout stayed inside it: no floats or absolutely
//! positioned descendants escaping to an outer context, no fixed boxes or z-index layers.
//! The focused control and a contenteditable host are never remembered (their caret and
//! the words editing records change without the DOM changing).

use crate::boxtree::{BoxKind, BoxTree};
use crate::dom::{Dom, NodeData, NodeId};
use crate::paint::Item;
use crate::layout::Rect;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Fingerprint 0 means "never reuse" (the box or something in it changes without the DOM
/// or styles changing).
pub const VOLATILE: u64 = 0;

/// Subtree fingerprints for every box (VOLATILE for boxes not in the tree).
pub fn fingerprints(tree: &BoxTree, focus: Option<NodeId>, host: Option<NodeId>) -> Vec<u64> {
    let dom = &tree.dom;
    let n = dom.nodes.len();
    let mut fp = vec![VOLATILE; n];
    // Post-order without recursion (trees can be deep).
    let mut order = Vec::with_capacity(n);
    let mut stack = vec![Dom::ROOT];
    while let Some(id) = stack.pop() {
        order.push(id);
        stack.extend(dom.nodes[id].children.iter().copied());
    }
    for &id in order.iter().rev() {
        let kind = tree.kind(id);
        if (kind == BoxKind::Node && (Some(id) == focus || Some(id) == host)) || id >= tree.styles.len() {
            continue;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Generated boxes get new ids on every build: they're known by where they are.
        if kind == BoxKind::Node {
            id.hash(&mut h);
        }
        (kind as u8).hash(&mut h);
        match &dom.nodes[id].data {
            NodeData::Element { tag, attrs } => {
                tag.hash(&mut h);
                attrs.hash(&mut h);
                // What a file input shows lives in the form state, not the DOM.
                if let Some(files) = dom.form_state.files.get(&id) {
                    files.iter().for_each(|f| f.name.hash(&mut h));
                }
            }
            NodeData::Text(t) => t.hash(&mut h),
            _ => 0u8.hash(&mut h),
        }
        // Anonymous boxes and inside markers get a fresh style on every build, derived from
        // their parent's: the parent's style identity stands for theirs.
        let style_of = match kind {
            BoxKind::AnonymousBlock | BoxKind::Marker => dom.nodes[id].parent.unwrap_or(id),
            _ => id,
        };
        (Rc::as_ptr(&tree.styles[style_of]) as usize).hash(&mut h);
        tree.markers.get(&id).hash(&mut h);
        let scroll = crate::layout::element_scroll(tree.origin(id));
        scroll.to_bits().hash(&mut h);
        let mut volatile = false;
        for &c in &dom.nodes[id].children {
            let f = fp[c];
            volatile |= f == VOLATILE;
            f.hash(&mut h);
        }
        if !volatile {
            fp[id] = h.finish().max(1);
        }
    }
    fp
}

/// What laying out one block box produced, relative to where it was placed.
///
/// Display items are kept as pieces: runs of the box's own items, and child boxes' entries
/// shared by reference (moved to where the child sat) instead of copied again at every
/// level of nesting.
pub struct Entry {
    pub pieces: Vec<Piece<Item>>,
    pub links: Vec<Piece<(Rect, String)>>,
    pub hits: Vec<Piece<(Rect, NodeId)>>,
    /// Floats it added to the formatting context around it: (x0, x1, y0, y1, left).
    pub floats: Vec<(f32, f32, f32, f32, bool)>,
    /// Height and bottom margin, as `block` returns them.
    pub h: f32,
    pub mb: f32,
    /// Pass it was made or last used in (old entries are dropped; see `Cache::pins`).
    pass: std::cell::Cell<u64>,
    /// What it holds itself (shared children count in their own entries).
    size: usize,
}

pub enum Piece<T> {
    Own(Vec<T>),
    /// A child box's entry, placed at this offset from the box's origin.
    Shared(Rc<Entry>, f32, f32),
}

/// Something layout outputs that a cached box keeps: display items, links, hit areas.
pub trait Out: Clone {
    fn shift(&mut self, dx: f32, dy: f32);
    fn list(e: &Entry) -> &[Piece<Self>];
}

impl Out for Item {
    fn shift(&mut self, dx: f32, dy: f32) {
        crate::layout::shift_item(self, dx, dy);
    }
    fn list(e: &Entry) -> &[Piece<Item>] {
        &e.pieces
    }
}

impl Out for (Rect, String) {
    fn shift(&mut self, dx: f32, dy: f32) {
        self.0.x += dx;
        self.0.y += dy;
    }
    fn list(e: &Entry) -> &[Piece<Self>] {
        &e.links
    }
}

impl Out for (Rect, NodeId) {
    fn shift(&mut self, dx: f32, dy: f32) {
        self.0.x += dx;
        self.0.y += dy;
    }
    fn list(e: &Entry) -> &[Piece<Self>] {
        &e.hits
    }
}

impl Entry {
    /// Append the box's items (or links, or hits), moved by (dx, dy).
    pub fn emit<T: Out>(&self, dx: f32, dy: f32, out: &mut Vec<T>) {
        for p in T::list(self) {
            match p {
                Piece::Own(v) => out.extend(v.iter().map(|it| {
                    let mut it = it.clone();
                    it.shift(dx, dy);
                    it
                })),
                Piece::Shared(e, ex, ey) => e.emit(dx + ex, dy + ey, out),
            }
        }
    }
}

/// Fingerprint of a run of links or hits seen from (ox, oy): count and summed positions.
pub fn rect_sig<T>(list: &[(Rect, T)], ox: f32, oy: f32) -> Sig {
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    for (r, _) in list {
        sx += (r.x - ox) as f64 + r.w as f64;
        sy += (r.y - oy) as f64 + r.h as f64;
    }
    Sig { n: list.len(), kinds: 0, sx, sy, text: 0 }
}

/// A cheap fingerprint of a run of display items as seen from (ox, oy): how many, their
/// kinds in order, and the sums of their positions and text lengths. Layout moves, wraps,
/// splices and reorders items it already placed; any of that changes this, so a parent
/// box only shares a child's entry when the child's items are still exactly as placed.
#[derive(Clone, Copy, PartialEq)]
pub struct Sig {
    n: usize,
    kinds: u64,
    sx: f64,
    sy: f64,
    text: usize,
}

pub fn sig(items: &[Item], ox: f32, oy: f32) -> Sig {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let (mut sx, mut sy, mut text) = (0.0f64, 0.0f64, 0usize);
    for it in items {
        std::mem::discriminant(it).hash(&mut h);
        let p = match it {
            Item::Rect { x, y, .. }
            | Item::Text { x, y, .. }
            | Item::Image { x, y, .. }
            | Item::Mark { x, y, .. }
            | Item::ScrollArea { x, y, .. }
            | Item::Box { x, y, .. }
            | Item::Ring { x, y, .. }
            | Item::Shadow { x, y, .. }
            | Item::ClipPush { x, y, .. }
            | Item::TransformPush { x, y, .. } => Some((*x, *y)),
            Item::StickyPush { y, .. } => Some((ox, *y)),
            Item::EffectPush(fx) => Some((fx.x, fx.y)),
            _ => None,
        };
        if let Some((x, y)) = p {
            sx += (x - ox) as f64;
            sy += (y - oy) as f64;
        }
        if let Item::Text { text: t, .. } = it {
            text += t.len();
        }
    }
    Sig { n: items.len(), kinds: h.finish(), sx, sy, text }
}

impl Sig {
    /// Same items, allowing for float rounding in the sums.
    pub fn matches(&self, o: &Sig) -> bool {
        let tol = 0.01 * (self.n.max(1) as f64);
        self.n == o.n && self.kinds == o.kinds && self.text == o.text && (self.sx - o.sx).abs() <= tol && (self.sy - o.sy).abs() <= tol
    }
}

#[derive(Default)]
struct Cache {
    entries: HashMap<u64, Rc<Entry>>,
    /// Fingerprints hash style pointers: the styles of the last few passes are kept alive
    /// so no new style can reuse an address an entry was keyed on.
    pins: Vec<(u64, Vec<Rc<crate::style::Style>>)>,
    pass: u64,
    env: u64,
    /// This pass's counts, for tests and the profiler.
    hits: usize,
    misses: usize,
}

/// Passes an entry survives without being used.
const KEEP_PASSES: u64 = 4;

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Start a layout pass. `env` covers everything outside the box tree that layout reads
/// (viewport, loaded images); it's part of every key, so a page and its frames (laid out
/// at different sizes) each find their own boxes.
pub fn begin_pass(env: u64, styles: &[Rc<crate::style::Style>]) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if !enabled() {
            c.entries.clear();
        }
        c.env = env;
        c.pass += 1;
        let pass = c.pass;
        c.pins.push((pass, styles.to_vec()));
        c.pins.retain(|(p, _)| p + KEEP_PASSES > pass);
        c.entries.retain(|_, e| e.pass.get() + KEEP_PASSES > pass);
        c.hits = 0;
        c.misses = 0;
    });
}

/// After a pass: keep the cache to a budget (a few times the page's own size), dropping
/// the smallest boxes first — they're the cheapest to lay out again.
pub fn end_pass(page_items: usize) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let budget = page_items.max(256) * 6;
        let mut total: usize = c.entries.values().map(|e| e.size).sum();
        if total <= budget {
            return;
        }
        let mut by_size: Vec<(usize, u64)> = c.entries.iter().map(|(k, e)| (e.size, *k)).collect();
        by_size.sort_unstable();
        for (size, k) in by_size {
            if total <= budget {
                break;
            }
            c.entries.remove(&k);
            total -= size;
        }
    });
}

/// A key with the pass's environment mixed in.
fn full_key(c: &Cache, key: u64) -> u64 {
    key ^ c.env.rotate_left(17).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

pub fn lookup(key: u64) -> Option<Rc<Entry>> {
    if !enabled() {
        return None;
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let pass = c.pass;
        let key = full_key(&c, key);
        match c.entries.get_mut(&key) {
            Some(e) => {
                // Used again: its styles are this pass's, which are pinned now.
                e.pass.set(pass);
                let e = e.clone();
                c.hits += 1;
                Some(e)
            }
            None => {
                c.misses += 1;
                None
            }
        }
    })
}

impl Entry {
    pub fn new(pieces: Vec<Piece<Item>>, links: Vec<Piece<(Rect, String)>>, hits: Vec<Piece<(Rect, NodeId)>>, floats: Vec<(f32, f32, f32, f32, bool)>, (h, mb): (f32, f32)) -> Entry {
        fn own<T>(l: &[Piece<T>]) -> usize {
            l.iter().map(|p| if let Piece::Own(v) = p { v.len() } else { 1 }).sum()
        }
        let size = own(&pieces) + own(&links) + own(&hits) + floats.len() + 1;
        Entry { pieces, links, hits, floats, h, mb, pass: std::cell::Cell::new(0), size }
    }
}

/// Remember a box's layout; the entry is returned so a parent can share it.
pub fn store(key: u64, e: Entry) -> Option<Rc<Entry>> {
    if !enabled() {
        return None;
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let key = full_key(&c, key);
        e.pass.set(c.pass);
        let e = Rc::new(e);
        c.entries.insert(key, e.clone());
        Some(e)
    })
}

/// (boxes reused, boxes laid out) in the last pass.
#[cfg_attr(not(test), allow(dead_code))]
pub fn stats() -> (usize, usize) {
    CACHE.with(|c| {
        let c = c.borrow();
        (c.hits, c.misses)
    })
}

/// (display items the cache holds itself, child entries it shares) — tests.
#[cfg(test)]
pub fn held() -> (usize, usize) {
    CACHE.with(|c| {
        let c = c.borrow();
        let own = c.entries.values().map(|e| e.pieces.iter().map(|p| if let Piece::Own(v) = p { v.len() } else { 0 }).sum::<usize>()).sum();
        let shared = c.entries.values().map(|e| e.pieces.iter().filter(|p| matches!(p, Piece::Shared(..))).count()).sum();
        (own, shared)
    })
}

/// Forget everything (tests; IRONSWIFT_NO_RELAYOUT_CACHE turns reuse off).
#[cfg_attr(not(test), allow(dead_code))]
pub fn clear() {
    CACHE.with(|c| *c.borrow_mut() = Cache::default());
}

fn enabled() -> bool {
    thread_local! {
        static ON: bool = std::env::var_os("IRONSWIFT_NO_RELAYOUT_CACHE").is_none();
    }
    ON.with(|o| *o)
}

/// A hasher for building keys.
pub fn hasher() -> std::collections::hash_map::DefaultHasher {
    std::collections::hash_map::DefaultHasher::new()
}

#[cfg(test)]
mod tests {
    use crate::dom::Dom;
    use crate::paint::Item;

    fn render(dom: &Dom, sheets: &[crate::css::Stylesheet], fonts: &crate::font::Fonts) -> crate::layout::Layout {
        let styles = crate::style::compute(dom, sheets);
        crate::layout::layout(dom, &styles, fonts, &crate::images::Images::default(), 800.0, 600.0)
    }

    fn same(a: &crate::layout::Layout, b: &crate::layout::Layout) {
        // Slots layout reserved and left empty are invisible wherever they sit.
        let show = |it: Option<&Item>| match it {
            Some(Item::Rect { w, h, color, .. }) if *w == 0.0 && *h == 0.0 && color.a == 0 => "empty".to_string(),
            it => rounded(&format!("{it:?}")),
        };
        if let Some(i) = (0..a.items.len().max(b.items.len())).find(|&i| show(a.items.get(i)) != show(b.items.get(i))) {
            panic!("item {i} of {}/{} differs:\n reused {:?}\n fresh  {:?}", a.items.len(), b.items.len(), a.items.get(i), b.items.get(i));
        }
        assert_eq!(rounded(&format!("{:?}", a.hits)), rounded(&format!("{:?}", b.hits)));
        assert!((a.height - b.height).abs() < 0.01);
        let q = |v: f32| (v * 100.0).round() as i64;
        let mut ka: Vec<_> = a.boxes.iter().map(|(k, (r, f))| (*k, q(r.x), q(r.y), q(r.w), q(r.h), *f)).collect();
        let mut kb: Vec<_> = b.boxes.iter().map(|(k, (r, f))| (*k, q(r.x), q(r.y), q(r.w), q(r.h), *f)).collect();
        ka.sort();
        kb.sort();
        assert_eq!(ka, kb);
    }

    /// Numbers in a debug string to 0.01 (a moved box's lines can land 0.00003 px off).
    fn rounded(s: &str) -> String {
        let mut out = String::new();
        let mut num = String::new();
        let flush = |num: &mut String, out: &mut String| {
            if num.contains('.') {
                if let Ok(v) = num.parse::<f64>() {
                    out.push_str(&format!("{:.2}", (v * 100.0).round() / 100.0 + 0.0));
                    num.clear();
                    return;
                }
            }
            out.push_str(num);
            num.clear();
        };
        for c in s.chars() {
            if c.is_ascii_digit() || c == '.' || (c == '-' && num.is_empty()) {
                num.push(c);
            } else {
                flush(&mut num, &mut out);
                out.push(c);
            }
        }
        flush(&mut num, &mut out);
        out
    }

    const PAGE: &str = "<style>.f{float:left;width:50px;height:20px;background:red} .a{position:absolute;top:0} section{margin:10px 0;border:1px solid}</style>\
        <body><section id=one><p>First <b>para</b> text</p><div class=f></div><p>beside float</p></section>\
        <section id=two><h2>Two</h2><ul><li>a<li>b</ul><p>mixed <i>inline</i></p>tail text<div>block</div></section>\
        <section id=three style=position:relative><span class=a>abs</span><p id=target>change me</p></section></body>";

    #[test]
    fn unchanged_page_is_reused_and_identical() {
        let fonts = crate::load_fonts().expect("fonts");
        super::clear();
        let dom = crate::html::parse(PAGE);
        let sheets = vec![crate::css::parse(&dom.text_content(dom.find_all("style")[0]))];
        let first = render(&dom, &sheets, fonts);
        let (h0, _) = super::stats();
        assert_eq!(h0, 0);
        let second = render(&dom, &sheets, fonts);
        let (h1, m1) = super::stats();
        assert!(h1 >= 1 && m1 <= 2, "hits {h1} misses {m1}");
        same(&first, &second);
    }

    #[test]
    fn a_change_redoes_only_its_path_and_matches_a_fresh_layout() {
        let fonts = crate::load_fonts().expect("fonts");
        super::clear();
        let mut dom = crate::html::parse(PAGE);
        let sheets = vec![crate::css::parse(&dom.text_content(dom.find_all("style")[0]))];
        render(&dom, &sheets, fonts);
        // Change the text in #target: longer, so it wraps differently.
        let p = dom.find_all("p").into_iter().find(|&p| dom.attr(p, "id") == Some("target")).unwrap();
        let t = dom.nodes[p].children[0];
        dom.nodes[t].data = crate::dom::NodeData::Text("changed to something quite a lot longer ".repeat(8));
        let incremental = render(&dom, &sheets, fonts);
        let (hits, _) = super::stats();
        assert!(hits >= 2, "sections one and two should be reused, got {hits}");
        super::clear();
        let fresh = render(&dom, &sheets, fonts);
        same(&incremental, &fresh);
    }

    /// Picking a file changes what an `<input type=file>` shows without changing the DOM
    /// (the files are kept in the form state): its box must be laid out again.
    #[test]
    fn a_picked_file_shows() {
        let fonts = crate::load_fonts().expect("fonts");
        super::clear();
        let mut dom = crate::html::parse("<section><p>Upload: <input type=file id=f></p></section><section><p>other</p></section>");
        render(&dom, &[], fonts);
        render(&dom, &[], fonts);
        let input = dom.find_all("input")[0];
        dom.form_state.files.insert(input, vec![crate::validation::ChosenFile { path: "/tmp/holiday-photo.jpg".into(), name: "holiday-photo.jpg".into(), size: 1, mime: "image/jpeg".into(), modified: Default::default() }]);
        let incremental = render(&dom, &[], fonts);
        assert!(format!("{:?}", incremental.items).contains("holiday-photo.jpg"), "the picked file's name isn't shown");
        super::clear();
        let fresh = render(&dom, &[], fonts);
        same(&incremental, &fresh);
    }

    /// Nested boxes share their children's cached items instead of copying them at every
    /// level, so the cache holds about one copy of the page's items, not one per level.
    #[test]
    fn nested_boxes_share_cached_items() {
        let fonts = crate::load_fonts().expect("fonts");
        super::clear();
        let mut html = String::new();
        for i in 0..40 {
            html.push_str(&format!("<section><div><article><p>Item {i} with <b>some</b> words</p><p>more</p></article></div></section>"));
        }
        let dom = crate::html::parse(&html);
        let lay = render(&dom, &[], fonts);
        let (own, shared) = super::held();
        assert!(shared > 100, "children are shared: {shared}");
        assert!(own <= lay.items.len() * 3 / 2, "{own} items held for a page of {}", lay.items.len());
        // And the page put back from the cache is the same page.
        let again = render(&dom, &[], fonts);
        same(&again, &lay);
    }

    /// A canvas, video or live frame redrawn at the same size changes no box: layout reads an
    /// image's size, and paint fetches its pixels by key, so the boxes stay reusable.
    #[test]
    fn redrawn_pictures_keep_boxes_reusable() {
        let fonts = crate::load_fonts().expect("fonts");
        super::clear();
        let dom = crate::html::parse("<section><p>Title</p><canvas width=40 height=30></canvas></section><section><p>other text</p></section>");
        let styles = crate::style::compute(&dom, &[]);
        let canvas = dom.find_all("canvas")[0];
        let mut images = crate::images::Images::default();
        images.insert(&format!("canvas:{canvas}"), image::RgbaImage::new(40, 30));
        crate::layout::layout(&dom, &styles, fonts, &images, 800.0, 600.0);
        images.insert(&format!("canvas:{canvas}"), image::RgbaImage::from_pixel(40, 30, image::Rgba([255, 0, 0, 255])));
        crate::layout::layout(&dom, &styles, fonts, &images, 800.0, 600.0);
        let (hits, _) = super::stats();
        assert!(hits >= 1, "a redrawn canvas threw every box away");
        // A picture that changes size does change layout.
        images.insert(&format!("canvas:{canvas}"), image::RgbaImage::new(80, 30));
        crate::layout::layout(&dom, &styles, fonts, &images, 800.0, 600.0);
        assert_eq!(super::stats().0, 0, "a resized picture must not reuse boxes laid out at the old size");
    }

    /// A page's web fonts arriving changes every word's width without changing the DOM or
    /// the styles: nothing laid out with the fallback font may be reused.
    #[test]
    fn web_fonts_arriving_lay_text_out_again() {
        let fonts = crate::load_fonts().expect("fonts");
        super::clear();
        let dom = crate::html::parse("<style>p{font-family:Webby,sans-serif;width:200px}</style><section><p>The quick brown fox jumps over the lazy dog again and again</p></section>");
        let sheets = vec![crate::css::parse(&dom.text_content(dom.find_all("style")[0]))];
        let styles = crate::style::compute(&dom, &sheets);
        let lay = |dom: &Dom| crate::layout::layout(dom, &styles, fonts, &crate::images::Images::default(), 800.0, 600.0);
        crate::font::take_web_families();
        lay(&dom);
        let mut fams = std::collections::HashMap::new();
        for name in ["Webby", "webby"] {
            fams.insert(name.to_string(), vec![(100, 900, false, crate::font::MONO)]);
        }
        crate::font::set_web_families(fams);
        let incremental = lay(&dom);
        super::clear();
        let fresh = lay(&dom);
        crate::font::take_web_families();
        same(&incremental, &fresh);
    }

    /// Every test page, through a run of structural edits (remove, insert, a class change,
    /// display: contents, display: none, a list item added, a paragraph emptied): after each,
    /// the reused layout matches a fresh one.
    #[test]
    fn structural_edits_match_fresh_layouts() {
        let fonts = crate::load_fonts().expect("fonts");
        crate::images::freeze_animations();
        let mut pages: Vec<_> = ["tests", "tests/real-world"]
            .iter()
            .flat_map(|d| std::fs::read_dir(d).unwrap().flatten().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "html"))
            .collect();
        pages.sort();
        type Edit = fn(&mut Dom) -> bool;
        // The element in the middle of the body (not a script or style), if any.
        fn middle(d: &Dom, nth: usize) -> Option<crate::dom::NodeId> {
            let body = d.find_all("body").first().copied()?;
            let els: Vec<_> = d
                .descendants(body)
                .into_iter()
                .filter(|&n| n != body && d.tag(n).is_some_and(|t| !matches!(t, "script" | "style" | "template" | "link" | "meta")))
                .collect();
            els.get((els.len() / 2 + nth).min(els.len().saturating_sub(1))).copied()
        }
        let edits: [(&str, Edit); 7] = [
            ("remove", |d| middle(d, 0).map(|n| d.detach(n)).is_some()),
            ("insert", |d| {
                let Some(n) = middle(d, 1) else { return false };
                let Some(parent) = d.nodes[n].parent else { return false };
                let div = d.create(crate::dom::NodeData::Element { tag: "div".into(), attrs: vec![] });
                let t = d.create(crate::dom::NodeData::Text("a new block with a few words".into()));
                d.insert(div, t, None);
                d.insert(parent, div, Some(n));
                true
            }),
            ("class", |d| middle(d, 2).map(|n| d.set_attr(n, "class", "edited-class")).is_some()),
            ("contents", |d| middle(d, 3).map(|n| d.set_attr(n, "style", "display:contents")).is_some()),
            ("none", |d| middle(d, 4).map(|n| d.set_attr(n, "style", "display:none")).is_some()),
            ("list item", |d| {
                let Some(&list) = d.find_all("ul").first().or(d.find_all("ol").first()) else { return false };
                let li = d.create(crate::dom::NodeData::Element { tag: "li".into(), attrs: vec![] });
                let t = d.create(crate::dom::NodeData::Text("added item".into()));
                d.insert(li, t, None);
                d.insert(list, li, None);
                true
            }),
            ("emptied", |d| {
                let Some(&p) = d.find_all("p").get(1) else { return false };
                d.remove_children(p);
                true
            }),
        ];
        for path in pages {
            let url = crate::net::normalize_input(path.to_str().unwrap());
            let mut page = crate::load(&url);
            if let Some(js) = page.js.as_mut() {
                js.settle(std::time::Duration::from_millis(50));
            }
            page.restyle();
            super::clear();
            page.layout(fonts, 900.0, 700.0);
            for (name, edit) in edits {
                if !crate::js::with_dom_mut(edit) {
                    continue;
                }
                page.restyle();
                let incremental = page.layout(fonts, 900.0, 700.0);
                super::clear();
                let fresh = page.layout(fonts, 900.0, 700.0);
                eprintln!("{} after {name}", path.display());
                same(&incremental, &fresh);
            }
        }
    }

    /// Every reference page: a second pass reuses boxes and matches a fresh layout, and so
    /// does a pass after one text node in the middle of the page changes.
    #[test]
    fn reference_pages_match_fresh_layouts() {
        let fonts = crate::load_fonts().expect("fonts");
        crate::images::freeze_animations();
        let mut pages: Vec<_> = ["tests", "tests/real-world"]
            .iter()
            .flat_map(|d| std::fs::read_dir(d).unwrap().flatten().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "html"))
            .collect();
        pages.sort();
        let mut reused = 0;
        for path in pages {
            let url = crate::net::normalize_input(path.to_str().unwrap());
            let mut page = crate::load(&url);
            if let Some(js) = page.js.as_mut() {
                js.settle(std::time::Duration::from_millis(50));
            }
            page.restyle();
            super::clear();
            let fresh = page.layout(fonts, 900.0, 700.0);
            let again = page.layout(fonts, 900.0, 700.0);
            reused += super::stats().0;
            same(&fresh, &again);
            // Change the middle text node (to longer text) and compare with a fresh layout.
            let changed = crate::js::with_dom_mut(|d| {
                let texts: Vec<_> = d.descendants(Dom::ROOT).into_iter().filter(|&n| matches!(&d.nodes[n].data, crate::dom::NodeData::Text(t) if t.trim().len() > 3) && d.nodes[n].parent.and_then(|p| d.tag(p)).is_some_and(|t| !matches!(t, "script" | "style" | "title"))).collect();
                let Some(&t) = texts.get(texts.len() / 2) else { return false };
                if let crate::dom::NodeData::Text(s) = &mut d.nodes[t].data {
                    s.push_str(" and some more words that make the line longer");
                }
                true
            });
            if !changed {
                continue;
            }
            page.restyle();
            let incremental = page.layout(fonts, 900.0, 700.0);
            super::clear();
            let fresh = page.layout(fonts, 900.0, 700.0);
            eprintln!("{}", path.display());
            same(&incremental, &fresh);
        }
        assert!(reused > 20, "only {reused} boxes reused");
    }
}

#[cfg(test)]
mod bench {
    /// `cargo test --release relayout_timing -- --ignored --nocapture`: full layout vs.
    /// relayout with nothing changed vs. after one text edit, on the bigger test pages.
    #[test]
    #[ignore]
    fn relayout_timing() {
        let fonts = crate::load_fonts().expect("fonts");
        for page in ["tests/real-world/bootstrap.html", "tests/real-world/react.html", "tests/tables.html", "tests/grid.html"] {
            let mut p = crate::load(&crate::net::normalize_input(page));
            p.restyle();
            let time = |p: &mut crate::Page, clear: bool| {
                let t = std::time::Instant::now();
                for _ in 0..10 {
                    if clear {
                        super::clear();
                    }
                    p.layout(fonts, 1000.0, 700.0);
                }
                t.elapsed().as_secs_f64() * 100.0
            };
            let full = time(&mut p, true);
            let same = time(&mut p, false);
            eprintln!("{page}: full {full:.2} ms, unchanged {same:.2} ms");
        }
    }
}

