# Contributing to Ironswift

Thanks for helping. Ironswift is a browser engine written from scratch in Rust; most of the
pipeline (HTML and CSS parsing, the DOM, style, layout, painting, the browser UI) is our own
code, with the Boa engine for JavaScript.

## Getting started

```sh
cargo build --release
./target/release/ironswift https://example.com      # the browser
./target/release/ironswift --png out.png file.html  # render a page to an image, no window
cargo test --release                                # unit and screenshot tests
```

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains how the pieces fit together.
- [docs/ADDING.md](docs/ADDING.md) walks through adding a CSS property or a DOM API.
- [docs/CONFORMANCE.md](docs/CONFORMANCE.md) shows how to run the official test suites
  (Web Platform Tests, html5lib, Test262) and the current scores.

## Good first changes

- A CSS property or value that isn't supported yet (check `style.rs`'s `apply`).
- A DOM or Web API method pages call that's missing (check `prelude.js`).
- A site that renders wrong: reduce it to a small HTML file and add it under `tests/`.

## Pull requests

- Keep each pull request to one change, with a test when one fits: a unit test, a page under
  `tests/` with a reference image, or a better score on a conformance suite.
- Run `cargo test --release` before sending; CI runs it on every push.
- Follow the existing style: `cargo fmt`, plain comments that say *why*, no new dependency
  without a good reason (say what it replaces or why writing it ourselves would be worse).
- Code is accepted under the project's [MIT license](LICENSE).

## Reporting bugs

Use the issue templates. For a site that breaks, the URL, a screenshot, and what
about:log shows are the most useful things to include. Security problems go through
[SECURITY.md](SECURITY.md), not public issues.

Everyone taking part is expected to follow the [code of conduct](CODE_OF_CONDUCT.md).
