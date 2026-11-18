//! Bytecode to machine code (task J2), for code blocks `support::scan` accepted.
//!
//! The compiled function works directly on the frame's registers in the VM stack, as raw
//! NaN-boxed 64-bit words. Every bytecode instruction becomes a Cranelift block, so any jump
//! target or loop header can be entered. Each instruction checks everything it needs first
//! (inputs are numbers, a destination doesn't hold a pointer) and only then writes, so when a
//! check fails it can stop and hand that same instruction to the interpreter.
//!
//! There is one way out: the function returns the bytecode offset where the interpreter should
//! go on. That is a `Return` instruction (with the accumulator set) when it finished, or the
//! instruction it couldn't do (a bailout).

use super::backend::JitBackend;
use super::helpers::{self, INT32_TAG, NONE, POINTER_MASK};
use crate::JsValue;
use crate::vm::{CodeBlock, Instruction, InstructionIterator};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{Block, InstBuilder, MemFlagsData, SigRef, StackSlot, StackSlotData, StackSlotKind, Value, types};
use cranelift_frontend::{FunctionBuilder, Switch};
use std::collections::HashMap;

/// What compiled code gets from [`super::runtime::enter`]. Field offsets are used by the
/// generated code; keep [`offsets`] in step.
#[repr(C)]
pub(crate) struct JitFrame {
    pub(crate) regs: *mut u64,
    pub(crate) args: *const u64,
    pub(crate) argc: u64,
    pub(crate) loop_count: *mut u64,
    pub(crate) loop_max: u64,
    pub(crate) acc: u64,
    pub(crate) acc_set: u64,
    pub(crate) entry_pc: u64,
    /// The context, for the steps compiled code hands to the interpreter (J3).
    pub(crate) ctx: *mut crate::Context,
    /// The frame's `fp`, to tell whether the top frame is still this one after a step.
    pub(crate) fp: u64,
}

mod offsets {
    pub(super) const REGS: i32 = 0;
    pub(super) const ARGS: i32 = 8;
    pub(super) const ARGC: i32 = 16;
    pub(super) const LOOP_COUNT: i32 = 24;
    pub(super) const LOOP_MAX: i32 = 32;
    pub(super) const ACC: i32 = 40;
    pub(super) const ACC_SET: i32 = 48;
    pub(super) const ENTRY_PC: i32 = 56;
}

/// What a compiled function returns besides a bytecode offset: it threw (the error waits in
/// `runtime`), or the interpreter's frames changed under it (a call pushed one, an exception
/// unwound); the interpreter goes on from wherever the VM now stands.
pub(crate) const EXIT_ERROR: u32 = u32::MAX;
pub(crate) const EXIT_RESUME: u32 = u32::MAX - 1;

const _: () = {
    assert!(size_of::<JsValue>() == 8);
    assert!(std::mem::offset_of!(JitFrame, args) == offsets::ARGS as usize);
    assert!(std::mem::offset_of!(JitFrame, argc) == offsets::ARGC as usize);
    assert!(std::mem::offset_of!(JitFrame, loop_count) == offsets::LOOP_COUNT as usize);
    assert!(std::mem::offset_of!(JitFrame, loop_max) == offsets::LOOP_MAX as usize);
    assert!(std::mem::offset_of!(JitFrame, acc) == offsets::ACC as usize);
    assert!(std::mem::offset_of!(JitFrame, acc_set) == offsets::ACC_SET as usize);
    assert!(std::mem::offset_of!(JitFrame, entry_pc) == offsets::ENTRY_PC as usize);
};

/// A compiled code block's entry point.
pub(crate) type JitFn = unsafe extern "C" fn(*mut JitFrame) -> u32;

const FALSE: u64 = 0x7FFA_0000_0000_0000;
const UNDEFINED: u64 = 0x7FFB_0000_0000_0001;

/// Compiles `code`; `None` if Cranelift refused (the caller leaves it to the interpreter).
pub(crate) fn compile(backend: &mut JitBackend, code: &CodeBlock) -> Result<JitFn, String> {
    let instructions: Vec<(u32, Instruction)> =
        InstructionIterator::new(&code.bytecode).map(|(pc, _, ins)| (pc as u32, ins)).collect();
    let end = code.bytecode.bytes.len() as u32;
    let ptr = backend.pointer_type();
    let sig = backend.signature(&[ptr], &[types::I32]);
    let bin_sig = backend.signature(&[types::I64, types::I64], &[types::I64]);
    let un_sig = backend.signature(&[types::I64], &[types::I64]);
    let step_sig = backend.signature(&[types::I64, ptr], &[types::I64]);
    let interp_sig = backend.signature(&[ptr, types::I32], &[types::I32]);

    let mut failed = None;
    let entry = backend.compile(sig, |b| {
        let mut g = Gen::new(b, ptr, bin_sig, un_sig, step_sig, interp_sig, &instructions, end);
        for i in 0..instructions.len() {
            g.instruction(i);
        }
        if g.unsupported.is_some() {
            failed = g.unsupported;
        }
        g.b.seal_all_blocks();
    })?;
    if let Some(name) = failed {
        return Err(format!("no code for {name}"));
    }
    // SAFETY: built above with the `JitFn` signature in the host calling convention.
    Ok(unsafe { std::mem::transmute::<*const u8, JitFn>(entry) })
}

struct Gen<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    frame: Value,
    ptr: types::Type,
    exit: Block,
    /// Goes to the block for the pc in its parameter (any instruction can be resumed).
    dispatch: Block,
    /// The instruction being compiled (its index), for slow paths.
    cur: usize,
    interp: SigRef,
    blocks: HashMap<u32, Block>,
    instructions: &'a [(u32, Instruction)],
    end: u32,
    bin: SigRef,
    un: SigRef,
    step: SigRef,
    step_out: StackSlot,
    unsupported: Option<&'static str>,
}

impl<'a, 'b> Gen<'a, 'b> {
    fn new(
        b: &'a mut FunctionBuilder<'b>,
        ptr: types::Type,
        bin: cranelift_codegen::ir::Signature,
        un: cranelift_codegen::ir::Signature,
        step: cranelift_codegen::ir::Signature,
        interp: cranelift_codegen::ir::Signature,
        instructions: &'a [(u32, Instruction)],
        end: u32,
    ) -> Self {
        let start = b.create_block();
        b.append_block_params_for_function_params(start);
        let exit = b.create_block();
        b.append_block_param(exit, types::I32);
        let mut blocks = HashMap::new();
        for (pc, _) in instructions {
            blocks.insert(*pc, b.create_block());
        }
        let bin = b.import_signature(bin);
        let un = b.import_signature(un);
        let step = b.import_signature(step);
        let interp = b.import_signature(interp);
        let step_out = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));

        // The start (the first block placed, so Cranelift's entry): read the frame, then go to
        // the block for the entry pc. Every instruction can be an entry (J3): compiled code
        // resumes after calls and caught exceptions anywhere in the function.
        b.switch_to_block(start);
        let frame = b.block_params(start)[0];
        let flags = MemFlagsData::trusted();
        let entry_pc = b.ins().load(types::I64, flags, frame, offsets::ENTRY_PC);
        let entry_pc32 = b.ins().ireduce(types::I32, entry_pc);
        let dispatch = b.create_block();
        b.append_block_param(dispatch, types::I32);
        b.ins().jump(dispatch, &[entry_pc32.into()]);

        b.switch_to_block(dispatch);
        let want = b.block_params(dispatch)[0];
        let want64 = b.ins().uextend(types::I64, want);
        let default = b.create_block();
        let mut switch = Switch::new();
        for (pc, _) in instructions {
            switch.set_entry(u128::from(*pc), blocks[pc]);
        }
        switch.emit(b, want64, default);
        b.switch_to_block(default);
        b.ins().jump(exit, &[want.into()]);

        // The exit: return the pc the interpreter goes on from.
        b.switch_to_block(exit);
        let pc = b.block_params(exit)[0];
        b.ins().return_(&[pc]);

        Self { b, frame, ptr, exit, dispatch, cur: 0, interp, blocks, instructions, end, bin, un, step, step_out, unsupported: None }
    }

    // ---- small pieces ----

    fn iconst(&mut self, v: u64) -> Value {
        self.b.ins().iconst(types::I64, v as i64)
    }

    /// The register file's address. Read from the frame every time: a step can grow the VM
    /// stack and move it.
    fn regs(&mut self) -> Value {
        self.b.ins().load(self.ptr, MemFlagsData::trusted(), self.frame, offsets::REGS)
    }

    fn reg(&mut self, r: impl Into<u32>) -> Value {
        let offset = (r.into() as i32) * 8;
        let regs = self.regs();
        self.b.ins().load(types::I64, MemFlagsData::trusted(), regs, offset)
    }

    fn put(&mut self, r: impl Into<u32>, v: Value) {
        let offset = (r.into() as i32) * 8;
        let regs = self.regs();
        self.b.ins().store(MemFlagsData::trusted(), v, regs, offset);
    }

    /// Leaves for the interpreter at `pc` when `cond` is true (it redoes that instruction);
    /// carries on otherwise. Only the loop limit still does this; everything else takes the
    /// slow path ([`Self::bail_if`]).
    fn exit_if(&mut self, cond: Value, pc: u32) {
        let cont = self.b.create_block();
        let pcv = self.b.ins().iconst(types::I32, i64::from(pc));
        self.b.ins().brif(cond, self.exit, &[pcv.into()], cont, &[]);
        self.b.switch_to_block(cont);
    }

    /// When `cond` is true, the interpreter does this whole instruction (each instruction checks
    /// everything before it writes anything), and compiled code goes on from where that left
    /// the frame; carries on in line otherwise.
    fn bail_if(&mut self, cond: Value, pc: u32) {
        let cont = self.b.create_block();
        let slow = self.b.create_block();
        self.b.ins().brif(cond, slow, &[], cont, &[]);
        self.b.switch_to_block(slow);
        let i = self.cur;
        self.interpret(i, pc);
        self.b.switch_to_block(cont);
    }

    /// The interpreter runs instruction `i` (at `pc`) for compiled code, then compiled code
    /// goes on: at the next instruction, at wherever the instruction jumped or an exception
    /// was caught, or out to the interpreter when its frames changed or it threw.
    fn interpret(&mut self, i: usize, pc: u32) {
        let pcv = self.b.ins().iconst(types::I32, i64::from(pc));
        let addr = self.iconst(super::runtime::step as *const () as usize as u64);
        let call = self.b.ins().call_indirect(self.interp, addr, &[self.frame, pcv]);
        let r = self.b.inst_results(call)[0];
        let next_pc = self.instructions.get(i + 1).map_or(self.end, |(p, _)| *p);
        let is_next = self.b.ins().icmp_imm_s(IntCC::Equal, r, i64::from(next_pc));
        let other = self.b.create_block();
        match self.next_block(i) {
            Some(next) => {
                self.b.ins().brif(is_next, next, &[], other, &[]);
            }
            None => {
                self.b.ins().brif(is_next, self.exit, &[r.into()], other, &[]);
            }
        }
        self.b.switch_to_block(other);
        let special = self.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, r, i64::from(EXIT_RESUME));
        self.b.ins().brif(special, self.exit, &[r.into()], self.dispatch, &[r.into()]);
    }

    fn is_pointer(&mut self, v: Value) -> Value {
        let masked = self.b.ins().band_imm_s(v, POINTER_MASK as i64);
        self.b.ins().icmp_imm_s(IntCC::Equal, masked, POINTER_MASK as i64)
    }

    fn bail_if_pointer(&mut self, v: Value, pc: u32) {
        let p = self.is_pointer(v);
        self.bail_if(p, pc);
    }

    /// Bail out if register `r` holds a pointer (overwriting it would leak its reference).
    fn check_dst(&mut self, r: impl Into<u32>, pc: u32) {
        let old = self.reg(r);
        self.bail_if_pointer(old, pc);
    }

    fn bail_if_eq(&mut self, v: Value, bad: u64, pc: u32) {
        let c = self.b.ins().icmp_imm_s(IntCC::Equal, v, bad as i64);
        self.bail_if(c, pc);
    }

    fn is_int32(&mut self, v: Value) -> Value {
        let hi = self.b.ins().ushr_imm_s(v, 32);
        self.b.ins().icmp_imm_s(IntCC::Equal, hi, INT32_TAG as i64)
    }

    fn box_int32(&mut self, v: Value) -> Value {
        let wide = self.b.ins().uextend(types::I64, v);
        self.b.ins().bor_imm_s(wide, (INT32_TAG << 32) as i64)
    }

    fn call(&mut self, sig: SigRef, f: usize, args: &[Value]) -> Value {
        let addr = self.iconst(f as u64);
        let call = self.b.ins().call_indirect(sig, addr, args);
        self.b.inst_results(call)[0]
    }

    fn next_block(&self, i: usize) -> Option<Block> {
        self.instructions.get(i + 1).map(|(pc, _)| self.blocks[pc])
    }

    /// Falls through to the next instruction (or out to the interpreter past the end).
    fn fall_through(&mut self, i: usize) {
        match self.next_block(i) {
            Some(next) => {
                self.b.ins().jump(next, &[]);
            }
            None => {
                let pcv = self.b.ins().iconst(types::I32, i64::from(self.end));
                self.b.ins().jump(self.exit, &[pcv.into()]);
            }
        }
    }

    /// Branches to `address` when `cond`, else to the next instruction.
    fn branch(&mut self, i: usize, cond: Value, address: u32) {
        let target = self.blocks.get(&address).copied();
        let next = self.next_block(i);
        let pc_end = self.b.ins().iconst(types::I32, i64::from(self.end));
        let pc_target = self.b.ins().iconst(types::I32, i64::from(address));
        match (target, next) {
            (Some(t), Some(n)) => self.b.ins().brif(cond, t, &[], n, &[]),
            (Some(t), None) => self.b.ins().brif(cond, t, &[], self.exit, &[pc_end.into()]),
            (None, Some(n)) => self.b.ins().brif(cond, self.exit, &[pc_target.into()], n, &[]),
            (None, None) => self.b.ins().brif(cond, self.exit, &[pc_target.into()], self.exit, &[pc_end.into()]),
        };
    }

    /// A 0/1/2 helper result: bails on 2, else gives the 0/1 as a condition.
    fn tri(&mut self, r: Value, pc: u32) -> Value {
        self.bail_if_eq(r, 2, pc);
        self.b.ins().icmp_imm_s(IntCC::NotEqual, r, 0)
    }

    fn bool_word(&mut self, bit: Value) -> Value {
        self.b.ins().bor_imm_s(bit, FALSE as i64)
    }

    // ---- instructions ----

    fn instruction(&mut self, i: usize) {
        let (pc, ref ins) = self.instructions[i];
        let block = self.blocks[&pc];
        self.b.switch_to_block(block);
        self.cur = i;
        let constant = |v: JsValue| v.jit_bits();
        match *ins {
            Instruction::StoreZero { dst } => self.store_const(i, pc, dst, constant(JsValue::from(0))),
            Instruction::StoreOne { dst } => self.store_const(i, pc, dst, constant(JsValue::from(1))),
            Instruction::StoreInt8 { dst, value } => self.store_const(i, pc, dst, constant(JsValue::from(i32::from(value)))),
            Instruction::StoreInt16 { dst, value } => self.store_const(i, pc, dst, constant(JsValue::from(i32::from(value)))),
            Instruction::StoreInt32 { dst, value } => self.store_const(i, pc, dst, constant(JsValue::from(value))),
            Instruction::StoreFloat { dst, value } => self.store_const(i, pc, dst, constant(JsValue::from(value))),
            Instruction::StoreDouble { dst, value } => self.store_const(i, pc, dst, constant(JsValue::from(value))),
            Instruction::StoreNan { dst } => self.store_const(i, pc, dst, constant(JsValue::nan())),
            Instruction::StorePositiveInfinity { dst } => self.store_const(i, pc, dst, constant(JsValue::positive_infinity())),
            Instruction::StoreNegativeInfinity { dst } => self.store_const(i, pc, dst, constant(JsValue::negative_infinity())),
            Instruction::StoreTrue { dst } => self.store_const(i, pc, dst, constant(JsValue::from(true))),
            Instruction::StoreFalse { dst } => self.store_const(i, pc, dst, constant(JsValue::from(false))),
            Instruction::StoreUndefined { dst } => self.store_const(i, pc, dst, constant(JsValue::undefined())),

            Instruction::Move { dst, src } => {
                let v = self.reg(src);
                self.bail_if_pointer(v, pc);
                self.check_dst(dst, pc);
                self.put(dst, v);
                self.fall_through(i);
            }
            Instruction::GetArgument { index, dst } => {
                let index: u32 = index.into();
                self.check_dst(dst, pc);
                let flags = MemFlagsData::trusted();
                let argc = self.b.ins().load(types::I64, flags, self.frame, offsets::ARGC);
                let present = self.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, argc, i64::from(index));
                let load = self.b.create_block();
                let join = self.b.create_block();
                self.b.append_block_param(join, types::I64);
                let undefined = self.iconst(UNDEFINED);
                self.b.ins().brif(present, load, &[], join, &[undefined.into()]);
                self.b.switch_to_block(load);
                let args = self.b.ins().load(types::I64, flags, self.frame, offsets::ARGS);
                let v = self.b.ins().load(types::I64, flags, args, (index as i32) * 8);
                self.b.ins().jump(join, &[v.into()]);
                self.b.switch_to_block(join);
                let v = self.b.block_params(join)[0];
                self.bail_if_pointer(v, pc);
                self.put(dst, v);
                self.fall_through(i);
            }

            Instruction::Add { dst, lhs, rhs } => self.add_sub(i, pc, dst, lhs, rhs, true),
            Instruction::Sub { dst, lhs, rhs } => self.add_sub(i, pc, dst, lhs, rhs, false),
            Instruction::Mul { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::mul),
            Instruction::Div { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::div),
            Instruction::Mod { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::rem),
            Instruction::BitAnd { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::bitand),
            Instruction::BitOr { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::bitor),
            Instruction::BitXor { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::bitxor),
            Instruction::ShiftLeft { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::shl),
            Instruction::ShiftRight { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::shr),
            Instruction::UnsignedShiftRight { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::ushr),
            Instruction::Eq { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::equals),
            Instruction::NotEq { dst, lhs, rhs } => self.binary(i, pc, dst, lhs, rhs, helpers::not_equals),

            Instruction::LessThan { dst, lhs, rhs } => self.compare(i, pc, dst, lhs, rhs, helpers::lt, false),
            Instruction::LessThanOrEq { dst, lhs, rhs } => self.compare(i, pc, dst, lhs, rhs, helpers::le, false),
            Instruction::GreaterThan { dst, lhs, rhs } => self.compare(i, pc, dst, lhs, rhs, helpers::gt, false),
            Instruction::GreaterThanOrEq { dst, lhs, rhs } => self.compare(i, pc, dst, lhs, rhs, helpers::ge, false),
            Instruction::StrictEq { dst, lhs, rhs } => self.compare(i, pc, dst, lhs, rhs, helpers::strict_equals, false),
            Instruction::StrictNotEq { dst, lhs, rhs } => self.compare(i, pc, dst, lhs, rhs, helpers::strict_equals, true),

            Instruction::Pos { value } => self.unary(i, pc, value, helpers::pos),
            Instruction::Neg { value } => self.unary(i, pc, value, helpers::neg),
            Instruction::BitNot { value } => self.unary(i, pc, value, helpers::bitnot),
            Instruction::LogicalNot { value } => {
                let v = self.reg(value);
                let t = self.call(self.un, helpers::truthy as *const () as usize, &[v]);
                self.bail_if_eq(t, 2, pc);
                let not = self.b.ins().bxor_imm_s(t, 1);
                let word = self.bool_word(not);
                self.put(value, word);
                self.fall_through(i);
            }
            Instruction::Inc { dst, src } => self.step(i, pc, dst, src, true),
            Instruction::Dec { dst, src } => self.step(i, pc, dst, src, false),

            Instruction::Jump { address } => match self.blocks.get(&u32::from(address)).copied() {
                Some(t) => {
                    self.b.ins().jump(t, &[]);
                }
                None => {
                    let pcv = self.b.ins().iconst(types::I32, i64::from(u32::from(address)));
                    self.b.ins().jump(self.exit, &[pcv.into()]);
                }
            },
            Instruction::JumpIfTrue { address, value } | Instruction::LogicalOr { address, value } => {
                let v = self.reg(value);
                let t = self.call(self.un, helpers::truthy as *const () as usize, &[v]);
                let cond = self.tri(t, pc);
                self.branch(i, cond, address.into());
            }
            Instruction::JumpIfFalse { address, value } | Instruction::LogicalAnd { address, value } => {
                let v = self.reg(value);
                let t = self.call(self.un, helpers::truthy as *const () as usize, &[v]);
                let cond = self.b.ins().icmp_imm_s(IntCC::Equal, t, 0);
                self.bail_if_eq(t, 2, pc);
                self.branch(i, cond, address.into());
            }
            Instruction::JumpIfNotLessThan { address, lhs, rhs } => self.jump_compare(i, pc, address.into(), lhs, rhs, IntCC::SignedLessThan, helpers::lt),
            Instruction::JumpIfNotLessThanOrEqual { address, lhs, rhs } => {
                self.jump_compare(i, pc, address.into(), lhs, rhs, IntCC::SignedLessThanOrEqual, helpers::le);
            }
            Instruction::JumpIfNotGreaterThan { address, lhs, rhs } => {
                self.jump_compare(i, pc, address.into(), lhs, rhs, IntCC::SignedGreaterThan, helpers::gt);
            }
            Instruction::JumpIfNotGreaterThanOrEqual { address, lhs, rhs } => {
                self.jump_compare(i, pc, address.into(), lhs, rhs, IntCC::SignedGreaterThanOrEqual, helpers::ge);
            }
            Instruction::JumpIfNotEqual { address, lhs, rhs } => {
                let (l, r) = (self.reg(lhs), self.reg(rhs));
                let t = self.call(self.bin, helpers::not_same as *const () as usize, &[l, r]);
                let cond = self.tri(t, pc);
                self.branch(i, cond, address.into());
            }

            Instruction::IncrementLoopIteration => {
                // As the interpreter: past the limit, it raises the error; else count.
                let flags = MemFlagsData::trusted();
                let counter = self.b.ins().load(types::I64, flags, self.frame, offsets::LOOP_COUNT);
                let count = self.b.ins().load(types::I64, flags, counter, 0);
                let max = self.b.ins().load(types::I64, flags, self.frame, offsets::LOOP_MAX);
                let over = self.b.ins().icmp(IntCC::UnsignedGreaterThan, count, max);
                // The interpreter raises the error (stepping this opcode would re-enter here).
                self.exit_if(over, pc);
                let next = self.b.ins().iadd_imm_s(count, 1);
                self.b.ins().store(flags, next, counter, 0);
                self.fall_through(i);
            }
            // The interpreter's own `SetAccumulator` (it owns the return value).
            Instruction::SetAccumulator { .. } => self.interpret(i, pc),
            // A no-op for calls; `new` frames never run compiled code.
            Instruction::CheckReturn => self.fall_through(i),
            // The interpreter returns (it knows about early exits and popping the frame).
            Instruction::Return => {
                let pcv = self.b.ins().iconst(types::I32, i64::from(pc));
                self.b.ins().jump(self.exit, &[pcv.into()]);
            }
            // Everything else: the interpreter runs it (`support::scan` only lets through opcodes
            // that finish in place).
            _ => self.interpret(i, pc),
        }
    }

    fn store_const(&mut self, i: usize, pc: u32, dst: impl Into<u32> + Copy, bits: u64) {
        self.check_dst(dst, pc);
        let v = self.iconst(bits);
        self.put(dst, v);
        self.fall_through(i);
    }

    /// A helper result: bail on [`NONE`], else store it.
    fn finish(&mut self, i: usize, pc: u32, dst: impl Into<u32> + Copy, r: Value) {
        self.bail_if_eq(r, NONE, pc);
        self.check_dst(dst, pc);
        self.put(dst, r);
        self.fall_through(i);
    }

    fn binary(&mut self, i: usize, pc: u32, dst: impl Into<u32> + Copy, lhs: impl Into<u32>, rhs: impl Into<u32>, f: extern "C" fn(u64, u64) -> u64) {
        let (l, r) = (self.reg(lhs), self.reg(rhs));
        let out = self.call(self.bin, f as *const () as usize, &[l, r]);
        self.finish(i, pc, dst, out);
    }

    fn unary(&mut self, i: usize, pc: u32, value: impl Into<u32> + Copy, f: extern "C" fn(u64) -> u64) {
        let v = self.reg(value);
        let out = self.call(self.un, f as *const () as usize, &[v]);
        self.finish(i, pc, value, out);
    }

    /// `Add`/`Sub`: int32 in line (with overflow going to the helper), anything else through
    /// the helper.
    fn add_sub(&mut self, i: usize, pc: u32, dst: impl Into<u32> + Copy, lhs: impl Into<u32>, rhs: impl Into<u32>, add: bool) {
        let (l, r) = (self.reg(lhs), self.reg(rhs));
        let fast = self.b.create_block();
        let ok = self.b.create_block();
        let slow = self.b.create_block();
        let join = self.b.create_block();
        self.b.append_block_param(join, types::I64);
        let li = self.is_int32(l);
        let ri = self.is_int32(r);
        let both = self.b.ins().band(li, ri);
        self.b.ins().brif(both, fast, &[], slow, &[]);

        self.b.switch_to_block(fast);
        let x = self.b.ins().ireduce(types::I32, l);
        let y = self.b.ins().ireduce(types::I32, r);
        let (sum, overflow) = if add { self.b.ins().sadd_overflow(x, y) } else { self.b.ins().ssub_overflow(x, y) };
        self.b.ins().brif(overflow, slow, &[], ok, &[]);
        self.b.switch_to_block(ok);
        let boxed = self.box_int32(sum);
        self.b.ins().jump(join, &[boxed.into()]);

        self.b.switch_to_block(slow);
        let f = if add { helpers::add } else { helpers::sub };
        let out = self.call(self.bin, f as *const () as usize, &[l, r]);
        self.b.ins().jump(join, &[out.into()]);

        self.b.switch_to_block(join);
        let out = self.b.block_params(join)[0];
        self.finish(i, pc, dst, out);
    }

    fn compare(&mut self, i: usize, pc: u32, dst: impl Into<u32> + Copy, lhs: impl Into<u32>, rhs: impl Into<u32>, f: extern "C" fn(u64, u64) -> u64, negate: bool) {
        let (l, r) = (self.reg(lhs), self.reg(rhs));
        let t = self.call(self.bin, f as *const () as usize, &[l, r]);
        self.bail_if_eq(t, 2, pc);
        let bit = if negate { self.b.ins().bxor_imm_s(t, 1) } else { t };
        let word = self.bool_word(bit);
        self.check_dst(dst, pc);
        self.put(dst, word);
        self.fall_through(i);
    }

    /// `JumpIfNot<cmp>`: jumps when the comparison is false. Int32 in line.
    fn jump_compare(&mut self, i: usize, pc: u32, address: u32, lhs: impl Into<u32>, rhs: impl Into<u32>, cc: IntCC, f: extern "C" fn(u64, u64) -> u64) {
        let (l, r) = (self.reg(lhs), self.reg(rhs));
        let fast = self.b.create_block();
        let slow = self.b.create_block();
        let join = self.b.create_block();
        self.b.append_block_param(join, types::I64);
        let li = self.is_int32(l);
        let ri = self.is_int32(r);
        let both = self.b.ins().band(li, ri);
        self.b.ins().brif(both, fast, &[], slow, &[]);

        self.b.switch_to_block(fast);
        let x = self.b.ins().ireduce(types::I32, l);
        let y = self.b.ins().ireduce(types::I32, r);
        let c = self.b.ins().icmp(cc, x, y);
        let c = self.b.ins().uextend(types::I64, c);
        self.b.ins().jump(join, &[c.into()]);

        self.b.switch_to_block(slow);
        let out = self.call(self.bin, f as *const () as usize, &[l, r]);
        self.b.ins().jump(join, &[out.into()]);

        self.b.switch_to_block(join);
        let t = self.b.block_params(join)[0];
        self.bail_if_eq(t, 2, pc);
        let not = self.b.ins().icmp_imm_s(IntCC::Equal, t, 0);
        self.branch(i, not, address);
    }

    /// `Inc`/`Dec`: writes `src` (the old value as a number) then `dst`, as the interpreter.
    fn step(&mut self, i: usize, pc: u32, dst: impl Into<u32> + Copy, src: impl Into<u32> + Copy, up: bool) {
        let v = self.reg(src);
        self.check_dst(dst, pc);
        let fast = self.b.create_block();
        let slow = self.b.create_block();
        let done = self.b.create_block();
        let is_int = self.is_int32(v);
        let n = self.b.ins().ireduce(types::I32, v);
        let limit = if up { i64::from(i32::MAX) } else { i64::from(i32::MIN) };
        let at_limit = self.b.ins().icmp_imm_s(IntCC::Equal, n, limit);
        let not_limit = self.b.ins().bxor_imm_s(at_limit, 1);
        let take_fast = self.b.ins().band(is_int, not_limit);
        self.b.ins().brif(take_fast, fast, &[], slow, &[]);

        // int32 not at the limit: `src` keeps its value, `dst` gets n ± 1.
        self.b.switch_to_block(fast);
        let m = self.b.ins().iadd_imm_s(n, if up { 1 } else { -1 });
        let boxed = self.box_int32(m);
        self.put(dst, boxed);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(slow);
        let out = self.b.ins().stack_addr(types::I64, self.step_out, 0);
        let f = if up { helpers::inc } else { helpers::dec };
        let r = self.call(self.step, f as *const () as usize, &[v, out]);
        self.bail_if_eq(r, 2, pc);
        let numeric = self.b.ins().stack_load(types::I64, types::I64, self.step_out, 0);
        let value = self.b.ins().stack_load(types::I64, types::I64, self.step_out, 8);
        self.put(src, numeric);
        self.put(dst, value);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(done);
        self.fall_through(i);
    }
}
