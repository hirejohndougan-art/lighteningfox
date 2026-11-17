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
}

pub fn set_focus(f: Option<(NodeId, bool)>) {
    FOCUS.with(|c| c.set(f));
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
    /// Is link/hit `i` part of a fixed box (which doesn't scroll)?
    fn is_fixed(&self, i: usize, links: bool) -> bool {
        self.fixed.iter().any(|(l, h)| if links { l.contains(&i) } else { h.contains(&i) })
    }

    /// The link under a point in the viewport (y measured from the top of the page area).
    /// Fixed boxes are checked first: they sit on top.
    pub fn link_at(&self, x: f32, view_y: f32, scroll: f32) -> Option<&String> {
        let pick = |fixed: bool| {
            self.links.iter().enumerate().rev().find(|(i, (r, _))| {
                self.is_fixed(*i, true) == fixed && r.contains(x, if fixed { view_y } else { view_y + scroll })
            })
        };
        pick(true).or_else(|| pick(false)).map(|(_, (_, l))| l)
    }

    /// The DOM node drawn under a point in the viewport.
    pub fn hit_at(&self, x: f32, view_y: f32, scroll: f32) -> Option<NodeId> {
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
}

pub fn layout(dom: &Dom, styles: &[Rc<Style>], fonts: &Fonts, images: &Images, width: f32, viewport_h: f32) -> Layout {
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
    };
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
    let mut fixed_depth = 0;
    items.retain(|it| match *it {
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
    Layout { items, links: cx.links, hits: cx.hits, fixed: cx.fixed, height, page_bg, boxes }
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
    for (_, l) in layers.iter_mut().filter(|(z, _)| *z > 0) {
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
    Word { text: String, node: NodeId, link: Option<String> },
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
        | Item::Box { x, y, .. }
        | Item::Ring { x, y, .. }
        | Item::Shadow { x, y, .. }
        | Item::ClipPush { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Item::ClipPop | Item::FixedPush | Item::FixedPop => {}
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

    /// Lay out an element as a block box. Returns (outer height, bottom margin).
    fn block(&mut self, id: NodeId, x: f32, y: f32, avail: f32, prev_mb: f32) -> (f32, f32) {
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
        let replaced = if self.dom.tag(id) == Some("img") { self.replaced_size(id, avail - ml - mr - extra) } else { None };
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
        // Form controls have their own natural width.
        let ctrl = self.control_kind(id);
        if let (Some(_), Len::Auto, None) = (ctrl, self.styles[id].width, forced.0) {
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
        // Collapse our top margin with the previous sibling's bottom margin.
        let mt_eff = mt.max(prev_mb) - prev_mb;
        let top = y + mt_eff;
        // Remember where this box's output starts (for position: relative).
        let start = (self.items.len(), self.links.len(), self.hits.len());
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
        let align = st.text_align;
        let is_flex = st.flex.container;
        // overflow: hidden and display: flow-root start a new float context that contains its floats.
        let new_bfc = st.clip || st.flow_root;
        if new_bfc {
            self.floats.push(Vec::new());
        }
        let (mut content_h, _) = match replaced {
            Some((rw, rh)) => {
                let src = self.dom.attr(id, "src").unwrap_or("").trim().to_string();
                if !self.styles[id].hidden {
                    self.items.push(Item::Image { x: cx, y: cy, w: rw, h: rh, src });
                }
                self.hits.push((Rect { x: cx, y: cy, w: rw, h: rh }, id));
                (rh, 0.0)
            }
            None if ctrl.is_some() => (self.control_content(id, cx, cy, w), 0.0),
            None if table.is_some() => {
                let stretch = self.styles[id].width != Len::Auto || forced.0.is_some();
                (self.table_children(table.as_ref().unwrap(), cx, cy, w, stretch), 0.0)
            }
            None if self.styles[id].grid.is_some() => {
                let definite = match (forced.1, self.styles[id].height) {
                    (Some(fh), _) => Some(fh - mt - mb - bt - pt - pb - bb),
                    (None, Len::Px(h)) => Some(h - vsizing),
                    _ => None,
                };
                (self.grid_children(id, cx, cy, w, definite), 0.0)
            }
            None if is_flex => {
                let definite = match (forced.1, st.height) {
                    (Some(fh), _) => Some(fh - mt - mb - bt - pt - pb - bb),
                    (None, Len::Px(h)) => Some(h - vsizing),
                    _ => None,
                };
                (self.flex_children(id, cx, cy, w, definite), 0.0)
            }
            None => self.block_children(id, cx, cy, w, align),
        };
        if new_bfc {
            content_h = content_h.max(self.float_bottom(true, true) - cy);
            self.floats.pop();
        }
        let st = &self.styles[id];
        if let (Len::Px(h), None) = (st.height, replaced) {
            content_h = content_h.max(h - vsizing);
        }
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
        // overflow: hidden clips children to the padding box.
        if st.clip {
            self.items[clip_slot] = Item::ClipPush { x: bx + bl, y: top + bt, w: box_w - bl - br, h: box_h - bt - bb, radius: (radius - bt).max(0.0) };
            self.items.push(Item::ClipPop);
        } else {
            placeholder(&mut self.items[clip_slot]);
        }
        let c = st.border_color;
        let uniform = bt == br && bt == bb && bt == bl;
        if !st.hidden && radius > 0.0 && uniform && bt > 0.0 {
            self.items.push(Item::Ring { x: bx, y: top, w: box_w, h: box_h, radius, width: bt, color: c });
        } else if !st.hidden {
            for (rx, ry, rw, rh) in [
                (bx, top, box_w, bt),
                (bx, top + box_h - bb, box_w, bb),
                (bx, top, bl, box_h),
                (bx + box_w - br, top, br, box_h),
            ] {
                if rw > 0.0 && rh > 0.0 {
                    self.items.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: c });
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
        // position: relative (and sticky, for now) shifts the box after layout.
        let st = &self.styles[id];
        if matches!(st.position, Position::Relative | Position::Sticky) {
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
        if positioned && z != 0 && !matches!(self.styles[id].position, Position::Absolute | Position::Fixed) {
            let lifted: Vec<Item> = self.items.drain(start.0..).collect();
            self.layers.push((z, lifted));
        }
        (mt_eff + box_h + mb, mb)
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
            let d = match self.forced.get(&id).copied() {
                Some((Some(fw), fh)) => self.layout_at(id, fw, fh),
                _ => self.layout_detached(id, cb.w),
            };
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
        let container_align = self.styles[id].text_align;

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
                    if self.is_out_of_flow(c) {
                        self.defer_absolute(c, x, y);
                        continue;
                    }
                    if !text.is_empty() {
                        items.push(FlexItem::Text(std::mem::take(&mut text)));
                    }
                    items.push(FlexItem::Node(c));
                }
                NodeData::Document => {}
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
                    match (st.flex.basis, st.width) {
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
                    let total: f32 = line.iter().map(|&i| flex_of(i).1 * bases[i]).sum();
                    if total > 0.0 {
                        for (k, &i) in line.iter().enumerate() {
                            sizes[k] = (sizes[k] + free * flex_of(i).1 * bases[i] / total).max(0.0);
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
                    if stretch && self.styles[*n].height == Len::Auto && boxes[k].h < cross - 0.5 {
                        boxes[k] = self.layout_at(*n, sizes[k], Some(cross));
                    }
                }
            }
            let gaps = col_gap * (line.len().saturating_sub(1)) as f32;
            let free = (w - sizes.iter().sum::<f32>() - gaps).max(0.0);
            let (mut pos, between) = justify_offsets(fl.justify, free, line.len());
            let line_baseline = boxes.iter().map(|d| d.baseline).fold(0.0f32, f32::max);
            for (k, d) in boxes.into_iter().enumerate() {
                let i = line[k];
                let dy = match align_of(self, &items[i]) {
                    AlignItems::Center => (cross - d.h) / 2.0,
                    AlignItems::End => cross - d.h,
                    AlignItems::Baseline => line_baseline - d.baseline,
                    _ => 0.0,
                };
                self.place_detached(d, x + pos, cy + dy);
                pos += sizes[k] + col_gap + between;
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
                _ => self.fonts.width(font, "Browse… No file selected.", size).ceil(),
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
        let faint = Color { a: (st.color.a as f32 * 0.5) as u8, ..st.color };
        let accent = Color { r: 0x0b, g: 0x6b, b: 0xd3, a: 255 };
        let white = Color { r: 255, g: 255, b: 255, a: 255 };
        let hidden = st.hidden;
        let mut items: Vec<Item> = Vec::new();
        let height = match c {
            Control::Text { password } => {
                let value = forms::value(self.dom, id);
                let (text, color) = if value.is_empty() {
                    (self.dom.attr(id, "placeholder").unwrap_or("").replace(['\n', '\r'], ""), faint)
                } else if password {
                    ("•".repeat(value.chars().count()), st.color)
                } else {
                    (value.clone(), st.color)
                };
                let tw = if value.is_empty() { 0.0 } else { self.fonts.width(font, &text, size) };
                // Keep the end of long text (where the caret is) in view while editing.
                let shift = if focus.is_some() { (tw + 2.0 - w).max(0.0) } else { 0.0 };
                items.push(Item::ClipPush { x, y, w, h: lh, radius: 0.0 });
                if !text.is_empty() {
                    items.push(Item::Text { x: x - shift, y: baseline(y), text, font, size, color });
                }
                if let Some((_, true)) = focus {
                    items.push(Item::Rect { x: (x - shift + tw).round(), y: baseline(y) - ascent, w: 1.0, h: size * 1.15, color: st.color });
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
                // Wrap each paragraph at word boundaries (long words break anywhere).
                let mut lines: Vec<String> = Vec::new();
                for para in src.split('\n') {
                    let mut line = String::new();
                    for word in para.split_inclusive(' ') {
                        let trial = format!("{line}{word}");
                        if self.fonts.width(mono, trial.trim_end(), size) <= w || line.is_empty() {
                            line = trial;
                            while self.fonts.width(mono, line.trim_end(), size) > w && line.chars().count() > 1 {
                                let cut = line.char_indices().rev().find(|&(i, _)| self.fonts.width(mono, &line[..i], size) <= w).map(|(i, _)| i).unwrap_or(1).max(1);
                                lines.push(line[..cut].to_string());
                                line = line[cut..].to_string();
                            }
                        } else {
                            lines.push(std::mem::take(&mut line));
                            line = word.to_string();
                        }
                    }
                    lines.push(line);
                }
                let h = (rows * lh).max(lh);
                let visible = (h / lh).floor().max(1.0) as usize;
                let first = if focus.is_some() { lines.len().saturating_sub(visible) } else { 0 };
                items.push(Item::ClipPush { x, y, w, h, radius: 0.0 });
                let mut ly = y;
                for l in lines.iter().skip(first) {
                    if !l.is_empty() {
                        items.push(Item::Text { x, y: baseline(ly), text: l.trim_end_matches(' ').to_string(), font: mono, size, color });
                    }
                    ly += lh;
                }
                if let Some((_, true)) = focus {
                    let last = if value.is_empty() { "" } else { lines.last().map(String::as_str).unwrap_or("") };
                    let cy = y + (lines.len().saturating_sub(first).max(1) - 1) as f32 * lh;
                    let cx = x + self.fonts.width(mono, last, size);
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
                // A small downward triangle at the right.
                let (ax, ay) = (x + w - arrow * 0.75, (y + lh / 2.0 - 2.0).round());
                for row in 0..4 {
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
                        items.push(Item::Text { x, y: baseline(y), text: "Browse… No file selected.".into(), font, size, color: st.color });
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
                    if self.is_out_of_flow(c) {
                        self.defer_absolute(c, x, y);
                        continue;
                    }
                    if !text.is_empty() {
                        items.push(FlexItem::Text(std::mem::take(&mut text)));
                    }
                    items.push(FlexItem::Node(c));
                }
                NodeData::Document => {}
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
                    let stretch = align == AlignItems::Stretch && self.styles[*n].height == Len::Auto;
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
            Len::Auto => width,
            l => {
                let ml = st.margin[3].resolve(avail);
                let mr = st.margin[1].resolve(avail);
                let edges = if st.border_box { 0.0 } else { st.padding[1].resolve(avail) + st.padding[3].resolve(avail) + st.border[1] + st.border[3] };
                l.resolve(avail) + edges + ml + mr
            }
        };
        d
    }

    /// Size of an <img>: CSS width/height, else the width/height attributes, else the
    /// image's own size, keeping its aspect ratio; then shrunk to fit `max-width`.
    fn replaced_size(&self, id: NodeId, avail: f32) -> Option<(f32, f32)> {
        let st = &self.styles[id];
        let intrinsic = self.dom.attr(id, "src").and_then(|s| self.images.size(s.trim()));
        let attr = |a: &str| self.dom.attr(id, a).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
        let w = match st.width {
            Len::Auto => attr("width"),
            l => Some(l.resolve(avail)),
        };
        let h = match st.height {
            Len::Px(v) => Some(v),
            _ => attr("height"),
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
        let font = Fonts::pick(false, false, false);
        let size = st.font_size;
        let line_h = size * st.line_height;
        let baseline = cy + (line_h - size) / 2.0 + self.fonts.ascent(font, size);
        let tw = self.fonts.width(font, &text, size);
        self.items.push(Item::Text { x: cx - tw - size * 0.5, y: baseline, text, font, size, color: st.color });
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
                            out.push(Tok::Word { text: line.to_string(), node: id, link: link.map(String::from) });
                        }
                    }
                    return;
                }
                if t.starts_with(|c: char| c.is_ascii_whitespace()) {
                    out.push(Tok::Space { node: id });
                }
                let mut first = true;
                for word in t.split_ascii_whitespace() {
                    if !first {
                        out.push(Tok::Space { node: id });
                    }
                    first = false;
                    out.push(Tok::Word { text: word.to_string(), node: id, link: link.map(String::from) });
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
                    "img" => {
                        match self.replaced_size(id, avail) {
                            Some((w, h)) => {
                                let src = self.dom.attr(id, "src").map(|s| s.trim().to_string());
                                out.push(Tok::Image { w, h, node: id, src, link: link.map(String::from) })
                            }
                            None => {
                                if let Some(alt) = self.dom.attr(id, "alt").filter(|a| !a.trim().is_empty()) {
                                    out.push(Tok::Word { text: format!("[{}]", alt.trim()), node: id, link: link.map(String::from) });
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
            NodeData::Document => {}
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
        let c = st.border_color;
        if radius > 0.0 && bt == br && bt == bb && bt == bl && bt > 0.0 {
            out.push(Item::Ring { x: x0, y: top, w, h, radius, width: bt, color: c });
            return;
        }
        for (rx, ry, rw, rh) in [(x0, top, w, bt), (x0, top + h - bb, w, bb), (x0, top, bl, h), (x1 - br, top, br, h)] {
            if rw > 0.0 && rh > 0.0 {
                out.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color: c });
            }
        }
    }

    fn font_of(&self, node: NodeId) -> (u8, f32) {
        let s = &self.styles[node];
        (Fonts::pick(s.bold, s.italic, s.mono), s.font_size)
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
                        pending_space = self.fonts.advance(font, ' ', size);
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
                Tok::Word { text, node, .. } => {
                    let (font, size) = self.font_of(*node);
                    let ww = self.fonts.width(font, text, size);
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, ww, i, (x, w), &mut est_y, &toks, &blocks);
                }
                Tok::Image { w: iw, .. } => {
                    let iw = iw.min(w);
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, iw, i, (x, w), &mut est_y, &toks, &blocks);
                }
                Tok::InlineBlock { .. } => {
                    let bw = blocks.get(&i).map_or(0.0, |d| d.w);
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, bw, i, (x, w), &mut est_y, &toks, &blocks);
                }
            }
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
            for p in &line.items {
                let (h, a, b) = match blocks.get(&p.tok) {
                    Some(d) => (d.h, d.baseline, d.h - d.baseline),
                    None => self.tok_size(&toks[p.tok]),
                };
                lh = lh.max(h);
                above = above.max(a);
                below = below.max(b);
            }
            if line.items.is_empty() {
                if let Some(ti) = line.blank {
                    cy += self.tok_size(&toks[ti]).0;
                }
                continue;
            }
            let lh = lh.max(above + below);
            let align = if self.measuring { Align::Left } else { align };
            let shift = line.x_off
                + match align {
                    Align::Left => 0.0,
                    Align::Center => ((line.max_w - line.width) / 2.0).max(0.0),
                    Align::Right => (line.max_w - line.width).max(0.0),
                };
            let baseline = cy + above;
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
                        self.inline_box(n, sx, x + shift + ex, baseline, w, &mut tmp);
                        boxes.extend(tmp.into_iter().map(|it| (pos, it)));
                    }
                }
            }
            for (depth, &(n, sx)) in open.iter().enumerate() {
                let mut tmp = Vec::new();
                self.inline_box(n, sx, x + shift + line.width, baseline, w, &mut tmp);
                boxes.extend(tmp.into_iter().map(|it| (depth, it)));
            }
            carried = open.into_iter().map(|(n, _)| n).collect();
            boxes.sort_by_key(|(d, _)| *d);
            for p in &line.items {
                let px = x + shift + p.x;
                match &toks[p.tok] {
                    Tok::Word { text, node, link } => {
                        let s = &self.styles[*node];
                        let (font, size) = self.font_of(*node);
                        if !s.hidden {
                            // Words of the same text run on one line become one text item
                            // ("Hello world" instead of "Hello" + "world"): far fewer items.
                            let space = self.fonts.advance(font, ' ', size);
                            let joined = match (last_text, self.items.last_mut()) {
                                (Some((n, end)), Some(Item::Text { text: t, .. }))
                                    if n == *node && !s.underline && (px - end - space).abs() < 0.5 =>
                                {
                                    t.push(' ');
                                    t.push_str(text);
                                    true
                                }
                                _ => false,
                            };
                            if !joined {
                                self.items.push(Item::Text { x: px, y: baseline, text: text.clone(), font, size, color: s.color });
                            }
                            last_text = if s.underline { None } else { Some((*node, px + p.w)) };
                            if s.underline {
                                // Extend the underline back across the space before this word.
                                let start = match prev_underline_end {
                                    Some((end, n)) if n == *node => end,
                                    _ => px,
                                };
                                let th = (size / 14.0).max(1.0);
                                self.items.push(Item::Rect { x: start, y: baseline + th + 1.0, w: px + p.w - start, h: th, color: s.color });
                                prev_underline_end = Some((px + p.w, *node));
                            } else {
                                prev_underline_end = None;
                            }
                        }
                        let r = Rect { x: px, y: cy, w: p.w, h: lh };
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
                            self.items.push(Item::Image { x: r.x, y: r.y, w: r.w, h: r.h, src: src.to_string() });
                        } else {
                            let c = Color { r: 200, g: 200, b: 200, a: 255 };
                            self.items.push(Item::Rect { x: r.x, y: r.y, w: r.w, h: r.h, color: c });
                        }
                    }
                    _ => {}
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
    ) {
        let line = lines.last_mut().unwrap();
        let mut sp = *pending_space;
        let lead: f32 = pending_opens.iter().map(|(_, w)| w).sum();
        if !line.items.is_empty() && line.width + sp + lead + ww > line.max_w {
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
