//! The "JavaScript JIT" setting (task J4): whether Boa's baseline JIT compiles busy functions.
//!
//! The JIT is built in by default (the `boa_jit` feature). `BOA_JIT=off|on|always` in the
//! environment wins over the setting, so tests and bug reports can pin the mode.

/// Turns the JIT on or off for this process, unless `BOA_JIT` already chose. Call before any
/// script runs: functions compiled earlier keep their machine code.
pub fn apply(enabled: bool) {
    #[cfg(feature = "boa_jit")]
    {
        use boa_engine::jit::{JitMode, set_mode};
        if std::env::var_os("BOA_JIT").is_none() {
            set_mode(if enabled { JitMode::On } else { JitMode::Off });
        }
    }
    #[cfg(not(feature = "boa_jit"))]
    let _ = enabled;
}

#[cfg(all(test, feature = "boa_jit"))]
mod tests {
    use boa_engine::jit::{JitMode, mode, set_mode};

    // One test, since the mode is process-wide and tests run in parallel threads.
    #[test]
    fn the_setting_turns_the_jit_off_and_on_unless_boa_jit_is_set() {
        let before = mode();
        if std::env::var_os("BOA_JIT").is_none() {
            super::apply(false);
            assert_eq!(mode(), JitMode::Off);
            super::apply(true);
            assert_eq!(mode(), JitMode::On);
        } else {
            set_mode(JitMode::Always);
            super::apply(false);
            assert_eq!(mode(), JitMode::Always, "BOA_JIT in the environment wins");
        }
        set_mode(before);
    }

    #[test]
    fn the_setting_is_saved_and_shown() {
        let mut s = crate::settings::Settings::default();
        assert!(s.js_jit, "on by default");
        crate::settings::apply_for_test(&mut s, "js_jit", "0");
        assert!(!s.js_jit);
        let page = crate::settings::page("");
        assert!(page.contains("name=\"js_jit\""), "about:settings has the switch");
    }
}
