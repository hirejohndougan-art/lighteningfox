# Swiftiron architecture

Swiftiron is one Rust program (about 15,000 lines) that does every stage of showing a web page
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

## The window (`main.rs`)

`run_window` owns everything on screen and loops at up to 60 frames a second (12 when idle):

- **Tabs**: the visible tab's state lives in local variables; background tabs are `Slot`s holding a
  `Saved` copy (page, DOM, scroll, history). Tabs not seen for a while are put to sleep (their page
  is freed and reloads when shown). The open tabs are saved every second for session restore.
- **Input**: keys go through `edit_keys` for the address bar and form fields (`textedit.rs`);
  clicks are hit-tested against the layout and dispatched to the page's JavaScript first.
- **Navigation**: `Go::To/Back/Forward/Reload`, same-document moves (`#fragment`, `pushState`) and
  form submission (`forms.rs`).

## Storage (all in the profile folder, `~/.local/share/swiftiron`)

| File | What |
|---|---|
| `cookies.tsv` | Cookies with an expiry date |
| `storage/` | `localStorage` per site |
| `history.tsv`, `bookmarks.tsv` | History and bookmarks (`library.rs`) |
| `settings.tsv` | Settings (`settings.rs`, edited on about:settings) |
| `session.txt` | Open tabs, for session restore |
| `crashes/` | Crash reports (`crashlog.rs`) |

Files are written privately (mode 600) and atomically (write a temporary file, then rename).

## Developer switches (environment variables)

| Variable | Effect |
|---|---|
| `SWIFTIRON_PROFILE=dir` | Use another profile folder (tests use a temporary one) |
| `SWIFTIRON_PRIVATE=1` | Don't record history |
| `SWIFTIRON_GPU=0` / `=1` | Turn GPU drawing off / allow software GPU adapters |
| `SWIFTIRON_TILES=0` | Paint the whole page every frame (no tile cache) |
| `SWIFTIRON_FRAME_STATS=1` | Print drawing times every 60 frames |
| `SWIFTIRON_TAB_SLEEP_SECONDS=n` | Put background tabs to sleep after n seconds |
| `SWIFTIRON_THIRD_PARTY_COOKIES=1`, `SWIFTIRON_KEEP_TRACKING_PARAMS=1` | Relax the privacy defaults |
| `SWIFTIRON_MEM_PROBE=1` | Print memory use after each startup stage |

## Command line

```sh
swiftiron [url]                              # open a window
swiftiron --png out.png [--width 1024] url   # render a page to a PNG and exit (used by the tests)
swiftiron --conformance-html5lib dir         # run the HTML-parsing conformance tests
```
