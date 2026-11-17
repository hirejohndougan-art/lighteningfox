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

### X7. Turn on Boa's Annex B (legacy web JavaScript)
**Status:** done
**After:** —
**Checklist:** none
**Files:** `Cargo.toml` (the `annex-b` feature of `boa_engine`)
`"abc".substr()`, `escape()`, `trimLeft()`, `<!--` comments in scripts and other web-required
legacy JavaScript are missing because Boa is built without `annex-b`. Turn it on; compare the
test suite, test262 and the media-source WPT before and after; fix or report regressions.
X1 turned the feature on at the same time (c1ce921); X7 measured it. Before → after: all 260
tests pass; WPT dom/nodes 1535/5629, dom/events 239/526, shadow-dom 245/402, IndexedDB
1098/1284 unchanged; media-source 207/325 → 215/339 (its helpers use `substr`); V8 suite score
259 → 255–256 (about 1%; Crypto about 6% slower, the rest within noise).

### X8. Review the small changes X5 and T10 made outside their files
**Status:** done
**After:** —
**Checklist:** none
**Files:** `src/boxtree.rs`, `src/js.rs`, `src/prelude.js`, `src/font.rs`, `tools/wpt.py`
The `<svg>` replaced-element entry in the box tree, `createElementNS` marking SVG/MathML
elements, the `document.body` setter, `font::web_families_generation`, and `WPT_TIMEOUT`:
check each against the spec and other browsers, add tests where missing.
Reviewed: the `<svg>` entry in `boxtree.rs` and `font::web_families_generation` are covered by
the relayout tests; the `document.body` setter follows the HTML steps and now has a test
(`jsapi_tests.rs`); `WPT_TIMEOUT` is documented in `tools/wpt.py`. Found and fixed: `tagName`
uppercased SVG and MathML elements (`LINEARGRADIENT`). `createElementNS` is right for SVG and
MathML; other namespaces and name checks are X16.

### X12. Images made with `new Image()` load without being in the page
**Status:** claimed by chat-opus-t12 2026-11-17 12:35
**After:** —
**Checklist:** none
**Files:** the image-loading code (`src/page_images.rs`, `src/images.rs`), `src/prelude.js` (the `Image` factory only)
An `<img>` created by script (`new Image()`, `document.createElement("img")`) and never added to
the document never loads: `complete` is undefined and `load`/`error` never fire. three.js's
`TextureLoader` and most WebGL texture code rely on this, so WebGL pages show no textures.
Load such images when their `src` is set, fire `load`/`error`, and give `complete`,
`naturalWidth`/`naturalHeight` and `decode()`. WPT `webgl/texImage2D-image-not-loaded.html` and
the `html/semantics/embedded-content/the-img-element` tests that apply.

### X13. WebSockets in site processes
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/websocket.rs`, the request parts of `src/process.rs`
Site processes have no network, so `new WebSocket()` fails in them (chat widgets, live
dashboards). Have the browser process open the socket for the page's process and relay frames
both ways over the existing request channel, with cookies attached in the browser (as `http`
requests do since R5: see `COOKIE_TOKEN`). Also use it from the frame process, then take network
access away from the frame renderer (`sandbox::protect_frame_renderer`, `no_net`). Cap open
sockets per process. Test with `tools/ws_echo.py`.

### X14. One process per site for cross-site iframes
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/frames.rs` (`isolated`), `src/process.rs` (the pool)
All cross-site iframes share one frame process today, so a compromised ad frame shares memory with
every other site's frames. Run iframes in the site process pool (one process per frame site,
`process.rs` `ensure`), with the frame process's `Auth` (storage off, tops = the frames'
top-level sites), and keep the iframe sandbox checks R5 added in `frames::pump`. Keep the
"stuck frame" stop working per process.

### X15. Turn site processes on by default
**Status:** free
**After:** X13, X14
**Checklist:** none
**Files:** `src/settings.rs` (the `site_processes` default), `docs/`
Make `site_processes` default to on once WebSockets and per-site frames work in them. Run the
screenshot tests and `tools/site_check.py --tracker` with it on and fix what got worse; note the
memory cost in the docs. John runs the live-site check on his computer afterwards.

### X9. Geolocation in Flatpak and with site isolation
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/geo.rs`, the location parts of `src/process.rs`, `packaging/flatpak/`
Leftovers from X6. GeoClue is on the system bus, which the Flatpak sandbox blocks: when the
system bus can't be reached (or inside Flatpak), ask the XDG Location portal
(`org.freedesktop.portal.Location` on the session bus) instead, and add its permission to the
Flatpak manifest. With site isolation on, pages run in sandboxed processes that can't reach
D-Bus: keep `geo.rs` in the browser process and send positions to the page's process.
Test the portal path the way `geo::tests::talks_to_geoclue` tests GeoClue (a pretend service
over a socket pair).

### X10. Take WebGL off the warning exemption
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/webgl.rs`, `src/webgl_raster.rs`, `src/webgl_tex.rs`, `src/glsl.rs`, `src/glsl_gen.rs`, `src/glsl_vm.rs`, the six `mod` lines in `src/main.rs`
R3 exempted these modules with `#[allow(dead_code, unused_mut)]` in main.rs while T12 was still
building WebGL. T12 is done: remove the six allows and fix the ~15 warnings they hide (unread
fields such as `antialias` and `webgl2`, an unused `POINTS` constant, the `Fma` variant, `NoTex`,
two needless `mut`s). No behavior changes; WebGL tests and WPT scores stay the same.

### X11. Lay boxes out on several cores
**Status:** free
**After:** —
**Checklist:** none (the checklist item was ticked with T04/T04b)
**Files:** `src/font.rs`, `src/layout.rs`, `src/preshape.rs`, new files for the parallel parts
Leftover from T04b: words are shaped across cores before layout, but boxes are still laid out
on one thread. `Fonts` keeps faces and caches in `Rc`/`RefCell`, shared with painting, so make
shaping and measuring thread-safe first (or give each thread its own shaping state), then lay
out independent blocks (for example a long page's top-level sections, or flex and grid items
measured on their own) at the same time. Painting also shapes whole lines again
(`paint.rs`), which could move to the same threads. Measure on a text-heavy page and on
tests/real-world pages before and after; keep every reference screenshot identical.

### X16. `createElementNS` for any namespace, with prefixes and name checks
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/dom.rs` (the `Ns` namespace map), `src/js.rs` (`n_create`), `src/prelude.js`
(`createElementNS`), `src/bindings/dom.rs` (`namespace_uri`, `prefix`, `tag_name`, `local_name`)
WPT `dom/nodes/Document-createElementNS.html` fails 596 subtests. Elements can only be in the
HTML, SVG or MathML namespace, so `createElementNS(null, "foo")` or a custom namespace makes an
HTML element (`tagName` "FOO", `namespaceURI` the XHTML one). Prefixes (`svg:rect`) aren't kept
(`prefix` is always null), and names aren't validated (`1foo` must throw InvalidCharacterError;
follow the current DOM "validate and extract" rules, which the test expects). Store any
namespace per element (null included) and its prefix; `tagName` uppercases only HTML elements in
HTML documents (already so for SVG/MathML). Also look at `createAttributeNS`/`setAttributeNS`
and `Document-createElement-namespace.html`. Run `tools/wpt.py <wpt> dom/nodes` before and after
(1535/5629 at the start); the XML/XHTML-document variants need `document.implementation`
`createDocument`, which can be left undone if it grows large.

### X17. Media Source Extensions: frame-level edits, sequence mode, tracks
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/mse.rs`, `src/mse.js`, `src/media.js` (MSE hooks only)
T10 handles coded frames per segment. Make `remove(start, end)` and `appendWindowStart/End`
drop the frames outside the range rather than whole segments (split a segment by rewriting its
`trun`/blocks, or keep a per-segment start/end trim the feed applies with ffmpeg's `-ss`/`-t`);
regenerate timestamps in `mode = "sequence"` (update `timestampOffset` after each append as the
spec says); fill `SourceBuffer.audioTracks`/`videoTracks` and `HTMLMediaElement`
`audioTracks`/`videoTracks` from the init segments; accept `srcObject = mediaSource`; support
`audio/mpeg` and `audio/aac` byte streams if cheap. Run `tools/wpt.py <wpt> media-source`
before and after (215/339 at the start; `mediasource-remove`, `-appendwindow`,
`-sequencemode-*`, `-avtracks`, `-timestamp-offset` are the files this should move).

### X18. Inline SVG: shape hit testing, SVG property animation, foreignObject
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/svg_inline.rs`, new code in new files; small hooks in `src/layout.rs`
(hit list) and `src/anim.rs`
Inline `<svg>` (X5) draws as one replaced box. (1) Clicks, hover and `:hover` target the whole
`<svg>`: give layout a hit for each shape (resvg/usvg can give node bounding boxes and test a
point against a path) so `event.target` is the `<path>`/`<circle>` under the pointer and hover
styles on shapes work. (2) CSS transitions and animations of `fill`, `stroke`, `opacity`,
`transform` on SVG elements don't run (`svg_decls` only takes the cascaded value); feed the
animated values in. (3) `<foreignObject>` content isn't drawn (resvg ignores it); lay its HTML
out as a box positioned over the SVG. Check with `tests/inline-svg.html` and new pages; keep the
screenshot tests the same unless more correct.

### X19. Layout memory: incremental box tree, shared cached items
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/boxtree.rs`, `src/relayout.rs`, `src/layout.rs` (the parts that call them)
R4 measured peak memory against the version before T01: 148 → 168 MB on a generated
20,000-node page (about 11 MB is the relayout cache, the rest the box tree), 47 → 53 MB on
`tests/real-world/react.html`. (1) The box tree copies the whole DOM on every layout: update it
in place when only some subtrees changed (the restyle pass knows which). (2) The relayout cache
stores a box's display items again in every enclosing box's entry: share them (e.g. store an
entry's children as references to their own entries) or lower the budget. Keep the relayout
tests (`cargo test --release relayout`, including `structural_edits_match_fresh_layouts`)
passing and measure with `cargo test --release relayout_timing -- --ignored --nocapture` and
peak RSS of `--png` runs before and after.

### X20. Let WPT media tests play under `tools/wpt.py`
**Status:** free
**After:** —
**Checklist:** none
**Files:** `tools/wpt.py`, the `--png` path in `src/main.rs` (small)
`--png` freezes animations and media so screenshots are stable, so WPT files that wait for
playback (`timeupdate`, `ended`, seeking while playing) never finish: about 29 `media-source`
files and many in `media`/`html/semantics/embedded-content/media-elements` give no result. Add a
mode (a flag or environment variable that `tools/wpt.py` sets for script tests) where media
plays in real time and the run ends when the test harness reports, with the usual timeout. Then
report the new numbers for `media-source` and `html/semantics/embedded-content/media-elements`.

## Review and cleanup of the overnight work

### R1. Re-run the official test suites
**Status:** done
**Result:** nothing got worse. DOM 1,535 of 5,629 (was 1,391), Shadow DOM 245/402 and 11/18
reference pages, IndexedDB 1,098/1,284, selectors 450/816 and 39/135 pages, flexbox 291/894
pages and 79/398, floats 19/78 pages; `docs/CONFORMANCE.md` updated.
**After:** —
**Checklist:** none
**Files:** whatever a regression needs
Run `tools/wpt.py` on dom/nodes, shadow-dom, css/selectors, IndexedDB, css/css-flexbox and
css/CSS2/floats; compare with `docs/CONFORMANCE.md` and fix anything that got worse overnight.

### R2. Site check: compare with last night
**Status:** done
**Result:** run of 11:30: 15 work, 19 usable, 6 broken, 0 crash (was 16/16/7/1).
`site-compat.json` now keeps each site's first script errors. Fixed: `crypto` (Mastodon,
Medium) and `PerformanceObserver` (Bing) didn't exist. **Left for X1** (these sites can't be
reached from the review chat): GitHub fell from Works to Broken (likeness 94 → 49) with
`TypeError: not a callable function` in core-*.js and marketing-essentials-*.js; `ReferenceError:
Image is not defined` on Wikipedia, YouTube and stats.wp.com although `new Image()` works on
test pages (an inline script early in the page, or a callback); `ReadableStream` missing
(Medium); NYT defines `nyt-betamax` twice ("already defined").
**After:** —
**Checklist:** none
**Files:** whatever a regression needs
Compare the new `docs/SITE_COMPAT.md` run with last night's (16 work, 16 usable, 7 broken,
1 crash) and fix any site that got worse. (X1 keeps fixing the sites that were already broken.)

### R3. Clean up the compiler warnings
**Status:** done
**After:** —
**Checklist:** none
**Files:** any, but only to remove dead code or fix warnings; no behavior changes
`cargo build --release` prints 128 warnings, mostly unused code left over from the parallel
work. Remove what's truly unused, fix the rest, and make CI fail on new warnings once it's clean
(`RUSTFLAGS=-D warnings` in the build step).
**Left undone:** WebGL (T12) is still being built, so its six modules (`webgl`, `webgl_raster`,
`webgl_tex`, `glsl`, `glsl_gen`, `glsl_vm`) carry `#[allow(dead_code, unused_mut)]` in main.rs;
T12 should remove those allows and fix what they hide when it's finished. Everything else builds
with no warnings (release, tests, no-GPU), and CI now fails on new ones.

### R4. Review the box tree and incremental relayout
**Status:** done
**After:** —
**Checklist:** none
**Files:** `src/boxtree.rs`, `src/layout.rs`
The largest overnight change. Read it for correctness (stale boxes after DOM edits, relayout
that misses a change, anonymous boxes, `display: contents`), add tests for each bug found, and
fix them. Check memory use and speed on a large page against the version before T01.
Found and fixed (tests in `relayout.rs`): a picked file never showed in a cached
`<input type=file>` (the files live in the form state, not the DOM); a canvas, video frame or
live iframe redrawn each frame threw every cached box away (the image stamp hashed identities,
not sizes); text laid out before a page's web fonts arrived kept the fallback font. A new test
runs every test page through seven kinds of structural edit (385 checks against fresh layouts,
all equal). Speed against the version before T01, median of 5 `--png` runs: a generated
20,000-node page 254 → 277 ms, Bootstrap 141 → 136 ms, React 271 → 282 ms, tables 85 → 90 ms,
grid 97 → 93 ms (about the same). Peak memory: 148 → 168 MB on the big page (about 11 MB is the
relayout cache, capped at 6× the page's display items; the rest is the box tree, a copy of the
DOM per layout), 39 → 41 MB on Bootstrap, 47 → 53 MB on React.
Left undone: the box tree is rebuilt in full every layout (building it incrementally would save
the copy); the cache stores nested boxes' items once per level (a shallower budget or shared
items would use less memory).

### R5. Security review of the per-site processes
**Status:** done
**After:** —
**Checklist:** none
**Done:** the frame process (every cross-site iframe) could read the whole profile (all cookies, storage, history, cache): Landlock now allows only its settings files, and its requests and `document.cookie` go through the browser. Renderers no longer see cookies: they send a placeholder, and the browser attaches cookies and keeps Set-Cookie (HttpOnly cookies never reach a renderer). `document.cookie` only for a document the process holds; no storage or extension calls from the frame process; at most 64 requests in flight per process; iframe `sandbox` top-navigation and popup rules and the tab-URL check enforced in the browser. Malformed or oversized messages end only that renderer. Tests for each rule (process.rs, frames.rs, sandbox.rs).
**Left undone:** the frame process is still one process for all cross-site frames, and keeps network access (for WebSockets), so a compromised frame can read other frames in it and make cookie-less requests directly; no ORB/CORB, so a compromised renderer can read cross-site responses it fetches with credentials; WebSockets don't work in site processes (no browser-side proxy) and send no cookies from frames; with third-party cookies turned on, the shared frame process reaches the non-HttpOnly cookies of any site framed under its pages' top-level sites; site processes are still off by default (the `site_processes` setting).
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
**Left undone:** tab ids are tab positions + 1, so they shift when an earlier tab closes; webRequest only has `onBeforeRequest` (cancel/redirect) and guesses the resource `type` from the request; no `declarativeNetRequest`, `runtime.connect` ports, `alarms`, `contextMenus` or toolbar buttons; no about:extensions page or installer (extensions load unpacked from `<profile>/extensions/` or `IRONSWIFT_EXTENSIONS`); a site process's content-script requests are only checked against the extension's match patterns and the process's sites.
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
