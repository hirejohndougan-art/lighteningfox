# Ironswift's copy of Boa

These are the Boa 0.22.0 crates from crates.io (MIT/Unlicense, see each crate's README), copied
here so Ironswift can make Boa faster. `Cargo.toml` points at them with `[patch.crates-io]`.

To see what we changed, diff a crate against the crates.io release, e.g.
`diff -ru ~/.cargo/registry/src/*/boa_gc-0.22.0/src vendor/boa/boa_gc/src`.

Measure every change with the V8 benchmark suite (see `docs/boa-speed.md`):

    cargo run --release --example jsbench -- --v8

## Changes

(Each change: what, why, and the before/after score.)

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
