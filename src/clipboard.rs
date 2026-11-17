//! The system clipboard for page scripts (navigator.clipboard).
//!
//! Writing is allowed only shortly after a click or key press on the page (the web's
//! "transient activation"), so a page can't quietly replace what you copied. Reading asks
//! first (the clipboard-read permission, handled in the prelude). Without a desktop
//! clipboard (headless runs, tests) the text is kept in memory instead.

use std::cell::RefCell;
use std::time::{Duration, Instant};

/// How long after a click a page may still write to the clipboard.
const ACTIVATION_WINDOW: Duration = Duration::from_secs(5);

thread_local! {
    static SYSTEM: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
    static TRIED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FALLBACK: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_GESTURE: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

fn with_system<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Option<R> {
    if std::env::var_os("IRONSWIFT_NO_SYSTEM_CLIPBOARD").is_some() {
        return None;
    }
    SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_none() && !TRIED.with(|t| t.replace(true)) {
            *s = arboard::Clipboard::new().ok();
        }
        s.as_mut().map(f)
    })
}

/// May the page write now? `activated` = a user's click or key press is being handled.
pub fn may_write(activated: bool) -> bool {
    if activated {
        LAST_GESTURE.with(|g| g.set(Some(Instant::now())));
        return true;
    }
    LAST_GESTURE.with(|g| g.get().is_some_and(|t| t.elapsed() <= ACTIVATION_WINDOW))
}

pub fn write(text: &str) {
    FALLBACK.with(|f| *f.borrow_mut() = text.to_string());
    with_system(|c| c.set_text(text.to_string()));
}

pub fn read() -> Option<String> {
    match with_system(|c| c.get_text()) {
        Some(Ok(t)) => Some(t),
        // Empty or non-text clipboard.
        Some(Err(_)) => Some(String::new()),
        None => Some(FALLBACK.with(|f| f.borrow().clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_needs_a_recent_gesture() {
        LAST_GESTURE.with(|g| g.set(None));
        assert!(!may_write(false));
        assert!(may_write(true));
        assert!(may_write(false), "still within the activation window");
        LAST_GESTURE.with(|g| g.set(Some(Instant::now() - Duration::from_secs(6))));
        assert!(!may_write(false));
    }

    #[test]
    fn text_round_trips_without_a_desktop() {
        unsafe { std::env::set_var("IRONSWIFT_NO_SYSTEM_CLIPBOARD", "1") };
        write("hello clipboard");
        assert_eq!(read().as_deref(), Some("hello clipboard"));
    }
}
