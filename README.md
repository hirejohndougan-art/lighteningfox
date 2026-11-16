# LighteningFox

A web browser engine written from scratch in Rust.

Every stage of the pipeline is our own code:

```
fetch ──► HTML parser ──► DOM ──► CSS parser ──► style cascade ──► layout ──► paint ──► window
net.rs     html.rs       dom.rs    css.rs          style.rs        layout.rs  paint.rs  main.rs
```

Crates are used only for the plumbing: `minifb` (a window and a pixel buffer),
`fontdue` (turning a font file's glyph outlines into pixels), `ureq` (HTTP/TLS), `png` (screenshots)
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
- **Interaction**: address bar, scrolling, clickable links, back history, resize reflow

Performance: a 70,000-node page parses, styles and lays out in about 0.25 s.

## Roadmap

1. **Images**: decode PNG/JPEG/GIF/WebP, intrinsic sizes
2. **Inline boxes**: backgrounds, padding and borders on `<span>`, `<code>`, buttons; `inline-block`
3. **Flexbox**, then **grid**: most modern layouts depend on these
4. **Positioning**: `position: relative/absolute/fixed/sticky`, `z-index`, `float`
5. **Tables**: real table layout
6. **Background loading**: fetch on a worker thread so the UI never freezes
7. **JavaScript speed**: optimize our DOM bindings first (querySelectorAll is the slowest),
   then profile Boa itself and work on its interpreter
8. **Forms**: text inputs, buttons, submission
9. **Tabs, bookmarks, cookies**
10. **GPU painting** and incremental layout for speed
