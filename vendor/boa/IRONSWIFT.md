# Ironswift's copy of Boa

These are the Boa 0.22.0 crates from crates.io (MIT/Unlicense, see each crate's README), copied
here so Ironswift can make Boa faster. `Cargo.toml` points at them with `[patch.crates-io]`.

To see what we changed, diff a crate against the crates.io release, e.g.
`diff -ru ~/.cargo/registry/src/*/boa_gc-0.22.0/src vendor/boa/boa_gc/src`.

Measure every change with the V8 benchmark suite (see `docs/boa-speed.md`):

    cargo run --release --example jsbench -- --v8

## Changes

### boa_ast: hash index for scope bindings

`src/scope.rs`: scopes with 12 or more bindings keep a hash index from name to binding (marked
"Ironswift change"). Name resolution used to scan every binding in each enclosing scope, which
made parsing and compiling minified bundles (hundreds of top-level names) close to quadratic.
Measured with the parse/compile split on real libraries (ms, best of 3):

| Script | Parse before → after | Compile before → after |
|---|---|---|
| react-dom 18 (132 KB) | 72.6 → 51.5 | 26.0 → 13.8 |
| Vue 3 (147 KB) | 65.8 → 55.7 | 35.0 → 22.6 |
| lodash (73 KB) | 43.8 → 35.8 | 28.5 → 11.1 |
| jQuery 3.7 (88 KB) | 33.2 → 32.3 | 10.1 → 8.3 |

The V8 suite barely changes (its scripts are small and not minified); page load is where it shows.

### Parser and compiler: stop copying syntax trees

- `boa_parser/src/parser/expression/primary/mod.rs`: a parenthesized expression is moved into its
  node instead of cloned. Minified bundles wrap the whole library in `(function(){...})`, so the
  entire tree was copied once.
- `boa_ast/src/operations/mod.rs`: `VarScopedDeclaration` borrows from the tree
  (`VarScopedDeclaration<'a>`) instead of cloning each `var` initializer and function body; the
  scope analyzer and bytecode compiler only read names from it. Callers in `scope_analyzer.rs` and
  `boa_engine/src/bytecompiler/declarations.rs` adjusted.
- `boa_parser/src/lexer/identifier.rs`: ASCII fast path for identifier characters (skips the
  Unicode table lookup).

On top of the scope index above (ms, best of 9, parse / compile):

| Script | Scope index only | + these changes |
|---|---|---|
| Ironswift prelude.js (88 KB) | 20.0 / 4.7 | 14.5 / 4.3 |
| Vue 3 | 62.4 / 22.4 | 47.2 / 13.5 |
| lodash | 33.0 / 11.0 | 23.5 / 7.9 |
| react-dom 18 | 51.5 / 13.8 | 38.5 / 10.1 |

Instructions to load react-dom (valgrind): 510 M stock → 269 M. V8 suite unchanged (within noise).

### Runtime speed (batch M)

Instruction counts below come from valgrind (`--tool=callgrind`) on a fixed amount of work per
benchmark; wall-clock times on the shared build machine vary by ±15%, too much to judge small
changes.

1. **Garbage collector (`boa_gc/src/lib.rs`).** Start collecting at 8 MB instead of 1 MB and let
   the heap double between collections instead of growing by ~1.4×. Fewer collections on busy
   pages, at the cost of a little more memory.
2. **Inline caches (`boa_engine/src/vm/inline_cache`, `object/shape`).** Each cache entry keeps
   the shape's address; a lookup compares addresses and only checks that the weak shape is
   still alive on a match. Before, every lookup upgraded every cached weak shape to a strong
   one (two reference-count updates each) and returned a cloned shape.
3. **Borrowed objects in property access (`value/mod.rs`, `vm/opcode/get|set/property.rs`).**
   `JsValue::as_object_ref` borrows the object inside a value instead of cloning it, used on
   the `obj.name` get/set paths.
4. **Inlining across crates (`boa_gc`).** Reference-count and borrow-flag helpers are marked
   `#[inline]` so `boa_engine` doesn't call into `boa_gc` for every object borrow and clone.
   The "is a sweep running" check on every `Gc` drop reads a global counter first and only
   asks the thread-local while some thread is actually sweeping.
5. **Rope strings (`boa_string/src/vtable/rope.rs`).** `a + b` with a result of 128+ code
   units links the two strings instead of copying; the characters are joined on first read.
   Building a 2 MB string with 200,000 `+=` went from 80 s to 1.4 s. Flattening and dropping
   use an explicit stack, so million-deep ropes are fine.

6. **Deep prototype caches (`vm/inline_cache`, `vm/opcode/get/property.rs`,
   `object/shape`).** Boa only cached properties found on the object or its direct prototype;
   methods inherited two or more levels up (class hierarchies, `Object.prototype` methods on
   instances) always took the slow path. Each cache site can now remember one deeper lookup:
   the shape of the receiver and of every prototype up to the holder. A per-thread counter
   (`shape_epoch`) goes up whenever a unique shape changes in place, since those keep their
   address; a hit needs the same shapes and the same counter. Proxies and other exotic
   objects are never cached this way. `deep_prototype_cache_invalidation` tests shadowing,
   deletes, prototype swaps, getters and proxies along the chain.

7. **Free drops for plain values (`value/inner/nan_boxed.rs`).** Dropping a number, boolean,
   `null` or `undefined` used to call the full drop routine; a single bit test now skips it,
   and only values holding a pointer make a call. NavierStokes −5%, Crypto −4.5%.

Changes 1–4 together, instructions per benchmark vs Boa 0.22.0: Richards −9%, DeltaBlue −7%,
RayTrace −6%, Splay −8%, EarleyBoyer −8%, RegExp −1%, NavierStokes +1%, Crypto +1%.
With changes 6 and 7 as well: Richards −9%, DeltaBlue −12%, RayTrace −6%, Splay −8%,
EarleyBoyer −15%, RegExp −2%, NavierStokes −4%, Crypto −4%.

### Bytecode compiler: fewer register moves (X22)

`cargo run --release --example bcdump -- file.js` prints the bytecode for a script and every
function in it (`CodeBlock::disassemble_all`, added for this). Three changes, all in
`boa_engine/src/bytecompiler`:

- **Property reads use a local's own register** (`mod.rs`, `PropertyAccess::Simple`): `p.x`,
  `a[i]` and `a[0]` on a local variable read its register directly instead of first copying it
  into a temporary (`Move`). Only when nothing runs between reading the object and using it: a
  constant name, or a key that is a literal or a name.
- **`local op= simple`** (`expression/assign.rs`): `s += a[i]`, `n *= 2` compute in the local's
  register (`Add s, s, x`) instead of copying out and back; as a statement the result isn't
  copied at all (`compile_assign_inner(.., discard)`). Only when the right side can't assign to
  the local (a literal, a name, or a property read of a name or `this` with a literal or name
  key), the binding is writable (so `const` still throws), and it lives in a register (not
  captured by a closure).
- **`return x` straight to the accumulator** (`statement/mod.rs`) when there's nothing to unwind
  (no `finally`, no iterator to close, no environment to pop), instead of Push, Pop and
  SetAccumulator.

A `for` loop summing an array went from 11 instructions per iteration to 7. Timing: loops with
property reads 27% faster (1.48 s → 1.07 s, `sum`/`dist` microbenchmark); the V8 suite is
unchanged within noise (Richards about +15%). Checked with Boa's 1,116 engine tests, Ironswift's
tests and edge cases (assignment inside the right side, `const`, captured variables, getters,
`finally`, `for…of`, generators, async, BigInt).

## Testing changes

Boa's own tests need its workspace: clone `boa-dev/boa` at tag `v0.22`, copy a changed crate's
`src/` over `core/<crate>/src`, and run `cargo test --release -p boa_engine --lib` there.
`tools/test262.sh` runs the official conformance suite.

### boa_engine: compiled-script cache (`src/script_cache.rs`)

`Context::eval_cached` / `Script::parse_cached` keep the compiled code of scripts of 2 KB or
more, per thread, and reuse it when the same source is evaluated again in any context: the
browser's own prelude on every page, a site's bundle on each of its pages, repeat visits.
Ironswift's `js.rs` uses it for the prelude files and page scripts.

Reuse is only safe when the new realm's global scope matches what it was before the original
compile, so each entry stores a snapshot of the global scope's bindings before and after
(`Scope::snapshot` / `Scope::restore`, new in `boa_ast/src/scope.rs`); a hit restores the
"after" snapshot, exactly what compiling again would produce. Scripts containing `eval(` are
never cached. Budget: about 24 MB of compiled code (estimated as 3× source), least recently used
dropped first; `script_cache::clear()` empties it.

Per page, parse + compile on a hit (ms): prelude.js 23.6 → 0.07, indexeddb.js 11.7 → 0.04,
react-dom 53.9 → 0.11, lodash 34.4 → 0.06.

Not done: a cache on disk (so the first page after starting the browser also skips parsing).
Boa's compiled code holds scope graphs and garbage-collected pointers and has no serialized
form; writing one is a larger job.

### boa_engine: baseline JIT groundwork (`src/jit/`, feature `jit`, task J1)

Off by default; Ironswift's `boa_jit` feature turns it on. With the feature, every `CodeBlock`
has call and loop counters (`function_call`/`function_construct` and `IncrementLoopIteration`
bump them) and a tier state; a hot block is scanned once for the opcodes the first compiler will
take. Cranelift 0.137 compiles into W^X memory (`JitBackend`). Nothing runs compiled code yet.
Plan and rules: `docs/JIT.md`. Tests: in the Boa workspace above, also add the `jit` feature and
the five optional `cranelift-*` dependencies (0.137.2) to `core/engine/Cargo.toml`, then
`cargo test --release -p boa_engine --features jit --lib jit::`. Boa's 1,116 engine tests pass
with `BOA_JIT=always`, and without the feature.

### boa_engine: ArrayBuffers over embedder-owned bytes

`builtins/array_buffer/block.rs` (new) and `builtins/array_buffer/mod.rs`: an `ArrayBuffer`'s
bytes are a `Block`, either owned (as before) or external (a pointer and length the embedder
owns). `JsArrayBuffer::from_external` (`object/builtins/jsarraybuffer.rs`) makes one; it must
have a detach key, so script can't transfer it and `resize` refuses it (it's fixed-length), and
detaching it drops the pointer without freeing anything. Ironswift uses it for
`WebAssembly.Memory.prototype.buffer`, so typed arrays see the module's memory itself (task P1b).

### boa_engine: global `let` declared by a later script

`environments/runtime/mod.rs` (`later_global_lexical`, used by `get_binding`, `set_binding`
and `is_initialized_binding`) and `vm/opcode/get/name.rs` (`GetNameGlobal`): a function
compiled before a later script declared `let x` had `x` bound to the global object, so it
threw "x is not defined" (and assigning created a global property beside the `let`). Now a
miss on the global object (and every assignment) checks the realm's global declarations first,
with the TDZ and `const` errors. Only misses pay for it: found global properties take the same
path as before. Found by WPT `wasm/jsapi` (`instanceTestFactory.js` reads a `let` from the test
file loaded after it); real pages split across scripts hit the same thing.

### boa_engine: baseline JIT, numbers and loops (`src/jit/compiler.rs`, `helpers.rs`, `runtime.rs`, task J2)

Eligible hot functions are compiled with Cranelift and run on the frame's own registers;
entered from `function_call` and `IncrementLoopIteration`. Small `jit`-only accessors added:
`JsValue::jit_bits`/`jit_from_bits` (`value/mod.rs`, `value/inner/nan_boxed.rs`) and
`Stack::jit_pointers` (`vm/mod.rs`). 19 JIT tests compare compiled and interpreted results for
every operator over edge values; the 1,116 engine tests pass with `BOA_JIT=always`. Number
loops 2.9× faster; V8 suite unchanged (`docs/boa-speed.md`).


### boa_engine: storing an array's `length` isn't cached (`src/builtins/array/mod.rs`)

`a.length = 0` goes through ArraySetLength, which deletes elements. The named-store inline
cache recorded it as a plain slot write, so from the second run of the same code on, only the
`length` slot changed: the elements stayed, and `a[0] = x` then overwrote one in place while
`length` stayed 0. Apollo's cache (`optimism`) empties its value arrays this way and threw
"unknown value" on NYT. Array `length` stores are now marked not cacheable (reads still are).

### boa_engine: baseline JIT, J3 (properties, calls, exceptions)

`jit/` (`runtime.rs` rewritten, `compiler.rs`, `support.rs`, `tests_j3.rs`), `vm/opcode/mod.rs`
(`OPCODE_JIT_STEPS` and `jit_steppable`, generated beside the handler tables, feature `jit`),
`vm/completion_record.rs` (`JitStep`, `IntoJitStep`), `vm/mod.rs` (`handle_error` is
`pub(crate)`; `handle_return` re-enters a compiled caller), `vm/opcode/call/mod.rs` (`Call` and
`CallSpread` enter a compiled callee), `builtins/function/mod.rs` (no longer enters compiled
code in `function_call`), `vm/opcode/iteration/loop_ops.rs` and `builtins/string/mod.rs`
(`IncrementLoopIteration::charge` for native loops). Compiled code hands every instruction it
doesn't compile to the interpreter one at a time, and JavaScript calls go through the
interpreter's frames, so nothing nests. Details in `docs/JIT.md`.

### boa_engine: baseline JIT, J4 (on by default)

`jit/support.rs` (`MAX_STEPPED_PERCENT`, `worth_compiling`), `jit/mod.rs` (`tier_up` rejects
"mostly interpreter work" in the default mode), `jit/tests.rs`. In `BOA_JIT=on` a function
whose instructions are more than 25% ones compiled code hands to the interpreter stays
interpreted; `always` is unchanged.

