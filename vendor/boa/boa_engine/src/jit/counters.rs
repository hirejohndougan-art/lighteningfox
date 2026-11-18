//! Per-code-block call and loop counters.

use super::{compiler::JitFn, mode, support::Rejection};
use std::cell::Cell;

/// Where a code block is on the way to machine code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TierState {
    /// Not hot yet (or the JIT is off).
    #[default]
    Interpreted,
    /// Hot, and every opcode is one the first compiler handles. J2 compiles these.
    Eligible,
    /// Compiled: calls and loop headers run machine code.
    Compiled,
    /// Hot, but it uses something the first compiler doesn't (or its code kept bailing out);
    /// it stays interpreted.
    Rejected(Rejection),
}

/// Counters kept in every [`CodeBlock`](crate::vm::CodeBlock). Two `Cell<u32>` bumps per call
/// or loop iteration, and only with the `jit` feature.
#[derive(Clone, Default)]
pub(crate) struct JitCounters {
    calls: Cell<u32>,
    loop_iterations: Cell<u32>,
    state: Cell<TierState>,
    code: Cell<Option<JitFn>>,
    entries: Cell<u32>,
    bailouts: Cell<u32>,
}

impl std::fmt::Debug for JitCounters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JitCounters")
            .field("calls", &self.calls.get())
            .field("loop_iterations", &self.loop_iterations.get())
            .field("state", &self.state.get())
            .field("compiled", &self.code.get().is_some())
            .finish()
    }
}

impl JitCounters {
    /// Counts a call. Returns `true` when the code block should tier up now.
    #[inline]
    pub(crate) fn note_call(&self) -> bool {
        let calls = self.calls.get().saturating_add(1);
        self.calls.set(calls);
        calls >= mode::call_threshold() && self.state.get() == TierState::Interpreted
    }

    /// Counts one loop iteration (`IncrementLoopIteration`). Returns `true` when the code
    /// block should tier up now.
    #[inline]
    pub(crate) fn note_loop_iteration(&self) -> bool {
        let iterations = self.loop_iterations.get().saturating_add(1);
        self.loop_iterations.set(iterations);
        iterations >= mode::loop_threshold() && self.state.get() == TierState::Interpreted
    }

    pub(crate) fn calls(&self) -> u32 {
        self.calls.get()
    }

    pub(crate) fn loop_iterations(&self) -> u32 {
        self.loop_iterations.get()
    }

    pub(crate) fn state(&self) -> TierState {
        self.state.get()
    }

    pub(crate) fn set_state(&self, state: TierState) {
        self.state.set(state);
    }

    #[inline]
    pub(crate) fn code(&self) -> Option<JitFn> {
        self.code.get()
    }

    pub(crate) fn set_code(&self, code: Option<JitFn>) {
        self.code.set(code);
    }

    /// Counts an entry into compiled code; returns the total.
    pub(crate) fn note_entry(&self) -> u32 {
        let n = self.entries.get().saturating_add(1);
        self.entries.set(n);
        n
    }

    /// Counts a bailout; returns the total.
    pub(crate) fn note_bailout(&self) -> u32 {
        let n = self.bailouts.get().saturating_add(1);
        self.bailouts.set(n);
        n
    }

    #[cfg(test)]
    pub(crate) fn bailouts(&self) -> u32 {
        self.bailouts.get()
    }
}
