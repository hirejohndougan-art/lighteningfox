//! The `BOA_JIT` switch.

use std::sync::atomic::{AtomicU8, Ordering};

/// Whether hot functions are handed to the JIT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitMode {
    /// Never tier up; every function stays in the interpreter.
    Off,
    /// Tier up after [`super::CALL_THRESHOLD`] calls or [`super::LOOP_THRESHOLD`] loop
    /// iterations.
    On,
    /// Tier up on the first call or loop iteration. For testing the JIT against the
    /// interpreter (`BOA_JIT=always`).
    Always,
}

impl JitMode {
    /// Parses a `BOA_JIT` value. Unknown values mean [`JitMode::On`].
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "0" | "off" | "false" | "no" | "never" => Self::Off,
            "always" | "force" | "eager" => Self::Always,
            _ => Self::On,
        }
    }

    const fn to_u8(self) -> u8 {
        match self {
            Self::Off => 1,
            Self::On => 2,
            Self::Always => 3,
        }
    }

    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Off),
            2 => Some(Self::On),
            3 => Some(Self::Always),
            _ => None,
        }
    }
}

/// 0 = not decided yet (read `BOA_JIT` on first use).
static MODE: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    /// Tests run in parallel threads of one process, so they set the mode per thread.
    pub(super) static TEST_MODE: std::cell::Cell<Option<JitMode>> = const { std::cell::Cell::new(None) };
}

/// The current mode: whatever [`set_mode`] last set, else the `BOA_JIT` environment variable,
/// else [`JitMode::On`].
#[inline]
#[must_use]
pub fn mode() -> JitMode {
    #[cfg(test)]
    if let Some(mode) = TEST_MODE.with(std::cell::Cell::get) {
        return mode;
    }
    if let Some(mode) = JitMode::from_u8(MODE.load(Ordering::Relaxed)) {
        return mode;
    }
    let mode = std::env::var("BOA_JIT").map_or(JitMode::On, |v| JitMode::parse(&v));
    // Keep a value someone else set meanwhile.
    let _ = MODE.compare_exchange(0, mode.to_u8(), Ordering::Relaxed, Ordering::Relaxed);
    JitMode::from_u8(MODE.load(Ordering::Relaxed)).unwrap_or(mode)
}

/// Sets the mode for the whole process (the embedder's settings switch). Code blocks that
/// already tiered up keep their state.
pub fn set_mode(mode: JitMode) {
    MODE.store(mode.to_u8(), Ordering::Relaxed);
}

/// The call count at which a code block tiers up.
#[inline]
pub(super) fn call_threshold() -> u32 {
    match mode() {
        JitMode::Off => u32::MAX,
        JitMode::On => super::CALL_THRESHOLD,
        JitMode::Always => 1,
    }
}

/// The loop iteration count at which a code block tiers up.
#[inline]
pub(super) fn loop_threshold() -> u32 {
    match mode() {
        JitMode::Off => u32::MAX,
        JitMode::On => super::LOOP_THRESHOLD,
        JitMode::Always => 1,
    }
}
