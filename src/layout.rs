//! Layout: positions every box and line of text, producing a display list.
//! Supports block flow (margins, padding, borders, width/max-width, auto-centering,
//! sibling margin collapsing) and inline flow (word wrapping, alignment, links, <br>, <pre>).

use crate::dom::{Dom, NodeData, NodeId};
use crate::font::Fonts;
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
    pub height: f32,
    pub page_bg: Color,
}

pub fn layout(dom: &Dom, styles: &[Style], fonts: &Fonts, width: f32) -> Layout {
    let mut cx = Ctx { dom, styles, fonts, items: Vec::new(), links: Vec::new() };
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
    Layout { items: cx.items, links: cx.links, height, page_bg }
}

struct Ctx<'a> {
    dom: &'a Dom,
    styles: &'a [Style],
    fonts: &'a Fonts,
    items: Vec<Item>,
    links: Vec<(Rect, String)>,
}

/// One unit of inline content.
enum Tok {
    Word { text: String, node: NodeId, link: Option<String> },
    Space { node: NodeId },
    Break { node: NodeId },
    Image { w: f32, h: f32 },
}

struct Placed {
    x: f32,
    w: f32,
    tok: usize,
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
        if st.max_width != Len::Auto {
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
        let (mut content_h, _) = self.block_children(id, cx, cy, w, align);
        if let Len::Px(h) = st.height {
            content_h = content_h.max(h);
        }
        let st = &self.styles[id];
        if st.display == Display::ListItem && st.list_style != ListStyle::None && !st.hidden {
            self.list_marker(id, cx, cy);
        }
        let box_w = w + extra;
        let box_h = bt + pt + content_h + pb + bb;
        let bx = x + ml;
        if st.background.a > 0 && !st.hidden {
            self.items[bg_slot] = Item::Rect { x: bx, y: top, w: box_w, h: box_h, color: st.background };
        }
        let c = st.border_color;
        if !st.hidden {
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

    fn flatten(&self, id: NodeId, link: Option<&str>, out: &mut Vec<Tok>) {
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
                        let num = |a: &str| self.dom.attr(id, a).and_then(|v| v.trim_end_matches("px").parse::<f32>().ok());
                        match (num("width"), num("height")) {
                            (Some(w), Some(h)) => out.push(Tok::Image { w, h }),
                            _ => {
                                if let Some(alt) = self.dom.attr(id, "alt").filter(|a| !a.trim().is_empty()) {
                                    out.push(Tok::Word { text: format!("[{}]", alt.trim()), node: id, link: link.map(String::from) });
                                }
                            }
                        }
                        return;
                    }
                    "input" => {
                        if self.dom.attr(id, "type") != Some("hidden") {
                            out.push(Tok::Image { w: 160.0, h: 22.0 });
                        }
                        return;
                    }
                    _ => {}
                }
                let href = if tag == "a" { self.dom.attr(id, "href") } else { None };
                let link = href.or(link);
                let block = self.is_block(id);
                if block {
                    out.push(Tok::Break { node: id });
                }
                for &c in &self.dom.nodes[id].children {
                    self.flatten(c, link, out);
                }
                if block {
                    out.push(Tok::Break { node: id });
                }
            }
            NodeData::Document => {}
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
        }
    }

    fn inline(&mut self, run: &[NodeId], x: f32, y: f32, w: f32, align: Align) -> f32 {
        let mut toks = Vec::new();
        for &id in run {
            self.flatten(id, None, &mut toks);
        }
        // Break into lines.
        let mut lines: Vec<Line> = vec![Line::default()];
        let mut pending_space = 0.0f32;
        for (i, t) in toks.iter().enumerate() {
            let line = lines.last_mut().unwrap();
            match t {
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
                    self.place(&mut lines, &mut pending_space, ww, i, w);
                }
                Tok::Image { w: iw, .. } => {
                    let iw = iw.min(w);
                    self.place(&mut lines, &mut pending_space, iw, i, w);
                }
            }
        }
        // Emit lines.
        let mut cy = y;
        for line in &lines {
            let (mut lh, mut above) = (0.0f32, 0.0f32);
            let mut below = 0.0f32;
            for p in &line.items {
                let (h, a, b) = self.tok_size(&toks[p.tok]);
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
            let shift = match align {
                Align::Left => 0.0,
                Align::Center => ((w - line.width) / 2.0).max(0.0),
                Align::Right => (w - line.width).max(0.0),
            };
            let baseline = cy + above;
            let mut prev_underline_end: Option<(f32, NodeId)> = None;
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
                        if let Some(href) = link {
                            self.links.push((Rect { x: px, y: cy, w: p.w, h: lh }, href.clone()));
                        }
                    }
                    Tok::Image { h, .. } => {
                        prev_underline_end = None;
                        let c = Color { r: 200, g: 200, b: 200, a: 255 };
                        self.items.push(Item::Rect { x: px, y: baseline - h, w: p.w, h: *h, color: c });
                    }
                    _ => {}
                }
            }
            cy += lh;
        }
        cy - y
    }

    fn place(&self, lines: &mut Vec<Line>, pending_space: &mut f32, ww: f32, tok: usize, max_w: f32) {
        let line = lines.last_mut().unwrap();
        let mut sp = *pending_space;
        if !line.items.is_empty() && line.width + sp + ww > max_w {
            lines.push(Line::default());
            sp = 0.0;
        }
        let line = lines.last_mut().unwrap();
        let px = line.width + sp;
        line.items.push(Placed { x: px, w: ww, tok });
        line.width = px + ww;
        *pending_space = 0.0;
    }
}
