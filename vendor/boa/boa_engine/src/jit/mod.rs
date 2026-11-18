//! Baseline JIT for Boa (Ironswift change; plan in `docs/JIT.md`).
//!
//! This is the groundwork (task J1): it decides *when* a function is hot, *whether* the first
//! version of the compiler could take it, and owns the Cranelift backend and its executable
//! memory. Nothing here runs compiled JavaScript yet; J2 adds the compiler that turns an
//! [`TierState::Eligible`] code block into machine code.
//!
//! The whole module only exists with the `jit` cargo feature (off by default).

#[cfg(feature = "jsvalue-enum")]
compile_error!("the baseline JIT reads NaN-boxed values; build without `jsvalue-enum`");

mod backend;
mod compiler;
mod counters;
mod helpers;
mod mode;
mod runtime;
mod support;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_j3;

pub use backend::JitBackend;
pub(crate) use counters::JitCounters;
pub use counters::TierState;
pub use mode::{JitMode, mode, set_mode};
pub use support::Rejection;
pub(crate) use runtime::{ContextJit, enter, enter_after_return, enter_at_call};

use crate::vm::CodeBlock;
use boa_gc::Gc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A function tiers up after this many calls (`BOA_JIT=on`, the default when the feature is
/// built in).
pub const CALL_THRESHOLD: u32 = 1_000;

/// A function tiers up after its loops have run this many iterations in total, even if it was
/// only called once (the entry for that case is a loop header, see `docs/JIT.md`).
pub const LOOP_THRESHOLD: u32 = 10_000;

static CONSIDERED: AtomicU64 = AtomicU64::new(0);
static ELIGIBLE: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);

/// How many hot functions the JIT has looked at since the process started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JitStats {
    /// Code blocks that crossed a threshold and were scanned.
    pub considered: u64,
    /// Of those, how many only use opcodes the first compiler handles.
    pub eligible: u64,
    /// Of those, how many use something it doesn't (they stay interpreted).
    pub rejected: u64,
}

/// Returns the process-wide tier-up statistics.
#[must_use]
pub fn stats() -> JitStats {
    JitStats {
        considered: CONSIDERED.load(Ordering::Relaxed),
        eligible: ELIGIBLE.load(Ordering::Relaxed),
        rejected: REJECTED.load(Ordering::Relaxed),
    }
}

/// Called by the VM when a code block's call or loop counter crosses its threshold.
///
/// Runs at most once per code block: it scans the bytecode and compiles it if the first
/// compiler can take it.
#[cold]
#[inline(never)]
pub(crate) fn tier_up(code: &Gc<CodeBlock>, context: &crate::Context) {
    if code.jit.state() != TierState::Interpreted || mode() == JitMode::Off {
        return;
    }
    CONSIDERED.fetch_add(1, Ordering::Relaxed);
    match support::scan(code) {
        Ok(()) => {
            ELIGIBLE.fetch_add(1, Ordering::Relaxed);
            code.jit.set_state(TierState::Eligible);
            runtime::compile(code, context);
        }
        Err(rejection) => {
            REJECTED.fetch_add(1, Ordering::Relaxed);
            code.jit.set_state(TierState::Rejected(rejection));
        }
    }
}
