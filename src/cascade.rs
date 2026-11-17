//! Cascade details: the CSS-wide keywords (`inherit`, `initial`, `unset`, `revert`),
//! cascade layer order, and `@container` conditions.

use crate::dom::{Dom, NodeId};
use crate::style::Style;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Properties that inherit by default (so `unset` means `inherit` for them).
pub fn is_inherited(name: &str) -> bool {
    matches!(
        name,
        "color" | "font" | "font-size" | "font-weight" | "font-style" | "font-family" | "font-variant" | "line-height" | "text-align" | "text-indent" | "text-transform"
            | "direction" | "white-space" | "visibility" | "cursor" | "list-style" | "list-style-type" | "list-style-position" | "border-spacing" | "border-collapse"
            | "text-shadow" | "letter-spacing" | "word-spacing" | "quotes" | "word-break" | "overflow-wrap" | "word-wrap" | "hyphens" | "tab-size" | "caption-side"
            | "empty-cells" | "text-decoration-color" | "text-rendering" | "-webkit-font-smoothing" | "text-wrap" | "text-wrap-mode" | "text-wrap-style"
            | "writing-mode" | "-webkit-writing-mode" | "text-orientation" | "-webkit-text-orientation" | "text-combine-upright" | "-webkit-text-combine"
    )
}

/// Give `dst` the value `src` has for property `name` (a longhand or a shorthand: all its
/// parts). Returns false for properties this doesn't know.
pub fn copy_prop(dst: &mut Style, src: &Style, name: &str) -> bool {
    let n = name.trim_start_matches("-webkit-");
    match n {
        "display" => {
            dst.display = src.display;
            dst.contents = src.contents;
        }
        "color" => dst.color = src.color,
        "font-size" => dst.font_size = src.font_size,
        "font-weight" => dst.bold = src.bold,
        "font-style" => dst.italic = src.italic,
        "font-family" => {
            dst.family = src.family.clone();
            dst.mono = src.mono;
        }
        "font" => {
            dst.font_size = src.font_size;
            dst.bold = src.bold;
            dst.italic = src.italic;
            dst.family = src.family.clone();
            dst.mono = src.mono;
            dst.line_height = src.line_height;
        }
        "line-height" => dst.line_height = src.line_height,
        "text-decoration" | "text-decoration-line" => {
            dst.underline = src.underline;
            dst.line_through = src.line_through;
            dst.overline = src.overline;
            if n == "text-decoration" {
                dst.deco_color = src.deco_color;
            }
        }
        "text-decoration-color" => dst.deco_color = src.deco_color,
        "text-transform" => dst.text_transform = src.text_transform,
        "text-indent" => dst.text_indent = src.text_indent,
        "cursor" => dst.cursor = src.cursor,
        "text-align" => {
            dst.text_align = src.text_align;
            dst.align_logical = src.align_logical;
        }
        "direction" => dst.rtl = src.rtl,
        "writing-mode" => dst.vertical.mode = src.vertical.mode,
        "text-orientation" => dst.vertical.orient = src.vertical.orient,
        "text-combine-upright" | "text-combine" => dst.vertical.combine = src.vertical.combine,
        "white-space" => {
            dst.pre = src.pre;
            dst.text = src.text.clone();
        }
        "letter-spacing" | "word-spacing" | "word-break" | "overflow-wrap" | "word-wrap" | "hyphens" | "text-overflow" | "text-wrap" | "text-wrap-mode" | "line-break"
        | "text-decoration-style" | "text-decoration-thickness" | "text-underline-offset" | "tab-size" => dst.text = src.text.clone(),
        "list-style" | "list-style-type" | "list-style-position" => {
            dst.list_style = src.list_style;
            dst.list_type = src.list_type;
            dst.list_inside = src.list_inside;
        }
        "visibility" => dst.hidden = src.hidden,
        "background-color" => {
            dst.background = src.background;
            dst.bg.color = src.bg.color;
        }
        "background" | "background-image" | "background-size" | "background-position" | "background-repeat" => {
            if n == "background" {
                dst.background = src.background;
            }
            dst.bg = src.bg.clone();
            if n != "background" {
                dst.bg.color = dst.background;
            }
        }
        "border-radius" => dst.radius = src.radius,
        // Each of these resets only its own part of the flex settings: `order: unset` after
        // `flex-basis: 100%` in one rule must not undo the basis (WordPress.org's header).
        "flex" => {
            dst.flex.grow = src.flex.grow;
            dst.flex.shrink = src.flex.shrink;
            dst.flex.basis = src.flex.basis;
        }
        "flex-grow" => dst.flex.grow = src.flex.grow,
        "flex-shrink" => dst.flex.shrink = src.flex.shrink,
        "flex-basis" => dst.flex.basis = src.flex.basis,
        "order" => dst.flex.order = src.flex.order,
        "flex-direction" => {
            dst.flex.column = src.flex.column;
            dst.flex.reverse = src.flex.reverse;
        }
        "flex-wrap" => dst.flex.wrap = src.flex.wrap,
        "flex-flow" => {
            dst.flex.column = src.flex.column;
            dst.flex.reverse = src.flex.reverse;
            dst.flex.wrap = src.flex.wrap;
        }
        "justify-content" => dst.flex.justify = src.flex.justify,
        "place-content" => {
            dst.flex.justify = src.flex.justify;
            dst.flex.align_content = src.flex.align_content;
        }
        "align-content" => dst.flex.align_content = src.flex.align_content,
        "align-items" => dst.flex.align_items = src.flex.align_items,
        "align-self" => dst.flex.align_self = src.flex.align_self,
        "place-items" => {
            dst.flex.align_items = src.flex.align_items;
            dst.flex.justify_items = src.flex.justify_items;
        }
        "justify-items" => dst.flex.justify_items = src.flex.justify_items,
        "justify-self" => dst.flex.justify_self = src.flex.justify_self,
        "gap" | "grid-gap" => {
            dst.flex.row_gap = src.flex.row_gap;
            dst.flex.column_gap = src.flex.column_gap;
        }
        "row-gap" => dst.flex.row_gap = src.flex.row_gap,
        "column-gap" => dst.flex.column_gap = src.flex.column_gap,
        "position" => dst.position = src.position,
        "top" => dst.inset[0] = src.inset[0],
        "right" => dst.inset[1] = src.inset[1],
        "bottom" => dst.inset[2] = src.inset[2],
        "left" => dst.inset[3] = src.inset[3],
        "inset" => dst.inset = src.inset,
        "z-index" => dst.z_index = src.z_index,
        "opacity" => dst.opacity = src.opacity,
        "aspect-ratio" => dst.aspect_ratio = src.aspect_ratio,
        "object-fit" => dst.object_fit = src.object_fit,
        "object-position" => dst.object_position = src.object_position,
        "overflow" | "overflow-x" | "overflow-y" => {
            dst.clip = src.clip;
            dst.scrolls = src.scrolls;
        }
        "box-shadow" => dst.shadow = src.shadow,
        "min-width" => dst.min_width = src.min_width,
        "min-height" => dst.min_height = src.min_height,
        "max-height" => dst.max_height = src.max_height,
        "max-width" => dst.max_width = src.max_width,
        "width" => dst.width = src.width,
        "height" => dst.height = src.height,
        "transform" | "translate" | "rotate" | "scale" => dst.transform = src.transform.clone(),
        "transform-origin" => dst.transform_origin = src.transform_origin.clone(),
        "filter" => dst.filter = src.filter.clone(),
        "backdrop-filter" => dst.backdrop = src.backdrop.clone(),
        "text-shadow" => dst.text_shadow = src.text_shadow.clone(),
        "box-sizing" => dst.border_box = src.border_box,
        "float" => dst.float = src.float,
        "clear" => dst.clear = src.clear,
        "border-spacing" => dst.border_spacing = src.border_spacing,
        "border-collapse" => dst.border_collapse = src.border_collapse,
        "vertical-align" => dst.valign = src.valign,
        "content" => dst.content = src.content.clone(),
        "appearance" => dst.appearance_none = src.appearance_none,
        "counter-reset" | "counter-set" | "counter-increment" => dst.counters = src.counters.clone(),
        "columns" | "column-count" | "column-width" | "column-rule" | "column-rule-width" | "column-rule-color" | "column-rule-style" => {
            dst.multicol = src.multicol.clone();
            dst.column_gap_set = src.column_gap_set;
        }
        "margin" => dst.margin = src.margin,
        "padding" => dst.padding = src.padding,
        "border" | "border-width" | "border-color" | "border-style" => {
            if n != "border-color" {
                dst.border = src.border;
                dst.border_line = src.border_line;
            }
            if n != "border-width" && n != "border-style" {
                dst.border_color = src.border_color;
                dst.border_color_set = src.border_color_set;
                dst.border_colors = src.border_colors;
            }
        }
        n if n.starts_with("border") && n.ends_with("radius") => dst.radius = src.radius,
        n if n.starts_with("border-") && (n.contains("image") || n.contains("inline") || n.contains("block")) => return false,
        n if n.starts_with("grid") => {
            dst.grid = src.grid.clone();
            dst.grid_place = src.grid_place.clone();
        }
        n if n.starts_with("margin-") => {
            let Some(i) = side(&n[7..]) else { return false };
            dst.margin[i] = src.margin[i];
        }
        n if n.starts_with("padding-") => {
            let Some(i) = side(&n[8..]) else { return false };
            dst.padding[i] = src.padding[i];
        }
        n if n.starts_with("border-") => {
            let rest = &n[7..];
            let (s, part) = rest.split_once('-').unwrap_or((rest, ""));
            let Some(i) = side(s) else { return false };
            if part != "color" {
                dst.border[i] = src.border[i];
            }
            if part != "width" && part != "style" {
                let mut sides = dst.border_colors.unwrap_or_default();
                sides[i] = src.border_colors.and_then(|c| c[i]).or(Some(src.side_colors()[i]));
                dst.border_colors = Some(sides);
            }
        }
        _ => return false,
    }
    true
}

fn side(s: &str) -> Option<usize> {
    ["top", "right", "bottom", "left"].iter().position(|x| *x == s)
}

/// The value a CSS-wide keyword gives a property: `inherit` takes the parent's, `initial`
/// the property's initial value, `unset` whichever of those the property normally does.
/// (`revert` is handled by the caller, which knows the browser's own rules.)
pub fn apply_keyword(st: &mut Style, parent: &Style, name: &str, kw: &str) {
    let inherit = match kw {
        "inherit" => true,
        "initial" => false,
        _ => is_inherited(name),
    };
    if inherit {
        copy_prop(st, parent, name);
    } else {
        let initial = Style::default();
        copy_prop(st, &initial, name);
    }
}

// ---------------------------------------------------------------- cascade layers

/// Every layer name (across the sheets, in order) mapped to its rank: later layers and a
/// layer's own rules (after its sub-layers) rank higher. Unlayered rules rank above all.
pub struct LayerOrder {
    ranks: HashMap<std::sync::Arc<str>, u32>,
    pub unlayered: u32,
}

impl LayerOrder {
    pub fn new<'a>(sheets: impl Iterator<Item = &'a crate::css::Stylesheet>) -> LayerOrder {
        let mut names: Vec<std::sync::Arc<str>> = Vec::new();
        for s in sheets {
            for l in &s.layers {
                if !names.contains(l) {
                    names.push(l.clone());
                }
            }
        }
        // Each name's sort key: its position among its siblings at every level, then MAX
        // (a layer's own rules come after its sub-layers).
        let mut sibling_pos: HashMap<String, u32> = HashMap::new();
        let mut counts: HashMap<String, u32> = HashMap::new();
        for n in &names {
            let parent = n.rsplit_once('.').map(|p| p.0).unwrap_or("").to_string();
            let c = counts.entry(parent).or_insert(0);
            sibling_pos.insert(n.to_string(), *c);
            *c += 1;
        }
        let key = |n: &str| -> Vec<u32> {
            let mut k = Vec::new();
            let mut prefix = String::new();
            for part in n.split('.') {
                if !prefix.is_empty() {
                    prefix.push('.');
                }
                prefix.push_str(part);
                k.push(sibling_pos.get(&prefix).copied().unwrap_or(0));
            }
            k.push(u32::MAX);
            k
        };
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| key(n));
        let ranks = sorted.iter().enumerate().map(|(i, n)| (n.clone(), i as u32)).collect();
        LayerOrder { ranks, unlayered: names.len() as u32 }
    }

    /// The sort key for a declaration: normal ones in later layers win; `!important` ones
    /// in earlier layers win (and unlayered important ones lose to layered ones).
    pub fn key(&self, layer: Option<&std::sync::Arc<str>>, important: bool) -> u32 {
        let r = layer.and_then(|l| self.ranks.get(l)).copied().unwrap_or(self.unlayered);
        if important { self.unlayered - r } else { r }
    }
}

// ---------------------------------------------------------------- @container

thread_local! {
    /// Each query container's content box size from the last layout.
    static SIZES: RefCell<HashMap<NodeId, (f32, f32)>> = RefCell::new(HashMap::new());
}

/// Record the size of every element with `container-type` after a layout. True if any
/// changed (styles that depend on them need computing again).
pub fn record(styles: &[Rc<Style>], boxes: &HashMap<NodeId, (crate::layout::Rect, bool)>) -> bool {
    let mut new = HashMap::new();
    for (&id, (r, _)) in boxes {
        let Some(st) = styles.get(id) else { continue };
        if st.container.is_none() {
            continue;
        }
        let w = r.w - st.border[1] - st.border[3] - st.padding[1].resolve(r.w) - st.padding[3].resolve(r.w);
        let h = r.h - st.border[0] - st.border[2] - st.padding[0].resolve(r.w) - st.padding[2].resolve(r.w);
        new.insert(id, (w.max(0.0), h.max(0.0)));
    }
    SIZES.with(|s| {
        let mut s = s.borrow_mut();
        let changed = *s != new;
        *s = new;
        changed
    })
}

/// Does an `@container` condition (several joined with " && ") hold for element `id`?
/// Each part may name its container: `sidebar (min-width: 400px)`.
pub fn matches(dom: &Dom, styles: &[Rc<Style>], id: NodeId, cond: &str) -> bool {
    cond.split(" && ").all(|part| {
        let part = part.trim();
        let (name, query) = match part.find('(') {
            Some(0) | None => (None, part),
            Some(p) => {
                let n = part[..p].trim();
                if n.eq_ignore_ascii_case("not") { (None, part) } else { (Some(n.to_string()), &part[p..]) }
            }
        };
        // The nearest ancestor that is a container (with that name).
        let mut cur = dom.nodes[id].parent;
        while let Some(a) = cur {
            if let Some(c) = styles.get(a).and_then(|s| s.container.as_ref()) {
                if name.as_deref().is_none_or(|n| c.1.split_whitespace().any(|x| x == n)) {
                    let Some((w, h)) = SIZES.with(|s| s.borrow().get(&a).copied()) else { return false };
                    // inline-size containers only answer width queries.
                    let h = if c.0 == 2 { h } else { f32::NAN };
                    return crate::css::container_query(&query.to_ascii_lowercase(), w, h);
                }
            }
            cur = dom.nodes[a].parent;
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_ranks() {
        let s = crate::css::parse("@layer reset, base; @layer base { a {} } @layer reset.inner { b {} } @layer reset { c {} }");
        let o = LayerOrder::new(std::iter::once(&s));
        let k = |n: &str| o.key(Some(&std::sync::Arc::from(n)), false);
        assert!(k("reset.inner") < k("reset"));
        assert!(k("reset") < k("base"));
        assert!(k("base") < o.key(None, false));
        // !important flips it.
        assert!(o.key(Some(&"reset".into()), true) > o.key(Some(&"base".into()), true));
        assert!(o.key(None, true) < o.key(Some(&"base".into()), true));
    }
}
