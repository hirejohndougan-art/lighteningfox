//! Which code blocks the first version of the compiler takes.
//!
//! The first version (J2) compiles functions that only use registers, number constants,
//! arithmetic, comparisons, branches and loops. Values are guarded at run time: if a register
//! holds anything but a number, boolean or `undefined` where compiled code needs a number, it
//! bails out to the interpreter at that instruction. The list here is the static half of that
//! decision; keep it in step with `docs/JIT.md`.

use crate::vm::{CodeBlock, InstructionIterator, Opcode};

/// Why a hot code block stays in the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejection {
    /// The first unsupported opcode's name, or the function kind (e.g. `"generator"`).
    pub reason: &'static str,
    /// Bytecode offset of that opcode, if the reason is an opcode.
    pub pc: Option<u32>,
}

impl Rejection {
    const fn kind(reason: &'static str) -> Self {
        Self { reason, pc: None }
    }
}

/// Returns `true` for the opcodes the first compiler handles.
pub(crate) const fn is_supported(opcode: Opcode) -> bool {
    matches!(
        opcode,
        // Constants into registers.
        Opcode::StoreZero
            | Opcode::StoreOne
            | Opcode::StoreInt8
            | Opcode::StoreInt16
            | Opcode::StoreInt32
            | Opcode::StoreFloat
            | Opcode::StoreDouble
            | Opcode::StoreNan
            | Opcode::StorePositiveInfinity
            | Opcode::StoreNegativeInfinity
            | Opcode::StoreTrue
            | Opcode::StoreFalse
            | Opcode::StoreUndefined
            // Locals and arguments.
            | Opcode::Move
            | Opcode::GetArgument
            // Number arithmetic (bails out unless both sides are numbers).
            | Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::Mod
            | Opcode::Neg
            | Opcode::Pos
            | Opcode::Inc
            | Opcode::Dec
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::BitNot
            | Opcode::ShiftLeft
            | Opcode::ShiftRight
            | Opcode::UnsignedShiftRight
            // Comparisons.
            | Opcode::LessThan
            | Opcode::LessThanOrEq
            | Opcode::GreaterThan
            | Opcode::GreaterThanOrEq
            | Opcode::StrictEq
            | Opcode::StrictNotEq
            | Opcode::Eq
            | Opcode::NotEq
            | Opcode::LogicalNot
            // Branches and loops.
            | Opcode::Jump
            | Opcode::JumpIfTrue
            | Opcode::JumpIfFalse
            | Opcode::JumpIfNotLessThan
            | Opcode::JumpIfNotLessThanOrEqual
            | Opcode::JumpIfNotGreaterThan
            | Opcode::JumpIfNotGreaterThanOrEqual
            | Opcode::JumpIfNotEqual
            | Opcode::LogicalAnd
            | Opcode::LogicalOr
            | Opcode::IncrementLoopIteration
            // Leaving the function.
            | Opcode::SetAccumulator
            // Ends every function; does nothing unless the frame is a `new` call, where
            // compiled code bails out to the interpreter.
            | Opcode::CheckReturn
            | Opcode::Return
    )
}

/// Checks a code block against the first compiler's limits.
pub(crate) fn scan(code: &CodeBlock) -> Result<(), Rejection> {
    if code.is_generator() || code.is_async() || code.is_async_generator() {
        return Err(Rejection::kind("generator or async function"));
    }
    if code.is_class_constructor() || code.is_derived_constructor() {
        return Err(Rejection::kind("class constructor"));
    }
    // J3: anything else the interpreter can run for compiled code (one instruction at a time,
    // its exceptions handled as the interpreter does, `try` included).
    for (pc, opcode, _) in InstructionIterator::new(&code.bytecode) {
        if !is_supported(opcode) && !crate::vm::opcode::jit_steppable(opcode) {
            return Err(Rejection {
                reason: opcode.as_str(),
                pc: Some(pc as u32),
            });
        }
    }
    Ok(())
}
