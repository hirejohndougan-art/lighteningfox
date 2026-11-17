# How to add things

Two worked recipes for the most common changes. Read [ARCHITECTURE.md](ARCHITECTURE.md) first for
where everything lives.

## Add a CSS property

Example: `text-indent` (indent the first line of a paragraph).

1. **Store it.** Add a field to `Style` in `src/style.rs`. Fields above the `// inherited` comment
   are not inherited, fields below it are. `text-indent` is inherited:

   ```rust
   pub text_indent: Len,
   ```

2. **Give it a starting value** in `impl Default for Style` (`text_indent: Len::Px(0.0)`), and, for an
   inherited property, copy it from the parent in `Style::inherit` (`text_indent: parent.text_indent`).
   A property that isn't inherited takes its default there instead.

3. **Parse it.** Add an arm to the big `match` in `pub fn apply(st, parent, name, value)`:

   ```rust
   "text-indent" => {
       if let Some(l) = parse_len(&lower, fs) {
           st.text_indent = l;
       }
   }
   ```

   `apply` receives the value after `var()` substitution; `lower` is the lower-cased value and `fs`
   the font size `em` lengths are relative to. `parse_len` already handles `calc()`, `em`, `rem` and
   percentages. Shorthands (like `margin`) are expanded
   by calling `apply` once per longhand.

4. **Use it** where it has an effect: in `src/layout.rs` for anything that moves boxes or text
   (here: start the first line of `inline()` `text_indent` further in), or in `src/paint.rs` for
   anything purely visual (colors, shadows).

5. **If the style sharing cache could be fooled**, check `attr_key` in `style.rs`: it lists the
   HTML attributes that change style (`style`, `dir`, `width`…). A new CSS property doesn't
   need anything there; a new *presentational attribute* does.

6. **Test it.** Make or extend a page in `tests/`, add it to the `PAGES` list in
   `screenshot_tests` (`src/main.rs`) if it's new, look at it with
   `cargo run --release -- --png /tmp/x.png tests/yourpage.html`, then approve the reference image
   with `UPDATE_REFERENCES=1 cargo test --release`. Then run the WPT reference tests for the area
   (see [CONFORMANCE.md](CONFORMANCE.md)) and note the change in pass count.

## Add a DOM API

Most DOM APIs are written in JavaScript in `src/prelude.js`, on top of the native functions in
`src/js.rs`. Write it in JavaScript when it can be built from what's already there; add a native
function when it needs data only Rust has, or when it's hot and needs speed.

### In JavaScript (most APIs)

Example: `Element.prototype.toggleAttribute` already looks like this in `prelude.js`:

```js
toggleAttribute(n, force) {
  const on = force === undefined ? !this.hasAttribute(n) : force;
  on ? this.setAttribute(n, "") : this.removeAttribute(n);
  return on;
}
```

Add methods to the right class (`Node`, `Element`, `Document`, `HTMLInputElement`…) and new
globals to the `Object.assign(g, { … })` block near the end of the file.

### With a native function

Example: a native `__byId(id)` (the real one backs `document.getElementById`).

1. **Write the function** in `src/js.rs`. Natives take `(this, args, context)`; read arguments with
   `arg_id` / `arg_str`, check node ids with `valid`, read the DOM with `with_dom` and change it
   with `with_dom_mut` (which also tells the window to restyle and lay out again):

   ```rust
   fn n_by_id(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
       let want = arg_str(a, 0, ctx)?;
       let found = element_by_id(&want);
       Ok(JsValue::from(found.map_or(-1.0, |n| n as f64)))
   }
   ```

2. **Register it** in the list of natives in `Js::new` (`("__byId", 1, n_by_id)`; the number is
   how many arguments it takes).

3. **Expose it** in `prelude.js`, turning node ids into element objects with `wrap(id)` (one id) or
   `__wrapList(ids)` (many):

   ```js
   getElementById(id) { return wrap(__byId(String(id))); }
   ```

4. **Test it.** Add a check to a test page's script, or better, run the WPT tests for the API
   (`python3 tools/wpt.py <wpt> dom/nodes`) and compare the pass count before and after.
   For speed-sensitive APIs, add a case to `tests/bench.html`.

## Before you commit

```sh
cargo build --release && cargo build --release --no-default-features   # with and without the GPU
cargo test --release                                                     # unit, parser and screenshot tests
```
