//! Layout: positions every box and line of text, producing a display list.
//! Supports block flow (margins, padding, borders, width/max-width, auto-centering,
//! sibling margin collapsing) and inline flow (word wrapping, alignment, links, <br>, <pre>).

use crate::dom::{Dom, NodeData, NodeId};
use crate::font::Fonts;
use crate::images::Images;
use crate::paint::Item;
use crate::style::{Align, Color, Display, Len, ListStyle, Style};

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
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
    pub height: f32,
    pub page_bg: Color,
}

pub fn layout(dom: &Dom, styles: &[Style], fonts: &Fonts, images: &Images, width: f32) -> Layout {
    let mut cx = Ctx { dom, styles, fonts, images, items: Vec::new(), links: Vec::new(), hits: Vec::new(), measuring: false };
    let (height, _) = cx.block_children(Dom::ROOT, 0.0, 0.0, width, Align::Left);
    // Canvas background comes from <html>, else <body>, like real browsers.
    let mut page_bg = Color::rgb(255, 255, 255);
    for tag in ["body", "html"] {
        if let Some(&id) = dom.find_all(tag).first() {
            if styles[id].background.a > 0 {
                page_bg = styles[id].background;
            }
        }
    }
    Layout { items: cx.items, links: cx.links, hits: cx.hits, height, page_bg }
}

struct Ctx<'a> {
    dom: &'a Dom,
    styles: &'a [Style],
    fonts: &'a Fonts,
    images: &'a Images,
    items: Vec<Item>,
    links: Vec<(Rect, String)>,
    hits: Vec<(Rect, NodeId)>,
    /// True while measuring content width: lines are left-aligned.
    measuring: bool,
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
        | Item::Box { x, y, .. }
        | Item::Ring { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
    }
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
        let pct = |l: Len| l.resolve(avail);
        let mt = pct(st.margin[0]);
        let mb = pct(st.margin[2]);
        let mut ml = pct(st.margin[3]);
        let mut mr = pct(st.margin[1]);
        let [pt, pr, pb, pl] = st.padding.map(pct);
        let [bt, br, bb, bl] = st.border;
        let extra = pl + pr + bl + br;
        let mut w = match st.width {
            Len::Auto => avail - ml - mr - extra,
            l => pct(l),
        };
        let mut constrained = st.width != Len::Auto;
        // A block-level <img> takes the image's size instead of its children's.
        let replaced = if self.dom.tag(id) == Some("img") { self.replaced_size(id, avail - ml - mr - extra) } else { None };
        if let Some((rw, _)) = replaced {
            w = rw;
            constrained = true;
        }
        if st.max_width != Len::Auto && replaced.is_none() {
            let max = pct(st.max_width);
            if w > max {
                w = max;
                constrained = true;
            }
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
        let bg_slot = self.items.len();
        self.items.push(Item::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0, color: Color::TRANSPARENT });

        let cx = x + ml + bl + pl;
        let cy = top + bt + pt;
        let align = st.text_align;
        let (mut content_h, _) = match replaced {
            Some((rw, rh)) => {
                let src = self.dom.attr(id, "src").unwrap_or("").trim().to_string();
                if !self.styles[id].hidden {
                    self.items.push(Item::Image { x: cx, y: cy, w: rw, h: rh, src });
                }
                self.hits.push((Rect { x: cx, y: cy, w: rw, h: rh }, id));
                (rh, 0.0)
            }
            None => self.block_children(id, cx, cy, w, align),
        };
        if let (Len::Px(h), None) = (st.height, replaced) {
            content_h = content_h.max(h);
        }
        let st = &self.styles[id];
        if st.display == Display::ListItem && st.list_style != ListStyle::None && !st.hidden {
            self.list_marker(id, cx, cy);
        }
        let box_w = w + extra;
        let box_h = bt + pt + content_h + pb + bb;
        let bx = x + ml;
        let radius = st.radius.resolve(box_w.min(box_h)).min(box_w.min(box_h) / 2.0);
        if st.bg.visible() && !st.hidden {
            self.items[bg_slot] = Item::Box { x: bx, y: top, w: box_w, h: box_h, radius, bg: Box::new(st.bg.clone()) };
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
        (mt_eff + box_h + mb, mb)
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
            if self.is_block(c) {
                if !run.is_empty() {
                    let h = self.inline(&run, x, cy, w, align);
                    if h > 0.0 {
                        prev_mb = 0.0;
                    }
                    cy += h;
                    run.clear();
                }
                let (h, mb) = self.block(c, x, cy, w, prev_mb);
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
        let saved = (std::mem::take(&mut self.items), std::mem::take(&mut self.links), std::mem::take(&mut self.hits));
        let st = &self.styles[id];
        let mut width = avail;
        if st.width == Len::Auto {
            let was = self.measuring;
            self.measuring = true;
            self.block(id, 0.0, 0.0, avail, 0.0);
            self.measuring = was;
            let mut right = 0.0f32;
            for (i, it) in self.items.iter().enumerate() {
                let r = match it {
                    Item::Text { x, text, font, size, .. } => x + self.fonts.width(*font, text, *size),
                    Item::Image { x, w, .. } => x + w,
                    Item::Box { x, w, .. } if i > 0 => x + w,
                    _ => 0.0,
                };
                right = right.max(r);
            }
            let st = &self.styles[id];
            let tail = st.padding[1].resolve(avail) + st.border[1] + st.margin[1].resolve(avail);
            width = (right.ceil() + tail).min(avail);
            self.items.clear();
            self.links.clear();
            self.hits.clear();
        }
        let (h, _) = self.block(id, 0.0, 0.0, width, 0.0);
        let st = &self.styles[id];
        let w = match st.width {
            Len::Auto => width,
            l => {
                let ml = st.margin[3].resolve(avail);
                let mr = st.margin[1].resolve(avail);
                l.resolve(avail) + st.padding[1].resolve(avail) + st.padding[3].resolve(avail) + st.border[1] + st.border[3] + ml + mr
            }
        };
        let items = std::mem::replace(&mut self.items, saved.0);
        let links = std::mem::replace(&mut self.links, saved.1);
        let hits = std::mem::replace(&mut self.hits, saved.2);
        let baseline = items
            .iter()
            .filter_map(|it| if let Item::Text { y, .. } = it { Some(*y) } else { None })
            .fold(None, |m: Option<f32>, y| Some(m.map_or(y, |m| m.max(y))))
            .unwrap_or(h);
        Detached { items, links, hits, w, h, baseline }
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
                if self.styles[id].display == Display::None {
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
                    "input" => {
                        if self.dom.attr(id, "type") != Some("hidden") {
                            out.push(Tok::Image { w: 160.0, h: 22.0, node: id, src: None, link: None });
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
        let visible = st.bg.visible() || st.border.iter().any(|&b| b > 0.0) || pl > 0.0 || pr > 0.0;
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
        // Lay out inline-blocks first: their size decides where lines break.
        let mut blocks: std::collections::HashMap<usize, Detached> = std::collections::HashMap::new();
        for (i, t) in toks.iter().enumerate() {
            if let Tok::InlineBlock { node, .. } = t {
                let d = self.layout_detached(*node, w);
                blocks.insert(i, d);
            }
        }
        // Break into lines.
        let mut lines: Vec<Line> = vec![Line::default()];
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
                            lines.push(Line::default());
                        }
                    } else {
                        lines.push(Line::default());
                    }
                    pending_space = 0.0;
                }
                Tok::Word { text, node, .. } => {
                    let (font, size) = self.font_of(*node);
                    let ww = self.fonts.width(font, text, size);
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, ww, i, w);
                }
                Tok::Image { w: iw, .. } => {
                    let iw = iw.min(w);
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, iw, i, w);
                }
                Tok::InlineBlock { .. } => {
                    let bw = blocks.get(&i).map_or(0.0, |d| d.w);
                    self.place(&mut lines, &mut pending_space, &mut pending_opens, bw, i, w);
                }
            }
        }
        // Emit lines.
        let mut cy = y;
        // Inline boxes still open from earlier lines: (node, depth).
        let mut carried: Vec<NodeId> = Vec::new();
        for line in &lines {
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
            let shift = match align {
                Align::Left => 0.0,
                Align::Center => ((w - line.width) / 2.0).max(0.0),
                Align::Right => (w - line.width).max(0.0),
            };
            let baseline = cy + above;
            let mut prev_underline_end: Option<(f32, NodeId)> = None;
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
                            self.items.push(Item::Text { x: px, y: baseline, text: text.clone(), font, size, color: s.color });
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
                        self.hits.push((r, *node));
                        if let Some(href) = link {
                            self.links.push((r, href.clone()));
                        }
                    }
                    Tok::InlineBlock { link, .. } => {
                        prev_underline_end = None;
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

    fn place(&self, lines: &mut Vec<Line>, pending_space: &mut f32, pending_opens: &mut Vec<(NodeId, f32)>, ww: f32, tok: usize, max_w: f32) {
        let line = lines.last_mut().unwrap();
        let mut sp = *pending_space;
        let lead: f32 = pending_opens.iter().map(|(_, w)| w).sum();
        if !line.items.is_empty() && line.width + sp + lead + ww > max_w {
            lines.push(Line::default());
            sp = 0.0;
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
}
