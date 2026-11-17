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
pub struct Entry {
    pub items: Vec<Item>,
    pub links: Vec<(Rect, String)>,
    pub hits: Vec<(Rect, NodeId)>,
    /// Floats it added to the formatting context around it: (x0, x1, y0, y1, left).
    pub floats: Vec<(f32, f32, f32, f32, bool)>,
    /// Height and bottom margin, as `block` returns them.
    pub h: f32,
    pub mb: f32,
    /// Pass it was made or last used in (old entries are dropped; see `Cache::pins`).
    pass: u64,
    size: usize,
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
        c.entries.retain(|_, e| e.pass + KEEP_PASSES > pass);
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
                if let Some(e) = Rc::get_mut(e) {
                    e.pass = pass;
                }
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
    pub fn new(items: Vec<Item>, links: Vec<(Rect, String)>, hits: Vec<(Rect, NodeId)>, floats: Vec<(f32, f32, f32, f32, bool)>, (h, mb): (f32, f32)) -> Entry {
        let size = items.len() + links.len() + hits.len() + floats.len() + 1;
        Entry { items, links, hits, floats, h, mb, pass: 0, size }
    }
}

pub fn store(key: u64, mut e: Entry) {
    if !enabled() {
        return;
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let key = full_key(&c, key);
        e.pass = c.pass;
        c.entries.insert(key, Rc::new(e));
    });
}

/// (boxes reused, boxes laid out) in the last pass.
#[cfg_attr(not(test), allow(dead_code))]
pub fn stats() -> (usize, usize) {
    CACHE.with(|c| {
        let c = c.borrow();
        (c.hits, c.misses)
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
