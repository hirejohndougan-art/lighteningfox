# Baseline JIT for Boa

Ironswift's copy of Boa (`vendor/boa`) gets a baseline JIT built on Cranelift. It turns hot
functions into machine code that does the same work as the interpreter, one bytecode instruction
at a time, without the interpreter's dispatch and decoding. It is built in stages:

| Task | What | Status |
|---|---|---|
| J1 | This plan, call/loop counters, the `boa_jit` cargo feature, Cranelift and W^X memory | done |
| J2 | Compile functions that use only locals, numbers, comparisons, branches and loops | done |
| J3 | Property reads/writes, calls in both directions, exceptions | done |
| J4 | On by default; x86-64 and ARM64; Settings switch; security review | |

## Building and switching it on

The JIT is behind a cargo feature, off by default:

    cargo build --release --features boa_jit          # Ironswift (forwards to boa_engine/jit)
    cargo test --release -p boa_engine --features jit jit::   # the JIT's own tests

When built in, the `BOA_JIT` environment variable picks the mode (`boa_engine::jit::set_mode`
does the same from code, for the Settings switch in J4):

| `BOA_JIT` | Effect |
|---|---|
| `off` (or `0`, `false`) | Never tier up. Same behaviour as a build without the feature. |
| `on` (default) | Tier up after `CALL_THRESHOLD` (1,000) calls or `LOOP_THRESHOLD` (10,000) loop iterations. |
| `always` | Tier up on the first call or loop iteration. Used to run test262 and Boa's tests with every function compiled. |

Code is in `vendor/boa/boa_engine/src/jit/`: `mode.rs` (the switch), `counters.rs` (per code
block counters and tier state), `support.rs` (which opcodes the first version takes),
`backend.rs` (Cranelift and executable memory), `tests.rs`.

## When a function tiers up

Every `CodeBlock` carries `JitCounters` (only with the feature): a call counter, a loop counter
and a `TierState` (`Interpreted`, `Eligible`, `Rejected(reason)`; J2 adds `Compiled`).

- **Calls** are counted in `function_call` and `function_construct`
  (`builtins/function/mod.rs`), the two places every ordinary JavaScript call and `new` goes
  through.
- **Loop iterations** are counted in `IncrementLoopIteration`, which the bytecode compiler emits
  once per iteration at the head of every `for`, `while`, `do`, `for-in` and `for-of` loop. A
  function called once with a long loop still gets hot.
- When either counter reaches its threshold, `jit::tier_up` runs once for that code block. In J1
  it only scans the bytecode (`support::scan`) and records `Eligible` or `Rejected`, with the
  first unsupported opcode and its offset; `jit::stats()` counts both. J2 compiles eligible
  blocks right there, on the main thread (baseline compiles are fast: no background thread, no
  locking against the interpreter).
- Counters saturate at `u32::MAX`; a code block is scanned at most once. A rejected block is
  never retried (J3 widens the opcode list instead).

## What the first version compiles (J2)

`support::is_supported` is the authoritative list. In words:

- **Constants:** `StoreZero`, `StoreOne`, `StoreInt8/16/32`, `StoreFloat`, `StoreDouble`,
  `StoreNan`, `StorePositive/NegativeInfinity`, `StoreTrue`, `StoreFalse`, `StoreUndefined`.
- **Locals:** `Move`, `GetArgument`.
- **Arithmetic:** `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Neg`, `Pos`, `Inc`, `Dec`, the bitwise
  operators and the three shifts.
- **Comparisons:** `LessThan`, `LessThanOrEq`, `GreaterThan`, `GreaterThanOrEq`, `StrictEq`,
  `StrictNotEq`, `Eq`, `NotEq`, `LogicalNot`.
- **Control flow:** `Jump`, `JumpIfTrue`, `JumpIfFalse`, the fused `JumpIfNot{LessThan,
  LessThanOrEqual, GreaterThan, GreaterThanOrEqual, Equal}`, `LogicalAnd`, `LogicalOr`,
  `IncrementLoopIteration`, `SetAccumulator`, `CheckReturn`, `Return`. `CheckReturn` ends every
  function and does nothing for an ordinary call; in a `new` call compiled code bails out there
  and the interpreter builds the result.

Whole functions are rejected when they are generators or async, class constructors, or have
exception handlers (`try`). Anything touching objects, strings, environments (closures that
capture a loop variable), `arguments` or other calls is rejected in J2; J3 adds property access,
calls and exceptions by calling into the interpreter's existing operation functions.

`Pow` is left out on purpose: JavaScript's `**` differs from C's `pow` for `NaN` and `±1`
exponents, so it needs its own helper. `Mod` uses `fmod` semantics, which match JavaScript's `%`
on doubles.

Type checks happen at run time. Each operation checks its inputs (an int32 or double for
arithmetic; a boolean for branches on booleans) and **bails out** when they are something else,
for example `Add` on a string. The interpreter then does that instruction and the rest.
Integer arithmetic is done on int32 with overflow checks; on overflow the result becomes a
double, exactly as the interpreter's `Add` does.

## How J2 is built

`compiler.rs` turns each bytecode instruction into a Cranelift block, so any jump target or
loop header can be entered. `helpers.rs` holds the operations compiled code calls rather than
inlining: each wraps the interpreter's own `*_fast` path for that opcode (`add_fast`,
`lt_fast`, …), so results are bit-for-bit the interpreter's, int32-or-double representation
included; they return a "no result" word for anything that isn't a number and compiled code
bails out. In line: int32 `Add`/`Sub` with overflow checks, `Inc`/`Dec` below the int32
limits, and the int32 compare-and-branch of loop conditions. `runtime.rs` compiles at tier-up
(on the thread's `JitBackend`), enters compiled code at the end of `function_call` and in
`IncrementLoopIteration`, and counts bailouts: after 64 bailouts, a function that bails out on
more than one entry in four goes back to the interpreter for good (`Rejected("bails out")`).
Set `BOA_JIT_LOG=1` to see functions Cranelift refused.

Compiled code gets a `JitFrame` (register and argument pointers, argument count, the frame's
loop counter and limit, an accumulator slot and the entry pc) and returns the pc the
interpreter goes on from. Reaching `Return` is just an exit at the `Return` instruction with the
accumulator set, so early exits and frame popping stay the interpreter's.

Checked (2026-11-18): test262 at the commit Boa 0.22 pins, 53,578 tests, gives the same result
for every test with `BOA_JIT=off` and `BOA_JIT=always` (51,226 pass, 571 fail, 1,781 ignored,
95.61%); Boa's engine tests and Ironswift's 404 tests pass with `BOA_JIT=always`. To repeat:
`tools/test262.sh` with `--features boa_engine/jit` added to its `cargo build`, run with each
mode and compare the `latest.json` files.

Measured (`docs/boa-speed.md`): number loops 2.9× faster; the V8 suite unchanged, since its hot
code reads properties and calls functions.

## What J3 adds: everything else, one instruction at a time

J3 takes every function except generators, async functions and class constructors (those stay
in the interpreter). Opcodes J2 compiles in line keep their machine code; **any other opcode is
run by the interpreter for compiled code**: compiled code calls `runtime::step(frame, pc)`,
which runs that one instruction with the interpreter's own operation
(`vm::opcode::OPCODE_JIT_STEPS`, generated beside the interpreter's handler table), and
compiled code goes on from where the instruction left the frame. So property reads and writes,
object and array literals, closures and environments, `for-in`/`for-of`, `new`, `typeof`,
`delete`, `in`, spread, `arguments`, `throw` and the rest behave exactly as interpreted, inline
caches included. `support::scan` accepts an opcode when J2 compiles it or its operation finishes
in place (`vm::opcode::jit_steppable`: its result is `()`, `JsResult<()>` or a `JsError`, plus
`Throw`, `ReThrow` and `Exception`); the opcodes left out return or yield, and only appear in
generators and async functions.

The J2 guards changed too: where J2 bailed out (a string in `i < n`, an object in a register
about to be overwritten, `Move` of an object), compiled code now hands that instruction to the
interpreter and carries on. Only the loop limit still leaves compiled code the old way.

- **Where compiled code goes on after a step:** the next instruction (the usual case, a direct
  jump), or any other instruction: every instruction is an entry, through a `dispatch` block that
  switches on the pc (a jump the step took, a `catch` it landed in). The register file's address
  is read from the `JitFrame` at every access, since a step can grow the VM stack.
- **Exceptions:** a step that throws runs the interpreter's `handle_error`, the same code the
  interpreter runs: a `catch` or `finally` in this function continues in compiled code; one
  further up the stack, or no handler, ends the compiled run (`EXIT_RESUME`) and the interpreter
  carries on from the frame that handles it. An exception that reaches a frame returning to
  Rust ends it with `EXIT_ERROR` and the entry point raises it.
- **Calls, compiled → anything:** a call to a native function finishes inside the step. A call
  to a JavaScript function pushes its frame, and compiled code ends its run with `EXIT_RESUME`.
  `runtime::run` then goes on in the callee's machine code if it has some (else the interpreter
  runs it). When the callee returns, the interpreter's `handle_return` calls
  `jit::enter_after_return`, and the caller goes on in machine code right after the call. So a
  JavaScript call never nests machine frames: deep recursion uses no more native stack than the
  interpreter, and the recursion limit is reached at the same depth.
- **Calls, anything → compiled:** a function with machine code starts in it from the `Call` and
  `CallSpread` opcodes (`jit::enter_at_call`), at a loop header, and after a call returns. Calls
  from Rust (`JsObject::call`) are not entered at their start: their frame is marked to return
  to Rust only after it is pushed, so it runs interpreted until its first loop header. Native
  loops that charge the loop limit (`String.prototype.repeat`) use
  `IncrementLoopIteration::charge`, which never enters compiled code (it ran the caller's
  machine code mid-call before).
- **Memory:** each `Context` owns its Cranelift module (`runtime::ContextJit`) and the list of
  code blocks compiled into it; dropping the context frees the machine code
  (`JitBackend::drop` calls `free_memory`, which `cranelift-jit` never does by itself) and sends
  those code blocks back to the interpreter, where they can tier up again in another context
  (Ironswift shares compiled scripts between pages). Copies of a code block start uncompiled.
  Code blocks with more than 32 KB of bytecode (`support::MAX_BYTECODE`) stay interpreted:
  Cranelift's time and memory grow faster than the function, and test262's generated scripts
  made it use gigabytes.
- **GC:** compiled code still keeps no `JsValue` in machine registers across a step: every
  instruction reads its operands from the register file and a step re-does a whole instruction.
  A step is where a collection can happen, and the register file is then re-read.

Checked (2026-11-18): test262 at the commit Boa 0.22 pins gives a byte-identical
`latest.json` for all 53,578 tests with `BOA_JIT=off` and `BOA_JIT=always` (51,226 pass, 95.61%;
the forced run peaks under 2 GB); Boa's 1,146 engine tests pass both ways.

Tests: `jit/tests.rs` and `jit/tests_j3.rs` (objects, calls both ways, closures, exceptions,
getters and proxies run once, generators called from compiled code, recursion limits, calls from
Rust), each comparing the JIT forced on with the interpreter. The `jit_diff` fuzz target
(`fuzz/`) runs any script both ways and fails on a difference.

## How compiled code runs and bails out

Registers stay where they are. The interpreter keeps a frame's registers in the VM stack
(`vm.stack`, slots `rp .. rp + register_count`), and compiled code reads and writes those same
slots. So there is never a separate compiled frame to convert: the frame *is* the interpreter's
frame.

- **Values:** Boa's `JsValue` is NaN-boxed (the default `nan_boxed.rs` layout): a 64-bit word
  where `0x7FF9_0000_xxxx_xxxx` is an int32, `0x7FFA…0/1` false/true, `0x7FFB…1` undefined, the
  pointer types are `0x7FFC`–`0x7FFF`, and everything else is a double. Compiled code tests the
  top 16 bits and works on the payloads directly. Numbers, booleans and `undefined` own nothing,
  so compiled code may overwrite a register holding one with raw bits. It never overwrites a
  register holding a pointer (`0x7FFC`–`0x7FFF`): that would leak a reference count. The J2
  compiler bails out before such a write, and J3 does those writes through a helper that drops
  the old value. The JIT refuses to build with the `jsvalue-enum` layout (it is never used by
  Ironswift).
- **Entry:** a compiled function has the signature
  `extern "C" fn(frame: *mut JitFrame) -> u32`, where `JitFrame` holds the register base
  pointer, the argument pointer and count, the loop counter and limit, an accumulator slot, and
  the entry `pc`. The function begins with a
  switch on the entry `pc`: `0` for a normal call, or the `pc` of any loop header
  (`IncrementLoopIteration`) for on-stack entry when a loop got hot in the middle of a call.
  Because registers are already in memory, entering at a loop header needs no translation.
- **Exit:** the return value is a status: `RETURNED` (the result is in the accumulator, the
  interpreter finishes the call as after `Return`) or `BAILED_OUT`. Before bailing out, compiled
  code writes back any values it holds only in machine registers and sets `frame.pc` to the start
  of the instruction it could not do. Every J2 instruction is checked *before* it has any effect,
  so the interpreter can always redo the instruction from its start. After a bailout the
  interpreter carries on and re-enters compiled code at the next loop header or call. If a
  function bails out often (after 64 bailouts, more than one entry in four), it is marked
  `Rejected("bails out")` and its compiled code is no longer used.
- **Where it is called:** at the end of `function_call`, once the new frame is set up (never for
  `new`), and in the `IncrementLoopIteration` handler for on-stack entry. Builds with the `fuzz`
  feature never enter compiled code, so its instruction budget stays exact. Work done in compiled
  code isn't charged to `run_async_with_budget`'s budget (it only decides when async code
  yields; the loop limit still applies).
- **Runtime limits:** compiled loops still count iterations against
  `RuntimeLimits::loop_iteration_limit` and bail out when it is reached, so the interpreter raises
  the same error at the same place.

## GC roots in compiled frames

Boa's collector (`boa_gc`) does not move objects, and it only runs when something allocates.
A value is a root while a reference outside the GC heap holds it, and the VM stack holds every
register of every frame. That gives two rules for compiled code:

1. **J2 code never allocates and never calls out**, and it only keeps numbers and booleans in
   machine registers, so no collection can happen while it runs and there is nothing to root.
2. **From J3 on, every call out of compiled code is a safepoint.** Before the call, every
   `JsValue` the compiled code has produced is stored in its register slot in the VM stack, so the
   collector sees it as a root. After the call, compiled code reloads what it needs from the
   slots: the call may have run a collection, and it may have grown `vm.stack` (a `Vec`), which
   moves the register file, so the register base pointer is also reloaded from the frame (`rp`
   plus the stack's current pointer). A raw object pointer in a machine register is only valid
   while the slot it was loaded from is unchanged and no call has happened since.

No stack maps are needed because compiled code never holds the only reference to anything.

## Executable memory (W^X)

Compiled code lives in memory that is never writable and executable at the same time.
`cranelift-jit` maps pages read+write while it writes a batch of functions, and
`finalize_definitions` turns them read+execute (on ARM64 it also flushes the instruction cache and
sets BTI when the CPU has it). Finalized pages are never written again; later functions go into
new pages. Patching compiled code in place (for inline caches) is not done: J3 keeps its caches in
data memory that compiled code reads. The test `executable_memory_is_never_writable` checks the
page permissions through `/proc/self/maps`.

`JitBackend` (one per `Context`, since J3) owns the Cranelift module, and frees all of its
code when it is dropped with the context. Code blocks whose functions are collected leave their
machine code behind until then (J4 measures that on real pages and can add a size cap).

## Cranelift version

`cranelift-codegen`, `cranelift-frontend`, `cranelift-jit`, `cranelift-module` and
`cranelift-native` 0.137.2 (needs Rust 1.97). WebAssembly (task P1a) chose the `wasmi` interpreter
rather than `wasmtime` (`docs/ARCHITECTURE.md`), so there is no second copy of Cranelift to keep
in step with. If WebAssembly ever moves to `wasmtime`, move the JIT to the Cranelift version that
`wasmtime` release uses.

## Security notes (for `SECURITY.md` in J4)

- W^X as above; no RWX mappings, no code patching.
- Every compiled operation keeps the interpreter's semantics; differences are found with the J3
  fuzz target (each program run with and without the JIT) and test262 with `BOA_JIT=always`.
- Type guards are the JIT's main risk: a missing guard turns a pointer into a number. Guards are
  generated in one place and tested with values of every type.
- `BOA_JIT=off` and the Settings switch turn it off without rebuilding.
