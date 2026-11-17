//! Style: selector matching, the cascade and computed values.

use crate::css::{self, Combinator, Compound, Decl, Selector, Stylesheet};
use crate::dom::{Dom, NodeData, NodeId};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    /// The element under the mouse, for :hover.
    static HOVER: Cell<Option<NodeId>> = const { Cell::new(None) };
}

pub fn set_hover(node: Option<NodeId>) {
    HOVER.with(|h| h.set(node));
}

/// Does any rule in these sheets use :hover? (Only then is it worth restyling on mouse moves.)
pub fn uses_hover(sheets: &[Stylesheet]) -> bool {
    sheets.iter().any(|sh| sh.rules.iter().any(|r| r.selectors.iter().any(|s| s.parts.iter().any(|p| p.pseudo.iter().any(|x| x == "hover")))))
}

pub const VIEWPORT_W: f32 = 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 255 };
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    None,
    Block,
    Inline,
    /// A block box that sits inside a line of text (buttons, nav items, badges).
    InlineBlock,
    ListItem,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Len {
    Px(f32),
    Pct(f32),
    /// px + percent, from calc() like `calc(100% - 20px)`.
    Mix(f32, f32),
    Auto,
}

impl Len {
    pub fn resolve(self, base: f32) -> f32 {
        match self {
            Len::Px(v) => v,
            Len::Pct(p) => base * p / 100.0,
            Len::Mix(px, p) => px + base * p / 100.0,
            Len::Auto => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ListStyle {
    Disc,
    Decimal,
    None,
}

/// A background image layer: a picture or a gradient.
#[derive(Debug, Clone, PartialEq)]
pub enum BgImage {
    Url(String),
    /// `angle` in degrees, CSS convention (0 = to top, 90 = to right, 180 = to bottom).
    Linear { angle: f32, stops: Vec<(Color, Option<f32>)> },
    Radial { stops: Vec<(Color, Option<f32>)> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BgSize {
    Auto,
    Cover,
    Contain,
    Size(Len, Len),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Background {
    pub color: Color,
    pub image: Option<BgImage>,
    pub size: BgSize,
    pub pos: (Len, Len),
    pub repeat: (bool, bool),
}

impl Default for Background {
    fn default() -> Self {
        Background {
            color: Color::TRANSPARENT,
            image: None,
            size: BgSize::Auto,
            pos: (Len::Pct(0.0), Len::Pct(0.0)),
            repeat: (true, true),
        }
    }
}

impl Background {
    pub fn visible(&self) -> bool {
        self.color.a > 0 || self.image.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Justify {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlignItems {
    Stretch,
    Start,
    End,
    Center,
    Baseline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TableRole {
    None,
    Table,
    RowGroup,
    Row,
    Cell,
    Caption,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridTemplate {
    pub columns: String,
    pub rows: String,
    pub areas: String,
    pub auto_rows: String,
    pub auto_columns: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridPlace {
    pub column: String,
    pub row: String,
    pub area: String,
}

/// Flex container and flex item properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flex {
    pub container: bool,
    pub column: bool,
    pub reverse: bool,
    pub wrap: bool,
    pub justify: Justify,
    pub align_items: AlignItems,
    /// For items: overrides the container's align-items.
    pub align_self: Option<AlignItems>,
    pub grow: f32,
    pub shrink: f32,
    pub basis: Len,
    pub order: i32,
    pub row_gap: Len,
    pub column_gap: Len,
    /// grid-template-columns: number of equal columns (simplified grid).
    pub grid_columns: Option<usize>,
}

impl Default for Flex {
    fn default() -> Self {
        Flex {
            container: false,
            column: false,
            reverse: false,
            wrap: false,
            justify: Justify::Start,
            align_items: AlignItems::Stretch,
            align_self: None,
            grow: 0.0,
            shrink: 1.0,
            basis: Len::Auto,
            order: 0,
            row_gap: Len::Px(0.0),
            column_gap: Len::Px(0.0),
            grid_columns: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Style {
    pub display: Display,
    // inherited
    pub color: Color,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// The `font-family` list, lower-case (web fonts are matched against it).
    pub family: Option<Rc<str>>,
    pub underline: bool,
    pub text_align: Align,
    /// text-align: start / end (0 = neither), which follow the writing direction.
    pub align_logical: u8,
    /// Right-to-left writing direction (`direction: rtl` or `dir="rtl"`).
    pub rtl: bool,
    pub line_height: f32,
    pub pre: bool,
    pub list_style: ListStyle,
    pub hidden: bool,
    /// CSS custom properties (`--name: value`), inherited. Shared until changed.
    pub vars: Rc<HashMap<String, String>>,
    // not inherited
    pub background: Color,
    pub bg: Background,
    pub radius: Len,
    pub flex: Flex,
    pub position: Position,
    /// top, right, bottom, left
    pub inset: [Len; 4],
    pub z_index: i32,
    pub clip: bool,
    /// overflow: auto / scroll: the box scrolls its content instead of only clipping it.
    pub scrolls: bool,
    pub shadow: Option<Shadow>,
    pub min_width: Len,
    pub min_height: Len,
    pub max_height: Len,
    /// `transform`, with the individual `translate`/`rotate`/`scale` properties folded in front.
    pub transform: Option<Rc<str>>,
    pub transform_origin: Option<Rc<str>>,
    /// box-sizing: border-box — width/height include padding and border.
    pub border_box: bool,
    /// float: Some(true) = left, Some(false) = right.
    pub float: Option<bool>,
    /// clear: (left, right)
    pub clear: (bool, bool),
    /// display: flow-root (starts a new block formatting context, contains floats)
    pub flow_root: bool,
    pub table_role: TableRole,
    /// CSS grid container and its templates (kept as written; resolved at layout time).
    pub grid: Option<Rc<GridTemplate>>,
    /// Grid item placement: grid-column, grid-row, grid-area (as written).
    pub grid_place: Option<Rc<GridPlace>>,
    pub border_spacing: f32,
    pub border_collapse: bool,
    pub valign: VAlign,
    pub margin: [Len; 4], // top right bottom left
    pub padding: [Len; 4],
    pub border: [f32; 4],
    pub border_color: Color,
    /// False until a border color is given; then the border follows `color` (currentColor).
    pub border_color_set: bool,
    pub width: Len,
    pub max_width: Len,
    pub height: Len,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            display: Display::Inline,
            color: Color::BLACK,
            font_size: 16.0,
            bold: false,
            italic: false,
            mono: false,
            family: None,
            underline: false,
            text_align: Align::Left,
            align_logical: 1,
            rtl: false,
            line_height: 1.25,
            pre: false,
            list_style: ListStyle::Disc,
            hidden: false,
            vars: Rc::default(),
            background: Color::TRANSPARENT,
            bg: Background::default(),
            radius: Len::Px(0.0),
            flex: Flex::default(),
            position: Position::Static,
            inset: [Len::Auto; 4],
            z_index: 0,
            clip: false,
            scrolls: false,
            shadow: None,
            min_width: Len::Auto,
            min_height: Len::Auto,
            max_height: Len::Auto,
            transform: None,
            transform_origin: None,
            border_box: false,
            float: None,
            clear: (false, false),
            flow_root: false,
            table_role: TableRole::None,
            grid: None,
            grid_place: None,
            border_spacing: 0.0,
            border_collapse: false,
            valign: VAlign::Baseline,
            margin: [Len::Px(0.0); 4],
            padding: [Len::Px(0.0); 4],
            border: [0.0; 4],
            border_color: Color::BLACK,
            border_color_set: false,
            width: Len::Auto,
            max_width: Len::Auto,
            height: Len::Auto,
        }
    }
}

impl Style {
    fn inherit(parent: &Style) -> Style {
        Style {
            color: parent.color,
            font_size: parent.font_size,
            bold: parent.bold,
            italic: parent.italic,
            mono: parent.mono,
            family: parent.family.clone(),
            underline: parent.underline,
            text_align: parent.text_align,
            align_logical: parent.align_logical,
            rtl: parent.rtl,
            line_height: parent.line_height,
            pre: parent.pre,
            list_style: parent.list_style,
            hidden: parent.hidden,
            vars: parent.vars.clone(),
            border_spacing: parent.border_spacing,
            border_collapse: parent.border_collapse,
            ..Style::default()
        }
    }
}

/// The browser's built-in default styles (the "user agent stylesheet").
pub const UA_CSS: &str = r#"
html, body, div, p, h1, h2, h3, h4, h5, h6, ul, ol, dl, dt, dd, pre, blockquote, form, fieldset,
header, footer, main, nav, section, article, aside, figure, figcaption, address, hr, table, tr,
thead, tbody, tfoot, caption, center, details, summary, menu, legend, hgroup, search, dialog { display: block; }
table { display: table; border-spacing: 2px; }
iframe { display: inline-block; border: 2px inset #767676; }
thead, tbody, tfoot { display: table-row-group; }
tr { display: table-row; }
td, th { display: table-cell; vertical-align: middle; }
caption { display: table-caption; text-align: center; }
colgroup, col { display: none; }
th { text-align: center; }
li { display: list-item; }
head, script, style, title, meta, link, noscript, template, base, datalist, dialog, svg { display: none; }
[hidden] { display: none; }
body { margin: 8px; }
p, dl, blockquote, figure { margin-top: 1em; margin-bottom: 1em; }
h1 { font-size: 2em; margin-top: 0.67em; margin-bottom: 0.67em; font-weight: bold; }
h2 { font-size: 1.5em; margin-top: 0.83em; margin-bottom: 0.83em; font-weight: bold; }
h3 { font-size: 1.17em; margin-top: 1em; margin-bottom: 1em; font-weight: bold; }
h4 { margin-top: 1.33em; margin-bottom: 1.33em; font-weight: bold; }
h5 { font-size: 0.83em; font-weight: bold; }
h6 { font-size: 0.67em; font-weight: bold; }
ul, ol, menu { margin-top: 1em; margin-bottom: 1em; padding-left: 40px; }
ol { list-style-type: decimal; }
li ul, li ol { margin-top: 0; margin-bottom: 0; }
dd { margin-left: 40px; }
blockquote, figure { margin-left: 40px; margin-right: 40px; }
b, strong, th, dt { font-weight: bold; }
i, em, cite, var, dfn, address { font-style: italic; }
code, kbd, samp, tt, pre { font-family: monospace; }
pre { white-space: pre; margin-top: 1em; margin-bottom: 1em; }
a:link { color: #0000ee; text-decoration: underline; }
u, ins { text-decoration: underline; }
small { font-size: 0.83em; }
big { font-size: 1.2em; }
center { text-align: center; }
hr { border: 1px solid #888; margin-top: 0.5em; margin-bottom: 0.5em; }
th, td { padding: 1px; }
input, select, textarea { display: inline-block; border: 1px solid #8f8f9d; background-color: #ffffff; color: #111111; padding: 2px 4px; border-radius: 3px; font-size: 13.33px; font-weight: normal; font-style: normal; text-indent: 0; }
input[type=hidden] { display: none; }
input[type=checkbox], input[type=radio] { padding: 0; margin: 3px 3px 3px 4px; border-color: #767676; border-radius: 2px; }
input[type=radio] { border-radius: 50%; }
input[type=submit], input[type=reset], input[type=button], input[type=image], select { padding: 2px 8px; background-color: #e9e9ed; border-radius: 4px; }
input[type=range], input[type=color] { border: none; background-color: transparent; padding: 0; }
option, optgroup, datalist { display: none; }
input:disabled, select:disabled, textarea:disabled, button:disabled { color: #8f8f9d; }
button { display: inline-block; font-size: 13.33px; padding: 2px 8px; border: 1px solid #8f8f9d; background-color: #e9e9ed; border-radius: 4px; text-align: center; }
"#;

struct Matched<'a> {
    important: bool,
    origin: u8, // 0 = UA, 1 = author, 2 = inline
    spec: (u32, u32, u32),
    order: usize,
    decl: &'a Decl,
}

/// Compute a style for every node.
///
/// Styles are shared to save memory: text uses its parent's style, and elements
/// whose matched rules, attributes and parent are identical (list items, table
/// cells, paragraphs in a row) share one computed style.
/// A shadow root's own stylesheets: `:host` rules (which style the host element, found by
/// the marker attribute the composed tree gives it) and the rest (which style only nodes
/// inside that shadow tree).
fn shadow_sheets(dom: &Dom) -> HashMap<NodeId, (Stylesheet, Stylesheet)> {
    let mut texts: HashMap<NodeId, String> = HashMap::new();
    for (id, n) in dom.nodes.iter().enumerate() {
        if let (NodeData::Element { tag, .. }, Some(&root)) = (&n.data, dom.render_scope.get(&id)) {
            if tag == "style" {
                let t = texts.entry(root).or_default();
                t.push_str(&dom.text_content(id));
                t.push('\n');
            }
        }
    }
    texts
        .into_iter()
        .map(|(root, css)| {
            let (mut host, mut inner) = (String::new(), String::new());
            for rule in split_rules(&css) {
                if rule.split('{').next().unwrap_or("").contains(":host") {
                    host.push_str(&rewrite_host(&rule, root));
                } else {
                    inner.push_str(&rule);
                }
                host.push('\n');
                inner.push('\n');
            }
            (root, (css::parse(&host), css::parse(&inner)))
        })
        .collect()
}

/// Split CSS text into top-level rules (`selector { … }`, or an @-rule with its block).
fn split_rules(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in css.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    out.push(css[start..=i].to_string());
                    start = i + 1;
                }
            }
            ';' if depth == 0 => {
                out.push(css[start..=i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}

/// `:host` → the host's marker attribute; `:host(.x)` → the marker plus `.x`.
fn rewrite_host(rule: &str, root: NodeId) -> String {
    let marker = format!("[__shadowhost=\"{root}\"]");
    let (sel, body) = rule.split_at(rule.find('{').unwrap_or(rule.len()));
    let mut out = String::new();
    let mut rest = sel;
    while let Some(i) = rest.find(":host") {
        out.push_str(&rest[..i]);
        rest = &rest[i + 5..];
        if let Some(inner) = rest.strip_prefix('(') {
            let mut depth = 1;
            let end = inner.char_indices().find(|&(_, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                depth == 0
            });
            let end = end.map_or(inner.len(), |(e, _)| e);
            out.push_str(&marker);
            out.push_str(&inner[..end]);
            rest = &inner[(end + 1).min(inner.len())..];
        } else {
            out.push_str(&marker);
        }
    }
    out.push_str(rest);
    out.push_str(body);
    out
}

pub fn compute(dom: &Dom, sheets: &[Stylesheet]) -> Vec<Rc<Style>> {
    let ua = css::parse(UA_CSS);
    let scoped = if dom.render_scope.is_empty() { HashMap::new() } else { shadow_sheets(dom) };
    let hidden = Rc::new(Style { display: Display::None, ..Style::default() });
    let mut styles: Vec<Rc<Style>> = vec![hidden; dom.nodes.len()];
    styles[0] = Rc::new(Style { display: Display::Block, ..Style::default() });
    let mut shared: HashMap<(usize, Vec<usize>, String), Rc<Style>> = HashMap::new();
    // Walk the tree parent-first (scripts can move nodes, so ids aren't in tree order).
    let mut stack: Vec<NodeId> = dom.nodes[0].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        stack.extend(dom.nodes[id].children.iter().rev().copied());
        let parent = dom.nodes[id].parent.unwrap_or(0);
        let parent_style = styles[parent].clone();
        let NodeData::Element { tag, attrs } = &dom.nodes[id].data else {
            // Text inherits everything it uses from its parent.
            styles[id] = parent_style;
            continue;
        };
        let mut matched: Vec<Matched> = Vec::new();
        let mut order = 0;
        // The page's stylesheets style the document; a shadow tree is styled only by its
        // own (plus the browser's), and a host also by its shadow tree's `:host` rules.
        let page: Vec<&Stylesheet> = match dom.render_scope.get(&id) {
            Some(root) => scoped.get(root).map(|(_, inner)| vec![inner]).unwrap_or_default(),
            None => {
                let mut v: Vec<&Stylesheet> = sheets.iter().collect();
                if let Some((host_rules, _)) = dom.attr(id, "__shadowhost").and_then(|r| r.parse::<NodeId>().ok()).and_then(|r| scoped.get(&r)) {
                    // Page rules win over :host rules: put those first.
                    v.insert(0, host_rules);
                }
                v
            }
        };
        for (origin, sheet) in std::iter::once(&ua).chain(page.into_iter()).enumerate() {
            let origin = if origin == 0 { 0 } else { 1 };
            for rule in &sheet.rules {
                let best = rule
                    .selectors
                    .iter()
                    .filter(|s| matches(dom, id, s))
                    .map(|s| s.specificity())
                    .max();
                if let Some(spec) = best {
                    for d in &rule.decls {
                        matched.push(Matched { important: d.important, origin, spec, order, decl: d });
                        order += 1;
                    }
                }
            }
        }
        // Same parent style + same rules + same attributes that affect style = same result.
        let attr_key: String = attrs
            .iter()
            .filter(|(k, _)| matches!(k.as_str(), "style" | "bgcolor" | "color" | "align" | "valign" | "width" | "border" | "cellpadding" | "cellspacing" | "hidden" | "dir"))
            .map(|(k, v)| format!("{k}={v};"))
            .collect::<String>()
            + tag
            // dir="auto" depends on the element's own text.
            + &if dom.attr(id, "dir").is_some_and(|d| d.eq_ignore_ascii_case("auto")) { format!("#{id}") } else { String::new() };
        let key = (Rc::as_ptr(&parent_style) as usize, matched.iter().map(|m| m.decl as *const Decl as usize).collect(), attr_key);
        if let Some(s) = shared.get(&key) {
            styles[id] = s.clone();
            continue;
        }
        let mut st = Style::inherit(&parent_style);
        let inline = dom.attr(id, "style").map(css::parse_declarations).unwrap_or_default();
        for d in &inline {
            matched.push(Matched { important: d.important, origin: 2, spec: (9, 9, 9), order, decl: d });
            order += 1;
        }
        matched.sort_by_key(|m| (m.important, m.origin, m.spec, m.order));
        // Custom properties first, so normal properties can use them via var().
        for m in matched.iter().filter(|m| m.decl.name.starts_with("--")) {
            let value = substitute_vars(&m.decl.value, &st.vars, 0);
            Rc::make_mut(&mut st.vars).insert(m.decl.name.clone(), value);
        }
        // HTML attributes like bgcolor and cellpadding beat the browser defaults
        // but lose to the page's own CSS, so they go in between.
        let mut hinted = false;
        for m in matched.iter().filter(|m| !m.decl.name.starts_with("--")) {
            if !hinted && (m.origin > 0 || m.important) {
                presentational_hints(dom, id, &mut st, &parent_style);
                hinted = true;
            }
            if m.decl.value.contains("var(") {
                let value = substitute_vars(&m.decl.value, &st.vars, 0);
                apply(&mut st, &parent_style, &m.decl.name, &value);
            } else {
                apply(&mut st, &parent_style, &m.decl.name, &m.decl.value);
            }
        }
        if !hinted {
            presentational_hints(dom, id, &mut st, &parent_style);
        }
        // Borders without a color of their own use the text color.
        if !st.border_color_set {
            st.border_color = st.color;
        }
        let st = Rc::new(st);
        shared.insert(key, st.clone());
        styles[id] = st;
    }
    styles
}

/// Old-fashioned HTML attributes that act like CSS (bgcolor, align, <font color>...).
fn presentational_hints(dom: &Dom, id: NodeId, st: &mut Style, parent: &Style) {
    if let Some(c) = dom.attr(id, "bgcolor").and_then(parse_color) {
        st.background = c;
        st.bg.color = c;
    }
    // dir="rtl" / "ltr" / "auto" (auto: the first strongly directional letter decides).
    if let Some(d) = dom.attr(id, "dir") {
        match d.trim().to_ascii_lowercase().as_str() {
            "rtl" => st.rtl = true,
            "ltr" => st.rtl = false,
            "auto" => st.rtl = first_strong_rtl(&dom.text_content(id)).unwrap_or(false),
            _ => {}
        }
    }
    if dom.tag(id) == Some("font") {
        if let Some(c) = dom.attr(id, "color").and_then(parse_color) {
            st.color = c;
        }
    }
    if let Some(a) = dom.attr(id, "align") {
        if dom.tag(id) == Some("img") || dom.tag(id) == Some("table") {
            match a.to_ascii_lowercase().as_str() {
                "left" => st.float = Some(true),
                "right" => st.float = Some(false),
                "center" | "middle" => {
                    st.margin[1] = Len::Auto;
                    st.margin[3] = Len::Auto;
                }
                _ => {}
            }
        } else {
            apply(st, parent, "text-align", a);
        }
    }
    // Old table attributes: border, cellpadding, cellspacing, valign.
    let tag = dom.tag(id).unwrap_or("");
    if tag == "table" {
        if let Some(b) = dom.attr(id, "border").map(|b| b.trim().parse::<f32>().unwrap_or(1.0)) {
            if b > 0.0 {
                st.border = [b; 4];
                st.border_color = Color::rgb(128, 128, 128);
                st.border_color_set = true;
            }
        }
        if let Some(sp) = dom.attr(id, "cellspacing").and_then(|v| v.trim().parse::<f32>().ok()) {
            st.border_spacing = sp;
        }
    }
    if tag == "td" || tag == "th" {
        let mut table = dom.nodes[id].parent;
        while let Some(t) = table {
            if dom.tag(t) == Some("table") {
                break;
            }
            table = dom.nodes[t].parent;
        }
        if let Some(t) = table {
            if dom.attr(t, "border").map_or(false, |b| b.trim().parse::<f32>().unwrap_or(1.0) > 0.0) {
                st.border = [1.0; 4];
                st.border_color = Color::rgb(160, 160, 160);
                st.border_color_set = true;
            }
            if let Some(p) = dom.attr(t, "cellpadding").and_then(|v| v.trim().parse::<f32>().ok()) {
                st.padding = [Len::Px(p); 4];
            }
        }
    }
    if let Some(v) = dom.attr(id, "valign") {
        apply(st, parent, "vertical-align", v);
    }
    if let Some(w) = dom.attr(id, "width") {
        if dom.tag(id) != Some("img") {
            let v = if w.chars().all(|c| c.is_ascii_digit()) { format!("{}px", w) } else { w.to_string() };
            apply(st, parent, "width", &v);
        }
    }
}

pub fn matches(dom: &Dom, id: NodeId, sel: &Selector) -> bool {
    match_from(dom, id, sel, sel.parts.len() - 1)
}

fn match_from(dom: &Dom, id: NodeId, sel: &Selector, idx: usize) -> bool {
    if !compound_matches(dom, id, &sel.parts[idx]) {
        return false;
    }
    if idx == 0 {
        return true;
    }
    let mut cur = dom.nodes[id].parent;
    match sel.combinators[idx - 1] {
        Combinator::Adjacent => prev_element(dom, id).map_or(false, |p| match_from(dom, p, sel, idx - 1)),
        Combinator::Sibling => {
            let mut p = prev_element(dom, id);
            while let Some(e) = p {
                if match_from(dom, e, sel, idx - 1) {
                    return true;
                }
                p = prev_element(dom, e);
            }
            false
        }
        Combinator::Child => cur.map_or(false, |p| match_from(dom, p, sel, idx - 1)),
        Combinator::Descendant => {
            while let Some(p) = cur {
                if match_from(dom, p, sel, idx - 1) {
                    return true;
                }
                cur = dom.nodes[p].parent;
            }
            false
        }
    }
}

fn prev_element(dom: &Dom, id: NodeId) -> Option<NodeId> {
    let parent = dom.nodes[id].parent?;
    let kids = &dom.nodes[parent].children;
    let pos = kids.iter().position(|&k| k == id)?;
    kids[..pos].iter().rev().copied().find(|&k| dom.tag(k).is_some())
}

/// Element siblings of `id` (including itself), in order.
fn element_siblings(dom: &Dom, id: NodeId) -> Vec<NodeId> {
    let parent = dom.nodes[id].parent.unwrap_or(0);
    dom.nodes[parent].children.iter().copied().filter(|&c| dom.tag(c).is_some()).collect()
}

/// Does a 1-based position match `an+b` (also "odd", "even", plain numbers)?
fn nth_matches(expr: &str, pos: usize) -> bool {
    let e: String = expr.split(" of ").next().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    let (a, b) = match e.as_str() {
        "odd" => (2, 1),
        "even" => (2, 0),
        _ => match e.find('n') {
            Some(i) => {
                let a = match &e[..i] {
                    "" | "+" => 1,
                    "-" => -1,
                    s => s.parse().unwrap_or(0),
                };
                let b = if e[i + 1..].is_empty() { 0 } else { e[i + 1..].parse().unwrap_or(0) };
                (a, b)
            }
            None => (0, e.parse().unwrap_or(0)),
        },
    };
    let pos = pos as i64;
    if a == 0 {
        return pos == b;
    }
    let k = pos - b;
    k % a == 0 && k / a >= 0
}

fn compound_matches(dom: &Dom, id: NodeId, c: &Compound) -> bool {
    let Some(tag) = dom.tag(id) else { return false };
    if let Some(t) = &c.tag {
        if t != tag {
            return false;
        }
    }
    if let Some(want) = &c.id {
        if dom.attr(id, "id") != Some(want.as_str()) {
            return false;
        }
    }
    for cls in &c.classes {
        if !dom.classes(id).any(|x| x == cls) {
            return false;
        }
    }
    for (k, op, v) in &c.attrs {
        let Some(actual) = dom.attr(id, k) else { return false };
        let Some(want) = v else { continue };
        let ok = match op {
            '~' => actual.split_whitespace().any(|w| w == want),
            '|' => actual == want || actual.starts_with(&format!("{want}-")),
            '^' => !want.is_empty() && actual.starts_with(want.as_str()),
            '$' => !want.is_empty() && actual.ends_with(want.as_str()),
            '*' => !want.is_empty() && actual.contains(want.as_str()),
            _ => actual == want,
        };
        if !ok {
            return false;
        }
    }
    for p in &c.pseudo {
        let ok = match p.as_str() {
            "link" | "any-link" => tag == "a" && dom.attr(id, "href").is_some(),
            "visited" => false,
            "root" => tag == "html",
            "first-child" | "last-child" | "only-child" => {
                let elems = element_siblings(dom, id);
                match p.as_str() {
                    "first-child" => elems.first() == Some(&id),
                    "last-child" => elems.last() == Some(&id),
                    _ => elems.len() == 1,
                }
            }
            "first-of-type" | "last-of-type" | "only-of-type" => {
                let same: Vec<NodeId> = element_siblings(dom, id).into_iter().filter(|&s| dom.tag(s) == Some(tag)).collect();
                match p.as_str() {
                    "first-of-type" => same.first() == Some(&id),
                    "last-of-type" => same.last() == Some(&id),
                    _ => same.len() == 1,
                }
            }
            "empty" => dom.nodes[id].children.iter().all(|&k| matches!(&dom.nodes[k].data, NodeData::Text(t) if t.is_empty())),
            "checked" => dom.attr(id, "checked").is_some() || dom.attr(id, "selected").is_some(),
            "focus" | "focus-visible" => crate::layout::focus() == Some(id),
            "focus-within" => crate::layout::focus().is_some_and(|f| {
                let mut cur = Some(f);
                while let Some(n) = cur {
                    if n == id {
                        return true;
                    }
                    cur = dom.nodes[n].parent;
                }
                false
            }),
            "placeholder-shown" => dom.attr(id, "placeholder").is_some() && dom.attr(id, "value").map_or(true, str::is_empty),
            "optional" => matches!(dom.tag(id), Some("input" | "select" | "textarea")) && dom.attr(id, "required").is_none(),
            "disabled" => dom.attr(id, "disabled").is_some(),
            "enabled" => matches!(tag, "input" | "button" | "select" | "textarea") && dom.attr(id, "disabled").is_none(),
            "required" => dom.attr(id, "required").is_some(),
            "hover" => {
                // The hovered element and all its ancestors are :hover.
                let mut n = HOVER.with(|h| h.get());
                let mut hit = false;
                while let Some(e) = n {
                    if e == id {
                        hit = true;
                        break;
                    }
                    n = dom.nodes[e].parent;
                }
                hit
            }
            other => {
                if let Some(arg) = other.strip_prefix("nth-child(").and_then(|a| a.strip_suffix(')')) {
                    let elems = element_siblings(dom, id);
                    elems.iter().position(|&e| e == id).map_or(false, |i| nth_matches(arg, i + 1))
                } else if let Some(arg) = other.strip_prefix("nth-last-child(").and_then(|a| a.strip_suffix(')')) {
                    let elems = element_siblings(dom, id);
                    elems.iter().rev().position(|&e| e == id).map_or(false, |i| nth_matches(arg, i + 1))
                } else if let Some(arg) = other.strip_prefix("nth-of-type(").and_then(|a| a.strip_suffix(')')) {
                    let same: Vec<NodeId> = element_siblings(dom, id).into_iter().filter(|&s| dom.tag(s) == Some(tag)).collect();
                    same.iter().position(|&e| e == id).map_or(false, |i| nth_matches(arg, i + 1))
                } else if let Some(arg) = other.strip_prefix("not(").and_then(|a| a.strip_suffix(')')) {
                    !css::parse_selector_list(arg).iter().any(|s| matches(dom, id, s))
                } else if let Some(arg) = ["is(", "where(", "matches(", "-webkit-any("]
                    .iter()
                    .find_map(|pre| other.strip_prefix(pre))
                    .and_then(|a| a.strip_suffix(')'))
                {
                    css::parse_selector_list(arg).iter().any(|s| matches(dom, id, s))
                } else {
                    false // :focus, :active, :visited, ::before... not supported yet
                }
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------- values

/// Replace every `var(--name, fallback)` with the variable's value.
pub fn substitute_vars(value: &str, vars: &HashMap<String, String>, depth: u32) -> String {
    if depth > 8 || !value.contains("var(") {
        return value.to_string();
    }
    let mut out = String::new();
    let mut rest = value;
    while let Some(pos) = rest.find("var(") {
        out.push_str(&rest[..pos]);
        let inner_start = pos + 4;
        let Some(len) = matching_paren(&rest[inner_start..]) else {
            out.push_str(&rest[pos..]);
            return out;
        };
        let inner = &rest[inner_start..inner_start + len];
        let (name, fallback) = match inner.find(',') {
            Some(c) => (inner[..c].trim(), Some(inner[c + 1..].trim())),
            None => (inner.trim(), None),
        };
        let replacement = vars.get(name).map(String::as_str).or(fallback).unwrap_or("");
        out.push_str(&substitute_vars(replacement, vars, depth + 1));
        rest = &rest[inner_start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// Length of the text up to the `)` that closes an already-opened paren.
fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 1;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// A calc() value: px + pct, or a plain number.
#[derive(Clone, Copy, Debug)]
struct CalcVal {
    px: f32,
    pct: f32,
    num: bool,
}

struct Calc<'a> {
    s: &'a [u8],
    i: usize,
    font_size: f32,
}

impl Calc<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<CalcVal> {
        let mut a = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let b = self.term()?;
            let sign = if op == b'+' { 1.0 } else { -1.0 };
            a = CalcVal { px: a.px + sign * b.px, pct: a.pct + sign * b.pct, num: a.num && b.num };
        }
        Some(a)
    }
    fn term(&mut self) -> Option<CalcVal> {
        let mut a = self.factor()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let b = self.factor()?;
            a = match (op, a.num, b.num) {
                (b'*', true, _) => CalcVal { px: b.px * a.px, pct: b.pct * a.px, num: b.num },
                (b'*', _, true) => CalcVal { px: a.px * b.px, pct: a.pct * b.px, num: false },
                (b'/', _, true) if b.px != 0.0 => CalcVal { px: a.px / b.px, pct: a.pct / b.px, num: a.num },
                _ => return None,
            };
        }
        Some(a)
    }
    fn factor(&mut self) -> Option<CalcVal> {
        self.ws();
        let rest = std::str::from_utf8(&self.s[self.i..]).ok()?;
        for f in ["calc(", "min(", "max(", "clamp("] {
            if rest.starts_with(f) {
                self.i += f.len();
                let mut args = vec![self.expr()?];
                while self.peek() == Some(b',') {
                    self.i += 1;
                    args.push(self.expr()?);
                }
                if self.peek() != Some(b')') {
                    return None;
                }
                self.i += 1;
                // min/max/clamp of mixed units can't stay symbolic; resolve % against the viewport.
                let flat = |v: CalcVal| v.px + v.pct * VIEWPORT_W / 100.0;
                let pick = |better: fn(f32, f32) -> bool| {
                    args.iter().copied().reduce(|a, b| if better(flat(b), flat(a)) { b } else { a })
                };
                return match f {
                    "calc(" => args.first().copied(),
                    "min(" => pick(|b, a| b < a),
                    "max(" => pick(|b, a| b > a),
                    _ if args.len() == 3 => {
                        let (lo, v, hi) = (args[0], args[1], args[2]);
                        Some(if flat(v) < flat(lo) { lo } else if flat(v) > flat(hi) { hi } else { v })
                    }
                    _ => None,
                };
            }
        }
        if rest.starts_with('(') {
            self.i += 1;
            let v = self.expr()?;
            if self.peek() != Some(b')') {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        // A number with an optional unit.
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], b'.' | b'-' | b'+')) {
            if (self.s[self.i] == b'-' || self.s[self.i] == b'+') && self.i > start {
                break;
            }
            self.i += 1;
        }
        let ustart = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphabetic() || self.s[self.i] == b'%') {
            self.i += 1;
        }
        let num: f32 = std::str::from_utf8(&self.s[start..ustart]).ok()?.parse().ok()?;
        let unit = std::str::from_utf8(&self.s[ustart..self.i]).ok()?;
        if unit.is_empty() {
            return Some(CalcVal { px: num, pct: 0.0, num: true });
        }
        Some(match parse_len(&format!("{num}{unit}"), self.font_size)? {
            Len::Px(px) => CalcVal { px, pct: 0.0, num: false },
            Len::Pct(p) => CalcVal { px: 0.0, pct: p, num: false },
            Len::Mix(px, p) => CalcVal { px, pct: p, num: false },
            Len::Auto => return None,
        })
    }
}

fn parse_calc(v: &str, font_size: f32) -> Option<Len> {
    let mut c = Calc { s: v.as_bytes(), i: 0, font_size };
    let r = c.expr()?;
    if c.peek().is_some() {
        return None;
    }
    Some(match (r.px != 0.0 || r.num, r.pct != 0.0) {
        (_, false) => Len::Px(r.px),
        (false, true) => Len::Pct(r.pct),
        (true, true) => Len::Mix(r.px, r.pct),
    })
}

pub fn parse_len(v: &str, font_size: f32) -> Option<Len> {
    let v = v.trim().to_ascii_lowercase();
    if v == "auto" {
        return Some(Len::Auto);
    }
    if v.starts_with("calc(") || v.starts_with("min(") || v.starts_with("max(") || v.starts_with("clamp(") {
        return parse_calc(&v, font_size);
    }
    let num_end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(v.len());
    let num: f32 = v[..num_end].parse().ok()?;
    let unit = &v[num_end..];
    Some(match unit {
        "" | "px" => Len::Px(num),
        "em" => Len::Px(num * font_size),
        "rem" => Len::Px(num * 16.0),
        "%" => Len::Pct(num),
        "pt" => Len::Px(num * 4.0 / 3.0),
        "vw" => Len::Px(num * VIEWPORT_W / 100.0),
        "vh" => Len::Px(num * 7.68),
        "ch" | "ex" => Len::Px(num * font_size * 0.5),
        _ => return None,
    })
}

pub fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let h = |s: &str| u8::from_str_radix(s, 16).ok();
        let d = |c: char| h(&format!("{c}{c}"));
        let c: Vec<char> = hex.chars().collect();
        return match c.len() {
            3 | 4 => Some(Color { r: d(c[0])?, g: d(c[1])?, b: d(c[2])?, a: if c.len() == 4 { d(c[3])? } else { 255 } }),
            6 | 8 => Some(Color {
                r: h(&hex[0..2])?,
                g: h(&hex[2..4])?,
                b: h(&hex[4..6])?,
                a: if c.len() == 8 { h(&hex[6..8])? } else { 255 },
            }),
            _ => None,
        };
    }
    if v.starts_with("rgb") {
        let inner = v.split_once('(')?.1.trim_end_matches(')');
        let parts: Vec<&str> = inner.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
        let comp = |s: &str| -> Option<u8> {
            Some(if let Some(p) = s.strip_suffix('%') { (p.parse::<f32>().ok()? * 2.55) as u8 } else { s.parse::<f32>().ok()?.clamp(0.0, 255.0) as u8 })
        };
        let a = match parts.get(3) {
            Some(s) => {
                if let Some(p) = s.strip_suffix('%') { (p.parse::<f32>().ok()? * 2.55) as u8 } else { (s.parse::<f32>().ok()? * 255.0) as u8 }
            }
            None => 255,
        };
        return Some(Color { r: comp(parts.first()?)?, g: comp(parts.get(1)?)?, b: comp(parts.get(2)?)?, a });
    }
    Some(match v.as_str() {
        "transparent" => Color::TRANSPARENT,
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "orange" => Color::rgb(255, 165, 0),
        "purple" => Color::rgb(128, 0, 128),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "lightgray" | "lightgrey" => Color::rgb(211, 211, 211),
        "darkgray" | "darkgrey" => Color::rgb(169, 169, 169),
        "maroon" => Color::rgb(128, 0, 0),
        "navy" => Color::rgb(0, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "olive" => Color::rgb(128, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "aqua" | "cyan" => Color::rgb(0, 255, 255),
        "fuchsia" | "magenta" => Color::rgb(255, 0, 255),
        "brown" => Color::rgb(165, 42, 42),
        "pink" => Color::rgb(255, 192, 203),
        "gold" => Color::rgb(255, 215, 0),
        "beige" => Color::rgb(245, 245, 220),
        "ivory" => Color::rgb(255, 255, 240),
        "whitesmoke" => Color::rgb(245, 245, 245),
        "darkblue" => Color::rgb(0, 0, 139),
        "darkgreen" => Color::rgb(0, 100, 0),
        "darkred" => Color::rgb(139, 0, 0),
        "steelblue" => Color::rgb(70, 130, 180),
        "slategray" | "slategrey" => Color::rgb(112, 128, 144),
        "dimgray" | "dimgrey" => Color::rgb(105, 105, 105),
        "indigo" => Color::rgb(75, 0, 130),
        "violet" => Color::rgb(238, 130, 238),
        "coral" => Color::rgb(255, 127, 80),
        "crimson" => Color::rgb(220, 20, 60),
        "tomato" => Color::rgb(255, 99, 71),
        "salmon" => Color::rgb(250, 128, 114),
        "khaki" => Color::rgb(240, 230, 140),
        "plum" => Color::rgb(221, 160, 221),
        "orchid" => Color::rgb(218, 112, 214),
        "tan" => Color::rgb(210, 180, 140),
        "chocolate" => Color::rgb(210, 105, 30),
        "sienna" => Color::rgb(160, 82, 45),
        "skyblue" => Color::rgb(135, 206, 235),
        "lightblue" => Color::rgb(173, 216, 230),
        "royalblue" => Color::rgb(65, 105, 225),
        "dodgerblue" => Color::rgb(30, 144, 255),
        "midnightblue" => Color::rgb(25, 25, 112),
        "turquoise" => Color::rgb(64, 224, 208),
        "seagreen" => Color::rgb(46, 139, 87),
        "forestgreen" => Color::rgb(34, 139, 34),
        "limegreen" => Color::rgb(50, 205, 50),
        "lightgreen" => Color::rgb(144, 238, 144),
        "darkorange" => Color::rgb(255, 140, 0),
        "goldenrod" => Color::rgb(218, 165, 32),
        "hotpink" => Color::rgb(255, 105, 180),
        "lavender" => Color::rgb(230, 230, 250),
        "linen" => Color::rgb(250, 240, 230),
        "snow" => Color::rgb(255, 250, 250),
        "honeydew" => Color::rgb(240, 255, 240),
        "aliceblue" => Color::rgb(240, 248, 255),
        "gainsboro" => Color::rgb(220, 220, 220),
        "slateblue" => Color::rgb(106, 90, 205),
        "darkslategray" | "darkslategrey" => Color::rgb(47, 79, 79),
        "lightyellow" => Color::rgb(255, 255, 224),
        "rebeccapurple" => Color::rgb(102, 51, 153),
        _ => return None,
    })
}

fn font_size_value(v: &str, parent: f32) -> Option<f32> {
    Some(match v.trim() {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "smaller" => parent * 0.83,
        "larger" => parent * 1.2,
        other => parse_len(other, parent)?.resolve(parent),
    })
}

/// Expand 1–4 values into [top, right, bottom, left].
fn four<T: Copy>(vals: &[T]) -> Option<[T; 4]> {
    Some(match vals.len() {
        1 => [vals[0]; 4],
        2 => [vals[0], vals[1], vals[0], vals[1]],
        3 => [vals[0], vals[1], vals[2], vals[1]],
        4 => [vals[0], vals[1], vals[2], vals[3]],
        _ => return None,
    })
}

fn side_index(s: &str) -> Option<usize> {
    ["top", "right", "bottom", "left"].iter().position(|x| *x == s)
}

fn border_width_word(t: &str, fs: f32) -> Option<f32> {
    match t {
        "thin" => Some(1.0),
        "medium" => Some(3.0),
        "thick" => Some(5.0),
        _ => parse_len(t, fs).map(|l| l.resolve(0.0)),
    }
}

/// Parse a `border` shorthand: returns (width, color).
fn border_shorthand(v: &str, fs: f32) -> (f32, Option<Color>) {
    let mut width = 3.0;
    let mut color = None;
    let mut has_style = false;
    for t in css::split_top(v, ' ') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if ["none", "hidden"].contains(&t) {
            return (0.0, None);
        }
        if ["solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset"].contains(&t) {
            has_style = true;
        } else if let Some(w) = border_width_word(t, fs) {
            width = w;
        } else if let Some(c) = parse_color(t) {
            color = Some(c);
        }
    }
    (if has_style { width } else { 0.0 }, color)
}

fn parse_repeat(t: &str) -> Option<(bool, bool)> {
    Some(match t.trim() {
        "repeat" => (true, true),
        "no-repeat" => (false, false),
        "repeat-x" => (true, false),
        "repeat-y" => (false, true),
        "space" | "round" => (true, true),
        "repeat no-repeat" => (true, false),
        "no-repeat repeat" => (false, true),
        _ => return None,
    })
}

fn parse_bg_size(v: &str, fs: f32) -> BgSize {
    match v.trim() {
        "cover" => BgSize::Cover,
        "contain" => BgSize::Contain,
        "" | "auto" | "auto auto" => BgSize::Auto,
        other => {
            let parts: Vec<Len> = other.split_whitespace().filter_map(|t| parse_len(t, fs)).collect();
            match parts.as_slice() {
                [w] => BgSize::Size(*w, Len::Auto),
                [w, h, ..] => BgSize::Size(*w, *h),
                _ => BgSize::Auto,
            }
        }
    }
}

/// `center`, `top right`, `50% 20px`, `right 10px top` (simplified) -> (x, y)
fn parse_bg_pos(v: &str, fs: f32) -> (Len, Len) {
    let words: Vec<&str> = v.split_whitespace().collect();
    let mut x = None;
    let mut y = None;
    for w in &words {
        match *w {
            "left" => x = Some(Len::Pct(0.0)),
            "right" => x = Some(Len::Pct(100.0)),
            "top" => y = Some(Len::Pct(0.0)),
            "bottom" => y = Some(Len::Pct(100.0)),
            "center" => {
                if x.is_none() {
                    x = Some(Len::Pct(50.0));
                } else if y.is_none() {
                    y = Some(Len::Pct(50.0));
                }
            }
            t => {
                if let Some(l) = parse_len(t, fs) {
                    if x.is_none() {
                        x = Some(l);
                    } else if y.is_none() {
                        y = Some(l);
                    }
                }
            }
        }
    }
    if words.len() == 1 && words[0] == "center" {
        return (Len::Pct(50.0), Len::Pct(50.0));
    }
    (x.unwrap_or(Len::Pct(50.0)), y.unwrap_or(Len::Pct(50.0)))
}

/// `url(...)`, `linear-gradient(...)`, `radial-gradient(...)`
pub fn parse_bg_image(v: &str) -> Option<BgImage> {
    let lower = v.to_ascii_lowercase();
    let inner = |prefix: &str| -> Option<String> {
        let start = lower.find(prefix)? + prefix.len();
        let end = v.rfind(')')?;
        (end >= start).then(|| v[start..end].to_string())
    };
    if lower.starts_with("url(") {
        let u = inner("url(")?;
        let u = u.trim().trim_matches(|c| c == '"' || c == '\'');
        return (!u.is_empty()).then(|| BgImage::Url(u.to_string()));
    }
    let is_linear = lower.starts_with("linear-gradient(") || lower.starts_with("-webkit-linear-gradient(") || lower.starts_with("repeating-linear-gradient(");
    let is_radial = lower.starts_with("radial-gradient(") || lower.starts_with("repeating-radial-gradient(");
    if !is_linear && !is_radial {
        return None;
    }
    let body = inner("gradient(")?;
    let mut args: Vec<String> = css::split_top(&body, ',').into_iter().map(|a| a.trim().to_string()).collect();
    let mut angle = 180.0;
    if let Some(first) = args.first().map(|a| a.to_ascii_lowercase()) {
        let is_stop = parse_color(first.split_whitespace().next().unwrap_or("")).is_some();
        if !is_stop {
            if is_linear {
                angle = if let Some(d) = first.strip_suffix("deg") {
                    d.trim().parse().unwrap_or(180.0)
                } else if let Some(t) = first.strip_suffix("turn") {
                    t.trim().parse::<f32>().unwrap_or(0.5) * 360.0
                } else {
                    let dir = first.trim_start_matches("to ").trim();
                    match dir {
                        "top" => 0.0,
                        "right" => 90.0,
                        "bottom" => 180.0,
                        "left" => 270.0,
                        "top right" | "right top" => 45.0,
                        "bottom right" | "right bottom" => 135.0,
                        "bottom left" | "left bottom" => 225.0,
                        "top left" | "left top" => 315.0,
                        _ => 180.0,
                    }
                };
            }
            args.remove(0);
        }
    }
    let mut stops = Vec::new();
    for a in args {
        let parts = css::split_top(&a, ' ');
        let Some(c) = parts.first().and_then(|c| parse_color(c.trim())) else { continue };
        let pos = parts.get(1).and_then(|p| p.trim().strip_suffix('%')).and_then(|p| p.parse::<f32>().ok()).map(|p| p / 100.0);
        stops.push((c, pos));
    }
    if stops.is_empty() {
        return None;
    }
    Some(if is_linear { BgImage::Linear { angle, stops } } else { BgImage::Radial { stops } })
}

pub fn apply(st: &mut Style, parent: &Style, name: &str, value: &str) {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if lower == "inherit" {
        return; // inherited props already inherited; good enough for now
    }
    let fs = st.font_size;
    let lens = |s: &str| -> Option<Vec<Len>> {
        css::split_top(s, ' ').iter().filter(|t| !t.trim().is_empty()).map(|t| parse_len(t, fs)).collect()
    };
    match name {
        "display" => {
            st.display = match lower.as_str() {
                "none" => Display::None,
                "inline" | "contents" => Display::Inline,
                "inline-block" | "inline-flex" | "inline-grid" | "inline-table" => Display::InlineBlock,
                "list-item" => Display::ListItem,
                "table-column" | "table-column-group" => Display::None,
                _ => Display::Block,
            };
            st.table_role = match lower.as_str() {
                "table" | "inline-table" => TableRole::Table,
                "table-row-group" | "table-header-group" | "table-footer-group" => TableRole::RowGroup,
                "table-row" => TableRole::Row,
                "table-cell" => TableRole::Cell,
                "table-caption" => TableRole::Caption,
                _ => TableRole::None,
            };
            st.flex.container = lower.contains("flex");
            st.flow_root = lower == "flow-root";
            if lower.contains("grid") {
                st.grid.get_or_insert_with(Default::default);
            } else {
                st.grid = None;
            }
        }
        "flex-direction" => {
            st.flex.column = lower.starts_with("column");
            st.flex.reverse = lower.ends_with("reverse");
        }
        "flex-wrap" => st.flex.wrap = lower.starts_with("wrap"),
        "flex-flow" => {
            for t in lower.split_whitespace() {
                match t {
                    "row" | "row-reverse" | "column" | "column-reverse" => apply(st, parent, "flex-direction", t),
                    "wrap" | "wrap-reverse" | "nowrap" => apply(st, parent, "flex-wrap", t),
                    _ => {}
                }
            }
        }
        "justify-content" => {
            st.flex.justify = match lower.trim_start_matches("safe ").trim() {
                "center" => Justify::Center,
                "flex-end" | "end" | "right" => Justify::End,
                "space-between" => Justify::SpaceBetween,
                "space-around" => Justify::SpaceAround,
                "space-evenly" => Justify::SpaceEvenly,
                _ => Justify::Start,
            }
        }
        "align-items" | "align-self" | "place-items" => {
            let a = match lower.split_whitespace().next().unwrap_or("") {
                "center" => Some(AlignItems::Center),
                "flex-start" | "start" | "self-start" | "top" => Some(AlignItems::Start),
                "flex-end" | "end" | "self-end" | "bottom" => Some(AlignItems::End),
                "baseline" | "first" | "last" => Some(AlignItems::Baseline),
                "stretch" | "normal" => Some(AlignItems::Stretch),
                _ => None,
            };
            if name == "align-self" {
                st.flex.align_self = a;
            } else if let Some(a) = a {
                st.flex.align_items = a;
            }
        }
        "flex-grow" => st.flex.grow = lower.parse().unwrap_or(0.0),
        "flex-shrink" => st.flex.shrink = lower.parse().unwrap_or(1.0),
        "flex-basis" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.flex.basis = l;
            } else if lower == "content" {
                st.flex.basis = Len::Auto;
            }
        }
        "flex" => {
            let parts: Vec<&str> = lower.split_whitespace().collect();
            match parts.as_slice() {
                ["none"] => (st.flex.grow, st.flex.shrink, st.flex.basis) = (0.0, 0.0, Len::Auto),
                ["auto"] => (st.flex.grow, st.flex.shrink, st.flex.basis) = (1.0, 1.0, Len::Auto),
                ["initial"] => (st.flex.grow, st.flex.shrink, st.flex.basis) = (0.0, 1.0, Len::Auto),
                [one] => match one.parse::<f32>() {
                    Ok(g) => (st.flex.grow, st.flex.shrink, st.flex.basis) = (g, 1.0, Len::Px(0.0)),
                    Err(_) => {
                        if let Some(l) = parse_len(one, fs) {
                            (st.flex.grow, st.flex.shrink, st.flex.basis) = (1.0, 1.0, l);
                        }
                    }
                },
                [g, rest @ ..] => {
                    st.flex.grow = g.parse().unwrap_or(0.0);
                    st.flex.basis = Len::Px(0.0);
                    for r in rest {
                        if let Ok(n) = r.parse::<f32>() {
                            st.flex.shrink = n;
                        } else if let Some(l) = parse_len(r, fs) {
                            st.flex.basis = l;
                        }
                    }
                }
                _ => {}
            }
        }
        "order" => st.flex.order = lower.parse().unwrap_or(0),
        "gap" | "grid-gap" => {
            let parts: Vec<Len> = lower.split_whitespace().filter_map(|t| parse_len(t, fs)).collect();
            if let Some(&r) = parts.first() {
                st.flex.row_gap = r;
                st.flex.column_gap = *parts.get(1).unwrap_or(&r);
            }
        }
        "row-gap" | "grid-row-gap" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.flex.row_gap = l;
            }
        }
        "column-gap" | "grid-column-gap" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.flex.column_gap = l;
            }
        }
        "grid-template-columns" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).columns = v.to_string(),
        "grid-template-rows" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).rows = v.to_string(),
        "grid-template-areas" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).areas = v.to_string(),
        "grid-auto-rows" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).auto_rows = v.to_string(),
        "grid-auto-columns" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).auto_columns = v.to_string(),
        "grid-template" => {
            // "rows / columns" (area strings in this form are not supported yet)
            let g = Rc::make_mut(st.grid.get_or_insert_with(Default::default));
            if let Some((r, c)) = v.split_once('/') {
                g.rows = r.trim().to_string();
                g.columns = c.trim().to_string();
            }
        }
        "grid-column" | "grid-row" | "grid-area" => {
            let p = Rc::make_mut(st.grid_place.get_or_insert_with(Default::default));
            match name {
                "grid-column" => p.column = v.to_string(),
                "grid-row" => p.row = v.to_string(),
                _ => p.area = v.to_string(),
            }
        }
        "grid-column-start" | "grid-column-end" | "grid-row-start" | "grid-row-end" => {
            let p = Rc::make_mut(st.grid_place.get_or_insert_with(Default::default));
            let slot = if name.starts_with("grid-column") { &mut p.column } else { &mut p.row };
            let (mut a, mut b) = match slot.split_once('/') {
                Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
                None if slot.is_empty() => ("auto".to_string(), "auto".to_string()),
                None => (slot.trim().to_string(), "auto".to_string()),
            };
            if name.ends_with("start") {
                a = v.to_string();
            } else {
                b = v.to_string();
            }
            *slot = format!("{a} / {b}");
        }
        "position" => {
            st.position = match lower.as_str() {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "sticky" | "-webkit-sticky" => Position::Sticky,
                _ => Position::Static,
            }
        }
        "top" | "right" | "bottom" | "left" => {
            let i = ["top", "right", "bottom", "left"].iter().position(|x| *x == name).unwrap();
            if let Some(l) = parse_len(&lower, fs) {
                st.inset[i] = l;
            }
        }
        "inset" => {
            if let Some(v) = lens(&lower).and_then(|l| four(&l)) {
                st.inset = v;
            }
        }
        "box-sizing" => st.border_box = lower == "border-box",
        "border-spacing" => {
            if let Some(l) = lower.split_whitespace().next().and_then(|t| parse_len(t, fs)) {
                st.border_spacing = l.resolve(0.0);
            }
        }
        "border-collapse" => st.border_collapse = lower == "collapse",
        "vertical-align" => {
            st.valign = match lower.as_str() {
                "top" | "text-top" => VAlign::Top,
                "middle" => VAlign::Middle,
                "bottom" | "text-bottom" => VAlign::Bottom,
                _ => VAlign::Baseline,
            }
        }
        "float" => {
            st.float = match lower.as_str() {
                "left" | "inline-start" => Some(true),
                "right" | "inline-end" => Some(false),
                _ => None,
            }
        }
        "clear" => {
            st.clear = match lower.as_str() {
                "left" | "inline-start" => (true, false),
                "right" | "inline-end" => (false, true),
                "both" => (true, true),
                _ => (false, false),
            }
        }
        "z-index" => st.z_index = lower.parse().unwrap_or(0),
        "overflow" | "overflow-x" | "overflow-y" => {
            // `overflow: x y` — the vertical value is the last one.
            let v = if name == "overflow" { lower.split_whitespace().last() } else { lower.split_whitespace().next() };
            let v = v.unwrap_or("visible");
            if name != "overflow-x" {
                st.scrolls = matches!(v, "auto" | "scroll" | "overlay");
            }
            st.clip = st.clip && name == "overflow-y" || matches!(v, "hidden" | "clip" | "auto" | "scroll" | "overlay");
        }
        "transform" => {
            st.transform = (lower != "none").then(|| Rc::from(lower.as_str()));
        }
        "transform-origin" => st.transform_origin = Some(Rc::from(lower.as_str())),
        // Individual transform properties apply before `transform`, in this order.
        "translate" | "rotate" | "scale" if lower != "none" => {
            let args = if name == "rotate" { lower.clone() } else { lower.split_whitespace().collect::<Vec<_>>().join(", ") };
            let f = format!("{name}({args})");
            st.transform = Some(Rc::from(match &st.transform {
                Some(t) => format!("{f} {t}"),
                None => f,
            }));
        }
        "min-height" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.min_height = l;
            }
        }
        "max-height" => {
            st.max_height = if lower == "none" { Len::Auto } else { parse_len(&lower, fs).unwrap_or(Len::Auto) };
        }
        "min-width" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.min_width = l;
            }
        }
        "box-shadow" => {
            st.shadow = None;
            if lower == "none" {
                return;
            }
            // First shadow only: [inset] x y [blur [spread]] color
            let first = css::split_top(&lower, ',').into_iter().next().unwrap_or_default();
            if first.contains("inset") {
                return;
            }
            let mut nums = Vec::new();
            let mut color = Color { r: 0, g: 0, b: 0, a: 64 };
            for t in css::split_top(&first, ' ') {
                let t = t.trim();
                if t.is_empty() {
                    continue;
                }
                if let Some(c) = parse_color(t) {
                    color = c;
                } else if let Some(l) = parse_len(t, fs) {
                    nums.push(l.resolve(0.0));
                }
            }
            if nums.len() >= 2 {
                st.shadow = Some(Shadow {
                    x: nums[0],
                    y: nums[1],
                    blur: *nums.get(2).unwrap_or(&0.0),
                    spread: *nums.get(3).unwrap_or(&0.0),
                    color,
                });
            }
        }
        "visibility" => st.hidden = lower == "hidden" || lower == "collapse",
        "color" => {
            if let Some(c) = parse_color(&lower) {
                st.color = c
            }
        }
        "background-color" => {
            if let Some(c) = parse_color(&lower) {
                st.background = c;
                st.bg.color = c;
            }
        }
        "background" => {
            // Shorthand: resets everything, then reads color, image, position / size, repeat.
            st.bg = Background::default();
            st.background = Color::TRANSPARENT;
            if lower == "none" {
                return;
            }
            // Only the last layer carries a color; we draw the first image layer.
            let layers = css::split_top(v, ',');
            if let Some(first) = layers.first() {
                st.bg.image = css::split_top(first, ' ').iter().find_map(|t| parse_bg_image(t.trim()));
            }
            let last = layers.last().map(|l| l.to_string()).unwrap_or_default();
            let (pos_part, size_part) = match css::split_top(&last, '/').as_slice() {
                [a, b, ..] => (a.clone(), Some(b.clone())),
                [a] => (a.clone(), None),
                _ => (String::new(), None),
            };
            let mut pos_words = Vec::new();
            for t in css::split_top(&pos_part, ' ') {
                let t = t.trim();
                let tl = t.to_ascii_lowercase();
                if t.is_empty() || parse_bg_image(t).is_some() {
                    continue;
                }
                if let Some(c) = parse_color(&tl) {
                    st.background = c;
                    st.bg.color = c;
                } else if let Some(r) = parse_repeat(&tl) {
                    st.bg.repeat = r;
                } else if !["scroll", "fixed", "local", "border-box", "padding-box", "content-box"].contains(&tl.as_str()) {
                    pos_words.push(tl);
                }
            }
            if !pos_words.is_empty() {
                st.bg.pos = parse_bg_pos(&pos_words.join(" "), fs);
            }
            if let Some(size) = size_part {
                // The size part may be followed by repeat/color words: take the leading size tokens.
                let mut size_words = Vec::new();
                for t in css::split_top(&size, ' ') {
                    let tl = t.trim().to_ascii_lowercase();
                    if let Some(c) = parse_color(&tl) {
                        st.background = c;
                        st.bg.color = c;
                    } else if let Some(r) = parse_repeat(&tl) {
                        st.bg.repeat = r;
                    } else if !tl.is_empty() {
                        size_words.push(tl);
                    }
                }
                st.bg.size = parse_bg_size(&size_words.join(" "), fs);
            }
        }
        "background-image" => {
            st.bg.image = css::split_top(v, ',').first().and_then(|l| parse_bg_image(l.trim()));
        }
        "background-size" => st.bg.size = parse_bg_size(&lower, fs),
        "background-position" => st.bg.pos = parse_bg_pos(&lower, fs),
        "background-repeat" => {
            if let Some(r) = parse_repeat(&lower) {
                st.bg.repeat = r;
            }
        }
        "border-radius" => {
            if let Some(first) = css::split_top(&lower, ' ').first() {
                if let Some(l) = parse_len(first.split('/').next().unwrap_or(""), fs) {
                    st.radius = l;
                }
            }
        }
        "font-size" => {
            if let Some(px) = font_size_value(&lower, parent.font_size) {
                st.font_size = px.max(1.0);
            }
        }
        "font-weight" => {
            st.bold = match lower.as_str() {
                "bold" | "bolder" => true,
                "normal" | "lighter" => false,
                n => n.parse::<u32>().map(|w| w >= 600).unwrap_or(st.bold),
            }
        }
        "font-style" => st.italic = lower == "italic" || lower.starts_with("oblique"),
        "font-family" => {
            st.mono = lower.contains("mono") || lower.contains("courier") || lower.contains("consolas");
            st.family = Some(Rc::from(lower.as_str()));
        }
        "font" => {
            // e.g. "italic bold 16px/1.5 Georgia, serif"
            let mut rest = lower.as_str();
            st.italic = false;
            st.bold = false;
            while let Some((tok, after)) = rest.split_once(' ') {
                if tok.chars().next().map_or(false, |c| c.is_ascii_digit() || c == '.') && !tok.chars().all(|c| c.is_ascii_digit()) {
                    break;
                }
                match tok {
                    "italic" | "oblique" => st.italic = true,
                    "bold" | "bolder" | "600" | "700" | "800" | "900" => st.bold = true,
                    _ => {}
                }
                rest = after.trim_start();
            }
            let (size_tok, families) = rest.split_once(' ').unwrap_or((rest, ""));
            let (size, lh) = size_tok.split_once('/').map(|(a, b)| (a, Some(b))).unwrap_or((size_tok, None));
            if let Some(px) = font_size_value(size, parent.font_size) {
                st.font_size = px;
            }
            if let Some(lh) = lh {
                apply(st, parent, "line-height", lh);
            }
            st.mono = families.contains("mono") || families.contains("courier");
            if !families.trim().is_empty() {
                st.family = Some(Rc::from(families.trim()));
            }
        }
        "line-height" => {
            st.line_height = if lower == "normal" {
                1.25
            } else if let Ok(n) = lower.parse::<f32>() {
                n
            } else {
                match parse_len(&lower, fs) {
                    Some(Len::Px(px)) => px / fs.max(1.0),
                    Some(Len::Pct(p)) => p / 100.0,
                    _ => st.line_height,
                }
            }
        }
        "text-align" => {
            st.text_align = match lower.as_str() {
                "center" | "-webkit-center" | "middle" => Align::Center,
                "right" | "end" => Align::Right,
                _ => Align::Left,
            };
            st.align_logical = match lower.as_str() {
                "start" | "justify" | "initial" => 1,
                "end" => 2,
                _ => 0,
            };
        }
        "direction" => match lower.as_str() {
            "rtl" => st.rtl = true,
            "ltr" => st.rtl = false,
            _ => {}
        },
        "white-space" => st.pre = lower.starts_with("pre") && lower != "pre-line",
        "text-decoration" | "text-decoration-line" => st.underline = lower.contains("underline"),
        "list-style" | "list-style-type" => {
            if lower.contains("none") {
                st.list_style = ListStyle::None;
            } else if lower.contains("decimal") || lower.contains("roman") || lower.contains("alpha") {
                st.list_style = ListStyle::Decimal;
            } else if lower.contains("disc") || lower.contains("circle") || lower.contains("square") {
                st.list_style = ListStyle::Disc;
            }
        }
        "margin" => {
            if let Some(m) = lens(&lower).and_then(|l| four(&l)) {
                st.margin = m
            }
        }
        "padding" => {
            if let Some(p) = lens(&lower).and_then(|l| four(&l)) {
                st.padding = p
            }
        }
        "width" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.width = l
            }
        }
        "max-width" => {
            if let Some(l) = parse_len(&lower, fs) {
                st.max_width = l
            } else if lower == "none" {
                st.max_width = Len::Auto
            }
        }
        "height" => {
            if let Some(l @ Len::Px(_)) = parse_len(&lower, fs) {
                st.height = l
            }
        }
        "border" => {
            let (w, c) = border_shorthand(&lower, fs);
            st.border = [w; 4];
            if let Some(c) = c {
                st.border_color = c;
                st.border_color_set = true;
            } else {
                st.border_color_set = false;
            }
        }
        "border-width" => {
            let ws: Option<Vec<f32>> = lower.split_whitespace().map(|t| border_width_word(t, fs)).collect();
            if let Some(w) = ws.and_then(|w| four(&w)) {
                st.border = w;
            }
        }
        "border-color" => {
            if let Some(c) = lower.split_whitespace().next().and_then(parse_color) {
                st.border_color = c;
                st.border_color_set = true;
            } else if lower.contains("currentcolor") {
                st.border_color_set = false;
            }
        }
        "border-style" if lower == "none" || lower == "hidden" => st.border = [0.0; 4],
        _ => {
            if let Some(side) = name.strip_prefix("margin-").and_then(side_index) {
                if let Some(l) = parse_len(&lower, fs) {
                    st.margin[side] = l;
                }
            } else if let Some(side) = name.strip_prefix("padding-").and_then(side_index) {
                if let Some(l) = parse_len(&lower, fs) {
                    st.padding[side] = l;
                }
            } else if let Some(rest) = name.strip_prefix("border-") {
                let (side, prop) = rest.split_once('-').unwrap_or((rest, ""));
                if let Some(i) = side_index(side) {
                    match prop {
                        "" => {
                            let (w, c) = border_shorthand(&lower, fs);
                            st.border[i] = w;
                            if let Some(c) = c {
                                st.border_color = c;
                                st.border_color_set = true;
                            }
                        }
                        "width" => {
                            if let Some(w) = border_width_word(&lower, fs) {
                                st.border[i] = w;
                            }
                        }
                        "color" => {
                            if let Some(c) = parse_color(&lower) {
                                st.border_color = c;
                                st.border_color_set = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calc_and_vars() {
        assert_eq!(parse_len("calc(100% - 20px)", 16.0), Some(Len::Mix(-20.0, 100.0)));
        assert_eq!(parse_len("calc(2 * 1em + 4px)", 10.0), Some(Len::Px(24.0)));
        assert_eq!(parse_len("calc((100% - 40px) / 2)", 16.0), Some(Len::Mix(-20.0, 50.0)));
        assert_eq!(parse_len("max(10px, 2rem)", 16.0), Some(Len::Px(32.0)));
        assert_eq!(parse_len("clamp(12px, 5vw, 40px)", 16.0), Some(Len::Px(40.0)));
        let mut v = HashMap::new();
        v.insert("--gap".to_string(), "12px".to_string());
        v.insert("--brand".to_string(), "var(--blue)".to_string());
        v.insert("--blue".to_string(), "#06c".to_string());
        assert_eq!(substitute_vars("calc(var(--gap) * 2)", &v, 0), "calc(12px * 2)");
        assert_eq!(substitute_vars("var(--brand)", &v, 0), "#06c");
        assert_eq!(substitute_vars("var(--missing, red)", &v, 0), "red");
    }
}


impl Style {
    /// text-align after start/end are resolved against the writing direction.
    pub fn effective_align(&self) -> Align {
        match (self.align_logical, self.rtl) {
            (1, true) => Align::Right,
            (2, true) => Align::Left,
            _ => self.text_align,
        }
    }
}

/// Hebrew, Arabic, Syriac, Thaana, N'Ko and other right-to-left scripts.
pub fn is_rtl_char(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// Some(true) if the first strongly directional letter is right-to-left, Some(false) if
/// left-to-right, None if there is none (digits and punctuation only).
pub fn first_strong_rtl(text: &str) -> Option<bool> {
    text.chars().find_map(|c| {
        if is_rtl_char(c) && c.is_alphabetic() {
            Some(true)
        } else if c.is_alphabetic() {
            Some(false)
        } else {
            None
        }
    })
}
