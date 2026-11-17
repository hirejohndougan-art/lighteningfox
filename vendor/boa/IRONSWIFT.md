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

Changes 1–4 together, instructions per benchmark vs Boa 0.22.0: Richards −9%, DeltaBlue −7%,
RayTrace −6%, Splay −8%, EarleyBoyer −8%, RegExp −1%, NavierStokes +1%, Crypto +1%.

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
