//! `content` on elements: <template>'s fragment, <meta>'s attribute, and an ordinary property
//! elsewhere. GitHub's React views set `content` on an element in strict code, which threw
//! "cannot set non-writable property: content" (there was only a getter, on every element).

use crate::jsapi_tests::{open, serial};

#[test]
fn content_is_template_meta_or_an_ordinary_property() {
    let _g = serial();
    let mut page = open("tests/observers.html");
    let r = page.js.as_mut().expect("scripts").run(r#"(() => {
        "use strict";
        const div = document.createElement("div");
        div.content = { x: 1 };
        const meta = document.createElement("meta");
        meta.setAttribute("content", "width=device-width");
        const was = meta.content;
        meta.content = "initial-scale=1";
        const t = document.createElement("template");
        t.innerHTML = "<b>hi</b>";
        let threw = false;
        try { t.content = null; } catch (e) { threw = true; }
        return [div.content.x, was, meta.getAttribute("content"), t.content.firstChild.nodeName, threw].join("|");
    })()"#).expect("runs");
    assert_eq!(r.trim_matches('"'), "1|width=device-width|initial-scale=1|B|false");
}
