# Lantern

A web browser engine written from scratch in Rust.

Every stage of the pipeline is our own code:

```
fetch ──► HTML parser ──► DOM ──► CSS parser ──► style cascade ──► layout ──► paint ──► window
net.rs     html.rs       dom.rs    css.rs          style.rs        layout.rs  paint.rs  main.rs
```

Crates are used only for the plumbing: `minifb` (a window and a pixel buffer),
`fontdue` (turning a font file's glyph outlines into pixels), `ureq` (HTTP/TLS) and `png` (screenshots).

## Build and run

```bash
# One-time: install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Fonts (most distros already have them)
sudo apt install fonts-dejavu-core

cargo build --release
./target/release/lantern                      # start page
./target/release/lantern example.com          # open a site
./target/release/lantern page.html --png out.png --width 1024   # render to an image
```

If the build complains about missing X11/Wayland headers:
`sudo apt install libxkbcommon-dev libwayland-dev libx11-dev`.

## Controls

| Action | Keys |
|---|---|
| Address bar | click it, `Ctrl+L` or `F6`; `Enter` to go, `Esc` to cancel |
| Scroll | mouse wheel, arrows, `Space`, `Page Up/Down`, `Home`, `End` |
| Back | `Backspace` or `Alt+Left` |
| Reload | `F5` or `Ctrl+R` |

Typing words that aren't a URL searches DuckDuckGo's HTML edition.

## What works (milestone 1)

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
7. **JavaScript**: embed Boa (pure Rust) and build the DOM bindings
8. **Forms**: text inputs, buttons, submission
9. **Tabs, bookmarks, cookies**
10. **GPU painting** and incremental layout for speed
