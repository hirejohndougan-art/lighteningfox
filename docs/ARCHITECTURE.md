# Ironswift architecture

Ironswift is one Rust program (about 15,000 lines) that does every stage of showing a web page
itself. External crates only handle plumbing: a window (`minifb`), HTTP and TLS (`ureq`/`rustls`),
image and font decoding (`image`, `ab_glyph`, `wuff`), text shaping (`rustybuzz`), the GPU
(`wgpu`) and the JavaScript language (`boa_engine`).

## The pipeline

```
 URL ─► net.rs ─► html.rs ─► dom.rs ─► css.rs + style.rs ─► layout.rs ─► paint.rs ─► tiles.rs ─► gpu.rs / window
         fetch     parse      tree      rules → computed     boxes and     pixels      cached      on screen
                                        style per node       lines →                   strips
                                                             display list
                   js.rs + prelude.js: scripts read and change the DOM; changes trigger restyle → layout
```

1. **Fetch** (`net.rs`, `cookies.rs`). `fetch_page` in `main.rs` runs on a worker thread so the window
   never freezes. It downloads the page, its stylesheets, scripts, images and web fonts in parallel,
   and handles redirects, cookies (third-party ones blocked), CORS and tracking-tag stripping.
   `about:` pages (`about:home`, `about:settings`, `about:history`, `about:bookmarks`, `about:log`,
   `about:crashes`) are generated here too.
2. **Parse HTML** (`html.rs`) into the **DOM** (`dom.rs`): an arena (`Vec<Node>`) where nodes refer to
   each other by index (`NodeId`). Index 0 is the document.
3. **Parse CSS** (`css.rs`) into rules: selectors plus declarations, `@media`, `@font-face`.
4. **Style** (`style.rs`): match selectors against every element, run the cascade (origin,
   specificity, `!important`, inline styles, inheritance) and turn the winning values into a
   `Style` struct per node. Identical siblings share one `Rc<Style>`.
5. **Layout** (`layout.rs`, with `grid.rs`, `transform.rs`): walk the styled tree and position every
   box and line of text: block, inline, flexbox, grid, floats, tables, positioned and sticky boxes.
   The output is a `Layout`: a **display list** (`Item::Rect`, `Item::Text`, `Item::Image`, clips,
   transforms…) plus hit-testing data (links, element boxes, scroll areas).
   Layout works on the **box tree** (`boxtree.rs`), not the DOM: `display: contents` unwrapped,
   `::before`/`::after` and list markers added, inline runs beside blocks wrapped in anonymous
   block boxes. Each box has a subtree fingerprint, and a block box whose fingerprint and inputs
   match the last pass is put back moved instead of laid out again (`relayout.rs`; set
   `IRONSWIFT_NO_RELAYOUT_CACHE=1` to turn that off when chasing a layout bug).
6. **Paint** (`paint.rs`, `font.rs`): draw the display list into a pixel buffer (`Canvas`, 0RGB `u32`s).
   Text is shaped (`rustybuzz`), glyphs come from system fonts, web fonts or fallback fonts, and
   are cached.
7. **Present** (`tiles.rs`, `gpu.rs`): the page is painted once into 256-pixel strips that scrolling
   reuses. With a graphics card, the strips become textures and each frame is a few rectangles;
   without one, the strips are copied into the window buffer.

## JavaScript

`js.rs` embeds Boa and exposes about 40 small native functions that work on node ids
(`__children`, `__insert`, `__query`, `__setAttr`…). `prelude.js` builds the web platform on top
of them in JavaScript: `document`, `Element` and its subclasses, events with bubbling, timers and
`requestAnimationFrame`, `fetch` and `XMLHttpRequest`, `history`, `localStorage`, forms and more.

- A page's scripts run in order; the event loop (`Js::tick`) runs one task at a time with a
  microtask checkpoint after each, and animation-frame callbacks once per frame.
- A script that changes the DOM sets a dirty flag (`js::with_dom_mut`); the window then restyles
  and lays the page out again before the next frame.
- Scripts that ask where things are (`getBoundingClientRect`, `offsetWidth`) get a fresh layout
  through `geometry.rs`.

## WebAssembly (`wasm/`, `wasm.js`)

The `WebAssembly` global is built in `wasm.js` on a few natives in `wasm/mod.rs`, which run
modules with the **wasmi** interpreter. Each page's script engine has one wasmi store holding
all its instances (so instances can import each other's functions); nothing in it is freed
before the page goes away. Calls nest freely (script → wasm → script import → wasm): while an
import runs, its wasm `Caller` stands in for the store. Exceptions thrown by imports come out
of wasm unchanged; traps become `WebAssembly.RuntimeError`, a stack overflow a `RangeError`.
Runaway modules are stopped by wasmi fuel (`wasm::FUEL`, about 20 seconds of work per call
from script), with the same uncatchable error as Boa's loop limit.

Loading (`wasm_loading.js`, `wasm/share.rs`, `wasm/esm.rs`, `wasm/csp.rs`):
`compileStreaming`/`instantiateStreaming` check the `Response` (`application/wasm`, ok, not
opaque, unread) and compile once the whole body is in (fetch bodies arrive whole). A `.wasm` ES
module (served as `application/wasm`) is compiled by the module loader and stands behind a
generated JavaScript module that imports what the wasm imports and exports its exports. A page
whose CSP limits scripts may only compile from bytes with `'wasm-unsafe-eval'` or
`'unsafe-eval'` (else `CompileError`). A `Module` sent with `postMessage` travels as its bytes
(shared, not copied) and is compiled again by the receiver: wasmi never frees a module's code
before its engine goes, so each page and worker keeps its own engine.

**Why wasmi and not wasmtime** (measured 2026-11-18 on a 2-core x86-64 VM, release builds,
compressing 8 MB of text with zstd 1.5 compiled by Emscripten (`@bokuweb/zstd-wasm`, 246 KB of
wasm), level 3):

| | wasmi 2.0 | wasmtime 50 (Cranelift) |
|---|---|---|
| Compile the module | 2 ms (validated; functions translated on first call) | 300 ms |
| Instantiate | 0.07 ms | 0.08 ms |
| Compress 8 MB | 450 ms (470 ms with fuel metering on) | 72 ms |
| Decompress | 115 ms | 14 ms |
| Added to a stripped binary | +2.5 MB | +11.3 MB |
| Peak memory (whole run) | 46 MB | 52 MB |

wasmtime runs 6–8 times faster, but compiles everything up front: about 1.2 ms per KB of
wasm here, so a 10–30 MB app (Figma, Unity games) would wait 10–40 s before its first call,
where wasmi starts at once. wasmi also needs no executable memory (nothing for the sandbox
or a JIT security review to allow), adds a quarter of the size, builds faster, and its fuel
gives an exact runaway limit; wasmtime would need epoch interruption from another thread.
The runtime is only used inside `wasm/`, so moving hot modules to a compiler later (wasmtime,
or Cranelift once the JIT tasks bring it in) stays a local change.

## Security (`security.rs`, `blocklist.rs`, `sandbox.rs`, `permissions.rs`)

- **Every page has a policy** (`security::PagePolicy`): its CSP (headers and `<meta>`),
  `X-Frame-Options`, and the sandbox of the frame it's in. `prepare` registers it by page
  URL before fetching anything, and `net::fetch_sub*` checks each request against it (with
  the request's purpose, `security::Dest`) and against the filter lists (`blocklist`).
- **Content blocking** (`blocklist.rs`): network rules are indexed by host, by a word the
  URL must contain, and by the page's domain; a blocked request can be answered with a
  stand-in from `redirects.js`. Before a page's first script, `Js::page_start` runs the
  scriptlets its filters name (`scriptlets.js`) and the procedural cosmetic filter runner
  (`cosmetic.js`); plain element hiding goes into the page's style sheets and is rebuilt as
  the page's classes and ids change. HTML filters (`##^…`) remove elements from the page's
  HTML in `prepare` before it's parsed for scripts; inline scripts pass through
  `blocklist::filter_inline_script` (`##^script:has-text()`, `rpnt`) before they run.
- **Scripts act for their page only.** The script engine holds a `PageCtx` (URL, top-level
  page, sandbox, policy) where scripts can't reach it; the natives for cookies, storage,
  network requests and history use it rather than any URL a script passes. A Boa host hook
  refuses `eval`/`new Function` unless the CSP allows it (approved inline handlers excepted).
- **Frames** carry a `FrameInfo` (top-level page, sandbox) while they load, so their cookies
  go in the top-level site's partition. Cross-site frames are drawn by a separate process,
  `ironswift --frame-renderer`, talking over stdin/stdout (`isolated` in `main.rs`) and
  locked down by `sandbox::protect_frame_renderer`. The browser itself calls
  `sandbox::protect_browser` once its window is open.
- **Site processes** (`process.rs`, the "Site isolation" setting, on by default; `site_isolation` in settings.tsv):
  each top-level site's pages run in their own `ironswift --site-renderer` process, locked
  down by `sandbox::protect_site_renderer` (no programs, no network, no writing; the browser
  makes requests, opens WebSockets (`ws_relay.rs`) and keeps cookies for it). It reads the
  installed-font list the browser keeps in the profile (`font-index.tsv`), since it can't run
  fontconfig. Each process costs about 50-70 MB (its own script engine and fonts) on top of
  the browser; the pool is capped by the machine's memory (`process::limit`). With
  `IRONSWIFT_SITE_PROCESS=1`, `--png` of an http(s) page takes the picture from a site
  process (`site_snapshot.rs`), and `tools/site_process_check.py` compares every screenshot
  test page drawn both ways. Video and audio decoders are started by the browser for it
  (`media_broker.rs`). The window does find in page, selection, menus, autofill, printing,
  devtools and the accessibility tree from what the process sends (`frames.rs` updates);
  pictures go through shared memory (`shm.rs`); big responses are streamed, and plain GETs
  go through the browser's disk cache.
- **Navigation** from a page (links, forms, scripts, `window.open`) goes through
  `security::page_may_navigate`, which keeps web pages away from `file:`, `about:`,
  `view-source:` and `data:` URLs. The address bar isn't limited.

## The window (`main.rs`)

`run_window` owns everything on screen and loops at up to 60 frames a second (12 when idle):

- **Tabs**: the visible tab's state lives in local variables; background tabs are `Slot`s holding a
  `Saved` copy (page, DOM, scroll, history). Tabs not seen for a while are put to sleep (their page
  is freed and reloads when shown). The open tabs are saved every second for session restore.
- **Input**: keys go through `edit_keys` for the address bar and form fields (`textedit.rs`);
  clicks are hit-tested against the layout and dispatched to the page's JavaScript first.
- **Navigation**: `Go::To/Back/Forward/Reload`, same-document moves (`#fragment`, `pushState`) and
  form submission (`forms.rs`).

## Storage (all in the profile folder, `~/.local/share/ironswift`)

| File | What |
|---|---|
| `cookies.tsv` | Cookies with an expiry date |
| `storage/` | `localStorage` per site |
| `history.tsv`, `bookmarks.tsv` | History and bookmarks (`library.rs`) |
| `settings.tsv` | Settings (`settings.rs`, edited on about:settings) |
| `session.txt` | Open tabs, for session restore |
| `crashes/` | Crash reports (`crashlog.rs`) |
| `filters/` | Filter lists (EasyList, EasyPrivacy, the uBlock lists, Peter Lowe's list, malware and phishing hosts, cookie-notice lists when turned on), your own rules in `custom.txt` / `custom-dangerous.txt` |
| `permissions.tsv` | Sites' camera, microphone, location and notification permissions |
| `update/` | A downloaded, verified update waiting to be installed on exit |

Files are written privately (mode 600) and atomically (write a temporary file, then rename).

## Developer switches (environment variables)

| Variable | Effect |
|---|---|
| `IRONSWIFT_PROFILE=dir` | Use another profile folder (tests use a temporary one) |
| `IRONSWIFT_PRIVATE=1` | Don't record history |
| `IRONSWIFT_GPU=0` / `=1` | Turn GPU drawing off / allow software GPU adapters |
| `IRONSWIFT_TILES=0` | Paint the whole page every frame (no tile cache) |
| `IRONSWIFT_FRAME_STATS=1` | Print drawing times every 60 frames |
| `IRONSWIFT_TAB_SLEEP_SECONDS=n` | Put background tabs to sleep after n seconds |
| `IRONSWIFT_THIRD_PARTY_COOKIES=1`, `IRONSWIFT_KEEP_TRACKING_PARAMS=1` | Relax the privacy defaults |
| `IRONSWIFT_MEM_PROBE=1` | Print memory use after each startup stage |
| `IRONSWIFT_NO_SANDBOX=1` | Don't install the system-call filter or Landlock rules (debugging) |
| `IRONSWIFT_FRAME_PROCESS=0` | Draw cross-site iframes in the browser process |
| `IRONSWIFT_SITE_PROCESS=1` / `=0` | Run each site's pages in its own process (overrides the setting); with `--png`, snapshot through one |
| `IRONSWIFT_WAIT_FOR=expr` | `--png`: play media and animations in real time until the script expression is true (`tools/wpt.py`) |
| `IRONSWIFT_NO_UPDATES=1` | No update checks and no filter-list downloads |

## Command line

```sh
ironswift [url]                              # open a window
ironswift --png out.png [--width 1024] url   # render a page to a PNG and exit (used by the tests)
ironswift --conformance-html5lib dir         # run the HTML-parsing conformance tests
ironswift --frame-renderer                   # (internal) the sandboxed process that draws cross-site iframes
```
