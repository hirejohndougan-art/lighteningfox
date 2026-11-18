//! Running compiled code: compiling on tier-up, entering at the start of a call or at a loop
//! header, and giving up on functions that keep bailing out.

use super::backend::JitBackend;
use super::compiler::{self, JitFrame};
use super::{JitMode, TierState, mode};
use crate::{Context, JsValue, vm::CodeBlock, vm::Opcode};
use std::cell::RefCell;

thread_local! {
    /// The Cranelift module for this thread's code (a `Context` stays on its thread).
    static BACKEND: RefCell<Option<JitBackend>> = const { RefCell::new(None) };
}

/// After this many bailouts, a function that bails out on more than one entry in four stops
/// using its compiled code.
const BAILOUT_GRACE: u32 = 64;

/// Compiles an eligible code block; on failure it stays interpreted for good.
pub(crate) fn compile(code: &CodeBlock) {
    let result = BACKEND.with(|b| {
        let mut b = b.borrow_mut();
        if b.is_none() {
            *b = Some(JitBackend::new()?);
        }
        compiler::compile(b.as_mut().expect("set above"), code)
    });
    match result {
        Ok(f) => {
            code.jit.set_code(Some(f));
            code.jit.set_state(TierState::Compiled);
        }
        Err(e) => {
            if std::env::var_os("BOA_JIT_LOG").is_some() {
                eprintln!("[boa jit] {}: compile failed: {e}", code.name().to_std_string_escaped());
            }
            code.jit.set_state(TierState::Rejected(super::Rejection { reason: "compile failed", pc: None }));
        }
    }
}

/// Runs the current frame's compiled code from the frame's `pc`, if it has some. Afterwards the
/// frame's `pc` is where the interpreter goes on: a `Return` (accumulator set) or the
/// instruction compiled code handed back.
#[inline]
pub(crate) fn enter(context: &mut Context) {
    let frame = context.vm.frame();
    let Some(f) = frame.code_block.jit.code() else { return };
    if frame.construct() || cfg!(feature = "fuzz") {
        return;
    }
    enter_compiled(context, f);
}

#[inline(never)]
fn enter_compiled(context: &mut Context, f: compiler::JitFn) {
    if mode() == JitMode::Off {
        return;
    }
    let loop_max = context.vm.runtime_limits.loop_iteration_limit();
    let vm = &mut context.vm;
    let Some(frame) = vm.frames.last_mut() else { return };
    let (regs, args, argc) = vm.stack.jit_pointers(frame);
    let mut jf = JitFrame {
        regs: regs.cast(),
        args: args.cast(),
        argc: argc as u64,
        loop_count: &raw mut frame.loop_iteration_count,
        loop_max,
        acc: 0,
        acc_set: 0,
        entry_pc: u64::from(frame.pc),
    };
    // SAFETY: `f` was compiled for this frame's code block; `jf` points at its live registers
    // and arguments, which nothing else touches while compiled code runs (it calls no code
    // that can reach the VM).
    let exit = unsafe { f(&raw mut jf) };
    frame.pc = exit;
    if jf.acc_set != 0 {
        // SAFETY: compiled code only stores non-pointer words in the accumulator.
        vm.return_value = unsafe { JsValue::jit_from_bits(jf.acc) };
    }

    // Count bailouts (an exit anywhere but a `Return`).
    let code = &frame.code_block;
    let returned = code.bytecode.bytes.get(exit as usize).is_some_and(|&b| Opcode::decode(b) == Opcode::Return);
    let jit = &code.jit;
    let entries = jit.note_entry();
    if !returned {
        let bailouts = jit.note_bailout();
        if bailouts >= BAILOUT_GRACE && bailouts.saturating_mul(4) > entries {
            jit.set_code(None);
            jit.set_state(TierState::Rejected(super::Rejection { reason: "bails out", pc: Some(exit) }));
        }
    }
}
