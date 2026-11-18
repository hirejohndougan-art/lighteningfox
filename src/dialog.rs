//! The top layer: modal `<dialog>`s (`showModal()`) and open popovers (`showPopover()`) are
//! drawn above the whole page in the order they were opened, a modal one over its
//! `::backdrop`. dialog.js keeps the open/close rules and events; this keeps which elements are
//! in the top layer (`Dom::top_layer`), so selectors (`:modal`, `:popover-open`) and layout can
//! see it.

use crate::dom::{Dom, NodeId};

/// A modal dialog (blocks the page behind it).
pub const MODAL: u8 = 0;
/// An open popover.
pub const POPOVER: u8 = 1;

/// Is `id` in the top layer as `kind`?
pub fn is(dom: &Dom, id: NodeId, kind: u8) -> bool {
    dom.top_layer.iter().any(|&(n, k)| n == id && k == kind)
}

pub mod script {
    use super::*;
    use crate::js::{mark_dirty, with_dom, with_dom_mut};
    use boa_engine::{Context, JsResult, JsValue};

    pub type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

    pub const NATIVES: &[(&str, usize, Native)] = &[("__topLayerAdd", 2, n_add), ("__topLayerRemove", 1, n_remove)];

    fn id_arg(a: &[JsValue], ctx: &mut Context) -> JsResult<Option<NodeId>> {
        let n = a.first().cloned().unwrap_or_default().to_number(ctx)?;
        Ok(with_dom(|d| (n >= 0.0 && (n as usize) < d.nodes.len()).then_some(n as usize)))
    }

    /// __topLayerAdd(id, kind): put the element on top (0 modal dialog, 1 popover).
    fn n_add(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = id_arg(a, ctx)? else { return Ok(JsValue::undefined()) };
        let kind = a.get(1).cloned().unwrap_or_default().to_number(ctx)? as u8;
        with_dom_mut(|d| {
            d.top_layer.retain(|&(n, _)| n != id);
            d.top_layer.push((id, kind.min(POPOVER)));
        });
        mark_dirty();
        Ok(JsValue::undefined())
    }

    /// __topLayerRemove(id).
    fn n_remove(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = id_arg(a, ctx)? else { return Ok(JsValue::undefined()) };
        with_dom_mut(|d| d.top_layer.retain(|&(n, _)| n != id));
        mark_dirty();
        Ok(JsValue::undefined())
    }
}
