# LighteningFox

A web browser engine written from scratch in Rust.

Every stage of the pipeline is our own code:

```
fetch ──► HTML parser ──► DOM ──► CSS parser ──► style cascade ──► layout ──► paint ──► window
net.rs     html.rs       dom.rs    css.rs          style.rs        layout.rs  paint.rs  main.rs
```

Crates are used only for the plumbing: `minifb` (a window and a pixel buffer),
`ab_glyph` (turning a font file's glyph outlines into pixels, on demand), `ureq` (HTTP/TLS), `png` (screenshots)
and **Boa** (`boa_engine`), a JavaScript engine written in Rust.

## JavaScript

`js.rs` exposes ~20 small Rust primitives that work on DOM node ids (`__children`, `__insert`,
`__query`...). `prelude.js` builds the web API pages use on top of them: `document`, `Element`,
`classList`, `style`, `innerHTML`, events with bubbling and `preventDefault`, `setTimeout`/`setInterval`,
`requestAnimationFrame`, `DOMContentLoaded`/`load`, `localStorage`, `location`, `console`.
When a script changes the DOM, LighteningFox restyles and reflows the page.

Benchmark (`tests/bench.html`, this machine):

| Workload | Time |
|---|---|
| fib(25) recursion | 87 ms |
| 1M-iteration loop | 138 ms |
| string building 50k | 30 ms |
| array map/filter 200k | 176 ms |
| object property churn 200k | 77 ms |
| JSON round trip (5k objects) | 70 ms |
| regex 20k matches | 70 ms |
| DOM create+append 2000 nodes | 20 ms |
| DOM querySelectorAll x200 | 393 ms |
| DOM innerHTML 500 items | 3 ms |

Run it yourself: `./target/release/lighteningfox tests/bench.html --png /tmp/b.png`

## Build and run

```bash
# One-time: install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Fonts (most distros already have them)
sudo apt install fonts-dejavu-core

cargo build --release
./target/release/lighteningfox                      # start page
./target/release/lighteningfox example.com          # open a site
./target/release/lighteningfox page.html --png out.png --width 1024   # render to an image
```

If the build complains about missing X11/Wayland headers:
`sudo apt install libxkbcommon-dev libwayland-dev libx11-dev`.

## Controls

| Action | Keys |
|---|---|
| Address bar | click it, `Ctrl+L` or `F6`; `Enter` to go, `Esc` to cancel |
| Scroll | mouse wheel, arrows, `Space`, `Page Up/Down`, `Home`, `End` |
| Back | ← button, `Backspace` or `Alt+Left` |
| Forward | → button or `Alt+Right` |
| Reload | ↻ button, `F5` or `Ctrl+R` |

Typing words that aren't a URL searches DuckDuckGo's HTML edition.

## What works

- **HTML**: forgiving parser, entities, void elements, raw-text elements, implied closes (`<p>`, `<li>`, `<td>`…)
- **CSS**: rules, `@media` (desktop width), selectors (type, class, id, attribute, descendant `a b`, child `a > b`,
  `:link`, `:first-child`, `:last-child`, `:root`), specificity, `!important`, inline `style=""`, `<link rel=stylesheet>`
- **Properties**: display, color, background, font size/weight/style/family, line-height, text-align,
  text-decoration, white-space, margin, padding, border, width, max-width, height, list-style, visibility
- **Layout**: block flow, sibling margin collapsing, auto-margin centering, inline flow with word wrapping,
  alignment, `<br>`, `<pre>`, list bullets and numbers
- **Images**: PNG, JPEG, GIF, WebP, BMP, ICO and `data:` URLs, fetched in parallel; sized by CSS,
  `width`/`height` attributes or the image itself, keeping aspect ratio; `max-width` scaling
- **Modern CSS values**: custom properties and `var()` with fallbacks, `calc()`, `min()`, `max()`, `clamp()`
- **Inline boxes**: backgrounds, padding and borders on `<span>`, `<code>`, `<kbd>`, `<mark>`, split across lines
- **Backgrounds**: `border-radius` with smooth corners, `background-image` with size/position/repeat,
  `linear-gradient()` and `radial-gradient()`
- **inline-block**: nav menus, buttons, badges and tiles sit in lines, aligned on their text baseline
- **Background loading**: pages, stylesheets, scripts and images download on worker threads; the window
  stays responsive, shows a loading bar, and the ↻ button turns into ✕ (stop), as does `Esc`
- **Flexbox**: direction, wrap, grow/shrink/basis, `justify-content`, `align-items`/`align-self`,
  `gap`, `order`, loose text as anonymous items
- **Bookmarks and history**: Ctrl+D or the ☆ in the address bar bookmarks a page; every visit is recorded (title, visit count, last visit) in private files in the profile folder (`LF_PRIVATE=1` records nothing); `about:history` (Ctrl+H, with "Clear history") and `about:bookmarks` (Ctrl+Shift+O); the address bar suggests bookmarks and history as you type (arrow keys or click to pick)
- **Text editing**: in fields, text areas and the address bar — caret movement (arrows, Home/End, Ctrl for whole words), Shift to select, Ctrl+A, Delete and Ctrl+Backspace, click to place the caret and double-click to select a word, Up/Down between lines in text areas, and Ctrl+C/X/V with the system clipboard (`arboard`, X11 and Wayland); one click in the address bar selects it
- **Tabs**: tab strip with titles, loading dots and close buttons; Ctrl+T, Ctrl+W, Ctrl+(Shift+)Tab, Ctrl+1–9, Ctrl+Shift+T to reopen; middle-click or Ctrl+click opens links in a background tab, `target="_blank"` and `window.open()` (only during a click — pop-ups are blocked) in a new tab; hidden tabs pause their scripts, and a page that loads in the background is only built when you look at it
- **Real-world libraries**: React 18, Vue 3, jQuery 3 and Bootstrap 5 render and respond to clicks (kept as screenshot tests in `tests/real-world/`); standard element interfaces (`HTMLInputElement`, `SVGElement`… with honest `instanceof`), `document.implementation.createHTMLDocument`, `DOMParser`
- **Web fonts**: `@font-face` with WOFF2, WOFF, TrueType and OpenType sources (decoded with the pure-Rust `wuff` crate); only fonts the page uses are downloaded, in parallel; weight and italic matching; characters a web font lacks fall back to the system font; generic families (`serif`, `monospace`…)
- **Event loop**: each timer and message is its own task with promise callbacks (microtasks) run after it; `requestAnimationFrame` runs once per drawn frame; `MessageChannel`, `window.postMessage`, `structuredClone`; headless snapshots wait for network requests and timers due within 0.5 s
- **Web Storage**: `localStorage` saved per site in the profile folder and kept between runs (5 MB per site), `sessionStorage` for the page; both also work as `localStorage.key = value`
- **History and in-page links**: `history.pushState`/`replaceState`/`back`/`go`, `popstate` and `hashchange`; Back/Forward between entries of the same page don't reload; `#anchor` links scroll to their target and add a history entry; `location` always reflects the current URL
- **Transforms**: `translate`, `scale`, `rotate`, `skew`, `matrix`, `transform-origin`, and the separate `translate`/`rotate`/`scale` properties; rotated and scaled elements are drawn off-screen and mapped with smooth sampling, and clicks follow them
- **Stacking contexts**: z-index layers nest inside their parent's stacking context, as in other browsers
- **Scrollable boxes**: `overflow: auto/scroll` boxes scroll with the mouse wheel (the innermost one under the pointer), show a scrollbar, hide and ignore clicks on content scrolled out of view, and expose `scrollTop`/`scrollHeight` to scripts; `min-height` and `max-height`
- **position: sticky**: headers, sidebars and section titles that stick while scrolling, stopping at the end of their parent
- **Layout data for scripts**: `getBoundingClientRect`, `offsetWidth/Height/Top`, `clientWidth`, `scrollHeight`, live `innerWidth`/`scrollY`, `scrollTo`/`scrollBy`/`scrollIntoView`, and `matchMedia` width queries; if a script changed the page, it's laid out again before answering
- **fetch() and XMLHttpRequest**: requests run on background threads and never freeze the page; JSON, text, status, headers, POST bodies, `AbortController`, sync XHR; cross-origin rules (CORS, including preflight) so a site can't read other sites' data or your local network; plus `URL`, `URLSearchParams`, `FormData`, `Headers`, `Response`
- **Cookies**: Set-Cookie with Domain, Path, Expires/Max-Age, Secure, HttpOnly, SameSite and the `__Host-`/`__Secure-` prefixes; cookies set during redirects are kept; `document.cookie`; saved to `~/.local/share/lighteningfox/cookies.tsv` (or `$LF_PROFILE`), readable only by you
- **Privacy by default**: third-party cookies blocked (set `LF_THIRD_PARTY_COOKIES=1` to allow), tracking parameters (utm_*, fbclid, gclid…) stripped from page URLs (`LF_KEEP_TRACKING_PARAMS=1` keeps them), Global Privacy Control sent, no telemetry
- **Forms**: text, password, email and other inputs, textareas, checkboxes, radio buttons, dropdowns, buttons; typing with a caret, Tab between fields, Enter to submit, GET and POST submission, reset, labels, `:focus`, and the JavaScript form APIs (`value`, `checked`, `selectedIndex`, `form.submit()`, submit/input/change events)
- **CSS grid**: px/%/`fr`/auto/`minmax()`/`fit-content()` tracks, `repeat(N | auto-fill | auto-fit)`, named areas, line numbers (negative too), spans, auto-placement, implicit rows/columns, gaps, align-items/align-self
- **Positioning**: `relative`, `absolute` (against the nearest positioned ancestor), `fixed` (stays put
  while scrolling, clickable), `top/right/bottom/left`, `inset`
- **Box effects**: `box-shadow` (offset, blur, spread), `overflow: hidden` with rounded clipping,
  `box-sizing: border-box`, `min-width`
- **Floats**: `float`, `clear`, text wrapping around floats, `display: flow-root`, `<img align>`
- **Tables**: auto column widths from content, `colspan`/`rowspan`, captions, `border-spacing`,
  `border-collapse`, cell `vertical-align`, old HTML attributes (`border`, `cellpadding`, `cellspacing`, `valign`)
- **Selectors**: `+` and `~`, attribute operators (`^=` `$=` `*=` `~=` `|=`), `:not()`, `:is()`, `:where()`,
  `:nth-child()`, `:nth-of-type()`, `:first/last/only-child`, `:empty`, `:checked`, `:hover` (live in the window)
- **Stacking**: `z-index` on positioned boxes; borders default to the text color (`currentColor`)
- **Interaction**: address bar, scrolling, clickable links, back history, resize reflow

Performance: a 70,000-node page parses, styles and lays out in about 0.25 s.

## Memory

Measured with a window open (RSS), after the memory work:

| Page | Before | After |
|---|---|---|
| Start page | 108 MB | 16 MB |
| Modern layout test page | 118 MB | 25 MB |
| Images test page | 122 MB | 29 MB |
| 70,000-element page | 168 MB | 38 MB |

What changed: font glyphs load on first use (was ~88 MB up front), computed styles are shared,
JavaScript starts only on pages with scripts, words on a line are one display item, attributes are
a short list, freed memory goes back to the OS, and the window polls slowly when idle.
`LF_MEM_PROBE=1 lighteningfox <url>` prints memory after each loading stage.

## Tests

```bash
cargo test --release                       # unit tests + screenshot tests
UPDATE_REFERENCES=1 cargo test --release   # approve new screenshots after an intended change
```

Screenshot tests render each page listed in `src/main.rs` (`screenshot_tests`) and compare it with
`tests/reference/*.png`. Failures save the new rendering to `target/screenshot-diffs/`.
GitHub Actions runs the same tests on every push (`.github/workflows/ci.yml`).

## Roadmap

1. **Vertical-align**, percentage heights and multi-column layout
2. **JavaScript speed**: optimize our DOM bindings first (querySelectorAll is the slowest),
   then profile Boa itself and work on its interpreter
3. **Sleeping tabs** and a settings page
4. **GPU painting** and incremental layout for speed
5. *Optional, after everything else:* a JIT compiler for Boa
