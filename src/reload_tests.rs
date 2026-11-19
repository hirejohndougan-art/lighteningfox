//! F5 while a page loads starts that page again (it used to stop the load: a tab restored from
//! the last session, still loading, was left on about:blank).

use crate::{Go, reload_target};

#[test]
fn reload_while_loading_reloads_the_page_being_loaded() {
    let waking = Go::To("https://dickinsonhistoricalsociety.org/".into());
    assert_eq!(reload_target(Some(&waking), "about:blank"), "https://dickinsonhistoricalsociety.org/");
    assert_eq!(reload_target(None, "https://example.com/"), "https://example.com/");
    assert_eq!(reload_target(Some(&Go::Back), "https://example.com/"), "https://example.com/");
}
