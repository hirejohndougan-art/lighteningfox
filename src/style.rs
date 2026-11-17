//! Style: selector matching, the cascade and computed values.

use crate::css::{self, Combinator, Compound, Decl, Selector, Stylesheet};
use crate::dom::{Dom, NodeData, NodeId};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    /// The element under the mouse, for :hover.
    static HOVER: Cell<Option<NodeId>> = const { Cell::new(None) };
    /// Styles reused from the previous pass (for measuring; see restyle.rs).
    pub static REUSED: Cell<usize> = const { Cell::new(0) };
    /// The element the mouse button is held down on, for :active.
    static ACTIVE: Cell<Option<NodeId>> = const { Cell::new(None) };
    /// Whether the last styled page has :active rules.
    static ACTIVE_RULES: Cell<bool> = const { Cell::new(false) };
    /// The page's address and the visited addresses, for :visited.
    static VISITED: std::cell::RefCell<Visited> = std::cell::RefCell::new(Default::default());
    /// Set on the threads that match selectors in parallel: the focused element as the
    /// window's thread sees it (layout keeps focus in its own thread-local).
    static FOCUS_SNAPSHOT: Cell<Option<Option<NodeId>>> = const { Cell::new(None) };
}

/// The page's address and the visited addresses (shared, so parallel matching threads can
/// have it without copying the history).
type Visited = std::sync::Arc<(String, std::collections::HashSet<String>)>;

fn focused() -> Option<NodeId> {
    FOCUS_SNAPSHOT.with(Cell::get).unwrap_or_else(crate::layout::focus)
}

/// What selector matching reads besides the DOM and the stylesheets (:hover, :active,
/// :focus, :visited), taken on the window's thread and set up on the threads that match
/// selectors in parallel.
#[derive(Clone)]
pub(crate) struct MatchState {
    hover: Option<NodeId>,
    active: Option<NodeId>,
    focus: Option<NodeId>,
    visited: Visited,
}

impl MatchState {
    pub(crate) fn capture() -> Self {
        MatchState { hover: HOVER.with(Cell::get), active: ACTIVE.with(Cell::get), focus: focused(), visited: VISITED.with(|v| v.borrow().clone()) }
    }

    pub(crate) fn install(&self) {
        HOVER.with(|h| h.set(self.hover));
        ACTIVE.with(|h| h.set(self.active));
        FOCUS_SNAPSHOT.with(|f| f.set(Some(self.focus)));
        VISITED.with(|v| *v.borrow_mut() = self.visited.clone());
    }
}

pub fn set_active(node: Option<NodeId>) {
    ACTIVE.with(|h| h.set(node));
}

pub fn uses_active() -> bool {
    ACTIVE_RULES.with(Cell::get)
}

/// The page :visited links are resolved against, and the addresses in history.
pub fn set_visited(base: &str, urls: std::collections::HashSet<String>) {
    crate::restyle::visited_changed();
    VISITED.with(|v| *v.borrow_mut() = std::sync::Arc::new((base.to_string(), urls)));
}

/// Run `f` as if no link had been visited (for styles that scripts can read).
pub fn without_visited<T>(f: impl FnOnce() -> T) -> T {
    let saved = VISITED.with(|v| std::mem::take(&mut *v.borrow_mut()));
    crate::restyle::visited_changed();
    let out = f();
    VISITED.with(|v| *v.borrow_mut() = saved);
    crate::restyle::visited_changed();
    out
}

fn is_visited(href: &str) -> bool {
    VISITED.with(|v| {
        let v = v.borrow();
        !v.1.is_empty() && v.1.contains(&crate::net::resolve(&v.0, href))
    })
}

/// Properties a :visited rule may change: only colors, so pages can't measure which links
/// were visited from layout.
fn visited_safe(name: &str) -> bool {
    matches!(name, "color" | "background-color" | "border-color" | "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" | "outline-color" | "text-decoration-color" | "column-rule-color" | "fill" | "stroke")
}

fn uses_visited(sel: &Selector) -> bool {
    sel.parts.iter().any(|p| p.pseudo.iter().any(|x| x.contains("visited")))
}

pub fn set_hover(node: Option<NodeId>) {
    HOVER.with(|h| h.set(node));
}

/// Does any rule in these sheets use :hover? (Only then is it worth restyling on mouse moves.)
pub fn uses_hover(sheets: &[Stylesheet]) -> bool {
    sheets.iter().any(|sh| sh.rules.iter().any(|r| r.selectors.iter().any(|s| s.parts.iter().any(|p| p.pseudo.iter().any(|x| x == "hover")))))
}


thread_local! {
    /// The root element's font size, which `rem` is relative to (16px until it's styled).
    static ROOT_FONT: std::cell::Cell<f32> = const { std::cell::Cell::new(16.0) };
}

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
    /// linear-, radial- or conic-gradient, maybe repeating (gradient.rs).
    Gradient(Box<crate::gradient::Gradient>),
}

/// object-fit: fill (stretch, the default), contain, cover, none, scale-down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
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
    /// Image layers below the first (`background-image: a, b, c`), top to bottom; and the
    /// full size/position/repeat lists when more than one was given (background.rs).
    pub more: Vec<BgImage>,
    pub sizes: Vec<BgSize>,
    pub positions: Vec<(Len, Len)>,
    pub repeats: Vec<(bool, bool)>,
}

impl Default for Background {
    fn default() -> Self {
        Background {
            color: Color::TRANSPARENT,
            image: None,
            size: BgSize::Auto,
            pos: (Len::Pct(0.0), Len::Pct(0.0)),
            repeat: (true, true),
            more: Vec::new(),
            sizes: Vec::new(),
            positions: Vec::new(),
            repeats: Vec::new(),
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

/// A multi-column container's settings.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MultiCol {
    pub count: Option<u32>,
    pub width: Option<f32>,
    /// column-rule: width (0 = none), color (None = the text color).
    pub rule_width: f32,
    pub rule_color: Option<Color>,
    pub rule_dashed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    Sub,
    Super,
    TextTop,
    TextBottom,
    /// A length or percentage (of the element's line height) to raise the baseline by.
    Shift(Len),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridTemplate {
    pub columns: String,
    pub rows: String,
    pub areas: String,
    pub auto_rows: String,
    pub auto_columns: String,
    /// grid-auto-flow: `row` (default), `column`, with `dense`.
    pub auto_flow: String,
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
    /// Grid: justify-items (container) and justify-self (item); None = normal (stretch).
    pub justify_items: Option<AlignItems>,
    pub justify_self: Option<AlignItems>,
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
            justify_items: None,
            justify_self: None,
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
    /// The numeric font-weight (100–900); `bold` is whether it's 600 or more. See `font_weight`.
    pub weight: u16,
    pub italic: bool,
    pub mono: bool,
    /// The `font-family` list, lower-case (web fonts are matched against it).
    pub family: Option<Rc<str>>,
    pub underline: bool,
    pub line_through: bool,
    pub overline: bool,
    /// text-decoration-color (else the text color).
    pub deco_color: Option<Color>,
    /// text-transform: 0 none, 1 uppercase, 2 lowercase, 3 capitalize.
    pub text_transform: u8,
    /// Spacing, wrapping, hyphenation, ellipsis and decoration style (text.rs).
    pub text: crate::text::TextStyle,
    /// text-indent of a block's first line.
    pub text_indent: Len,
    /// The mouse pointer over it: 0 auto, 1 pointer (hand), 2 text, 3 default arrow,
    /// 4 not-allowed/wait (arrow), 5 move/grab.
    pub cursor: u8,
    pub text_align: Align,
    /// text-align: start / end (0 = neither), which follow the writing direction.
    pub align_logical: u8,
    /// Right-to-left writing direction (`direction: rtl` or `dir="rtl"`).
    pub rtl: bool,
    /// writing-mode, text-orientation, text-combine-upright (vertical.rs).
    pub vertical: crate::vertical::Vertical,
    pub line_height: f32,
    pub pre: bool,
    pub list_style: ListStyle,
    /// The `list-style-type` keyword (`disc`, `decimal`, `lower-roman`…) for list markers.
    pub list_type: &'static str,
    /// `list-style-position: inside`: the marker sits in the first line of text.
    pub list_inside: bool,
    /// `display: contents`: the element makes no box; its children take its place.
    pub contents: bool,
    /// The used `color-scheme` is dark (`light-dark()` picks its second color). Inherited.
    pub dark_scheme: bool,
    pub hidden: bool,
    /// The product of this element's and its ancestors' opacity (applied to its colors).
    pub fade: f32,
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
    /// opacity (0–1); not inherited — layout fades everything the element draws.
    pub opacity: f32,
    /// Border line style: 0 solid, 1 dashed, 2 dotted, 3 double (one style for all sides).
    pub border_line: u8,
    /// aspect-ratio (width / height), when set.
    pub aspect_ratio: Option<f32>,
    /// object-fit for images: how the picture fills its box.
    pub object_fit: ObjectFit,
    /// object-position (x, y): where the picture sits in its box; 50% 50% by default.
    pub object_position: (Len, Len),
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
    /// `filter` and `backdrop-filter` (not inherited).
    pub filter: Option<Rc<[crate::effects::Filter]>>,
    pub backdrop: Option<Rc<[crate::effects::Filter]>>,
    /// `text-shadow` (inherited).
    pub text_shadow: Option<Rc<[crate::effects::TextShadow]>>,
    /// `animation-*` and `transition-*` (not inherited).
    pub anim: Option<Rc<crate::anim::AnimSpec>>,
    /// box-sizing: border-box — width/height include padding and border.
    pub border_box: bool,
    /// float: Some(true) = left, Some(false) = right.
    pub float: Option<bool>,
    /// clear: (left, right)
    pub clear: (bool, bool),
    /// display: flow-root (starts a new block formatting context, contains floats)
    pub flow_root: bool,
    /// Multi-column layout (column-count, column-width, column-rule), when set.
    pub multicol: Option<MultiCol>,
    /// column-gap or gap was given (multi-column's default gap is 1em, not 0).
    pub column_gap_set: bool,
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
    /// `content` as written (for ::before, ::after and ::marker boxes).
    pub content: Option<Rc<str>>,
    /// `counter-reset`, `counter-set` and `counter-increment` as written.
    pub counters: Option<Rc<[(u8, Rc<str>)]>>,
    /// Styles of this element's ::before, ::after, ::marker and ::placeholder.
    pub pseudo: Option<Rc<Pseudos>>,
    /// A query container (`container-type`: 1 inline-size, 2 size) and its names.
    pub container: Option<(u8, Rc<str>)>,
    /// `appearance: none`: form controls drop their built-in decorations.
    pub appearance_none: bool,
    /// Colors given for single sides (top, right, bottom, left); other sides use `border_color`.
    pub border_colors: Option<[Option<Color>; 4]>,
    /// On SVG elements: the page's winning declarations of SVG properties (fill, stroke…),
    /// lowest priority first, handed to the SVG renderer by `svg_inline.rs`.
    pub svg_decls: Option<Rc<Vec<(String, String)>>>,
}

/// The pseudo-element styles an element has rules for.
#[derive(Debug, Clone, Default)]
pub struct Pseudos {
    pub before: Option<Rc<Style>>,
    pub after: Option<Rc<Style>>,
    pub marker: Option<Rc<Style>>,
    pub placeholder: Option<Rc<Style>>,
}

/// Which pseudo-element a selector ends in: 0 ::before, 1 ::after, 2 ::marker, 3 ::placeholder.
pub fn pseudo_kind(name: &str) -> Option<usize> {
    match name {
        "::before" => Some(0),
        "::after" => Some(1),
        "::marker" => Some(2),
        "::placeholder" | "::-webkit-input-placeholder" | "::-moz-placeholder" | "-ms-input-placeholder" | "::-ms-input-placeholder" => Some(3),
        _ => None,
    }
}

impl Default for Style {
    fn default() -> Self {
        Style {
            display: Display::Inline,
            color: Color::BLACK,
            font_size: 16.0,
            bold: false,
            weight: 400,
            italic: false,
            mono: false,
            family: None,
            underline: false,
            line_through: false,
            overline: false,
            deco_color: None,
            text_transform: 0,
            text: crate::text::TextStyle::default(),
            text_indent: Len::Px(0.0),
            cursor: 0,
            text_align: Align::Left,
            align_logical: 1,
            rtl: false,
            vertical: crate::vertical::Vertical::default(),
            line_height: 1.25,
            pre: false,
            list_style: ListStyle::Disc,
            list_type: "disc",
            list_inside: false,
            contents: false,
            hidden: false,
            vars: Rc::default(),
            background: Color::TRANSPARENT,
            bg: Background::default(),
            radius: Len::Px(0.0),
            flex: Flex::default(),
            position: Position::Static,
            inset: [Len::Auto; 4],
            z_index: 0,
            opacity: 1.0,
            border_line: 0,
            aspect_ratio: None,
            object_fit: ObjectFit::Fill,
            object_position: (Len::Pct(50.0), Len::Pct(50.0)),
            fade: 1.0,
            clip: false,
            scrolls: false,
            shadow: None,
            min_width: Len::Auto,
            min_height: Len::Auto,
            max_height: Len::Auto,
            transform: None,
            transform_origin: None,
            filter: None,
            backdrop: None,
            text_shadow: None,
            anim: None,
            border_box: false,
            float: None,
            clear: (false, false),
            flow_root: false,
            multicol: None,
            column_gap_set: false,
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
            content: None,
            counters: None,
            pseudo: None,
            container: None,
            appearance_none: false,
            border_colors: None,
            svg_decls: None,
            dark_scheme: false,
        }
    }
}

impl Style {
    /// The font-weight to match fonts with: `weight`, kept in step with `bold` (which some
    /// code sets on its own).
    pub fn font_weight(&self) -> u16 {
        match (self.bold, self.weight >= 600) {
            (true, false) => 700,
            (false, true) => 400,
            _ => self.weight,
        }
    }

    pub(crate) fn inherit(parent: &Style) -> Style {
        Style {
            color: parent.color,
            font_size: parent.font_size,
            bold: parent.bold,
            weight: parent.weight,
            italic: parent.italic,
            mono: parent.mono,
            family: parent.family.clone(),
            underline: parent.underline,
            line_through: parent.line_through,
            overline: parent.overline,
            deco_color: parent.deco_color,
            text_transform: parent.text_transform,
            text: parent.text.inherit(),
            text_indent: parent.text_indent,
            cursor: parent.cursor,
            text_align: parent.text_align,
            align_logical: parent.align_logical,
            rtl: parent.rtl,
            vertical: parent.vertical.inherit(),
            line_height: parent.line_height,
            pre: parent.pre,
            list_style: parent.list_style,
            list_type: parent.list_type,
            list_inside: parent.list_inside,
            hidden: parent.hidden,
            fade: parent.fade,
            text_shadow: parent.text_shadow.clone(),
            vars: parent.vars.clone(),
            border_spacing: parent.border_spacing,
            border_collapse: parent.border_collapse,
            dark_scheme: parent.dark_scheme,
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
foreignObject { display: block; }
audio:not([controls]) { display: none; }
thead, tbody, tfoot { display: table-row-group; }
tr { display: table-row; }
td, th { display: table-cell; vertical-align: middle; }
caption { display: table-caption; text-align: center; }
colgroup, col { display: none; }
th { text-align: center; }
li { display: list-item; }
head, script, style, title, meta, link, noscript, template, base, datalist, dialog { display: none; }
[hidden] { display: none; }
slot { display: contents; }
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
a:visited { color: #551a8b; text-decoration: underline; }
a:link:active, a:visited:active { color: #ee0000; }
u, ins { text-decoration: underline; }
small { font-size: 0.83em; }
sub { vertical-align: sub; font-size: 0.83em; }
sup { vertical-align: super; font-size: 0.83em; }
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
    /// Cascade layer rank (see `cascade::LayerOrder::key`).
    layer: u32,
    spec: (u32, u32, u32),
    order: usize,
    decl: &'a Decl,
}

/// The rules that matched one element, in cascade order of appearance (`order` is the next
/// free position, for its inline style).
#[derive(Default)]
pub(crate) struct Matches<'a> {
    all: Vec<Matched<'a>>,
    /// ::before, ::after, ::marker, ::placeholder.
    pseudo: [Vec<Matched<'a>>; 4],
    order: usize,
}

/// Everything selector matching needs for one styling pass, shared (read-only) by the
/// threads that match elements in parallel.
pub(crate) struct MatchCx<'a> {
    ua: &'a Stylesheet,
    sheets: &'a [Stylesheet],
    scoped: &'a HashMap<NodeId, (Stylesheet, Stylesheet)>,
    /// Per stylesheet (by address): rules whose selectors end in a pseudo-element, rules filed
    /// by id/class/tag (see selmatch.rs), and whether each rule's @media matches the window.
    pseudo_index: HashMap<usize, Vec<PseudoRule>>,
    rule_index: HashMap<usize, crate::selmatch::SheetIndex>,
    media: HashMap<usize, Vec<bool>>,
    layers: crate::cascade::LayerOrder,
}

impl<'a> MatchCx<'a> {
    fn new(ua: &'a Stylesheet, sheets: &'a [Stylesheet], scoped: &'a HashMap<NodeId, (Stylesheet, Stylesheet)>) -> Self {
        let mut cx = MatchCx {
            ua,
            sheets,
            scoped,
            pseudo_index: HashMap::new(),
            rule_index: HashMap::new(),
            media: HashMap::new(),
            layers: crate::cascade::LayerOrder::new(std::iter::once(ua).chain(sheets.iter())),
        };
        // Each @media condition is checked once per pass, not once per element.
        let mut seen: HashMap<&str, bool> = HashMap::new();
        for sheet in std::iter::once(ua).chain(sheets.iter()).chain(scoped.values().flat_map(|(a, b)| [a, b])) {
            let key = sheet as *const Stylesheet as usize;
            cx.pseudo_index.insert(key, pseudo_rules(sheet));
            cx.rule_index.insert(key, crate::selmatch::SheetIndex::new(sheet));
            let media = sheet.rules.iter().map(|r| r.media.as_deref().is_none_or(|m| *seen.entry(m).or_insert_with(|| css::media_matches(m)))).collect();
            cx.media.insert(key, media);
        }
        cx
    }

    fn media_ok(&self, sheet: &Stylesheet, rule: usize) -> bool {
        match self.media.get(&(sheet as *const Stylesheet as usize)) {
            Some(m) => m[rule],
            None => sheet.rules[rule].media.as_deref().is_none_or(css::media_matches),
        }
    }

    /// Match every rule against element `id`. `styles` = the styles computed so far, for
    /// @container conditions; without them (matching ahead, in parallel) an element that a
    /// rule's @container condition applies to gives None and is matched again in order.
    pub(crate) fn match_element(&self, dom: &Dom, id: NodeId, bloom: &crate::selmatch::Bloom, styles: Option<&[Rc<Style>]>, candidates: &mut Vec<u32>) -> Option<Matches<'a>> {
        let mut matched: Vec<Matched<'a>> = Vec::new();
        let mut order = 0;
        // The page's stylesheets style the document; a shadow tree is styled only by its
        // own (plus the browser's), and a host also by its shadow tree's `:host` rules.
        let page: Vec<&'a Stylesheet> = match dom.render_scope.get(&id) {
            Some(root) => self.scoped.get(root).map(|(_, inner)| vec![inner]).unwrap_or_default(),
            None => {
                let mut v: Vec<&'a Stylesheet> = self.sheets.iter().collect();
                if let Some((host_rules, _)) = dom.attr(id, "__shadowhost").and_then(|r| r.parse::<NodeId>().ok()).and_then(|r| self.scoped.get(&r)) {
                    // Page rules win over :host rules: put those first.
                    v.insert(0, host_rules);
                }
                v
            }
        };
        let container_ok = |c: &str| styles.map(|st| crate::cascade::matches(dom, st, id, c));
        let mut pmatched: [Vec<Matched<'a>>; 4] = Default::default();
        for (origin, sheet) in std::iter::once(self.ua).chain(page).enumerate() {
            let origin = if origin == 0 { 0 } else { 1 };
            for pr in self.pseudo_index.get(&(sheet as *const Stylesheet as usize)).map(|v| v.as_slice()).unwrap_or(&[]) {
                let rule = &sheet.rules[pr.rule];
                if !self.media_ok(sheet, pr.rule) || !matches(dom, id, &pr.sel) {
                    continue;
                }
                if let Some(c) = rule.container.as_deref() {
                    if !container_ok(c)? {
                        continue;
                    }
                }
                for d in &rule.decls {
                    let layer = self.layers.key(rule.layer.as_ref(), d.important);
                    pmatched[pr.kind].push(Matched { important: d.important, origin, layer, spec: pr.spec, order, decl: d });
                    order += 1;
                }
            }
            match self.rule_index.get(&(sheet as *const Stylesheet as usize)) {
                Some(ix) => ix.candidates(dom, id, candidates),
                None => {
                    candidates.clear();
                    candidates.extend(0..sheet.rules.len() as u32);
                }
            }
            for &ri in candidates.iter() {
                let rule = &sheet.rules[ri as usize];
                if !self.media_ok(sheet, ri as usize) {
                    continue;
                }
                let mut best: Option<(u32, u32, u32)> = None;
                let mut only_visited = true;
                for s in rule.selectors.iter().filter(|s| crate::selmatch::may_match(s, bloom) && matches(dom, id, s)) {
                    best = best.max(Some(s.specificity()));
                    only_visited &= uses_visited(s);
                }
                if best.is_some()
                    && let Some(c) = rule.container.as_deref()
                    && !container_ok(c)?
                {
                    continue;
                }
                if let Some(spec) = best {
                    for d in rule.decls.iter().filter(|d| !only_visited || visited_safe(&d.name)) {
                        let layer = self.layers.key(rule.layer.as_ref(), d.important);
                        matched.push(Matched { important: d.important, origin, layer, spec, order, decl: d });
                        order += 1;
                    }
                }
            }
        }
        Some(Matches { all: matched, pseudo: pmatched, order })
    }
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
                let sel = rule.split('{').next().unwrap_or("");
                if !sel.contains(":host") || sel.trim_start().starts_with('@') {
                    inner.push_str(&rule);
                } else {
                    // `:host(…)` as the subject styles the host; as an ancestor
                    // (`:host([open]) .panel`) it styles nodes inside, which see the host as
                    // their ancestor in the flat tree.
                    let body = &rule[sel.len()..];
                    let (mut on_host, mut inside) = (Vec::new(), Vec::new());
                    for one in split_top_level(sel, ',') {
                        // The subject: what follows the last combinator outside brackets.
                        let one_t = one.trim();
                        let mut depth = 0i32;
                        let mut cut = 0;
                        for (i, c) in one_t.char_indices() {
                            match c {
                                '(' | '[' => depth += 1,
                                ')' | ']' => depth -= 1,
                                ' ' | '>' | '+' | '~' if depth == 0 => cut = i + 1,
                                _ => {}
                            }
                        }
                        let last = &one_t[cut..];
                        if last.contains(":host") { on_host.push(one) } else { inside.push(one) }
                    }
                    if !on_host.is_empty() {
                        host.push_str(&rewrite_host(&format!("{}{body}", on_host.join(",")), root));
                    }
                    if !inside.is_empty() {
                        inner.push_str(&rewrite_host(&format!("{}{body}", inside.join(",")), root));
                    }
                }
                host.push('\n');
                inner.push('\n');
            }
            (root, (css::parse(&host), css::parse(&inner)))
        })
        .collect()
}

/// Split `s` at `sep` outside parentheses and brackets (selector lists, compounds).
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.retain(|p| !p.trim().is_empty());
    out
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
    css::forget_media_answers();
    ROOT_FONT.with(|r| r.set(16.0));
    let ua = css::parse(UA_CSS);
    let scoped = if dom.render_scope.is_empty() { HashMap::new() } else { shadow_sheets(dom) };
    let min_font = crate::settings::get().min_font_size as f32;
    let hidden = Rc::new(Style { display: Display::None, ..Style::default() });
    let mut styles: Vec<Rc<Style>> = vec![hidden; dom.nodes.len()];
    // Restyle only what changed since the last pass (see restyle.rs).
    let feats = crate::restyle::features(sheets.iter());
    let reuse = crate::restyle::enabled() && !feats.full && dom.render_scope.is_empty();
    let mut cache = if reuse { crate::restyle::take(crate::restyle::global_key(sheets, crate::restyle::visited_gen())) } else { crate::restyle::Cache::default() };
    let states = crate::restyle::States::new(dom, HOVER.with(|h| h.get()), ACTIVE.with(|h| h.get()), crate::layout::focus());
    let mut next: Vec<Option<crate::restyle::Entry>> = Vec::new();
    if reuse {
        next.resize_with(dom.nodes.len(), || None);
    }
    styles[0] = cache.root.clone().unwrap_or_else(|| Rc::new(Style { display: Display::Block, ..Style::default() }));
    let mut shared: HashMap<(usize, Vec<usize>, String), Rc<Style>> = HashMap::new();
    let mut blooms: Vec<crate::selmatch::Bloom> = vec![[0; 4]; dom.nodes.len()];
    let mut candidates: Vec<u32> = Vec::new();
    let cx = MatchCx::new(&ua, sheets, &scoped);
    ACTIVE_RULES.with(|a| a.set(sheets.iter().any(|sh| sh.rules.iter().any(|r| r.selectors.iter().any(|s| s.parts.iter().any(|p| p.pseudo.iter().any(|x| x == "active")))))));
    // Selector matching doesn't depend on the parent's computed style: on big pages it runs
    // for every element at once across the CPU cores first (see parallel.rs), and the
    // in-order pass below only cascades.
    let mut sigs: Vec<u64> = Vec::new();
    let mut pre: Vec<Option<Matches>> = Vec::new();
    if crate::parallel::worth_it(dom.nodes.len()) {
        let mut todo: Vec<NodeId> = Vec::new();
        if reuse {
            sigs = vec![0; dom.nodes.len()];
        }
        let mut stack: Vec<NodeId> = dom.nodes[0].children.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            stack.extend(dom.nodes[id].children.iter().rev().copied());
            let parent = dom.nodes[id].parent.unwrap_or(0);
            blooms[id] = crate::selmatch::child_bloom(dom, parent, &blooms[parent]);
            if !matches!(dom.nodes[id].data, NodeData::Element { .. }) {
                continue;
            }
            if reuse {
                sigs[id] = crate::restyle::signature(dom, id, &feats, &states);
                // Probably reused as it was: match it later only if it turns out it isn't.
                if cache.nodes.get(id).is_some_and(|o| o.as_ref().is_some_and(|o| o.sig == sigs[id])) {
                    continue;
                }
            }
            todo.push(id);
        }
        pre = crate::parallel::match_all(&cx, dom, &todo, &blooms);
    }
    crate::anim::begin_pass(sheets);
    // Walk the tree parent-first (scripts can move nodes, so ids aren't in tree order).
    let mut stack: Vec<NodeId> = dom.nodes[0].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        stack.extend(dom.nodes[id].children.iter().rev().copied());
        let parent = dom.nodes[id].parent.unwrap_or(0);
        if pre.is_empty() {
            blooms[id] = crate::selmatch::child_bloom(dom, parent, &blooms[parent]);
        }
        let bloom = blooms[id];
        let parent_style = styles[parent].clone();
        // rem follows the root element's font size; the root is styled before what's inside.
        if parent != 0 && dom.nodes[parent].parent == Some(0) {
            ROOT_FONT.with(|r| r.set(parent_style.font_size));
        }
        let NodeData::Element { tag, attrs } = &dom.nodes[id].data else {
            // Text inherits everything it uses from its parent.
            styles[id] = parent_style;
            continue;
        };
        let sig = match sigs.get(id) {
            Some(&sig) => sig,
            None if reuse => crate::restyle::signature(dom, id, &feats, &states),
            None => 0,
        };
        if reuse {
            if let Some(Some(old)) = cache.nodes.get_mut(id) {
                if old.sig == sig && Rc::ptr_eq(&old.parent, &parent_style) && old.style.anim.is_none() && !crate::anim::has_state(id) {
                    styles[id] = old.style.clone();
                    next[id] = cache.nodes[id].take();
                    REUSED.with(|r| r.set(r.get() + 1));
                    continue;
                }
            }
        }
        let found = match pre.get_mut(id).and_then(Option::take) {
            Some(m) => m,
            None => cx.match_element(dom, id, &bloom, Some(&styles), &mut candidates).unwrap_or_default(),
        };
        let Matches { all: mut matched, pseudo: pmatched, mut order } = found;
        // Same parent style + same rules + same attributes that affect style = same result.
        let attr_key: String = attrs
            .iter()
            .filter(|(k, _)| matches!(k.as_str(), "style" | "bgcolor" | "color" | "align" | "valign" | "width" | "border" | "cellpadding" | "cellspacing" | "hidden" | "dir"))
            .map(|(k, v)| format!("{k}={v};"))
            .collect::<String>()
            + tag
            // dir="auto" depends on the element's own text.
            + &if dom.attr(id, "dir").is_some_and(|d| d.eq_ignore_ascii_case("auto")) { format!("#{id}") } else { String::new() };
        let mut decl_key: Vec<usize> = matched.iter().map(|m| m.decl as *const Decl as usize).collect();
        for (k, pm) in pmatched.iter().enumerate() {
            if !pm.is_empty() {
                decl_key.push(k);
                decl_key.extend(pm.iter().map(|m| m.decl as *const Decl as usize));
            }
        }
        let key = (Rc::as_ptr(&parent_style) as usize, decl_key, attr_key);
        // Animated elements each move on their own clock: never share their style.
        let animated = crate::anim::has_state(id) || matched.iter().any(|m| crate::anim::is_anim_prop(&m.decl.name)) || dom.attr(id, "style").is_some_and(|s| s.contains("animation") || s.contains("transition"));
        if let Some(s) = shared.get(&key).filter(|_| !animated) {
            if reuse {
                next[id] = Some(crate::restyle::Entry { sig, parent: parent_style.clone(), style: s.clone() });
            }
            styles[id] = s.clone();
            continue;
        }
        let mut st = Style::inherit(&parent_style);
        let inline = dom.attr(id, "style").map(css::parse_declarations).unwrap_or_default();
        for d in &inline {
            matched.push(Matched { important: d.important, origin: 2, layer: cx.layers.key(None, d.important), spec: (9, 9, 9), order, decl: d });
            order += 1;
        }
        matched.sort_by_key(|m| (m.important, m.origin, m.layer, m.spec, m.order));
        // Custom properties first, so normal properties can use them via var(). They may refer
        // to each other in any order (`--b: var(--a)` before `--a` is declared), so all of this
        // element's are gathered first and then resolved against each other and the inherited
        // ones. `initial` makes one unset (var() then takes its fallback).
        let mut declared: Vec<(&str, &str)> = Vec::new();
        for m in matched.iter().filter(|m| m.decl.name.starts_with("--")) {
            declared.retain(|(n, _)| *n != m.decl.name.as_str());
            declared.push((m.decl.name.as_str(), m.decl.value.as_str()));
        }
        if !declared.is_empty() {
            let mut raw: HashMap<String, String> = (*st.vars).clone();
            for (n, v) in &declared {
                if v.trim().eq_ignore_ascii_case("initial") {
                    raw.remove(*n);
                } else {
                    raw.insert(n.to_string(), v.to_string());
                }
            }
            let vars = Rc::make_mut(&mut st.vars);
            for (n, _) in &declared {
                match raw.get(*n) {
                    Some(v) => {
                        let value = substitute_vars(v, &raw, 0);
                        vars.insert(n.to_string(), value);
                    }
                    None => {
                        vars.remove(*n);
                    }
                }
            }
        }
        // HTML attributes like bgcolor and cellpadding beat the browser defaults
        // but lose to the page's own CSS, so they go in between.
        // color-scheme first: light-dark() in any other property depends on it.
        if let Some(m) = matched.iter().filter(|m| m.decl.name == "color-scheme").next_back() {
            let v = substitute_vars(&m.decl.value, &st.vars, 0);
            if let Some(d) = scheme_is_dark(&v) {
                st.dark_scheme = d;
            }
        }
        DARK_SCHEME.with(|d| d.set(st.dark_scheme));
        let mut hinted = false;
        for m in matched.iter().filter(|m| !m.decl.name.starts_with("--")) {
            if !hinted && (m.origin > 0 || m.important) {
                presentational_hints(dom, id, &mut st, &parent_style);
                hinted = true;
            }
            let value = if m.decl.value.contains("var(") { substitute_vars(&m.decl.value, &st.vars, 0) } else { m.decl.value.clone() };
            let kw = value.trim().to_ascii_lowercase();
            if css::is_wide_keyword(&kw) {
                // `revert` in the page's CSS goes back to the browser's own rule for it.
                let ua_value = (kw.starts_with("revert") && m.origin > 0)
                    .then(|| matched.iter().filter(|u| u.origin == 0 && !u.important && u.decl.name == m.decl.name).next_back().map(|u| u.decl.value.clone()))
                    .flatten();
                match ua_value {
                    Some(v) => apply(&mut st, &parent_style, &m.decl.name, &v),
                    None => crate::cascade::apply_keyword(&mut st, &parent_style, &m.decl.name, if kw.starts_with("revert") { "unset" } else { &kw }),
                }
                continue;
            }
            apply(&mut st, &parent_style, &m.decl.name, &value);
        }
        if !hinted {
            presentational_hints(dom, id, &mut st, &parent_style);
        }
        if dom.ns_of(id) == crate::dom::Ns::Svg {
            st.svg_decls = crate::svg_inline::declared(matched.iter().filter(|m| m.origin > 0).map(|m| m.decl), &st.vars);
        }
        // Transitions and animations override the cascaded values.
        if st.anim.is_some() || crate::anim::has_state(id) {
            let mut won: Vec<(String, String)> = Vec::new();
            for m in matched.iter().filter(|m| !m.decl.name.starts_with("--")) {
                let value = if m.decl.value.contains("var(") { substitute_vars(&m.decl.value, &st.vars, 0) } else { m.decl.value.clone() };
                won.retain(|(n, _)| *n != m.decl.name);
                won.push((m.decl.name.clone(), value.trim().to_string()));
            }
            let svg = dom.ns_of(id) == crate::dom::Ns::Svg;
            if svg {
                // SVG presentation attributes (fill="red") are what a transition starts from.
                won.splice(0..0, crate::svg_inline::presentation_attrs(dom, id));
            }
            let mut svg_anim: Vec<(String, String)> = Vec::new();
            for (name, value) in crate::anim::animate(id, st.anim.clone().as_deref(), &won) {
                let value = if value.contains("var(") { substitute_vars(&value, &st.vars, 0) } else { value };
                if svg && crate::svg_inline::is_svg_prop(&name) {
                    svg_anim.push((name.clone(), value.clone()));
                }
                apply(&mut st, &parent_style, &name, &value);
            }
            if !svg_anim.is_empty() {
                st.svg_decls = crate::svg_inline::with_animated(st.svg_decls.take(), svg_anim);
            }
        }
        // The reader's minimum font size (Settings → Appearance).
        if min_font > 0.0 && st.font_size < min_font {
            st.font_size = min_font;
        }
        // opacity: approximated by fading the element's colors (and, through `fade`,
        // its descendants'). An inherited text color already carries the parent's fade.
        let own = st.opacity;
        st.fade = parent_style.fade * own;
        if st.fade < 1.0 {
            let mul = |c: &mut Color, f: f32| c.a = (c.a as f32 * f).round() as u8;
            let inherited = st.color == parent_style.color;
            mul(&mut st.color, if inherited { own } else { st.fade });
            let f = st.fade;
            mul(&mut st.background, f);
            mul(&mut st.bg.color, f);
            if st.border_color_set {
                mul(&mut st.border_color, f);
            }
            if let Some(sides) = st.border_colors.as_mut() {
                for c in sides.iter_mut().flatten().filter(|c| !(c.a == 0 && c.r == 0 && c.g == 0 && c.b == 1)) {
                    mul(c, f);
                }
            }
            if st.deco_color != parent_style.deco_color {
                if let Some(c) = st.deco_color.as_mut() {
                    mul(c, f);
                }
            }
        }
        // Borders without a color of their own use the text color.
        if !st.border_color_set {
            st.border_color = st.color;
        }
        if pmatched.iter().any(|p| !p.is_empty()) {
            let mut ps = Pseudos::default();
            for (k, mut pm) in pmatched.into_iter().enumerate() {
                if pm.is_empty() {
                    continue;
                }
                pm.sort_by_key(|m| (m.important, m.origin, m.layer, m.spec, m.order));
                let p = cascade_pseudo(&st, &pm);
                let slot = match k {
                    0 => &mut ps.before,
                    1 => &mut ps.after,
                    2 => &mut ps.marker,
                    _ => &mut ps.placeholder,
                };
                *slot = Some(Rc::new(p));
            }
            st.pseudo = Some(Rc::new(ps));
        }
        let st = Rc::new(st);
        if !animated {
            shared.insert(key, st.clone());
        }
        if reuse {
            next[id] = Some(crate::restyle::Entry { sig, parent: parent_style.clone(), style: st.clone() });
        }
        styles[id] = st;
    }
    crate::anim::end_pass();
    if reuse {
        cache.root = Some(styles[0].clone());
        cache.nodes = next;
        crate::restyle::store(cache);
    }
    if crate::settings::get().high_contrast {
        crate::contrast::force_colors(dom, &mut styles);
    }
    styles
}

/// A rule with a selector that ends in a pseudo-element: the selector without it picks
/// the originating element.
struct PseudoRule {
    rule: usize,
    kind: usize,
    sel: Selector,
    spec: (u32, u32, u32),
}

fn pseudo_rules(sheet: &Stylesheet) -> Vec<PseudoRule> {
    let mut out = Vec::new();
    for (i, rule) in sheet.rules.iter().enumerate() {
        for sel in &rule.selectors {
            let Some(last) = sel.parts.last() else { continue };
            let Some((pos, kind)) = last.pseudo.iter().enumerate().find_map(|(p, n)| pseudo_kind(n).map(|k| (p, k))) else { continue };
            let mut stripped = sel.clone();
            let lc = stripped.parts.last_mut().unwrap();
            lc.pseudo.remove(pos);
            // `::before:hover` etc. aren't supported: skip rules with more pseudo-elements.
            if lc.pseudo.iter().any(|p| p.starts_with("::")) {
                continue;
            }
            let spec = sel.specificity();
            out.push(PseudoRule { rule: i, kind, sel: stripped, spec });
        }
    }
    out
}

/// A pseudo-element's style: inherits from its element, then its own rules apply.
fn cascade_pseudo(parent: &Style, pm: &[Matched]) -> Style {
    let mut p = Style::inherit(parent);
    for m in pm.iter().filter(|m| m.decl.name.starts_with("--")) {
        let value = substitute_vars(&m.decl.value, &p.vars, 0);
        Rc::make_mut(&mut p.vars).insert(m.decl.name.clone(), value);
    }
    for m in pm.iter().filter(|m| !m.decl.name.starts_with("--")) {
        if m.decl.value.contains("var(") {
            let value = substitute_vars(&m.decl.value, &p.vars, 0);
            apply(&mut p, parent, &m.decl.name, &value);
        } else {
            apply(&mut p, parent, &m.decl.name, &m.decl.value);
        }
    }
    p.fade = parent.fade * p.opacity;
    if !p.border_color_set {
        p.border_color = p.color;
    }
    p
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
    let mut cur = sel_parent(dom, id);
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
                cur = sel_parent(dom, p);
            }
            false
        }
    }
}

/// The parent selectors see. In the flat tree a slotted element sits inside its `<slot>`, but
/// the page's selectors (`my-el > button`) see the light tree: there the parent is the host.
fn sel_parent(dom: &Dom, id: NodeId) -> Option<NodeId> {
    let p = dom.nodes[id].parent?;
    let Some(&scope) = dom.render_scope.get(&p) else { return Some(p) };
    if dom.tag(p) != Some("slot") || dom.render_scope.get(&id) == Some(&scope) {
        return Some(p);
    }
    // Climb out of the shadow tree to its host.
    let mut cur = Some(p);
    while let Some(n) = cur {
        if dom.render_scope.get(&n) != Some(&scope) {
            return Some(n);
        }
        cur = dom.nodes[n].parent;
    }
    None
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
            "any-link" => matches!(tag, "a" | "area") && dom.attr(id, "href").is_some(),
            "link" => matches!(tag, "a" | "area") && dom.attr(id, "href").is_some_and(|h| !is_visited(h)),
            "visited" => matches!(tag, "a" | "area") && dom.attr(id, "href").is_some_and(is_visited),
            "active" => {
                let mut n = ACTIVE.with(|h| h.get());
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
            "focus" | "focus-visible" => focused() == Some(id),
            "focus-within" => focused().is_some_and(|f| {
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
                } else if let Some(arg) = other.strip_prefix("has(").and_then(|a| a.strip_suffix(')')) {
                    has_matches(dom, id, arg)
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

/// `:has(...)`: relative selectors — "img" (a descendant), "> img" (a child),
/// "+ p" (the next sibling) and "~ p" (a later sibling).
fn has_matches(dom: &Dom, id: NodeId, arg: &str) -> bool {
    css::split_top(arg, ',').iter().any(|part| {
        let part = part.trim();
        let (comb, rest) = match part.chars().next() {
            Some(c @ ('>' | '+' | '~')) => (c, part[1..].trim()),
            _ => (' ', part),
        };
        let sels = css::parse_selector_list(rest);
        let hit = |n: NodeId| dom.tag(n).is_some() && sels.iter().any(|s| matches(dom, n, s));
        match comb {
            '>' => dom.nodes[id].children.iter().any(|&c| hit(c)),
            '+' | '~' => {
                let sibs = element_siblings(dom, id);
                let after = sibs.iter().skip_while(|&&s| s != id).skip(1);
                if comb == '+' {
                    after.take(1).any(|&s| hit(s))
                } else {
                    after.into_iter().any(|&s| hit(s))
                }
            }
            _ => {
                let mut stack: Vec<NodeId> = dom.nodes[id].children.clone();
                while let Some(n) = stack.pop() {
                    if hit(n) {
                        return true;
                    }
                    stack.extend(dom.nodes[n].children.iter().copied());
                }
                false
            }
        }
    })
}

/// getComputedStyle: the element's used values as a JSON object of CSS property names.
/// `size` is the element's border box (width, height) when it has been laid out.
pub fn computed_json(st: &Style, size: Option<(f32, f32)>) -> String {
    let rgb = |c: Color| if c.a == 255 { format!("rgb({}, {}, {})", c.r, c.g, c.b) } else { format!("rgba({}, {}, {}, {})", c.r, c.g, c.b, (c.a as f32 / 255.0 * 1000.0).round() / 1000.0) };
    let px = |v: f32| format!("{}px", (v * 100.0).round() / 100.0);
    let len = |l: Len| match l {
        Len::Px(v) => px(v),
        Len::Pct(p) => format!("{p}%"),
        Len::Mix(a, p) => format!("calc({p}% + {a}px)"),
        Len::Auto => "auto".into(),
    };
    let display = match st.display {
        Display::None => "none",
        _ if st.grid.is_some() => if st.display == Display::InlineBlock { "inline-grid" } else { "grid" },
        _ if st.flex.container => if st.display == Display::InlineBlock { "inline-flex" } else { "flex" },
        Display::Block => "block",
        Display::Inline => "inline",
        Display::InlineBlock => "inline-block",
        Display::ListItem => "list-item",
    };
    let position = match st.position {
        Position::Static => "static",
        Position::Relative => "relative",
        Position::Absolute => "absolute",
        Position::Fixed => "fixed",
        Position::Sticky => "sticky",
    };
    let align = match (st.align_logical, st.text_align) {
        (1, _) if !st.rtl => "start",
        (_, Align::Left) => "left",
        (_, Align::Center) => "center",
        (_, Align::Right) => "right",
    };
    let mut deco: Vec<&str> = Vec::new();
    if st.underline { deco.push("underline"); }
    if st.overline { deco.push("overline"); }
    if st.line_through { deco.push("line-through"); }
    let sides = ["top", "right", "bottom", "left"];
    let mut m: Vec<(String, String)> = vec![
        ("display".into(), display.into()),
        ("position".into(), position.into()),
        ("color".into(), rgb(st.color)),
        ("background-color".into(), rgb(if st.bg.color.a > 0 { st.bg.color } else { st.background })),
        ("font-size".into(), px(st.font_size)),
        ("font-weight".into(), if st.bold { "700" } else { "400" }.into()),
        ("font-style".into(), if st.italic { "italic" } else { "normal" }.into()),
        ("font-family".into(), st.family.as_deref().unwrap_or(if st.mono { "monospace" } else { "sans-serif" }).into()),
        ("line-height".into(), px(st.line_height * st.font_size)),
        ("text-align".into(), align.into()),
        ("direction".into(), if st.rtl { "rtl" } else { "ltr" }.into()),
        ("text-decoration-line".into(), if deco.is_empty() { "none".into() } else { deco.join(" ") }),
        ("text-transform".into(), ["none", "uppercase", "lowercase", "capitalize"][st.text_transform.min(3) as usize].into()),
        ("letter-spacing".into(), if st.text.letter_spacing == 0.0 { "normal".into() } else { px(st.text.letter_spacing) }),
        ("word-spacing".into(), px(st.text.word_spacing)),
        ("visibility".into(), if st.hidden { "hidden" } else { "visible" }.into()),
        ("opacity".into(), format!("{}", st.opacity)),
        ("overflow".into(), if st.clip { "hidden" } else { "visible" }.into()),
        ("z-index".into(), if st.z_index == 0 { "auto".into() } else { st.z_index.to_string() }),
        ("box-sizing".into(), if st.border_box { "border-box" } else { "content-box" }.into()),
        ("cursor".into(), ["auto", "pointer", "text", "default", "default", "move"][st.cursor.min(5) as usize].into()),
        ("white-space".into(), if st.pre { "pre" } else { "normal" }.into()),
    ];
    m.extend(st.vertical.computed());
    for i in 0..4 {
        m.push((format!("margin-{}", sides[i]), len(st.margin[i])));
        m.push((format!("padding-{}", sides[i]), len(st.padding[i])));
        m.push((format!("border-{}-width", sides[i]), px(st.border[i])));
        m.push((format!("border-{}-color", sides[i]), rgb(st.border_color)));
        m.push((format!("border-{}-style", sides[i]), if st.border[i] > 0.0 { ["solid", "dashed", "dotted", "double"][st.border_line.min(3) as usize] } else { "none" }.into()));
        m.push((sides[i].to_string(), len(st.inset[i])));
    }
    // width/height: the used content size when laid out, else the specified value.
    let (w, h) = match size {
        Some((w, h)) if st.display != Display::None => {
            let edge = |a: usize, b: usize| if st.border_box { 0.0 } else { st.padding[a].resolve(w) + st.padding[b].resolve(w) + st.border[a] + st.border[b] };
            (px((w - edge(1, 3)).max(0.0)), px((h - edge(0, 2)).max(0.0)))
        }
        _ => (len(st.width), len(st.height)),
    };
    m.push(("width".into(), w));
    m.push(("height".into(), h));
    let mut o = String::from("{");
    for (i, (k, v)) in m.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&format!("{:?}:{:?}", k, v));
    }
    // Custom properties, for getPropertyValue("--name"): their values as written, with var()
    // already substituted.
    let json = |s: &str| {
        let mut q = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => q.push_str("\\\""),
                '\\' => q.push_str("\\\\"),
                c if (c as u32) < 0x20 => q.push_str(&format!("\\u{:04x}", c as u32)),
                c => q.push(c),
            }
        }
        q.push('"');
        q
    };
    for (k, v) in st.vars.iter() {
        if o.len() > 1 {
            o.push(',');
        }
        o.push_str(&json(k));
        o.push(':');
        o.push_str(&json(v.trim()));
    }
    o.push('}');
    o
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
                let flat = |v: CalcVal| v.px + v.pct * css::viewport().0 / 100.0;
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
        "rem" => Len::Px(num * ROOT_FONT.with(std::cell::Cell::get)),
        "%" => Len::Pct(num),
        "pt" => Len::Px(num * 4.0 / 3.0),
        // Viewport units follow the window (css::viewport), not a fixed 1024x768.
        "vw" | "svw" | "lvw" | "dvw" => Len::Px(num * css::viewport().0 / 100.0),
        "vh" | "svh" | "lvh" | "dvh" => Len::Px(num * css::viewport().1 / 100.0),
        "vmin" => Len::Px(num * css::viewport().0.min(css::viewport().1) / 100.0),
        "vmax" => Len::Px(num * css::viewport().0.max(css::viewport().1) / 100.0),
        "ch" | "ex" => Len::Px(num * font_size * 0.5),
        _ => return None,
    })
}

/// hsl(), hwb(), lab(), lch(), oklab(), oklch() — in either the comma or space syntax, with an
/// optional alpha after "/" (or as a 4th comma argument).
fn color_function(v: &str) -> Option<Color> {
    let (name, rest) = v.split_once('(')?;
    let name = name.trim();
    if !matches!(name, "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch") {
        return None;
    }
    let inner = rest.trim_end().trim_end_matches(')');
    let (main, alpha) = match inner.split_once('/') {
        Some((m, a)) => (m, Some(a.trim())),
        None => (inner, None),
    };
    let mut parts: Vec<&str> = main.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    let alpha = alpha.or_else(|| if parts.len() == 4 { parts.pop() } else { None });
    if parts.len() != 3 {
        return None;
    }
    // A number, with % scaled to `pct_of` (100% = pct_of); "none" = 0.
    let num = |s: &str, pct_of: f32| -> Option<f32> {
        if s == "none" {
            return Some(0.0);
        }
        match s.strip_suffix('%') {
            Some(p) => p.parse::<f32>().ok().map(|v| v / 100.0 * pct_of),
            None => s.parse::<f32>().ok(),
        }
    };
    let hue = |s: &str| -> Option<f32> {
        let s = s.trim();
        let deg = if let Some(v) = s.strip_suffix("deg") {
            v.parse::<f32>().ok()?
        } else if let Some(v) = s.strip_suffix("grad") {
            v.parse::<f32>().ok()? * 0.9
        } else if let Some(v) = s.strip_suffix("rad") {
            v.parse::<f32>().ok()?.to_degrees()
        } else if let Some(v) = s.strip_suffix("turn") {
            v.parse::<f32>().ok()? * 360.0
        } else if s == "none" {
            0.0
        } else {
            s.parse::<f32>().ok()?
        };
        Some(deg.rem_euclid(360.0))
    };
    let a = match alpha {
        Some(a) => num(a, 1.0)?.clamp(0.0, 1.0),
        None => 1.0,
    };
    let to8 = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    let hsl_rgb = |h: f32, s: f32, l: f32| -> (f32, f32, f32) {
        let f = |n: f32| {
            let k = (n + h / 30.0) % 12.0;
            l - s * l.min(1.0 - l) * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
        };
        (f(0.0), f(8.0), f(4.0))
    };
    // Linear-light sRGB to the sRGB curve.
    let gamma = |x: f32| if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.max(0.0).powf(1.0 / 2.4) - 0.055 };
    let (r, g, b) = match name {
        "hsl" | "hsla" => {
            let (h, s, l) = (hue(parts[0])?, num(parts[1], 1.0)? / if parts[1].ends_with('%') { 1.0 } else { 100.0 }, num(parts[2], 1.0)? / if parts[2].ends_with('%') { 1.0 } else { 100.0 });
            hsl_rgb(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
        }
        "hwb" => {
            let (h, mut wh, mut bl) = (hue(parts[0])?, num(parts[1], 1.0)? / if parts[1].ends_with('%') { 1.0 } else { 100.0 }, num(parts[2], 1.0)? / if parts[2].ends_with('%') { 1.0 } else { 100.0 });
            if wh + bl >= 1.0 {
                let gray = wh / (wh + bl);
                (gray, gray, gray)
            } else {
                let (r, g, b) = hsl_rgb(h, 1.0, 0.5);
                wh = wh.max(0.0);
                bl = bl.max(0.0);
                let f = |c: f32| c * (1.0 - wh - bl) + wh;
                (f(r), f(g), f(b))
            }
        }
        "oklab" | "oklch" => {
            let l = num(parts[0], 1.0)?;
            let (aa, bb) = if name == "oklab" {
                (num(parts[1], 0.4)?, num(parts[2], 0.4)?)
            } else {
                let c = num(parts[1], 0.4)?;
                let h = hue(parts[2])?.to_radians();
                (c * h.cos(), c * h.sin())
            };
            let l_ = l + 0.396_337_78 * aa + 0.215_803_76 * bb;
            let m_ = l - 0.105_561_346 * aa - 0.063_854_17 * bb;
            let s_ = l - 0.089_484_18 * aa - 1.291_485_5 * bb;
            let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
            (
                gamma(4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3),
                gamma(-1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3),
                gamma(-0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3),
            )
        }
        _ => {
            // CIE lab()/lch() (D50 white), via XYZ to sRGB.
            let l = num(parts[0], 100.0)?;
            let (aa, bb) = if name == "lab" {
                (num(parts[1], 125.0)?, num(parts[2], 125.0)?)
            } else {
                let c = num(parts[1], 150.0)?;
                let h = hue(parts[2])?.to_radians();
                (c * h.cos(), c * h.sin())
            };
            let fy = (l + 16.0) / 116.0;
            let (fx, fz) = (fy + aa / 500.0, fy - bb / 200.0);
            let inv = |t: f32| if t.powi(3) > 0.008856 { t.powi(3) } else { (116.0 * t - 16.0) / 903.3 };
            let (x, y, z) = (0.964_22 * inv(fx), if l > 8.0 { fy.powi(3) } else { l / 903.3 }, 0.825_21 * inv(fz));
            // D50 → D65 (Bradford), then XYZ → linear sRGB.
            let (x6, y6, z6) = (0.955_576_6 * x - 0.023_039_3 * y + 0.063_163_6 * z, -0.028_289_5 * x + 1.009_941_6 * y + 0.021_007_7 * z, 0.012_298_2 * x - 0.020_483 * y + 1.329_909_8 * z);
            (
                gamma(3.240_97 * x6 - 1.537_383 * y6 - 0.498_611 * z6),
                gamma(-0.969_244 * x6 + 1.875_968 * y6 + 0.041_555 * z6),
                gamma(0.055_63 * x6 - 0.203_977 * y6 + 1.056_972 * z6),
            )
        }
    };
    Some(Color { r: to8(r), g: to8(g), b: to8(b), a: to8(a) })
}

/// The used color scheme for a `color-scheme` value: dark only when dark is offered and light
/// isn't (this browser prefers light). None for values that leave it as inherited.
fn scheme_is_dark(v: &str) -> Option<bool> {
    let v = v.trim().to_ascii_lowercase();
    if v.is_empty() || css::is_wide_keyword(&v) {
        return None;
    }
    let words: Vec<&str> = v.split_whitespace().collect();
    Some(words.contains(&"dark") && !words.contains(&"light"))
}

thread_local! {
    /// The color scheme of the element being styled, for `light-dark()`.
    static DARK_SCHEME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(inner) = v.strip_prefix("light-dark(").and_then(|r| r.strip_suffix(')')) {
        let mut depth = 0;
        let comma = inner.char_indices().find(|&(_, c)| {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            c == ',' && depth == 0
        })?;
        let (light, dark) = (&inner[..comma.0], &inner[comma.0 + 1..]);
        return parse_color(if DARK_SCHEME.with(std::cell::Cell::get) { dark } else { light });
    }
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
    if let Some(c) = color_function(&v) {
        return Some(c);
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

pub(crate) fn parse_repeat(t: &str) -> Option<(bool, bool)> {
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

pub(crate) fn parse_bg_size(v: &str, fs: f32) -> BgSize {
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
pub(crate) fn parse_bg_pos(v: &str, fs: f32) -> (Len, Len) {
    // 1–4 values: `center`, `10px 20%`, `right 12px center`, `right 10px bottom 5px`.
    let words: Vec<&str> = v.split_whitespace().collect();
    let kw = |w: &str| matches!(w, "left" | "right" | "top" | "bottom" | "center");
    let mut x = None;
    let mut y = None;
    let mut centers = 0;
    let mut i = 0;
    let mut plain: Vec<Len> = Vec::new();
    while i < words.len() {
        let w = words[i];
        if kw(w) {
            // An edge keyword followed by a length: an offset from that edge.
            let off = words.get(i + 1).filter(|n| !kw(n)).and_then(|n| parse_len(n, fs));
            if off.is_some() {
                i += 1;
            }
            let from_end = |o: Option<Len>| match o {
                None => Len::Pct(100.0),
                Some(Len::Px(p)) => Len::Mix(-p, 100.0),
                Some(Len::Pct(p)) => Len::Pct(100.0 - p),
                Some(Len::Mix(p, q)) => Len::Mix(-p, 100.0 - q),
                Some(other) => other,
            };
            match w {
                "left" => x = Some(off.unwrap_or(Len::Pct(0.0))),
                "right" => x = Some(from_end(off)),
                "top" => y = Some(off.unwrap_or(Len::Pct(0.0))),
                "bottom" => y = Some(from_end(off)),
                _ => centers += 1,
            }
        } else if let Some(l) = parse_len(w, fs) {
            plain.push(l);
        }
        i += 1;
    }
    // Plain lengths (two-value syntax) fill x then y; `center` fills whatever is left.
    for l in plain {
        if x.is_none() {
            x = Some(l);
        } else if y.is_none() {
            y = Some(l);
        }
    }
    let _ = centers;
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
    if lower.contains("gradient(") {
        return crate::gradient::parse(v, 16.0).map(|g| BgImage::Gradient(Box::new(g)));
    }
    None
}

pub fn apply(st: &mut Style, parent: &Style, name: &str, value: &str) {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if lower == "inherit" {
        return; // inherited props already inherited; good enough for now
    }
    if crate::text::apply(&mut st.text, name, &lower, st.font_size) {
        return;
    }
    if let Some(parts) = crate::shorthand::expand(name, v, st.rtl) {
        for (n, val) in parts {
            apply(st, parent, &n, &val);
        }
        return;
    }
    let fs = st.font_size;
    if crate::anim::is_anim_prop(name) {
        crate::anim::apply_prop(&mut st.anim, name, v);
        return;
    }
    match name {
        "filter" | "-webkit-filter" | "backdrop-filter" | "-webkit-backdrop-filter" => {
            if let Some(f) = crate::effects::parse_filters(v, fs) {
                let f = (!f.is_empty()).then(|| Rc::from(f));
                if name.ends_with("backdrop-filter") { st.backdrop = f } else { st.filter = f }
            }
            return;
        }
        "text-shadow" => {
            if let Some(s) = crate::effects::parse_text_shadow(v, fs) {
                st.text_shadow = (!s.is_empty()).then(|| Rc::from(s));
            }
            return;
        }
        _ => {}
    }
    let lens = |s: &str| -> Option<Vec<Len>> {
        css::split_top(s, ' ').iter().filter(|t| !t.trim().is_empty()).map(|t| parse_len(t, fs)).collect()
    };
    if name.starts_with("border") && !name.contains("radius") && !name.contains("spacing") && !name.contains("collapse") && !name.contains("image") && !name.contains("color") && !name.contains("width") {
        for w in lower.split_whitespace() {
            match w {
                "solid" | "groove" | "ridge" | "inset" | "outset" => st.border_line = 0,
                "dashed" => st.border_line = 1,
                "dotted" => st.border_line = 2,
                "double" => st.border_line = 3,
                _ => continue,
            }
            break;
        }
    }
    match name {
        "display" => {
            st.contents = lower == "contents";
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
            st.column_gap_set = true;
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
            st.column_gap_set = lower != "normal";
            if let Some(l) = parse_len(&lower, fs) {
                st.flex.column_gap = l;
            }
        }
        "column-count" => st.multicol.get_or_insert_with(Default::default).count = lower.parse::<u32>().ok().filter(|n| *n > 0),
        "column-width" => st.multicol.get_or_insert_with(Default::default).width = parse_len(&lower, fs).and_then(|l| if let Len::Px(v) = l { Some(v) } else { None }).filter(|v| *v > 0.0),
        "columns" => {
            // `columns: 3`, `columns: 12em`, `columns: 12em 3` in either order; `auto` leaves one unset.
            let mc = st.multicol.get_or_insert_with(Default::default);
            mc.count = None;
            mc.width = None;
            for t in lower.split_whitespace() {
                if let Ok(n) = t.parse::<u32>() {
                    mc.count = Some(n).filter(|n| *n > 0);
                } else if let Some(Len::Px(v)) = parse_len(t, fs) {
                    mc.width = Some(v).filter(|v| *v > 0.0);
                }
            }
        }
        "column-rule" => {
            let (w, c) = border_shorthand(&lower, fs);
            let mc = st.multicol.get_or_insert_with(Default::default);
            mc.rule_width = w;
            mc.rule_color = c;
            mc.rule_dashed = lower.contains("dashed") || lower.contains("dotted");
        }
        "column-rule-width" => {
            if let Some(w) = border_width_word(lower.trim(), fs) {
                st.multicol.get_or_insert_with(Default::default).rule_width = w;
            }
        }
        "column-rule-color" => st.multicol.get_or_insert_with(Default::default).rule_color = parse_color(&lower),
        "column-rule-style" => {
            let mc = st.multicol.get_or_insert_with(Default::default);
            if matches!(lower.as_str(), "none" | "hidden") {
                mc.rule_width = 0.0;
            } else if mc.rule_width == 0.0 {
                mc.rule_width = 3.0;
            }
            mc.rule_dashed = matches!(lower.as_str(), "dashed" | "dotted");
        }
        "grid-template-columns" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).columns = v.to_string(),
        "grid-template-rows" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).rows = v.to_string(),
        "grid-template-areas" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).areas = v.to_string(),
        "grid-auto-rows" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).auto_rows = v.to_string(),
        "grid-auto-columns" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).auto_columns = v.to_string(),
        "grid-template" => {
            // `rows / columns`, or area strings each followed by its row size:
            // `"head head" 60px "nav main" 1fr / 200px 1fr`.
            let g = Rc::make_mut(st.grid.get_or_insert_with(Default::default));
            let (r, c) = match css::split_top(v, '/').as_slice() {
                [r, c, ..] => (r.trim().to_string(), c.trim().to_string()),
                [r] => (r.trim().to_string(), String::new()),
                _ => (String::new(), String::new()),
            };
            if r.contains('"') || r.contains('\'') {
                let mut areas = Vec::new();
                let mut rows = Vec::new();
                let mut last_was_area = false;
                for tok in css::split_top(&r, ' ') {
                    let t = tok.trim();
                    if t.is_empty() || t.starts_with('[') {
                        continue;
                    }
                    if t.starts_with('"') || t.starts_with('\'') {
                        if last_was_area {
                            rows.push("auto".to_string());
                        }
                        areas.push(t.to_string());
                        last_was_area = true;
                    } else {
                        rows.push(t.to_string());
                        last_was_area = false;
                    }
                }
                if last_was_area {
                    rows.push("auto".to_string());
                }
                g.areas = areas.join(" ");
                g.rows = rows.join(" ");
            } else {
                g.rows = r;
            }
            g.columns = c;
        }
        "grid-auto-flow" => Rc::make_mut(st.grid.get_or_insert_with(Default::default)).auto_flow = lower.clone(),
        "justify-items" | "justify-self" => {
            let a = match lower.trim_start_matches("legacy ").trim_start_matches("safe ").trim_start_matches("unsafe ").split_whitespace().next().unwrap_or("") {
                "center" => Some(AlignItems::Center),
                "start" | "flex-start" | "self-start" | "left" => Some(AlignItems::Start),
                "end" | "flex-end" | "self-end" | "right" => Some(AlignItems::End),
                "stretch" => Some(AlignItems::Stretch),
                _ => None,
            };
            if name == "justify-items" {
                st.flex.justify_items = a;
            } else {
                st.flex.justify_self = a;
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
        "opacity" => {
            let v = lower.trim();
            let n = match v.strip_suffix('%') {
                Some(p) => p.parse::<f32>().ok().map(|p| p / 100.0),
                None => v.parse::<f32>().ok(),
            };
            if let Some(n) = n {
                st.opacity = n.clamp(0.0, 1.0);
            }
        }
        "object-fit" => {
            st.object_fit = match lower.as_str() {
                "contain" => ObjectFit::Contain,
                "cover" => ObjectFit::Cover,
                "none" => ObjectFit::None,
                "scale-down" => ObjectFit::ScaleDown,
                _ => ObjectFit::Fill,
            }
        }
        "object-position" => st.object_position = parse_bg_pos(&lower, fs),
        "aspect-ratio" => {
            // "16 / 9", "1.5", "auto 4/3" (auto is ignored: we use the ratio).
            let v = lower.replace("auto", "");
            let v = v.trim();
            st.aspect_ratio = match v.split_once('/') {
                Some((a, b)) => match (a.trim().parse::<f32>(), b.trim().parse::<f32>()) {
                    (Ok(a), Ok(b)) if a > 0.0 && b > 0.0 => Some(a / b),
                    _ => None,
                },
                None => v.parse::<f32>().ok().filter(|r| *r > 0.0),
            };
        }
        "border-spacing" => {
            if let Some(l) = lower.split_whitespace().next().and_then(|t| parse_len(t, fs)) {
                st.border_spacing = l.resolve(0.0);
            }
        }
        "border-collapse" => st.border_collapse = lower == "collapse",
        "vertical-align" => {
            st.valign = match lower.as_str() {
                "top" => VAlign::Top,
                "text-top" => VAlign::TextTop,
                "middle" => VAlign::Middle,
                "bottom" => VAlign::Bottom,
                "text-bottom" => VAlign::TextBottom,
                "sub" => VAlign::Sub,
                "super" => VAlign::Super,
                other => match parse_len(other, fs) {
                    Some(l) if l != Len::Auto => VAlign::Shift(l),
                    _ => VAlign::Baseline,
                },
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
        "content" => st.content = Some(v.into()),
        "appearance" | "-webkit-appearance" | "-moz-appearance" => st.appearance_none = lower == "none",
        "container-type" | "container-name" | "container" => {
            let (mut kind, mut names) = st.container.clone().map(|(k, n)| (k, n.to_string())).unwrap_or((0, String::new()));
            let (n, t) = match name {
                "container-type" => (None, Some(lower.as_str())),
                "container-name" => (Some(v), None),
                _ => match v.split_once('/') {
                    Some((n, t)) => (Some(n.trim()), Some(t.trim())),
                    None => (Some(v), Some("normal")),
                },
            };
            if let Some(t) = t {
                kind = match t.trim().to_ascii_lowercase().as_str() {
                    "inline-size" => 1,
                    "size" => 2,
                    _ => 0,
                };
            }
            if let Some(n) = n {
                names = if n.eq_ignore_ascii_case("none") { String::new() } else { n.to_string() };
            }
            st.container = (kind > 0).then(|| (kind, names.into()));
        }
        "counter-reset" | "counter-set" | "counter-increment" => {
            let kind = match name {
                "counter-reset" => 0,
                "counter-set" => 1,
                _ => 2,
            };
            let mut list: Vec<(u8, Rc<str>)> = st.counters.as_deref().map(|c| c.iter().filter(|(k, _)| *k != kind).cloned().collect()).unwrap_or_default();
            if lower != "none" {
                list.push((kind, v.into()));
            }
            st.counters = (!list.is_empty()).then(|| list.into());
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
        "background" => crate::background::apply_shorthand(st, v, fs),
        "background-image" => crate::background::set_images(&mut st.bg, v),
        "background-size" => crate::background::set_sizes(&mut st.bg, &lower, fs),
        "background-position" => crate::background::set_positions(&mut st.bg, &lower, fs),
        "background-repeat" => crate::background::set_repeats(&mut st.bg, &lower),
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
            let pw = parent.font_weight();
            st.weight = match lower.as_str() {
                "bold" => 700,
                "normal" => 400,
                // Relative to the parent's weight (CSS Fonts: the bolder/lighter table).
                "bolder" => if pw < 350 { 400 } else if pw < 550 { 700 } else { 900 },
                "lighter" => if pw < 550 { 100 } else if pw < 750 { 400 } else { 700 },
                n => n.parse::<f32>().ok().filter(|w| (1.0..=1000.0).contains(w)).map(|w| w.round() as u16).unwrap_or_else(|| st.font_weight()),
            };
            st.bold = st.weight >= 600;
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
            st.weight = 400;
            while let Some((tok, after)) = rest.split_once(' ') {
                if tok.chars().next().map_or(false, |c| c.is_ascii_digit() || c == '.') && !tok.chars().all(|c| c.is_ascii_digit()) {
                    break;
                }
                match tok {
                    "italic" | "oblique" => st.italic = true,
                    "bold" | "bolder" => (st.bold, st.weight) = (true, 700),
                    w if w.len() == 3 && w.ends_with("00") && w.as_bytes()[0].is_ascii_digit() => {
                        st.weight = w.parse().unwrap_or(400);
                        st.bold = st.weight >= 600;
                    }
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
        "writing-mode" | "-webkit-writing-mode" | "text-orientation" | "-webkit-text-orientation" | "text-combine-upright" | "-webkit-text-combine" => {
            st.vertical.apply(name, &lower);
        }
        "direction" => match lower.as_str() {
            "rtl" => st.rtl = true,
            "ltr" => st.rtl = false,
            _ => {}
        },
        "white-space" => st.pre = lower.starts_with("pre") && lower != "pre-line",
        "text-decoration" | "text-decoration-line" => {
            st.underline = lower.contains("underline");
            st.line_through = lower.contains("line-through");
            st.overline = lower.contains("overline");
            if name == "text-decoration" {
                // The shorthand may carry a color too: text-decoration: underline red.
                if let Some(c) = lower.split_whitespace().filter(|w| !matches!(*w, "underline" | "overline" | "line-through" | "none" | "solid" | "double" | "dotted" | "dashed" | "wavy")).find_map(parse_color) {
                    st.deco_color = Some(c);
                }
            }
        }
        "text-decoration-color" => st.deco_color = parse_color(&lower),
        "text-transform" => {
            st.text_transform = match lower.trim() {
                "uppercase" => 1,
                "lowercase" => 2,
                "capitalize" => 3,
                _ => 0,
            }
        }
        "text-indent" => {
            if let Some(l) = parse_len(lower.split_whitespace().next().unwrap_or("0"), st.font_size) {
                st.text_indent = l;
            }
        }
        "cursor" => {
            let first = lower.rsplit(',').next().unwrap_or("").trim();
            st.cursor = match first {
                "pointer" => 1,
                "text" | "vertical-text" => 2,
                "default" | "not-allowed" | "wait" | "progress" | "help" | "context-menu" | "no-drop" | "cell" | "crosshair" => 3,
                "move" | "grab" | "grabbing" | "all-scroll" => 5,
                _ => 0,
            }
        }
        "list-style-position" => st.list_inside = lower.trim() == "inside",
        "list-style" | "list-style-type" => {
            if name == "list-style" {
                st.list_inside = lower.split_whitespace().any(|w| w == "inside");
            }
            if let Some(t) = lower.split_whitespace().find_map(list_type_keyword) {
                st.list_type = t;
            }
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
            if let Some(l) = parse_len(&lower, fs) {
                st.height = l
            }
        }
        "border" => {
            let (w, c) = border_shorthand(&lower, fs);
            st.border = [w; 4];
            st.border_colors = None;
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
            st.border_colors = None;
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
                            st.set_side_color(i, c);
                        }
                        "width" => {
                            if let Some(w) = border_width_word(&lower, fs) {
                                st.border[i] = w;
                            }
                        }
                        "color" => {
                            if let Some(c) = parse_color(&lower) {
                                st.set_side_color(i, Some(c));
                            } else if lower == "currentcolor" {
                                st.set_side_color(i, None);
                            }
                        }
                        // One line style for the whole box: `none` removes just this side.
                        "style" => match lower.as_str() {
                            "none" | "hidden" => st.border[i] = 0.0,
                            "dashed" => st.border_line = 1,
                            "dotted" => st.border_line = 2,
                            "double" => st.border_line = 3,
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn color_functions() {
        let c = |s: &str| parse_color(s).map(|c| (c.r, c.g, c.b, c.a));
        assert_eq!(c("hsl(0, 100%, 50%)"), Some((255, 0, 0, 255)));
        assert_eq!(c("hsl(120deg 100% 25% / 50%)"), Some((0, 128, 0, 128)));
        assert_eq!(c("hsla(240, 100%, 50%, 0.5)"), Some((0, 0, 255, 128)));
        assert_eq!(c("hwb(0 0% 0%)"), Some((255, 0, 0, 255)));
        assert_eq!(c("hwb(0 50% 50%)"), Some((128, 128, 128, 255)));
        // oklch / lab of pure colors land within a step or two of the sRGB values.
        let near = |s: &str, want: (u8, u8, u8)| {
            let (r, g, b, _) = c(s).unwrap();
            assert!((r as i32 - want.0 as i32).abs() <= 2 && (g as i32 - want.1 as i32).abs() <= 2 && (b as i32 - want.2 as i32).abs() <= 2, "{s} -> {r},{g},{b}");
        };
        near("oklch(62.8% 0.2577 29.23)", (255, 0, 0));
        near("oklab(1 0 0)", (255, 255, 255));
        near("lab(54.29 80.82 69.91)", (255, 0, 0));
        near("lch(0 0 0)", (0, 0, 0));
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
    /// One side's border color (None: the text color, `currentColor`).
    fn set_side_color(&mut self, side: usize, c: Option<Color>) {
        let mut sides = self.border_colors.unwrap_or_default();
        sides[side] = Some(c.unwrap_or(Color { a: 0, r: 0, g: 0, b: 1 }));
        self.border_colors = Some(sides);
    }

    /// Each side's border color: top, right, bottom, left.
    pub fn side_colors(&self) -> [Color; 4] {
        let mut out = [self.border_color; 4];
        if let Some(sides) = &self.border_colors {
            for (o, s) in out.iter_mut().zip(sides) {
                match s {
                    // The marker for currentColor.
                    Some(Color { a: 0, r: 0, g: 0, b: 1 }) => *o = self.color,
                    Some(c) => *o = *c,
                    None => {}
                }
            }
        }
        out
    }

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

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn visited_changes_only_colors_and_active_follows_the_press() {
        let dom = crate::html::parse("<a id=v href='/seen'>a</a><a id=n href='/new'><b>b</b></a>");
        let sheet = css::parse("a:visited { color: rgb(255, 0, 0); width: 100px } a:active { color: rgb(0, 128, 0) }");
        let find = |name: &str| dom.find_all("a").into_iter().find(|&a| dom.attr(a, "id") == Some(name)).unwrap();
        let (v, n) = (find("v"), find("n"));
        set_visited("http://x.test/page", ["http://x.test/seen".to_string()].into_iter().collect());
        let st = compute(&dom, std::slice::from_ref(&sheet));
        assert_eq!(st[v].color, Color::rgb(255, 0, 0));
        assert_eq!(st[v].width, Len::Auto, "layout properties don't apply to :visited");
        assert_eq!(st[n].color, Color::rgb(0, 0, 238));
        // Pressing the <b> inside the second link makes the link :active.
        set_active(dom.nodes[n].children.first().copied());
        let st = compute(&dom, std::slice::from_ref(&sheet));
        assert_eq!(st[n].color, Color::rgb(0, 128, 0));
        assert!(uses_active());
        set_active(None);
        set_visited("", Default::default());
    }
}

/// A `list-style-type` keyword this engine can draw.
fn list_type_keyword(w: &str) -> Option<&'static str> {
    const KNOWN: &[&str] = &[
        "none", "disc", "circle", "square", "decimal", "decimal-leading-zero", "lower-roman", "upper-roman",
        "lower-alpha", "upper-alpha", "lower-latin", "upper-latin", "lower-greek",
    ];
    KNOWN.iter().find(|k| **k == w).copied()
}
