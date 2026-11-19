//! Enter in text fields: since the window moved to winit (Nov 17) Enter's "\r" isn't passed on
//! as typed text, so the address bar, form fields and textareas must act on the Enter key.

use crate::{KeyPress, edit_keys, textedit::TextEdit};
use crate::window::Key;

fn press(key: Key) -> KeyPress {
    KeyPress { key, ctrl: false, shift: false, alt: false }
}

#[test]
fn the_enter_key_submits_a_single_line_field() {
    let mut ed = TextEdit::new("");
    let typed: Vec<char> = "google.com".chars().collect();
    let r = edit_keys(&mut ed, &[], &typed, false, false, &mut None);
    assert!(!r.enter);
    let r = edit_keys(&mut ed, &[press(Key::Enter)], &[], false, false, &mut None);
    assert!(r.enter, "Enter submits the address bar");
    assert_eq!(ed.text, "google.com");
    let r = edit_keys(&mut ed, &[press(Key::NumPadEnter)], &[], false, false, &mut None);
    assert!(r.enter, "so does the keypad's Enter");
}

#[test]
fn the_enter_key_starts_a_new_line_in_a_textarea_once() {
    let mut ed = TextEdit::new("a");
    ed.move_to(1, false);
    edit_keys(&mut ed, &[press(Key::Enter)], &[], false, true, &mut None);
    assert_eq!(ed.text, "a\n");
    // A window that sends the key and its "\r" too gets one new line, not two.
    edit_keys(&mut ed, &[press(Key::Enter)], &['\r'], false, true, &mut None);
    assert_eq!(ed.text, "a\n\n");
    // "\r" on its own (no key) still works.
    let r = edit_keys(&mut TextEdit::new("x"), &[], &['\r'], false, false, &mut None);
    assert!(r.enter);
}
