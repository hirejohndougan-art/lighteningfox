# Fuzzing Ironswift's parsers

Every page Ironswift opens is untrusted input, so the parsers are fuzzed: random and
mutated input is thrown at them, looking for panics, hangs and stack overflows.

Targets (in `fuzz_targets/`, all built from the real source files in `../src`):

| Target | What it parses |
|---|---|
| `html` | whole HTML documents (tokenizer + tree builder) |
| `html_fragment` | `innerHTML`-style fragments in 14 different context elements |
| `xhtml` | XHTML documents (the XML parser) |
| `css` | style sheets, inline declarations and selector lists |
| `javascript` | Boa's JavaScript parser, as a script and as a module |

## Coverage-guided fuzzing (nightly Rust)

```sh
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run html            # runs until stopped or a crash is found
cargo +nightly fuzz run css -- -max_total_time=600
```

Crashing inputs land in `fuzz/artifacts/<target>/`; replay one with
`cargo +nightly fuzz run <target> <file>`. The weekly `Fuzz` workflow in
`.github/workflows/fuzz.yml` runs every target for 10 minutes.

## Quick check on stable Rust

`smoke` is a small mutation fuzzer that needs no nightly toolchain. It starts from the
test pages in `../tests`, mutates them, and saves any input that panics or takes over two
seconds under `artifacts/smoke/` (the input about to run is always in `artifacts/smoke/last-<target>`,
so a stack overflow, which kills the process, can still be replayed):

```sh
cd fuzz
cargo run --release --bin smoke -- 200000      # iterations per target, optional seed
```

CI runs it on every push.

## Found so far

- Nesting overflowed the stack and crashed the whole browser. A page needed only
  ~600 nested brackets in a script (`f(f(f(…`), ~3,000 nested `[` in an array literal, or
  20,000 nested `<div>`s. Fixed: the parser stops nesting at 512 levels (as Chrome
  does), scripts nested more than 1,000 brackets deep are refused, and the browser runs on
  a 512 MB stack.
