//! The browser window, on winit (X11, native Wayland, Windows and macOS) with softbuffer for
//! software drawing.
//!
//! It keeps the small polling interface the window loop in main.rs was written against
//! (`update`, `get_keys_pressed`, `get_mouse_pos`, an input callback for typed text...): each
//! `update` pumps winit's events and collects them for the next frame. On top of that,
//! winit brings keyboard shortcuts that follow the keyboard layout (Ctrl+Z on AZERTY), text
//! from input methods (IME) for languages typed with composition, and the screen's HiDPI
//! scale.

use std::collections::HashSet;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::{CursorIcon, WindowId};

/// Keys, by position on a US keyboard (letters follow the active layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9,
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15,
    Down, Left, Right, Up,
    Apostrophe, Backquote, Backslash, Comma, Equal, LeftBracket, Minus, Period, RightBracket, Semicolon, Slash,
    Backspace, Delete, End, Enter, Escape, Home, Insert, Menu, PageDown, PageUp, Pause, Space, Tab,
    NumLock, CapsLock, ScrollLock,
    LeftShift, RightShift, LeftCtrl, RightCtrl,
    NumPad0, NumPad1, NumPad2, NumPad3, NumPad4, NumPad5, NumPad6, NumPad7, NumPad8, NumPad9,
    NumPadDot, NumPadSlash, NumPadAsterisk, NumPadMinus, NumPadPlus, NumPadEnter,
    LeftAlt, RightAlt, LeftSuper, RightSuper,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRepeat {
    Yes,
    No,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MouseMode {
    /// Positions outside the window too.
    Pass,
    /// Clamped to the window.
    Clamp,
    /// None while the pointer is outside the window.
    Discard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CursorStyle {
    Arrow,
    Ibeam,
    Crosshair,
    /// The pointing hand (links, buttons).
    ClosedHand,
    OpenHand,
    ResizeLeftRight,
    ResizeUpDown,
    ResizeAll,
}

/// Receives typed text and key changes as they happen (between frames too).
pub trait InputCallback {
    /// A typed character (never a control character).
    fn add_char(&mut self, c: u32);
    /// A key went down (also when held and repeating) or up.
    fn set_key_state(&mut self, _key: Key, _down: bool) {}
}

#[derive(Clone, Copy, Debug)]
pub struct WindowOptions {
    pub resize: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions { resize: false }
    }
}

fn key_from_code(c: KeyCode) -> Key {
    use Key::*;
    match c {
        KeyCode::Digit0 => Key0, KeyCode::Digit1 => Key1, KeyCode::Digit2 => Key2, KeyCode::Digit3 => Key3, KeyCode::Digit4 => Key4,
        KeyCode::Digit5 => Key5, KeyCode::Digit6 => Key6, KeyCode::Digit7 => Key7, KeyCode::Digit8 => Key8, KeyCode::Digit9 => Key9,
        KeyCode::KeyA => A, KeyCode::KeyB => B, KeyCode::KeyC => C, KeyCode::KeyD => D, KeyCode::KeyE => E, KeyCode::KeyF => F,
        KeyCode::KeyG => G, KeyCode::KeyH => H, KeyCode::KeyI => I, KeyCode::KeyJ => J, KeyCode::KeyK => K, KeyCode::KeyL => L,
        KeyCode::KeyM => M, KeyCode::KeyN => N, KeyCode::KeyO => O, KeyCode::KeyP => P, KeyCode::KeyQ => Q, KeyCode::KeyR => R,
        KeyCode::KeyS => S, KeyCode::KeyT => T, KeyCode::KeyU => U, KeyCode::KeyV => V, KeyCode::KeyW => W, KeyCode::KeyX => X,
        KeyCode::KeyY => Y, KeyCode::KeyZ => Z,
        KeyCode::F1 => F1, KeyCode::F2 => F2, KeyCode::F3 => F3, KeyCode::F4 => F4, KeyCode::F5 => F5, KeyCode::F6 => F6,
        KeyCode::F7 => F7, KeyCode::F8 => F8, KeyCode::F9 => F9, KeyCode::F10 => F10, KeyCode::F11 => F11, KeyCode::F12 => F12,
        KeyCode::F13 => F13, KeyCode::F14 => F14, KeyCode::F15 => F15,
        KeyCode::ArrowDown => Down, KeyCode::ArrowLeft => Left, KeyCode::ArrowRight => Right, KeyCode::ArrowUp => Up,
        KeyCode::Quote => Apostrophe, KeyCode::Backquote => Backquote, KeyCode::Backslash => Backslash, KeyCode::Comma => Comma,
        KeyCode::Equal => Equal, KeyCode::BracketLeft => LeftBracket, KeyCode::Minus => Minus, KeyCode::Period => Period,
        KeyCode::BracketRight => RightBracket, KeyCode::Semicolon => Semicolon, KeyCode::Slash => Slash,
        KeyCode::Backspace => Backspace, KeyCode::Delete => Delete, KeyCode::End => End, KeyCode::Enter => Enter,
        KeyCode::Escape => Escape, KeyCode::Home => Home, KeyCode::Insert => Insert, KeyCode::ContextMenu => Menu,
        KeyCode::PageDown => PageDown, KeyCode::PageUp => PageUp, KeyCode::Pause => Pause, KeyCode::Space => Space, KeyCode::Tab => Tab,
        KeyCode::NumLock => NumLock, KeyCode::CapsLock => CapsLock, KeyCode::ScrollLock => ScrollLock,
        KeyCode::ShiftLeft => LeftShift, KeyCode::ShiftRight => RightShift, KeyCode::ControlLeft => LeftCtrl, KeyCode::ControlRight => RightCtrl,
        KeyCode::Numpad0 => NumPad0, KeyCode::Numpad1 => NumPad1, KeyCode::Numpad2 => NumPad2, KeyCode::Numpad3 => NumPad3,
        KeyCode::Numpad4 => NumPad4, KeyCode::Numpad5 => NumPad5, KeyCode::Numpad6 => NumPad6, KeyCode::Numpad7 => NumPad7,
        KeyCode::Numpad8 => NumPad8, KeyCode::Numpad9 => NumPad9, KeyCode::NumpadDecimal => NumPadDot, KeyCode::NumpadDivide => NumPadSlash,
        KeyCode::NumpadMultiply => NumPadAsterisk, KeyCode::NumpadSubtract => NumPadMinus, KeyCode::NumpadAdd => NumPadPlus,
        KeyCode::NumpadEnter => NumPadEnter,
        KeyCode::AltLeft => LeftAlt, KeyCode::AltRight => RightAlt, KeyCode::SuperLeft => LeftSuper, KeyCode::SuperRight => RightSuper,
        _ => Unknown,
    }
}

/// A letter key as the active layout labels it (so Ctrl+Z is Z on an AZERTY keyboard too).
fn letter(c: char) -> Option<Key> {
    use Key::*;
    const LETTERS: [Key; 26] = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    let c = c.to_ascii_lowercase();
    c.is_ascii_lowercase().then(|| LETTERS[(c as u8 - b'a') as usize])
}

/// The window's current HiDPI scale (0 until a window exists), for zoom::device_scale.
static SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn reported_scale() -> Option<f32> {
    let s = f32::from_bits(SCALE.load(std::sync::atomic::Ordering::Relaxed));
    (s > 0.0).then_some(s)
}

/// Event state collected between frames.
struct State {
    attrs: Option<winit::window::WindowAttributes>,
    window: Option<Arc<winit::window::Window>>,
    error: Option<String>,
    open: bool,
    /// The window has keyboard focus.
    focused: bool,
    down: HashSet<Key>,
    /// Keys pressed this frame, and whether each was a held-key repeat.
    pressed: Vec<(Key, bool)>,
    released: Vec<Key>,
    mouse: Option<(f32, f32)>,
    last_mouse: (f32, f32),
    buttons: [bool; 3],
    /// Buttons pressed during this frame: a quick click (down and up between two frames)
    /// still reads as down for one frame.
    clicked: [bool; 3],
    wheel: (f32, f32),
    ctrl_or_alt: bool,
    callback: Option<Box<dyn InputCallback>>,
}

impl State {
    fn key(&mut self, key: Key, down: bool, repeat: bool) {
        if down {
            self.down.insert(key);
            self.pressed.push((key, repeat));
        } else {
            self.down.remove(&key);
            self.released.push(key);
        }
        if let Some(cb) = self.callback.as_mut() {
            cb.set_key_state(key, down);
        }
    }

    fn text(&mut self, s: &str) {
        if let Some(cb) = self.callback.as_mut() {
            for c in s.chars().filter(|c| !c.is_control()) {
                cb.add_char(c as u32);
            }
        }
    }
}

impl ApplicationHandler for State {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let Some(attrs) = self.attrs.take() else { return };
        match el.create_window(attrs) {
            Ok(w) => {
                w.set_ime_allowed(true);
                SCALE.store((w.scale_factor() as f32).to_bits(), std::sync::atomic::Ordering::Relaxed);
                self.window = Some(Arc::new(w));
                self.open = true;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if std::env::var_os("IRONSWIFT_DEBUG_EVENTS").is_some() {
            eprintln!("[event] {event:?}");
        }
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => self.open = false,
            WindowEvent::Focused(true) => self.focused = true,
            WindowEvent::Focused(false) => {
                self.focused = false;
                // Keys let go while another window had focus would otherwise stay "down".
                let held: Vec<Key> = self.down.drain().collect();
                for k in held {
                    self.released.push(k);
                    if let Some(cb) = self.callback.as_mut() {
                        cb.set_key_state(k, false);
                    }
                }
                self.buttons = [false; 3];
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                if is_synthetic {
                    return;
                }
                let mut key = match event.physical_key {
                    PhysicalKey::Code(c) => key_from_code(c),
                    PhysicalKey::Unidentified(_) => Key::Unknown,
                };
                // Letters follow the keyboard layout, so shortcuts land on the labelled key.
                if let winit::keyboard::Key::Character(s) = &event.key_without_modifiers() {
                    let mut chars = s.chars();
                    if let (Some(c), None) = (chars.next(), chars.next()) {
                        if let Some(k) = letter(c) {
                            key = k;
                        }
                    }
                }
                let down = event.state == ElementState::Pressed;
                if matches!(key, Key::LeftCtrl | Key::RightCtrl | Key::LeftAlt | Key::RightAlt) {
                    self.ctrl_or_alt = down || self.down.iter().any(|k| *k != key && matches!(k, Key::LeftCtrl | Key::RightCtrl | Key::LeftAlt | Key::RightAlt));
                }
                if key != Key::Unknown {
                    self.key(key, down, event.repeat);
                }
                // Typed text (not Ctrl/Alt shortcuts, whose "text" is a control code).
                if down && !self.ctrl_or_alt {
                    if let Some(t) = event.text.as_deref() {
                        let t = t.to_string();
                        self.text(&t);
                    }
                }
            }
            // Text composed in an input method (Chinese, Japanese, Korean...).
            WindowEvent::Ime(Ime::Commit(t)) => self.text(&t),
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.ctrl_or_alt = s.control_key() || s.alt_key();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x as f32, position.y as f32);
                self.mouse = Some(p);
                self.last_mouse = p;
            }
            WindowEvent::CursorLeft { .. } => self.mouse = None,
            WindowEvent::MouseInput { state, button, .. } => {
                let i = match button {
                    winit::event::MouseButton::Left => 0,
                    winit::event::MouseButton::Middle => 1,
                    winit::event::MouseButton::Right => 2,
                    _ => return,
                };
                self.buttons[i] = state == ElementState::Pressed;
                if self.buttons[i] {
                    self.clicked[i] = true;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // The window loop expects about 2 per wheel click (as X11 reported them).
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 2.0, y * 2.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32 / 22.0, p.y as f32 / 22.0),
                };
                self.wheel.0 += x;
                self.wheel.1 += y;
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                SCALE.store((scale_factor as f32).to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

type Surface = softbuffer::Surface<Arc<winit::window::Window>, Arc<winit::window::Window>>;

pub struct Window {
    event_loop: EventLoop<()>,
    state: State,
    surface: Option<Surface>,
    frame: Duration,
    last_frame: Instant,
}

impl Window {
    pub fn new(title: &str, width: usize, height: usize, opts: WindowOptions) -> Result<Window, String> {
        // The window loop runs on the browser's own thread (it needs a big stack), not main.
        let mut builder = EventLoop::builder();
        #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_any_thread(true);
        }
        #[cfg(windows)]
        {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        let event_loop = builder.build().map_err(|e| e.to_string())?;
        let attrs = winit::window::Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::PhysicalSize::new(width as u32, height as u32))
            .with_resizable(opts.resize)
            .with_window_icon(app_icon());
        #[cfg(all(target_os = "linux", not(target_os = "android")))]
        let attrs = {
            use winit::platform::wayland::WindowAttributesExtWayland;
            WindowAttributesExtWayland::with_name(attrs, "ironswift", "ironswift")
        };
        let mut w = Window {
            event_loop,
            state: State {
                attrs: Some(attrs),
                window: None,
                error: None,
                open: false,
                focused: true,
                down: HashSet::new(),
                pressed: Vec::new(),
                released: Vec::new(),
                mouse: None,
                last_mouse: (0.0, 0.0),
                buttons: [false; 3],
                clicked: [false; 3],
                wheel: (0.0, 0.0),
                ctrl_or_alt: false,
                callback: None,
            },
            surface: None,
            frame: Duration::from_millis(16),
            last_frame: Instant::now(),
        };
        let start = Instant::now();
        while w.state.window.is_none() && w.state.error.is_none() && start.elapsed() < Duration::from_secs(5) {
            w.event_loop.pump_app_events(Some(Duration::from_millis(10)), &mut w.state);
        }
        if let Some(e) = w.state.error.take() {
            return Err(e);
        }
        if w.state.window.is_none() {
            return Err("the window system didn't open a window".into());
        }
        Ok(w)
    }

    fn win(&self) -> &winit::window::Window {
        self.state.window.as_deref().expect("window")
    }

    pub fn is_open(&self) -> bool {
        self.state.open
    }

    /// Does the window have keyboard focus?
    pub fn is_active(&mut self) -> bool {
        self.state.focused
    }

    /// The drawing area in device pixels.
    pub fn get_size(&self) -> (usize, usize) {
        let s = self.win().inner_size();
        (s.width as usize, s.height as usize)
    }

    pub fn set_title(&mut self, title: &str) {
        self.win().set_title(title);
    }

    pub fn set_target_fps(&mut self, fps: usize) {
        self.frame = if fps == 0 { Duration::ZERO } else { Duration::from_secs_f64(1.0 / fps as f64) };
    }

    pub fn set_input_callback(&mut self, cb: Box<dyn InputCallback>) {
        self.state.callback = Some(cb);
    }

    pub fn set_cursor_style(&mut self, c: CursorStyle) {
        self.win().set_cursor(match c {
            CursorStyle::Arrow => CursorIcon::Default,
            CursorStyle::Ibeam => CursorIcon::Text,
            CursorStyle::Crosshair => CursorIcon::Crosshair,
            CursorStyle::ClosedHand => CursorIcon::Pointer,
            CursorStyle::OpenHand => CursorIcon::Grab,
            CursorStyle::ResizeLeftRight => CursorIcon::EwResize,
            CursorStyle::ResizeUpDown => CursorIcon::NsResize,
            CursorStyle::ResizeAll => CursorIcon::Move,
        });
    }

    pub fn is_key_down(&self, k: Key) -> bool {
        self.state.down.contains(&k)
    }

    pub fn get_keys(&self) -> Vec<Key> {
        self.state.down.iter().copied().collect()
    }

    pub fn get_keys_pressed(&self, repeat: KeyRepeat) -> Vec<Key> {
        self.state.pressed.iter().filter(|(_, r)| repeat == KeyRepeat::Yes || !r).map(|(k, _)| *k).collect()
    }

    pub fn get_keys_released(&self) -> Vec<Key> {
        self.state.released.clone()
    }

    pub fn get_mouse_pos(&self, mode: MouseMode) -> Option<(f32, f32)> {
        match mode {
            MouseMode::Discard => self.state.mouse,
            MouseMode::Pass => Some(self.state.mouse.unwrap_or(self.state.last_mouse)),
            MouseMode::Clamp => {
                let (w, h) = self.get_size();
                let (x, y) = self.state.mouse.unwrap_or(self.state.last_mouse);
                Some((x.clamp(0.0, w as f32 - 1.0), y.clamp(0.0, h as f32 - 1.0)))
            }
        }
    }

    pub fn get_mouse_down(&self, b: MouseButton) -> bool {
        let i = match b {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        };
        self.state.buttons[i] || self.state.clicked[i]
    }

    /// How far the wheel turned since the last frame (about 2 per click; up is positive).
    pub fn get_scroll_wheel(&self) -> Option<(f32, f32)> {
        let w = self.state.wheel;
        (w.0 != 0.0 || w.1 != 0.0).then_some(w)
    }

    /// Wait for the frame rate, then collect the window's events for the next frame.
    pub fn update(&mut self) {
        // Last frame's input has been seen; collect this frame's.
        self.state.pressed.clear();
        self.state.released.clear();
        self.state.wheel = (0.0, 0.0);
        self.state.clicked = [false; 3];
        let next = self.last_frame + self.frame;
        let now = Instant::now();
        if next > now {
            // Wait out the frame, taking events as they come.
            self.pump(Some(next - now));
            while Instant::now() < next {
                self.pump(Some(next - Instant::now()));
            }
        }
        self.last_frame = Instant::now();
        self.pump(Some(Duration::ZERO));
    }

    fn pump(&mut self, timeout: Option<Duration>) {
        let status = self.event_loop.pump_app_events(timeout, &mut self.state);
        if let winit::platform::pump_events::PumpStatus::Exit(_) = status {
            self.state.open = false;
        }
    }

    /// Show `buf` (0RGB pixels, `w`×`h`), then collect events like `update`.
    pub fn update_with_buffer(&mut self, buf: &[u32], w: usize, h: usize) -> Result<(), String> {
        self.present(buf, w, h)?;
        self.update();
        Ok(())
    }

    fn present(&mut self, buf: &[u32], w: usize, h: usize) -> Result<(), String> {
        let win = self.state.window.clone().ok_or("no window")?;
        if self.surface.is_none() {
            let ctx = softbuffer::Context::new(win.clone()).map_err(|e| e.to_string())?;
            self.surface = Some(softbuffer::Surface::new(&ctx, win.clone()).map_err(|e| e.to_string())?);
        }
        let (ww, wh) = self.get_size();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(ww as u32), NonZeroU32::new(wh as u32)) else { return Ok(()) };
        let surface = self.surface.as_mut().unwrap();
        surface.resize(nw, nh).map_err(|e| e.to_string())?;
        let mut out = surface.buffer_mut().map_err(|e| e.to_string())?;
        let cw = w.min(ww);
        for y in 0..wh {
            let dst = &mut out[y * ww..(y + 1) * ww];
            if y < h {
                dst[..cw].copy_from_slice(&buf[y * w..y * w + cw]);
                dst[cw..].fill(0xFFFFFF);
            } else {
                dst.fill(0xFFFFFF);
            }
        }
        win.pre_present_notify();
        out.present().map_err(|e| e.to_string())
    }
}

impl raw_window_handle::HasWindowHandle for Window {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        self.win().window_handle()
    }
}

impl raw_window_handle::HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        self.win().display_handle()
    }
}

/// The app icon (taskbar, Alt+Tab) for X11 and Windows; Wayland takes it from the installed
/// desktop file, macOS from the app bundle.
fn app_icon() -> Option<winit::window::Icon> {
    let img = image::load_from_memory(include_bytes!("../assets/icon/png/ironswift-64.png")).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map() {
        assert_eq!(key_from_code(KeyCode::KeyA), Key::A);
        assert_eq!(key_from_code(KeyCode::Digit7), Key::Key7);
        assert_eq!(key_from_code(KeyCode::NumpadAdd), Key::NumPadPlus);
        assert_eq!(key_from_code(KeyCode::ArrowUp), Key::Up);
        assert_eq!(letter('Z'), Some(Key::Z));
        assert_eq!(letter('é'), None);
    }
}
