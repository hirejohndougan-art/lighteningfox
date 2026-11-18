//! Running compiled code: compiling on tier-up, entering it (at the start of a call, at a loop
//! header, and back in a compiled caller after a call returns), the interpreter steps compiled
//! code hands back (J3), and giving up on functions that keep bailing out.
//!
//! Compiled code never calls into another compiled function directly and never waits for a
//! callee: a call it can't finish (a JavaScript callee) ends its run with [`EXIT_RESUME`], and
//! [`enter`]'s loop goes on in the callee; when the callee returns, the interpreter's
//! `handle_return` brings the caller back here. So a deep JavaScript recursion never nests
//! machine frames.

use super::backend::JitBackend;
use super::compiler::{self, EXIT_ERROR, EXIT_RESUME, JitFrame};
use super::{JitMode, TierState, mode};
use crate::vm::completion_record::{CompletionRecord, JitStep};
use crate::vm::opcode::OPCODE_JIT_STEPS;
use crate::{Context, JsError, JsNativeError, JsResult, vm::CodeBlock, vm::Opcode};
use std::cell::{Cell, RefCell};
use std::ops::ControlFlow;

thread_local! {
    /// The Cranelift module for this thread's code (a `Context` stays on its thread).
    static BACKEND: RefCell<Option<JitBackend>> = const { RefCell::new(None) };
    /// The error a compiled function's [`EXIT_ERROR`] stands for.
    static THROWN: RefCell<Option<JsError>> = const { RefCell::new(None) };
    /// While a step runs a `Call` for compiled code: the frame count then. The callee it
    /// pushes is entered by [`enter`]'s loop once compiled code has returned, not inside it.
    static STEPPING: Cell<usize> = const { Cell::new(0) };
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

/// Whether the top frame has machine code to run now.
#[inline]
fn runnable(context: &Context) -> Option<compiler::JitFn> {
    let frame = context.vm.frames.last()?;
    let f = frame.code_block.jit.code()?;
    if frame.construct() || cfg!(feature = "fuzz") {
        return None;
    }
    Some(f)
}

/// Runs the top frame's compiled code from the frame's `pc`, if it has some, and then the
/// compiled code of whatever frame that leaves on top. Afterwards the interpreter goes on from
/// the VM as it stands; an `Err` is an exception the interpreter has to raise there.
#[inline]
pub(crate) fn enter(context: &mut Context) -> JsResult<()> {
    if runnable(context).is_none() {
        return Ok(());
    }
    run(context)
}

/// From `Call`/`CallSpread`, once a JavaScript callee's frame is pushed. Not while a step runs
/// that `Call` for compiled code: [`enter`]'s loop takes the callee then.
#[inline]
pub(crate) fn enter_at_call(context: &mut Context) -> JsResult<()> {
    if STEPPING.with(Cell::get) == context.vm.frames.len().saturating_sub(1) {
        return Ok(());
    }
    enter(context)
}

/// From the interpreter's `handle_return`, back in the caller after a call.
#[inline]
pub(crate) fn enter_after_return(context: &mut Context) -> JsResult<()> {
    enter(context)
}

#[inline(never)]
fn run(context: &mut Context) -> JsResult<()> {
    loop {
        if mode() == JitMode::Off {
            return Ok(());
        }
        let Some(f) = runnable(context) else { return Ok(()) };
        let loop_max = context.vm.runtime_limits.loop_iteration_limit();
        let ctx: *mut Context = context;
        let vm = &mut context.vm;
        let depth = vm.frames.len();
        let Some(frame) = vm.frames.last_mut() else { return Ok(()) };
        let (regs, args, argc) = vm.stack.jit_pointers(frame);
        let fp = frame.fp;
        let code = frame.code_block.clone();
        let mut jf = JitFrame {
            regs: regs.cast(),
            args: args.cast(),
            argc: argc as u64,
            loop_count: &raw mut frame.loop_iteration_count,
            loop_max,
            acc: 0,
            acc_set: 0,
            entry_pc: u64::from(frame.pc),
            ctx,
            fp: u64::from(fp),
        };
        // SAFETY: `f` was compiled for this frame's code block; `jf` points at its live
        // registers and arguments. Compiled code touches the VM only through `step`, which
        // refreshes those pointers after anything that can move them.
        let exit = unsafe { f(&raw mut jf) };
        match exit {
            EXIT_ERROR => {
                let err = THROWN.with(|t| t.borrow_mut().take());
                return Err(err.unwrap_or_else(|| JsNativeError::error().with_message("compiled code lost an exception").into()));
            }
            EXIT_RESUME => {
                // The frames changed: go on in the new top frame's code, if it has some.
                let same = context.vm.frames.len() == depth && context.vm.frames.last().is_some_and(|f| f.fp == fp);
                if same {
                    return Ok(());
                }
            }
            pc => {
                if let Some(frame) = context.vm.frames.last_mut() {
                    frame.pc = pc;
                }
                note_exit(&code, pc);
                return Ok(());
            }
        }
    }
}

/// Counts bailouts (an exit anywhere but a `Return`).
fn note_exit(code: &CodeBlock, exit: u32) {
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

/// The interpreter runs the instruction at `pc` for compiled code (J3). Returns where compiled
/// code goes on: the frame's new pc (the next instruction, a jump target, a `catch`), or
/// [`EXIT_RESUME`] when the frames changed (a JavaScript callee was pushed, an exception
/// unwound out of this frame), or [`EXIT_ERROR`] when the exception has to go to the
/// interpreter (it reached a frame that returns to Rust).
pub(crate) extern "C" fn step(jf: *mut JitFrame, pc: u32) -> u32 {
    // SAFETY: compiled code passes its own frame, which `run` keeps alive for the call.
    let jf = unsafe { &mut *jf };
    // SAFETY: `run` set this from the `&mut Context` it was called with; nothing else uses
    // that context while compiled code runs.
    let context = unsafe { &mut *jf.ctx };
    let depth = context.vm.frames.len();
    let opcode = Opcode::decode(context.vm.frame().code_block.bytecode.bytes[pc as usize]);
    let outer = STEPPING.with(|s| s.replace(depth));
    let result = OPCODE_JIT_STEPS[opcode as usize](context, pc as usize);
    STEPPING.with(|s| s.set(outer));
    match result {
        JitStep::Done | JitStep::Flow(ControlFlow::Continue(())) => {}
        JitStep::Flow(ControlFlow::Break(CompletionRecord::Throw(err))) => {
            THROWN.with(|t| *t.borrow_mut() = Some(err));
            return EXIT_ERROR;
        }
        // `ReThrow` returning from a generator: never in code the JIT takes.
        JitStep::Flow(ControlFlow::Break(_)) => return EXIT_RESUME,
        JitStep::Threw(err) => {
            // As the interpreter: this frame's `catch`, or unwinding.
            match context.handle_error(err) {
                ControlFlow::Continue(()) => {}
                ControlFlow::Break(CompletionRecord::Throw(err)) => {
                    THROWN.with(|t| *t.borrow_mut() = Some(err));
                    return EXIT_ERROR;
                }
                ControlFlow::Break(_) => return EXIT_RESUME,
            }
        }
    }
    let vm = &mut context.vm;
    if vm.frames.len() != depth {
        return EXIT_RESUME;
    }
    let Some(frame) = vm.frames.last_mut() else { return EXIT_RESUME };
    if u64::from(frame.fp) != jf.fp {
        return EXIT_RESUME;
    }
    // Still this frame: the step may have grown the stack or the frame list.
    let (regs, args, argc) = vm.stack.jit_pointers(frame);
    jf.regs = regs.cast();
    jf.args = args.cast();
    jf.argc = argc as u64;
    jf.loop_count = &raw mut frame.loop_iteration_count;
    frame.pc
}
