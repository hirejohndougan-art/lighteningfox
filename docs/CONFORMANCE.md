# Conformance

How closely Ironswift follows the web standards, measured with the official test suites. These
are a baseline to measure progress against, not a finish line: each fix should move a number.

## Results (16 November 2026)

| Suite | What it checks | Result |
|---|---|---|
| **Test262** | The JavaScript language (Boa 0.22, our engine) | **51,225 of 53,578 passed (95.6%)**; 572 failed, 1,781 skipped as features Boa hasn't built |
| **HTML parsing** (WPT `html/syntax/parsing`, the html5lib tree-construction tests) | The exact tree built from tricky or broken markup | 376 of 1,944 exact (19.3%); 757 (38.9%) if comment and doctype nodes, which Ironswift doesn't keep yet, are ignored |
| **DOM** (WPT `dom/nodes`) | `Node`, `Element`, `Document` and friends | 1,234 of 5,457 subtests (22.6%) |
| **CSS selectors** (WPT `css/selectors`) | Selector matching | 323 of 660 script subtests; 30 of 135 reference pages |
| **Flexbox** (WPT `css/css-flexbox`) | Flex layout | 223 of 894 reference pages; 61 of 398 script subtests |
| **Floats** (WPT `css/CSS2/floats`) | Float layout | 18 of 78 reference pages; 19 of 51 script subtests |

Reference tests pass only when the test page and its reference page render to *identical*
pixels at 800×600. Official WPT allows small pixel differences on some tests, so these numbers
are slightly stricter than wpt.fyi's.

What the numbers point to, in order of payoff:

1. **The full HTML5 parsing algorithm** (checklist section 3), including comment and doctype
   nodes: most HTML-parsing failures and many DOM failures come from the simplified parser.
2. **Missing DOM pieces**: `createDocument`/`createDocumentType`, comment and doctype nodes,
   namespaces, `Range`. Many DOM test files stop in their setup step because of these.
3. **CSS**: `::before`/`::after`, `opacity`, and the remaining flexbox details.

## Running the suites

```sh
# HTML parsing: point at a WPT checkout's .dat files
cargo run --release -- --conformance-html5lib /path/to/wpt/html/syntax/parsing/resources

# DOM and CSS: script tests and reference tests from a WPT checkout (needs python3 with Pillow)
python3 tools/wpt.py /path/to/wpt dom/nodes css/selectors css/CSS2/floats css/css-flexbox

# JavaScript: Test262 against our Boa version (downloads ~400 MB into ./conformance)
sh tools/test262.sh
```

A WPT checkout of just the needed folders:

```sh
git clone --depth 1 --filter=blob:none --sparse https://github.com/web-platform-tests/wpt
cd wpt && git sparse-checkout set resources fonts html/syntax/parsing dom/nodes css/css-flexbox css/CSS2/floats css/selectors
```

`tools/wpt.py` swaps the checkout's `resources/testharnessreport.js` for ours (the original is
kept as `.orig`), serves the files on localhost and runs the browser headless on each test.
