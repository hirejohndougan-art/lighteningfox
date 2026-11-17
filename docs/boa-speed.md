# Boa speed

Ironswift runs page scripts with its own copy of Boa in `vendor/boa` (see `vendor/boa/IRONSWIFT.md`
for the list of changes). This page tracks how fast it is.

## Measuring

The V8 benchmark suite (version 7: Richards, DeltaBlue, Crypto, RayTrace, EarleyBoyer, RegExp,
Splay, NavierStokes; the files come from Google's Octane repository, BSD license) is in
`tools/bench/v8`. Higher scores are better.

    cargo run --release --example jsbench -- --v8            # full run, about 2.5 minutes
    cargo run --release --example jsbench -- --v8 --quick    # rough run, about 15 seconds
    cargo run --release --example jsbench -- --v8 Splay      # one benchmark
    cargo run --release --example jsbench -- some.js         # time any script

Run the full suite for numbers that go in the table below; the quick run moves by ±15% from run
to run.

## Profiling

`tools/profile_js.sh` profiles a script and draws a flame graph:

    tools/profile_js.sh tools/bench/v8/splay.js     # uses perf if installed, else valgrind

To profile a whole page load instead (DOM, layout and scripts together):

    valgrind --tool=callgrind ./target/release/ironswift --png out.png tests/real-world/react.html
    callgrind_annotate --inclusive=no callgrind.out.* | head -60

### What the profiles show (2026-11-17)

- **Real pages (React 18, Vue 3, jQuery in `tests/real-world`):** loading is dominated by
  *parsing*: the lexer and its token buffer, `malloc`/`free`/`memcpy` while building the syntax
  tree, parse errors created and thrown away during backtracking
  (`drop_glue::<boa_parser::error::Error>`, ~1.5%), keyword lookups (`phf` map, ~1.2%) and
  `Sym::to_js_string` while compiling. Running the libraries' setup code is a small share.
  So page-load speed mostly comes from the parser and compiler work (parse/compile tables in
  `vendor/boa/IRONSWIFT.md`).
- **Script-heavy work (the V8 suite, app logic after load):** property reads (`get_by_name` and
  the inline cache), reference-count updates when values are copied and dropped, function call
  setup (`push_frame`, `function_call`, `handle_return`) and `this` lookups. Inline caches,
  borrowed objects, cross-crate inlining and deep prototype caches address the first two.
  Function call overhead is the next big target.

## Scores

Full runs on the 2-core build machine. Each row is one change in `vendor/boa`.

| Change | Richards | DeltaBlue | Crypto | RayTrace | EarleyBoyer | RegExp | Splay | NavierStokes | Score |
|---|---|---|---|---|---|---|---|---|---|
| Boa 0.22.0 as released | 139 | 127 | 150 | 220 | 269 | 38.5 | 632 | 366 | 231 |
| Boa 0.22.0 as released (rerun 2026-11-17 04:20, same session as next row) | 126 | 121 | 162 | 230 | 248 | 37.4 | 548 | 317 | 217 |
| Batch M runtime changes 1–7 + parser/compiler changes (`vendor/boa/IRONSWIFT.md`) | 170 | 158 | 152 | 237 | 292 | 38.3 | 544 | 384 | 238 |

Run-to-run noise on this machine is about ±10% per benchmark, so compare rows from the same
session. Instruction counts (valgrind, in `vendor/boa/IRONSWIFT.md`) are steadier: every
benchmark now does 2–15% less work than stock. SplayLatency, which scores GC pauses, was
1093 vs 1145 stock: the bigger GC heap (change 1) makes each pause a little longer. A
generational or incremental collector is the fix if pauses become a problem on real pages.
