//! Clicks as other browsers fire them: mousedown on press, then mouseup and `click` on
//! release (at the nearest element that both were over), `dblclick` after a second click in
//! the same place, and `contextmenu` before the browser's own menu opens.

use crate::dom::{Dom, NodeId};
use crate::js::Js;
use std::time::Instant;

/// A left-button press on the page, waiting for its release to become a click.
#[derive(Clone)]
pub struct Press {
    /// The element under the mouse at the press (None: nothing there).
    pub node: Option<NodeId>,
    /// Where it was: window x, y, and y in the page's viewport.
    pub mx: f32,
    pub my: f32,
    pub view_y: f32,
    /// The link under the press, if any.
    pub hit: Option<String>,
    /// A keyboard activation (Enter or Space): released wherever the mouse really is.
    pub synthetic: bool,
    /// The document pressed in; a release in another one (after a navigation) is no click.
    pub doc: u64,
}

/// The element a click goes to: the nearest common ancestor (or self) of the elements under
/// the press and the release. None if they share none (the release left the page).
pub fn target(d: &Dom, down: Option<NodeId>, up: Option<NodeId>) -> Option<NodeId> {
    let (a, b) = (down?, up?);
    // The page may have been replaced between press and release.
    if a >= d.nodes.len() || b >= d.nodes.len() {
        return None;
    }
    let mut chain = Vec::new();
    let mut cur = Some(a);
    while let Some(x) = cur {
        chain.push(x);
        cur = d.nodes.get(x).and_then(|n| n.parent);
    }
    let mut cur = Some(b);
    while let Some(x) = cur {
        if chain.contains(&x) {
            return Some(x);
        }
        cur = d.nodes.get(x).and_then(|n| n.parent);
    }
    None
}

/// The last click on the page, to tell when the next one makes a double click.
pub struct Last {
    at: Instant,
    x: f32,
    y: f32,
}

impl Default for Last {
    fn default() -> Self {
        Last { at: Instant::now() - std::time::Duration::from_secs(5), x: -100.0, y: -100.0 }
    }
}

impl Last {
    /// A click happened at (x, y) (window pixels). True if it completes a double click.
    pub fn record(&mut self, x: f32, y: f32) -> bool {
        let double = self.at.elapsed().as_millis() < 450 && (x - self.x).abs() < 5.0 && (y - self.y).abs() < 5.0;
        // A third click starts over rather than making a second double click.
        *self = if double { Last::default() } else { Last { at: Instant::now(), x, y } };
        double
    }
}

/// Fire `dblclick` at `node`; (x, y) are the page's client coordinates.
pub fn dblclick(js: &mut Js, node: NodeId, x: f32, y: f32) -> bool {
    js.user_event(node as i64, "dblclick", &format!("{{button:0,detail:2,clientX:{x},clientY:{y}}}"))
}

/// Fire `contextmenu` at `node` before the browser's menu opens. False if the page
/// cancelled it, so the browser shows no menu.
pub fn contextmenu(js: &mut Js, node: NodeId, x: f32, y: f32) -> bool {
    js.user_event(node as i64, "contextmenu", &format!("{{button:2,clientX:{x},clientY:{y}}}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_goes_to_the_common_ancestor() {
        let d = crate::html::parse("<div id=a><span id=b>x</span><span id=c>y</span></div><p id=d>z</p>");
        let id = |name: &str| d.descendants(Dom::ROOT).into_iter().find(|&n| d.attr(n, "id") == Some(name)).unwrap();
        let (a, b, c, p) = (id("a"), id("b"), id("c"), id("d"));
        assert_eq!(target(&d, Some(b), Some(b)), Some(b));
        assert_eq!(target(&d, Some(b), Some(c)), Some(a));
        assert_eq!(target(&d, Some(b), Some(a)), Some(a));
        assert_ne!(target(&d, Some(b), Some(p)), Some(b));
        assert_eq!(target(&d, Some(b), None), None);
    }

    #[test]
    fn second_click_nearby_is_a_double_click() {
        let mut l = Last::default();
        assert!(!l.record(10.0, 10.0));
        assert!(l.record(12.0, 11.0));
        assert!(!l.record(12.0, 11.0));
        assert!(!l.record(80.0, 11.0));
    }
}
