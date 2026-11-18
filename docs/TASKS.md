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

**Standard browser first (2026-11-18):** the original plan is nearly done. Until it is, take only
these tasks, in this order of preference: X1b, P1a–P1d, J1–J4. Leave the optional add-ons (O, P2–P8,
B tasks) alone unless none of those is free.

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

### AB1. Content blocking on a par with uBlock Origin
**Status:** done
**Left undone:** `trusted-edit-inbound-object` (it needs uBO's JSONPath dialect; only YouTube's rules use it) and other rarely used scriptlets; `##^responseheader()`; HTML filters support plain CSS selectors and one trailing `:has-text()`; `$header=`, `$permissions=`, `$replace=`, `$urltransform`, `$cname`; `$method` assumes GET; `:others()` selects nothing; the `css` fuzz target stubs out `@keyframes` parsing (anim.rs needs the style engine). In debug builds a page with many scriptlets can need a little over 2 MB of stack (tests on 2 MB threads: run them in release, as CI does). Checks with all the lists loaded take about 30 µs per request (release). The `filters` fuzz target ran 288,000 inputs (8 minutes, AddressSanitizer, seeded from the uBlock and EasyList lists) without a crash; run it with `RUSTC_BOOTSTRAP=1 cargo fuzz run filters` where there's no nightly toolchain.
**After:** —
**Checklist:** none (the §14 ad-blocking items are already ticked; this deepens them)
**Files:** `src/blocklist.rs`, new `src/scriptlets.js` and `src/cosmetic.js`, `src/net.rs`, `src/js.rs` (document-start hook), `src/settings.rs`, locales
Bring `blocklist.rs` up to what uBlock Origin's lists need: `!#if`/`!#include`, regex rules,
`$badfilter`, `$redirect` stand-in resources, `$removeparam`, `$csp`, `$denyallow`/`$to`,
`$method`, `$generichide`/`$specifichide`/`$genericblock`, `$popup`, `example.*` domains,
highly generic and procedural cosmetic filters (`#?#`, `:has-text`, `:upward`, `:remove()`,
`:style()`), and `##+js(...)` scriptlets written from scratch (MIT, not uBO's GPL code). Add
the uBlock filter lists and Peter Lowe's list.

### AB2. The last two uBlock Origin filter features
**Status:** done (chat-opus-t12 2026-11-17)
**Done:** `trusted-edit-inbound-object` with uBlock Origin's query syntax (`jsonEdit` in `src/scriptlets.js`: `.key`, `..key`, `[*]`, `[n]`, `[?.a.b]` filters with `==`, `!=`, `*=`, `^=`, `$=`, `=/re/`; edits `+={json}`, `=value`, `=repl({...})`, or removal); `##^responseheader(name)` drops `location`, `refresh`, `report-to` or `set-cookie` from a page's own response in `net::http_stream`, with `#@#` exceptions. Tests: `content_blocking_more_scriptlets` (the YouTube rule's query and two more), `blocklist::tests::response_header_filters`.
**Left undone:** uBlock Origin's rarer query forms (`[-]` array removal inside queries, `{…}` property lists) aren't parsed; queries using them are skipped.
**After:** AB1
**Checklist:** none (follow-up to AB1)
**Files:** `src/scriptlets.js`, `src/blocklist.rs`, `src/main.rs` (`prepare`, response headers)
- `trusted-edit-inbound-object`: edit an object a function is called with, using uBlock
  Origin's JSONPath-style syntax (`[?.attestationRequest][?.context.client.userAgent*="channel"]
  .context.client+={"clientScreen":"CHANNEL"}`: filters `[?.a.b==x]`, `*=`, `^=`, `$=`, `=/re/`,
  `+=` merge, `=repl({...})`). Only YouTube's rules in `ublock-filters.txt` use it today.
- `##^responseheader(name)`: drop a response header from a page before it's used (uBlock
  Origin only allows a short list of names, e.g. `refresh`, `location`, `report-to`).
Test both like `content_blocking_more_scriptlets` and `trusted_scriptlets_and_script_filters`.

### X1. Fix the broken and crashing sites (now: AP News and MDN)
**Status:** done
**Left undone:** AP News loads but slowly (~65 s on replay): its scripts measure and change the
page in turns, and each answer is a full restyle and relayout (~1 s with `:has()` turning the
restyle cache off), so it needs T02's incremental relayout. MDN is close to Chrome; still
different: its hero headline wraps sooner, the search button's styling, and a sliver of the
language menu shows. `getComputedStyle` doesn't give custom properties' cascaded-only values
for pseudo-elements. Sort comparators: Boa's own unit test for the new merge sort can't run from
this workspace (the page test covers it).
**After:** —
**Checklist:** none
**Files:** whatever each fix needs; keep changes small (other chats are editing layout and JS)
`docs/SITE_COMPAT.md`: AP News crashes; MDN, Bing, BBC News, New York Times, The Guardian,
rust-lang.org and WordPress.org render badly. Start with the crash. Find the cause of each,
fix it in the engine (no site-specific hacks), and add a small test page for each cause.
Progress: AP News no longer hangs and rust-lang.org is fixed; pages' own `<script>` and
`<style>`/`<link>` insertions, shadow-DOM slots, `light-dark()`, viewport units, TreeWalker,
CSSOM sheets and Annex B now work. BBC, NYT, The Guardian, Bing and WordPress.org moved to
X1a–X1e; this task keeps AP News (it now panics in a sort, see `x1-data2`) and MDN.

### X1a. Make BBC News render like Chrome
**Status:** done
**After:** —
**Checklist:** §21 "BBC News renders like Chrome" (required fix-up)
**Done:** the replay (recording `rec-x1a`) matches Chrome's first screenful with no script errors: React hydration no longer throws (settable `defaultValue`, Navigation Timing names in `performance.measure`), the menus show (out-of-flow boxes don't widen shrink-to-fit boxes), numeric font weights pick the right web-font face (Reith 500), letter-spaced titles don't wrap, `fr` rows of auto-height grids hold their items, grid `align-content` works and grid items' percentage heights use their grid area. Tests: `tests/sites/bbc-startup.html`, `abs-shrink.html`, `letter-spacing-shrink.html`, `grid-fr-rows.html`, `grid-align-content.html`, `tests/font-weights.html`. WPT css-flexbox ref 291 -> 294/894, CSS2/floats ref 19 -> 21/78.
**Left undone:** the consent pop-up (a third-party iframe from cdn.privacy-mgmt.com) doesn't show; in a local replay on a 2-core machine the lazy images below the first screen stay grey because React finishes hydrating after the snapshot (they load in the GitHub recording); `align-content` is done for grid rows only (not multi-line flex containers).
**Files:** whatever each fix needs; keep changes small, new code in new files (others are in layout and JS too)
Split out of X1. Recording: folder `www-bbc-com-news` in branch `x1-data2` (URL `https://www.bbc.com/news`). Follow
`docs/SITE_DEBUGGING.md`: replay it, compare with `chrome.png`, find each cause, fix it in the
engine (no site-specific hacks), and add a small test page in `tests/sites/` with a test for each.
Next.js + Emotion: X1's `document.referrer` and live-stylesheet fixes should move it on a lot. Record it again (`record-sites.yml`, branch `rec-x1a`) once the first errors
are fixed: code that never ran was never recorded. Done when the replay looks like Chrome's
first screenful and has no script errors of its own (ads and trackers missing is fine).
Start by recording it again with today's `main`: `x1-data2` predates X1's live-stylesheet fix.

### X1b. Make the New York Times render like Chrome
**Status:** claimed by chat-x1b-2 2026-11-18 05:16
**After:** —
**Checklist:** §21 "The New York Times renders like Chrome" (required fix-up)
**Files:** whatever each fix needs; keep changes small, new code in new files (others are in layout and JS too)
Split out of X1. Recording: folder `www-nytimes-com` in branch `x1-data2` (URL `https://www.nytimes.com/`). Follow
`docs/SITE_DEBUGGING.md`: replay it, compare with `chrome.png`, find each cause, fix it in the
engine (no site-specific hacks), and add a small test page in `tests/sites/` with a test for each.
Renders only 768px tall with 14 script errors (`crypto is not defined` was one) and takes ~10 s of script. Record it again (`record-sites.yml`, branch `rec-x1b`) once the first errors
are fixed: code that never ran was never recorded. Done when the replay looks like Chrome's
first screenful and has no script errors of its own (ads and trackers missing is fine).
Start by recording it again with today's `main`: `x1-data2` predates X1's live-stylesheet fix.
**Progress (2026-11-17, chat-opus-x1b):** module scripts run once per URL; `<a>` URL parts (hyperlink.js); ChildNode methods on text/comment nodes (childnode.js); custom properties shared in layers (cssvars.rs, NYT replay 6 GB → 1.4 GB); page height covers overflowing content and html/body overflow goes to the window (doc_extent.rs; WPT refs 440 → 461); Boa value-stack limit raised (React's commit finished); Date parsing fallbacks and English Intl.DateTimeFormat (datetime.js, masthead back). Script errors 14 → 1; whole front page renders. Recording workflow: secret scrubbing, `wait` input.
**Still to do:** replay the `rec-x1b` recording made with `wait=300` (story images come from GraphQL fetched late; earlier recordings lack them); "Today's Paper" overlaps the "U.S." nav item; no border on the "free to read" box; the elections widget's Svelte hydration fails ("not a DOM node", below the first screen). Chrome's screenshot in new recordings is NYT's robot-block page: compare with `x1-data2`'s.

### X1c. Make The Guardian render like Chrome
**Status:** done (chat-opus-x5 2026-11-17 16:40)
**After:** —
**Checklist:** §21 "The Guardian renders like Chrome" (required fix-up)
**Files:** whatever each fix needs; keep changes small, new code in new files (others are in layout and JS too)
Split out of X1. Recording: folder `www-theguardian-com` in branch `x1-data2` (URL `https://www.theguardian.com/`). Follow
`docs/SITE_DEBUGGING.md`: replay it, compare with `chrome.png`, find each cause, fix it in the
engine (no site-specific hacks), and add a small test page in `tests/sites/` with a test for each.
Redirects to `/us`; 11 script errors in the recording. Record it again (`record-sites.yml`, branch `rec-x1c`) once the first errors
are fixed: code that never ran was never recorded. Done when the replay looks like Chrome's
first screenful and has no script errors of its own (ads and trackers missing is fine).
Start by recording it again with today's `main`: `x1-data2` predates X1's live-stylesheet fix.
**Done:** the replay matches Chrome's first screenful from the top bar down, and the
only script errors left are Ophan tracker beacons (a random view id in every URL, so they can
never be in a recording). Fixes, each with a test: a block inside an inline gets its own block
box; `document.doctype`; `performance.navigation`/`performance.timing`; SVG presentation
attributes take `var()`; `visibility: hidden` content keeps its size; IntersectionObserverEntry
fields on the prototype (pages no longer load the polling polyfill); shrink-to-fit leaves room
for inner blocks' right padding; `matchMedia` gives the stylesheets' answer (`dpcm` too).
Script errors 11 → 0 of its own. WPT CSS2/normal-flow 70 → 75/117 script, 11 → 17/46 ref;
CSS2/floats 19 → 23/36 script, 19 → 20/78 ref; css-flexbox unchanged (one subtest lost:
block-in-inline-client-rects-001, the span's own rectangle after a split).
**Left undone:** the grey 970×250 ad placeholder at the top stays, because the built-in block
list stops Google's ad server so the slot never reports "empty" (the full EasyList hides it).
"Support the Guardian" doesn't show in the replay: its script chunk loaded after the recorder's
5 s window, so it isn't in the recording (the replay does ask for it). Headline text draws a
little heavier than Chrome's (same Medium face; antialiasing). Recording branch `rec-x1c` can
be deleted.

### X1d. Make Bing render like Chrome
**Status:** done (chat-opus-t12 2026-11-17)
**Done:** Bing's replay has no script errors and its search bar renders like Chrome's: window
`on*` handler properties exist (assigning `onload` threw), navigation timing fills in load
timestamps, `width: fit-content / max-content / min-content` work (an absolute box between
`left` and `right` shrinks to fit and auto margins center it), `font: inherit` and other font
`inherit`s undo UA defaults, and `<textarea>` draws in its own font (monospace only by default).
Test: `tests/sites/bing-searchbox.html` (`fit_content_search_box_is_centered`).
**Left undone:** a faint 1px horizontal line across the photo near y=550; Chrome's reference
(live site, 1280×2000 window, before the photo loads) differs in classes and timing, so the
comparison is by CSS, not pixels.
**After:** —
**Checklist:** §21 "Bing renders like Chrome" (required fix-up)
**Files:** whatever each fix needs; keep changes small, new code in new files (others are in layout and JS too)
Split out of X1. Recording: folder `www-bing-com` in branch `x1-data2` (URL `https://www.bing.com/`). Follow
`docs/SITE_DEBUGGING.md`: replay it, compare with `chrome.png`, find each cause, fix it in the
engine (no site-specific hacks), and add a small test page in `tests/sites/` with a test for each.
Likeness was negative: almost nothing of the page shows (291 nodes, 768px). Record it again (`record-sites.yml`, branch `rec-x1d`) once the first errors
are fixed: code that never ran was never recorded. Done when the replay looks like Chrome's
first screenful and has no script errors of its own (ads and trackers missing is fine).
Start by recording it again with today's `main`: `x1-data2` predates X1's live-stylesheet fix.

### X1e. Make WordPress.org render like Chrome
**Status:** done
**Left undone:** close to Chrome now (header, fonts, layout, no script errors of its own).
Still different: the menu's dropdown arrows draw as black squares (CSS `mask-image` isn't
supported), headings set in Inter at weight 600 come out regular (one weight per variable web
font), menu items sit closer together so "Swag ↗" wraps, and the emoji check's worker fails
(`OffscreenCanvas` in workers needs canvas drawing without a document; WordPress just skips its
emoji fallback). Faces split by `unicode-range` use their basic-Latin subset; choosing a face per
character (for pages in other scripts) is still to do. Recording branch `rec-x1e` can be deleted.
**After:** —
**Checklist:** none
**Files:** whatever each fix needs; keep changes small, new code in new files (others are in layout and JS too)
Split out of X1. Recording: folder `wordpress-org` in branch `x1-data2` (URL `https://wordpress.org/`). Follow
`docs/SITE_DEBUGGING.md`: replay it, compare with `chrome.png`, find each cause, fix it in the
engine (no site-specific hacks), and add a small test page in `tests/sites/` with a test for each.
4 script errors (a JSON parse error among them). Record it again (`record-sites.yml`, branch `rec-x1e`) once the first errors
are fixed: code that never ran was never recorded. Done when the replay looks like Chrome's
first screenful and has no script errors of its own (ads and trackers missing is fine).
Start by recording it again with today's `main`: `x1-data2` predates X1's live-stylesheet fix.

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
**Status:** done
**After:** —
**Checklist:** none
**Done:** `src/img_load.rs` + `src/img.js`: script-made images load in the background (as images, no CORS), fire `load`/`error`, and give `complete`, `naturalWidth`/`naturalHeight`, `width`/`height`, `currentSrc`, `crossOrigin` and `decode()`; their pixels go to the canvas store for drawImage and texImage2D. three.js `TextureLoader` textures render. Fixed on the way: unread GLSL fragment inputs no longer fail linking, and `TEXTURE0`–`TEXTURE31` exist. WPT `webgl/`: 86/87.
**Left undone:** `<img>` from the page's HTML fires `load` only once a script listens for it or reads `complete`/`naturalWidth` (it then loads a second time here); no `loading=lazy` or `srcset` re-selection for script images; canvas tainting by cross-origin images isn't tracked.
**Files:** the image-loading code (`src/page_images.rs`, `src/images.rs`), `src/prelude.js` (the `Image` factory only)
An `<img>` created by script (`new Image()`, `document.createElement("img")`) and never added to
the document never loads: `complete` is undefined and `load`/`error` never fire. three.js's
`TextureLoader` and most WebGL texture code rely on this, so WebGL pages show no textures.
Load such images when their `src` is set, fire `load`/`error`, and give `complete`,
`naturalWidth`/`naturalHeight` and `decode()`. WPT `webgl/texImage2D-image-not-loaded.html` and
the `html/semantics/embedded-content/the-img-element` tests that apply.

### X13. WebSockets in site processes
**Status:** done
**After:** —
**Checklist:** none
**Files:** `src/websocket.rs`, the request parts of `src/process.rs`
Site processes have no network, so `new WebSocket()` fails in them (chat widgets, live
dashboards). Have the browser process open the socket for the page's process and relay frames
both ways over the existing request channel, with cookies attached in the browser (as `http`
requests do since R5: see `COOKIE_TOKEN`). Also use it from the frame process, then take network
access away from the frame renderer (`sandbox::protect_frame_renderer`, `no_net`). Cap open
sockets per process. Test with `tools/ws_echo.py`.
Done in `src/ws_relay.rs`: renderers ask the browser to open sockets (`ws_open`), send in order
(`ws_send`) and get events through `ws_poll`, which the browser answers as soon as something
happens. Cookies, HSTS and the blocklist are applied in the browser; sockets are bound to a
random per-renderer token tied to its sites, capped at 64 per renderer, and closed when their
renderer stops polling for 90 s. The frame renderer's filter now refuses internet sockets
(`protect_frame_renderer` passes `no_net`). Tests: token ownership, and end to end the real
`--frame-renderer` with no network echoing through a local WebSocket server.
Left undone: `bufferedAmount` reads 0 in renderers (the browser holds the queue); the
end-to-end test covers the frame renderer, and site processes use the same code but aren't
started by a test (they need the window).

### X14. One process per site for cross-site iframes
**Status:** done
**After:** —
**Checklist:** none
**Done:** `frames::isolated` runs one frame process per frame site (`--frame-renderer <site>`, refusing other sites' frames), each with an `Auth` limited to its own frames' top-level sites; at most 6 processes (`IRONSWIFT_FRAME_PROCESS_MAX`), then an idle one is ended or the least busy one is shared (`allow`); `stop_stuck` stops only the process drawing the stuck snapshot. Checked with two cross-site iframes (two processes, cookies right in each) and with the cap at 1.
**Left undone:** frame processes are separate from the site-process pool in process.rs (two pools, each with its own cap); a frame that navigates to another site keeps running in the first site's process (no process swap on navigation); frame processes still have network access (until X13 moves WebSockets to the browser).
**Files:** `src/frames.rs` (`isolated`), `src/process.rs` (the pool)
All cross-site iframes share one frame process today, so a compromised ad frame shares memory with
every other site's frames. Run iframes in the site process pool (one process per frame site,
`process.rs` `ensure`), with the frame process's `Auth` (storage off, tops = the frames'
top-level sites), and keep the iframe sandbox checks R5 added in `frames::pump`. Keep the
"stuck frame" stop working per process.

### X15. Turn site processes on by default
**Status:** done (chat-opus-t12 2026-11-17)
**Done:** `site_processes` defaults to on; saved as `site_isolation` so profiles that saved the old off default get it too; the setting's hint updated in all six languages; `docs/ARCHITECTURE.md` updated. 45 of 45 screenshot pages were already identical both ways (X23).
**Left undone:** `site_check.py --tracker` needs internet access: John runs the live-site check.
**Progress:** (chat-nsX16; X23 since fixed video and audio, so all 45 screenshot pages are identical both ways) `--png` of an http(s) page goes through a site process with `IRONSWIFT_SITE_PROCESS=1` (`src/site_snapshot.rs`); `tools/site_process_check.py` draws every screenshot test page both ways. Fixed: site processes had no system fonts with a fresh profile (the browser now writes the font list they read). 43 of 45 pages identical; animated images differ only because site processes animate (snapshots freeze); `media.html` shows "This video can't be played". Memory: about 50-70 MB per site process on top of the browser (in `docs/ARCHITECTURE.md`). `site_check.py --tracker` needs internet access, so it wasn't run here; John runs the live-site check after the default changes.
**After:** X13, X14, X23, O3
**Checklist:** §21 "Site processes on by default" (required fix-up)
**Files:** `src/settings.rs` (the `site_processes` default), `docs/`
Make `site_processes` default to on once WebSockets and per-site frames work in them. Run the
screenshot tests and `tools/site_check.py --tracker` with it on and fix what got worse; note the
memory cost in the docs. John runs the live-site check on his computer afterwards.


### X23. Video and audio in site processes
**Status:** done
**Done:** a site process keeps downloaded media in an in-memory file and asks the browser over a Unix socket handed down at start (`src/media_broker.rs`) to start `ffmpeg`, `ffprobe` or the sound player — by name, never a path — and gets the program's pipes back (SCM_RIGHTS); media.rs reads them as before. The browser starts decoders through `ironswift --media-decoder`, which locks them down first (`sandbox::protect_media_decoder`: no network, no writing, read-only system files; test `media_decoder_lockdown`), and stops a site process's programs when it goes away. `tools/site_process_check.py`: 45 of 45 screenshot pages identical in-process and in a site process (`media.html` included). In-process playback unchanged: media tests (decoding now also tested from an in-memory file), WPT `media-source` 284/424, `media-elements` 223/285 (one pitch subtest flips between runs with the old binary too).
**Left undone:** sound wasn't heard here (no sound player installed); its path is the same pipe hand-off as video.
**After:** —
**Checklist:** §21 "Video and audio in site processes" (required fix-up)
**Files:** `src/media.rs` (starting the decoders), `src/process.rs` / `src/sandbox.rs` (small hooks), new code in new files
Found in X15: a site process can't start programs or write files, and media decoding runs
`ffmpeg`/`ffprobe` (and a sound player) on a downloaded temp file, so `<video>`/`<audio>` show
"This video can't be played" with site processes on. Have the browser start the decoders for
the site process (passing their pipes back, e.g. over a Unix socket with SCM_RIGHTS), feed them
the media bytes through a pipe instead of a temp file, and lock the decoders down too (they
parse untrusted files: no network, no writing, read-only system files). Sound is played by the
browser. Check with `tools/site_process_check.py media.html` and `cargo test --release media`.

### X24. Permission prompts from site processes
**Status:** done (chat-opus-t12 2026-11-17)
**Done:** a top-level site-process page's request goes to the window (`Update::Permission`), checked against the shown page's origin; already-decided sites are answered at once, otherwise the usual prompt shows for that tab and the answer goes back (`Event::PermDone`). Every site process is told each decision and each about:settings reset (`process::tell_permission`, `permissions::learned`/`forgotten`). A frame's requests are refused at once. Tested in the real window (prompt, Allow, `Notification.permission` then "granted") and `frames::tests::site_process_pages_ask_the_window_for_permissions`.
**Left undone:** a request from a site-process page in a background tab waits until you switch to it (as before for local pages, the prompt belongs to the tab shown).
**After:** —
**Checklist:** none
**Files:** `src/js.rs` (`n_perm_request`), `src/process.rs`, `src/frames.rs`, the prompt part of `src/main.rs`, `src/permissions.rs`
Left from X9. A page in a site process that asks for a permission (location, notifications,
camera, microphone, clipboard-read) never sees a prompt: `__permRequest` queues the request in
the site process, and only the window's own pages are read by `js::take_permission_requests`
(main.rs), so the page's promise waits forever. Send the request to the browser (an update or a
request over the process's pipe, checked against the pages the process holds), show the prompt
for that tab, and send the answer back to `__permDone`. A site process also reads
`permissions.tsv` once when it starts: after you answer (or reset a site on about:settings),
tell its processes. Test with a real sandboxed renderer, as `geo_relay::tests` does.

### X25. Lay out independent blocks on several cores
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/style.rs` (Rc → Arc), `src/font.rs`, `src/layout.rs`, `src/relayout.rs`, new files for the parallel parts
Left from X11 (which shapes lines across cores). `Style` holds `Rc` fields (family, custom
properties, transforms, filters, grid, animations), so styles can't be read on other threads:
make them `Arc`. Make `Fonts`' caches thread-safe or per thread. Then lay out blocks that don't
depend on each other at once (new block formatting contexts with no floats reaching in, flex and
grid items measured on their own), each into its own item list at its own origin, and merge them
in order, with the relayout cache kept right. Measure with `RELAYOUT_PAGES=<page> cargo test
--release relayout_timing -- --ignored --nocapture` (1500 paragraphs on 2 cores: ~180 ms first
layout, ~70 ms warm, of which ~60 ms is line breaking); keep every reference screenshot identical.

### X26. Media Source Extensions: the details X17 left
**Status:** free
**After:** —
**Checklist:** none
**Files:** `src/mse.rs`, `src/mse_frames.rs`, `src/mse.js`, `src/media.js` (MSE hooks only)
Left from X17: an audio frame that straddles `appendWindowStart` is dropped, not trimmed (the spec
keeps the part inside the window); laced WebM blocks (several frames in one block) count as one
frame; `remove()` doesn't drop the element's `readyState` to HAVE_METADATA when it removes the
current position; `QuotaExceededError` isn't an interface of its own (WPT
`mediasource-appendbuffer-quota-exceeded` checks `ex.constructor`). Run WPT `media-source` before
and after.
### X27. align-content for multi-line flex containers
**Status:** done (chat-opus-t12 2026-11-17)
**Done:** a wrapping flex container with a set height places its lines with `align-content` (center, end, space-between/around/evenly via `grid_align::row_offsets`); normal/stretch share the extra height among the lines and auto-height items stretch into it. Lines are measured first, then placed. Test: `tests/sites/flex-align-content.html` (`flex_align_content_spreads_wrapped_lines`). WPT `align-content-*` reference tests 4/26 → 23/26; the 62 flex-wrap reference tests 18 → 20, none lost (the full css-flexbox run was cut off by a workspace restart, so only those two subsets were compared).
**Left undone:** column-direction wrapping containers (lines side by side) don't use align-content yet; the 13 align-content script tests fail before and after (their harness).
**After:** —
**Checklist:** none
**Files:** the flex-wrap part of `src/layout.rs` (`flex_children`), `src/grid_align.rs` (reuse `row_offsets`)
Left from X1a: `align-content` (center, end, space-between/around/evenly, stretch) now works for
grid rows only. A wrapping flex container with a set height still stacks its lines at the top
instead of distributing them. `Flex::align_content` is already parsed and cascaded. Add a test page
in `tests/sites/` and run `python3 tools/wpt.py <wpt> css/css-flexbox` before and after
(`align-content-*` tests).

### X28. Let `--png` snapshots wait for work started by late animation frames and timers
**Status:** free
**After:** —
**Checklist:** none
**Files:** `Js::settle` in `src/js.rs`, `load_lazy_now` in `src/page_images.rs`
Left from X1a: on a slow machine, BBC News starts hydrating React (from a `requestAnimationFrame`
promise) only when `settle`'s 5 seconds are up, so the snapshot is taken before React hears the
`load` events of lazy images and removes their grey placeholders. Replay `rec-x1a`
(`www-bbc-com-news`) on a 2-core machine to see it: the "More to explore" pictures stay grey. Look
at running promise jobs after each animation frame before deciding the page is idle, and at
loading lazy images earlier, then taking one more short settle after they arrive. Keep `--png`
runs of ordinary pages as fast as now (time `tools/wpt.py` before and after).

### X29. Consent pop-ups served from third-party iframes (BBC News, Sourcepoint)
**Status:** free
**After:** —
**Checklist:** none
**Files:** whatever it needs; new JS in a file listed in `EXTRA_APIS`
Left from X1a: Chrome shows BBC's "Terms of Use & Privacy" dialog (Sourcepoint,
`cdn.privacy-mgmt.com`); Ironswift shows nothing. In the replay its messaging script ends with
"Failed to fetch" (requests it never got to make when recorded); on GitHub's live run it shows no
error but no dialog either. Record BBC again, find which request or API stops it, and fix that in
the engine. Optional for the site to look right (ads and trackers missing is fine), but many news
sites use the same consent service.

### X9. Geolocation in Flatpak and with site isolation
**Status:** done
**Result:** `geo.rs` falls back to `org.freedesktop.portal.Location` (session bus) inside Flatpak
or when the system bus or GeoClue can't be reached; tested against a pretend portal over a socket
pair (`geo::tests::talks_to_the_location_portal`, including the user saying no). Pages in site
processes get positions from the browser process through `geo_relay.rs` (`geo_start`/`geo_stop`/
`geo_gone`/`geo_poll`, checked against the browser's permission list and the process's pages);
tested end to end with a real sandboxed renderer. The Flatpak manifest needs no finish-arg for the
portal (always reachable; Flathub rejects `--talk-name` for portals): a comment says so. No WPT
change (geolocation WPTs need a permission prompt to be answered).
**Left undone:** permission *prompts* from site processes aren't forwarded to the window (for any
permission: location, notifications, camera, microphone), and a site process reads
`permissions.tsv` once when it starts. So with site isolation on, only sites allowed beforehand get
a position; a new request waits forever. Not tried on a real Flatpak or a real portal (none here).
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
**Status:** done (the whole crate now builds with no warnings)
**After:** —
**Checklist:** none
**Files:** `src/webgl.rs`, `src/webgl_raster.rs`, `src/webgl_tex.rs`, `src/glsl.rs`, `src/glsl_gen.rs`, `src/glsl_vm.rs`, the six `mod` lines in `src/main.rs`
R3 exempted these modules with `#[allow(dead_code, unused_mut)]` in main.rs while T12 was still
building WebGL. T12 is done: remove the six allows and fix the ~15 warnings they hide (unread
fields such as `antialias` and `webgl2`, an unused `POINTS` constant, the `Fma` variant, `NoTex`,
two needless `mut`s). No behavior changes; WebGL tests and WPT scores stay the same.

### X11. Lay boxes out on several cores
**Status:** done (the line-shaping part; see Left undone)
**Result:** Layout joins each line's words into one text item and measures it whole
(`Ctx::content_right`, for shrink-to-fit edges), which shaped every line of a big block on one
thread; painting then reused those shapes. `preshape::texts` now shapes a big block's unshaped
lines across cores before they're measured (same plan/run code as the word pass). The font caches
hit for every word use foldhash (still randomly seeded). 1500 paragraphs on 2 cores
(`RELAYOUT_PAGES=<page> cargo test --release relayout_timing -- --ignored --nocapture`, which now
also times a cold first layout, median of 7): first layout ~240 ms (1 thread) / ~245 ms (2 threads
before) → ~180 ms; warm layout ~80 → ~70 ms; whole `--png` render 603 → 467 ms (1 vs 2 threads).
Renders are byte-identical with 1 and 2 threads (that page and real-world/bootstrap.html); all 46
reference screenshots unchanged. Real-world test pages lay out in under 1 ms either way.
**Left undone:** laying out independent blocks on several threads. `Style` holds `Rc` fields
(family, custom properties, transforms, filters, grid, animations), so styles can't be shared with
other threads until those become `Arc` (a large change to style.rs); `Fonts` would also need its
`RefCell` caches made thread-safe or per thread, and `layout::Ctx` (items, floats, counters,
positioned boxes, the relayout cache) split and merged per block, with margin collapsing and floats
deciding which blocks are really independent. On the 2-core machines this is aimed at, the rest of
layout (inline line breaking, `flatten`) is ~60 ms warm on the 1500-paragraph page, so the gain left
is modest.
**After:** —
**Checklist:** §21 "Lay boxes out on several cores" (required fix-up)
**Files:** `src/font.rs`, `src/layout.rs`, `src/preshape.rs`, new files for the parallel parts
Leftover from T04b: words are shaped across cores before layout, but boxes are still laid out
on one thread. `Fonts` keeps faces and caches in `Rc`/`RefCell`, shared with painting, so make
shaping and measuring thread-safe first (or give each thread its own shaping state), then lay
out independent blocks (for example a long page's top-level sections, or flex and grid items
measured on their own) at the same time. Painting also shapes whole lines again
(`paint.rs`), which could move to the same threads. Measure on a text-heavy page and on
tests/real-world pages before and after; keep every reference screenshot identical.

### X16. `createElementNS` for any namespace, with prefixes and name checks
**Status:** done
**Done:** elements keep any namespace (or none) and their prefix (`Ns::Other` + `Dom::ns_names`); `createElementNS` and `createElement` check names with the current "validate and extract" rules (`src/qname.rs`); `tagName`/`nodeName` give the qualified name (uppercased only for HTML elements); `instanceof HTMLElement`/`HTMLSpanElement`/`HTMLUnknownElement`/`SVGElement` go by namespace (an HTML `<title>` was an `SVGElement`). Also: `setAttributeNS`/`getAttributeNS`/`hasAttributeNS`/`removeAttributeNS` keep attribute namespaces (`Dom::attr_ns`, `src/ns.js`); attribute names are lowercased on HTML elements only (script-set SVG `viewBox` now works); `getElementsByTagName` matches qualified names; `getElementsByTagNameNS`, `lookupNamespaceURI`, `lookupPrefix`, `isDefaultNamespace`. WPT `dom/nodes`: 1543 → 2166 of 5629, no subtest worse; `Document-createElementNS.html` passes every HTML-document case (206/596).
**Left undone:** the XML/XHTML-document variants (390 of the createElementNS subtests, about 98 in `Document-createElement.html`) need XML documents (`document.implementation.createDocument`, XML iframes); there are no `Attr` nodes, so `createAttributeNS`/`setAttributeNodeNS` and most of `attributes.html` still fail; the HTML parser doesn't put `xlink:`/`xml:` attributes on SVG into their namespaces; two attributes with the same qualified name in different namespaces share one slot.
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
**Status:** done
**Result:** WPT `media-source` 214/339 → 240/299 (25 → 28 files with no result). Now all passing:
`mediasource-remove` (13 → 17 alone; one subtest sometimes fails in a full parallel run when the
test's own data download comes back empty), `-sequencemode-append-buffer` 0 → 3, `-avtracks` 0 → 4,
`-timestamp-offset` 10 → 15, `-activesourcebuffers` 3 → 8, `-sourcebuffer-mode` 4 → 6,
`-sourcebuffer-mode-timestamps` 0 → 2; `-buffered` 4 → 7, `-changetype` 6 → 7. Segments keep
their coded frames (`src/mse_frames.rs`): per-track buffered ranges, frame-level removal with
dependent frames, MP4/WebM segments rewritten without dropped frames, MP4 edit lists applied,
`audio/aac` and `audio/mpeg` with generated timestamps, track lists, `srcObject = mediaSource`,
metadata as soon as the init segments are in, Chrome's quotas (150 MB, 12 MB for sound only).
**Left undone:** the three `mediasource-changetype-play*` files (3 of 43 subtests passed before)
now get past their duration checks and wait for real playback, so they give no result under
`--png` until X20; audio frames that straddle `appendWindowStart` are dropped, not trimmed; laced
WebM blocks count as one frame; `remove()` doesn't drop `readyState` to HAVE_METADATA;
`QuotaExceededError` as its own interface (the quota test checks `ex.constructor`) is missing.
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
**Status:** done
**Done:** (1) each shape's painted area (fill, stroke outline; text and images by their boxes) becomes hit strips after the `<svg>`'s own hit (`src/svg_hit.rs`), so `event.target`, hover, `:hover` and `cursor` reach the shape under the pointer; strips line up with the box so relayout reuse matches fresh layouts, and touching equal strips merge. (2) CSS animations and transitions of `fill`, `stroke`, `stroke-width`, opacities and `transform` on SVG elements reach the drawing (presentation attributes are the starting values); CSS `transform` values on SVG shapes are written in SVG syntax (`rotate(45deg)` and `translate(10px, 0)` were ignored before, even without animation). (3) `<foreignObject>` HTML is laid out in user units at its x/y/width/height, clipped, and drawn over the drawing through the viewBox and `transform`s (`src/svg_foreign.rs`); it gets clicks. Tests: `svg_hit::tests`, `svg_foreign::tests`, `svg_inline::tests::css_transforms_in_svg_syntax`; screenshot tests unchanged.
**Left undone:** shape hits come from the layout pass, so during an animation they follow the drawing only when the box is laid out again; `pointer-events` on shapes isn't read (always visiblePainted); nested `<svg>` viewports and percentage x/y/width/height on foreignObject are ignored; scaled foreignObject text is drawn through the off-screen transform path, so it's softer than unscaled text (as with CSS `scale()`).
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
**Status:** done
**Done:** (2) cache entries keep their own items, links and hits and refer to their cached children's entries (moved to where they sat) instead of copying them again at every level; a child is shared only when its output is still exactly as placed (a fingerprint of count, kinds, positions and text). The cache now holds about 1.2x the page's items instead of about 3x (test `nested_boxes_share_cached_items`). (1) layout keeps the last box tree with a hash of what it was built from (node data, structure, namespaces, picked files, style identities) and reuses it when nothing changed (test `unchanged_inputs_reuse_the_tree`). Measured on a generated 20,000-node page (24,019 nodes, 44,028 items): RSS after layout 76 → 73 MB, the cache's share about 10 → 6 MB; peak RSS of `--png` 170 → 163–165 MB (most of the peak is the 1024x16000 screenshot and its encode buffer, about 110 MB); React 56.6 → 56.8 MB and Bootstrap 41.0 → 41.5 MB (the same within noise). Unchanged relayout of the big page 14.8 → 9.7 ms; `relayout_timing` unchanged: Bootstrap 0.08 → 0.04 ms, grid 0.05 → 0.03 ms, full layouts about the same.
**Left undone:** the box tree is reused only when nothing it's made of changed; a DOM edit or restyle still rebuilds it in full (patching just the changed subtrees needs anonymous, marker and pseudo boxes rebuilt per subtree, and their ids are renumbered on every build). Keeping the last tree doesn't lower peak memory (it costs what the per-layout copy did), it saves the time. The `--png` peak could drop about 50 MB by encoding the screenshot without the whole-image RGB copy in `save_png`.
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
**Status:** done
**Done:** `IRONSWIFT_WAIT_FOR=<script expression>` makes `--png` run the page in real time (media and animations not frozen) until the expression is true, up to `IRONSWIFT_WAIT_SECS` (default 25), then runs `IRONSWIFT_WAIT_EXPIRED` and waits two more seconds (`Js::wait_for`). `tools/wpt.py` sets them for script tests (wait for the reporter's `__wptDone`, timeout − 5 s, then the harness's `timeout()` so finished subtests still count); reference tests stay frozen. `media-source`: 232/292 with 29 files giving no result → 282/424 and 284/424 in two runs, every file with a result. `html/semantics/embedded-content/media-elements`: 180/228 with 19 files giving no result → 224/285, every file with a result. No subtest worse in either (one `mediasource-sourcebuffer-mode` subtest failed once under load and passed in the rerun and alone).
**Left undone:** files that never finish now use most of the timeout (a `media-source` run takes about 2 minutes instead of 1¼); the media tests that time out (25 subtests in `media-source`) are failures in the media code, not in the runner.
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

## Optional additions (added by John 2026-11-17)

Build checklist §18. Each one picks up where the done task it names left off.

### O7. Claude connection: automated compatibility testing over MCP
**Status:** free (released 2026-11-18 04:45: chat-opus-x5 went quiet after 2026-11-17 17:33)
**Progress (2026-11-17, chat-opus-x5):** part 1 pushed in dd3eacf: `ironswift --mcp` server in `src/mcp.rs` (767 lines) and the `--mcp` flag in `src/main.rs`. Check which tools it has before going on; `docs/CLAUDE_CONNECTION.md` and the integration test aren't written yet.
**After:** —
**Checklist:** §18 "Claude connection for automated compatibility testing"
**Files:** new `src/automation.rs` (or `src/mcp/`), `src/main.rs` (the `--mcp` flag), `tools/site_check.py`, new `docs/CLAUDE_CONNECTION.md`
Let Claude drive Ironswift and find, test and fix compatibility gaps on its own.

1. `ironswift --mcp`: an MCP server over stdio (no network port, so nothing outside the computer
   can reach it), headless, with a fresh temporary profile. Tools:
   - `open_url`, `click`, `type`, `scroll`, `wait_for` (selector or idle)
   - `screenshot` (PNG), `compare_with_chrome` (Chrome's rendering + the 0-100 likeness score
     and a diff picture, reusing `site_check.py`'s code)
   - `script_errors` and `console_log`, `failed_requests` (status, blocked, unsupported)
   - `dom` (serialized tree or one element), `computed_style`, `box` (an element's layout box),
     `run_js` (evaluate in the page)
   - `unsupported_features`: every unimplemented JS API, CSS property or HTML element the page
     touched (log these from the bindings, the CSS parser and the tree builder)
   - `run_site_check` (all or some sites, returns grades) and `run_wpt` (a WPT directory, returns
     pass/fail counts and the failing subtests)
2. `docs/CLAUDE_CONNECTION.md`: how to add it to Claude Code (`claude mcp add ironswift --
   target/release/ironswift --mcp`) and the loop to follow: pick the lowest-graded site in
   `docs/site-compat.json`, open it next to Chrome, list script errors and unsupported features,
   make the smallest test case, fix it, re-run the site and the WPT areas touched, commit with the
   before and after likeness. One site or one feature per commit.
3. Tests: an integration test that starts `--mcp`, opens `tests/` pages and checks each tool's answer.

When it's done, John sets up a scheduled Claude task that runs the loop every night and writes a
summary of what it fixed and what got worse.

### O1. Web compatibility: every tracked site to "Works"
**Status:** free
**After:** O7, X1a, X1b, X1c, X1d, X1e
**Checklist:** §18 "Web compatibility"
**Files:** wherever the failing sites lead; `tools/sites.json`, `docs/SITE_COMPAT.md`
`docs/SITE_COMPAT.md` (2026-11-17): 15 works, 19 usable, 6 broken. Take the broken sites first,
then the usable ones, one site per commit, the same way as X1a–X1e. Re-run `tools/wpt.py` for the
areas you touch and keep every score the same or better. Add more sites to `tools/sites.json` as
the list gets to "Works".

### O2. Boa: further speed work
**Status:** free
**After:** X22
**Checklist:** §18 "Boa further optimization"
**Files:** `vendor/boa/` (log changes in `vendor/boa/IRONSWIFT.md`), `docs/boa-speed.md`
After X22's compiler work: inline caches and object shapes for property access, faster calls
and argument handling, string interning, cheaper garbage collection. Profile with
`tools/profile_js.sh`, measure each change with the full V8 suite and add the scores to
`docs/boa-speed.md`. No JIT here (that stays "not for chats").

### O3. Site isolation: close the gaps and turn it on by default
**Status:** done (chat-opus-t12 2026-11-17)
**Done:** everything T05 left undone now works on pages in site processes. The process sends what the window needs and the window does the rest (`frames.rs` Event/Update): find in page, text selection and copy (text runs); pushState / `#fragment` links / Back-Forward (history ops, origin-checked); `<select>` lists; the right-click menu (after the page's contextmenu, with the link and image under the mouse); developer tools and the console (`frames::site_ask`); print and Save as PDF (sheets laid out in the process); the accessibility tree and screen-reader actions (`a11y_remote.rs`); autofill and saved logins (the store stays in the browser: `autofill::FieldInfo` / `Submitted` / `Event::Fill`); pictures through shared memory (`shm.rs`); plain GETs through the browser's disk cache and big bodies streamed in pieces. Tested under Xvfb in the real window for each, plus unit tests for every new message. Then X15.
**Left undone:** Ctrl+A goes to the page (select all from the menu works); copying from a text field inside a site-process page; selection and find highlights assume the page's fonts measure the same in the browser (web fonts can shift highlights slightly); the console form in about:devtools doesn't submit on Enter (also for local pages; the Run button works); frame processes (cross-site iframes) still get bodies whole and send pictures over the pipe.
**After:** —
**Checklist:** §18 "Sandboxing and site isolation"
**Files:** `src/process.rs`, `src/sandbox.rs`, `src/main.rs`
Make everything T05 left undone work in a site process: find in page, text selection and copy,
the page context menu, devtools, `<select>` dropdowns, autofill, printing, the accessibility
tree, pushState address changes; shared memory for pictures; streamed and cached responses.
Then do X15 (default on).

### O4. WebGL: finish WebGL 2
**Status:** done
**Left undone:** drawing on the GPU — measured and planned in `docs/WEBGL_GPU.md` (full-screen 1280×720 textured pass: 77 ms in software on 2 threads), not built: step 1 there (VM → WGSL translator checked by naga) is the next task; real multisample antialiasing (multisampled renderbuffers draw single-sampled, context reports antialias: false); BPTC and ASTC compressed formats; signed RGTC disagrees with conformance/extensions/s3tc-and-rgtc.html; transform feedback from PBOs and after context loss (8 checks); readPixels with the implementation read format (RGB10_A2 → UNSIGNED_INT_2_10_10_10_REV); the remaining textures/misc validation pages (tex-new-formats, tex-storage-2d/3d limits, copy-texture-image checks); getFramebufferAttachmentParameter edge cases; GLSL compile-error cases in glsl3/ (forbidden-operators, invalid-invariant, misplaced-version-directive …).
**Result:** `tools/khronos_webgl.py` runs the Khronos conformance suite (test profiles turn anti-fingerprinting off: its readPixels noise fails exact-color checks). conformance2 without the texture folder, same settings before and after: 126/258 pages, 17694/24025 checks → 173/258, 23898/24375 (same 258 pages; the check count moves as pages stop early or don't). conformance2/textures: 220/926 pages → 861/926. Done: sampler/texture type checks at draw, attribute base-type checks, aliased attribute locations, enable-flag and readPixels pack checks, uniform/vertexAttrib range checks; occlusion queries and fence syncs (query 579/579, sync 14/14); transform feedback (479/487: PBO and lost-context cases left); draw buffers and OES_draw_buffers_indexed (per-buffer blend and color mask, missing-output check; draw-buffers 229/229, oes-draw-buffers-indexed 1605/1605); sRGB formats (encode on render/clear, decode on sampling, blending, copies and mipmaps) and a rewritten blitFramebuffer (LINEAR filter, depth/stencil, full validation; blit and sRGB pages 5/14 → 12/14, the other 2 need multisampling); multisampled renderbuffers (4 samples reported, drawn single-sampled; completeness, readPixels and blit-resolve rules; real antialiasing isn't done, so getContextAttributes still says antialias: false); pixel buffers (readPixels into PIXEL_PACK_BUFFER, tex(Sub)Image2D/3D from PIXEL_UNPACK_BUFFER, WebGL 2 pack/unpack parameter rules, reads from a 3D/array layer); video and DOM-source uploads (a <video>'s current frame, WebGL 2 source sub-rectangles from UNPACK_SKIP_* with flip-Y applied first, unpackColorSpace "display-p3"; textures/video 68/68, image 68/68); ImageBitmap uploads (createImageBitmap options and rectangles, bitmaps from video, WebGL ignoring flip/premultiply for bitmaps); compressed formats (ETC1, ETC2/EAC and RGTC decoders in src/webgl_etc.rs, compressed uploads from pixel buffers and shared memory, 2D-array compressed textures, block-size rules; conformance/extensions etc, etc1, s3tc-srgb pass; signed RGTC disagrees with s3tc-and-rgtc.html; BPTC and ASTC not offered); render-feedback loops, attached-level completeness, immutable base/max clamping, generateMipmap format rules, wide points clipped by depth only (lines stay 1 px wide: ALIASED_LINE_WIDTH_RANGE is [1, 1], as in Chrome and Firefox). GPU study: `docs/WEBGL_GPU.md`.
**After:** —
**Checklist:** §18 "WebGL"
**Files:** `src/webgl*.rs`, `src/glsl*.rs`, `src/gpu.rs`, `webgl.js`
From T12's left-undone list: antialiasing/multisampling, transform feedback, queries and syncs,
pixel buffers, video as a texture, more compressed texture formats, wide lines, sRGB. Run the
Khronos WebGL conformance suite and record the score. Look at drawing on the GPU instead of in
software.

### O4a. WebGL on the GPU, step 1: shader translator
**Status:** free
**After:** O4
**Checklist:** — (follow-up)
**Files:** new `src/glsl_wgsl.rs`, `src/glsl_vm.rs` (read-only use), tests
Step 1 of `docs/WEBGL_GPU.md`: translate compiled VM programs (`glsl_vm.rs`) to WGSL and check
each with naga's WGSL front end and validator (already in the build through wgpu). Add a tool or
test that translates every shader pair in the Khronos suite and reports how many validate;
target 95% of the conformance pages' programs. No GPU needed. Steps 2–5 in the doc follow.

### O5. WebRTC
**Status:** free
**After:** —
**Checklist:** §18 "WebRTC"
**Files:** new `src/webrtc/`, a new JS file
`getUserMedia` for camera and microphone (the permission prompts already exist),
`RTCPeerConnection` with ICE/STUN/TURN, DTLS-SRTP audio and video, and data channels. Use a
Rust crate such as `webrtc-rs` rather than writing the protocols by hand. Test with a two-tab
video call page and a public test site.

### O6. Extensions: the rest of the WebExtensions API
**Status:** free
**After:** —
**Checklist:** §18 "Extensions"
**Files:** `src/extensions/`
From T13's left-undone list: stable tab ids, `declarativeNetRequest`, the other `webRequest`
events, `runtime.connect` ports, `alarms`, `contextMenus`, toolbar buttons and popups, an
about:extensions page and installing packed extensions (.xpi/.crx). Goal: uBlock Origin and Dark
Reader run unmodified.

### O8. Theming: light, dark and follow the system
**Status:** free
**After:** —
**Checklist:** §18 "Theming"
**Files:** `src/settings.rs`, `src/chrome.rs` (browser UI colors), `src/css.rs` and `src/prelude.js` (`prefers-color-scheme`, now always light), `src/style.rs` (default `color-scheme`), `src/start.html`, the settings page, internal pages (error pages, downloads, history, devtools)
A **Theme** setting with three choices: **System** (the default), **Light** and **Dark**.

- System follows the desktop's light/dark preference through the XDG desktop portal
  (`org.freedesktop.appearance` `color-scheme`, which GNOME and KDE both set), falling back to
  the GTK theme name, and switches live when the desktop changes, with no restart.
- The whole browser UI follows it: tab strip, toolbar, address bar and its dropdown, menus,
  dialogs, prompts, panels, the settings page, the start page, error pages and devtools. Keep
  every color in one palette table per theme (no colors scattered through drawing code), and
  check text contrast with `src/contrast.rs` (4.5:1 or better).
- Pages see it too: `prefers-color-scheme` in CSS and in `matchMedia` answers from the theme,
  pages that declare `color-scheme: light dark` get dark form controls and scrollbars, and
  `matchMedia` listeners fire when it changes. Pages that don't ask for dark stay as they are.
- High contrast (forced colors) still wins over the theme.
- Screenshot tests for the browser UI and the start page in both themes; a test page with
  `prefers-color-scheme` rules.

### O9. Android version
**Status:** free
**After:** —
**Checklist:** §18 "Android"
**Files:** `Cargo.toml`, `#[cfg]` gates, `src/platform.rs`, `src/media.rs`, `src/sysfonts.rs`, new `mobile/android/`, `.github/workflows/`
Doesn't wait for X21, but X21 is also gating Linux-only code: pull first and use its
`cfg(target_os = "linux")` gates rather than adding a second set.

- Build for Android with winit's `android-activity` and wgpu (Vulkan, GLES fallback); package
  an APK/AAB with `cargo-apk` or `xbuild`, signed, built in CI.
- Android versions of the Linux-only parts: fonts from `/system/fonts` and `fonts.xml` instead of
  fontconfig; video and audio through MediaCodec/AAudio (the NDK) instead of the ffmpeg program;
  location, notifications and permission prompts through Android's APIs; the accessibility tree
  through AccessKit's Android backend; per-site processes as isolated services, keeping seccomp.
- A touch UI: address bar at the top or bottom (a setting), tab switcher, tap targets of 48 dp,
  pinch-zoom, scrolling with momentum, pull to refresh, long-press menus, the on-screen keyboard
  without covering the focused field, and the back button/gesture.
- Mobile page behavior: honor `<meta name="viewport">`, send a mobile user agent, touch and
  pointer events, `device-width` media queries.
- Test on an emulator in CI (screenshot tests) and on one real phone; run `tools/site_check.py`
  with a mobile viewport.

## Features the major browsers have (added by John 2026-11-17)

Build checklist §19 (site features) and §20 (browser features). Site features come first:
sites break without them. Run `tools/site_check.py` and the matching WPT directories before and
after each one.

### P1. WebAssembly (split into P1a–P1d below)
**Status:** split: P1a–P1d
**Checklist:** §9 "WebAssembly" (moved from §19 to the standard browser on 2026-11-18); ticked when P1d is done

### P1a. WebAssembly: pick the runtime and run modules
**Status:** free
**After:** —
**Checklist:** none (P1d closes §9 "WebAssembly")
**Files:** new `src/wasm/`, `src/js.rs` (the `WebAssembly` global), a new `src/wasm.js` if glue is easier in JS
Embed an existing Rust runtime rather than writing one: measure `wasmtime` (Cranelift) against
`wasmi` on startup time, binary size, memory and run speed (a CPU-heavy module such as a
compression or image library), pick one and write down why in `docs/ARCHITECTURE.md`.
Then `WebAssembly.Module`, `Instance`, `compile`, `instantiate`, `validate`, `CompileError`,
`LinkError`, `RuntimeError`, with imported and exported functions and numbers (i32, i64 as
BigInt, f32, f64) passed to and from Boa. Runaway-script limits must also stop a looping module.
Push in stages; P1b and P1c start once `instantiate` and function calls work.

### P1b. WebAssembly: Memory, Table and Global
**Status:** free
**After:** P1a
**Checklist:** none
**Files:** `src/wasm/`
`WebAssembly.Memory` (its `buffer` is an `ArrayBuffer` over the module's memory, `grow()` detaches
the old one; shared memory as `SharedArrayBuffer` where cross-origin isolation allows),
`WebAssembly.Table` (funcref/externref, `get`/`set`/`grow`) and `WebAssembly.Global`, imported and
exported both ways. Typed arrays over `memory.buffer` must see writes made by the module.

### P1c. WebAssembly: streaming, modules and security
**Status:** free
**After:** P1a
**Checklist:** none
**Files:** `src/wasm/`, the fetch and module-loading code
`compileStreaming`/`instantiateStreaming` from a `Response` or fetch promise (checks the
`application/wasm` type, compiles as bytes arrive), `import` of `.wasm` files in ES modules, the
CSP `wasm-unsafe-eval` rule, WebAssembly in Web Workers, and in site processes when Site isolation
is on. `postMessage` of a `WebAssembly.Module` to a worker.

### P1d. WebAssembly: conformance and real apps
**Status:** free
**After:** P1b, P1c
**Checklist:** §9 "WebAssembly"
**Files:** `tools/` (a spec-test runner), whatever the fixes need
Run WPT `wasm/jsapi` (and `wasm/webapi`) and the WebAssembly spec test suite (`.wast` files;
write a small runner) and record the scores in `docs/CONFORMANCE.md`. Check that ffmpeg.wasm, an
Emscripten/Unity WebGL game, a Figma-style canvas demo and SQLite in the browser (sql.js) load and
run; add a small test page in `tests/sites/` for each problem fixed.

### P2. Service workers, offline pages, push and installable web apps
**Status:** free
**After:** —
**Checklist:** §19 "Service workers"
**Files:** new `src/serviceworker.rs`, `src/worker.rs`, `src/net.rs`, `src/cache.rs`
`navigator.serviceWorker` registration, lifecycle (install, activate, update, skipWaiting,
clients.claim), `fetch` interception, the Cache API (`caches`), `clients`, and offline loading.
Then Web Push (`PushManager`, VAPID, delivery through a push service while the browser runs) with
the existing notification prompt, and installing a site as an app from its web app manifest (its
own window and desktop entry). Partition by site like storage. Run WPT `service-workers/`.

### P3. `<dialog>` and popovers
**Status:** done (chat-opus-t12 2026-11-18)
**Done:** the top layer (`src/dialog.rs`, `Dom::top_layer`) with `:modal`, `:popover-open`, `:open`, a `::backdrop` pseudo-element behind modal dialogs, and `height: fit-content` for centering; `src/dialog.js`: `show`/`showModal`/`close`/`requestClose`, `returnValue`, `closedBy`, cancel/close and beforetoggle/toggle (`ToggleEvent`) events, `<form method=dialog>` (submit events now carry `event.submitter`), Esc, focus in and back, the page inert behind a modal; the `popover` attribute and API, `popovertarget`/`popovertargetaction`, light dismiss. Mouse buttons over empty space now go to the root element. Tested in the real window (site process) and `dialogs_and_popovers`. WPT `the-dialog-element` 68/446 subtests and 10/45 reference pages; `popovers` 1225/3899 and 7/19 (nothing before).
**Left undone:** most remaining WPT failures use test_driver (real key presses and clicks) or iframes of other sizes; popover anchoring (`position-anchor`, `hint` stacking rules), nested-popover ancestry beyond containment, and `inert` on other elements aren't done; focus isn't trapped inside a modal when tabbing.
**After:** —
**Checklist:** §19 "Dialog and popover"
**Files:** `src/dom_classes.rs`, `src/prelude.js`, `src/layout.rs`, `src/paint.rs`
`HTMLDialogElement` (`show`, `showModal`, `close`, `returnValue`, `<form method=dialog>`, Esc,
`::backdrop`, inert page behind a modal), the popover attribute and API (`popover`,
`popovertarget`, `showPopover`, `togglePopover`, light dismiss, `:popover-open`) and the top
layer they share. Run WPT `html/semantics/interactive-elements/the-dialog-element` and `html/semantics/popovers`.

### P4. Fullscreen and picture-in-picture
**Status:** free
**After:** —
**Checklist:** §19 "Fullscreen and picture-in-picture"
**Files:** `src/main.rs`, `src/window.rs`, `src/media.rs`, `src/media.js`
`element.requestFullscreen()`, `document.exitFullscreen()`, `:fullscreen`, `fullscreenchange`,
a "press Esc to exit" notice, and F11 for the whole window. Picture-in-picture for `<video>`
(`requestPictureInPicture`, now a rejected promise) in a small always-on-top window, plus a PiP
button on video controls.

### P5. Newer CSS and page features
**Status:** free
**After:** —
**Checklist:** §19 "Newer CSS and page features"
**Files:** `src/css.rs`, `src/style.rs`, `src/layout.rs`, `src/anim.rs`, `src/prelude.js`
CSS anchor positioning (`anchor-name`, `position-anchor`, `anchor()`, `position-try`), view
transitions (same-document `startViewTransition` and cross-document), scroll-driven animations
(`animation-timeline`, `scroll()`, `view()`), the Navigation API (`navigation`, `navigate` event),
customizable `<select>`, `field-sizing`, `text-box`, and the other features that reached Baseline
in 2026. One feature per commit, each with its WPT directory.

### P6. Passkeys (WebAuthn)
**Status:** free
**After:** —
**Checklist:** §19 "Passkeys"
**Files:** new `src/webauthn.rs`, `src/autofill.rs`
`navigator.credentials.create/get` with `PublicKeyCredential`: passkeys stored in the profile
(encrypted, alongside saved passwords), USB security keys over CTAP2 (`ctap-hid-fido2` or
`authenticator` crate), and the account picker. Test against webauthn.io.

### P7. WebGPU
**Status:** free
**After:** —
**Checklist:** §19 "WebGPU"
**Files:** new `src/webgpu.rs`, a new JS file, `src/gpu.rs`
`navigator.gpu` on top of `wgpu` (already used for drawing), with WGSL through `naga`. Run the
WebGPU conformance suite (CTS) subsets that fit; check three.js's WebGPU renderer.

### P8. Smaller site APIs
**Status:** free
**After:** —
**Checklist:** §19 "Speech, game controllers, share, payments"
**Files:** new small modules as needed
Speech synthesis (`speechSynthesis` through speech-dispatcher) and speech recognition (a local
model such as Vosk or Whisper, opt-in); the Gamepad API (`gilrs`); `navigator.share` through the
desktop portal; `PaymentRequest` reporting no methods cleanly so sites fall back.

### B1. Sync across devices
**Status:** free
**After:** —
**Checklist:** §20 "Sync"
**Files:** new `src/sync/`, new `tools/sync-server/`
Sync bookmarks, history, saved passwords, open tabs and settings between computers. End-to-end
encrypted (the server never sees the data; the key comes from a passphrase), a small server that
can be self-hosted, and an account page in settings. Where the public server runs is John's call:
build it so the address is a setting.

### B2. Reader mode
**Status:** free
**After:** —
**Checklist:** §20 "Reader mode"
**Files:** new `src/reader.rs`, a reader page
An address-bar button on article pages that shows just the text and pictures: font, size,
width and theme controls, estimated reading time, and read-aloud once P8's speech is in.

### B3. Page translation
**Status:** free
**After:** —
**Checklist:** §20 "Page translation"
**Files:** new `src/translate/`
Translate a page on the computer, never sending it anywhere, like Firefox: Bergamot/Marian
models downloaded per language pair on first use, translating text nodes in place and new text as
it appears. Offer it when a page's language isn't the browser's.

### B4. Spellcheck
**Status:** free
**After:** —
**Checklist:** §20 "Spellcheck"
**Files:** `src/textedit.rs`, `src/editing.rs`, new `src/spell.rs`
Wavy red underlines in text fields and contenteditable, suggestions in the right-click menu,
"add to dictionary", using Hunspell dictionaries (system ones, or downloaded), honoring the
`spellcheck` attribute and the field's language.

### B5. Tab groups, vertical tabs and split view
**Status:** free
**After:** —
**Checklist:** §20 "Tab management"
**Files:** `src/chrome.rs`, `src/main.rs`, session restore
Named, colored tab groups that collapse and are kept in session restore; a vertical tab list
option; two pages side by side in one window; a tab search (Ctrl+Shift+A).

### B6. Password generator and breach alerts
**Status:** free
**After:** —
**Checklist:** §20 "Password extras"
**Files:** `src/autofill.rs`, the passwords page
Suggest a strong password in new-password fields and save it; a passwords page that flags
reused and weak passwords and checks for breaches with Have I Been Pwned's range API (only the
first 5 characters of the hash leave the computer); import from and export to CSV.

### B7. Screenshot tool, reading list and side panel
**Status:** free
**After:** —
**Checklist:** §20 "Screenshots, reading list, side panel"
**Files:** `src/chrome.rs`, `src/main.rs`
A screenshot tool (visible area, full page, or a chosen region, saved or copied); a reading list
(save for later, marked read); a side panel that holds bookmarks, history, the reading list and a
second page.

### B8. Encrypted DNS (DNS over HTTPS)
**Status:** free
**After:** —
**Checklist:** §20 "Encrypted DNS"
**Files:** `src/net.rs`, `src/settings.rs`
A DNS setting: system (default), or DNS over HTTPS with a choice of providers or a custom
address, falling back to the system when it fails, unless set to strict.

### B9. iOS version
**Status:** free
**After:** O9, X21
**Checklist:** §20 "iOS version"
**Files:** `Cargo.toml`, `src/platform.rs`, `mobile/`
Reuse O9's touch UI. Apple allows browsers with their own engine only in the EU, so check
Apple's terms first and write down what they require before building.

## Not for the chats

### X21. Windows and macOS builds (from batch J)
**Status:** Windows done for now (main-chat, 2026-11-17 19:10); macOS builds and passes its
checks but its real window hasn't been tried.
**Result:** CI builds, tests (336 pass) and opens the real window on Windows every push
(`tools/windows_smoke.ps1`): the page draws, Tab and typing fill a form field, Ctrl+T opens a tab
and typing goes to the address bar, Ctrl+W closes it, clicks work. Screenshots of that run:
`docs/windows-window-test.png`. Windows has its own profile folder (%APPDATA%), fonts (DejaVu
shipped, else Arial/Consolas), file chooser, printing, default-browser registration (the user
picks Ironswift in Default apps), toast notifications, location (Windows location service),
`file:` URLs, and a release zip; no self-update outside Linux.
**Known gaps (new tasks if wanted):** screen readers on Windows (AccessKit's Windows adapter via
accesskit_winit); page processes on Windows aren't locked down (job objects / AppContainer); the
first window can be larger than a small screen (1040 px wide on a 1024 px screen); on machines
with no graphics card it draws in software (fine, slower). macOS: real window untested, no
signing (needs an Apple developer account).

### X22. Boa: bytecode compiler emits fewer instructions (from batch M)
**Status:** done (main-chat, 2026-11-17 19:40). Property reads on locals, `local op= simple`
and `return` emit fewer instructions (a summing loop: 11 → 7 per iteration); such loops run 27%
faster, V8 suite unchanged within noise. Details in `vendor/boa/IRONSWIFT.md`.
**After:** X21
**Checklist:** §14 Bytecode compiler optimizations
**Files:** `vendor/boa/boa_engine/src/bytecompiler`
boa-chat stopped after compile speed. Make the compiler emit fewer, cheaper instructions
(constant folding, fused compare-and-jump, fewer register moves) and measure with the V8 harness.

### DRM (Widevine)
**Status:** not for chats: needs a license agreement with Google; John decides.
**Checklist:** §7 "DRM (Widevine) for Netflix and similar"

### Full (optimizing) JIT for Boa
**Status:** not for chats: optional, only after the whole app is built (John's decision).
**Checklist:** §18 "Full (optimizing) JIT compiler for Boa"

### J1. Baseline JIT for Boa: plan and groundwork
**Status:** free
**After:** —
**Checklist:** none (J4 closes §14 "Baseline JIT compiler (via Cranelift)")
**Files:** `vendor/boa/boa_engine/src/vm/`, new `vendor/boa/boa_engine/src/jit/`, `docs/JIT.md`
Released for chats 2026-11-18 (the full JIT stays optional). Write `docs/JIT.md`: which bytecode
opcodes the first version compiles, when a function tiers up (call and loop counters in the VM),
how compiled code falls back to the interpreter mid-function (bail out with the frame's registers),
how GC roots in compiled frames are found, and executable memory that is never writable and
executable at once (W^X). Add the counters and a `boa_jit` cargo feature, off by default, with
Cranelift (`cranelift-codegen`, `cranelift-frontend`, `cranelift-jit`) behind it. If P1a picked
`wasmtime`, share its Cranelift version.

### J2. Baseline JIT: numbers, locals, branches and loops
**Status:** free
**After:** J1
**Checklist:** none
**Files:** `vendor/boa/boa_engine/src/jit/`
Compile functions that only use locals, number arithmetic, comparisons, branches and loops to
machine code; anything else (or a value that isn't a number) bails out to the interpreter. Same
results as the interpreter on every test262 and Boa test with the JIT forced on for every
function (`BOA_JIT=always`). Measure the summing-loop and V8 suite in `docs/boa-speed.md`.

### J3. Baseline JIT: properties, calls and exceptions
**Status:** free
**After:** J2
**Checklist:** none
**Files:** `vendor/boa/boa_engine/src/jit/`, `fuzz/`
Property reads and writes (calling into the VM's existing paths), function calls in both
directions, `try`/`catch`/`finally` and generators/async functions either compiled or left to
the interpreter. Add a fuzz target that runs each program with and without the JIT and fails on
any difference.

### J4. Baseline JIT: on by default
**Status:** free
**After:** J3
**Checklist:** §14 "Baseline JIT compiler (via Cranelift)"
**Files:** `vendor/boa/`, `Cargo.toml`, `src/settings.rs`
x86-64 and ARM64, Linux and Windows (and macOS if CI allows); a Settings switch "JavaScript JIT"
(on by default) for the cautious; test262, WPT and the site check unchanged; record the speedups.
Note the security review points in `SECURITY.md`.

### Live-site check on John's computer
**Status:** not for chats: John's to do (needs open internet).
**Checklist:** §1 "Live-site check"

### Delete the site-recording branches
**Status:** not for chats: John's to do (the chats' git access can't delete branches).
**Checklist:** none
Delete these branches on GitHub (repository → Branches, or `git push origin --delete <name>`):
`x1-record`, `x1-record2`, `x1-data`, `rec-x1`, `rec-x1e`, and each `rec-x1a`…`rec-x1d` once its
task is done (the helper chats list theirs in their reports). Keep `x1-data2` until X1a–X1d are
all done: their recordings are in it.

## Still with their batch chats (see `docs/BATCHES.md`)

- Batch J: §16 Windows and macOS builds
- Batch K: §4 shorthand expansion; §5 grid extras; §5 exact `height` on overflowing boxes
- Batch M: §14 Boa bytecode compiler optimizations
