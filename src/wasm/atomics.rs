//! Atomic instructions in modules without shared memory (P1d).
//!
//! wasmi has no threads proposal, so it refuses any module that uses an atomic instruction.
//! Emscripten builds often use them even when they don't run threads (ffmpeg.wasm's
//! single-threaded core does), and there they act on memory nobody else can see: an atomic
//! load is a load, an atomic add is a load, an add and a store. So a module whose memories are
//! all unshared gets its atomics rewritten into those plain instructions before wasmi sees
//! it, which gives the same results. (What can differ: atomics trap on unaligned addresses,
//! plain accesses don't.) `memory.atomic.wait` traps on unshared memory, as the spec says;
//! `notify` wakes nobody and returns 0; `atomic.fence` does nothing.
//!
//! A module with shared memory still fails to compile: that needs real threads.

use wasm_encoder::reencode::{Reencode, RoundtripReencoder};
use wasm_encoder::{BlockType, Instruction as I, MemArg, ValType};
use wasmparser::{Operator as O, Parser, Payload, TypeRef};

/// What an atomic instruction does, with its value type (true = i64) and access size in bits.
#[derive(Clone, Copy)]
enum Atomic {
    Fence,
    Notify,
    Wait(u8),
    Load(bool, u8),
    Store(bool, u8),
    Rmw(bool, u8, Bin),
    Cmpxchg(bool, u8),
}

#[derive(Clone, Copy)]
enum Bin {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Xchg,
}

fn classify(op: &O) -> Option<(Atomic, Option<wasmparser::MemArg>)> {
    use Atomic::*;
    use Bin::*;
    Some(match *op {
        O::AtomicFence => (Fence, None),
        O::MemoryAtomicNotify { memarg } => (Notify, Some(memarg)),
        O::MemoryAtomicWait32 { memarg } => (Wait(3), Some(memarg)),
        O::MemoryAtomicWait64 { memarg } => (Wait(3), Some(memarg)),
        O::I32AtomicLoad { memarg } => (Load(false, 32), Some(memarg)),
        O::I32AtomicLoad8U { memarg } => (Load(false, 8), Some(memarg)),
        O::I32AtomicLoad16U { memarg } => (Load(false, 16), Some(memarg)),
        O::I64AtomicLoad { memarg } => (Load(true, 64), Some(memarg)),
        O::I64AtomicLoad8U { memarg } => (Load(true, 8), Some(memarg)),
        O::I64AtomicLoad16U { memarg } => (Load(true, 16), Some(memarg)),
        O::I64AtomicLoad32U { memarg } => (Load(true, 32), Some(memarg)),
        O::I32AtomicStore { memarg } => (Store(false, 32), Some(memarg)),
        O::I32AtomicStore8 { memarg } => (Store(false, 8), Some(memarg)),
        O::I32AtomicStore16 { memarg } => (Store(false, 16), Some(memarg)),
        O::I64AtomicStore { memarg } => (Store(true, 64), Some(memarg)),
        O::I64AtomicStore8 { memarg } => (Store(true, 8), Some(memarg)),
        O::I64AtomicStore16 { memarg } => (Store(true, 16), Some(memarg)),
        O::I64AtomicStore32 { memarg } => (Store(true, 32), Some(memarg)),
        O::I32AtomicRmwAdd { memarg } => (Rmw(false, 32, Add), Some(memarg)),
        O::I32AtomicRmw8AddU { memarg } => (Rmw(false, 8, Add), Some(memarg)),
        O::I32AtomicRmw16AddU { memarg } => (Rmw(false, 16, Add), Some(memarg)),
        O::I64AtomicRmwAdd { memarg } => (Rmw(true, 64, Add), Some(memarg)),
        O::I64AtomicRmw8AddU { memarg } => (Rmw(true, 8, Add), Some(memarg)),
        O::I64AtomicRmw16AddU { memarg } => (Rmw(true, 16, Add), Some(memarg)),
        O::I64AtomicRmw32AddU { memarg } => (Rmw(true, 32, Add), Some(memarg)),
        O::I32AtomicRmwSub { memarg } => (Rmw(false, 32, Sub), Some(memarg)),
        O::I32AtomicRmw8SubU { memarg } => (Rmw(false, 8, Sub), Some(memarg)),
        O::I32AtomicRmw16SubU { memarg } => (Rmw(false, 16, Sub), Some(memarg)),
        O::I64AtomicRmwSub { memarg } => (Rmw(true, 64, Sub), Some(memarg)),
        O::I64AtomicRmw8SubU { memarg } => (Rmw(true, 8, Sub), Some(memarg)),
        O::I64AtomicRmw16SubU { memarg } => (Rmw(true, 16, Sub), Some(memarg)),
        O::I64AtomicRmw32SubU { memarg } => (Rmw(true, 32, Sub), Some(memarg)),
        O::I32AtomicRmwAnd { memarg } => (Rmw(false, 32, And), Some(memarg)),
        O::I32AtomicRmw8AndU { memarg } => (Rmw(false, 8, And), Some(memarg)),
        O::I32AtomicRmw16AndU { memarg } => (Rmw(false, 16, And), Some(memarg)),
        O::I64AtomicRmwAnd { memarg } => (Rmw(true, 64, And), Some(memarg)),
        O::I64AtomicRmw8AndU { memarg } => (Rmw(true, 8, And), Some(memarg)),
        O::I64AtomicRmw16AndU { memarg } => (Rmw(true, 16, And), Some(memarg)),
        O::I64AtomicRmw32AndU { memarg } => (Rmw(true, 32, And), Some(memarg)),
        O::I32AtomicRmwOr { memarg } => (Rmw(false, 32, Or), Some(memarg)),
        O::I32AtomicRmw8OrU { memarg } => (Rmw(false, 8, Or), Some(memarg)),
        O::I32AtomicRmw16OrU { memarg } => (Rmw(false, 16, Or), Some(memarg)),
        O::I64AtomicRmwOr { memarg } => (Rmw(true, 64, Or), Some(memarg)),
        O::I64AtomicRmw8OrU { memarg } => (Rmw(true, 8, Or), Some(memarg)),
        O::I64AtomicRmw16OrU { memarg } => (Rmw(true, 16, Or), Some(memarg)),
        O::I64AtomicRmw32OrU { memarg } => (Rmw(true, 32, Or), Some(memarg)),
        O::I32AtomicRmwXor { memarg } => (Rmw(false, 32, Xor), Some(memarg)),
        O::I32AtomicRmw8XorU { memarg } => (Rmw(false, 8, Xor), Some(memarg)),
        O::I32AtomicRmw16XorU { memarg } => (Rmw(false, 16, Xor), Some(memarg)),
        O::I64AtomicRmwXor { memarg } => (Rmw(true, 64, Xor), Some(memarg)),
        O::I64AtomicRmw8XorU { memarg } => (Rmw(true, 8, Xor), Some(memarg)),
        O::I64AtomicRmw16XorU { memarg } => (Rmw(true, 16, Xor), Some(memarg)),
        O::I64AtomicRmw32XorU { memarg } => (Rmw(true, 32, Xor), Some(memarg)),
        O::I32AtomicRmwXchg { memarg } => (Rmw(false, 32, Xchg), Some(memarg)),
        O::I32AtomicRmw8XchgU { memarg } => (Rmw(false, 8, Xchg), Some(memarg)),
        O::I32AtomicRmw16XchgU { memarg } => (Rmw(false, 16, Xchg), Some(memarg)),
        O::I64AtomicRmwXchg { memarg } => (Rmw(true, 64, Xchg), Some(memarg)),
        O::I64AtomicRmw8XchgU { memarg } => (Rmw(true, 8, Xchg), Some(memarg)),
        O::I64AtomicRmw16XchgU { memarg } => (Rmw(true, 16, Xchg), Some(memarg)),
        O::I64AtomicRmw32XchgU { memarg } => (Rmw(true, 32, Xchg), Some(memarg)),
        O::I32AtomicRmwCmpxchg { memarg } => (Cmpxchg(false, 32), Some(memarg)),
        O::I32AtomicRmw8CmpxchgU { memarg } => (Cmpxchg(false, 8), Some(memarg)),
        O::I32AtomicRmw16CmpxchgU { memarg } => (Cmpxchg(false, 16), Some(memarg)),
        O::I64AtomicRmwCmpxchg { memarg } => (Cmpxchg(true, 64), Some(memarg)),
        O::I64AtomicRmw8CmpxchgU { memarg } => (Cmpxchg(true, 8), Some(memarg)),
        O::I64AtomicRmw16CmpxchgU { memarg } => (Cmpxchg(true, 16), Some(memarg)),
        O::I64AtomicRmw32CmpxchgU { memarg } => (Cmpxchg(true, 32), Some(memarg)),
        _ => return None,
    })
}

fn load(wide: bool, bits: u8, m: MemArg) -> I<'static> {
    match (wide, bits) {
        (false, 8) => I::I32Load8U(m),
        (false, 16) => I::I32Load16U(m),
        (false, _) => I::I32Load(m),
        (true, 8) => I::I64Load8U(m),
        (true, 16) => I::I64Load16U(m),
        (true, 32) => I::I64Load32U(m),
        (true, _) => I::I64Load(m),
    }
}

fn store(wide: bool, bits: u8, m: MemArg) -> I<'static> {
    match (wide, bits) {
        (false, 8) => I::I32Store8(m),
        (false, 16) => I::I32Store16(m),
        (false, _) => I::I32Store(m),
        (true, 8) => I::I64Store8(m),
        (true, 16) => I::I64Store16(m),
        (true, 32) => I::I64Store32(m),
        (true, _) => I::I64Store(m),
    }
}

fn bin(wide: bool, op: Bin) -> Option<I<'static>> {
    Some(match (wide, op) {
        (false, Bin::Add) => I::I32Add,
        (false, Bin::Sub) => I::I32Sub,
        (false, Bin::And) => I::I32And,
        (false, Bin::Or) => I::I32Or,
        (false, Bin::Xor) => I::I32Xor,
        (true, Bin::Add) => I::I64Add,
        (true, Bin::Sub) => I::I64Sub,
        (true, Bin::And) => I::I64And,
        (true, Bin::Or) => I::I64Or,
        (true, Bin::Xor) => I::I64Xor,
        (_, Bin::Xchg) => return None,
    })
}

/// What the rewriter needs to know about the module before its code.
#[derive(Default)]
struct Info {
    /// Parameter count of each type (0 for non-function types).
    params: Vec<u32>,
    /// Type of each function defined in the module, in order.
    funcs: Vec<u32>,
    /// Whether each memory (imports first) is 64-bit.
    mem64: Vec<bool>,
    shared: bool,
    atomics: bool,
}

fn scan(bytes: &[u8]) -> Option<Info> {
    let mut info = Info::default();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.ok()? {
            Payload::TypeSection(r) => {
                for group in r {
                    for sub in group.ok()?.into_types() {
                        let n = match &sub.composite_type.inner {
                            wasmparser::CompositeInnerType::Func(f) => f.params().len() as u32,
                            _ => 0,
                        };
                        info.params.push(n);
                    }
                }
            }
            Payload::ImportSection(r) => {
                for import in r {
                    if let TypeRef::Memory(m) = import.ok()?.ty {
                        info.shared |= m.shared;
                        info.mem64.push(m.memory64);
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    info.funcs.push(t.ok()?);
                }
            }
            Payload::MemorySection(r) => {
                for m in r {
                    let m = m.ok()?;
                    info.shared |= m.shared;
                    info.mem64.push(m.memory64);
                }
            }
            Payload::CodeSectionEntry(body) => {
                if !info.atomics {
                    let mut ops = body.get_operators_reader().ok()?;
                    while !ops.eof() {
                        if classify(&ops.read().ok()?).is_some() {
                            info.atomics = true;
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(info)
}

struct Lower {
    info: Info,
    next: usize,
}

/// Temporary locals added to a function that has atomics: five i32s, then five i64s.
#[derive(Clone, Copy)]
struct Temps(u32);

impl Temps {
    /// address, value, old value, expected, replacement
    fn get(self, wide: bool, which: u32) -> u32 {
        self.0 + if wide { 5 } else { 0 } + which
    }
}

const ADDR: u32 = 0;
const VAL: u32 = 1;
const OLD: u32 = 2;
const EXPECTED: u32 = 3;
const REPLACEMENT: u32 = 4;

impl Lower {
    fn emit(&mut self, f: &mut wasm_encoder::Function, temps: Temps, atomic: Atomic, memarg: Option<wasmparser::MemArg>) {
        let m = memarg.map(|m| self.mem_arg(m));
        let addr64 = memarg.is_some_and(|m| self.info.mem64.get(m.memory as usize).copied().unwrap_or(false));
        let a = temps.get(addr64, ADDR);
        match atomic {
            Atomic::Fence => {}
            Atomic::Notify => {
                f.instruction(&I::Drop).instruction(&I::Drop).instruction(&I::I32Const(0));
            }
            Atomic::Wait(n) => {
                for _ in 0..n {
                    f.instruction(&I::Drop);
                }
                f.instruction(&I::Unreachable);
            }
            Atomic::Load(wide, bits) => {
                f.instruction(&load(wide, bits, m.expect("memarg")));
            }
            Atomic::Store(wide, bits) => {
                f.instruction(&store(wide, bits, m.expect("memarg")));
            }
            Atomic::Rmw(wide, bits, op) => {
                let m = m.expect("memarg");
                let (v, old) = (temps.get(wide, VAL), temps.get(wide, OLD));
                f.instruction(&I::LocalSet(v)).instruction(&I::LocalSet(a));
                f.instruction(&I::LocalGet(a)).instruction(&load(wide, bits, m)).instruction(&I::LocalSet(old));
                f.instruction(&I::LocalGet(a));
                match bin(wide, op) {
                    Some(op) => {
                        f.instruction(&I::LocalGet(old)).instruction(&I::LocalGet(v)).instruction(&op);
                    }
                    None => {
                        f.instruction(&I::LocalGet(v));
                    }
                }
                f.instruction(&store(wide, bits, m)).instruction(&I::LocalGet(old));
            }
            Atomic::Cmpxchg(wide, bits) => {
                let m = m.expect("memarg");
                let (e, r, old) = (temps.get(wide, EXPECTED), temps.get(wide, REPLACEMENT), temps.get(wide, OLD));
                f.instruction(&I::LocalSet(r)).instruction(&I::LocalSet(e)).instruction(&I::LocalSet(a));
                f.instruction(&I::LocalGet(a)).instruction(&load(wide, bits, m)).instruction(&I::LocalTee(old));
                f.instruction(&I::LocalGet(e));
                // A narrow cmpxchg compares with the expected value cut to its width.
                let full = if wide { 64 } else { 32 };
                if bits < full {
                    let mask = (1u64 << bits) - 1;
                    if wide {
                        f.instruction(&I::I64Const(mask as i64)).instruction(&I::I64And);
                    } else {
                        f.instruction(&I::I32Const(mask as u32 as i32)).instruction(&I::I32And);
                    }
                }
                f.instruction(if wide { &I::I64Eq } else { &I::I32Eq });
                f.instruction(&I::If(BlockType::Empty));
                f.instruction(&I::LocalGet(a)).instruction(&I::LocalGet(r)).instruction(&store(wide, bits, m));
                f.instruction(&I::End).instruction(&I::LocalGet(old));
            }
        }
    }
}

impl Reencode for Lower {
    type Error = std::convert::Infallible;

    fn parse_function_body(
        &mut self,
        code: &mut wasm_encoder::CodeSection,
        func: wasmparser::FunctionBody<'_>,
    ) -> Result<(), wasm_encoder::reencode::Error<Self::Error>> {
        let ty = self.info.funcs.get(self.next).copied().unwrap_or(0);
        self.next += 1;
        let params = self.info.params.get(ty as usize).copied().unwrap_or(0);
        let mut locals = Vec::new();
        let mut count = 0u32;
        for pair in func.get_locals_reader()? {
            let (n, t) = pair?;
            count += n;
            locals.push((n, self.val_type(t)?));
        }
        let mut has_atomics = false;
        let mut ops = func.get_operators_reader()?;
        while !ops.eof() {
            if classify(&ops.read()?).is_some() {
                has_atomics = true;
                break;
            }
        }
        if !has_atomics {
            let mut f = wasm_encoder::Function::new(locals);
            let mut ops = func.get_operators_reader()?;
            while !ops.eof() {
                f.instruction(&self.parse_instruction(&mut ops)?);
            }
            code.function(&f);
            return Ok(());
        }
        let temps = Temps(params + count);
        locals.push((5, ValType::I32));
        locals.push((5, ValType::I64));
        let mut f = wasm_encoder::Function::new(locals);
        let mut ops = func.get_operators_reader()?;
        while !ops.eof() {
            let op = ops.read()?;
            match classify(&op) {
                Some((atomic, memarg)) => self.emit(&mut f, temps, atomic, memarg),
                None => {
                    f.instruction(&self.instruction(op)?);
                }
            }
        }
        code.function(&f);
        Ok(())
    }
}

/// The module with its atomics rewritten, if it has atomics and no shared memory.
pub(super) fn lower(bytes: &[u8]) -> Option<Vec<u8>> {
    let info = scan(bytes)?;
    if info.shared || !info.atomics {
        return None;
    }
    let mut lower = Lower { info, next: 0 };
    let mut module = wasm_encoder::Module::new();
    lower.parse_core_module(&mut module, Parser::new(0), bytes).ok()?;
    let _ = RoundtripReencoder;
    Some(module.finish())
}
