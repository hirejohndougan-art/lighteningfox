# Changelog

Ironswift follows [semantic versioning](https://semver.org). While the version is 0.x, any
release may change behavior; the minor number (0.**2**.0) goes up for new features and the
patch number (0.2.**1**) for fixes only. Each release gets a Git tag (`v0.2.0`) and an
entry here, newest first.

## Unreleased (0.2.0)

### Added
- **HTML parsing** rewritten to follow the HTML standard: passes all 7,046 html5lib tokenizer
  tests and all 1,972 tree-construction tests; scripts run as the parser reaches them,
  `document.write`, `async`/`defer`; slow pages show while they download; XHTML parsed as XML.
- **Shadow DOM**: `attachShadow`, declarative shadow roots, slots, scoped styles, `slotchange`,
  events across shadow trees (60% of the official Shadow DOM tests).
- **Iframes** load and draw their own documents with their own scripts.
- **Browser UI**: zoom per site, find in page, text selection and copy, right-click menu,
  downloads panel, save as PDF and print, view source, developer tools, link status bar.
- **Networking**: HTTP/2 and HTTP/3, Brotli and zstd, a disk cache, downloads, old text
  encodings, proxy settings, clearer error pages.
- Standard event dispatch (capture, target, bubble), mouse/keyboard/focus event classes,
  `window.<id>` named access, `template.content`.

### Changed
- Renamed from LighteningFox to Ironswift.
- The window prefers X11 (XWayland) over native Wayland for a reliable mouse pointer and keys.

## 0.1.0

First version: the full pipeline from network to pixels (HTML, CSS, layout, painting, the
Boa JavaScript engine), tabs, history, bookmarks, forms, cookies, and site storage.
