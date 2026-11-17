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

- **Security and privacy**:
  - The same-origin policy is enforced by the browser for cookies, `localStorage`,
    `fetch`/XHR and `pushState` (scripts could previously name another site's origin).
  - Content Security Policy (headers and `<meta>`), `X-Frame-Options`, and `<iframe sandbox>`.
  - Web pages can't open, frame or read local files, `about:` pages or `data:` documents.
  - The browser runs under a seccomp system-call filter; cross-site iframes are drawn in
    their own sandboxed process (seccomp, no programs, Landlock: no writes).
  - Ad and tracker blocking on a par with uBlock Origin: EasyList, EasyPrivacy, the uBlock
    filter lists and Peter Lowe's list, kept up to date automatically. Regex rules,
    `$important`, `$badfilter`, `$redirect` (harmless stand-ins for blocked ad and analytics
    libraries, so pages keep working), `$removeparam`, `$csp`, `$denyallow`/`$to`, `$popup`,
    `example.*` domains, `!#if`/`!#include`; procedural cosmetic filters (`:has-text`,
    `:upward`, `:remove`, `:style`…); and scriptlets (`##+js(…)`) that defuse anti-adblock
    walls, written from scratch. Optional hiding of cookie notices. A warning before sites on
    malware and phishing lists or that a filter blocks outright.
  - Embedded sites get partitioned cookies and storage; per-site cookie rules (always
    allow, block, delete on close) and a list of sites with cookies on about:settings.
  - Permission prompts for camera, microphone, location and notifications, remembered per site.
  - Fingerprinting: fixed screen size, processor count and fonts for scripts; canvas noise
    ready for when `<canvas>` lands.
  - Daily update checks; per-user installs update themselves (SHA-256 verified).
  - Fuzz targets for the HTML, CSS, JavaScript, CSP and filter-list parsers.

### Changed
- Renamed from LighteningFox to Ironswift.
- The window prefers X11 (XWayland) over native Wayland for a reliable mouse pointer and keys.

## 0.1.0

First version: the full pipeline from network to pixels (HTML, CSS, layout, painting, the
Boa JavaScript engine), tabs, history, bookmarks, forms, cookies, and site storage.
