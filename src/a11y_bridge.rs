//! Hands the accessibility tree (`a11y.rs`) to Linux screen readers (Orca and others) over
//! AT-SPI, through AccessKit. Nothing is built until a screen reader asks: `update` costs
//! nothing while no assistive technology is listening.
//!
//! The tree a screen reader sees: the window, then the browser's own controls (tab list,
//! back / forward / reload, the address field) and the page (a web document holding the page's
//! accessibility tree). Screen readers can focus and press things; those requests come back
//! to the window loop through `take_requests`.

use crate::a11y;
use crate::dom::{Dom, NodeId};
use crate::layout::Layout;
use crate::style::Style;
use accesskit::{Action, ActionRequest, Node, NodeId as AkId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WINDOW: AkId = AkId(1);
const TOOLBAR: AkId = AkId(2);
const BACK: AkId = AkId(3);
const FORWARD: AkId = AkId(4);
const RELOAD: AkId = AkId(5);
const ADDRESS: AkId = AkId(6);
const TABLIST: AkId = AkId(7);
/// Tab i is TAB0 + i.
const TAB0: u64 = 100;
/// DOM node n is PAGE0 + n.
const PAGE0: u64 = 10_000;

/// What a screen reader asked the browser to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Move keyboard focus to a page element.
    Focus(NodeId),
    /// Press a page element (a link, a button, a checkbox…).
    Click(NodeId),
    /// Scroll a page element into view.
    ScrollTo(NodeId),
    /// Set a text field's value.
    SetValue(NodeId, String),
    Back,
    Forward,
    Reload,
    FocusAddress,
    /// Switch to tab i.
    Tab(usize),
}

/// The browser UI state the bridge describes besides the page.
pub struct Chrome<'a> {
    pub tabs: &'a [String],
    pub active_tab: usize,
    pub address: &'a str,
    pub editing_address: bool,
    pub can_back: bool,
    pub can_forward: bool,
    /// The window's inner size in pixels and where the page starts (below the toolbars).
    pub win_w: f32,
    pub win_h: f32,
    pub page_top: f32,
    /// Toolbar layout: (back, forward, reload) button x positions and width, the address
    /// field's x, the tab strip height, and the tab rectangles (x, width).
    pub buttons: [f32; 3],
    pub button_w: f32,
    pub field_x: f32,
    pub tab_h: f32,
    pub tool_h: f32,
    pub tab_rects: &'a [(f32, f32)],
}

/// The page as it is drawn.
pub struct Page<'a> {
    pub dom: &'a Dom,
    pub styles: &'a [Rc<Style>],
    pub layout: &'a Layout,
    pub focused: Option<NodeId>,
    pub scroll: f32,
    pub url: &'a str,
}

/// Windows and macOS need AccessKit's per-window adapters (accesskit_winit); until then the
/// bridge is Linux-only and `Bridge::new` returns None elsewhere.
#[cfg(target_os = "linux")]
type Adapter = accesskit_unix::Adapter;
#[cfg(not(target_os = "linux"))]
struct Adapter;
#[cfg(not(target_os = "linux"))]
impl Adapter {
    fn update_window_focus_state(&mut self, _: bool) {}
    fn update_if_active(&mut self, _: impl FnOnce() -> TreeUpdate) {}
}

pub struct Bridge {
    adapter: Adapter,
    requests: Receiver<ActionRequest>,
    /// Set by the activation handler: a screen reader wants a tree.
    wanted: Arc<Mutex<bool>>,
    last: Option<Instant>,
    /// A cheap fingerprint of what was last sent, to skip identical updates.
    last_key: u64,
}

struct Activation(Arc<Mutex<bool>>);
impl accesskit::ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // The window loop builds the tree on its own thread (the DOM isn't Send); returning
        // None makes the adapter wait for the first `update_if_active`.
        if let Ok(mut w) = self.0.lock() {
            *w = true;
        }
        None
    }
}

struct Actions(Mutex<Sender<ActionRequest>>);
impl accesskit::ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        if let Ok(s) = self.0.lock() {
            let _ = s.send(request);
        }
    }
}

struct Deactivation(Arc<Mutex<bool>>);
impl accesskit::DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {
        if let Ok(mut w) = self.0.lock() {
            *w = false;
        }
    }
}

impl Bridge {
    /// Connect to the accessibility bus. `IRONSWIFT_A11Y=0` turns this off.
    pub fn new() -> Option<Bridge> {
        if std::env::var("IRONSWIFT_A11Y").is_ok_and(|v| v == "0") {
            return None;
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (Activation(Arc::default()), Actions(Mutex::new(channel().0)), Deactivation(Arc::default()));
            None
        }
        #[cfg(target_os = "linux")]
        {
            let wanted = Arc::new(Mutex::new(false));
            let (tx, rx) = channel();
            let adapter = accesskit_unix::Adapter::new(Activation(wanted.clone()), Actions(Mutex::new(tx)), Deactivation(wanted.clone()));
            Some(Bridge { adapter, requests: rx, wanted, last: None, last_key: 0 })
        }
    }

    /// Whether a screen reader is listening.
    pub fn active(&self) -> bool {
        self.wanted.lock().map(|w| *w).unwrap_or(false)
    }

    /// A screen reader has connected and nothing has been sent yet: the window should draw a
    /// frame (the tree is sent from the drawing code, where the toolbar state is at hand).
    pub fn wants_frame(&self) -> bool {
        self.active() && self.last_key == 0
    }

    pub fn window_focused(&mut self, focused: bool) {
        self.adapter.update_window_focus_state(focused);
    }

    /// Send the current state, at most every 100 ms unless focus moved. Call every frame; it
    /// does nothing while no screen reader listens.
    pub fn update(&mut self, title: &str, chrome: &Chrome, page: &Page) {
        if !self.active() {
            // Send everything afresh if one connects later.
            self.last_key = 0;
            return;
        }
        let key = fingerprint(title, chrome, page);
        let soon = self.last.is_some_and(|t| t.elapsed() < Duration::from_millis(100));
        if key == self.last_key || (soon && !focus_changed(self.last_key, key)) {
            return;
        }
        self.last_key = key;
        self.last = Some(Instant::now());
        self.adapter.update_if_active(|| tree_update(title, chrome, page));
    }

    /// Requests from screen readers since the last call.
    pub fn take_requests(&mut self) -> Vec<Request> {
        let mut out = Vec::new();
        while let Ok(r) = self.requests.try_recv() {
            if let Some(q) = request_for(&r) {
                out.push(q);
            }
        }
        out
    }
}

/// The low 16 bits of the fingerprint hold the focus, so a focus move is always sent at once.
fn focus_changed(a: u64, b: u64) -> bool {
    (a & 0xffff) != (b & 0xffff)
}

fn fingerprint(title: &str, chrome: &Chrome, page: &Page) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    };
    eat(title.as_bytes());
    eat(chrome.address.as_bytes());
    eat(&[chrome.editing_address as u8, chrome.can_back as u8, chrome.can_forward as u8, chrome.active_tab as u8]);
    for t in chrome.tabs {
        eat(t.as_bytes());
    }
    eat(page.url.as_bytes());
    eat(&(page.dom.nodes.len() as u64).to_le_bytes());
    eat(&(page.layout.items.len() as u64).to_le_bytes());
    eat(&page.scroll.to_bits().to_le_bytes());
    eat(&chrome.win_w.to_bits().to_le_bytes());
    let f = if chrome.editing_address { 0xfffe } else { page.focused.map_or(0xffff, |f| (f as u64) & 0xfffd) };
    (h & !0xffff) | f
}

fn request_for(r: &ActionRequest) -> Option<Request> {
    let id = r.target_node.0;
    if id >= PAGE0 {
        let n = (id - PAGE0) as NodeId;
        return match r.action {
            Action::Focus => Some(Request::Focus(n)),
            Action::Click => Some(Request::Click(n)),
            Action::ScrollIntoView => Some(Request::ScrollTo(n)),
            Action::SetValue => match &r.data {
                Some(accesskit::ActionData::Value(v)) => Some(Request::SetValue(n, v.to_string())),
                _ => None,
            },
            _ => None,
        };
    }
    if (TAB0..PAGE0).contains(&id) && matches!(r.action, Action::Click | Action::Focus) {
        return Some(Request::Tab((id - TAB0) as usize));
    }
    match (AkId(id), r.action) {
        (BACK, Action::Click) => Some(Request::Back),
        (FORWARD, Action::Click) => Some(Request::Forward),
        (RELOAD, Action::Click) => Some(Request::Reload),
        (ADDRESS, Action::Focus | Action::Click) => Some(Request::FocusAddress),
        _ => None,
    }
}

/// The AccessKit role for an ARIA role from `a11y.rs`.
pub fn role(aria: &str) -> Role {
    match aria {
        "text" => Role::Label,
        "link" => Role::Link,
        "button" => Role::Button,
        "heading" => Role::Heading,
        "paragraph" => Role::Paragraph,
        "list" => Role::List,
        "listitem" => Role::ListItem,
        "textbox" => Role::TextInput,
        "searchbox" => Role::SearchInput,
        "checkbox" => Role::CheckBox,
        "radio" => Role::RadioButton,
        "radiogroup" => Role::RadioGroup,
        "combobox" => Role::ComboBox,
        "listbox" => Role::ListBox,
        "option" => Role::ListBoxOption,
        "img" => Role::Image,
        "table" => Role::Table,
        "grid" => Role::Grid,
        "treegrid" => Role::TreeGrid,
        "row" => Role::Row,
        "rowgroup" => Role::RowGroup,
        "cell" => Role::Cell,
        "gridcell" => Role::GridCell,
        "columnheader" => Role::ColumnHeader,
        "rowheader" => Role::RowHeader,
        "navigation" => Role::Navigation,
        "main" => Role::Main,
        "banner" => Role::Banner,
        "contentinfo" => Role::ContentInfo,
        "complementary" => Role::Complementary,
        "form" => Role::Form,
        "region" => Role::Region,
        "search" => Role::Search,
        "article" => Role::Article,
        "group" => Role::Group,
        "figure" => Role::Figure,
        "caption" => Role::Caption,
        "separator" => Role::Splitter,
        "slider" => Role::Slider,
        "spinbutton" => Role::SpinButton,
        "progressbar" => Role::ProgressIndicator,
        "meter" => Role::Meter,
        "scrollbar" => Role::ScrollBar,
        "dialog" => Role::Dialog,
        "alertdialog" => Role::AlertDialog,
        "alert" => Role::Alert,
        "status" => Role::Status,
        "log" => Role::Log,
        "marquee" => Role::Marquee,
        "timer" => Role::Timer,
        "note" => Role::Note,
        "feed" => Role::Feed,
        "tab" => Role::Tab,
        "tablist" => Role::TabList,
        "tabpanel" => Role::TabPanel,
        "menu" => Role::Menu,
        "menubar" => Role::MenuBar,
        "menuitem" => Role::MenuItem,
        "menuitemcheckbox" => Role::MenuItemCheckBox,
        "menuitemradio" => Role::MenuItemRadio,
        "tree" => Role::Tree,
        "treeitem" => Role::TreeItem,
        "switch" => Role::Switch,
        "toolbar" => Role::Toolbar,
        "tooltip" => Role::Tooltip,
        "blockquote" => Role::Blockquote,
        "code" => Role::Code,
        "emphasis" => Role::Emphasis,
        "strong" => Role::Strong,
        "mark" => Role::Mark,
        "term" => Role::Term,
        "definition" => Role::Definition,
        "deletion" => Role::ContentDeletion,
        "insertion" => Role::ContentInsertion,
        "time" => Role::Time,
        "math" => Role::Math,
        "application" => Role::Application,
        "document" => Role::Document,
        "directory" => Role::List,
        _ => Role::GenericContainer,
    }
}

/// Where a DOM node is drawn, in page coordinates: its own box, or the union of the areas
/// drawn for it and its descendants.
pub fn node_rect(dom: &Dom, lay: &Layout, n: NodeId) -> Option<crate::layout::Rect> {
    if let Some((r, _)) = lay.boxes.get(&n) {
        return Some(*r);
    }
    let mut acc: Option<(f32, f32, f32, f32)> = None;
    for (r, h) in &lay.hits {
        if *h == n || dom.is_inside(*h, n) {
            let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
            acc = Some(match acc {
                None => (x0, y0, x1, y1),
                Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
            });
        }
    }
    acc.map(|(x0, y0, x1, y1)| crate::layout::Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x0: x as f64, y0: y as f64, x1: (x + w) as f64, y1: (y + h) as f64 }
}

fn tree_update(title: &str, chrome: &Chrome, page: &Page) -> TreeUpdate {
    let mut nodes: Vec<(AkId, Node)> = Vec::new();
    let t = a11y::build(page.dom, Some(page.styles), page.focused);
    let page_id = |i: usize| AkId(PAGE0 + t.nodes[i].dom as u64);

    // The window.
    let mut win = Node::new(Role::Window);
    win.set_label(if title.is_empty() { "Ironswift".to_string() } else { format!("{title} — Ironswift") });
    win.set_children(vec![TABLIST, TOOLBAR, page_id(0)]);
    win.set_bounds(rect(0.0, 0.0, chrome.win_w, chrome.win_h));
    nodes.push((WINDOW, win));

    // Tabs.
    let mut tl = Node::new(Role::TabList);
    tl.set_label(crate::i18n::t("a11y-tabs"));
    tl.set_bounds(rect(0.0, 0.0, chrome.win_w, chrome.tab_h));
    let mut kids = Vec::new();
    for (i, name) in chrome.tabs.iter().enumerate() {
        let id = AkId(TAB0 + i as u64);
        let mut tab = Node::new(Role::Tab);
        tab.set_label(if name.is_empty() { crate::i18n::t("tab-untitled") } else { name.clone() });
        tab.set_selected(i == chrome.active_tab);
        tab.add_action(Action::Click);
        tab.add_action(Action::Focus);
        if let Some(&(x, w)) = chrome.tab_rects.get(i) {
            tab.set_bounds(rect(x, 0.0, w, chrome.tab_h));
        }
        kids.push(id);
        nodes.push((id, tab));
    }
    tl.set_children(kids);
    nodes.push((TABLIST, tl));

    // Toolbar: back, forward, reload and the address field.
    let mut tb = Node::new(Role::Toolbar);
    tb.set_label(crate::i18n::t("a11y-toolbar"));
    tb.set_bounds(rect(0.0, chrome.tab_h, chrome.win_w, chrome.tool_h));
    tb.set_children(vec![BACK, FORWARD, RELOAD, ADDRESS]);
    nodes.push((TOOLBAR, tb));
    for (k, (id, label, enabled)) in
        [(BACK, "button-back", chrome.can_back), (FORWARD, "button-forward", chrome.can_forward), (RELOAD, "button-reload", true)]
            .into_iter()
            .enumerate()
    {
        let mut b = Node::new(Role::Button);
        b.set_label(crate::i18n::t(label));
        b.set_bounds(rect(chrome.buttons[k], chrome.tab_h + 3.0, chrome.button_w, chrome.tool_h - 6.0));
        if enabled {
            b.add_action(Action::Click);
        } else {
            b.set_disabled();
        }
        nodes.push((id, b));
    }
    let mut addr = Node::new(Role::TextInput);
    addr.set_label(crate::i18n::t("a11y-address"));
    addr.set_value(chrome.address.to_string());
    addr.set_bounds(rect(chrome.field_x, chrome.tab_h + 3.0, chrome.win_w - chrome.field_x - 8.0, chrome.tool_h - 6.0));
    addr.add_action(Action::Focus);
    nodes.push((ADDRESS, addr));

    // The page.
    let mut focus = page_id(0);
    let doc_rect = (0.0, chrome.page_top, chrome.win_w, chrome.win_h - chrome.page_top);
    for (i, n) in t.nodes.iter().enumerate() {
        let id = page_id(i);
        let mut node = if i == 0 {
            let mut d = Node::new(Role::RootWebArea);
            d.set_url(page.url.to_string());
            d.set_bounds(rect(doc_rect.0, doc_rect.1, doc_rect.2, doc_rect.3));
            d.set_scroll_y(page.scroll as f64);
            d
        } else {
            Node::new(role(n.role))
        };
        if n.role == "text" {
            node.set_value(n.name.clone());
        } else if !n.name.is_empty() {
            node.set_label(n.name.clone());
        }
        if !n.description.is_empty() {
            node.set_description(n.description.clone());
        }
        if let Some(v) = &n.value {
            if n.role != "text" {
                node.set_value(v.clone());
            }
        }
        if let Some(l) = n.level {
            node.set_level(l as usize);
        }
        if let Some(h) = &n.href {
            node.set_url(crate::net::resolve(page.url, h));
        }
        if let Some((min, max, now)) = n.range {
            node.set_min_numeric_value(min);
            node.set_max_numeric_value(max);
            node.set_numeric_value(now);
        }
        let s = &n.states;
        if s.focusable {
            node.add_action(Action::Focus);
        }
        if s.disabled {
            node.set_disabled();
        }
        if let Some(c) = s.checked {
            node.set_toggled(match c {
                Some(true) => Toggled::True,
                Some(false) => Toggled::False,
                None => Toggled::Mixed,
            });
        }
        if let Some(p) = s.pressed {
            node.set_toggled(Toggled::from(p));
        }
        if let Some(e) = s.expanded {
            node.set_expanded(e);
        }
        if let Some(sel) = s.selected {
            node.set_selected(sel);
        }
        if s.required {
            node.set_required();
        }
        if s.readonly {
            node.set_read_only();
        }
        if s.invalid {
            node.set_invalid(accesskit::Invalid::True);
        }
        if s.modal {
            node.set_modal();
        }
        if let Some(live) = s.live {
            node.set_live(if live == "assertive" { accesskit::Live::Assertive } else { accesskit::Live::Polite });
        }
        if matches!(n.role, "link" | "button" | "checkbox" | "radio" | "switch" | "tab" | "menuitem" | "menuitemcheckbox" | "menuitemradio" | "option" | "combobox" | "treeitem")
            || (s.focusable && n.role == "generic")
        {
            node.add_action(Action::Click);
        }
        if matches!(n.role, "textbox" | "searchbox") && !s.readonly && !s.disabled {
            node.add_action(Action::SetValue);
        }
        if i != 0 {
            node.add_action(Action::ScrollIntoView);
            if let Some(r) = node_rect(page.dom, page.layout, n.dom) {
                let fixed = page.layout.boxes.get(&n.dom).is_some_and(|b| b.1);
                let y = chrome.page_top + r.y - if fixed { 0.0 } else { page.scroll };
                node.set_bounds(rect(r.x, y, r.w, r.h));
            }
        }
        node.set_children(n.children.iter().map(|&c| page_id(c)).collect::<Vec<_>>());
        if s.focused {
            focus = id;
        }
        nodes.push((id, node));
    }
    if chrome.editing_address {
        focus = ADDRESS;
    }
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo { root: WINDOW, toolkit_name: Some("Ironswift".into()), toolkit_version: Some(env!("CARGO_PKG_VERSION").into()) }),
        tree_id: TreeId::ROOT,
        focus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_tree_maps_to_accesskit() {
        let dom = crate::html::parse(r#"<title>T</title><h1>Hi</h1><a href="/x">Go</a><input type="checkbox" checked aria-label="Ok">"#);
        let styles = crate::style::compute(&dom, &[]);
        let lay = Layout::default();
        let tabs = vec!["T".to_string()];
        let chrome = Chrome {
            tabs: &tabs,
            active_tab: 0,
            address: "http://a/",
            editing_address: false,
            can_back: false,
            can_forward: false,
            win_w: 800.0,
            win_h: 600.0,
            page_top: 70.0,
            buttons: [6.0, 38.0, 70.0],
            button_w: 30.0,
            field_x: 108.0,
            tab_h: 34.0,
            tool_h: 36.0,
            tab_rects: &[(0.0, 200.0)],
        };
        let page = Page { dom: &dom, styles: &styles, layout: &lay, focused: None, scroll: 0.0, url: "http://a/" };
        let u = tree_update("T", &chrome, &page);
        let find = |r: Role| u.nodes.iter().find(|(_, n)| n.role() == r).map(|(_, n)| n.clone());
        assert_eq!(find(Role::Heading).unwrap().label(), Some("Hi"));
        let link = find(Role::Link).unwrap();
        assert_eq!(link.url(), Some("http://a/x"));
        assert!(link.supports_action(Action::Click));
        assert_eq!(find(Role::CheckBox).unwrap().toggled(), Some(Toggled::True));
        assert!(find(Role::RootWebArea).is_some());
        assert_eq!(find(Role::Button).unwrap().label(), Some("Back"));
        assert_eq!(u.focus, AkId(PAGE0));
        // Every child named exists.
        let ids: std::collections::HashSet<AkId> = u.nodes.iter().map(|n| n.0).collect();
        for (_, n) in &u.nodes {
            for c in n.children() {
                assert!(ids.contains(c), "missing child {c:?}");
            }
        }
        assert_eq!(request_for(&ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: AkId(PAGE0 + 5), data: None }), Some(Request::Click(5)));
        assert_eq!(request_for(&ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: BACK, data: None }), Some(Request::Back));
    }
}
