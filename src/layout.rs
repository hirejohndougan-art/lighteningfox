//! Layout: positions every box and line of text, producing a display list.
//! Supports block flow (margins, padding, borders, width/max-width, auto-centering,
//! sibling margin collapsing) and inline flow (word wrapping, alignment, links, <br>, <pre>).

use crate::dom::{Dom, NodeData, NodeId};
use crate::font::Fonts;
use crate::grid;
use crate::style::Background;
use crate::forms::{self, Control};
use std::cell::Cell;

thread_local! {
    /// The focused form control and whether its caret is currently shown.
    static FOCUS: Cell<Option<(NodeId, bool)>> = const { Cell::new(None) };
    static EDIT: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
    /// How far each scrollable box (overflow: auto/scroll) is scrolled.
    static SCROLLS: std::cell::RefCell<HashMap<NodeId, f32>> = std::cell::RefCell::new(HashMap::new());
}

pub fn element_scroll(id: NodeId) -> f32 {
    SCROLLS.with(|s| s.borrow().get(&id).copied().unwrap_or(0.0))
}

pub fn set_element_scroll(id: NodeId, y: f32) {
    SCROLLS.with(|s| {
        s.borrow_mut().insert(id, y.max(0.0));
    });
}

/// A number that changes whenever any box's scroll position does (for script observers).
pub fn element_scroll_stamp() -> f64 {
    SCROLLS.with(|s| s.borrow().iter().map(|(&k, &v)| (k as f64 + 1.0) * v as f64).sum())
}

/// Take a page's box scroll positions out (its tab is going to the background) / put them back.
pub fn take_element_scrolls() -> HashMap<NodeId, f32> {
    SCROLLS.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

pub fn set_element_scrolls(m: HashMap<NodeId, f32>) {
    SCROLLS.with(|s| *s.borrow_mut() = m);
}

/// A new page: every box starts unscrolled.
pub fn reset_element_scrolls() {
    SCROLLS.with(|s| s.borrow_mut().clear());
}

pub fn set_focus(f: Option<(NodeId, bool)>) {
    FOCUS.with(|c| c.set(f));
}

/// The focused field's caret and selection anchor (character indices).
pub fn set_edit(caret: usize, anchor: usize) {
    EDIT.with(|e| e.set(Some((caret, anchor))));
}

pub fn clear_edit() {
    EDIT.with(|e| e.set(None));
}

pub fn focus() -> Option<NodeId> {
    FOCUS.with(|c| c.get()).map(|f| f.0)
}

use crate::images::Images;
use crate::paint::Item;
use crate::style::{Align, AlignItems, Color, Display, Justify, Len, ListStyle, Position, Style, TableRole, VAlign};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Layout {
    /// The layout drawn at `z` times its size (page zoom). Scripts keep the unscaled layout;
    /// this copy is what's drawn and what clicks are matched against.
    pub fn scaled(&self, z: f32) -> Layout {
        if (z - 1.0).abs() < 1e-4 {
            return self.clone();
        }
        let r = |r: &Rect| Rect { x: r.x * z, y: r.y * z, w: r.w * z, h: r.h * z };
        let len = |l: Len| match l {
            Len::Px(v) => Len::Px(v * z),
            Len::Mix(px, pct) => Len::Mix(px * z, pct),
            other => other,
        };
        let items = self
            .items
            .iter()
            .map(|it| match it {
                Item::Rect { x, y, w, h, color } => Item::Rect { x: x * z, y: y * z, w: w * z, h: h * z, color: *color },
                Item::Text { x, y, text, font, size, color } => Item::Text { x: x * z, y: y * z, text: text.clone(), font: *font, size: size * z, color: *color },
                Item::Image { x, y, w, h, src, view } => Item::Image { x: x * z, y: y * z, w: w * z, h: h * z, src: src.clone(), view: view.as_ref().map(|v| Box::new(v.map(|c| c * z))) },
                Item::Box { x, y, w, h, radius, bg } => {
                    let mut bg = bg.clone();
                    bg.pos = (len(bg.pos.0), len(bg.pos.1));
                    if let crate::style::BgSize::Size(a, b) = bg.size {
                        bg.size = crate::style::BgSize::Size(len(a), len(b));
                    }
                    for p in bg.positions.iter_mut() {
                        *p = (len(p.0), len(p.1));
                    }
                    for sz in bg.sizes.iter_mut() {
                        if let crate::style::BgSize::Size(a, b) = *sz {
                            *sz = crate::style::BgSize::Size(len(a), len(b));
                        }
                    }
                    Item::Box { x: x * z, y: y * z, w: w * z, h: h * z, radius: radius * z, bg }
                }
                Item::Ring { x, y, w, h, radius, width, color } => Item::Ring { x: x * z, y: y * z, w: w * z, h: h * z, radius: radius * z, width: width * z, color: *color },
                Item::Shadow { x, y, w, h, radius, blur, color } => Item::Shadow { x: x * z, y: y * z, w: w * z, h: h * z, radius: radius * z, blur: blur * z, color: *color },
                Item::ClipPush { x, y, w, h, radius } => Item::ClipPush { x: x * z, y: y * z, w: w * z, h: h * z, radius: radius * z },
                Item::Mark { id, x, y, w, h } => Item::Mark { id: *id, x: x * z, y: y * z, w: w * z, h: h * z },
                Item::StickyPush { id, y, h, top, limit } => Item::StickyPush { id: *id, y: y * z, h: h * z, top: top * z, limit: limit * z },
                Item::ScrollArea { id, x, y, w, h, max } => Item::ScrollArea { id: *id, x: x * z, y: y * z, w: w * z, h: h * z, max: max * z },
                Item::TransformPush { m, x, y, w, h } => Item::TransformPush { m: [m[0], m[1], m[2], m[3], m[4] * z, m[5] * z], x: x * z, y: y * z, w: w * z, h: h * z },
                Item::EffectPush(fx) => Item::EffectPush(Box::new(fx.scaled(z))),
                other => other.clone(),
            })
            .collect();
        Layout {
            items,
            links: self.links.iter().map(|(rc, s)| (r(rc), s.clone())).collect(),
            hits: self.hits.iter().map(|(rc, n)| (r(rc), *n)).collect(),
            fixed: self.fixed.clone(),
            height: self.height * z,
            page_bg: self.page_bg,
            boxes: self.boxes.iter().map(|(k, (rc, f))| (*k, (r(rc), *f))).collect(),
            sticky: self.sticky.iter().map(|(rc, a, b)| (r(rc), a * z, b * z)).collect(),
            scrollers: self.scrollers.iter().map(|(n, rc, m)| (*n, r(rc), m * z)).collect(),
        }
    }

    /// Is link/hit `i` part of a fixed box (which doesn't scroll)?
    pub fn is_fixed(&self, i: usize, links: bool) -> bool {
        self.fixed.iter().any(|(l, h)| if links { l.contains(&i) } else { h.contains(&i) })
    }

    /// The innermost scrollable box under a viewport point that can still scroll by `dy`.
    pub fn scroller_at(&self, x: f32, view_y: f32, scroll: f32, dy: f32) -> Option<(NodeId, f32)> {
        let y = view_y + scroll;
        self.scrollers.iter().rev().filter(|(_, r, _)| r.contains(x, y)).find_map(|&(id, _, max)| {
            let cur = element_scroll(id).min(max);
            let can = if dy > 0.0 { cur < max - 0.5 } else { cur > 0.5 };
            can.then_some((id, max))
        })
    }

    /// The sticky box (where it sits unscrolled) drawn at page point (x, y), and its current offset.
    fn sticky_at(&self, x: f32, y: f32, scroll: f32) -> Option<(Rect, f32)> {
        self.sticky.iter().rev().find_map(|&(r, top, limit)| {
            let off = crate::paint::sticky_offset(r.y, top, limit, scroll);
            Rect { y: r.y + off, ..r }.contains(x, y).then_some((r, off))
        })
    }

    /// The link under a point in the viewport (y measured from the top of the page area).
    /// Fixed boxes are checked first: they sit on top.
    pub fn link_at(&self, x: f32, view_y: f32, scroll: f32) -> Option<&String> {
        if let Some((r, off)) = self.sticky_at(x, view_y + scroll, scroll) {
            let y = view_y + scroll - off;
            if let Some((_, l)) = self.links.iter().rev().find(|(lr, _)| lr.contains(x, y) && r.contains(x, y)) {
                return Some(l);
            }
        }
        let pick = |fixed: bool| {
            self.links.iter().enumerate().rev().find(|(i, (r, _))| {
                self.is_fixed(*i, true) == fixed && r.contains(x, if fixed { view_y } else { view_y + scroll })
            })
        };
        pick(true).or_else(|| pick(false)).map(|(_, (_, l))| l)
    }

    /// The DOM node drawn under a point in the viewport.
    pub fn hit_at(&self, x: f32, view_y: f32, scroll: f32) -> Option<NodeId> {
        if let Some((r, off)) = self.sticky_at(x, view_y + scroll, scroll) {
            let y = view_y + scroll - off;
            if let Some((_, n)) = self.hits.iter().rev().find(|(hr, _)| hr.contains(x, y) && r.contains(x, y)) {
                return Some(*n);
            }
        }
        let pick = |fixed: bool| {
            self.hits.iter().enumerate().rev().find(|(i, (r, _))| {
                self.is_fixed(*i, false) == fixed && r.contains(x, if fixed { view_y } else { view_y + scroll })
            })
        };
        pick(true).or_else(|| pick(false)).map(|(_, (_, n))| *n)
    }
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Default, Clone)]
pub struct Layout {
    pub items: Vec<Item>,
    pub links: Vec<(Rect, String)>,
    /// Clickable areas mapped to the DOM node drawn there (for JS click events).
    pub hits: Vec<(Rect, NodeId)>,
    /// Index ranges of `links` and `hits` that belong to position: fixed boxes.
    pub fixed: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
    pub height: f32,
    pub page_bg: Color,
    /// Each block-level element's border box in page coordinates; `true` = position: fixed
    /// (then the box is relative to the window, not the page).
    pub boxes: HashMap<NodeId, (Rect, bool)>,
    /// position: sticky boxes: (border box where it sits unscrolled, top inset, how far it may move).
    pub sticky: Vec<(Rect, f32, f32)>,
    /// Scrollable boxes: (element, visible area in page coordinates, how far it can scroll).
    pub scrollers: Vec<(NodeId, Rect, f32)>,
}

/// The key an iframe's rendered document is stored under in `Images`.
pub fn frame_key(dom: &Dom, id: NodeId) -> String {
    // The same page framed with different sandbox flags can look different.
    let sandbox = dom.attr(id, "sandbox").map(|s| format!(" sandbox={}", s.trim())).unwrap_or_default();
    let key = match dom.attr(id, "srcdoc") {
        Some(doc) => {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in doc.bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100000001b3);
            }
            format!("iframe:srcdoc:{h:x}")
        }
        None => format!("iframe:{}", dom.attr(id, "src").unwrap_or("").trim()),
    };
    key + &sandbox
}

pub fn layout(dom: &Dom, styles: &[Rc<Style>], fonts: &Fonts, images: &Images, width: f32, viewport_h: f32) -> Layout {
    // ::before and ::after boxes are laid out as extra nodes; hits on them count as their element's.
    let Some((full, full_styles)) = crate::pseudo::expand(dom, styles) else {
        return layout_tree(dom, styles, fonts, images, width, viewport_h);
    };
    let mut lay = layout_tree(&full, &full_styles, fonts, images, width, viewport_h);
    let n = dom.nodes.len();
    let real = |mut id: NodeId| {
        while id >= n {
            id = full.nodes[id].parent.unwrap_or(0);
        }
        id
    };
    for h in &mut lay.hits {
        h.1 = real(h.1);
    }
    for s in &mut lay.scrollers {
        s.0 = real(s.0);
    }
    lay.boxes.retain(|&id, _| id < n);
    lay
}

fn layout_tree(dom: &Dom, styles: &[Rc<Style>], fonts: &Fonts, images: &Images, width: f32, viewport_h: f32) -> Layout {
    let mut cx = Ctx {
        dom,
        styles,
        fonts,
        images,
        items: Vec::new(),
        links: Vec::new(),
        hits: Vec::new(),
        measuring: false,
        forced: HashMap::new(),
        abs: vec![Vec::new()],
        fixed: Vec::new(),
        floats: vec![Vec::new()],
        pad_top: HashMap::new(),
        layers: Vec::new(),
        flow_child: false,
        absorb_top: None,
        cb_heights: vec![Some(viewport_h)],
    };
    crate::editing::begin_layout();
    let (height, _) = cx.block_children(Dom::ROOT, 0.0, 0.0, width, Align::Left);
    // The page grows to contain floats that hang below the last block.
    let height = height.max(cx.float_bottom(true, true));
    // Absolute and fixed boxes with no positioned ancestor sit against the page.
    let root_abs = cx.abs.pop().unwrap_or_default();
    cx.place_absolutes(root_abs, Rect { x: 0.0, y: 0.0, w: width, h: viewport_h });
    // Canvas background comes from <html>, else <body>, like real browsers.
    let mut page_bg = Color::rgb(255, 255, 255);
    for tag in ["body", "html"] {
        if let Some(&id) = dom.find_all(tag).first() {
            if styles[id].background.a > 0 {
                page_bg = styles[id].background;
            }
        }
    }
    let mut items = merge_layers(std::mem::take(&mut cx.items), std::mem::take(&mut cx.layers));
    // Pull out the box marks: scripts need them, painting doesn't.
    let mut boxes = HashMap::new();
    let mut scrollers = Vec::new();
    let mut fixed_depth = 0;
    items.retain(|it| match *it {
        Item::ScrollArea { id, x, y, w, h, max } => {
            scrollers.push((id, Rect { x, y, w, h }, max));
            false
        }
        Item::Mark { id, x, y, w, h } => {
            boxes.insert(id, (Rect { x, y, w, h }, fixed_depth > 0));
            false
        }
        Item::FixedPush => {
            fixed_depth += 1;
            true
        }
        Item::FixedPop => {
            fixed_depth -= 1;
            true
        }
        _ => true,
    });
    // A sticky box can travel until it reaches the bottom of its parent's content box.
    let mut sticky = Vec::new();
    for it in items.iter_mut() {
        if let Item::StickyPush { id, y, h, top, limit } = it {
            let parent = dom.nodes[*id].parent;
            let bottom = parent
                .and_then(|p| boxes.get(&p).map(|(r, _)| r.y + r.h - styles[p].padding[2].resolve(width) - styles[p].border[2]))
                .unwrap_or(height);
            *limit = (bottom - styles[*id].margin[2].resolve(width) - (*y + *h)).max(0.0);
            if let Some((r, _)) = boxes.get(id) {
                sticky.push((Rect { y: *y, ..*r }, *top, *limit));
            }
        }
    }
    Layout { items, links: cx.links, hits: cx.hits, fixed: cx.fixed, height, page_bg, boxes, sticky, scrollers }
}

struct Ctx<'a> {
    dom: &'a Dom,
    styles: &'a [Rc<Style>],
    fonts: &'a Fonts,
    images: &'a Images,
    items: Vec<Item>,
    links: Vec<(Rect, String)>,
    hits: Vec<(Rect, NodeId)>,
    /// True while measuring content width: lines are left-aligned.
    measuring: bool,
    /// Sizes imposed by a flex container: (outer width, outer height).
    forced: HashMap<NodeId, (Option<f32>, Option<f32>)>,
    /// One list per open containing block of absolutely positioned boxes.
    abs: Vec<Vec<AbsPending>>,
    fixed: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
    /// Floats placed so far, one list per block formatting context.
    floats: Vec<Vec<FloatBox>>,
    /// Extra top padding pushed into a table cell to center or bottom-align its content.
    pad_top: HashMap<NodeId, f32>,
    /// Positioned boxes with a z-index, painted below (negative) or above (positive)
    /// the normal flow: (z-index, items).
    layers: Vec<(i32, Vec<Item>)>,
    /// Set by block_children just before laying out a child in normal flow (its margins
    /// can collapse with its parent's); every other caller lays a box out on its own.
    flow_child: bool,
    /// A first child whose top margin its parent already took into its own.
    absorb_top: Option<NodeId>,
    /// Content heights of the enclosing containing blocks, when definite (percentage
    /// heights resolve against the innermost one; `None` makes them act as auto).
    cb_heights: Vec<Option<f32>>,
}

/// Merge z-index layers into a display list: negative ones underneath, positive on top.
fn merge_layers(items: Vec<Item>, mut layers: Vec<(i32, Vec<Item>)>) -> Vec<Item> {
    if layers.is_empty() {
        return items;
    }
    layers.sort_by_key(|(z, _)| *z); // stable: equal z keeps document order
    let mut out = Vec::new();
    for (_, l) in layers.iter_mut().filter(|(z, _)| *z < 0) {
        out.append(l);
    }
    out.extend(items);
    for (_, l) in layers.iter_mut().filter(|(z, _)| *z >= 0) {
        out.append(l);
    }
    out
}

/// A table's cells placed on its grid, with each column's min and max content widths.
struct TableGrid {
    captions: Vec<NodeId>,
    rows: Vec<NodeId>,
    cells: Vec<TableCell>,
    ncols: usize,
    min: Vec<f32>,
    max: Vec<f32>,
    spacing: f32,
}

struct TableCell {
    node: NodeId,
    row: usize,
    col: usize,
    rowspan: usize,
    colspan: usize,
}

impl TableGrid {
    fn edges(&self) -> f32 {
        self.spacing * (self.ncols as f32 + 1.0)
    }
    /// The width the table would like: every column at its widest content.
    fn preferred(&self) -> f32 {
        self.max.iter().sum::<f32>().max(self.min.iter().sum::<f32>()) + self.edges()
    }
    /// Column widths for a table whose content box is `w` wide.
    fn widths(&self, w: f32, stretch: bool) -> Vec<f32> {
        let avail = (w - self.edges()).max(0.0);
        let (smin, smax): (f32, f32) = (self.min.iter().sum(), self.max.iter().sum());
        if smax <= avail {
            if stretch && smax > 0.0 {
                self.max.iter().map(|m| m + (avail - smax) * m / smax).collect()
            } else if stretch {
                vec![avail / self.ncols.max(1) as f32; self.ncols]
            } else {
                self.max.clone()
            }
        } else if smin >= avail {
            self.min.clone()
        } else {
            let span: f32 = self.max.iter().zip(&self.min).map(|(a, b)| a - b).sum();
            self.min
                .iter()
                .zip(&self.max)
                .map(|(mn, mx)| mn + if span > 0.0 { (avail - smin) * (mx - mn) / span } else { 0.0 })
                .collect()
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FloatBox {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    left: bool,
}

/// One unit of inline content.
enum Tok {
    /// `glue`: no space before it (a piece of a word split where a line may break);
    /// `hyphen`: a hyphen shows if the line breaks right after it.
    Word { text: String, node: NodeId, link: Option<String>, glue: bool, hyphen: bool },
    Space { node: NodeId },
    Break { node: NodeId },
    Image { w: f32, h: f32, node: NodeId, src: Option<String>, link: Option<String> },
    /// Start/end of an inline element with a visible box (background, border, padding).
    /// `w` = its left (open) or right (close) padding + border.
    Open { node: NodeId, w: f32 },
    Close { node: NodeId, w: f32 },
    /// An inline-block, laid out in place as its own little block.
    InlineBlock { node: NodeId, link: Option<String> },
}

/// Adjoining vertical margins being collapsed: the largest positive one plus the most
/// negative one is the gap they leave.
#[derive(Clone, Copy, Default)]
struct Margins {
    pos: f32,
    neg: f32,
}

impl Margins {
    fn of(m: f32) -> Margins {
        Margins::default().add(m)
    }
    fn add(self, m: f32) -> Margins {
        Margins { pos: self.pos.max(m), neg: self.neg.min(m) }
    }
    fn join(self, o: Margins) -> Margins {
        Margins { pos: self.pos.max(o.pos), neg: self.neg.min(o.neg) }
    }
    fn total(self) -> f32 {
        self.pos + self.neg
    }
}

/// Where vertical-align puts an item on its line.
#[derive(Clone, Copy, PartialEq)]
enum VPos {
    /// Raise the item's baseline this far above the line's baseline (negative lowers it).
    Raise(f32),
    /// Align the item's top (or bottom) with the line box's.
    Top,
    Bottom,
}

/// A piece of layout produced away from its final position (inline-blocks).
struct Detached {
    items: Vec<Item>,
    links: Vec<(Rect, String)>,
    hits: Vec<(Rect, NodeId)>,
    w: f32,
    h: f32,
    /// Distance from the top to the baseline of its last line of text (or its bottom).
    baseline: f32,
}

fn shift_item(it: &mut Item, dx: f32, dy: f32) {
    match it {
        Item::Rect { x, y, .. }
        | Item::Text { x, y, .. }
        | Item::Image { x, y, .. }
        | Item::Mark { x, y, .. }
        | Item::ScrollArea { x, y, .. }
        | Item::Box { x, y, .. }
        | Item::Ring { x, y, .. }
        | Item::Shadow { x, y, .. }
        | Item::ClipPush { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Item::StickyPush { y, .. } => *y += dy,
        Item::TransformPush { m, x, y, .. } => {
            *x += dx;
            *y += dy;
            *m = crate::transform::mul(crate::transform::translate(dx, dy), crate::transform::mul(*m, crate::transform::translate(-dx, -dy)));
        }
        Item::EffectPush(fx) => fx.shift(dx, dy),
        Item::TransformPop | Item::EffectPop => {}
        Item::ClipPop | Item::FixedPush | Item::FixedPop | Item::StickyPop => {}
    }
}

/// An absolutely positioned box waiting for its containing block's size.
/// (node, where it would have been in normal flow: x, y)
type AbsPending = (NodeId, f32, f32);

/// A flex item: an element, or a run of loose text inside the container.
enum FlexItem {
    Node(NodeId),
    Text(Vec<NodeId>),
}

struct Placed {
    x: f32,
    w: f32,
    tok: usize,
    /// Inline boxes that start just before this item: (node, box start x).
    opens: Vec<(NodeId, f32)>,
    /// Inline boxes that end just after this item: (node, box end x).
    closes: Vec<(NodeId, f32)>,
}

#[derive(Default)]
struct Line {
    items: Vec<Placed>,
    width: f32,
    /// Space left free by floats: the line starts `x_off` in and is `max_w` wide.
    x_off: f32,
    max_w: f32,
    /// The line was pushed down below a float to this y.
    skip_to: Option<f32>,
    /// A <br> on an otherwise empty line still takes up a line of height.
    blank: Option<usize>,
}

impl<'a> Ctx<'a> {
    fn is_block(&self, id: NodeId) -> bool {
        matches!(self.dom.nodes[id].data, NodeData::Element { .. })
            && matches!(self.styles[id].display, Display::Block | Display::ListItem)
    }

    /// Does this block start a new block formatting context of its own? Its margins then
    /// never collapse with its children's.
    fn own_bfc(&self, id: NodeId) -> bool {
        let st = &self.styles[id];
        st.clip
            || st.flow_root
            || st.flex.container
            || st.grid.is_some()
            || st.table_role != TableRole::None
            || st.float.is_some()
            || matches!(st.position, Position::Absolute | Position::Fixed)
            || st.display == Display::InlineBlock
            || st.multicol.is_some_and(|m| m.count.is_some() || m.width.is_some())
            || self.dom.nodes[id].parent == Some(Dom::ROOT)
            || matches!(self.dom.tag(id), Some("img" | "iframe" | "input" | "select" | "textarea" | "button"))
    }

    /// The first in-flow child, if it is a block with nothing but whitespace before it.
    fn first_flow_block(&self, id: NodeId) -> Option<NodeId> {
        for &c in &self.dom.nodes[id].children {
            match &self.dom.nodes[c].data {
                NodeData::Text(t) if t.trim().is_empty() && !self.styles[c].pre => continue,
                NodeData::Element { .. } => {}
                NodeData::Text(_) => return None,
                _ => continue,
            }
            if self.styles[c].display == Display::None || self.is_out_of_flow(c) {
                continue;
            }
            if self.is_float(c) || !self.is_block(c) {
                return None;
            }
            return Some(c);
        }
        None
    }

    /// Can `id`'s top margin collapse with its first child's (no border, padding or new
    /// formatting context between them)?
    fn top_collapses(&self, id: NodeId, avail: f32) -> bool {
        let st = &self.styles[id];
        st.border[0] == 0.0 && st.padding[0].resolve(avail) == 0.0 && !self.pad_top.contains_key(&id) && !self.own_bfc(id)
    }

    /// `id`'s top margin together with the top margins of the first children it collapses with.
    fn top_chain(&self, id: NodeId, avail: f32) -> Margins {
        let st = &self.styles[id];
        let m = Margins::of(st.margin[0].resolve(avail));
        if !self.top_collapses(id, avail) {
            return m;
        }
        match self.first_flow_block(id) {
            Some(c) => {
                let inner = self.content_width_of(id, avail);
                m.join(self.top_chain(c, inner))
            }
            None => m,
        }
    }

    /// Rough content width of a normal-flow block (for resolving its children's percentage
    /// margins while collapsing).
    fn content_width_of(&self, id: NodeId, avail: f32) -> f32 {
        let st = &self.styles[id];
        let edges = st.padding[1].resolve(avail) + st.padding[3].resolve(avail) + st.border[1] + st.border[3];
        let outer = avail - st.margin[1].resolve(avail) - st.margin[3].resolve(avail);
        match st.width {
            Len::Auto => outer - edges,
            l => l.resolve(avail) - if st.border_box { edges } else { 0.0 },
        }
        .max(0.0)
    }

    /// A height-like length in px: percentages need a containing block of definite height.
    fn definite_len(&self, l: Len) -> Option<f32> {
        match l {
            Len::Auto => None,
            Len::Px(v) => Some(v),
            Len::Pct(_) | Len::Mix(..) => self.cb_heights.last().copied().flatten().map(|ph| l.resolve(ph)),
        }
    }

    /// Lay out an element as a block box. Returns (outer height, bottom margin).
    fn block(&mut self, id: NodeId, x: f32, y: f32, avail: f32, prev_mb: f32) -> (f32, f32) {
        let flow = std::mem::replace(&mut self.flow_child, false);
        let absorbed = self.absorb_top == Some(id);
        if absorbed {
            self.absorb_top = None;
        }
        let st = &self.styles[id];
        let forced = self.forced.get(&id).copied().unwrap_or((None, None));
        let pct = |l: Len| l.resolve(avail);
        let mt = pct(st.margin[0]);
        let mb = pct(st.margin[2]);
        let mut ml = pct(st.margin[3]);
        let mut mr = pct(st.margin[1]);
        let [pt, pr, pb, pl] = st.padding.map(pct);
        let pt = pt + self.pad_top.get(&id).copied().unwrap_or(0.0);
        let [bt, br, bb, bl] = st.border;
        let extra = pl + pr + bl + br;
        // With border-box sizing, width and max-width already include padding and border.
        let sizing = if st.border_box { extra } else { 0.0 };
        let vsizing = if st.border_box { pt + pb + bt + bb } else { 0.0 };
        let mut w = match st.width {
            Len::Auto => avail - ml - mr - extra,
            l => pct(l) - sizing,
        };
        let mut constrained = st.width != Len::Auto;
        // A block-level <img> takes the image's size instead of its children's.
        let replaced = if matches!(self.dom.tag(id), Some("img" | "iframe")) { self.replaced_size(id, avail - ml - mr - extra) } else { None };
        if let Some((rw, _)) = replaced {
            w = rw;
            constrained = true;
        }
        if st.max_width != Len::Auto && replaced.is_none() {
            let max = pct(st.max_width) - sizing;
            if w > max {
                w = max;
                constrained = true;
            }
        }
        if st.min_width != Len::Auto {
            w = w.max(pct(st.min_width) - sizing);
        }
        // Form controls have their own natural width. While measuring how wide something
        // wants to be, a percentage width has nothing to be a percentage of, so it counts
        // as the natural width too (as in other browsers).
        let ctrl = self.control_kind(id);
        let pct_while_measuring = self.measuring && matches!(self.styles[id].width, Len::Pct(_) | Len::Mix(..));
        if ctrl.is_some() && forced.0.is_none() && (self.styles[id].width == Len::Auto || pct_while_measuring) {
            w = self.control_width(id);
            constrained = true;
        }
        let st = &self.styles[id];
        // Tables size themselves from their cells' content.
        let table = if st.table_role == TableRole::Table && replaced.is_none() {
            let grid = self.table_grid(id);
            if self.styles[id].width == Len::Auto && forced.0.is_none() {
                w = w.min(grid.preferred()).max(grid.min.iter().sum::<f32>() + grid.edges());
                constrained = true;
            }
            Some(grid)
        } else {
            None
        };
        let st = &self.styles[id];
        // A flex container decides this item's outer width.
        if let Some(fw) = forced.0 {
            w = fw - ml - mr - extra;
            constrained = false;
        }
        w = w.max(0.0);
        if constrained {
            let free = avail - w - extra;
            match (st.margin[3] == Len::Auto, st.margin[1] == Len::Auto) {
                (true, true) => {
                    ml = (free / 2.0).max(0.0);
                    mr = ml;
                }
                (true, false) => ml = (free - mr).max(0.0),
                _ => {}
            }
        }
        let _ = mr;
        // Collapse our top margin with the previous sibling's bottom margin, and (through a
        // parent with no border or padding on top) with our first child's top margin. A
        // first child whose margin its parent already took sits right at the top.
        let bfc = !flow || self.own_bfc(id);
        let top_coll = !bfc && self.top_collapses(id, avail);
        let chain = match (absorbed, flow) {
            (true, _) => Margins::default(),
            (false, true) => self.top_chain(id, avail),
            (false, false) => Margins::of(mt),
        };
        let mt_eff = if absorbed { 0.0 } else { Margins::of(prev_mb).join(chain).total() - prev_mb };
        let top = y + mt_eff;
        // Remember where this box's output starts (for position: relative).
        let start = (self.items.len(), self.links.len(), self.hits.len());
        let layers_start = self.layers.len();
        let positioned = st.position != Position::Static;
        if positioned {
            self.abs.push(Vec::new());
        }
        // Slots filled in once the height is known: shadow, background, clip.
        let shadow_slot = self.items.len();
        self.items.push(Item::ClipPop);
        let bg_slot = self.items.len();
        self.items.push(Item::ClipPop);
        let clip_slot = self.items.len();
        self.items.push(Item::ClipPop);
        let placeholder = |it: &mut Item| *it = Item::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0, color: Color::TRANSPARENT };

        let cx = x + ml + bl + pl;
        let cy = top + bt + pt;
        let align = st.effective_align();
        let is_flex = st.flex.container;
        // overflow: hidden and display: flow-root start a new float context that contains its floats.
        let new_bfc = st.clip || st.flow_root;
        if new_bfc {
            self.floats.push(Vec::new());
        }
        if top_coll {
            self.absorb_top = self.first_flow_block(id);
        }
        // Heights: lengths, and percentages of a containing block whose height is definite.
        let st = &self.styles[id];
        let spec_h = self.definite_len(st.height).map(|h| h - vsizing);
        let max_h = self.definite_len(st.max_height).map(|h| h - vsizing);
        let min_h = self.definite_len(st.min_height).map(|h| h - vsizing);
        let definite = match forced.1 {
            Some(fh) => Some(fh - mt - mb - bt - pt - pb - bb),
            None => spec_h.map(|h| match max_h {
                Some(mx) => h.min(mx),
                None => h,
            }),
        }
        .map(|h| h.max(min_h.unwrap_or(0.0)).max(0.0));
        self.cb_heights.push(definite);
        let (mut content_h, last_mb) = match replaced {
            Some((rw, rh)) => {
                let src = self.replaced_src(id);
                if !self.styles[id].hidden {
                    self.push_image(id, cx, cy, rw, rh, src);
                }
                self.hits.push((Rect { x: cx, y: cy, w: rw, h: rh }, id));
                (rh, 0.0)
            }
            None if ctrl.is_some() => (self.control_content(id, cx, cy, w), 0.0),
            None if table.is_some() => {
                let stretch = self.styles[id].width != Len::Auto || forced.0.is_some();
                (self.table_children(table.as_ref().unwrap(), cx, cy, w, stretch), 0.0)
            }
            None if self.styles[id].grid.is_some() => (self.grid_children(id, cx, cy, w, definite), 0.0),
            None if is_flex => (self.flex_children(id, cx, cy, w, definite), 0.0),
            None if self.columns_of(id, w).is_some() => (self.multicol(id, cx, cy, w, align), 0.0),
            None => self.block_children(id, cx, cy, w, align),
        };
        self.cb_heights.pop();
        if self.absorb_top.is_some_and(|c| self.dom.nodes[c].parent == Some(id)) {
            self.absorb_top = None;
        }
        if new_bfc {
            content_h = content_h.max(self.float_bottom(true, true) - cy);
            self.floats.pop();
        }
        // The last child's bottom margin collapses through a parent with no border, padding
        // or fixed height below it: it becomes part of the parent's bottom margin.
        let st = &self.styles[id];
        let bottom_coll = !bfc
            && pb == 0.0
            && bb == 0.0
            && spec_h.is_none()
            && matches!(st.min_height, Len::Auto | Len::Px(0.0))
            && forced.1.is_none()
            && st.aspect_ratio.is_none();
        if bottom_coll {
            content_h -= last_mb;
        }
        let st = &self.styles[id];
        let natural_h = content_h;
        if let (None, Some(r), None) = (spec_h, st.aspect_ratio, replaced) {
            // aspect-ratio gives an auto height from the width (content may still grow it).
            let h = if st.border_box { (w + pl + pr + bl + br) / r - vsizing } else { w / r };
            content_h = content_h.max(h);
        }
        if let (Some(h), None) = (spec_h, replaced) {
            // A clipping box keeps exactly its height; others grow to fit (for now).
            content_h = if st.clip { h } else { content_h.max(h) };
        }
        if let (Some(mx), None) = (max_h, replaced) {
            content_h = content_h.min(mx);
        }
        if let Some(mn) = min_h {
            content_h = content_h.max(mn);
        }
        content_h = content_h.max(0.0);
        if let Some(fh) = forced.1 {
            content_h = content_h.max(fh - mt - mb - bt - pt - pb - bb);
        }
        if st.display == Display::ListItem && st.list_style != ListStyle::None && !st.hidden {
            self.list_marker(id, cx, cy);
        }
        let st = &self.styles[id];
        let box_w = w + extra;
        let box_h = bt + pt + content_h + pb + bb;
        let bx = x + ml;
        let radius = st.radius.resolve(box_w.min(box_h)).min(box_w.min(box_h) / 2.0);
        match st.shadow {
            Some(sh) if !st.hidden => {
                self.items[shadow_slot] = Item::Shadow {
                    x: bx + sh.x - sh.spread,
                    y: top + sh.y - sh.spread,
                    w: box_w + 2.0 * sh.spread,
                    h: box_h + 2.0 * sh.spread,
                    radius: radius + sh.spread.max(0.0),
                    blur: sh.blur,
                    color: sh.color,
                }
            }
            _ => placeholder(&mut self.items[shadow_slot]),
        }
        if st.bg.visible() && !st.hidden {
            self.items[bg_slot] = Item::Box { x: bx, y: top, w: box_w, h: box_h, radius, bg: Box::new(st.bg.clone()) };
        } else {
            placeholder(&mut self.items[bg_slot]);
        }
        // Scrollable boxes: move the content by the scroll offset, then (for any clipping
        // box) keep links and hit areas to the visible part.
        if st.clip && !self.measuring {
            let clip = Rect { x: bx + bl, y: top + bt, w: box_w - bl - br, h: box_h - bt - bb };
            let overflow = (natural_h - content_h).max(0.0);
            if st.scrolls && overflow > 0.5 {
                let off = element_scroll(id).min(overflow);
                if off > 0.0 {
                    for it in &mut self.items[clip_slot + 1..] {
                        shift_item(it, 0.0, -off);
                    }
                    for (r, _) in &mut self.links[start.1..] {
                        r.y -= off;
                    }
                    for (r, _) in &mut self.hits[start.2..] {
                        r.y -= off;
                    }
                }
                self.items.push(Item::ScrollArea { id, x: clip.x, y: clip.y, w: clip.w, h: clip.h, max: overflow });
                // A thin scrollbar thumb along the right edge.
                if !st.hidden && clip.h > 20.0 {
                    let track = clip.h - 4.0;
                    let thumb = (track * clip.h / (clip.h + overflow)).max(16.0);
                    let ty = clip.y + 2.0 + (track - thumb) * (off / overflow);
                    self.items.push(Item::Box {
                        x: clip.x + clip.w - 6.0,
                        y: ty,
                        w: 4.0,
                        h: thumb,
                        radius: 2.0,
                        bg: Box::new(Background { color: Color { r: 0, g: 0, b: 0, a: 90 }, ..Default::default() }),
                    });
                }
            }
            let cut = |r: &mut Rect| {
                let (x0, y0) = (r.x.max(clip.x), r.y.max(clip.y));
                let (x1, y1) = ((r.x + r.w).min(clip.x + clip.w), (r.y + r.h).min(clip.y + clip.h));
                *r = Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) };
            };
            for (r, _) in &mut self.links[start.1..] {
                cut(r);
            }
            for (r, _) in &mut self.hits[start.2..] {
                cut(r);
            }
        }
        // overflow: hidden clips children to the padding box.
        if st.clip {
            self.items[clip_slot] = Item::ClipPush { x: bx + bl, y: top + bt, w: box_w - bl - br, h: box_h - bt - bb, radius: (radius - bt).max(0.0) };
            self.items.push(Item::ClipPop);
        } else {
            placeholder(&mut self.items[clip_slot]);
        }
        let [ct, cr, cbm, cl] = st.side_colors();
        let c = ct;
        let uniform = bt == br && bt == bb && bt == bl && ct == cr && ct == cbm && ct == cl;
        if !st.hidden && radius > 0.0 && uniform && bt > 0.0 {
            self.items.push(Item::Ring { x: bx, y: top, w: box_w, h: box_h, radius, width: bt, color: c });
        } else if !st.hidden {
            for (rx, ry, rw, rh, c) in [
                (bx, top, box_w, bt, ct),
                (bx, top + box_h - bb, box_w, bb, cbm),
                (bx, top, bl, box_h, cl),
                (bx + box_w - br, top, br, box_h, cr),
            ] {
                if rw > 0.0 && rh > 0.0 {
                    border_side(&mut self.items, st.border_line, rx, ry, rw, rh, c);
                }
            }
        }
        if !self.measuring {
            self.items.push(Item::Mark { id, x: bx, y: top, w: box_w, h: box_h });
        }
        // Absolutely positioned descendants are placed against this box's padding box.
        if positioned {
            let pending = self.abs.pop().unwrap_or_default();
            let cb = Rect { x: bx + bl, y: top + bt, w: box_w - bl - br, h: box_h - bt - bb };
            self.place_absolutes(pending, cb);
        }
        // position: sticky moves with scrolling at paint time; wrap its items.
        let st = &self.styles[id];
        let sticky_top = match (st.position, st.inset[0]) {
            (Position::Sticky, Len::Auto) => None,
            (Position::Sticky, t) => Some(t.resolve(avail)),
            _ => None,
        };
        if let (Some(top_inset), false) = (sticky_top, self.measuring) {
            self.items.insert(start.0, Item::StickyPush { id, y: top, h: box_h, top: top_inset, limit: 0.0 });
            self.items.push(Item::StickyPop);
        }
        // A stacking context (a transform, or a positioned box with a z-index) keeps the
        // z-index layers of its descendants inside itself, ordered among its own content.
        let forms_context = self.styles[id].transform.is_some() || self.styles[id].filter.is_some() || self.styles[id].backdrop.is_some() || positioned && self.styles[id].z_index != 0;
        if forms_context && self.layers.len() > layers_start && !self.measuring {
            let inner: Vec<(i32, Vec<Item>)> = self.layers.drain(layers_start..).collect();
            let own: Vec<Item> = self.items.drain(start.0..).collect();
            let merged = merge_layers(own, inner);
            self.items.extend(merged);
        }
        // filter / backdrop-filter: the element's items are drawn off-screen and filtered.
        let (fil, bd) = (self.styles[id].filter.clone(), self.styles[id].backdrop.clone());
        if (fil.is_some() || bd.is_some()) && !self.measuring {
            let area = self.items_bounds(start.0, (bx, top, box_w, box_h));
            let fx = crate::effects::Effect {
                x: bx,
                y: top,
                w: box_w,
                h: box_h,
                radius,
                area,
                filter: fil.map(|f| f.to_vec()).unwrap_or_default(),
                backdrop: bd.map(|f| f.to_vec()).unwrap_or_default(),
            };
            self.items.insert(start.0, Item::EffectPush(Box::new(fx)));
            self.items.push(Item::EffectPop);
        }
        // CSS transforms. A pure translation just moves everything; anything else is drawn
        // off-screen and mapped through the matrix at paint time.
        let mut transformed = false;
        if let (Some(spec), false) = (self.styles[id].transform.clone(), self.measuring) {
            let origin = self.styles[id].transform_origin.clone();
            let fs = self.styles[id].font_size;
            if let Some(local) = crate::transform::matrix(&spec, origin.as_deref().unwrap_or(""), box_w, box_h, fs) {
                let m = crate::transform::mul(crate::transform::translate(bx, top), crate::transform::mul(local, crate::transform::translate(-bx, -top)));
                transformed = true;
                if crate::transform::is_translation(&m) {
                    self.shift_since(start, m[4], m[5]);
                } else {
                    let (ax, ay, aw, ah) = self.items_bounds(start.0, (bx, top, box_w, box_h));
                    self.items.insert(start.0, Item::TransformPush { m, x: ax, y: ay, w: aw, h: ah });
                    self.items.push(Item::TransformPop);
                    let map = |r: &mut Rect| {
                        let (x, y, w, h) = crate::transform::bounds(&m, r.x, r.y, r.w, r.h);
                        *r = Rect { x, y, w, h };
                    };
                    self.links[start.1..].iter_mut().for_each(|(r, _)| map(r));
                    self.hits[start.2..].iter_mut().for_each(|(r, _)| map(r));
                }
            }
        }
        // position: relative shifts the box after layout.
        let st = &self.styles[id];
        if st.position == Position::Relative {
            let off = |a: Len, b: Len| match (a, b) {
                (Len::Auto, Len::Auto) => 0.0,
                (Len::Auto, b) => -b.resolve(avail),
                (a, _) => a.resolve(avail),
            };
            let dx = off(st.inset[3], st.inset[1]);
            let dy = off(st.inset[0], st.inset[2]);
            if dx != 0.0 || dy != 0.0 {
                self.shift_since(start, dx, dy);
            }
        }
        // A positioned box with a z-index paints in its own layer.
        let z = self.styles[id].z_index;
        let _ = transformed;
        if positioned && (z != 0 || sticky_top.is_some()) && !matches!(self.styles[id].position, Position::Absolute | Position::Fixed) {
            let lifted: Vec<Item> = self.items.drain(start.0..).collect();
            self.layers.push((z, lifted));
        }
        // An empty box lets its margins collapse through it with the ones around it.
        if bottom_coll && bt == 0.0 && pt == 0.0 && box_h <= 0.0 {
            let through = Margins::of(prev_mb).join(chain).add(mb).add(last_mb).total();
            return (through - prev_mb, through);
        }
        let mb_out = if bottom_coll { Margins::of(mb).add(last_mb).total() } else { mb };
        (mt_eff + box_h + mb_out, mb_out)
    }

    /// Page-area bounding box of the paint items from index `from` on, at least `base`.
    fn items_bounds(&self, from: usize, base: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let (mut x0, mut y0, mut x1, mut y1) = (base.0, base.1, base.0 + base.2, base.1 + base.3);
        let mut grow = |x: f32, y: f32, w: f32, h: f32| {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + w);
            y1 = y1.max(y + h);
        };
        for it in &self.items[from..] {
            match it {
                Item::Rect { x, y, w, h, .. } | Item::Image { x, y, w, h, .. } | Item::Box { x, y, w, h, .. } | Item::Ring { x, y, w, h, .. } => grow(*x, *y, *w, *h),
                Item::Shadow { x, y, w, h, blur, .. } => grow(x - blur, y - blur, w + 2.0 * blur, h + 2.0 * blur),
                Item::Text { x, y, text, font, size, .. } => grow(*x, y - size, self.fonts.width(*font, text, *size), size * 1.4),
                Item::TransformPush { m, x, y, w, h } => {
                    let (bx, by, bw, bh) = crate::transform::bounds(m, *x, *y, *w, *h);
                    grow(bx, by, bw, bh)
                }
                Item::EffectPush(fx) => {
                    let m = fx.margin();
                    grow(fx.area.0 - m, fx.area.1 - m, fx.area.2 + 2.0 * m, fx.area.3 + 2.0 * m)
                }
                _ => {}
            }
        }
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// Move everything emitted since `start` by (dx, dy).
    fn shift_since(&mut self, start: (usize, usize, usize), dx: f32, dy: f32) {
        for it in &mut self.items[start.0..] {
            shift_item(it, dx, dy);
        }
        for (r, _) in &mut self.links[start.1..] {
            r.x += dx;
            r.y += dy;
        }
        for (r, _) in &mut self.hits[start.2..] {
            r.x += dx;
            r.y += dy;
        }
    }

    fn is_float(&self, id: NodeId) -> bool {
        matches!(self.dom.nodes[id].data, NodeData::Element { .. })
            && self.styles[id].float.is_some()
            && self.styles[id].display != Display::None
            && !self.is_out_of_flow(id)
    }

    fn has_floats(&self) -> bool {
        self.floats.last().map_or(false, |f| !f.is_empty())
    }

    /// Lowest bottom edge of the floats on the given sides in the current context.
    fn float_bottom(&self, left: bool, right: bool) -> f32 {
        self.floats
            .last()
            .map(|fs| fs.iter().filter(|f| (f.left && left) || (!f.left && right)).map(|f| f.y1).fold(0.0, f32::max))
            .unwrap_or(0.0)
    }

    /// Free horizontal space inside [x, x+w] for a band of height h starting at y:
    /// (offset from x, width).
    fn band(&self, x: f32, w: f32, y: f32, h: f32) -> (f32, f32) {
        let (mut l, mut r) = (x, x + w);
        if let Some(fs) = self.floats.last() {
            for f in fs {
                if f.y1 > y && f.y0 < y + h.max(1.0) && f.x1 > x && f.x0 < x + w {
                    if f.left {
                        l = l.max(f.x1);
                    } else {
                        r = r.min(f.x0);
                    }
                }
            }
        }
        (l - x, (r - l).max(0.0))
    }

    /// Place a floated element at the left or right edge, as high as it fits from y down.
    fn place_float(&mut self, id: NodeId, x: f32, y: f32, w: f32) {
        let left = self.styles[id].float == Some(true);
        let d = self.layout_detached(id, w);
        let (dw, dh) = (d.w, d.h);
        let mut fy = y;
        // Never above an earlier float (floats keep their order).
        if let Some(fs) = self.floats.last() {
            if let Some(last) = fs.last() {
                fy = fy.max(last.y0);
            }
        }
        let (cl, cr) = self.styles[id].clear;
        if cl || cr {
            fy = fy.max(self.float_bottom(cl, cr));
        }
        for _ in 0..64 {
            let (off, avail) = self.band(x, w, fy, dh);
            if avail + 0.5 >= dw || !self.has_floats() {
                let fx = if left { x + off } else { x + off + avail - dw };
                let fx = fx.max(x);
                self.place_detached(d, fx, fy);
                if let Some(fs) = self.floats.last_mut() {
                    fs.push(FloatBox { x0: fx, x1: fx + dw, y0: fy, y1: fy + dh, left });
                }
                return;
            }
            // Not enough room: drop below the float that ends first.
            let next = self
                .floats
                .last()
                .and_then(|fs| fs.iter().filter(|f| f.y1 > fy).map(|f| f.y1).reduce(f32::min));
            match next {
                Some(n) => fy = n,
                None => break,
            }
        }
        let fx = if left { x } else { x + w - dw };
        self.place_detached(d, fx, fy);
        if let Some(fs) = self.floats.last_mut() {
            fs.push(FloatBox { x0: fx, x1: fx + dw, y0: fy, y1: fy + dh, left });
        }
    }

    /// A new line, narrowed by any floats beside it.
    fn new_line(&self, x: f32, w: f32, y: f32) -> Line {
        let (x_off, max_w) = if self.has_floats() { self.band(x, w, y, 16.0) } else { (0.0, w) };
        Line { x_off, max_w, ..Line::default() }
    }

    fn is_out_of_flow(&self, id: NodeId) -> bool {
        matches!(self.dom.nodes[id].data, NodeData::Element { .. })
            && matches!(self.styles[id].position, Position::Absolute | Position::Fixed)
    }

    /// Hold an absolutely positioned box until its containing block is laid out.
    fn defer_absolute(&mut self, id: NodeId, x: f32, y: f32) {
        let fixed = self.styles[id].position == Position::Fixed;
        let frame = if fixed { self.abs.first_mut() } else { self.abs.last_mut() };
        if let Some(f) = frame {
            f.push((id, x, y));
        }
    }

    /// Lay out absolutely positioned boxes against containing block `cb`.
    fn place_absolutes(&mut self, pending: Vec<AbsPending>, cb: Rect) {
        for (id, sx, sy) in pending {
            let st = &self.styles[id];
            let [top, right, bottom, left] = st.inset;
            let horiz = |l: Len| l.resolve(cb.w);
            let vert = |l: Len| l.resolve(cb.h);
            // Both sides set and no width: stretch between them.
            if st.width == Len::Auto && left != Len::Auto && right != Len::Auto {
                self.forced.insert(id, (Some((cb.w - horiz(left) - horiz(right)).max(0.0)), None));
            }
            if st.height == Len::Auto && top != Len::Auto && bottom != Len::Auto {
                let e = self.forced.entry(id).or_insert((None, None));
                e.1 = Some((cb.h - vert(top) - vert(bottom)).max(0.0));
            }
            self.cb_heights.push(Some(cb.h));
            let d = match self.forced.get(&id).copied() {
                Some((Some(fw), fh)) => self.layout_at(id, fw, fh),
                _ => self.layout_detached(id, cb.w),
            };
            self.cb_heights.pop();
            self.forced.remove(&id);
            let fixed = self.styles[id].position == Position::Fixed;
            let x = match (left, right) {
                (Len::Auto, Len::Auto) => sx,
                (Len::Auto, r) => cb.x + cb.w - horiz(r) - d.w,
                (l, _) => cb.x + horiz(l),
            };
            let y = match (top, bottom) {
                (Len::Auto, Len::Auto) => sy,
                (Len::Auto, b) => cb.y + cb.h - vert(b) - d.h,
                (t, _) => cb.y + vert(t),
            };
            let (l0, h0) = (self.links.len(), self.hits.len());
            let i0 = self.items.len();
            if fixed {
                self.items.push(Item::FixedPush);
            }
            self.place_detached(d, x, y);
            if fixed {
                self.items.push(Item::FixedPop);
                self.fixed.push((l0..self.links.len(), h0..self.hits.len()));
            }
            let z = self.styles[id].z_index;
            if z != 0 {
                let lifted: Vec<Item> = self.items.drain(i0..).collect();
                self.layers.push((z, lifted));
            }
        }
    }

    fn place_detached(&mut self, d: Detached, dx: f32, dy: f32) {
        for mut it in d.items {
            shift_item(&mut it, dx, dy);
            self.items.push(it);
        }
        let mv = |r: Rect| Rect { x: r.x + dx, y: r.y + dy, ..r };
        self.links.extend(d.links.into_iter().map(|(r, l)| (mv(r), l)));
        self.hits.extend(d.hits.into_iter().map(|(r, n)| (mv(r), n)));
    }

    /// Run `f` with empty output lists; return what it produced as a Detached box.
    /// Absolute boxes inside are placed against the detached box itself.
    fn detach(&mut self, f: impl FnOnce(&mut Self) -> (f32, f32)) -> Detached {
        let saved = (std::mem::take(&mut self.items), std::mem::take(&mut self.links), std::mem::take(&mut self.hits));
        self.abs.push(Vec::new());
        self.floats.push(Vec::new());
        let saved_layers = std::mem::take(&mut self.layers);
        let (w, h) = f(self);
        // A detached box contains its own floats.
        let h = h.max(self.float_bottom(true, true));
        self.floats.pop();
        let pending = self.abs.pop().unwrap_or_default();
        self.place_absolutes(pending, Rect { x: 0.0, y: 0.0, w, h });
        // z-index layers inside a detached box stack within it.
        let mine = std::mem::replace(&mut self.layers, saved_layers);
        let items = merge_layers(std::mem::replace(&mut self.items, saved.0), mine);
        let links = std::mem::replace(&mut self.links, saved.1);
        let hits = std::mem::replace(&mut self.hits, saved.2);
        let baseline = items
            .iter()
            .filter_map(|it| if let Item::Text { y, .. } = it { Some(*y) } else { None })
            .fold(None, |m: Option<f32>, y| Some(m.map_or(y, |m| m.max(y))))
            .unwrap_or(h);
        Detached { items, links, hits, w, h, baseline }
    }

    /// Lay out an element at a given outer width (and optionally outer height).
    fn layout_at(&mut self, id: NodeId, outer_w: f32, outer_h: Option<f32>) -> Detached {
        let prev = self.forced.insert(id, (Some(outer_w), outer_h));
        let d = self.detach(|cx| (outer_w, cx.block(id, 0.0, 0.0, outer_w, 0.0).0));
        match prev {
            Some(p) => self.forced.insert(id, p),
            None => self.forced.remove(&id),
        };
        d
    }

    /// Rightmost edge of the content drawn in a detached box (text, images, inner boxes).
    fn content_right(&self, items: &[Item], skip_first_boxes: bool) -> f32 {
        let mut right = 0.0f32;
        for (i, it) in items.iter().enumerate() {
            let r = match it {
                Item::Text { x, text, font, size, .. } => x + self.fonts.width(*font, text, *size),
                Item::Image { x, w, .. } => x + w,
                Item::Box { x, w, .. } if !(skip_first_boxes && i < 3) => x + w,
                _ => 0.0,
            };
            right = right.max(r);
        }
        right
    }

    /// Children of a flex (or simple grid) container. Returns the content height.
    fn flex_children(&mut self, id: NodeId, x: f32, y: f32, w: f32, definite_h: Option<f32>) -> f32 {
        let fl = self.styles[id].flex;
        let col_gap = fl.column_gap.resolve(w);
        let row_gap = fl.row_gap.resolve(definite_h.unwrap_or(w));
        let container_align = self.styles[id].effective_align();

        // Gather items: every child element, plus runs of loose non-blank text.
        let mut items: Vec<FlexItem> = Vec::new();
        let mut text: Vec<NodeId> = Vec::new();
        for c in self.dom.nodes[id].children.clone() {
            match &self.dom.nodes[c].data {
                NodeData::Text(t) => {
                    if !t.trim().is_empty() || !text.is_empty() {
                        text.push(c);
                    }
                }
                NodeData::Element { .. } => {
                    if self.styles[c].display == Display::None {
                        continue;
                    }
                    // Any element box, even an out-of-flow one, ends a run of loose text.
                    if !text.is_empty() {
                        items.push(FlexItem::Text(std::mem::take(&mut text)));
                    }
                    if self.is_out_of_flow(c) {
                        self.defer_absolute(c, x, y);
                        continue;
                    }
                    items.push(FlexItem::Node(c));
                }
                _ => {}
            }
        }
        if text.iter().any(|&t| matches!(&self.dom.nodes[t].data, NodeData::Text(s) if !s.trim().is_empty())) {
            items.push(FlexItem::Text(text));
        }
        let order = |it: &FlexItem| match it {
            FlexItem::Node(n) => self.styles[*n].flex.order,
            FlexItem::Text(_) => 0,
        };
        items.sort_by_key(|it| order(it));
        if fl.reverse {
            items.reverse();
        }
        if items.is_empty() {
            return definite_h.unwrap_or(0.0);
        }
        let align_of = |cx: &Self, it: &FlexItem| match it {
            FlexItem::Node(n) => cx.styles[*n].flex.align_self.unwrap_or(fl.align_items),
            FlexItem::Text(_) => fl.align_items,
        };

        if fl.column {
            // ---- Column: items stack vertically.
            let mut boxes: Vec<Detached> = Vec::new();
            for it in &items {
                let d = match it {
                    FlexItem::Node(n) => {
                        let stretch = align_of(self, it) == AlignItems::Stretch && self.styles[*n].width == Len::Auto;
                        if stretch {
                            self.layout_at(*n, w, None)
                        } else {
                            self.layout_detached(*n, w)
                        }
                    }
                    FlexItem::Text(nodes) => self.detach_text(nodes, w, container_align),
                };
                boxes.push(d);
            }
            let gaps = row_gap * (boxes.len().saturating_sub(1)) as f32;
            let natural: f32 = boxes.iter().map(|d| d.h).sum::<f32>() + gaps;
            // Grow into a definite height.
            if let Some(dh) = definite_h {
                let free = dh - natural;
                let total_grow: f32 = items.iter().map(|it| if let FlexItem::Node(n) = it { self.styles[*n].flex.grow } else { 0.0 }).sum();
                if free > 0.0 && total_grow > 0.0 {
                    for (i, it) in items.iter().enumerate() {
                        if let FlexItem::Node(n) = it {
                            let g = self.styles[*n].flex.grow;
                            if g > 0.0 {
                                let target = boxes[i].h + free * g / total_grow;
                                boxes[i] = self.layout_at(*n, boxes[i].w, Some(target));
                            }
                        }
                    }
                }
            }
            let used: f32 = boxes.iter().map(|d| d.h).sum::<f32>() + gaps;
            let free = definite_h.map_or(0.0, |dh| (dh - used).max(0.0));
            let (mut pos, between) = justify_offsets(fl.justify, free, boxes.len());
            for (i, d) in boxes.into_iter().enumerate() {
                let dx = match align_of(self, &items[i]) {
                    AlignItems::Center => (w - d.w) / 2.0,
                    AlignItems::End => w - d.w,
                    _ => 0.0,
                };
                let h = d.h;
                self.place_detached(d, x + dx.max(0.0), y + pos);
                pos += h + row_gap + between;
            }
            return (pos - row_gap - between).max(0.0).max(definite_h.unwrap_or(0.0));
        }

        // ---- Row: base sizes, then lines, then grow/shrink, then place.
        let grid = fl.grid_columns;
        let mut bases: Vec<f32> = Vec::new();
        for it in &items {
            let b = match (grid, it) {
                (Some(n), _) => (w - col_gap * (n as f32 - 1.0)) / n as f32,
                (None, FlexItem::Node(n)) => {
                    let st = &self.styles[*n];
                    let edges = st.padding[1].resolve(w) + st.padding[3].resolve(w) + st.border[1] + st.border[3]
                        + st.margin[1].resolve(w) + st.margin[3].resolve(w);
                    // border-box: the size already includes padding and border; add margins only.
                    let edges = if st.border_box { st.margin[1].resolve(w) + st.margin[3].resolve(w) } else { edges };
                    // Measuring an intrinsic size: percentages have no basis yet and act as auto.
                    let pct_measure = self.measuring && matches!(st.width, Len::Pct(_) | Len::Mix(..));
                    let width = if pct_measure { Len::Auto } else { st.width };
                    match (st.flex.basis, width) {
                        (Len::Auto, Len::Auto) if pct_measure && self.control_kind(*n).is_some() => self.control_outer(*n, w),
                        (Len::Auto, Len::Auto) => self.layout_detached(*n, w).w,
                        (Len::Auto, wl) => wl.resolve(w) + edges,
                        (b, _) => b.resolve(w) + edges,
                    }
                }
                (None, FlexItem::Text(nodes)) => self.detach_text(nodes, w, Align::Left).w,
            };
            bases.push(b.max(0.0));
        }
        // Break into lines.
        let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
        let mut used = 0.0;
        for (i, &b) in bases.iter().enumerate() {
            let line = lines.last_mut().unwrap();
            let need = if line.is_empty() { b } else { used + col_gap + b };
            let full = match grid {
                Some(n) => line.len() >= n,
                None => fl.wrap && !line.is_empty() && need > w + 0.5,
            };
            if full {
                lines.push(vec![i]);
                used = b;
            } else {
                line.push(i);
                used = need;
            }
        }
        let mut cy = y;
        let single_line = lines.len() == 1;
        for (li, line) in lines.iter().enumerate() {
            // Flexible lengths.
            let mut sizes: Vec<f32> = line.iter().map(|&i| bases[i]).collect();
            if grid.is_none() {
                let gaps = col_gap * (line.len().saturating_sub(1)) as f32;
                let free = w - sizes.iter().sum::<f32>() - gaps;
                let flex_of = |i: usize| match &items[i] {
                    FlexItem::Node(n) => (self.styles[*n].flex.grow, self.styles[*n].flex.shrink),
                    FlexItem::Text(_) => (0.0, 1.0),
                };
                if free > 0.0 {
                    let total: f32 = line.iter().map(|&i| flex_of(i).0).sum();
                    if total > 0.0 {
                        for (k, &i) in line.iter().enumerate() {
                            sizes[k] += free * flex_of(i).0 / total;
                        }
                    }
                } else if free < 0.0 {
                    // Shrink, but never below an item's minimum: its min-width, or by default
                    // its narrowest content (unless it clips its overflow). Items that hit
                    // their minimum stop, and the rest share what's still left.
                    let mins: Vec<f32> = line
                        .iter()
                        .map(|&i| match &items[i] {
                            FlexItem::Node(n) => {
                                let st = &self.styles[*n];
                                match st.min_width {
                                    Len::Auto if st.clip || st.width != Len::Auto => 0.0,
                                    Len::Auto => self.content_widths(*n).0,
                                    l => l.resolve(w),
                                }
                            }
                            FlexItem::Text(nodes) => {
                                let nodes = nodes.clone();
                                self.detach_text(&nodes, 0.0, Align::Left).w
                            }
                        })
                        .collect();
                    let mut frozen = vec![false; line.len()];
                    for _ in 0..line.len() + 1 {
                        let left = w - gaps - sizes.iter().sum::<f32>();
                        if left >= -0.01 {
                            break;
                        }
                        let total: f32 = line.iter().enumerate().filter(|(k, _)| !frozen[*k]).map(|(_, &i)| flex_of(i).1 * bases[i]).sum();
                        if total <= 0.0 {
                            break;
                        }
                        let mut any = false;
                        for (k, &i) in line.iter().enumerate() {
                            if frozen[k] {
                                continue;
                            }
                            let want = sizes[k] + left * flex_of(i).1 * bases[i] / total;
                            if want < mins[k].min(bases[i]) {
                                sizes[k] = mins[k].min(bases[i]);
                                frozen[k] = true;
                                any = true;
                            }
                        }
                        if !any {
                            for (k, &i) in line.iter().enumerate() {
                                if !frozen[k] {
                                    sizes[k] = (sizes[k] + left * flex_of(i).1 * bases[i] / total).max(0.0);
                                }
                            }
                            break;
                        }
                    }
                }
            }
            // Lay each item out at its final width.
            let mut boxes: Vec<Detached> = Vec::new();
            for (k, &i) in line.iter().enumerate() {
                let d = match &items[i] {
                    FlexItem::Node(n) => self.layout_at(*n, sizes[k], None),
                    FlexItem::Text(nodes) => self.detach_text(nodes, sizes[k], Align::Left),
                };
                boxes.push(d);
            }
            let mut cross = boxes.iter().map(|d| d.h).fold(0.0f32, f32::max);
            if single_line {
                if let Some(dh) = definite_h {
                    cross = cross.max(dh);
                }
            }
            // Stretch items with no height of their own to the line's height.
            for (k, &i) in line.iter().enumerate() {
                if let FlexItem::Node(n) = &items[i] {
                    let stretch = align_of(self, &items[i]) == AlignItems::Stretch;
                    if stretch && self.definite_len(self.styles[*n].height).is_none() && boxes[k].h < cross - 0.5 {
                        boxes[k] = self.layout_at(*n, sizes[k], Some(cross));
                    }
                }
            }
            let gaps = col_gap * (line.len().saturating_sub(1)) as f32;
            let free = (w - sizes.iter().sum::<f32>() - gaps).max(0.0);
            // Auto margins on the main axis take the free space first (margin-left: auto
            // pushes an item to the end, as in navigation bars); justify-content gets the rest.
            let autos: Vec<(bool, bool)> = line
                .iter()
                .map(|&i| match &items[i] {
                    FlexItem::Node(n) => (self.styles[*n].margin[3] == Len::Auto, self.styles[*n].margin[1] == Len::Auto),
                    FlexItem::Text(_) => (false, false),
                })
                .collect();
            let n_auto = autos.iter().map(|&(l, r)| l as usize + r as usize).sum::<usize>();
            let share = if n_auto > 0 { free / n_auto as f32 } else { 0.0 };
            let (mut pos, between) = if n_auto > 0 { (0.0, 0.0) } else { justify_offsets(fl.justify, free, line.len()) };
            let line_baseline = boxes.iter().map(|d| d.baseline).fold(0.0f32, f32::max);
            for (k, d) in boxes.into_iter().enumerate() {
                let i = line[k];
                if autos[k].0 {
                    pos += share;
                }
                let dy = match align_of(self, &items[i]) {
                    AlignItems::Center => (cross - d.h) / 2.0,
                    AlignItems::End => cross - d.h,
                    AlignItems::Baseline => line_baseline - d.baseline,
                    _ => 0.0,
                };
                self.place_detached(d, x + pos, cy + dy);
                pos += sizes[k] + col_gap + between;
                if autos[k].1 {
                    pos += share;
                }
            }
            cy += cross;
            if li + 1 < lines.len() {
                cy += row_gap;
            }
        }
        (cy - y).max(definite_h.unwrap_or(0.0))
    }

    fn role(&self, id: NodeId, role: TableRole, tags: &[&str]) -> bool {
        matches!(self.dom.nodes[id].data, NodeData::Element { .. })
            && self.styles[id].display != Display::None
            && (self.styles[id].table_role == role || tags.contains(&self.dom.tag(id).unwrap_or("")))
    }

    /// Find a table's rows and cells, place cells on the grid (colspan/rowspan),
    /// and measure each column's narrowest and widest content.
    fn table_grid(&mut self, table: NodeId) -> TableGrid {
        let st = &self.styles[table];
        let spacing = if st.border_collapse { 0.0 } else { st.border_spacing };
        let mut captions = Vec::new();
        let mut rows = Vec::new();
        for c in self.dom.nodes[table].children.clone() {
            if self.role(c, TableRole::Caption, &["caption"]) {
                captions.push(c);
            } else if self.role(c, TableRole::RowGroup, &["thead", "tbody", "tfoot"]) {
                for r in self.dom.nodes[c].children.clone() {
                    if self.role(r, TableRole::Row, &["tr"]) {
                        rows.push(r);
                    }
                }
            } else if self.role(c, TableRole::Row, &["tr"]) {
                rows.push(c);
            }
        }
        let nrows = rows.len();
        let mut taken: Vec<Vec<bool>> = vec![Vec::new(); nrows];
        let mut cells = Vec::new();
        let mut ncols = 0;
        for (r, &row) in rows.iter().enumerate() {
            let mut col = 0;
            for c in self.dom.nodes[row].children.clone() {
                if !self.role(c, TableRole::Cell, &["td", "th"]) {
                    continue;
                }
                let num = |a: &str| self.dom.attr(c, a).and_then(|v| v.trim().parse::<usize>().ok());
                let colspan = num("colspan").unwrap_or(1).clamp(1, 1000);
                let rowspan = match num("rowspan") {
                    Some(0) => nrows - r,
                    Some(n) => n.min(nrows - r).max(1),
                    None => 1,
                };
                while taken[r].get(col).copied().unwrap_or(false) {
                    col += 1;
                }
                for rr in r..r + rowspan {
                    if taken[rr].len() < col + colspan {
                        taken[rr].resize(col + colspan, false);
                    }
                    for cc in col..col + colspan {
                        taken[rr][cc] = true;
                    }
                }
                cells.push(TableCell { node: c, row: r, col, rowspan, colspan });
                col += colspan;
                ncols = ncols.max(col);
            }
        }
        let mut min = vec![0.0f32; ncols];
        let mut max = vec![0.0f32; ncols];
        // Single-column cells first, then spread wider cells over their columns.
        let mut spans = Vec::new();
        for cell in &cells {
            let (mn, mx) = self.cell_widths(cell.node);
            if cell.colspan == 1 {
                min[cell.col] = min[cell.col].max(mn);
                max[cell.col] = max[cell.col].max(mx);
            } else {
                spans.push((cell.col, cell.colspan, mn, mx));
            }
        }
        for (col, span, mn, mx) in spans {
            let gaps = spacing * (span as f32 - 1.0);
            let have_min: f32 = min[col..col + span].iter().sum::<f32>() + gaps;
            let have_max: f32 = max[col..col + span].iter().sum::<f32>() + gaps;
            for c in col..col + span {
                if mn > have_min {
                    min[c] += (mn - have_min) / span as f32;
                }
                if mx > have_max {
                    max[c] += (mx - have_max) / span as f32;
                }
            }
        }
        for c in 0..ncols {
            max[c] = max[c].max(min[c]);
        }
        TableGrid { captions, rows, cells, ncols, min, max, spacing }
    }

    /// A cell's narrowest (longest word) and widest (no wrapping) outer width.
    fn cell_widths(&mut self, cell: NodeId) -> (f32, f32) {
        let st = &self.styles[cell];
        if let Len::Px(px) = st.width {
            let edges = if st.border_box { 0.0 } else { st.padding[1].resolve(0.0) + st.padding[3].resolve(0.0) + st.border[1] + st.border[3] };
            let w = px + edges;
            let (mn, _) = self.content_widths(cell);
            return (mn.max(w), mn.max(w));
        }
        self.content_widths(cell)
    }

    fn content_widths(&mut self, id: NodeId) -> (f32, f32) {
        if self.control_kind(id).is_some() && self.styles[id].width == Len::Auto {
            let w = self.control_outer(id, 0.0);
            return (w, w);
        }
        let st = &self.styles[id];
        let tail = st.padding[1].resolve(0.0) + st.border[1] + st.margin[1].resolve(0.0);
        let was = self.measuring;
        self.measuring = true;
        let narrow = self.detach(|cx| (0.0, cx.block(id, 0.0, 0.0, 0.0, 0.0).0));
        let wide = self.detach(|cx| (100_000.0, cx.block(id, 0.0, 0.0, 100_000.0, 0.0).0));
        self.measuring = was;
        let mn = self.content_right(&narrow.items, true).ceil() + tail;
        let mx = self.content_right(&wide.items, true).ceil() + tail;
        (mn, mx.max(mn))
    }

    /// Lay out a table's captions, rows and cells inside a content box `w` wide.
    fn table_children(&mut self, g: &TableGrid, x: f32, y: f32, w: f32, stretch: bool) -> f32 {
        let mut cy = y;
        for &cap in &g.captions {
            let d = self.layout_at(cap, w, None);
            let h = d.h;
            self.place_detached(d, x, cy);
            cy += h;
        }
        let widths = g.widths(w, stretch);
        let s = g.spacing;
        let mut col_x = Vec::with_capacity(g.ncols);
        let mut cx = x + s;
        for cw in &widths {
            col_x.push(cx);
            cx += cw + s;
        }
        let span_w = |c: &TableCell| widths[c.col..c.col + c.colspan].iter().sum::<f32>() + s * (c.colspan as f32 - 1.0);
        // Natural heights decide row heights.
        let mut row_h: Vec<f32> = g.rows.iter().map(|&r| if let Len::Px(h) = self.styles[r].height { h } else { 0.0 }).collect();
        let mut natural = Vec::with_capacity(g.cells.len());
        for c in &g.cells {
            let h = self.layout_at(c.node, span_w(c), None).h;
            natural.push(h);
            if c.rowspan == 1 {
                row_h[c.row] = row_h[c.row].max(h);
            }
        }
        for (i, c) in g.cells.iter().enumerate() {
            if c.rowspan > 1 {
                let have: f32 = row_h[c.row..c.row + c.rowspan].iter().sum::<f32>() + s * (c.rowspan as f32 - 1.0);
                if natural[i] > have {
                    row_h[c.row + c.rowspan - 1] += natural[i] - have;
                }
            }
        }
        let mut row_y = Vec::with_capacity(g.rows.len());
        let mut ry = cy + s;
        for h in &row_h {
            row_y.push(ry);
            ry += h + s;
        }
        // Row backgrounds sit under their cells.
        for (r, &row) in g.rows.iter().enumerate() {
            let st = &self.styles[row];
            if st.bg.visible() && !st.hidden {
                self.items.push(Item::Box { x: x + s, y: row_y[r], w: (w - 2.0 * s).max(0.0), h: row_h[r], radius: 0.0, bg: Box::new(st.bg.clone()) });
            }
        }
        for (i, c) in g.cells.iter().enumerate() {
            let target = row_h[c.row..c.row + c.rowspan].iter().sum::<f32>() + s * (c.rowspan as f32 - 1.0);
            let free = (target - natural[i]).max(0.0);
            let shift = match self.styles[c.node].valign {
                VAlign::Middle => free / 2.0,
                VAlign::Bottom => free,
                _ => 0.0,
            };
            if shift > 0.0 {
                self.pad_top.insert(c.node, shift);
            }
            let d = self.layout_at(c.node, span_w(c), Some(target));
            self.pad_top.remove(&c.node);
            self.place_detached(d, col_x[c.col], row_y[c.row]);
        }
        let end = if g.rows.is_empty() { cy } else { ry };
        end - y
    }

    /// The kind of form control `id` is. <button> lays out its children like any box, so it's not one here.
    fn control_kind(&self, id: NodeId) -> Option<Control> {
        if self.dom.tag(id) == Some("button") {
            return None;
        }
        forms::control(self.dom, id)
    }

    fn input_label(&self, id: NodeId, c: Control) -> String {
        match c {
            Control::Button { submit, reset } => self.dom.attr(id, "value").map(String::from).unwrap_or_else(|| {
                if self.dom.attr(id, "type").is_some_and(|t| t.eq_ignore_ascii_case("image")) {
                    self.dom.attr(id, "alt").unwrap_or("Submit").to_string()
                } else if submit {
                    "Submit".into()
                } else if reset {
                    "Reset".into()
                } else {
                    String::new()
                }
            }),
            _ => String::new(),
        }
    }

    /// Natural content width of a form control.
    fn control_width(&self, id: NodeId) -> f32 {
        let (font, size) = self.font_of(id);
        let ch = self.fonts.width(font, "0", size).max(size * 0.5);
        let attr_n = |name: &str, d: f32| self.dom.attr(id, name).and_then(|v| v.trim().parse::<f32>().ok()).filter(|&n| n > 0.0).unwrap_or(d);
        match self.control_kind(id) {
            Some(Control::Text { .. }) => (attr_n("size", 20.0) * ch).round(),
            Some(Control::TextArea) => (attr_n("cols", 20.0) * self.fonts.width(Fonts::pick(false, false, true), "0", size)).round(),
            Some(Control::Checkbox | Control::Radio) => 13.0,
            Some(c @ Control::Button { .. }) => self.fonts.width(font, &self.input_label(id, c), size).ceil(),
            Some(Control::Select) => {
                let widest = forms::options(self.dom, id).into_iter().map(|o| self.fonts.width(font, &forms::option_label(self.dom, o), size)).fold(0.0, f32::max);
                (widest + size * 1.4).ceil()
            }
            Some(Control::Other) => match self.dom.attr(id, "type").unwrap_or("").to_ascii_lowercase().as_str() {
                "color" => 40.0,
                "range" => 130.0,
                _ => (self.fonts.width(font, "Browse… No file selected.", size) + 18.0).ceil(),
            },
            None => 0.0,
        }
    }

    /// Width of a control's whole box: content plus padding, border and margin.
    fn control_outer(&self, id: NodeId, avail: f32) -> f32 {
        let st = &self.styles[id];
        let p = |i: usize| st.padding[i].resolve(avail);
        let m = |i: usize| st.margin[i].resolve(avail);
        let edges = p(1) + p(3) + st.border[1] + st.border[3];
        self.control_width(id) + edges + m(1) + m(3)
    }

    /// Draw what's inside a form control (value, placeholder, check mark, caret) in the
    /// content box at (x, y), `w` wide. Returns the content height.
    fn control_content(&mut self, id: NodeId, x: f32, y: f32, w: f32) -> f32 {
        let Some(c) = self.control_kind(id) else { return 0.0 };
        let st = self.styles[id].clone();
        let (font, size) = self.font_of(id);
        let lh = (size * st.line_height.max(1.15)).round();
        let ascent = self.fonts.ascent(font, size);
        let baseline = |top: f32| (top + (lh - size) / 2.0 + ascent).round();
        let focus = FOCUS.with(|f| f.get()).filter(|f| f.0 == id);
        // ::placeholder sets the placeholder's color; otherwise it's the text color, faded.
        let faint = match st.pseudo.as_ref().and_then(|p| p.placeholder.as_ref()) {
            Some(p) if p.color != st.color => p.color,
            _ => Color { a: (st.color.a as f32 * 0.5) as u8, ..st.color },
        };
        let accent = Color { r: 0x0b, g: 0x6b, b: 0xd3, a: 255 };
        let white = Color { r: 255, g: 255, b: 255, a: 255 };
        let selection = Color { r: 0xb4, g: 0xd5, b: 0xfe, a: 255 };
        let hidden = st.hidden;
        let mut items: Vec<Item> = Vec::new();
        let height = match c {
            Control::Text { password } => {
                let value = forms::value(self.dom, id);
                let n = value.chars().count();
                let (text, color) = if value.is_empty() {
                    (self.dom.attr(id, "placeholder").unwrap_or("").replace(['\n', '\r'], ""), faint)
                } else if password {
                    ("•".repeat(n), st.color)
                } else {
                    (value.clone(), st.color)
                };
                // Caret and selection (character indices into the value).
                let (caret, anchor) = EDIT.with(Cell::get).filter(|_| focus.is_some()).unwrap_or((n, n));
                let (caret, anchor) = (caret.min(n), anchor.min(n));
                let upto = |i: usize| if value.is_empty() { 0.0 } else { self.fonts.width(font, &text.chars().take(i).collect::<String>(), size) };
                let caret_x = upto(caret);
                // Scroll long text so the caret stays in view.
                let shift = if focus.is_some() { (caret_x + 2.0 - w).max(0.0) } else { 0.0 };
                items.push(Item::ClipPush { x, y, w, h: lh, radius: 0.0 });
                if focus.is_some() && caret != anchor {
                    let (a, b) = (upto(caret.min(anchor)), upto(caret.max(anchor)));
                    items.push(Item::Rect { x: x - shift + a, y: baseline(y) - ascent, w: b - a, h: size * 1.2, color: selection });
                }
                if !text.is_empty() {
                    items.push(Item::Text { x: x - shift, y: baseline(y), text, font, size, color });
                }
                if let Some((_, true)) = focus {
                    items.push(Item::Rect { x: (x - shift + caret_x).round(), y: baseline(y) - ascent, w: 1.0, h: size * 1.15, color: st.color });
                }
                items.push(Item::ClipPop);
                lh
            }
            Control::TextArea => {
                let mono = Fonts::pick(st.bold, st.italic, true);
                let rows = self.dom.attr(id, "rows").and_then(|r| r.trim().parse::<f32>().ok()).filter(|&r| r > 0.0).unwrap_or(2.0);
                let value = forms::value(self.dom, id);
                let (src, color) = if value.is_empty() {
                    (self.dom.attr(id, "placeholder").unwrap_or("").to_string(), faint)
                } else {
                    (value.clone(), st.color)
                };
                // Wrap each paragraph at word boundaries (long words break anywhere), remembering
                // where each line starts in the text so the caret can be placed.
                let mut lines: Vec<(String, usize)> = Vec::new();
                let mut offset = 0usize;
                for para in src.split('\n') {
                    let mut line = String::new();
                    let mut start = offset;
                    for word in para.split_inclusive(' ') {
                        let trial = format!("{line}{word}");
                        if self.fonts.width(mono, trial.trim_end(), size) <= w || line.is_empty() {
                            line = trial;
                            while self.fonts.width(mono, line.trim_end(), size) > w && line.chars().count() > 1 {
                                let cut = line.char_indices().rev().find(|&(i, _)| self.fonts.width(mono, &line[..i], size) <= w).map(|(i, _)| i).unwrap_or(1).max(1);
                                let head = line[..cut].to_string();
                                let taken = head.chars().count();
                                lines.push((head, start));
                                start += taken;
                                line = line[cut..].to_string();
                            }
                        } else {
                            let taken = line.chars().count();
                            lines.push((std::mem::take(&mut line), start));
                            start += taken;
                            line = word.to_string();
                        }
                    }
                    lines.push((line, start));
                    offset += para.chars().count() + 1;
                }
                let n = value.chars().count();
                let (caret, anchor) = EDIT.with(Cell::get).filter(|_| focus.is_some()).unwrap_or((n, n));
                let (caret, anchor) = (caret.min(n), anchor.min(n));
                let caret_line = if value.is_empty() { 0 } else { lines.iter().rposition(|(_, st0)| *st0 <= caret).unwrap_or(0) };
                let h = (rows * lh).max(lh);
                let visible = (h / lh).floor().max(1.0) as usize;
                // Keep the caret's line in view.
                let first = if focus.is_some() { caret_line.saturating_sub(visible - 1) } else { 0 };
                items.push(Item::ClipPush { x, y, w, h, radius: 0.0 });
                let (sa, sb) = (caret.min(anchor), caret.max(anchor));
                let mut ly = y;
                for (l, start) in lines.iter().skip(first) {
                    let len = l.chars().count();
                    if focus.is_some() && sa != sb && !value.is_empty() && *start + len >= sa && *start <= sb {
                        let from = sa.max(*start) - start;
                        let to = (sb.min(start + len)) - start;
                        let px = |i: usize| self.fonts.width(mono, &l.chars().take(i).collect::<String>(), size);
                        let extra = if sb > start + len { size * 0.4 } else { 0.0 };
                        items.push(Item::Rect { x: x + px(from), y: baseline(ly) - ascent, w: px(to) - px(from) + extra, h: size * 1.2, color: selection });
                    }
                    if !l.is_empty() {
                        items.push(Item::Text { x, y: baseline(ly), text: l.trim_end_matches(' ').to_string(), font: mono, size, color });
                    }
                    ly += lh;
                }
                if let Some((_, true)) = focus {
                    let (line, start) = if value.is_empty() { ("", 0) } else { lines.get(caret_line).map(|(l, s0)| (l.as_str(), *s0)).unwrap_or(("", 0)) };
                    let col = caret.saturating_sub(start);
                    let cy = y + (caret_line.saturating_sub(first)) as f32 * lh;
                    let cx = x + self.fonts.width(mono, &line.chars().take(col).collect::<String>(), size);
                    items.push(Item::Rect { x: cx.round(), y: baseline(cy) - ascent, w: 1.0, h: size * 1.15, color: st.color });
                }
                items.push(Item::ClipPop);
                h
            }
            Control::Checkbox | Control::Radio => {
                let checked = self.dom.attr(id, "checked").is_some();
                let disabled = forms::is_disabled(self.dom, id);
                let fill = if disabled { Color { r: 150, g: 150, b: 160, a: 255 } } else { accent };
                let side = 13.0f32.min(w);
                if checked && c == Control::Checkbox {
                    let bg = Background { color: fill, ..Default::default() };
                    items.push(Item::Box { x, y, w: side, h: side, radius: 2.0, bg: Box::new(bg) });
                    // A check mark drawn as two strokes of small squares.
                    let pts = [(2.5, 6.5), (3.5, 7.5), (4.5, 8.5), (5.5, 9.5), (6.5, 8.5), (7.5, 7.5), (8.5, 6.5), (9.5, 5.5), (10.5, 4.5), (11.0, 3.5)];
                    for (px, py) in pts {
                        items.push(Item::Rect { x: x + px - 1.0, y: y + py - 1.0, w: 2.0, h: 2.0, color: white });
                    }
                } else if checked {
                    let bg = Background { color: fill, ..Default::default() };
                    items.push(Item::Box { x: x + 3.0, y: y + 3.0, w: side - 6.0, h: side - 6.0, radius: side, bg: Box::new(bg) });
                }
                side
            }
            Control::Button { .. } => {
                let label = self.input_label(id, c);
                let tw = self.fonts.width(font, &label, size);
                items.push(Item::Text { x: (x + (w - tw) / 2.0).round(), y: baseline(y), text: label, font, size, color: st.color });
                lh
            }
            Control::Select => {
                let label = forms::selected_option(self.dom, id).map(|o| forms::option_label(self.dom, o)).unwrap_or_default();
                let arrow = size * 1.2;
                items.push(Item::ClipPush { x, y, w: (w - arrow).max(0.0), h: lh, radius: 0.0 });
                items.push(Item::Text { x, y: baseline(y), text: label, font, size, color: st.color });
                items.push(Item::ClipPop);
                // A small downward triangle at the right (unless the page draws its own).
                let (ax, ay) = (x + w - arrow * 0.75, (y + lh / 2.0 - 2.0).round());
                for row in (0..4).filter(|_| !st.appearance_none) {
                    let r = row as f32;
                    items.push(Item::Rect { x: (ax + r).round(), y: ay + r, w: 8.0 - 2.0 * r, h: 1.0, color: st.color });
                }
                lh
            }
            Control::Other => {
                match self.dom.attr(id, "type").unwrap_or("").to_ascii_lowercase().as_str() {
                    "color" => {
                        let v = self.dom.attr(id, "value").unwrap_or("#000000");
                        let color = crate::style::parse_color(v).unwrap_or(Color { r: 0, g: 0, b: 0, a: 255 });
                        items.push(Item::Rect { x: x + 2.0, y: y + 2.0, w: w - 4.0, h: lh - 4.0, color });
                    }
                    "range" => {
                        let v: f32 = self.dom.attr(id, "value").and_then(|v| v.parse().ok()).unwrap_or(50.0);
                        let min: f32 = self.dom.attr(id, "min").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                        let max: f32 = self.dom.attr(id, "max").and_then(|v| v.parse().ok()).unwrap_or(100.0);
                        let t = if max > min { ((v - min) / (max - min)).clamp(0.0, 1.0) } else { 0.5 };
                        let mid = (y + lh / 2.0).round();
                        items.push(Item::Rect { x, y: mid - 2.0, w, h: 4.0, color: Color { r: 200, g: 200, b: 210, a: 255 } });
                        items.push(Item::Rect { x, y: mid - 2.0, w: w * t, h: 4.0, color: accent });
                        let bg = Background { color: accent, ..Default::default() };
                        items.push(Item::Box { x: x + (w - 14.0) * t, y: mid - 7.0, w: 14.0, h: 14.0, radius: 7.0, bg: Box::new(bg) });
                    }
                    _ => {
                        let names: Vec<String> = self.dom.form_state.files.get(&id).map(|f| f.iter().map(|f| f.name.clone()).collect()).unwrap_or_default();
                        let button = "Browse…";
                        let bw = self.fonts.width(font, button, size) + 12.0;
                        let edge = Color { r: 150, g: 150, b: 160, a: 255 };
                        items.push(Item::Rect { x, y: y - 1.0, w: bw, h: lh + 2.0, color: edge });
                        items.push(Item::Rect { x: x + 1.0, y, w: bw - 2.0, h: lh, color: Color { r: 239, g: 239, b: 242, a: 255 } });
                        items.push(Item::Text { x: x + 6.0, y: baseline(y), text: button.into(), font, size, color: st.color });
                        items.push(Item::ClipPush { x: x + bw + 6.0, y, w: (w - bw - 6.0).max(0.0), h: lh, radius: 0.0 });
                        items.push(Item::Text { x: x + bw + 6.0, y: baseline(y), text: crate::filepicker::label(&names), font, size, color: st.color });
                        items.push(Item::ClipPop);
                    }
                }
                lh
            }
        };
        // The whole box is clickable, and a focused control gets a ring.
        let [pt, pr, pb, pl] = st.padding.map(|p| p.resolve(w));
        let [bt, br, bb, bl] = st.border;
        let outer = Rect { x: x - pl - bl, y: y - pt - bt, w: w + pl + pr + bl + br, h: height + pt + pb + bt + bb };
        self.hits.push((outer, id));
        if focus.is_some() {
            let radius = st.radius.resolve(outer.w.min(outer.h)) + 2.0;
            items.push(Item::Ring { x: outer.x - 2.0, y: outer.y - 2.0, w: outer.w + 4.0, h: outer.h + 4.0, radius, width: 2.0, color: accent });
        }
        if !hidden {
            self.items.extend(items);
        }
        height
    }

    /// CSS grid: size the tracks, place every item (explicitly, by area name or
    /// automatically), then lay items out in their cells. Returns the content height.
    fn grid_children(&mut self, id: NodeId, x: f32, y: f32, w: f32, definite_h: Option<f32>) -> f32 {
        let tpl = self.styles[id].grid.clone().unwrap_or_default();
        let fl = self.styles[id].flex;
        let fs = self.styles[id].font_size;
        let col_gap = fl.column_gap.resolve(w);
        let row_gap = fl.row_gap.resolve(definite_h.unwrap_or(w));

        // Items: child elements, plus runs of loose text.
        let mut items: Vec<FlexItem> = Vec::new();
        let mut text: Vec<NodeId> = Vec::new();
        for c in self.dom.nodes[id].children.clone() {
            match &self.dom.nodes[c].data {
                NodeData::Text(t) => {
                    if !t.trim().is_empty() || !text.is_empty() {
                        text.push(c);
                    }
                }
                NodeData::Element { .. } => {
                    if self.styles[c].display == Display::None {
                        continue;
                    }
                    // Any element box, even an out-of-flow one, ends a run of loose text.
                    if !text.is_empty() {
                        items.push(FlexItem::Text(std::mem::take(&mut text)));
                    }
                    if self.is_out_of_flow(c) {
                        self.defer_absolute(c, x, y);
                        continue;
                    }
                    items.push(FlexItem::Node(c));
                }
                _ => {}
            }
        }
        if text.iter().any(|&t| matches!(&self.dom.nodes[t].data, NodeData::Text(s) if !s.trim().is_empty())) {
            items.push(FlexItem::Text(text));
        }
        items.sort_by_key(|it| match it {
            FlexItem::Node(n) => self.styles[*n].flex.order,
            FlexItem::Text(_) => 0,
        });

        let mut cols = grid::parse_tracks(&tpl.columns, w, col_gap, fs);
        let mut rows = grid::parse_tracks(&tpl.rows, definite_h.unwrap_or(0.0), row_gap, fs);
        let auto_col = grid::parse_tracks(&tpl.auto_columns, w, col_gap, fs).first().copied().unwrap_or(grid::Track::AUTO);
        let auto_row = grid::parse_tracks(&tpl.auto_rows, 0.0, row_gap, fs).first().copied().unwrap_or(grid::Track::AUTO);
        let areas = grid::parse_areas(&tpl.areas);
        if cols.is_empty() {
            cols.push(auto_col);
        }
        let n_cols_explicit = cols.len();
        let n_rows_explicit = rows.len().max(areas.rows);

        // Placement: definite positions first, then auto-placement row by row.
        struct Placed {
            item: usize,
            col: usize,
            row: usize,
            cspan: usize,
            rspan: usize,
        }
        let mut placed: Vec<Placed> = Vec::new();
        let mut taken: Vec<Vec<bool>> = Vec::new();
        let mark = |taken: &mut Vec<Vec<bool>>, r: usize, c: usize, rs: usize, cs: usize| {
            while taken.len() < r + rs {
                taken.push(Vec::new());
            }
            for row in taken.iter_mut().skip(r).take(rs) {
                if row.len() < c + cs {
                    row.resize(c + cs, false);
                }
                for cell in row.iter_mut().skip(c).take(cs) {
                    *cell = true;
                }
            }
        };
        let free = |taken: &Vec<Vec<bool>>, r: usize, c: usize, rs: usize, cs: usize| {
            (r..r + rs).all(|rr| (c..c + cs).all(|cc| !taken.get(rr).and_then(|row| row.get(cc)).copied().unwrap_or(false)))
        };
        let mut auto_items = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let place = match it {
                FlexItem::Node(n) => self.styles[*n].grid_place.clone(),
                FlexItem::Text(_) => None,
            };
            let (c, r) = match &place {
                Some(p) => grid::resolve_place(p, &areas, n_cols_explicit, n_rows_explicit),
                None => ((None, 1), (None, 1)),
            };
            match (c, r) {
                ((Some(c0), cs), (Some(r0), rs)) => {
                    mark(&mut taken, r0, c0, rs, cs);
                    placed.push(Placed { item: i, col: c0, row: r0, cspan: cs, rspan: rs });
                }
                _ => auto_items.push((i, c, r)),
            }
        }
        let mut cursor = (0usize, 0usize); // (row, col)
        for (i, (c0, cs), (r0, rs)) in auto_items {
            let ncols = cols.len().max(cs);
            let (mut r, mut c) = match (r0, c0) {
                (Some(r), _) => (r, c0.unwrap_or(0)),
                (None, Some(c)) => (cursor.0, c),
                _ => cursor,
            };
            for _ in 0..10_000 {
                if c + cs > ncols {
                    c = 0;
                    r += 1;
                    continue;
                }
                if free(&taken, r, c, rs, cs) {
                    break;
                }
                if c0.is_some() {
                    r += 1;
                } else {
                    c += 1;
                }
            }
            mark(&mut taken, r, c, rs, cs);
            placed.push(Placed { item: i, col: c, row: r, cspan: cs, rspan: rs });
            if r0.is_none() && c0.is_none() {
                cursor = (r, c + cs);
            }
        }
        // Implicit tracks for anything placed beyond the template.
        let need_cols = placed.iter().map(|p| p.col + p.cspan).max().unwrap_or(0);
        while cols.len() < need_cols {
            cols.push(auto_col);
        }
        let need_rows = placed.iter().map(|p| p.row + p.rspan).max().unwrap_or(0).max(n_rows_explicit);
        while rows.len() < need_rows {
            rows.push(auto_row);
        }

        // Column widths: content sizes for auto tracks, then fr shares of what's left.
        let mut content: Vec<(f32, f32)> = vec![(0.0, 0.0); cols.len()];
        for p in placed.iter().filter(|p| p.cspan == 1) {
            if cols[p.col].needs_content() {
                let (mn, mx) = match &items[p.item] {
                    FlexItem::Node(n) => self.content_widths(*n),
                    FlexItem::Text(nodes) => {
                        let nodes = nodes.clone();
                        let narrow = self.detach_text(&nodes, 0.0, Align::Left).w;
                        let wide = self.detach_text(&nodes, 100_000.0, Align::Left).w;
                        (narrow, wide)
                    }
                };
                content[p.col].0 = content[p.col].0.max(mn);
                content[p.col].1 = content[p.col].1.max(mx);
            }
        }
        let widths = grid::size_tracks(&cols, &content, w, col_gap, true);
        let mut col_x = Vec::with_capacity(widths.len());
        let mut cx = x;
        for cw in &widths {
            col_x.push(cx);
            cx += cw + col_gap;
        }
        let span_w = |p: &Placed| widths[p.col..p.col + p.cspan].iter().sum::<f32>() + col_gap * (p.cspan as f32 - 1.0);

        // Row heights from the items' natural heights.
        let mut natural = vec![0.0f32; placed.len()];
        let mut row_content: Vec<(f32, f32)> = vec![(0.0, 0.0); rows.len()];
        for (k, p) in placed.iter().enumerate() {
            let h = match &items[p.item] {
                FlexItem::Node(n) => self.layout_at(*n, span_w(p), None).h,
                FlexItem::Text(nodes) => self.detach_text(&nodes.clone(), span_w(p), Align::Left).h,
            };
            natural[k] = h;
            if p.rspan == 1 {
                row_content[p.row].0 = row_content[p.row].0.max(h);
                row_content[p.row].1 = row_content[p.row].1.max(h);
            }
        }
        let mut heights = grid::size_tracks(&rows, &row_content, definite_h.unwrap_or(0.0), row_gap, definite_h.is_some());
        for (k, p) in placed.iter().enumerate() {
            if p.rspan > 1 {
                let have: f32 = heights[p.row..p.row + p.rspan].iter().sum::<f32>() + row_gap * (p.rspan as f32 - 1.0);
                if natural[k] > have {
                    heights[p.row + p.rspan - 1] += natural[k] - have;
                }
            }
        }
        let mut row_y = Vec::with_capacity(heights.len());
        let mut ry = y;
        for rh in &heights {
            row_y.push(ry);
            ry += rh + row_gap;
        }

        // Lay each item out in its area.
        for (k, p) in placed.iter().enumerate() {
            let area_h = heights[p.row..p.row + p.rspan].iter().sum::<f32>() + row_gap * (p.rspan as f32 - 1.0);
            let (iw, ix, iy) = (span_w(p), col_x[p.col], row_y[p.row]);
            match &items[p.item] {
                FlexItem::Node(n) => {
                    let align = self.styles[*n].flex.align_self.unwrap_or(fl.align_items);
                    let stretch = align == AlignItems::Stretch && self.definite_len(self.styles[*n].height).is_none();
                    let d = self.layout_at(*n, iw, stretch.then_some(area_h));
                    let dy = match align {
                        AlignItems::Center => (area_h - d.h) / 2.0,
                        AlignItems::End => area_h - d.h,
                        _ => 0.0,
                    };
                    self.place_detached(d, ix, iy + dy.max(0.0));
                }
                FlexItem::Text(nodes) => {
                    let d = self.detach_text(&nodes.clone(), iw, Align::Left);
                    self.place_detached(d, ix, iy);
                }
            }
            let _ = natural[k];
        }
        let total = if heights.is_empty() { 0.0 } else { ry - row_gap - y };
        total.max(definite_h.unwrap_or(0.0))
    }

    /// Lay out loose text inside a flex container as an anonymous box.
    fn detach_text(&mut self, nodes: &[NodeId], w: f32, align: Align) -> Detached {
        let nodes = nodes.to_vec();
        let mut d = self.detach(|cx| {
            let was = cx.measuring;
            cx.measuring = align == Align::Left && was;
            let h = cx.inline(&nodes, 0.0, 0.0, w, align);
            cx.measuring = was;
            (w, h)
        });
        if align == Align::Left {
            d.w = self.content_right(&d.items, false).ceil().min(w);
        }
        d
    }

    /// Lay out a node's children: block children stack vertically, runs of
    /// inline children become line boxes. Returns (height, last bottom margin).
    fn block_children(&mut self, id: NodeId, x: f32, y: f32, w: f32, align: Align) -> (f32, f32) {
        let mut cy = y;
        let mut prev_mb = 0.0;
        let mut run: Vec<NodeId> = Vec::new();
        let children = self.dom.nodes[id].children.clone();
        for c in children {
            if self.styles[c].display == Display::None {
                continue;
            }
            if self.is_out_of_flow(c) {
                self.defer_absolute(c, x, cy);
                continue;
            }
            if self.is_float(c) {
                if !run.is_empty() {
                    let h = self.inline(&run, x, cy, w, align);
                    if h > 0.0 {
                        prev_mb = 0.0;
                    }
                    cy += h;
                    run.clear();
                }
                self.place_float(c, x, cy, w);
                continue;
            }
            if self.is_block(c) {
                if !run.is_empty() {
                    let h = self.inline(&run, x, cy, w, align);
                    if h > 0.0 {
                        prev_mb = 0.0;
                    }
                    cy += h;
                    run.clear();
                }
                // clear: move below the floats on that side.
                let (cl, cr) = self.styles[c].clear;
                if cl || cr {
                    let below = self.float_bottom(cl, cr);
                    if below > cy {
                        cy = below;
                        prev_mb = 0.0;
                    }
                }
                // A box that starts its own float context sits beside floats instead of under them.
                let st = &self.styles[c];
                let (bx, bw) = if (st.clip || st.flow_root || st.flex.container) && self.has_floats() {
                    let (off, avail) = self.band(x, w, cy, 1.0);
                    (x + off, avail)
                } else {
                    (x, w)
                };
                self.flow_child = true;
                let (h, mb) = self.block(c, bx, cy, bw, prev_mb);
                cy += h;
                prev_mb = mb;
            } else {
                run.push(c);
            }
        }
        if !run.is_empty() {
            let h = self.inline(&run, x, cy, w, align);
            if h > 0.0 {
                prev_mb = 0.0;
            }
            cy += h;
        }
        (cy - y, prev_mb)
    }

    /// Multi-column: (number of columns, column width, gap) for a content width `w`.
    fn columns_of(&self, id: NodeId, w: f32) -> Option<(usize, f32, f32)> {
        let st = &self.styles[id];
        let mc = st.multicol?;
        let gap = if st.column_gap_set { st.flex.column_gap.resolve(w) } else { st.font_size };
        let n = match (mc.count, mc.width) {
            (None, None) => return None,
            (Some(c), None) => c as usize,
            (c, Some(cw)) => {
                let fit = (((w + gap) / (cw + gap)).floor() as usize).max(1);
                c.map_or(fit, |c| fit.min(c as usize))
            }
        }
        .clamp(1, 1000);
        let cw = ((w - gap * (n - 1) as f32) / n as f32).max(1.0);
        Some((n, cw, gap))
    }

    /// Lay out a multi-column container's content: all of it in one column of the column
    /// width, then cut that into balanced columns between lines (or between boxes with a
    /// background, which are kept whole) and set the pieces side by side.
    fn multicol(&mut self, id: NodeId, x: f32, y: f32, w: f32, align: Align) -> f32 {
        let Some((n, cw, gap)) = self.columns_of(id, w) else { return 0.0 };
        let d = self.detach(|cx| {
            let (h, mb) = cx.block_children(id, 0.0, 0.0, cw, align);
            (cw, h - mb)
        });
        // Vertical extents of everything that must not be cut.
        let mut spans: Vec<(f32, f32)> = Vec::new();
        for it in &d.items {
            match *it {
                Item::Rect { y, h, .. } | Item::Image { y, h, .. } | Item::Box { y, h, .. } | Item::Ring { y, h, .. } | Item::ClipPush { y, h, .. } => spans.push((y, y + h)),
                Item::Shadow { y, h, blur, .. } => spans.push((y - blur, y + h + blur)),
                Item::Text { y, size, .. } => spans.push((y - size, y + size * 0.3)),
                _ => {}
            }
        }
        // Line boxes (from the hit areas of their text) are what columns break between.
        for (r, _) in &d.hits {
            spans.push((r.y, r.y + r.h));
        }
        spans.retain(|(a, b)| b > a);
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Merge overlapping extents into groups; columns may start at any group's top.
        let mut groups: Vec<(f32, f32)> = Vec::new();
        for (a, b) in spans {
            match groups.last_mut() {
                Some(g) if a < g.1 - 0.5 => g.1 = g.1.max(b),
                _ => groups.push((a, b)),
            }
        }
        // Column starts for a given column height: fill greedily.
        let starts_for = |limit: f32| -> Vec<f32> {
            let mut starts = vec![0.0f32];
            for &(a, b) in &groups {
                let s = *starts.last().unwrap();
                if b - s > limit && a > s + 0.5 {
                    starts.push(a);
                }
            }
            starts
        };
        let total = d.h.max(groups.last().map_or(0.0, |g| g.1));
        let tallest = groups.iter().map(|(a, b)| b - a).fold(0.0f32, f32::max);
        // Balance: the shortest column height that fits everything in n columns.
        let (mut lo, mut hi) = ((total / n as f32).max(tallest), total.max(1.0));
        if starts_for(lo).len() <= n {
            hi = lo;
        } else {
            for _ in 0..24 {
                let mid = (lo + hi) / 2.0;
                if starts_for(mid).len() <= n {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
        }
        let mut starts = starts_for(hi);
        starts.truncate(n);
        let col_of = |y: f32| starts.iter().rposition(|&s| y >= s - 0.5).unwrap_or(0);
        let offset = |c: usize| (x + c as f32 * (cw + gap), y - starts[c]);
        // Move each item into its column. Pops follow their push.
        let mut stack: Vec<usize> = Vec::new();
        let mut last = 0usize;
        let mut height = 0.0f32;
        for mut it in d.items {
            let c = match &it {
                Item::Rect { y, .. }
                | Item::Image { y, .. }
                | Item::Box { y, .. }
                | Item::Ring { y, .. }
                | Item::Mark { y, .. }
                | Item::ScrollArea { y, .. }
                | Item::ClipPush { y, .. }
                | Item::TransformPush { y, .. }
                | Item::StickyPush { y, .. } => col_of(*y),
                Item::Shadow { y, blur, .. } => col_of(*y - *blur),
                Item::Text { y, size, .. } => col_of(*y - *size),
                Item::EffectPush(fx) => col_of(fx.area.1),
                Item::ClipPop | Item::TransformPop | Item::StickyPop | Item::FixedPop | Item::EffectPop => stack.pop().unwrap_or(last),
                Item::FixedPush => last,
            };
            if matches!(it, Item::ClipPush { .. } | Item::TransformPush { .. } | Item::StickyPush { .. } | Item::FixedPush | Item::EffectPush(_)) {
                stack.push(c);
            }
            last = c;
            let (dx, dy) = offset(c);
            if let Item::Rect { y, h, .. } | Item::Box { y, h, .. } | Item::Image { y, h, .. } = &it {
                height = height.max(y + h - starts[c]);
            } else if let Item::Text { y, size, .. } = &it {
                height = height.max(y + size * 0.3 - starts[c]);
            }
            shift_item(&mut it, dx, dy);
            self.items.push(it);
        }
        for (mut r, l) in d.links {
            let (dx, dy) = offset(col_of(r.y));
            r.x += dx;
            r.y += dy;
            self.links.push((r, l));
        }
        for (mut r, node) in d.hits {
            let (dx, dy) = offset(col_of(r.y));
            r.x += dx;
            r.y += dy;
            height = height.max(r.y + r.h - y);
            self.hits.push((r, node));
        }
        // The last column holds whatever is left, so measure columns from their starts.
        for (i, &s) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(total);
            height = height.max(end - s);
        }
        let height = height.min(total).max(0.0);
        // Rules between columns that have content.
        let st = &self.styles[id];
        if let Some(mc) = st.multicol {
            if mc.rule_width > 0.0 && !st.hidden {
                let color = mc.rule_color.unwrap_or(st.color);
                for c in 1..starts.len() {
                    let rx = x + c as f32 * (cw + gap) - gap / 2.0 - mc.rule_width / 2.0;
                    border_side(&mut self.items, if mc.rule_dashed { 1 } else { 0 }, rx, y, mc.rule_width, height, color);
                }
            }
        }
        height
    }

    /// Lay out an element as a block at (0, 0), away from the main display list.
    /// Auto width shrinks to fit the content (measured with a first, left-aligned pass).
    fn layout_detached(&mut self, id: NodeId, avail: f32) -> Detached {
        if let Some((Some(fw), fh)) = self.forced.get(&id).copied() {
            return self.layout_at(id, fw, fh);
        }
        let st = &self.styles[id];
        let mut width = avail;
        if st.width == Len::Auto && self.control_kind(id).is_some() {
            width = self.control_outer(id, avail);
        } else if st.width == Len::Auto {
            // Measure: lay out at the full width, left-aligned, and see how far content reaches.
            let was = self.measuring;
            self.measuring = true;
            let probe = self.detach(|cx| (avail, cx.block(id, 0.0, 0.0, avail, 0.0).0));
            self.measuring = was;
            let right = self.content_right(&probe.items, true);
            let st = &self.styles[id];
            let tail = st.padding[1].resolve(avail) + st.border[1] + st.margin[1].resolve(avail);
            width = (right.ceil() + tail).min(avail);
        }
        let mut d = self.detach(|cx| (width, cx.block(id, 0.0, 0.0, width, 0.0).0));
        let st = &self.styles[id];
        d.w = match st.width {
            // Shrink-to-fit never goes below the content's own minimum width: content that
            // can't wrap any narrower (a word, a table cell) makes the box that wide.
            Len::Auto => {
                let tail = st.padding[1].resolve(avail) + st.border[1] + st.margin[1].resolve(avail);
                width.max(self.content_right(&d.items, true).ceil() + tail)
            }
            l => {
                let ml = st.margin[3].resolve(avail);
                let mr = st.margin[1].resolve(avail);
                let edges = if st.border_box { 0.0 } else { st.padding[1].resolve(avail) + st.padding[3].resolve(avail) + st.border[1] + st.border[3] };
                l.resolve(avail) + edges + ml + mr
            }
        };
        d
    }

    /// The image key a replaced element paints: an `<img>`'s src, or the rendering of an
    /// `<iframe>`'s document.
    fn replaced_src(&self, id: NodeId) -> String {
        if self.dom.tag(id) == Some("iframe") {
            frame_key(self.dom, id)
        } else {
            // srcset / <picture> pick the picture (srcset.rs).
            crate::srcset::current_src(self.dom, id).unwrap_or_default()
        }
    }

    /// Size of an <img> (or iframe): CSS width/height, else the width/height attributes, else
    /// the image's own size, keeping its aspect ratio; then shrunk to fit `max-width`.
    /// An <img>'s own size in CSS px: its picture's pixel size divided by the srcset density.
    fn image_size(&self, id: NodeId) -> Option<(f32, f32)> {
        let (src, density) = crate::srcset::current(self.dom, id)?;
        self.images.size(&src).map(|(w, h)| (w / density.max(1e-3), h / density.max(1e-3)))
    }

    /// Draw an image filling the box (x, y, w, h) as `object-fit` and `object-position` say,
    /// clipped to the box when it spills over.
    fn push_image(&mut self, id: NodeId, x: f32, y: f32, w: f32, h: f32, src: String) {
        let st = &self.styles[id];
        let (fit, pos) = (st.object_fit, st.object_position);
        let natural = if self.dom.tag(id) == Some("img") { self.image_size(id) } else { None };
        let Some((iw, ih)) = natural.filter(|_| fit != crate::style::ObjectFit::Fill).filter(|(a, b)| *a > 0.0 && *b > 0.0) else {
            self.items.push(Item::Image { x, y, w, h, src, view: None });
            return;
        };
        use crate::style::ObjectFit as F;
        let contain = (w / iw).min(h / ih);
        let s = match fit {
            F::Contain => contain,
            F::Cover => (w / iw).max(h / ih),
            F::None => 1.0,
            F::ScaleDown => contain.min(1.0),
            F::Fill => 1.0,
        };
        let (dw, dh) = (iw * s, ih * s);
        let place = |l: Len, free: f32| match l {
            Len::Pct(p) => free * p / 100.0,
            Len::Mix(px, p) => px + free * p / 100.0,
            other => other.resolve(0.0),
        };
        let (ox, oy) = (place(pos.0, w - dw), place(pos.1, h - dh));
        // The item keeps the element's box (for sizing and hit tests); the picture inside it
        // is placed by `view` and cut off at the box.
        self.items.push(Item::Image { x, y, w, h, src, view: Some(Box::new([ox, oy, dw, dh])) });
    }

    fn replaced_size(&self, id: NodeId, avail: f32) -> Option<(f32, f32)> {
        let st = &self.styles[id];
        // An iframe is 300×150 unless sized; its content doesn't change that.
        let intrinsic = if self.dom.tag(id) == Some("iframe") { Some((300.0, 150.0)) } else { self.image_size(id) };
        let attr = |a: &str| self.dom.attr(id, a).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
        let w = match st.width {
            Len::Auto => attr("width"),
            l => Some(l.resolve(avail)),
        };
        let h = match st.height {
            Len::Auto => attr("height"),
            l => self.definite_len(l).or_else(|| attr("height")),
        };
        let (mut w, mut h) = match (w, h, intrinsic) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some((iw, ih))) => (w, w * ih / iw.max(1.0)),
            (None, Some(h), Some((iw, ih))) => (h * iw / ih.max(1.0), h),
            (None, None, Some((iw, ih))) => (iw, ih),
            _ => return None,
        };
        if st.max_width != Len::Auto {
            let max = st.max_width.resolve(avail);
            if w > max && w > 0.0 {
                h *= max / w;
                w = max;
            }
        }
        Some((w.max(0.0), h.max(0.0)))
    }

    fn list_marker(&mut self, id: NodeId, cx: f32, cy: f32) {
        let st = &self.styles[id];
        let text = match st.list_style {
            ListStyle::Decimal => {
                let parent = self.dom.nodes[id].parent.unwrap_or(0);
                let n = self.dom.nodes[parent]
                    .children
                    .iter()
                    .filter(|&&c| self.dom.tag(c) == Some("li"))
                    .position(|&c| c == id)
                    .unwrap_or(0)
                    + 1;
                format!("{n}.")
            }
            _ => "•".to_string(),
        };
        let line_h = st.font_size * st.line_height;
        // ::marker can change the marker's color, size and text.
        let marker = st.pseudo.as_ref().and_then(|p| p.marker.clone());
        let (text, size, color, bold) = match &marker {
            Some(m) => {
                let custom = m.content.as_deref().and_then(|c| crate::pseudo::content_text(c, self.dom, id, &Default::default(), &mut 0));
                (custom.unwrap_or(text), m.font_size, m.color, m.bold)
            }
            None => (text, st.font_size, st.color, false),
        };
        let font = Fonts::pick(bold, false, false);
        let baseline = cy + (line_h - size) / 2.0 + self.fonts.ascent(font, size);
        let tw = self.fonts.width(font, &text, size);
        self.items.push(Item::Text { x: cx - tw - size * 0.5, y: baseline, text, font, size, color });
    }

    // ------------------------------------------------------------ inline

    fn flatten(&self, id: NodeId, link: Option<&str>, avail: f32, out: &mut Vec<Tok>) {
        match &self.dom.nodes[id].data {
            NodeData::Text(t) => {
                if self.styles[id].pre {
                    for (i, line) in t.replace('\t', "    ").split('\n').enumerate() {
                        if i > 0 {
                            out.push(Tok::Break { node: id });
                        }
                        if !line.is_empty() {
                            out.push(Tok::Word { text: transform(line, self.styles[id].text_transform), node: id, link: link.map(String::from), glue: false, hyphen: false });
                        }
                    }
                    return;
                }
                if t.starts_with(|c: char| c.is_ascii_whitespace()) {
                    out.push(Tok::Space { node: id });
                }
                let mut first = true;
                let ts = &self.styles[id].text;
                let lang = if ts.hyphens == crate::text::Hyphens::Auto { self.lang_of(id) } else { None };
                for word in t.split_ascii_whitespace() {
                    if !first {
                        out.push(Tok::Space { node: id });
                    }
                    first = false;
                    // Pieces between the places a line may break (UAX #14, hyphenation).
                    let word = transform(word, self.styles[id].text_transform);
                    for (k, p) in crate::text::pieces(&word, ts, lang.as_deref()).into_iter().enumerate() {
                        out.push(Tok::Word { text: p.text, node: id, link: link.map(String::from), glue: k > 0, hyphen: p.hyphen });
                    }
                }
                if !first && t.ends_with(|c: char| c.is_ascii_whitespace()) {
                    out.push(Tok::Space { node: id });
                }
            }
            NodeData::Element { tag, .. } => {
                if self.styles[id].display == Display::None || self.is_out_of_flow(id) || self.is_float(id) {
                    return;
                }
                match tag.as_str() {
                    "br" => return out.push(Tok::Break { node: id }),
                    "img" | "iframe" if !(tag == "iframe" && self.styles[id].display == Display::InlineBlock) => {
                        match self.replaced_size(id, avail) {
                            Some((w, h)) => {
                                let src = Some(self.replaced_src(id));
                                out.push(Tok::Image { w, h, node: id, src, link: link.map(String::from) })
                            }
                            None => {
                                if let Some(alt) = self.dom.attr(id, "alt").filter(|a| !a.trim().is_empty()) {
                                    out.push(Tok::Word { text: format!("[{}]", alt.trim()), node: id, link: link.map(String::from), glue: false, hyphen: false });
                                }
                            }
                        }
                        return;
                    }
                    _ => {}
                }
                let href = if tag == "a" { self.dom.attr(id, "href") } else { None };
                let link = href.or(link);
                if self.styles[id].display == Display::InlineBlock {
                    return out.push(Tok::InlineBlock { node: id, link: link.map(String::from) });
                }
                let block = self.is_block(id);
                if block {
                    out.push(Tok::Break { node: id });
                }
                let boxed = if block { None } else { self.inline_box_edges(id, avail) };
                if let Some((l, _)) = boxed {
                    out.push(Tok::Open { node: id, w: l });
                }
                for &c in &self.dom.nodes[id].children {
                    self.flatten(c, link, avail, out);
                }
                if let Some((_, r)) = boxed {
                    out.push(Tok::Close { node: id, w: r });
                }
                if block {
                    out.push(Tok::Break { node: id });
                }
            }
            _ => {}
        }
    }

    /// For an inline element that draws a box: (left padding+border, right padding+border).
    fn inline_box_edges(&self, id: NodeId, avail: f32) -> Option<(f32, f32)> {
        let st = &self.styles[id];
        let pl = st.padding[3].resolve(avail);
        let pr = st.padding[1].resolve(avail);
        let ml = st.margin[3].resolve(avail);
        let mr = st.margin[1].resolve(avail);
        let visible = st.bg.visible() || st.border.iter().any(|&b| b > 0.0) || pl > 0.0 || pr > 0.0 || ml != 0.0 || mr != 0.0;
        visible.then(|| (pl + st.border[3] + st.margin[3].resolve(avail), pr + st.border[1] + st.margin[1].resolve(avail)))
    }

    /// Paint the background and borders of an inline box segment on one line.
    fn inline_box(&self, node: NodeId, x0: f32, x1: f32, baseline: f32, avail: f32, out: &mut Vec<Item>) {
        let st = &self.styles[node];
        if st.hidden {
            return;
        }
        let (font, size) = self.font_of(node);
        let ascent = self.fonts.ascent(font, size);
        let descent = size * 0.24;
        let pt = st.padding[0].resolve(avail);
        let pb = st.padding[2].resolve(avail);
        let [bt, br, bb, bl] = st.border;
        let ml = st.margin[3].resolve(avail);
        let mr = st.margin[1].resolve(avail);
        let (x0, x1) = (x0 + ml, x1 - mr);
        let top = baseline - ascent - pt - bt;
        let h = ascent + descent + pt + pb + bt + bb;
        let w = x1 - x0;
        let radius = st.radius.resolve(w.min(h)).min(w.min(h) / 2.0);
        if st.bg.visible() {
            out.push(Item::Box { x: x0, y: top, w, h, radius, bg: Box::new(st.bg.clone()) });
        }
        let [ct, cr, cbm, cl] = st.side_colors();
        if radius > 0.0 && bt == br && bt == bb && bt == bl && bt > 0.0 && ct == cr && ct == cbm && ct == cl {
            out.push(Item::Ring { x: x0, y: top, w, h, radius, width: bt, color: ct });
            return;
        }
        for (rx, ry, rw, rh, c) in [(x0, top, w, bt, ct), (x0, top + h - bb, w, bb, cbm), (x0, top, bl, h, cl), (x1 - br, top, br, h, cr)] {
            if rw > 0.0 && rh > 0.0 {
                out.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: c });
            }
        }
    }

    /// How vertical-align on `node` and its inline ancestors moves an item off the line's
    /// baseline. `own` is the item's (height above, depth below) its baseline.
    fn vpos(&self, node: NodeId, own: (f32, f32)) -> VPos {
        // A text node shares its parent's style; vertical-align belongs to the element.
        let mut n = match (&self.dom.nodes[node].data, self.dom.nodes[node].parent) {
            (NodeData::Text(_), Some(p)) => p,
            _ => node,
        };
        if n == Dom::ROOT || self.styles[n].display != Display::Inline && n != node {
            return VPos::Raise(0.0);
        }
        let mut metrics = own;
        let mut total = 0.0f32;
        loop {
            let st = &self.styles[n];
            let parent = self.dom.nodes[n].parent;
            let (pfont, psize) = parent.map_or((0, st.font_size), |p| self.font_of(p));
            let p_ascent = self.fonts.ascent(pfont, psize);
            let p_descent = psize * 0.24;
            let (a, b) = metrics;
            total += match st.valign {
                VAlign::Baseline => 0.0,
                VAlign::Sub => -psize * 0.2,
                VAlign::Super => psize * 0.34,
                VAlign::TextTop => p_ascent - a,
                VAlign::TextBottom => b - p_descent,
                // Middle of the box at the parent's baseline plus half its x-height.
                VAlign::Middle => psize * 0.25 - (a - b) / 2.0,
                VAlign::Shift(l) => l.resolve(st.font_size * st.line_height),
                VAlign::Top => return VPos::Top,
                VAlign::Bottom => return VPos::Bottom,
            };
            // Keep going up through inline ancestors (a <span> around an <img>, nested <sup>).
            match parent {
                Some(p) if self.styles[p].display == Display::Inline && p != Dom::ROOT => {
                    n = p;
                    let (_, a, b) = self.tok_size(&Tok::Space { node: p });
                    metrics = (a, b);
                }
                _ => break,
            }
        }
        VPos::Raise(total)
    }

    /// position: relative offsets of `node` and its inline ancestors, for content on a line.
    /// An inline-block applies its own offset when it is laid out, so it starts from its parent.
    fn inline_rel(&self, node: NodeId, avail: f32) -> (f32, f32) {
        let mut n = match &self.dom.nodes[node].data {
            NodeData::Text(_) => self.dom.nodes[node].parent,
            _ if self.styles[node].display == Display::InlineBlock => self.dom.nodes[node].parent,
            _ => Some(node),
        };
        let (mut dx, mut dy) = (0.0, 0.0);
        while let Some(id) = n {
            let st = &self.styles[id];
            if id == Dom::ROOT || !(st.display == Display::Inline || id == node) {
                break;
            }
            if st.position == Position::Relative {
                let off = |a: Len, b: Len| match (a, b) {
                    (Len::Auto, Len::Auto) => 0.0,
                    (Len::Auto, b) => -b.resolve(avail),
                    (a, _) => a.resolve(avail),
                };
                dx += off(st.inset[3], st.inset[1]);
                dy += off(st.inset[0], st.inset[2]);
            }
            n = self.dom.nodes[id].parent;
        }
        (dx, dy)
    }

    /// The language an element's text is in (`lang` on it or an ancestor).
    fn lang_of(&self, mut id: NodeId) -> Option<String> {
        loop {
            if let Some(l) = self.dom.attr(id, "lang").or_else(|| self.dom.attr(id, "xml:lang")) {
                return Some(l.to_string());
            }
            id = self.dom.nodes[id].parent?;
        }
    }

    /// A word's width with its letter-spacing (letter-spaced text is drawn one letter at a
    /// time, so it's measured that way too).
    fn word_width(&self, node: NodeId, text: &str) -> f32 {
        let (font, size) = self.font_of(node);
        let ls = self.styles[node].text.letter_spacing;
        if ls == 0.0 {
            return self.fonts.width(font, text, size);
        }
        crate::text::letters(text).iter().map(|l| self.fonts.width(font, l, size) + ls).sum()
    }

    fn font_of(&self, node: NodeId) -> (u8, f32) {
        let s = &self.styles[node];
        (self.fonts.resolve(s.family.as_deref(), s.bold, s.italic, s.mono), s.font_size)
    }

    fn tok_size(&self, t: &Tok) -> (f32, f32, f32) {
        // (line height, ascent above baseline incl. half-leading, descent part)
        match t {
            Tok::Word { node, .. } | Tok::Space { node } | Tok::Break { node } => {
                let s = &self.styles[*node];
                let (font, size) = self.font_of(*node);
                let lh = size * s.line_height;
                let above = (lh - size) / 2.0 + self.fonts.ascent(font, size);
                (lh, above, lh - above)
            }
            Tok::Image { h, .. } => (*h, *h, 0.0),
            Tok::Open { node, .. } | Tok::Close { node, .. } => self.tok_size(&Tok::Space { node: *node }),
            Tok::InlineBlock { .. } => (0.0, 0.0, 0.0), // sized from its laid-out box instead
        }
    }

    fn inline(&mut self, run: &[NodeId], x: f32, y: f32, w: f32, align: Align) -> f32 {
        let mut toks = Vec::new();
        for &id in run {
            self.flatten(id, None, w, &mut toks);
        }
        // overflow-wrap: break-word / anywhere: a word wider than the line is split to fit.
        if toks.iter().any(|t| matches!(t, Tok::Word { node, .. } if self.styles[*node].text.break_long_words)) {
            let mut out = Vec::with_capacity(toks.len());
            for t in toks {
                match t {
                    Tok::Word { text, node, link, glue, hyphen } if self.styles[node].text.break_long_words && !self.measuring && self.word_width(node, &text) > w => {
                        let parts = crate::text::split_to_fit(&text, w.max(1.0), &|s| self.word_width(node, s));
                        let n = parts.len();
                        for (k, part) in parts.into_iter().enumerate() {
                            out.push(Tok::Word { text: part, node, link: link.clone(), glue: glue || k > 0, hyphen: hyphen && k + 1 == n });
                        }
                    }
                    t => out.push(t),
                }
            }
            toks = out;
        }
        // Absolutely positioned and floated elements inside this text leave the flow.
        let mut stack: Vec<NodeId> = run.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if self.is_out_of_flow(n) {
                self.defer_absolute(n, x, y);
            } else if self.is_float(n) {
                self.place_float(n, x, y, w);
            } else if self.styles[n].display == Display::Inline {
                stack.extend(self.dom.nodes[n].children.iter().rev().copied());
            }
        }
        // Lay out inline-blocks first: their size decides where lines break.
        let mut blocks: HashMap<usize, Detached> = HashMap::new();
        for (i, t) in toks.iter().enumerate() {
            if let Tok::InlineBlock { node, .. } = t {
                let d = self.layout_detached(*node, w);
                blocks.insert(i, d);
            }
        }
        // Break into lines. Each line's width depends on floats beside it, so track
        // an estimate of where each line will sit vertically.
        let mut est_y = y;
        let mut lines: Vec<Line> = vec![self.new_line(x, w, y)];
        // text-indent moves the first line of a block in (only the block's first run).
        if let Some(&first) = run.first() {
            if let Some(p) = self.dom.nodes[first].parent {
                if self.dom.nodes[p].children.first() == Some(&first) {
                    let ind = self.styles[p].text_indent.resolve(w);
                    if ind != 0.0 {
                        let l = &mut lines[0];
                        l.x_off += ind;
                        l.max_w -= ind;
                    }
                }
            }
        }
        let mut pending_space = 0.0f32;
        let mut pending_opens: Vec<(NodeId, f32)> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            let line = lines.last_mut().unwrap();
            match t {
                Tok::Open { node, w: ow } => pending_opens.push((*node, *ow)),
                Tok::Close { node, w: cw } => {
                    if let Some(pos) = pending_opens.iter().position(|(n, _)| n == node) {
                        // Empty box (nothing placed inside): drop it.
                        pending_opens.remove(pos);
                    } else if let Some(last) = line.items.last_mut() {
                        last.closes.push((*node, line.width + cw));
                        line.width += cw;
                        pending_space = 0.0;
                    }
                }
                Tok::Space { node } => {
                    if !line.items.is_empty() {
                        let (font, size) = self.font_of(*node);
                        let t = &self.styles[*node].text;
                        pending_space = self.fonts.advance(font, ' ', size) + t.word_spacing + t.letter_spacing;
                    }
                }
                Tok::Break { node } => {
                    if line.items.is_empty() {
                        if self.dom.tag(*node) == Some("br") {
                            line.blank = Some(i);
                            est_y += self.line_est_height(lines.last().unwrap(), &toks, &blocks);
                            lines.push(self.new_line(x, w, est_y));
                        }
                    } else {
                        est_y += self.line_est_height(lines.last().unwrap(), &toks, &blocks);
                        lines.push(self.new_line(x, w, est_y));
                    }
                    pending_space = 0.0;
                }
                Tok::Word { text, node, hyphen, .. } => {
                    let ww = self.word_width(*node, text);
                    // white-space: nowrap: no line break between its words.
                    let wrap = !self.styles[*node].text.nowrap;
                    // A piece that may end a line with a hyphen needs room for the hyphen.
                    let extra = if *hyphen { self.word_width(*node, "-") } else { 0.0 };
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, ww, i, (x, w), &mut est_y, &toks, &blocks, (wrap, extra));
                }
                Tok::Image { w: iw, node, .. } => {
                    let iw = iw.min(w);
                    let wrap = !self.styles[*node].text.nowrap;
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, iw, i, (x, w), &mut est_y, &toks, &blocks, (wrap, 0.0));
                }
                Tok::InlineBlock { node, .. } => {
                    let bw = blocks.get(&i).map_or(0.0, |d| d.w);
                    let wrap = !self.styles[*node].text.nowrap;
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, bw, i, (x, w), &mut est_y, &toks, &blocks, (wrap, 0.0));
                }
            }
        }
        // Right-to-left text: put each line's words in visual order.
        let base_rtl = run.first().and_then(|&n| self.dom.nodes[n].parent).is_some_and(|p| self.styles[p].rtl);
        for line in lines.iter_mut() {
            bidi_reorder(line, &toks, base_rtl);
        }
        // Emit lines.
        let mut cy = y;
        // Inline boxes still open from earlier lines: (node, depth).
        let mut carried: Vec<NodeId> = Vec::new();
        for line in &lines {
            if let Some(to) = line.skip_to {
                cy = cy.max(to);
            }
            let (mut lh, mut above) = (0.0f32, 0.0f32);
            let mut below = 0.0f32;
            // vertical-align: each item's position and (above, below) its own baseline.
            let mut vps: Vec<(VPos, f32, f32)> = Vec::with_capacity(line.items.len());
            let (mut tall, mut any_top) = (0.0f32, false);
            for p in &line.items {
                let (h, a, b) = match blocks.get(&p.tok) {
                    Some(d) => (d.h, d.baseline, d.h - d.baseline),
                    None => self.tok_size(&toks[p.tok]),
                };
                let vp = self.vpos(tok_node(&toks[p.tok]), (a, b));
                match vp {
                    VPos::Raise(r) => {
                        lh = lh.max(h);
                        above = above.max(a + r);
                        below = below.max(b - r);
                    }
                    VPos::Top | VPos::Bottom => {
                        tall = tall.max(h);
                        any_top |= vp == VPos::Top;
                    }
                }
                vps.push((vp, a, b));
            }
            if line.items.is_empty() {
                if let Some(ti) = line.blank {
                    cy += self.tok_size(&toks[ti]).0;
                }
                continue;
            }
            // Top- and bottom-aligned items taller than the rest stretch the line: down for
            // top-aligned ones, and the baseline moves down for bottom-aligned ones.
            if tall > above + below {
                if any_top {
                    below = tall - above;
                } else {
                    above = tall - below;
                }
            }
            let lh = lh.max(above + below).max(tall);
            let align = if self.measuring { Align::Left } else { align };
            let shift = line.x_off
                + match align {
                    Align::Left => 0.0,
                    Align::Center => ((line.max_w - line.width) / 2.0).max(0.0),
                    Align::Right => (line.max_w - line.width).max(0.0),
                };
            let baseline = cy + above;
            let base_at = |vp: VPos, a: f32, b: f32| match vp {
                VPos::Raise(r) => baseline - r,
                VPos::Top => cy + a,
                VPos::Bottom => cy + lh - b,
            };
            let box_base = |n: NodeId| {
                let (_, a, b) = self.tok_size(&Tok::Space { node: n });
                base_at(self.vpos(n, (a, b)), a, b) + self.inline_rel(n, w).1
            };
            let box_dx = |n: NodeId| self.inline_rel(n, w).0;
            let mut prev_underline_end: Option<(f32, NodeId)> = None;
            let mut last_text: Option<(NodeId, f32)> = None;
            // Inline box backgrounds go under this line's text: collect, then insert.
            let line_start = self.items.len();
            let mut boxes: Vec<(usize, Item)> = Vec::new();
            let mut open: Vec<(NodeId, f32)> = carried.drain(..).map(|n| (n, x + shift)).collect();
            for p in &line.items {
                for &(n, bx) in &p.opens {
                    open.push((n, x + shift + bx));
                }
                for &(n, ex) in &p.closes {
                    if let Some(pos) = open.iter().rposition(|(o, _)| *o == n) {
                        let (_, sx) = open.remove(pos);
                        let mut tmp = Vec::new();
                        let dx = box_dx(n);
                        self.inline_box(n, sx + dx, x + shift + ex + dx, box_base(n), w, &mut tmp);
                        boxes.extend(tmp.into_iter().map(|it| (pos, it)));
                    }
                }
            }
            for (depth, &(n, sx)) in open.iter().enumerate() {
                let mut tmp = Vec::new();
                let dx = box_dx(n);
                self.inline_box(n, sx + dx, x + shift + line.width + dx, box_base(n), w, &mut tmp);
                boxes.extend(tmp.into_iter().map(|it| (depth, it)));
            }
            carried = open.into_iter().map(|(n, _)| n).collect();
            boxes.sort_by_key(|(d, _)| *d);
            // Text items whose text has a text-shadow (their shadows go in after the line).
            let mut shadowed: Vec<(usize, NodeId)> = Vec::new();
            // text-overflow: ellipsis on a block that clips: a line too long for it ends in "…".
            let block_style = run.first().and_then(|&n| self.dom.nodes[n].parent).map(|p| &self.styles[p]);
            let cut = block_style.filter(|b| b.text.ellipsis && b.clip && !self.measuring && line.width > line.max_w + 0.5).map(|_| line.max_w);
            for (pi, p) in line.items.iter().enumerate() {
                if cut.is_some_and(|c| p.x >= c - 0.5) {
                    break;
                }
                let (rdx, rdy) = self.inline_rel(tok_node(&toks[p.tok]), w);
                let px = x + shift + p.x + rdx;
                let (vp, va, vb) = vps[pi];
                let baseline = base_at(vp, va, vb) + rdy;
                match &toks[p.tok] {
                    Tok::Word { text, node, link, glue, hyphen } => {
                        let s = &self.styles[*node];
                        let (font, size) = self.font_of(*node);
                        if !self.measuring {
                            crate::editing::record_word(self.dom, *node, px, baseline, size, font, text);
                        }
                        // A hyphen where the line breaks after a soft or automatic hyphen point.
                        let last_on_line = pi + 1 == line.items.len();
                        let shown: String = if *hyphen && last_on_line { format!("{text}-") } else { text.clone() };
                        let shown = match cut {
                            Some(c) if p.x + p.w > c => crate::text::ellipsize(&shown, (c - p.x).max(0.0), &|t| self.word_width(*node, t)),
                            _ => shown,
                        };
                        let text = &shown;
                        let ls = s.text.letter_spacing;
                        if !s.hidden && ls != 0.0 {
                            // Letter-spaced: one item per letter.
                            let mut lx = px;
                            for l in crate::text::letters(text) {
                                if s.text_shadow.is_some() {
                                    shadowed.push((self.items.len(), *node));
                                }
                                self.items.push(Item::Text { x: lx, y: baseline, text: l.to_string(), font, size, color: s.color });
                                lx += self.fonts.width(font, l, size) + ls;
                            }
                            last_text = None;
                        }
                        if !s.hidden && ls == 0.0 {
                            // Words of the same text run on one line become one text item
                            // ("Hello world" instead of "Hello" + "world"): far fewer items.
                            let space = self.fonts.advance(font, ' ', size);
                            let joined = match (last_text, self.items.last_mut()) {
                                // Right-to-left words stay separate: the line's word order was
                                // already resolved, and shaping a mixed run would reverse it.
                                (Some((n, end)), Some(Item::Text { text: t, .. }))
                                    if n == *node
                                        && !s.underline
                                        && !s.line_through
                                        && !s.overline
                                        && (px - end - space).abs() < 0.5
                                        && !text.chars().any(crate::style::is_rtl_char)
                                        && !t.chars().any(crate::style::is_rtl_char) =>
                                {
                                    t.push(' ');
                                    t.push_str(text);
                                    true
                                }
                                // A piece of the same word (split where a line could break).
                                (Some((n, end)), Some(Item::Text { text: t, .. })) if *glue && n == *node && (px - end).abs() < 0.5 => {
                                    t.push_str(text);
                                    true
                                }
                                _ => false,
                            };
                            if !joined {
                                if s.text_shadow.is_some() {
                                    shadowed.push((self.items.len(), *node));
                                }
                                self.items.push(Item::Text { x: px, y: baseline, text: text.clone(), font, size, color: s.color });
                            }
                            let deco = s.underline || s.line_through || s.overline;
                            last_text = if deco { None } else { Some((*node, px + p.w)) };
                            if deco {
                                // Extend the lines back across the space before this word.
                                let start = match prev_underline_end {
                                    Some((end, n)) if n == *node => end,
                                    _ => px,
                                };
                                let th = s.text.deco_thickness.unwrap_or((size / 14.0).max(1.0));
                                let color = s.deco_color.unwrap_or(s.color);
                                let w = px + p.w - start;
                                let style = s.text.deco_style;
                                if s.underline {
                                    let off = s.text.underline_offset.unwrap_or(th + 1.0);
                                    crate::text::deco_line(&mut self.items, start, baseline + off, w, th, color, style);
                                }
                                if s.line_through {
                                    crate::text::deco_line(&mut self.items, start, baseline - size * 0.3, w, th, color, style);
                                }
                                if s.overline {
                                    crate::text::deco_line(&mut self.items, start, baseline - self.fonts.ascent(font, size), w, th, color, style);
                                }
                                prev_underline_end = Some((px + p.w, *node));
                            } else {
                                prev_underline_end = None;
                            }
                        }
                        let r = Rect { x: px, y: cy + rdy, w: p.w, h: lh };
                        // Likewise one hit area per run of words on a line.
                        match self.hits.last_mut() {
                            Some((h, n)) if *n == *node && h.y == r.y && h.h == r.h && r.x >= h.x + h.w - 0.5 && r.x - (h.x + h.w) < 40.0 => {
                                h.w = r.x + r.w - h.x;
                            }
                            _ => self.hits.push((r, *node)),
                        }
                        if let Some(href) = link {
                            match self.links.last_mut() {
                                Some((h, l)) if l == href && h.y == r.y && h.h == r.h && r.x >= h.x + h.w - 0.5 && r.x - (h.x + h.w) < 40.0 => {
                                    h.w = r.x + r.w - h.x;
                                }
                                _ => self.links.push((r, href.clone())),
                            }
                        }
                    }
                    Tok::InlineBlock { link, .. } => {
                        prev_underline_end = None;
                        last_text = None;
                        if let Some(d) = blocks.remove(&p.tok) {
                            let (dx, dy) = (px, baseline - d.baseline);
                            if let Some(href) = link {
                                self.links.push((Rect { x: dx, y: dy, w: d.w, h: d.h }, href.clone()));
                            }
                            for mut it in d.items {
                                shift_item(&mut it, dx, dy);
                                self.items.push(it);
                            }
                            let mv = |r: Rect| Rect { x: r.x + dx, y: r.y + dy, ..r };
                            self.links.extend(d.links.into_iter().map(|(r, l)| (mv(r), l)));
                            self.hits.extend(d.hits.into_iter().map(|(r, n)| (mv(r), n)));
                        }
                    }
                    Tok::Image { h, node, src, link, .. } => {
                        prev_underline_end = None;
                        last_text = None;
                        let r = Rect { x: px, y: baseline - h, w: p.w, h: *h };
                        self.hits.push((r, *node));
                        if let Some(href) = link {
                            self.links.push((r, href.clone()));
                        }
                        let loaded = src.as_deref().filter(|s| self.images.get(s).is_some());
                        if self.styles[*node].hidden {
                        } else if let Some(src) = loaded {
                            let src = src.to_string();
                            self.push_image(*node, r.x, r.y, r.w, r.h, src);
                        } else {
                            let c = Color { r: 200, g: 200, b: 200, a: 255 };
                            self.items.push(Item::Rect { x: r.x, y: r.y, w: r.w, h: r.h, color: c });
                        }
                    }
                    _ => {}
                }
            }
            for &(i, n) in shadowed.iter().rev() {
                if let Some(sh) = self.styles[n].text_shadow.clone() {
                    let extra = crate::effects::text_shadow_items(&self.items[i], &sh);
                    self.items.splice(i..i, extra);
                }
            }
            if !boxes.is_empty() {
                self.items.splice(line_start..line_start, boxes.into_iter().map(|(_, it)| it));
            }
            cy += lh;
        }
        cy - y
    }

    #[allow(clippy::too_many_arguments)]
    fn place(
        &self,
        lines: &mut Vec<Line>,
        pending_space: &mut f32,
        pending_opens: &mut Vec<(NodeId, f32)>,
        ww: f32,
        tok: usize,
        (x0, w): (f32, f32),
        est_y: &mut f32,
        toks: &[Tok],
        blocks: &HashMap<usize, Detached>,
        (wrap, extra): (bool, f32),
    ) {
        let line = lines.last_mut().unwrap();
        let mut sp = *pending_space;
        let lead: f32 = pending_opens.iter().map(|(_, w)| w).sum();
        if wrap && !line.items.is_empty() && line.width + sp + lead + ww + extra > line.max_w {
            *est_y += self.line_est_height(lines.last().unwrap(), toks, blocks);
            lines.push(self.new_line(x0, w, *est_y));
            sp = 0.0;
        }
        // Too narrow beside a float even for this one item: move the line down past the float.
        for _ in 0..32 {
            let line = lines.last().unwrap();
            if !line.items.is_empty() || lead + ww <= line.max_w || line.max_w >= w {
                break;
            }
            let next = self
                .floats
                .last()
                .and_then(|fs| fs.iter().filter(|f| f.y1 > *est_y).map(|f| f.y1).reduce(f32::min));
            match next {
                Some(n) => {
                    *est_y = n;
                    let fresh = self.new_line(x0, w, n);
                    let line = lines.last_mut().unwrap();
                    line.x_off = fresh.x_off;
                    line.max_w = fresh.max_w;
                    line.skip_to = Some(n);
                }
                None => break,
            }
        }
        let line = lines.last_mut().unwrap();
        let mut bx = line.width + sp;
        let mut opens = Vec::new();
        for (node, w) in pending_opens.drain(..) {
            opens.push((node, bx));
            bx += w;
        }
        line.items.push(Placed { x: bx, w: ww, tok, opens, closes: Vec::new() });
        line.width = bx + ww;
        *pending_space = 0.0;
    }

    /// Rough height of a line, used to predict where the next line goes.
    fn line_est_height(&self, line: &Line, toks: &[Tok], blocks: &HashMap<usize, Detached>) -> f32 {
        if line.items.is_empty() {
            return line.blank.map_or(0.0, |t| self.tok_size(&toks[t]).0);
        }
        let mut h = 0.0f32;
        for p in &line.items {
            h = h.max(match blocks.get(&p.tok) {
                Some(d) => d.h,
                None => self.tok_size(&toks[p.tok]).0,
            });
        }
        h
    }
}

/// The DOM node an inline token belongs to.
fn tok_node(t: &Tok) -> NodeId {
    match t {
        Tok::Word { node, .. } | Tok::Space { node } | Tok::Break { node } | Tok::Image { node, .. } | Tok::Open { node, .. } | Tok::Close { node, .. } | Tok::InlineBlock { node, .. } => *node,
    }
}

/// Where the first item starts and the extra space between items, for justify-content.
fn justify_offsets(j: Justify, free: f32, n: usize) -> (f32, f32) {
    let n = n.max(1) as f32;
    match j {
        Justify::Start => (0.0, 0.0),
        Justify::End => (free, 0.0),
        Justify::Center => (free / 2.0, 0.0),
        Justify::SpaceBetween if n > 1.0 => (0.0, free / (n - 1.0)),
        Justify::SpaceBetween => (0.0, 0.0),
        Justify::SpaceAround => (free / n / 2.0, free / n),
        Justify::SpaceEvenly => (free / (n + 1.0), free / (n + 1.0)),
    }
}

/// Reorder a line's items for bidirectional text (the Unicode Bidi Algorithm at word
/// granularity): each word takes the direction of its first strong letter, neutral items
/// (numbers, punctuation, images) take their neighbours' direction, then runs are reversed
/// level by level (rule L2). Shaping already put the letters inside each word in visual order.
fn bidi_reorder(line: &mut Line, toks: &[Tok], base_rtl: bool) {
    if line.items.len() < 1 {
        return;
    }
    let dirs: Vec<Option<bool>> = line
        .items
        .iter()
        .map(|p| match &toks[p.tok] {
            Tok::Word { text, .. } => crate::style::first_strong_rtl(text),
            _ => None,
        })
        .collect();
    if !base_rtl && !dirs.iter().any(|d| *d == Some(true)) {
        return;
    }
    let n = dirs.len();
    // Resolve neutrals: same strong direction on both sides wins, else the base direction.
    let resolved: Vec<bool> = (0..n)
        .map(|i| match dirs[i] {
            Some(d) => d,
            None => {
                let before = dirs[..i].iter().rev().find_map(|d| *d).unwrap_or(base_rtl);
                let after = dirs[i + 1..].iter().find_map(|d| *d).unwrap_or(base_rtl);
                if before == after { before } else { base_rtl }
            }
        })
        .collect();
    let levels: Vec<u8> = resolved
        .iter()
        .map(|&r| match (base_rtl, r) {
            (false, false) => 0,
            (false, true) => 1,
            (true, true) => 1,
            (true, false) => 2,
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    let max = *levels.iter().max().unwrap_or(&0);
    let min_odd = levels.iter().copied().filter(|l| l % 2 == 1).min().unwrap_or(1);
    let mut level = max;
    while level >= min_odd && level > 0 {
        let mut i = 0;
        while i < n {
            if levels[order[i]] >= level {
                let start = i;
                while i < n && levels[order[i]] >= level {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    if order.iter().enumerate().all(|(i, &o)| i == o) {
        return;
    }
    // Lay the items out again in visual order, keeping the spaces between them.
    let gap = |i: usize| if i == 0 { 0.0 } else { line.items[i].x - (line.items[i - 1].x + line.items[i - 1].w) };
    let mut new_x = vec![0.0f32; n];
    let mut pen = line.items[0].x;
    for (k, &i) in order.iter().enumerate() {
        if k > 0 {
            let prev = order[k - 1];
            pen += gap(i.max(prev)).max(0.0);
        }
        new_x[i] = pen;
        pen += line.items[i].w;
    }
    let mut items: Vec<Placed> = std::mem::take(&mut line.items);
    for (i, p) in items.iter_mut().enumerate() {
        let dx = new_x[i] - p.x;
        p.x += dx;
        for o in p.opens.iter_mut() {
            o.1 += dx;
        }
        for c in p.closes.iter_mut() {
            c.1 += dx;
        }
    }
    let mut slots: Vec<Option<Placed>> = items.into_iter().map(Some).collect();
    line.items = order.iter().map(|&i| slots[i].take().expect("each item once")).collect();
}

/// Applies `text-transform` (1 uppercase, 2 lowercase, 3 capitalize) to a word.
fn transform(word: &str, how: u8) -> String {
    match how {
        1 => word.to_uppercase(),
        2 => word.to_lowercase(),
        3 => {
            let mut c = word.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        }
        _ => word.to_string(),
    }
}

/// Draws one border side as rectangles: solid, dashed, dotted (square dots) or double.
fn border_side(items: &mut Vec<Item>, line: u8, x: f32, y: f32, w: f32, h: f32, color: Color) {
    let horizontal = w >= h;
    let thick = if horizontal { h } else { w };
    let len = if horizontal { w } else { h };
    match line {
        1 | 2 if len > thick * 3.0 => {
            // Dashes are 3× the width long with equal gaps; dots are width-sized.
            let (on, off) = if line == 1 { (thick * 3.0, thick * 3.0) } else { (thick, thick) };
            let n = ((len + off) / (on + off)).floor().max(1.0);
            // Stretch the gaps so the pattern ends flush at both corners.
            let gap = if n > 1.0 { (len - n * on) / (n - 1.0) } else { 0.0 };
            for i in 0..n as usize {
                let p = i as f32 * (on + gap);
                if horizontal {
                    items.push(Item::Rect { x: x + p, y, w: on, h, color });
                } else {
                    items.push(Item::Rect { x, y: y + p, w, h: on, color });
                }
            }
        }
        3 if thick >= 3.0 => {
            let t = (thick / 3.0).round().max(1.0);
            if horizontal {
                items.push(Item::Rect { x, y, w, h: t, color });
                items.push(Item::Rect { x, y: y + h - t, w, h: t, color });
            } else {
                items.push(Item::Rect { x, y, w: t, h, color });
                items.push(Item::Rect { x: x + w - t, y, w: t, h, color });
            }
        }
        _ => items.push(Item::Rect { x, y, w, h, color }),
    }
}
