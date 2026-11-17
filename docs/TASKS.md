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
**Status:** claimed by chat-opus-t05 2026-11-17 04:55
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
**Status:** claimed by chat-opus-t12 2026-11-17 04:55
**After:** —
**Checklist:** §7 "WebGL"
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
**Status:** claimed by chat-opus-x2 2026-11-17 05:00
**After:** —
**Checklist:** none
**Files:** the fetch parts of `src/js.rs` / `src/prelude.js`, `src/net.rs`
`Response.blob()` and `arrayBuffer()` hand back the body as text, which corrupts images, fonts
and other binary files (most of the canvas `drawImage`-from-`fetch` WPT failures). Keep bodies as
bytes; `text()` and `json()` decode them.

### X3. Import maps
**Status:** claimed by chat-opus-x3 2026-11-17 05:01
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
**Status:** claimed by chat-opus-x5 2026-11-17 04:52
**After:** —
**Checklist:** none
**Files:** new `src/svg_inline.rs`; reuse the SVG image renderer from batch C
SVG written directly in HTML is hidden today (SVG *images* work). Draw it, with CSS styling of
its elements where practical.

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

## Checklist tasks that wait on others

### T02. Incremental relayout
**Status:** claimed by chat-opus-a 2026-11-17 10:40
**After:** T01
**Checklist:** §5 "Incremental relayout (only redo what changed)"
**Files:** `src/boxtree.rs`, `src/layout.rs`

### T03. Vertical text for East Asian languages
**Status:** claimed by chat-opus-t07 2026-11-17 10:30
**After:** T01
**Checklist:** §6 "Vertical text for East Asian layouts"
**Files:** `src/boxtree.rs`, text parts of `src/layout.rs`, `src/font.rs`
`writing-mode` (vertical-rl/lr), `text-orientation`, `text-combine-upright`.

### T04. Parallel styling and layout
**Status:** free
**After:** T01 and batch K done (`docs/BATCHES.md`)
**Checklist:** §14 "Parallel styling and layout across CPU cores (Rayon)"
**Files:** `src/style.rs`, `src/cascade.rs`, `src/boxtree.rs`
Styles use `Rc` and per-thread state (hover, viewport, animations) today; see batch L's notes.

### T06. Shared process pool
**Status:** free
**After:** T05
**Checklist:** §14 "Shared process pool (like Firefox) instead of one process per site"
**Files:** `src/process.rs`
Cap the number of site processes on small laptops and reuse them, keeping sites from different
origins apart when memory allows.

### T08. Bindings generated from WebIDL
**Status:** free
**After:** T07
**Checklist:** §9 "Generate bindings automatically from the WebIDL spec files" and §14 "Generate
bindings from WebIDL so every API gets the fast path" (two items)
**Files:** new `tools/webidl/`, generated `src/bindings/`

### T10. Media Source Extensions
**Status:** free
**After:** T09
**Checklist:** §7 "Media Source Extensions (needed for YouTube and most streaming)"
**Files:** `src/media.rs`

### T11. Fingerprinting noise on audio
**Status:** free
**After:** T09
**Checklist:** §13 "Fingerprinting: add the per-site noise to audio readouts once audio exists"
**Files:** `src/media.rs` (Web Audio readouts), the existing noise function

### T13. Extensions (WebExtensions API)
**Status:** free
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
