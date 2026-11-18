//! Tests for the JavaScript APIs pages use: load a test page from `tests/`, let its
//! scripts run, then check what they did to the document.

use crate::js;

/// These tests share the engine's global network queue: run them one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Load a page, run its scripts and timers, and return the engine for more checks.
fn open(path: &str) -> crate::Page {
    let profile = std::env::temp_dir().join(format!("ironswift-jsapi-{}", std::process::id()));
    // SAFETY: tests only read this variable; every test sets the same value.
    unsafe { std::env::set_var("IRONSWIFT_PROFILE", &profile) };
    crate::load_fonts().expect("fonts");
    crate::geometry::set_viewport(800.0, 600.0);
    let url = crate::net::normalize_input(path);
    let mut page = crate::load(&url);
    if let Some(j) = page.js.as_mut() {
        j.settle(std::time::Duration::from_secs(2));
    }
    page
}

fn text_of(id: &str) -> String {
    js::with_dom(|d| {
        let n = d.descendants(crate::dom::Dom::ROOT).into_iter().find(|&n| d.attr(n, "id") == Some(id));
        n.map(|n| d.text_content(n)).unwrap_or_default()
    })
}

fn eval(page: &mut crate::Page, code: &str) -> String {
    page.js.as_mut().expect("scripts").run(code).unwrap_or_else(|e| format!("error: {e}"))
}

#[test]
fn es_modules() {
    let _serial = serial();
    let _page = open("tests/modules.html");
    assert_eq!(text_of("out"), "hello modules 42 7 modules.html");
    assert_eq!(text_of("dyn"), "dyn 42");
    // Modules wait for the parser, like deferred scripts.
    assert_eq!(text_of("order"), "classic;classic2;module;");
    let _ = eval;
}

#[test]
fn custom_elements() {
    let _serial = serial();
    let _page = open("tests/custom-elements.html");
    assert_eq!(
        text_of("log"),
        "name:null>Ada;connected Ada;upgraded true true;created true;name:null>Grace;connected Grace;\
         name:null>Linus;connected Linus;disconnected Linus;name:null>Alan;connected Alan;bad SyntaxError get true;whenDefined;"
    );
}

#[test]
fn intersection_and_resize_observers() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    assert_eq!(text_of("io"), "top:true:1.00;far:false:0.00;");
    assert_eq!(text_of("ro"), "box:200x50/210;");
    // Scroll the far box into view; the page gets a scroll event and a new intersection.
    eval(&mut page, "scrollTo(0, 1800)");
    let js = page.js.as_mut().unwrap();
    js.after_layout();
    js.settle(std::time::Duration::from_millis(200));
    assert_eq!(text_of("io"), "top:true:1.00;far:false:0.00;top:false:0.00;far:true:1.00;");
    assert_eq!(text_of("ev"), "scroll1640;");
    // Resizing the box reports its new size once.
    eval(&mut page, "document.getElementById('box').style.width = '300px'");
    let js = page.js.as_mut().unwrap();
    js.after_layout();
    js.settle(std::time::Duration::from_millis(200));
    assert_eq!(text_of("ro"), "box:200x50/210;box:300x50/310;");
}

#[test]
fn user_events() {
    let _serial = serial();
    let mut page = open("tests/user-events.html");
    let (btn, outer, field) = js::with_dom(|d| {
        let find = |id: &str| d.descendants(crate::dom::Dom::ROOT).into_iter().find(|&n| d.attr(n, "id") == Some(id)).unwrap();
        (find("btn"), find("outer"), find("field"))
    });
    let js = page.js.as_mut().unwrap();
    let mut p = crate::userinput::Pointer::default();
    let m = crate::userinput::Mods::default();
    p.update(js, Some(btn), 10.0, 12.0, [false; 3], m);
    p.update(js, Some(btn), 10.0, 12.0, [true, false, false], m);
    js.click(btn);
    p.update(js, Some(btn), 10.0, 12.0, [false; 3], m);
    p.update(js, None, 500.0, 500.0, [false; 3], m);
    let _ = outer;
    js.dispatch(field, "focus");
    assert!(crate::userinput::key(js, crate::window::Key::A, false, false, m));
    assert!(!crate::userinput::key(js, crate::window::Key::X, false, false, m), "cancelled keydown");
    assert!(!crate::userinput::wheel(js, Some(btn), 1.0, 1.0, 0.0, 45.0, m), "cancelled wheel");
    assert_eq!(
        text_of("log"),
        "mouseover;outer-enter;mouseenter;mousedown@10,12;click;mouseup;mouseout;mouseleave;outer-leave;\
         focus true false;focusin;keydown a KeyA true;keydown x KeyX true;wheel 45;"
    );
}

#[test]
fn promises_with_timers_and_network() {
    let _serial = serial();
    let _page = open("tests/async.html");
    assert_eq!(
        text_of("log"),
        "start;sync end;microtask;then;unhandled oops;t0;timer;fetch 2;text true;caught;all 1,2;done;"
    );
}

#[test]
fn fetch_keeps_binary_bodies() {
    let _serial = serial();
    let _page = open("tests/fetch-binary.html");
    assert_eq!(
        text_of("log"),
        "ab 148 137,80,78,71;blob 148 image/png 137,80,78,71;bytes 148 clone 148;json-bytes true;\
         made 255,0,233,65;utf8 \u{e9};fromblob text/x-test hi;strbytes 195,169;xhr 148 137,80,78,71;\
         xhrblob 148;done;"
    );
}

#[test]
fn webidl_generated_bindings() {
    let _serial = serial();
    let _page = open("tests/webidl-bindings.html");
    let log = text_of("log");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines[0], "DIV,DIV,div,a,x y,2,3,p1,p2,p2,p1,B,2,true,a,0,2");
    assert_eq!(lines[1], "v,true,,a,a,true,false,BODY,HEAD,HTML,2");
    assert_eq!(lines[2], "4,2,20,10,0,2,33", "compareDocumentPosition");
    assert_eq!(lines[3], "true,false,true,false,true,false,true,true,id|class|data-k");
    assert_eq!(lines[4], "TypeError,TypeError,TypeError,TypeError,1,1,get parentNode,TypeError", "WebIDL checks");
    // document.doctype exists (the page has one) and its textContent is null.
    assert_eq!(lines[5], "one,3,c,,,two three");
    assert_eq!(
        lines[6],
        "http://www.w3.org/1999/xhtml,http://www.w3.org/2000/svg,,true,B,,wo,o ,IndexSizeError true,abcd,ef"
    );
    assert_eq!(lines[7], "1,1,HTML,HTML,UTF-8,UTF-8,text/html,CSS1Compat,0,0,0");
}

#[test]
fn developer_console_runs_code() {
    let _serial = serial();
    let mut page = open("tests/custom-elements.html");
    let js = page.js.as_mut().unwrap();
    assert_eq!(js.run("1 + 2").unwrap(), "3");
    assert_eq!(js.run("'hi'").unwrap(), "\"hi\"");
    assert_eq!(js.run("document.querySelector('user-card')").unwrap(), "<user-card>");
    assert_eq!(js.run("[1, 'a', {b: 2}]").unwrap(), "[1, \"a\", {b: 2}]");
    assert!(js.run("nope()").unwrap_err().contains("ReferenceError"));
    // The console shows what was run and its result.
    crate::devtools::set_inspected(page.doc);
    crate::js::with_dom(|d| crate::devtools::capture(d, &page.url, "", page.loaded_at, 1, 0));
    crate::devtools::record("1 + 2", Ok("3".into()));
    let html = crate::devtools::page();
    assert!(html.contains("&gt; 1 + 2") && html.contains("action=\"about:devtools-run\""));
    if let Ok(p) = std::env::var("DEVTOOLS_DUMP") { std::fs::write(p, &html).unwrap(); }
}

#[test]
fn geolocation() {
    let _serial = serial();
    let _geo = crate::geo::TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: tests only read this variable; no other test sets it.
    unsafe { std::env::set_var("IRONSWIFT_FAKE_LOCATION", "29.4608,-95.0513,30") };
    let url = crate::net::normalize_input("tests/geolocation.html");
    let profile = std::env::temp_dir().join(format!("ironswift-jsapi-{}", std::process::id()));
    // SAFETY: as in open().
    unsafe { std::env::set_var("IRONSWIFT_PROFILE", &profile) };
    crate::permissions::set(&url, crate::permissions::Kind::Location, true);
    let _page = open("tests/geolocation.html");
    assert_eq!(text_of("out"), "29.4608,-95.0513,30,,,number,true,true,7");
    assert_eq!(text_of("watch"), "watched 1");
    assert_eq!(text_of("types"), "number,true,3,true");
    assert_eq!(text_of("bad"), "TypeError");
    crate::permissions::set(&url, crate::permissions::Kind::Location, false);
    // Denied: the error callback gets PERMISSION_DENIED.
    let mut page = open("tests/geolocation.html");
    if let Some(j) = page.js.as_mut() {
        j.settle(std::time::Duration::from_secs(1));
    }
    assert_eq!(text_of("out"), "error 1 User denied Geolocation");
}

#[test]
fn import_maps() {
    let _serial = serial();
    let _page = open("tests/importmap.html");
    assert_eq!(text_of("a"), "dep via import map");
    assert_eq!(text_of("b"), "lib/ prefix");
    // A scope remaps "dep" for modules inside importmap/.
    assert_eq!(text_of("c"), "scoped dep");
    assert_eq!(text_of("d"), "resolve ok");
    // A second map adds "late" but can't override the first map's "dep".
    assert_eq!(text_of("e"), "dynamic dep via import map");
}

#[test]
fn extension_content_scripts() {
    let _serial = serial();
    crate::extensions::install_for_tests(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/extensions/test-ext"));
    let mut page = open("tests/extension-test.html");
    // The content scripts run in their own world: the page sees neither `chrome`, the
    // `__extCall` native nor the scripts' globals, and replacing JSON.stringify in the page
    // doesn't reach the extension's calls. Let the page hear the background's tabs.sendMessage (pages check every 100 ms).
    for _ in 0..30 {
        if text_of("out").contains("poked") {
            break;
        }
        if let Some(j) = page.js.as_mut() {
            j.tick();
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        text_of("out"),
        "start saw before;page ran yes;chrome function page sees undefined;\
         page sees undefined,undefined,undefined,undefined;echo hi extension-test.html;\
         storage {\"a\":1} hooked no;bg poking;poked;"
    );
}

// Causes found on live sites (task X1); the pages in tests/sites/ say where each came from.

#[test]
fn unchanged_attribute_writes_keep_the_layout() {
    let _serial = serial();
    let mut page = open("tests/sites/layout-thrash.html");
    assert_eq!(text_of("out"), "measured");
    let before = crate::geometry::layouts_done();
    let r = eval(
        &mut page,
        "let t = 0; for (const a of document.getElementById('nav').children) { t += a.clientWidth; a.removeAttribute('data-hidden'); a.setAttribute('href', '#'); a.style.setProperty('color', '#123456', 'important'); } t > 0",
    );
    assert_eq!(r, "true");
    // Measuring 60 items between writes that change nothing needs at most one fresh layout.
    assert!(crate::geometry::layouts_done() - before <= 1, "{} layouts", crate::geometry::layouts_done() - before);
    // A real change still shows.
    eval(&mut page, "document.querySelector('#nav a').setAttribute('data-hidden', '')");
    assert_eq!(eval(&mut page, "document.querySelector('#nav a').clientWidth"), "0");
}

#[test]
fn deeply_nested_flex_lays_out_quickly() {
    let _serial = serial();
    let started = std::time::Instant::now();
    let mut page = open("tests/sites/nested-flex.html");
    let w = eval(&mut page, "let e = document.getElementById('root'); while (e.firstElementChild && e.firstElementChild.tagName === 'DIV') e = e.firstElementChild; e.getBoundingClientRect().width > 0");
    assert_eq!(w, "true");
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "took {:?}", started.elapsed());
}

#[test]
fn observer_ignores_the_widget_it_restyles() {
    let _serial = serial();
    let _page = open("tests/sites/mutation-theme.html");
    assert_eq!(text_of("out"), "calls 2 rgb(255, 255, 255)");
}

#[test]
fn rem_follows_the_root_font_size() {
    let _serial = serial();
    let mut page = open("tests/sites/rem-root.html");
    assert_eq!(eval(&mut page, "getComputedStyle(document.body).fontSize"), "\"18px\"");
    assert_eq!(eval(&mut page, "document.getElementById('box').getBoundingClientRect().width"), "120");
}

#[test]
fn flex_end_menu_shrinks_to_its_items() {
    let _serial = serial();
    let mut page = open("tests/sites/flex-end-shrink.html");
    // The menu sits on the logo's line, at the right.
    assert_eq!(eval(&mut page, "document.getElementById('menu').getBoundingClientRect().top === document.getElementById('brand').getBoundingClientRect().top"), "true");
    assert_eq!(eval(&mut page, "document.getElementById('menu').getBoundingClientRect().width < 400"), "true");
}

#[test]
fn fit_content_search_box_is_centered() {
    let _serial = serial();
    let mut page = open("tests/sites/bing-searchbox.html");
    // width: fit-content sizes the fixed bar to its 500px form, and auto margins center it.
    assert_eq!(eval(&mut page, "document.getElementById('sbox').getBoundingClientRect().width"), "500");
    assert_eq!(eval(&mut page, "(() => { const r = document.getElementById('sbox').getBoundingClientRect(); return Math.abs(r.left - (innerWidth - r.width) / 2) < 1 })()"), "true");
    // An in-flow max-content block shrinks to its text and centers too.
    assert_eq!(eval(&mut page, "(() => { const r = document.getElementById('pill').getBoundingClientRect(); return r.width < 300 && Math.abs(r.left - (innerWidth - r.width) / 2) < 1 })()"), "true");
    // font: inherit replaces the textarea's monospace default.
    assert_eq!(eval(&mut page, "getComputedStyle(document.getElementById('q')).fontFamily.includes('arial')"), "true");
    // The load handler could read the navigation timing.
    assert_eq!(text_of("out"), "true 0 true");
}

#[test]
fn flex_align_content_spreads_wrapped_lines() {
    let _serial = serial();
    let mut page = open("tests/sites/flex-align-content.html");
    // The top of each item, relative to its container: [first line, second line].
    let tops = |page: &mut crate::Page, id: &str| {
        eval(page, &format!("(() => {{ const c = document.getElementById('{id}'); const t = c.getBoundingClientRect().top; const k = c.children; return [k[0].getBoundingClientRect().top - t, k[2] ? k[2].getBoundingClientRect().top - t : -1, k[0].getBoundingClientRect().height].join(','); }})()"))
            .trim_matches('"')
            .to_string()
    };
    // 200px tall, two 40px lines: 120px free.
    assert_eq!(tops(&mut page, "center"), "60,100,40");
    assert_eq!(tops(&mut page, "between"), "0,160,40");
    assert_eq!(tops(&mut page, "end"), "120,160,40");
    // normal (stretch): each line gets half the free height, and auto-height items fill it.
    assert_eq!(tops(&mut page, "stretch"), "0,100,100");
    // A single-line container ignores align-content: its line is the container's height.
    assert_eq!(tops(&mut page, "nowrap"), "0,-1,40");
}

#[test]
fn dialogs_and_popovers() {
    let _serial = serial();
    let mut page = open("tests/sites/dialog-popover.html");
    let settle = |page: &mut crate::Page| {
        if let Some(j) = page.js.as_mut() {
            j.settle(std::time::Duration::from_millis(100));
        }
    };
    // A modal dialog: in the top layer, centered, the page behind it inert.
    let r = eval(&mut page, r#"(() => {
        d.addEventListener('close', () => window.closed = (window.closed || '') + '[' + d.returnValue + ']');
        d.showModal();
        const r = d.getBoundingClientRect();
        outside.click();
        return [d.open, d.matches(':modal'), Math.abs(r.left + r.width / 2 - innerWidth / 2) < 2, Math.abs(r.top + r.height / 2 - innerHeight / 2) < 2, window.outsideClicks || 0].join(',');
    })()"#);
    assert_eq!(r.trim_matches('"'), "true,true,true,true,0");
    // Esc cancels it; close events come after.
    eval(&mut page, "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))");
    settle(&mut page);
    assert_eq!(eval(&mut page, "[d.open, d.matches(':modal'), window.closed].join(',')").trim_matches('"'), "false,false,[]");
    // close(value) sets returnValue; <form method=dialog> closes with the button's value.
    eval(&mut page, "d.showModal(); d.close('kept')");
    settle(&mut page);
    eval(&mut page, "d.showModal(); const e = new Event('submit', {bubbles: true, cancelable: true}); Object.defineProperty(e, 'submitter', {value: yes}); d.querySelector('form').dispatchEvent(e)");
    settle(&mut page);
    assert_eq!(eval(&mut page, "[d.open, window.closed, outside.click(), window.outsideClicks].join(',')").trim_matches('"'), "false,[][kept][yes],,1");
    // showModal on an open modal is fine; on a dialog opened with show() it's an error.
    assert_eq!(eval(&mut page, "d.show(); let e2 = ''; try { d.showModal() } catch (x) { e2 = x.name } d.close(); e2").trim_matches('"'), "InvalidStateError");
    // Popovers: a popovertarget button toggles one; a press outside closes it (light dismiss);
    // a manual one stays; Esc closes the top auto popover.
    let r = eval(&mut page, r#"(() => {
        const out = [];
        pop.addEventListener('toggle', (e) => window.toggles = (window.toggles || '') + e.newState + ';');
        menu.click(); out.push(pop.matches(':popover-open'), getComputedStyle(pop).display);
        menu.click(); out.push(pop.matches(':popover-open'));
        pop.showPopover(); manual.showPopover();
        outside.dispatchEvent(new MouseEvent('mousedown', {bubbles: true}));
        out.push(pop.matches(':popover-open'), manual.matches(':popover-open'), getComputedStyle(manual).display);
        manual.hidePopover();
        pop.togglePopover(); document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}));
        out.push(pop.matches(':popover-open'), pop.popover, manual.popover, menu.popoverTargetElement === pop);
        return out.join(',');
    })()"#);
    assert_eq!(r.trim_matches('"'), "true,block,false,false,true,block,false,auto,manual,true");
    settle(&mut page);
    assert!(eval(&mut page, "window.toggles").contains("open;closed"), "toggle events");
}

#[test]
fn tree_walker_and_node_iterator() {
    let _serial = serial();
    let _page = open("tests/sites/tree-walker.html");
    assert_eq!(text_of("out"), "P,!c,B,SPAN");
    assert_eq!(text_of("it"), "a,b,s");
    // A rejected <p> hides its <b>; parentNode() from <span> may return the root itself.
    assert_eq!(text_of("filt"), "SPAN;DIV");
}

#[test]
fn light_dark_follows_color_scheme() {
    let _serial = serial();
    let mut page = open("tests/sites/light-dark.html");
    let bg = |page: &mut crate::Page, id: &str| eval(page, &format!("getComputedStyle(document.getElementById('{id}')).backgroundColor"));
    assert_eq!(bg(&mut page, "light"), "\"rgb(255, 255, 255)\"");
    assert_eq!(bg(&mut page, "dark"), "\"rgb(27, 27, 27)\"");
    assert_eq!(bg(&mut page, "short"), "\"rgb(0, 0, 200)\"");
    assert_eq!(eval(&mut page, "getComputedStyle(document.getElementById('inner')).color"), "\"rgb(255, 255, 255)\"");
}

#[test]
fn scripts_the_page_inserts_run() {
    let _serial = serial();
    let _page = open("tests/sites/dynamic-scripts.html");
    assert_eq!(text_of("out"), "inline true;async true;innerHTML true;load 1 ext;runs 1;error");
}

/// `document.body = …` (HTML: replace the body, or add one under <html>) and
/// `createElementNS` for SVG and MathML (in their namespaces, names kept as written).
#[test]
fn document_body_setter_and_create_element_ns() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r#"(() => {
            const out = [];
            try { document.body = document.createElement("div"); } catch (e) { out.push(e.name); }
            try { document.body = null; } catch (e) { out.push(e.name); }
            const nb = document.createElement("body");
            nb.id = "fresh";
            document.body = nb;
            out.push(document.body.id, document.querySelectorAll("body").length, nb.parentNode === document.documentElement);
            document.body = nb;
            out.push(document.querySelectorAll("body").length);
            const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
            const grad = document.createElementNS("http://www.w3.org/2000/svg", "linearGradient");
            const math = document.createElementNS("http://www.w3.org/1998/Math/MathML", "mi");
            out.push(svg.namespaceURI, grad.localName, grad.tagName, math.namespaceURI);
            return out.join(",");
        })()"#,
    );
    assert_eq!(
        r.trim_matches('"'),
        "HierarchyRequestError,HierarchyRequestError,fresh,1,true,1,http://www.w3.org/2000/svg,linearGradient,linearGradient,http://www.w3.org/1998/Math/MathML"
    );
}

#[test]
fn styles_scripts_add_or_change_apply() {
    let _serial = serial();
    let _page = open("tests/sites/live-styles.html");
    let g = "rgb(0, 128, 0)";
    assert_eq!(text_of("out"), format!("{g};{g};{g};{g};true;\"\""));
}

#[test]
fn inconsistent_sort_comparators_dont_crash() {
    let _serial = serial();
    let _page = open("tests/sites/random-sort.html");
    assert_eq!(text_of("out"), "true 300 bdac");
}

/// createElementNS keeps any namespace (or none) and the prefix, and checks the name (X16).
#[test]
fn create_element_ns_any_namespace() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r#"(() => {
            const out = [];
            const a = document.createElementNS(null, "foo");
            out.push(a.namespaceURI, a.tagName, a.prefix, a instanceof HTMLElement);
            const b = document.createElementNS("test", "html:SPAN");
            out.push(b.namespaceURI, b.prefix, b.localName, b.tagName, b.nodeName);
            const c = document.createElementNS("http://www.w3.org/1999/xhtml", "html:span");
            out.push(c.tagName, c instanceof HTMLSpanElement, document.createElementNS("http://www.w3.org/1999/xhtml", "SPAN") instanceof HTMLUnknownElement);
            for (const [ns, q] of [[null, "1foo"], [null, "f:oo"], ["", "foo:"], ["x", "xmlns"]]) {
                try { document.createElementNS(ns, q); out.push("ok"); } catch (e) { out.push(e.name); }
            }
            try { document.createElement("<p>"); } catch (e) { out.push(e.name); }
            const k = b.cloneNode(); out.push(k.prefix, k.namespaceURI, k.isEqualNode(b));
            document.body.appendChild(b);
            out.push(document.getElementsByTagName("html:SPAN").length, document.getElementsByTagName("SPAN").length);
            out.push(document.createElement("title") instanceof SVGElement);
            return out.join(",");
        })()"#,
    );
    assert_eq!(
        r.trim_matches('"'),
        ",foo,,false,test,html,SPAN,html:SPAN,html:SPAN,HTML:SPAN,true,true,\
         InvalidCharacterError,NamespaceError,InvalidCharacterError,NamespaceError,InvalidCharacterError,html,test,true,1,0,false"
    );
}

/// Namespaced attributes, getElementsByTagNameNS and the namespace lookups (X16).
#[test]
fn namespaced_attributes_and_lookups() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const XL = "http://www.w3.org/1999/xlink", XMLNS = "http://www.w3.org/2000/xmlns/";
            const el = document.createElementNS("urn:a", "p:el");
            el.setAttributeNS(XL, "xlink:Href", "#x");
            el.setAttributeNS(XMLNS, "xmlns:q", "urn:q");
            el.setAttribute("Plain", "1");
            out.push(el.getAttributeNS(XL, "Href"), el.getAttribute("xlink:Href"), el.hasAttributeNS(null, "Href"), el.getAttribute("Plain"));
            out.push(el.lookupNamespaceURI("p"), el.lookupNamespaceURI("q"), el.lookupPrefix("urn:a"), el.isDefaultNamespace(null));
            try { el.setAttributeNS(null, "a:b", "1"); } catch (e) { out.push(e.name); }
            el.removeAttributeNS(XL, "Href");
            out.push(el.hasAttribute("xlink:Href"));
            const d = document.createElement("div");
            d.setAttribute("ABC", "1");
            out.push(d.getAttributeNames().join("+"), d.getAttribute("aBc"));
            document.body.appendChild(el);
            out.push(document.getElementsByTagNameNS("urn:a", "el").length, document.getElementsByTagNameNS("*", "el").length, document.body.lookupNamespaceURI(null));
            return out.join(",");
        })()"##,
    );
    assert_eq!(
        r.trim_matches('"'),
        "#x,#x,false,1,urn:a,urn:q,p,true,NamespaceError,false,abc,1,1,1,http://www.w3.org/1999/xhtml"
    );
}

#[test]
fn template_inner_html_and_custom_property_order() {
    let _serial = serial();
    let _page = open("tests/sites/lit-template.html");
    assert_eq!(text_of("out"), "2;0;name/?hidden$lit$;true;rgb(0, 128, 0);rgb(0, 128, 0);rgb(0, 0, 255)");
}

#[test]
fn module_scripts_run_once_per_url() {
    let _serial = serial();
    let _page = open("tests/sites/module-once.html");
    assert_eq!(text_of("out"), "1;1;1");
}

#[test]
fn anchors_work_as_urls() {
    let _serial = serial();
    let _page = open("tests/sites/anchor-url.html");
    assert_eq!(
        text_of("out"),
        "https://example.com/news/world/x.html?q=1#top https: example.com example.com  /news/world/x.html ?q=1 #top https://example.com \
         u p site.test:8080 8080 https://u:p@site.test:8080/z?k=v#h \"\" \":\" \"\" undefined"
    );
}

/// DocumentType and document.doctype (The Guardian's consent script used DocumentType).
#[test]
fn document_type() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r#"(() => {
            const dt = document.doctype;
            return [typeof DocumentType, dt instanceof DocumentType, dt instanceof Node, dt.nodeType, dt.name, dt.publicId, dt.systemId,
                    dt.nodeName, document.firstChild === dt, Object.prototype.toString.call(dt), document.body instanceof DocumentType].join(",");
        })()"#,
    );
    assert_eq!(r.trim_matches('"'), "function,true,true,10,html,,,html,true,[object DocumentType],false");
}

#[test]
fn a_block_inside_an_inline_keeps_its_own_box() {
    let _serial = serial();
    let mut page = open("tests/sites/block-in-inline.html");
    let size = |page: &mut crate::Page, id: &str| eval(page, &format!("(() => {{ const r = document.getElementById('{id}').getBoundingClientRect(); return Math.round(r.width) + 'x' + Math.round(r.height); }})()"));
    // 98px wide, not the whole page (its height follows the picture once that has loaded).
    assert!(size(&mut page, "pic").starts_with("\"98x"), "{}", size(&mut page, "pic"));
    // width 120px + 2 × 10px padding, inside a link inside a span.
    assert!(size(&mut page, "boxed").starts_with("\"140x"), "{}", size(&mut page, "boxed"));
    // An inline holding only text is still an inline: it sits on the line with the words around it.
    assert_eq!(eval(&mut page, "document.getElementById('plain').getBoundingClientRect().left > 0"), "true");
}

/// BBC News startup: DocumentType exists, defaultValue can be set, "navigationStart" is a mark.
#[test]
fn doctype_default_value_and_navigation_marks() {
    let _serial = serial();
    let _page = open("tests/sites/bbc-startup.html");
    assert_eq!(text_of("out"), "object;object;object;true;10;b;y;0");
}

/// BBC News menu: absolutely positioned children don't widen a shrink-to-fit box.
#[test]
fn abspos_children_do_not_widen_shrink_to_fit() {
    let _serial = serial();
    let _page = open("tests/sites/abs-shrink.html");
}

/// WordPress.org: script.text and navigation timing entries.
#[test]
fn script_text_and_navigation_timing() {
    let _serial = serial();
    let _page = open("tests/sites/wp-bits.html");
    assert_eq!(text_of("out"), "true;two words;link;navigation;true;true");
}

#[test]
fn shrink_to_fit_items_and_flex_keywords() {
    let _serial = serial();
    let _page = open("tests/sites/wp-header.html");
    assert_eq!(text_of("out"), "true;true;true");
}

/// performance.navigation and performance.timing (The Guardian's tracker module stopped
/// halfway on performance.navigation.type, and its exports were then "not defined").
#[test]
fn legacy_navigation_timing() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        "[performance.navigation.type, performance.navigation.redirectCount, performance.navigation.TYPE_RELOAD, \
          performance.timing.navigationStart > 0, performance.timing.responseStart - performance.timing.connectEnd, \
          typeof performance.timing.loadEventStart].join(',')",
    );
    assert_eq!(r.trim_matches('"'), "0,0,1,true,0,number");
}

#[test]
fn content_below_a_full_height_root_scrolls() {
    let _serial = serial();
    let mut page = open("tests/sites/tall-root.html");
    let fonts = crate::load_fonts().expect("fonts");
    let lay = page.layout(fonts, 800.0, 600.0);
    assert!(lay.height > 3000.0 && lay.height < 4000.0, "page height {}", lay.height);
    // body's overflow belongs to the window: the link below the first screen isn't clipped.
    assert!(lay.links.iter().any(|(r, u)| u.ends_with("#end") && r.h > 0.0 && r.y >= 3000.0), "end link clipped");
}

/// BBC News headlines: font-weight 500 uses the family's 500 face, not the 400 one.
#[test]
fn numeric_font_weights_pick_the_closest_face() {
    let _serial = serial();
    let mut page = open("tests/font-weights.html");
    let w = |page: &mut crate::Page, id: &str| eval(page, &format!("document.getElementById('{id}').getBoundingClientRect().width"));
    let (w300, w400, w500, w600, short) = (w(&mut page, "w300"), w(&mut page, "w400"), w(&mut page, "w500"), w(&mut page, "w600"), w(&mut page, "short"));
    assert_eq!(w300, w400, "300 falls back to the 400 face");
    assert_ne!(w400, w500, "500 has its own face");
    assert_eq!(w500, w600, "600 falls back to the 500 face (no heavier one)");
    assert_eq!(w500, short, "the font shorthand's weight");
}

/// The content blocker's scriptlets and procedural cosmetic filters, run in a real page.
#[test]
fn content_blocking_scriptlets_and_procedural_filters() {
    let _serial = serial();
    let mut page = open("tests/content-blocking.html");
    let call = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let calls = vec![
        call(&["set-constant", "adBlock.detected", "false"]),
        call(&["set-constant", "showAds", "noopFunc"]),
        call(&["aopr", "adsLib"]),
        call(&["aopw", "blockerCheck"]),
        call(&["nostif", "adblock"]),
        call(&["json-prune", "ads data.[].promo"]),
        call(&["json-prune", "entries.[-].isAd"]),
        call(&["aeld", "click", "trackAd"]),
        call(&["remove-attr", "onclick", ".hero"]),
        call(&["remove-class", "wall"]),
        call(&["nowoif", "/popunder/"]),
    ];
    let procs = [".card:has-text(Sponsored)", ".label:has-text(Advertisement):upward(2)", ".slot:remove()", ".feed > .box:has(.label:has-text(News)):style(outline: 2px solid red)"];
    let code = crate::blocklist::code_for(&calls, &procs);
    let r = eval(&mut page, &code);
    assert!(!r.starts_with("error"), "{r}");
    if let Some(j) = page.js.as_mut() {
        j.settle(std::time::Duration::from_millis(500));
    }
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            window.adBlock = { detected: true };
            out.push(String(adBlock.detected));
            out.push(typeof showAds);
            try { window.adsLib; out.push("read"); } catch (e) { out.push(e.name); }
            try { window.blockerCheck = 1; out.push("wrote"); } catch (e) { out.push(e.name); }
            window.fired = "no";
            setTimeout(function () { /* adblock check */ window.fired = "adblock"; }, 0);
            setTimeout(function () { window.other = "ran"; }, 0);
            const j = JSON.parse('{"ads":[1],"data":[{"promo":1,"id":2}],"keep":3}');
            out.push(JSON.stringify(j).replace(/"/g, "'"));
            out.push(JSON.parse('{"entries":[{"id":1},{"id":2,"isAd":true}]}').entries.length);
            window.clicked = "";
            document.body.addEventListener("click", function () { /* trackAd */ window.clicked += "t"; });
            document.body.addEventListener("click", function () { window.clicked += "k"; });
            document.body.dispatchEvent(new Event("click"));
            out.push(window.clicked);
            out.push(String(window.open("https://x.example/popunder")));
            return out.join("|");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "false|function|ReferenceError|ReferenceError|{'data':[{'id':2}],'keep':3}|1|k|null");
    if let Some(j) = page.js.as_mut() {
        j.settle(std::time::Duration::from_millis(300));
    }
    let r = eval(
        &mut page,
        r##"(() => {
            const d = (id) => { const e = document.getElementById(id); return e ? (e.style.display || "-") : "gone"; };
            return [window.fired, window.other, d("c1"), d("c2"), d("b1"), d("b2"), d("s1"),
                document.getElementById("b2").style.outline ? "styled" : "plain",
                String(document.getElementById("h1").hasAttribute("onclick")), document.getElementById("w1").className || "-"].join("|");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "no|ran|-|none|none|-|gone|styled|false|-");
}

/// The Guardian's "Sign in": visibility: hidden content still takes its room.
#[test]
fn hidden_content_keeps_its_size() {
    let _serial = serial();
    let _page = open("tests/sites/hidden-takes-room.html");
    assert_eq!(text_of("out"), "true;true;true");
}

/// IntersectionObserverEntry's fields live on its prototype, so feature checks pass and pages
/// don't load the polling polyfill (The Guardian).
#[test]
fn intersection_entry_fields_are_on_the_prototype() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(&mut page, "['intersectionRatio' in IntersectionObserverEntry.prototype, new IntersectionObserverEntry({intersectionRatio: 0.5}).intersectionRatio].join(',')");
    assert_eq!(r.trim_matches('"'), "true,0.5");
}

/// BBC News section titles: trailing letter-spacing counts in a shrink-to-fit width.
#[test]
fn letter_spaced_titles_fit_their_flex_item() {
    let _serial = serial();
    let _page = open("tests/sites/letter-spacing-shrink.html");
    assert_eq!(text_of("out"), "20");
}

/// BBC News: an fr row in a grid of auto height is as tall as its items.
#[test]
fn fr_rows_hold_their_items_in_auto_height_grids() {
    let _serial = serial();
    let _page = open("tests/sites/grid-fr-rows.html");
    assert_eq!(text_of("next"), "180;true");
}

#[test]
fn custom_properties_inherit_in_layers() {
    let _serial = serial();
    let _page = open("tests/sites/many-vars.html");
    assert_eq!(text_of("out"), "899px;rgb(0, 128, 0);rgb(1, 2, 3);7px;5px");
}

/// The Guardian's "Print subscriptions": shrink-to-fit leaves room for inner blocks' right padding.
#[test]
fn shrink_to_fit_counts_inner_padding() {
    let _serial = serial();
    let _page = open("tests/sites/shrink-padding.html");
    assert_eq!(text_of("out"), "true;true;true");
}


/// More scriptlets, including the trusted ones uBlock Origin's own lists use.
#[test]
fn content_blocking_more_scriptlets() {
    let _serial = serial();
    let mut page = open("tests/content-blocking.html");
    let call = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let calls = vec![
        call(&["trusted-set-constant", "cfg.ads", "{\"on\":false}"]),
        call(&["href-sanitizer", "a.out", "text"]),
        call(&["spoof-css", ".adsbox", "display", "block", "height", "90px"]),
        call(&["prevent-innerHTML", "", "/adblock/i"]),
        call(&["trusted-replace-outbound-text", "getMsg", "/blocker/", "friend"]),
        call(&["trusted-replace-argument", "report", "0", "\"clean\""]),
        call(&["trusted-suppress-native-method", "console.warn", "/AdBlock/"]),
        call(&["trusted-create-html", ".feed", "<span id=\"made\">made</span>"]),
        call(&["trusted-prevent-dom-bypass", "Node.prototype.appendChild", "fetch"]),
        call(&["trusted-edit-inbound-object", "send", "0", "[?.attestationRequest][?.context.client.userAgent*=\"channel\"].context.client+={\"clientScreen\":\"CHANNEL\"}"]),
        call(&["trusted-edit-inbound-object", "send2", "0", "..ads"]),
        call(&["trusted-edit-inbound-object", "send3", "0", ".items[*][?.kind==\"ad\"].title=repl({\"regex\":\"Buy\",\"replacement\":\"Skip\"})"]),
    ];
    let code = crate::blocklist::code_for(&calls, &[]);
    eval(&mut page, "window.getMsg = function () { return 'hello blocker'; }; window.report = function (x) { window.reported = x; }; window.send = function (o) { window.sent = o; }; window.send2 = function (o) { window.sent2 = o; }; window.send3 = function (o) { window.sent3 = o; };");
    let r = eval(&mut page, &code);
    assert!(!r.starts_with("error"), "{r}");
    if let Some(j) = page.js.as_mut() {
        j.settle(std::time::Duration::from_millis(300));
    }
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            window.cfg = { ads: { on: true } };
            out.push(String(cfg.ads.on));
            out.push(document.getElementById("l1").getAttribute("href"));
            const b = document.getElementById("bait1");
            b.style.display = "none";
            out.push(getComputedStyle(b).display, getComputedStyle(b).getPropertyValue("height"));
            const d = document.createElement("div");
            d.innerHTML = "<b>AdBlock detected</b>";
            out.push(String(d.childNodes.length));
            out.push(getMsg());
            report("dirty");
            out.push(window.reported);
            out.push(String(console.warn("AdBlock found")));
            out.push(document.getElementById("made") ? "made" : "none");
            const kid = document.createElement("p");
            out.push(String(document.body.appendChild(kid) === kid));
            return out.join("|");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "false|https://bayou.example/story|block|90px|0|hello friend|clean|undefined|made|true");
    // trusted-edit-inbound-object: the filtered object is merged into; others are left alone.
    let r = eval(
        &mut page,
        r##"(() => {
            send({ attestationRequest: 1, context: { client: { userAgent: "x channel y" } } });
            const a = JSON.stringify(sent.context.client);
            send({ attestationRequest: 1, context: { client: { userAgent: "desktop" } } });
            const b = JSON.stringify(sent.context.client);
            send2({ a: { ads: [1], keep: 2 }, ads: 3 });
            const c = JSON.stringify(sent2);
            send3({ items: [{ kind: "ad", title: "Buy now" }, { kind: "post", title: "Buy later" }] });
            const d = sent3.items.map(i => i.title).join(",");
            return [a, b, c, d].join("|");
        })()"##,
    );
    assert_eq!(r.trim_matches('"').replace("\\\"", "'"), r#"{'userAgent':'x channel y','clientScreen':'CHANNEL'}|{'userAgent':'desktop'}|{'a':{'keep':2}}|Skip now,Buy later"#);
}

#[test]
fn text_and_comment_nodes_have_child_node_methods() {
    let _serial = serial();
    let _page = open("tests/sites/childnode-text.html");
    assert_eq!(text_of("out"), "a2<b></b>1<!--c-->x<u></u>Z<i></i> function");
}

/// matchMedia gives the stylesheet's answer: resolution queries included (The Guardian's
/// cards use `not all and (min-resolution: 0.001dpcm)` to pick out Safari).
#[test]
fn match_media_agrees_with_stylesheets() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(&mut page, "[matchMedia('not all and (min-resolution:0.001dpcm)').matches, matchMedia('(min-resolution: 1dppx)').matches, matchMedia('(min-width: 1px)').matches, matchMedia('print').matches].join(',')");
    assert_eq!(r.trim_matches('"'), "false,true,true,false");
}

#[test]
fn deep_recursion_with_many_locals() {
    let _serial = serial();
    let _page = open("tests/sites/deep-frames.html");
    assert_eq!(text_of("out"), "1500");
}

/// BBC News: align-content centres grid rows; percentage heights of grid items use their grid
/// area (a picture's adds nothing while rows are sized); padded links measure their padding.
#[test]
fn grid_align_content_areas_and_padded_links() {
    let _serial = serial();
    let _page = open("tests/sites/grid-align-content.html");
    assert_eq!(text_of("out"), "120;26;true");
}

#[test]
fn dates_parse_and_format_like_browsers() {
    let _serial = serial();
    let _page = open("tests/sites/dates.html");
    assert_eq!(
        text_of("parse"),
        "2026-11-17T22:33:56.000Z 2026-11-17T13:37:18.000Z 2026-11-17T18:37:18.123Z 2026-11-17T23:33:56.000Z \
         2026-11-17T15:00:00.000Z 2026-11-17T00:00:00.000Z NaN"
    );
    assert_eq!(
        text_of("fmt"),
        "11/17/2026, 17:33:56 EST | month=11,day=17,year=2026,hour=17,minute=33,second=56,timeZoneName=EST | \
         11/17/2026, 5:33:56 PM | Tuesday, November 17, 2026 | Nov 17, 2026, 2:33 PM | 1 PM GMT+01:00 | \
         Invalid Date | true | string | 5"
    );
}

/// The navigation entry is a PerformanceNavigationTiming, and first paints are recorded (The
/// Guardian waits 2.4 s for first-contentful-paint before starting when either is missing).
#[test]
fn navigation_and_paint_entries() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let ctx_layout = |page: &mut crate::Page| {
        let fonts = crate::load_fonts().expect("fonts");
        let _ = page.layout(fonts, 800.0, 600.0);
        if let Some(j) = page.js.as_mut() {
            j.after_layout();
        }
    };
    ctx_layout(&mut page);
    let r = eval(
        &mut page,
        "[performance.getEntriesByType('navigation')[0] instanceof PerformanceNavigationTiming, performance.getEntriesByType('paint').map(e => e.name).join('+'), PerformanceObserver.supportedEntryTypes.includes('paint')].join(',')",
    );
    assert_eq!(r.trim_matches('"'), "true,first-paint+first-contentful-paint,true");
}


/// WebGL 2 queries and fences (O4): occlusion samples are counted, the result and the fence
/// signal wait for the event loop, and misuse is reported.
#[test]
fn webgl2_queries_and_fences() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            const sh = (t, src) => { const s = gl.createShader(t); gl.shaderSource(s, src); gl.compileShader(s); return s; };
            const p = gl.createProgram();
            gl.attachShader(p, sh(gl.VERTEX_SHADER, "#version 300 es\nvoid main(){ gl_Position = vec4(0,0,0,1); gl_PointSize = 4.0; }"));
            gl.attachShader(p, sh(gl.FRAGMENT_SHADER, "#version 300 es\nprecision mediump float; out vec4 c; void main(){ c = vec4(1); }"));
            gl.linkProgram(p);
            gl.useProgram(p);
            const q = gl.createQuery();
            out.push(gl.isQuery(q));
            gl.beginQuery(gl.ANY_SAMPLES_PASSED, q);
            gl.beginQuery(gl.ANY_SAMPLES_PASSED_CONSERVATIVE, gl.createQuery());
            out.push(gl.getError() === gl.INVALID_OPERATION, gl.getQuery(gl.ANY_SAMPLES_PASSED, gl.CURRENT_QUERY) === q);
            const before = __glSamplesPassed(gl.__cid);
            gl.drawArrays(gl.POINTS, 0, 1);
            out.push(__glSamplesPassed(gl.__cid) - before);
            gl.endQuery(gl.ANY_SAMPLES_PASSED);
            out.push(gl.getQueryParameter(q, gl.QUERY_RESULT_AVAILABLE), gl.getQueryParameter(q, gl.QUERY_RESULT));
            gl.endQuery(gl.ANY_SAMPLES_PASSED);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            const f = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
            out.push(gl.getSyncParameter(f, gl.SYNC_STATUS) === gl.UNSIGNALED, gl.clientWaitSync(f, 0, 0) === gl.TIMEOUT_EXPIRED);
            gl.clientWaitSync(f, 0, 1);
            out.push(gl.getError() === gl.INVALID_OPERATION, gl.getError());
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "false,true,true,16,false,1,true,true,true,true,0");
}

/// WebGL 2 transform feedback (O4): a program's chosen outputs land in the bound buffer, a
/// primitives-written query counts them, and misuse is reported.
#[test]
fn webgl2_transform_feedback_captures() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            const sh = (t, src) => { const s = gl.createShader(t); gl.shaderSource(s, src); gl.compileShader(s); return s; };
            const p = gl.createProgram();
            gl.attachShader(p, sh(gl.VERTEX_SHADER, "#version 300 es\nin float a; out float twice; void main(){ twice = a * 2.0; gl_Position = vec4(a, 0, 0, 1); }"));
            gl.attachShader(p, sh(gl.FRAGMENT_SHADER, "#version 300 es\nprecision mediump float; out vec4 c; void main(){ c = vec4(1); }"));
            gl.transformFeedbackVaryings(p, ["twice"], gl.INTERLEAVED_ATTRIBS);
            gl.linkProgram(p);
            gl.useProgram(p);
            const vb = gl.createBuffer();
            gl.bindBuffer(gl.ARRAY_BUFFER, vb);
            gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([1, 2, 3]), gl.STATIC_DRAW);
            gl.enableVertexAttribArray(0);
            gl.vertexAttribPointer(0, 1, gl.FLOAT, false, 0, 0);
            const tb = gl.createBuffer();
            gl.bindBuffer(gl.TRANSFORM_FEEDBACK_BUFFER, tb);
            gl.bufferData(gl.TRANSFORM_FEEDBACK_BUFFER, 12, gl.STATIC_READ);
            const tf = gl.createTransformFeedback();
            gl.bindTransformFeedback(gl.TRANSFORM_FEEDBACK, tf);
            gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER, 0, tb);
            const q = gl.createQuery();
            gl.enable(gl.RASTERIZER_DISCARD);
            gl.beginQuery(gl.TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
            gl.beginTransformFeedback(gl.POINTS);
            gl.drawArrays(gl.LINES, 0, 2);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            gl.drawArrays(gl.POINTS, 0, 3);
            gl.drawArrays(gl.POINTS, 0, 1);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            gl.endTransformFeedback();
            gl.endQuery(gl.TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
            gl.bindTransformFeedback(gl.TRANSFORM_FEEDBACK, null);
            const got = new Float32Array(3);
            gl.getBufferSubData(gl.TRANSFORM_FEEDBACK_BUFFER, 0, got);
            out.push(Array.from(got).join(" "), q.__result, gl.getTransformFeedbackVarying(p, 0).name, gl.getError());
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "true,true,2 4 6,3,twice,0");
}

/// OES_draw_buffers_indexed: per-buffer blend and color mask; a WebGL 2 draw into an
/// active draw buffer the shader doesn't write is INVALID_OPERATION unless it's masked off.
#[test]
fn webgl2_draw_buffers_indexed() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            out.push(gl.getIndexedParameter(gl.COLOR_WRITEMASK, 0) === null && gl.getError() === gl.INVALID_ENUM);
            const ext = gl.getExtension("OES_draw_buffers_indexed");
            const fb = gl.createFramebuffer();
            gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
            for (const i of [0, 1]) {
                const t = gl.createTexture();
                gl.bindTexture(gl.TEXTURE_2D, t);
                gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
                gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0 + i, gl.TEXTURE_2D, t, 0);
            }
            gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);
            ext.colorMaskiOES(1, true, false, true, false);
            gl.clearColor(1, 1, 1, 1);
            gl.clear(gl.COLOR_BUFFER_BIT);
            const px = new Uint8Array(4);
            gl.readBuffer(gl.COLOR_ATTACHMENT1);
            gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
            out.push(Array.from(px).join(" "), gl.getIndexedParameter(gl.COLOR_WRITEMASK, 1).join(" "));
            ext.blendFunciOES(1, gl.CONSTANT_COLOR, gl.CONSTANT_ALPHA);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            ext.enableiOES(gl.BLEND, 8);
            out.push(gl.getError() === gl.INVALID_VALUE);
            const sh = (t, src) => { const s = gl.createShader(t); gl.shaderSource(s, src); gl.compileShader(s); return s; };
            const p = gl.createProgram();
            gl.attachShader(p, sh(gl.VERTEX_SHADER, "#version 300 es\nvoid main(){ gl_Position = vec4(0, 0, 0, 1); gl_PointSize = 1.0; }"));
            gl.attachShader(p, sh(gl.FRAGMENT_SHADER, "#version 300 es\nprecision mediump float; out vec4 c; void main(){ c = vec4(1); }"));
            gl.linkProgram(p);
            gl.useProgram(p);
            gl.drawArrays(gl.POINTS, 0, 1);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            ext.colorMaskiOES(1, false, false, false, false);
            gl.drawArrays(gl.POINTS, 0, 1);
            out.push(gl.getError());
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "true,255 0 255 0,true false true false,true,true,true,0");
}

/// sRGB framebuffers encode clears and decode on blits to linear targets; blitFramebuffer
/// checks its rectangle sizes.
#[test]
fn webgl2_srgb_clear_and_blit() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            const target = (fmt) => {
                const t = gl.createTexture();
                gl.bindTexture(gl.TEXTURE_2D, t);
                gl.texImage2D(gl.TEXTURE_2D, 0, fmt, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
                const fb = gl.createFramebuffer();
                gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
                gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
                return fb;
            };
            const px = new Uint8Array(4);
            const s = target(gl.SRGB8_ALPHA8);
            gl.clearColor(0.5, 0.5, 0.5, 1);
            gl.clear(gl.COLOR_BUFFER_BIT);
            gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
            out.push(px[0]);
            const l = target(gl.RGBA8);
            gl.bindFramebuffer(gl.READ_FRAMEBUFFER, s);
            gl.blitFramebuffer(0, 0, 2, 2, 0, 0, 2, 2, gl.COLOR_BUFFER_BIT, gl.LINEAR);
            gl.bindFramebuffer(gl.READ_FRAMEBUFFER, l);
            gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
            out.push(px[0], gl.getError());
            gl.blitFramebuffer(-1, 0, 0x7fffffff, 2, 0, 0, 2, 2, gl.COLOR_BUFFER_BIT, gl.NEAREST);
            out.push(gl.getError() === gl.INVALID_VALUE);
            gl.blitFramebuffer(0, 0, 2, 2, 0, 0, 2, 2, gl.DEPTH_BUFFER_BIT, gl.LINEAR);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "188,128,0,true,true");
}

/// Multisampled renderbuffers: the one supported sample count, completeness, no direct
/// readPixels, resolve by blit; renderbuffer depth starts at 1.
#[test]
fn webgl2_multisample_renderbuffers() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            out.push(Array.from(gl.getInternalformatParameter(gl.RENDERBUFFER, gl.RGBA8, gl.SAMPLES)).join(" "),
                     gl.getInternalformatParameter(gl.RENDERBUFFER, gl.RGBA8UI, gl.SAMPLES).length,
                     gl.getParameter(gl.MAX_SAMPLES));
            const rb = gl.createRenderbuffer();
            gl.bindRenderbuffer(gl.RENDERBUFFER, rb);
            gl.renderbufferStorageMultisample(gl.RENDERBUFFER, 2, gl.RGBA8, 2, 2);
            out.push(gl.getRenderbufferParameter(gl.RENDERBUFFER, gl.RENDERBUFFER_SAMPLES));
            gl.renderbufferStorageMultisample(gl.RENDERBUFFER, 1, gl.DEPTH_STENCIL, 2, 2);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            const ms = gl.createFramebuffer();
            gl.bindFramebuffer(gl.FRAMEBUFFER, ms);
            gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, rb);
            const d = gl.createRenderbuffer();
            gl.bindRenderbuffer(gl.RENDERBUFFER, d);
            gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, 2, 2);
            gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, d);
            out.push(gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
            gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, null);
            gl.clearColor(0, 1, 0, 1);
            gl.clear(gl.COLOR_BUFFER_BIT);
            const px = new Uint8Array(4);
            gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
            gl.blitFramebuffer(0, 0, 2, 2, 0, 0, 2, 2, gl.COLOR_BUFFER_BIT, gl.NEAREST);
            gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
            gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
            out.push(Array.from(px).join(" "), gl.getError());
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "4,0,4,4,true,true,true,0 255 0 255,0");
}

/// Pixel buffers: readPixels into PIXEL_PACK_BUFFER, texImage from PIXEL_UNPACK_BUFFER, the
/// "bound or not" rules; reading a layer of a 3D texture through framebufferTextureLayer.
#[test]
fn webgl2_pixel_buffers_and_layers() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            const t = gl.createTexture();
            gl.bindTexture(gl.TEXTURE_3D, t);
            const up = gl.createBuffer();
            gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, up);
            const data = new Uint8Array(2 * 1 * 2 * 4);
            for (let i = 0; i < data.length; i++) data[i] = i * 10;
            gl.bufferData(gl.PIXEL_UNPACK_BUFFER, data, gl.STATIC_DRAW);
            gl.texImage3D(gl.TEXTURE_3D, 0, gl.RGBA8, 2, 1, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            gl.texImage3D(gl.TEXTURE_3D, 0, gl.RGBA8, 2, 1, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, 0);
            out.push(gl.getError());
            gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, null);
            const fb = gl.createFramebuffer();
            gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
            gl.framebufferTextureLayer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, t, 0, 1);
            const pack = gl.createBuffer();
            gl.bindBuffer(gl.PIXEL_PACK_BUFFER, pack);
            gl.bufferData(gl.PIXEL_PACK_BUFFER, 8, gl.STREAM_READ);
            gl.readPixels(0, 0, 2, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(8));
            out.push(gl.getError() === gl.INVALID_OPERATION);
            gl.readPixels(0, 0, 2, 1, gl.RGBA, gl.UNSIGNED_BYTE, 0);
            const got = new Uint8Array(8);
            gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, got);
            out.push(got.join(" "), gl.getError());
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "true,0,true,80 90 100 110 120 130 140 150,0");
}

#[test]
fn flex_items_overflow_a_definite_height_line() {
    let _serial = serial();
    let _page = open("tests/sites/flex-overflow-end.html");
    // flex-end hangs up from the 0-height line; center spills both ways; stretch keeps its
    // content height.
    assert_eq!(text_of("out"), "60-100 190-230 320-360");
}

/// WebGL 2 DOM-source uploads: UNPACK_SKIP_* with an explicit size take a sub-rectangle
/// (of the flipped source with UNPACK_FLIP_Y), and unpackColorSpace "display-p3" converts.
#[test]
fn webgl2_dom_source_subrect_and_p3() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const c = document.createElement("canvas"); c.width = 4; c.height = 4;
            const x = c.getContext("2d");
            x.fillStyle = "#ff0000"; x.fillRect(0, 0, 4, 2);
            x.fillStyle = "#00ff00"; x.fillRect(0, 2, 4, 2);
            const gl = document.createElement("canvas").getContext("webgl2");
            const fb = gl.createFramebuffer();
            const read = (t) => {
                gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
                gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
                const px = new Uint8Array(4);
                gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
                return px.slice(0, 3).join(" ");
            };
            const upload = (flip, space) => {
                const t = gl.createTexture();
                gl.bindTexture(gl.TEXTURE_2D, t);
                gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, flip);
                gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 2);
                gl.unpackColorSpace = space;
                gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 4, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, c);
                gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 0);
                return read(t);
            };
            out.push(upload(false, "srgb"), upload(true, "srgb"), upload(true, "display-p3"));
            gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
            gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 3);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 4, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, c);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "0 255 0,255 0 0,234 51 35,true");
}

/// createImageBitmap options (imageOrientation, premultiplyAlpha, source rectangle) reach
/// WebGL uploads, which ignore UNPACK_FLIP_Y for bitmaps; an ImageData bitmap keeps the color
/// of transparent pixels.
#[test]
fn image_bitmap_options_reach_webgl() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    eval(
        &mut page,
        r##"(() => {
            const d = new ImageData(1, 2);
            d.data.set([255, 0, 0, 0, 0, 255, 0, 128]);
            Promise.all([
                createImageBitmap(d, { premultiplyAlpha: "none" }),
                createImageBitmap(d, { imageOrientation: "flipY", premultiplyAlpha: "premultiply" }),
                createImageBitmap(d, 0, 1, 1, 1),
            ]).then((bs) => { window.__bitmaps = bs; });
        })()"##,
    );
    page.js.as_mut().unwrap().settle(std::time::Duration::from_secs(1));
    let r = eval(
        &mut page,
        r##"(() => {
            const gl = document.createElement("canvas").getContext("webgl2");
            const fb = gl.createFramebuffer();
            gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
            return window.__bitmaps.map((b) => {
                const t = gl.createTexture();
                gl.bindTexture(gl.TEXTURE_2D, t);
                gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, b);
                gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
                gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
                const px = new Uint8Array(4);
                gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
                return b.width + "x" + b.height + ":" + px.join(" ");
            }).join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "1x2:255 0 0 0,1x2:0 128 0 128,1x1:0 255 0 128");
}

/// Compressed textures: the ETC/RGTC extensions, uploads from a pixel buffer and from shared
/// memory, and the block-size and sub-image rules.
#[test]
fn webgl2_compressed_texture_rules() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            const s3tc = gl.getExtension("WEBGL_compressed_texture_s3tc");
            const etc1 = gl.getExtension("WEBGL_compressed_texture_etc1");
            const etc = gl.getExtension("WEBGL_compressed_texture_etc");
            const rgtc = gl.getExtension("EXT_texture_compression_rgtc");
            out.push(!!(etc1 && etc && rgtc));
            gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
            gl.compressedTexImage2D(gl.TEXTURE_2D, 0, etc.COMPRESSED_RGBA8_ETC2_EAC, 4, 4, 0, new Uint8Array(new SharedArrayBuffer(16)));
            out.push(gl.getError());
            gl.compressedTexImage2D(gl.TEXTURE_2D, 0, s3tc.COMPRESSED_RGBA_S3TC_DXT5_EXT, 3, 4, 0, new Uint8Array(16));
            out.push(gl.getError() === gl.INVALID_OPERATION);
            gl.compressedTexImage2D(gl.TEXTURE_2D, 0, etc1.COMPRESSED_RGB_ETC1_WEBGL, 4, 4, 0, new Uint8Array(8));
            gl.compressedTexSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 4, 4, etc1.COMPRESSED_RGB_ETC1_WEBGL, new Uint8Array(8));
            out.push(gl.getError() === gl.INVALID_OPERATION);
            gl.generateMipmap(gl.TEXTURE_2D);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            const pbo = gl.createBuffer();
            gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, pbo);
            gl.bufferData(gl.PIXEL_UNPACK_BUFFER, 16, gl.STATIC_DRAW);
            gl.compressedTexImage2D(gl.TEXTURE_2D, 0, rgtc.COMPRESSED_RED_GREEN_RGTC2_EXT, 4, 4, 0, 16, 0);
            out.push(gl.getError());
            gl.compressedTexImage2D(gl.TEXTURE_2D, 0, rgtc.COMPRESSED_RED_GREEN_RGTC2_EXT, 4, 4, 0, 16, 1);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "true,0,true,true,true,0,true");
}


/// Drawing while sampling a texture level that is also the render target is
/// INVALID_OPERATION; a mutable texture's attached level must be in [base, q]; immutable
/// textures clamp TEXTURE_BASE_LEVEL.
#[test]
fn webgl2_feedback_loops_and_level_rules() {
    let _serial = serial();
    let mut page = open("tests/observers.html");
    let r = eval(
        &mut page,
        r##"(() => {
            const out = [];
            const gl = document.createElement("canvas").getContext("webgl2");
            const sh = (t, src) => { const s = gl.createShader(t); gl.shaderSource(s, src); gl.compileShader(s); return s; };
            const p = gl.createProgram();
            gl.attachShader(p, sh(gl.VERTEX_SHADER, "#version 300 es\nvoid main(){ gl_Position = vec4(0, 0, 0, 1); gl_PointSize = 1.0; }"));
            gl.attachShader(p, sh(gl.FRAGMENT_SHADER, "#version 300 es\nprecision mediump float; uniform sampler2D t; out vec4 c; void main(){ c = texture(t, vec2(0)); }"));
            gl.linkProgram(p);
            gl.useProgram(p);
            const t = gl.createTexture();
            gl.bindTexture(gl.TEXTURE_2D, t);
            gl.texStorage2D(gl.TEXTURE_2D, 2, gl.RGBA8, 2, 2);
            const fb = gl.createFramebuffer();
            gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
            gl.drawArrays(gl.POINTS, 0, 1);
            out.push(gl.getError() === gl.INVALID_OPERATION);
            // Sampling only level 1 (base 5 clamps to 1) while drawing into level 0 is fine.
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_BASE_LEVEL, 5);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
            gl.drawArrays(gl.POINTS, 0, 1);
            out.push(gl.getError());
            const m = gl.createTexture();
            gl.bindTexture(gl.TEXTURE_2D, m);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
            gl.texImage2D(gl.TEXTURE_2D, 1, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, 0);
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, m, 1);
            out.push(gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, 1);
            out.push(gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE);
            return out.join(",");
        })()"##,
    );
    assert_eq!(r.trim_matches('"'), "true,0,true,true");
}

/// NYT: `--x: ;` is a valid, empty custom property ("space toggle"), so `var(--x, fallback)`
/// gives nothing, not the fallback. The "free to read" box's border color depends on it.
#[test]
fn empty_custom_property_space_toggle() {
    let _serial = serial();
    let mut page = open("tests/sites/space-toggle.html");
    let border = |page: &mut crate::Page, id: &str| eval(page, &format!("getComputedStyle(document.getElementById('{id}')).borderTopColor"));
    assert_eq!(border(&mut page, "light"), "\"rgb(18, 18, 18)\"");
    assert_eq!(border(&mut page, "dark"), "\"rgb(248, 248, 248)\"");
}

/// NYT: createComment() made an empty text node and `new Text()`/`new Comment()` weren't
/// nodes, so Svelte's hydration (comment anchors) broke.
#[test]
fn comments_and_text_constructors_make_nodes() {
    let _serial = serial();
    let _page = open("tests/sites/comment-nodes.html");
    assert_eq!(text_of("out"), "8#comment[|<!--[0--><span>a</span><!--]--><!--[--><!--]-->|3xtruetrue|8ytrue|sibling|needs new");
}

/// NYT: scripts parsed into a <template>'s contents ran; only a clone put into the document
/// should run (once).
#[test]
fn template_scripts_are_inert() {
    let _serial = serial();
    let _page = open("tests/sites/template-scripts.html");
    assert_eq!(text_of("out"), "plain,contents:1");
}

/// NYT: innerHTML parses with the element as context, so `<tr>` survives in a <template> or
/// <tbody> (and is still dropped in a <div>).
#[test]
fn inner_html_uses_its_element_as_context() {
    let _serial = serial();
    let _page = open("tests/sites/innerhtml-context.html");
    assert_eq!(text_of("out"), "TR2|TD|1<tr><td>c</td></tr>|d");
}

// Causes found running WebAssembly apps and WPT wasm/ (task P1d); each page says where from.

#[test]
fn wasm_atomics_without_shared_memory() {
    let _serial = serial();
    let _page = open("tests/sites/wasm-atomics.html");
    assert_eq!(text_of("out"), "true,40,42,255,263,7,263,0,5,42,0,false");
}

#[test]
fn text_decoder_knows_utf16() {
    let _serial = serial();
    let _page = open("tests/sites/emscripten-utf16.html");
    assert_eq!(text_of("out"), "utf-16le,Wasm,OK,euc-kr");
}

#[test]
fn function_sees_a_later_scripts_let() {
    let _serial = serial();
    let _page = open("tests/sites/global-let-later.html");
    assert_eq!(text_of("out"), "number:7");
}

#[test]
fn module_worker_runs_an_es_module() {
    let _serial = serial();
    let mut page = open("tests/sites/module-worker.html");
    if let Some(j) = page.js.as_mut() {
        j.settle(std::time::Duration::from_secs(3));
    }
    assert_eq!(text_of("out"), "42 TypeError");
}

/// NYT: getComputedStyle sees an animation where it is now (here finished, held by
/// `forwards`), not at 0 s.
#[test]
fn computed_style_follows_running_animations() {
    let _serial = serial();
    let mut page = open("tests/sites/anim-fill.html");
    let op = |page: &mut crate::Page| eval(page, "getComputedStyle(document.getElementById('box')).opacity");
    op(&mut page);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(op(&mut page), "\"1\"");
}

/// NYT: document.adoptedStyleSheets and ShadowRoot.adoptedStyleSheets: drawn in their own
/// scope, kept up to date when a sheet changes; `instanceof ShadowRoot` works.
#[test]
fn adopted_style_sheets_apply() {
    let _serial = serial();
    let mut page = open("tests/sites/adopted-sheets.html");
    if let Some(j) = page.js.as_mut() {
        j.settle(std::time::Duration::from_secs(1));
    }
    assert_eq!(text_of("out"), "true:0|true|true|true|rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}

/// NYT: NodeList and HTMLCollection have their own prototypes, so tagging them (core-js)
/// leaves arrays alone.
#[test]
fn dom_collections_have_their_own_prototypes() {
    let _serial = serial();
    let _page = open("tests/sites/dom-collections.html");
    assert_eq!(text_of("out"), "[object Array]|true|true|true|true|function");
}
