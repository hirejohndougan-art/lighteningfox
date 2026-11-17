# Open tasks

One task per chat. This replaces claiming whole batches: the batches in `docs/BATCHES.md` that
are still running (J, K, M) keep going with the chats that hold them, and everything else that's
left is a task below.

**How to claim a task**

1. `git pull --rebase origin main`, then read this file.
2. Take the **first** task (top to bottom) whose Status is `free` and whose "After" tasks are
   all `done`.
3. Set its Status to `claimed by <your chat name> <date time>`, commit only this file, and push
   to `main`.
4. If the push is rejected, another chat pushed first: `git pull --rebase origin main`, read this
   file again, and if your task is now claimed by someone else, take the next free one.
   Never force-push.
5. When it's finished, set Status to `done`, add a **Left undone:** line if anything is, push,
   tick the checklists (see the prompt), then claim the next free task.

Status values: `free`, `claimed by …`, `done`, `blocked: <reason>`, `not for chats: <reason>`.
"Checklist" names the item(s) in the build checklist that the task closes; follow-ups close none.

---

## Checklist tasks you can start now

### T01. A real box tree
**Status:** done
**Left undone:** anonymous table objects (CSS 2 §17.2.1: stray cells/rows get no anonymous table wrapper yet); floats beside text still end an inline run instead of sitting in the anonymous block's lines (kept as layout did before); `counter-reset: list-item` scopes inside lists and a few reversed-list counter edge cases (WPT li-value-reversed-005/020/023–030).
**After:** —
**Checklist:** §5 "A real box tree (layout tree separate from the DOM)"
**Files:** `src/layout.rs` (split into new `src/boxtree.rs`), `src/paint.rs` callers
Build a box tree from the styled DOM (anonymous block/inline boxes, `display: contents`,
pseudo-elements, list markers) and lay it out instead of walking the DOM directly. Keep every
reference screenshot and the WPT scores (flexbox, floats, CSS2) the same or better. Other tasks
wait on this one, so push in stages.

### T05. Each top-level site in its own sandboxed process
**Status:** done
**Left undone:** Off by default — turn on with Settings → "Site isolation" or `IRONSWIFT_SITE_PROCESS=1`, because on pages in a site process these don't work yet: find in page, text selection/copy, the page context menu, devtools/inspect, `<select>` dropdowns (they cycle), autofill/passwords, printing, the accessibility tree, and pushState address changes. Pictures go over a pipe whole (no shared memory). Responses aren't cached and arrive whole (no streaming; a download is fetched twice). WebSockets fail (the process has no sockets and no broker for them yet). Iframes inside a site-process page are drawn in that same process, not live and not in their own site's process. Tested by hand under Xvfb: rendering, links, back, form posts, typing, scrolling, cookies/storage/fetch through the browser, a second site in its own process with none of the first site's cookies.
**After:** —
**Checklist:** §13 "Process isolation: run each top-level site in its own sandboxed process"
**Files:** `src/sandbox.rs`, new `src/process.rs`, the page-loading parts of `src/main.rs`
Cross-site iframes already run in a Landlock-sandboxed `--frame-renderer` process; extend that so
each top-level site's parsing, scripts and layout run in their own process, with the browser
process keeping the window, network, cookies and storage.

### T07. DOM objects as native Boa classes
**Status:** done
**After:** —
**Checklist:** §14 "Expose DOM objects as native Boa classes instead of JavaScript wrappers over numeric ids"
**Files:** `src/js.rs`, `src/prelude.js`, new `src/dom_classes.rs`
Measure before and after with the JavaScript benchmark page and `tools/perf_check.py`.
Left undone: setters and tree-changing methods (appendChild, setAttribute, innerHTML, …) are still JavaScript, on purpose: custom elements and MutationObserver hook the global mutation natives, so a native method would skip their reactions. The document object, shadow roots and `new Text()`-style objects are still plain JS objects (the native getters read their `__id`).

### T09. `<video>` and `<audio>` playback
**Status:** done
**Left undone:** media is downloaded whole before playing (no streaming, Range requests or Media Source Extensions, so blob:/MSE sites like YouTube don't play); no <track> captions, fullscreen or picture-in-picture; controls are always shown (no hide-on-idle, no volume slider or keyboard keys); unmuted autoplay is blocked but script play() isn't gated on user activation; canvas drawImage(video) still draws nothing; needs the ffmpeg/ffprobe programs at run time (and sandboxed frame processes must be allowed to start them).
**After:** —
**Checklist:** §7 "`<video>` and `<audio>` playback"
**Files:** new `src/media.rs`, a new JS file for HTMLMediaElement, replaced-element parts of `src/layout.rs`
GStreamer or ffmpeg bindings; play/pause/seek/volume, the `controls` UI, poster images, media
events, and the HTMLMediaElement API. Add the new system libraries to the packages in `packaging/`.

### T12. WebGL
**Status:** done
**After:** —
**Checklist:** §7 "WebGL"
**Done:** WebGL 1 and most of WebGL 2 in software (`glsl.rs`, `glsl_gen.rs`, `glsl_vm.rs` for GLSL ES 1.00/3.00; `webgl.rs`, `webgl_raster.rs`, `webgl_tex.rs`, `webgl.js`), big draws shaded on several threads. WPT `webgl/`: 85/87 subtests, 2/2 reftests. three.js 0.170 renders lit scenes. Test page `tests/webgl.html` with a reference screenshot.
**Left undone:** no antialiasing/multisampling (contexts report `antialias: false`); transform feedback, queries and syncs are stubs; no pixel pack/unpack buffers; compressed textures are S3TC only (no ETC/ASTC/BPTC/RGTC); video can't be a texture yet; `OES_draw_buffers_indexed` is a no-op; depth/stencil `blitFramebuffer` and sRGB conversion are skipped; `lineWidth` > 1 draws 1-pixel lines; the Khronos conformance suite hasn't been run (the WPT idlharness test fails on harness setup). Images made with `new Image()` that aren't in the document never load (`complete` is undefined), so three.js `TextureLoader` textures don't appear. That's a DOM image-loading follow-up.
**Files:** new `src/webgl.rs` (on wgpu or glow), a new JS file, `src/canvas.rs` only for `getContext`
WebGL 1 first (`getContext("webgl")`), then WebGL 2 if time allows. Run the WebGL conformance
tests that fit.

## Follow-ups (not checklist items; found by the batch chats)

### X1. Fix the broken and crashing sites
**Status:** claimed by chat-opus-x1 2026-11-17 04:52
**After:** —
**Checklist:** none
**Files:** whatever each fix needs; keep changes small (other chats are editing layout and JS)
`docs/SITE_COMPAT.md`: AP News crashes; MDN, Bing, BBC News, New York Times, The Guardian,
rust-lang.org and WordPress.org render badly. Start with the crash. Find the cause of each,
fix it in the engine (no site-specific hacks), and add a small test page for each cause.

### X2. Binary bodies in `fetch()`
**Status:** done
**Left undone:** `Response.body` / `Request.body` as a ReadableStream, and `Request.arrayBuffer()`/`blob()`/`text()` (Request has no body readers yet). XHR `responseText` on a binary response is decoded once up front, not lazily.
**After:** —
**Checklist:** none
**Files:** the fetch parts of `src/js.rs` / `src/prelude.js`, `src/net.rs`
`Response.blob()` and `arrayBuffer()` hand back the body as text, which corrupts images, fonts
and other binary files (most of the canvas `drawImage`-from-`fetch` WPT failures). Keep bodies as
bytes; `text()` and `json()` decode them.

### X3. Import maps
**Status:** done
**Left undone:** `integrity` entries are parsed but not checked against downloads (subresource integrity isn't enforced for any script yet); scripts inserted by JavaScript after load (including import maps) still don't run, as before.
**After:** —
**Checklist:** none
**Files:** the ES-module loader in `src/js.rs` / its JS file
`<script type="importmap">` so bare specifiers like `import "react"` resolve.

### X4. Click and animation events
**Status:** done
**Left undone:** AnimationEvent.animation (needs document.getAnimations()); animationcancel when an animation is removed; real-mouse click tests (WPT uievents/click needs test_driver input, which headless runs lack).
**After:** —
**Checklist:** none
**Files:** the event parts of `src/main.rs`, `src/prelude.js`, the animation code in `src/style.rs`
Fire `click` on release (mousedown → mouseup → click, as in other browsers) and add `dblclick`
and `contextmenu`; fire `transitionend`, `transitionstart`, `animationstart`, `animationend` and
`animationiteration`. Check that no reference screenshot or test changes.

### X5. Inline `<svg>` in pages
**Status:** done
**After:** —
**Checklist:** none
**Files:** new `src/svg_inline.rs`; reuse the SVG image renderer from batch C
SVG written directly in HTML is hidden today (SVG *images* work). Draw it, with CSS styling of
its elements where practical.
Left undone: clicks and hover hit the whole `<svg>` box, not the shape under the pointer (events
target the `<svg>` element); `<foreignObject>` content isn't drawn; an `<svg>` with no viewBox
sized differently by CSS is scaled rather than clipped; CSS animations/transitions on SVG
properties (fill, stroke) aren't animated; selectors that only match through SVG mixed-case
tag names (`linearGradient`) may not match.

### X6. Geolocation
**Status:** done
**After:** —
**Checklist:** none
**Files:** new `src/geo.rs`, the geolocation part of the JS APIs
Allowed requests report "position unavailable" today. Ask GeoClue over D-Bus for a position.
**Left undone:** the Flatpak build can't reach GeoClue (it's on the system bus, which the sandbox
blocks); it needs the XDG Location portal instead. `altitudeAccuracy` is always null (GeoClue
doesn't report it). When T05 moves pages into sandboxed processes, `src/geo.rs` must stay in the
browser process and pass positions to the page process.

## Review and cleanup of the overnight work

### R1. Re-run the official test suites
**Status:** claimed by main-chat 2026-11-17 11:10
**After:** —
**Checklist:** none
**Files:** whatever a regression needs
Run `tools/wpt.py` on dom/nodes, shadow-dom, css/selectors, IndexedDB, css/css-flexbox and
css/CSS2/floats; compare with `docs/CONFORMANCE.md` and fix anything that got worse overnight.

### R2. Site check: compare with last night
**Status:** claimed by main-chat 2026-11-17 11:10
**After:** —
**Checklist:** none
**Files:** whatever a regression needs
Compare the new `docs/SITE_COMPAT.md` run with last night's (16 work, 16 usable, 7 broken,
1 crash) and fix any site that got worse. (X1 keeps fixing the sites that were already broken.)

### R3. Clean up the compiler warnings
**Status:** claimed by chat-opus-geo 2026-11-17 11:35
**After:** —
**Checklist:** none
**Files:** any, but only to remove dead code or fix warnings; no behavior changes
`cargo build --release` prints 128 warnings, mostly unused code left over from the parallel
work. Remove what's truly unused, fix the rest, and make CI fail on new warnings once it's clean
(`RUSTFLAGS=-D warnings` in the build step).

### R4. Review the box tree and incremental relayout
**Status:** claimed by chat-opus-x5 2026-11-17 11:45
**After:** —
**Checklist:** none
**Files:** `src/boxtree.rs`, `src/layout.rs`
The largest overnight change. Read it for correctness (stale boxes after DOM edits, relayout
that misses a change, anonymous boxes, `display: contents`), add tests for each bug found, and
fix them. Check memory use and speed on a large page against the version before T01.

### R5. Security review of the per-site processes
**Status:** claimed by chat-opus-t12 2026-11-17 12:05
**After:** —
**Checklist:** none
**Files:** `src/process.rs`, `src/sandbox.rs`, the IPC code
Check what a compromised site process can ask the browser process to do (reading other sites'
cookies or storage, opening files, network requests as another origin), that the seccomp and
Landlock rules still hold in each process, and that a crashing site process can't take the
browser down. Write tests for each rule.

## Checklist tasks that wait on others

### T02. Incremental relayout
**Status:** done
**Left undone:** the box tree and fingerprints are still rebuilt in full each pass (cheap: ~0.1 ms on Bootstrap); boxes holding absolutely positioned, fixed or z-index descendants that escape them, content inside multi-column boxes, and min/max-content measuring passes are always laid out afresh.
**After:** T01
**Checklist:** §5 "Incremental relayout (only redo what changed)"
**Files:** `src/boxtree.rs`, `src/layout.rs`

### T03. Vertical text for East Asian languages
**Status:** done
**After:** T01
**Checklist:** §6 "Vertical text for East Asian layouts"
**Files:** `src/boxtree.rs`, text parts of `src/layout.rs`, `src/font.rs`
`writing-mode` (vertical-rl/lr), `text-orientation`, `text-combine-upright`.
Left undone: logical properties (inline-size, margin-block…) still map as in horizontal text; body→root writing-mode propagation; a descendant's text-orientation applies only to its own text runs (letter-spaced text uses the root's); padding of a horizontal box inside vertical text lands on the turned sides; link/hit areas inside such a box are turned with the column; sideways glyph placement is centred approximately (WPT orientation-font ref tests still differ by a few pixels).

### T04. Parallel styling and layout
**Status:** done
**After:** T01 and batch K done (`docs/BATCHES.md`)
**Checklist:** §14 "Parallel styling and layout across CPU cores (Rayon)"
**Files:** `src/style.rs`, `src/cascade.rs`, `src/boxtree.rs`
Styles use `Rc` and per-thread state (hover, viewport, animations) today; see batch L's notes.
**Left undone:** layout is still on one thread; that half is now T04b below. Styling: rules
inside `@container` are matched in order, not in parallel (they need ancestors' styles).

### T04b. Parallel layout
**Status:** done
**After:** T04
**Checklist:** §14 "Parallel styling and layout across CPU cores (Rayon)" (the layout half; tick
the item when this is done)
**Files:** `src/font.rs`, `src/layout.rs`, new files for the parallel parts
On text-heavy pages layout is most of the time (1500 paragraphs: layout ~200 ms, styling ~4 ms;
see `src/parallel.rs` for how styling was split). In a profile of that page, layout spends about
a third of its time shaping words (`Fonts::shape`) and another third in `word_width` and
`Fonts::resolve` (hashing the font-family string for every word). `Fonts` keeps faces and
caches in `Rc`/`RefCell` shared with painting, so shaping can't run on other threads until
those become thread-safe (or each thread gets its own shaping state). Measure before and after;
keep every reference screenshot identical.
**Left undone:** words are shaped across cores before layout (src/preshape.rs), but boxes are
still laid out on one thread: laying out independent blocks at once needs `layout::Ctx` and
`Fonts` (Rc/RefCell caches shared with painting) made thread-safe, and T02 is reworking
layout.rs now. On 2 cores the parallel shaping saves only a few percent (1500 paragraphs: layout
173 → ~137 ms, mostly from two single-thread fixes in font.rs); it saves more with more cores.
Painting shapes whole lines again (`paint.rs`), which this pass can't do ahead of layout.

### T06. Shared process pool
**Status:** done
**Left undone:** A shared process can be asked for data of every site it holds, so sharing weakens isolation between those sites (as in Firefox); sites aren't moved back to their own process when memory frees up. The cap and the 512 MiB free-memory floor are fixed rules, not tuned on a real small laptop.
**After:** T05
**Checklist:** §14 "Shared process pool (like Firefox) instead of one process per site"
**Files:** `src/process.rs`
Cap the number of site processes on small laptops and reuse them, keeping sites from different
origins apart when memory allows.

### T08. Bindings generated from WebIDL
**Status:** done
**Left undone:** 64 members (Node, Element, CharacterData, Text, Document) go through the generated bindings; the rest of each interface (everything that changes the tree or attributes, on purpose, plus Document's long tail) is still JavaScript in prelude.js, and `--report` lists them. The generator doesn't yet handle union, dictionary, callback or variadic arguments, overloads, setters, or non-node interfaces (events, fetch, canvas…), so those APIs don't get generated glue yet.
**After:** T07
**Checklist:** §9 "Generate bindings automatically from the WebIDL spec files" and §14 "Generate
bindings from WebIDL so every API gets the fast path" (two items)
**Files:** new `tools/webidl/`, generated `src/bindings/`

### T10. Media Source Extensions
**Status:** done
**After:** T09
**Checklist:** §7 "Media Source Extensions (needed for YouTube and most streaming)"
**Files:** `src/media.rs`
Left undone: coded frames are handled per segment, not per frame, so `remove()` drops whole
segments, `appendWindowStart/End` don't trim, and an overlapping append replaces whole segments;
`mode = "sequence"` is accepted but timestamps aren't regenerated; MPEG audio (`audio/mpeg`,
`audio/aac`) byte streams aren't supported; `audioTracks`/`videoTracks` are empty;
`srcObject = mediaSource`, MediaSource in workers (`MediaSourceHandle`) and ManagedMediaSource
are missing. WPT media-source: 194/313 subtests; most files that need playback to run give no
result under `tools/wpt.py`, whose snapshot mode freezes media. Several WPT helpers also need
`String.prototype.substr`, which the engine lacks (Boa's `annex-b` feature is off).

### T11. Fingerprinting noise on audio
**Status:** done
**Left undone:** There was no Web Audio to add noise to, so this built it (`src/webaudio.js`) and noised its readouts (rendered buffers, all four analyser reads). Web Audio here is one channel inside (stereo outputs repeat it), makes no sound through the speakers, and has no `decodeAudioData` (rejects with EncodingError), AudioWorklet, panning, convolution or ScriptProcessor callbacks (those nodes pass sound through unchanged). Its numbers are close to but not the same as Chrome's or Firefox's (simpler compressor, plain waveforms instead of band-limited tables).
**After:** T09
**Checklist:** §13 "Fingerprinting: add the per-site noise to audio readouts once audio exists"
**Files:** `src/media.rs` (Web Audio readouts), the existing noise function

### T13. Extensions (WebExtensions API)
**Status:** done
**Left undone:** content scripts share the page's JavaScript globals (no separate isolated world; `chrome` itself stays hidden from the page); tab ids are tab positions + 1, so they shift when an earlier tab closes; webRequest only has `onBeforeRequest` (cancel/redirect) and guesses the resource `type` from the request; no `declarativeNetRequest`, `runtime.connect` ports, `alarms`, `contextMenus` or toolbar buttons; no about:extensions page or installer (extensions load unpacked from `<profile>/extensions/` or `IRONSWIFT_EXTENSIONS`); a site process's content-script requests are only checked against the extension's match patterns and the process's sites.
**After:** T01, T05, T07
**Checklist:** §11 "Extensions (WebExtensions API)"
**Files:** new `src/extensions/`
Start with content scripts, `storage`, `tabs` and `webRequest` blocking, enough to run a simple
ad blocker or dark-mode extension.

## Not for the chats

### DRM (Widevine)
**Status:** not for chats: needs a license agreement with Google; John decides.
**Checklist:** §7 "DRM (Widevine) for Netflix and similar"

### Baseline JIT and the full JIT for Boa
**Status:** not for chats: only after the whole app is built (John's decision).
**Checklist:** §14 "Baseline JIT compiler (via Cranelift)" and "Optional … a JIT compiler for Boa"

### Funding plan; live-site check on John's computer
**Status:** not for chats: John's to do.
**Checklist:** §16 "Funding plan", §1 "Live-site check"

## Still with their batch chats (see `docs/BATCHES.md`)

- Batch J: §16 Windows and macOS builds
- Batch K: §4 shorthand expansion; §5 grid extras; §5 exact `height` on overflowing boxes
- Batch M: §14 Boa bytecode compiler optimizations
