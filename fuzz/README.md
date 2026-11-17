# Fuzzing

Fuzz targets for everything that reads text from the web:

| Target | What it feeds random input to |
|---|---|
| `html` | the HTML tokenizer and tree builder (also the XHTML path and script pauses) |
| `css` | the CSS parser, selector parser and URL rewriting |
| `js` | the Boa JavaScript engine, with the browser's loop and recursion limits |
| `csp` | Content Security Policy parsing and source matching (`security.rs`) |
| `filters` | EasyList-syntax filter parsing and URL matching (`blocklist.rs`) |

Install [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (`cargo install cargo-fuzz`).
It needs a nightly compiler; on stable, `RUSTC_BOOTSTRAP=1` works:

```sh
cd fuzz
RUSTC_BOOTSTRAP=1 cargo fuzz run -O html corpus/html seeds/html -- -max_total_time=600
```

The first folder collects new inputs (it's ignored by git); `seeds/` holds the starting
inputs. A crash is saved under `artifacts/<target>/`; reproduce it with
`cargo fuzz run html artifacts/html/crash-…`, fix it, and add the input to `seeds/` so it
stays fixed. CI runs every target for a few minutes each week (`.github/workflows/fuzz.yml`).
