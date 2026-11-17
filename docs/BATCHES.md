# Work batches

The remaining build-checklist tasks, split into batches so up to 10 chats can work at once
without editing the same code. Each chat takes **one batch at a time**.

**How to claim a batch**

1. `git pull --rebase origin main`, then read this file.
2. Take the **first** batch whose Status is `free` and whose "Start after" batches are `done`.
3. Change its Status to `claimed by <your chat name> <date time>`, commit only this file,
   and push to `main`.
4. If the push is rejected, someone else pushed first: `git pull --rebase origin main`, read
   this file again, and if your batch is now claimed by another chat, pick the next free one.
   Never force-push.
5. When the batch is finished (code pushed, tests passing), set its Status to `done` and list
   what is left undone, if anything, under it. Then claim the next batch.

Status values: `free`, `claimed by …`, `done`, `blocked: <reason>`.

The full task wording is in the build checklist doc ("Ironswift — Build Checklist",
https://claude.ai/artifact/Ff1b9Caj8jzoSPjf7JhrC1). Section numbers below match it.

---

## A. Merge the install packages and crash fixes
**Status:** done
**Left over:** John must turn on GitHub Pages once (repo Settings → Pages → Source: "GitHub
Actions"); then the site publishes at https://hirejohndougan-art.github.io/lighteningfox/. The
Flatpak manifest couldn't be built here (no flatpak-builder or Flathub access); the release
workflow builds it on GitHub, so check that job on the first tagged release. A Flathub listing
would need a from-source build with vendored crates.
**Start after:** —
**Main files:** `packaging/`, `.github/workflows/`, `fuzz/`, `src/treebuilder.rs`, `src/main.rs` (stack size only)

- Pull request #1 (`parallel-work` branch) conflicts with `main`. Bring over only what `main`
  lacks: the `.deb`, AppImage and `.tar.gz` packaging with its release steps, and the
  deep-nesting crash fixes (HTML nesting limit of 512, script nesting limit, larger stack).
  Drop its ad blocker and fuzz targets, which duplicate what `main` already has. Merge the two
  release workflows into one. Then close the pull request.
- §16 Install packages: add a Flatpak.
- §16 Site-compatibility tracker: a list of popular sites and how well each works
  (build on `tools/site_check.py`).
- §16 Website and download page (a GitHub Pages site in `site/`).

## B. Text and fonts (all of §6)
**Status:** claimed by security-chat 2026-11-17 02:16
**Start after:** —
**Main files:** `src/font.rs`, text parts of `src/layout.rs` and `src/paint.rs`

- Find system fonts by name (fontconfig) and honor `font-family` lists
- Unicode line breaking (UAX #14), hyphenation, `word-break`, `overflow-wrap`
- `letter-spacing`, `word-spacing`, `text-overflow: ellipsis`
- Text decoration thickness and line styles (wavy, dotted)
- Subpixel positioning and hinting for sharper text on low-resolution screens
- Color emoji
- Vertical text for East Asian layouts (last; may be left for later if it needs the box tree)

## C. Images
**Status:** done (images-chat, 2026-11-17)
Left for later: animated AVIF (still AVIF works); `<svg>` written inline in a page (SVG
*images* work; inline SVG elements are still hidden); a picture inside an SVG image that
points at another file isn't loaded (browsers don't either).
**Start after:** —
**Main files:** `src/images.rs`, image parts of `src/paint.rs` and `src/layout.rs`

- §7 AVIF and SVG images
- §7 `object-fit` and `object-position`
- §7 Conic and repeating gradients; multiple background layers
- §7 Lazy loading (`loading=lazy`) and responsive images (`srcset`, `<picture>`)
- §7 Animated GIF and WebP
- §7 Image memory budget: evict decoded images that are off-screen or on closed pages
- §14 Decode images at their display size, not full resolution
- §14 Drop off-screen images and decode them again when they scroll back into view

## D. Canvas and drawing
**Status:** claimed by layout-chat 2026-11-17 02:15
**Start after:** —
**Main files:** new `src/canvas.rs`, a new prelude file for the canvas API, `src/paint.rs` (shapes)

- §7 `<canvas>` 2D drawing API
- §8 Anti-aliased shapes and paths (tiny-skia)
- §13 Fingerprinting: per-site noise on canvas readouts (the noise function already exists)

## E. Effects and animation
**Status:** done (effects-chat 2026-11-17)
**Start after:** —
**Main files:** `src/paint.rs`, `src/tiles.rs`, `src/gpu.rs`, the drawing and scrolling parts of `src/main.rs`

- §8 Text shadows
- §8 Filters (`blur`, `grayscale`, …) and `backdrop-filter`
- §8 CSS transitions and animations (`@keyframes`)
- §8 Only repaint damaged regions
- §8 HiDPI screens
- §8 Smooth scrolling at 60 frames per second

Left undone:
- `transitionend` / `animationend` events aren't fired yet.
- HiDPI scales the page but not the browser's own tabs and toolbar; that needs winit's scale-change events (batch J).
- A running animation restyles and lays out the whole page each frame (only repainting is incremental); incremental restyle/relayout is batch K / N.
- `filter` applies to block boxes, not inline elements (spans); `scroll-behavior: smooth` isn't supported.

## F. Forms and iframes
**Status:** claimed by forms-chat 2026-11-17 02:22
**Start after:** —
**Main files:** `src/forms.rs`, `src/textedit.rs`, form parts of `src/main.rs`

- §10 Date, color and file pickers
- §10 Multipart form uploads (file inputs)
- §10 Validation (`required`, `pattern`, `min`, `max`) and error messages
- §10 `contenteditable` (rich text editors)
- §10 Autofill and saved passwords
- §10 Keyboard input methods for other languages (IME) — may be `blocked` until batch J moves to winit
- §3 Clicking and scrolling inside an `<iframe>` — moved to batch F2 below

## F2. Clicking and scrolling inside iframes (split from F)
**Status:** claimed by iframes-chat 2026-11-17 03:24
**Start after:** —
**Main files:** the iframe parts of `src/main.rs` (`render_frames`, `render_frame`, `draw_frame`, `isolated`), new `src/frames.rs`, `src/sandbox.rs` only if needed

- §3 Clicking and scrolling inside an `<iframe>`: frames are drawn today as still pictures by the
  frame worker thread (same-site) or the `--frame-renderer` process (cross-site). Keep each
  frame's document alive, send it mouse clicks, wheel scrolling and keyboard input, and redraw
  it when it changes, for both kinds. Batch F (forms) keeps working on forms in `main.rs`;
  keep iframe code in new files where possible.

## G. JavaScript APIs, part 1
**Status:** done
**Start after:** —
**Main files:** `src/js.rs`, `src/prelude.js` (add new APIs in new `src/*.js` files where possible)

- §9 ES modules (`<script type=module>`, `import`)
- §9 Custom elements (web components)
- §9 `IntersectionObserver`, `ResizeObserver`
- §9 Keyboard, mouse, scroll, focus, input and form events from the user
- §9 Promises and `async`/`await` tied to network and timers (check what's missing; may already work)
- §9 Developer console: show errors and run JavaScript

Left undone in G:
- Import maps (`<script type=importmap>`): bare specifiers like `"react"` still fail to load. Needs its own small task.
- Custom elements: customized built-ins (`is="…"`, which Safari also lacks), the `:defined` CSS selector (style.rs, batch K's file), `adoptedCallback`, and ElementInternals beyond the basics (ARIA, form validation). WPT custom-elements: 156/745 subtests. Most of the rest are ElementInternals ARIA tests and tests that use iframes.
- The browser still fires `click` on mouse press, so the order is mousedown → click → mouseup. Moving click to release would change how every page click behaves, so I left it. `dblclick` and `contextmenu` aren't fired yet.
- Developer console: it runs in the about:devtools snapshot page. There's no live-updating log, no autocomplete and no docked panel.

## H. JavaScript APIs, part 2
**Status:** claimed by netapi-chat 2026-11-17 02:22
**Start after:** —
**Main files:** new `src/*.js` files, `src/js.rs` natives, `src/net.rs`

- §9 `WebSocket`
- §9 Web Workers on separate threads
- §9 Clipboard, notifications and geolocation, with permission prompts (prompts already exist from §13)

## I. Accessibility and languages (all of §15)
**Status:** claimed by a11y-chat 2026-11-17 02:30
**Split (two chats ended up on this batch):** a11y-chat does the screen-reader bridge
(AT-SPI/AccessKit), high contrast (pages and the browser's own UI) and translating the rest of
about:settings. The other chat (already did the tree, Intl, translated UI and page keyboard
navigation) does the right-to-left browser UI and keyboard control of the browser's toolbar,
tabs, menus and panels. a11y-chat marks the batch done and updates the checklist when both are finished.

- Accessibility tree built from the DOM and ARIA attributes
- Expose it to Linux screen readers through AT-SPI (AccessKit)
- Full keyboard navigation of pages and browser UI
- High-contrast setting
- Translated browser UI (Fluent localization files)
- `Intl` APIs in JavaScript (dates, numbers, sorting by language)
- Right-to-left browser UI

## J. Windowing and platforms
**Status:** claimed by effects-chat 2026-11-17 03:20
**Start after:** A, E (both touch the window loop and builds)
**Main files:** `src/main.rs` window setup, `src/gpu.rs`, `Cargo.toml`, CI

- §11 Move to winit for keyboard shortcuts, clipboard and Wayland
- §16 Native Wayland support
- §16 Windows and macOS builds

## K. CSS (§4)
**Status:** claimed by css-chat 2026-11-17 02:40 (the old section 4 chat switched to batch A work and finished)
**Start after:** —
**Main files:** `src/css.rs`, `src/style.rs`, `src/grid.rs` (grid extras)

- Spec-compliant CSS tokenizer (CSS Syntax Level 3)
- `:active`, `:visited`
- `::before`, `::after` with `content`, `::marker`, `::placeholder`
- `@import`, `@font-face`, `@supports`, `@layer`, `@container`
- Cascade layers and correct `inherit`, `initial`, `unset`, `revert`
- Shorthand expansion for every shorthand
- Selector matching speed: bloom filters, rule hashing by id/class/tag
- Restyle only what changed after a script edits the DOM
- §5 Grid extras: subgrid, `dense` packing, named grid lines, `justify-items`/`justify-self`
- §5 Exact `height` on boxes whose content overflows

## L. Performance (our code)
**Status:** claimed by jsapi-chat 2026-11-17 03:35
**Start after:** C, G (they change the same DOM and image paths)
**Main files:** `src/js.rs`, `src/dom.rs`, `src/main.rs`, CI

- §14 Index elements by class and tag
- §14 Batch DOM changes so one restyle and relayout runs per frame
- §14 Memory budget for a 4 GB laptop: free closed tabs, cap caches
- §14 Startup time under 300 ms
- §14 Parallel styling and layout across CPU cores (Rayon)
- §14 Speed regression tests in CI
- §14 Mark the two ad-blocking items done if the existing EasyList blocking covers them

## M. Boa engine speed
**Status:** claimed by boa-chat 2026-11-17 02:20
**Note:** two chats ended up on this batch. To keep out of each other's files: boa-chat takes
parsing, compiling and bytecode caching (`vendor/boa/boa_parser`, `vendor/boa/boa_ast`,
`vendor/boa/boa_engine/src/bytecompiler`); the chat that added the V8 harness takes the runtime
items (strings, inline caches, shapes, GC: `boa_string`, `boa_gc`, `boa_engine/src/vm`, `object`).
**Start after:** —
**Main files:** a fork of Boa (vendored under `vendor/boa` or a git dependency), `tools/`

- §14 Profile Boa on real site scripts (perf, flamegraphs)
- §14 Inline caches for property lookups
- §14 Object shapes (hidden classes) tuning
- §14 Faster string handling (ropes for `+=`, interned atoms)
- §14 Bytecode compiler optimizations
- §14 Garbage collector tuning
- §14 Bytecode caching so repeat visits skip parsing
- §14 Track progress against Boa's V8-benchmark score
- Not in this batch: the baseline JIT (only after the whole app is built)

## N. Big restructurings (one chat at a time, after the others)
**Status:** free
**Start after:** B, C, D, E, F, G, H, I, K, L
**Main files:** most of `src/`

- §5 A real box tree (layout tree separate from the DOM)
- §5 Incremental relayout
- §13 Process isolation: each top-level site in its own sandboxed process
- §14 Shared process pool
- §14 Expose DOM objects as native Boa classes; generate bindings from WebIDL (also §9)

## Not for the chats

- §1 Live-site check: needs John's machine (`python3 tools/site_check.py`).
- §7 `<video>`/`<audio>`, Media Source Extensions, DRM, WebGL; §11 extensions;
  §16 funding plan; the Boa JIT: later, by decision.
