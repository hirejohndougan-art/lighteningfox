# Conformance

How closely Ironswift follows the web standards, measured with the official test suites. These
are a baseline to measure progress against, not a finish line: each fix should move a number.

## Results (16 November 2026)

| Suite | What it checks | Result |
|---|---|---|
| **Test262** | The JavaScript language (Boa 0.22, our engine) | **51,225 of 53,578 passed (95.6%)**; 572 failed, 1,781 skipped as features Boa hasn't built |
| **HTML parsing** (WPT `html/syntax/parsing`, the html5lib tree-construction tests) | The exact tree built from tricky or broken markup | 1,972 of 1,972 (100%), including the tests that run scripts during parsing |
| **HTML tokenizing** (html5lib-tests `tokenizer/*.test`) | Tags, attributes, comments, doctypes, character references, script-data escapes, processing instructions | 7,046 of 7,046 (100%) |
| **DOM** (WPT `dom/nodes`) | `Node`, `Element`, `Document` and friends | 1,535 of 5,629 subtests (27.3%) |
| **Shadow DOM** (WPT `shadow-dom`) | attachShadow, slots, scoped styles, events across shadow trees | 245 of 402 script subtests (60.9%); 11 of 18 reference pages |
| **IndexedDB** (WPT `IndexedDB`) | Databases, stores, indexes, cursors, transactions | 1,098 of 1,284 subtests (85.5%) |
| **CSS selectors** (WPT `css/selectors`) | Selector matching | 450 of 816 script subtests; 39 of 135 reference pages |
| **Flexbox** (WPT `css/css-flexbox`) | Flex layout | 291 of 894 reference pages; 79 of 398 script subtests |
| **Floats** (WPT `css/CSS2/floats`) | Float layout | 19 of 78 reference pages; 19 of 51 script subtests |
| **WebGL 2** (Khronos `conformance2`, without `textures/`; 18 November) | WebGL 2 API, GLSL ES 3.00, rendering | 173 of 258 pages fully pass; 23,898 of 24,375 checks (98.0%) |
| **WebGL 2 textures** (Khronos `conformance2/textures`; 18 November) | Uploads from arrays, images, canvases, video, ImageBitmap; formats | 861 of 926 pages fully pass |
| **WebAssembly JS API** (WPT `wasm/jsapi`, standard parts; 18 November) | `Module`, `Instance`, `Memory`, `Table`, `Global`, `validate`/`compile`/`instantiate`, errors | 887 of 929 subtests (95.5%): constructor 347/347, instance 142/142, table 91/95, global 132/145, module 46/49, memory 52/72, top level 77/79 |
| **WebAssembly Web API** (WPT `wasm/webapi`; 18 November) | `compileStreaming`/`instantiateStreaming`, MIME and CORS checks, `Response` bodies | 248 of 273 subtests (90.8%) |
| **WebAssembly spec tests** (WebAssembly/spec `test/core`, through the JS API with `tools/wasm_spec.py`; 18 November) | The core language: instructions, validation, linking, traps | 17,087 of 17,680 steps (96.6%); proposal folders: bulk-memory 7,482/7,485, memory64 8,655/8,659, multi-memory 911/912, simd 1,403/1,403 and relaxed-simd 8/8 (both mostly skipped: v128 can't cross to script), exceptions 18/92, gc 80/769 |

Reference tests pass only when the test page and its reference page render to *identical*
pixels at 800×600. Official WPT allows small pixel differences on some tests, so these numbers
are slightly stricter than wpt.fyi's.

WebAssembly runs on wasmi, which has no function references, GC, exception handling or
threads: every remaining spec-test failure in `test/core` is a module using function
references or GC types (`br_table`, `return_call*`, `call_ref`, `ref_*`, `type-*`, typed
tables), and they also account for `exceptions`/`gc` and WPT's `jsapi/exception`, `gc`,
`tag`, `jspi` and `js-string`. WPT's own copy of the spec suite (`wasm/core`) checks results
with wrapper modules that need GC, so it isn't the measure here (19.3% for that reason). Other
known gaps: shared memory (`memory/*shared*`), `toResizableBuffer`/`toFixedLengthBuffer`, the
tentative type reflection (`*.type()`, `WebAssembly.Function`), the tentative `.wasm` ES module
integration (`esm-integration`, 9 of 61), `ReadableStream` bodies and the tests that need WPT's
own server (`?pipe=` headers, a second origin for CORS).

What the numbers point to, in order of payoff:

1. **The full HTML5 parsing algorithm** (checklist section 3), including comment and doctype
   nodes: most HTML-parsing failures and many DOM failures come from the simplified parser.
2. **Missing DOM pieces**: `createDocument`/`createDocumentType`, comment and doctype nodes,
   namespaces, `Range`. Many DOM test files stop in their setup step because of these.
3. **CSS**: `::before`/`::after`, `opacity`, and the remaining flexbox details.

## Running the suites

```sh
# HTML parsing: point at a WPT checkout's .dat files
cargo run --release -- --conformance-html5lib /path/to/wpt/html/syntax/parsing/resources
cargo run --release -- --conformance-tokenizer /path/to/html5lib-tests/tokenizer

# DOM and CSS: script tests and reference tests from a WPT checkout (needs python3 with Pillow)
python3 tools/wpt.py /path/to/wpt dom/nodes shadow-dom IndexedDB css/selectors css/CSS2/floats css/css-flexbox

# WebAssembly: the JS and Web API tests, then the spec tests (.wast) through the JS API
python3 tools/wpt.py /path/to/wpt wasm/jsapi wasm/jsapi/constructor wasm/jsapi/global wasm/jsapi/instance wasm/jsapi/memory wasm/jsapi/module wasm/jsapi/table wasm/webapi
python3 tools/wasm_spec.py /path/to/wpt /path/to/spec/test/core   # git clone --depth 1 https://github.com/WebAssembly/spec

# JavaScript: Test262 against our Boa version (downloads ~400 MB into ./conformance)
sh tools/test262.sh
```

A WPT checkout of just the needed folders:

```sh
git clone --depth 1 --filter=blob:none --sparse https://github.com/web-platform-tests/wpt
cd wpt && git sparse-checkout set resources common fonts interfaces wasm IndexedDB html/syntax/parsing dom/nodes shadow-dom css/css-flexbox css/CSS2/floats css/selectors
```

WebGL: the Khronos suite from a sparse checkout of
[KhronosGroup/WebGL](https://github.com/KhronosGroup/WebGL) (`sdk/tests/js`, `resources`,
`conformance2`, and `conformance/extensions` for the compressed-texture pages):

```sh
python3 tools/khronos_webgl.py /path/to/WebGL conformance2/rendering conformance2/textures/misc
# KHR_DETAILS=out.json keeps every page's failures; KHR_JOBS and KHR_TIMEOUT tune the run
```

The runner's profiles turn anti-fingerprinting off, since its noise on `readPixels` fails the
exact-color checks.

`tools/wpt.py` swaps the checkout's `resources/testharnessreport.js` for ours (the original is
kept as `.orig`), serves the files on localhost and runs the browser headless on each test.
