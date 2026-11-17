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

## Scores

Full runs on the 2-core build machine. Each row is one change in `vendor/boa`.

| Change | Richards | DeltaBlue | Crypto | RayTrace | EarleyBoyer | RegExp | Splay | NavierStokes | Score |
|---|---|---|---|---|---|---|---|---|---|
| Boa 0.22.0 as released | 139 | 127 | 150 | 220 | 269 | 38.5 | 632 | 366 | 231 |
