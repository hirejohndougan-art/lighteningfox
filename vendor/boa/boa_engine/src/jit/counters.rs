//! Per-code-block call and loop counters.

use super::{mode, support::Rejection};
use std::cell::Cell;

/// Where a code block is on the way to machine code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TierState {
    /// Not hot yet (or the JIT is off).
    #[default]
    Interpreted,
    /// Hot, and every opcode is one the first compiler handles. J2 compiles these.
    Eligible,
    /// Hot, but it uses something the first compiler doesn't; it stays interpreted.
    Rejected(Rejection),
}

/// Counters kept in every [`CodeBlock`](crate::vm::CodeBlock). Two `Cell<u32>` bumps per call
/// or loop iteration, and only with the `jit` feature.
#[derive(Debug, Clone, Default)]
pub(crate) struct JitCounters {
    calls: Cell<u32>,
    loop_iterations: Cell<u32>,
    state: Cell<TierState>,
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
}
