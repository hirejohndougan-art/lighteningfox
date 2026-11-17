//! Turning the window's mouse, keyboard and wheel input into DOM events for the page's
//! scripts (mousedown, mousemove, mouseover…, keydown, keyup, wheel, scroll).
//! The JavaScript side (userevents.js) builds the event objects and dispatches them.

use crate::dom::NodeId;
use crate::js::Js;
use crate::window::Key;

/// Modifier keys held during an event.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Mods {
    fn js(&self) -> String {
        format!("ctrlKey:{},shiftKey:{},altKey:{},metaKey:false", self.ctrl, self.shift, self.alt)
    }
}

/// The mouse as the page has seen it: what it was over, where, and which buttons were down.
#[derive(Default)]
pub struct Pointer {
    over: Option<NodeId>,
    pos: (f32, f32),
    down: [bool; 3],
    started: bool,
    raw: Option<(f32, f32, [bool; 3])>,
}

impl Pointer {
    /// Has the raw mouse state changed since the last call? (Saves a hit test per frame.)
    pub fn changed(&mut self, x: f32, y: f32, down: [bool; 3]) -> bool {
        let raw = (x, y, down);
        let c = !self.started || self.raw != Some(raw);
        self.raw = Some(raw);
        c
    }

    /// A new page: nothing is under the mouse yet as far as it knows.
    pub fn reset(&mut self) {
        *self = Pointer::default();
    }

    /// Called every frame with the mouse over the page: `node` is under it, (x, y) in the
    /// page's CSS pixels relative to the window, `down` = left, middle, right buttons.
    /// Fires mouseover/out, enter/leave and mousemove when it moved, and mousedown/mouseup
    /// when a button changed. Returns true if a mousedown was cancelled by the page.
    pub fn update(&mut self, js: &mut Js, node: Option<NodeId>, x: f32, y: f32, down: [bool; 3], mods: Mods) -> bool {
        let m = mods.js();
        if !self.started || node != self.over || (x, y) != self.pos {
            js.pointer_move(node, x, y, &format!("{{{m}}}"));
            self.over = node;
            self.pos = (x, y);
            self.started = true;
        }
        let mut cancelled = false;
        // DOM button numbers: 0 left, 1 middle, 2 right.
        for (i, &d) in down.iter().enumerate() {
            if d == self.down[i] {
                continue;
            }
            self.down[i] = d;
            let Some(n) = node else { continue };
            let init = format!("{{button:{i},clientX:{x},clientY:{y},{m}}}");
            let ok = js.user_event(n as i64, if d { "mousedown" } else { "mouseup" }, &init);
            cancelled |= d && !ok;
            if !d && i == 1 {
                js.user_event(n as i64, "auxclick", &init);
            }
        }
        cancelled
    }

    /// The mouse left the page area (or the window).
    pub fn left(&mut self, js: &mut Js) {
        if self.over.is_some() {
            js.pointer_move(None, self.pos.0, self.pos.1, "{}");
            self.over = None;
        }
    }
}

/// The wheel turned over `node`. Returns false if the page cancelled it (no scrolling).
pub fn wheel(js: &mut Js, node: Option<NodeId>, x: f32, y: f32, dx: f32, dy: f32, mods: Mods) -> bool {
    let Some(n) = node else { return true };
    js.user_event(n as i64, "wheel", &format!("{{deltaX:{dx},deltaY:{dy},deltaMode:0,clientX:{x},clientY:{y},{}}}", mods.js()))
}

/// A scrollable box was scrolled by the user.
pub fn scrolled(js: &mut Js, node: NodeId) {
    js.user_event(node as i64, "scroll", "{}");
}

/// A key went down (`up` = false) or up, at the focused element. Returns false if the page
/// cancelled it, so the browser shouldn't act on the key.
pub fn key(js: &mut Js, k: Key, up: bool, repeat: bool, mods: Mods) -> bool {
    let (key, code) = names(k, mods.shift);
    let init = format!("{{key:{},code:{},repeat:{repeat},location:{},{}}}", quote(&key), quote(&code), location(k), mods.js());
    let ok = js.user_event(-1, if up { "keyup" } else { "keydown" }, &init);
    // keypress (deprecated, but still used) follows keydown for keys that type a character.
    if ok && !up && key.chars().count() == 1 && !mods.ctrl && !mods.alt {
        return js.user_event(-1, "keypress", &init);
    }
    ok
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn location(k: Key) -> u8 {
    match k {
        Key::LeftShift | Key::LeftCtrl | Key::LeftAlt | Key::LeftSuper => 1,
        Key::RightShift | Key::RightCtrl | Key::RightAlt | Key::RightSuper => 2,
        Key::NumPad0 | Key::NumPad1 | Key::NumPad2 | Key::NumPad3 | Key::NumPad4 | Key::NumPad5 | Key::NumPad6 | Key::NumPad7 | Key::NumPad8 | Key::NumPad9 => 3,
        Key::NumPadDot | Key::NumPadSlash | Key::NumPadAsterisk | Key::NumPadMinus | Key::NumPadPlus | Key::NumPadEnter => 3,
        _ => 0,
    }
}

/// The DOM `key` and `code` for a key (US keyboard layout).
pub fn names(k: Key, shift: bool) -> (String, String) {
    use Key::*;
    let letters = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    if let Some(i) = letters.iter().position(|&l| l == k) {
        let c = (b'a' + i as u8) as char;
        let key = if shift { c.to_ascii_uppercase() } else { c };
        return (key.to_string(), format!("Key{}", c.to_ascii_uppercase()));
    }
    let digits = [Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9];
    if let Some(i) = digits.iter().position(|&d| d == k) {
        let key = if shift { [")", "!", "@", "#", "$", "%", "^", "&", "*", "("][i].to_string() } else { i.to_string() };
        return (key, format!("Digit{i}"));
    }
    let pads = [NumPad0, NumPad1, NumPad2, NumPad3, NumPad4, NumPad5, NumPad6, NumPad7, NumPad8, NumPad9];
    if let Some(i) = pads.iter().position(|&d| d == k) {
        return (i.to_string(), format!("Numpad{i}"));
    }
    let fkeys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    if let Some(i) = fkeys.iter().position(|&f| f == k) {
        let n = format!("F{}", i + 1);
        return (n.clone(), n);
    }
    let (key, code): (&str, &str) = match k {
        Space => (" ", "Space"),
        Enter => ("Enter", "Enter"),
        NumPadEnter => ("Enter", "NumpadEnter"),
        Tab => ("Tab", "Tab"),
        Backspace => ("Backspace", "Backspace"),
        Delete => ("Delete", "Delete"),
        Insert => ("Insert", "Insert"),
        Escape => ("Escape", "Escape"),
        Left => ("ArrowLeft", "ArrowLeft"),
        Right => ("ArrowRight", "ArrowRight"),
        Up => ("ArrowUp", "ArrowUp"),
        Down => ("ArrowDown", "ArrowDown"),
        Home => ("Home", "Home"),
        End => ("End", "End"),
        PageUp => ("PageUp", "PageUp"),
        PageDown => ("PageDown", "PageDown"),
        LeftShift => ("Shift", "ShiftLeft"),
        RightShift => ("Shift", "ShiftRight"),
        LeftCtrl => ("Control", "ControlLeft"),
        RightCtrl => ("Control", "ControlRight"),
        LeftAlt => ("Alt", "AltLeft"),
        RightAlt => ("Alt", "AltRight"),
        LeftSuper => ("Meta", "MetaLeft"),
        RightSuper => ("Meta", "MetaRight"),
        CapsLock => ("CapsLock", "CapsLock"),
        NumLock => ("NumLock", "NumLock"),
        ScrollLock => ("ScrollLock", "ScrollLock"),
        Pause => ("Pause", "Pause"),
        Menu => ("ContextMenu", "ContextMenu"),
        Minus => (if shift { "_" } else { "-" }, "Minus"),
        Equal => (if shift { "+" } else { "=" }, "Equal"),
        LeftBracket => (if shift { "{" } else { "[" }, "BracketLeft"),
        RightBracket => (if shift { "}" } else { "]" }, "BracketRight"),
        Backslash => (if shift { "|" } else { "\\" }, "Backslash"),
        Semicolon => (if shift { ":" } else { ";" }, "Semicolon"),
        Apostrophe => (if shift { "\"" } else { "'" }, "Quote"),
        Comma => (if shift { "<" } else { "," }, "Comma"),
        Period => (if shift { ">" } else { "." }, "Period"),
        Slash => (if shift { "?" } else { "/" }, "Slash"),
        Backquote => (if shift { "~" } else { "`" }, "Backquote"),
        NumPadDot => (".", "NumpadDecimal"),
        NumPadSlash => ("/", "NumpadDivide"),
        NumPadAsterisk => ("*", "NumpadMultiply"),
        NumPadMinus => ("-", "NumpadSubtract"),
        NumPadPlus => ("+", "NumpadAdd"),
        _ => ("Unidentified", "Unidentified"),
    };
    (key.to_string(), code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(names(Key::A, false), ("a".into(), "KeyA".into()));
        assert_eq!(names(Key::A, true), ("A".into(), "KeyA".into()));
        assert_eq!(names(Key::Key1, true), ("!".into(), "Digit1".into()));
        assert_eq!(names(Key::Enter, false), ("Enter".into(), "Enter".into()));
        assert_eq!(names(Key::Space, false), (" ".into(), "Space".into()));
        assert_eq!(quote("\""), "\"\\\"\"");
    }
}
