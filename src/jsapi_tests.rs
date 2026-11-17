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
    assert_eq!(lines[5], "one,3,c,,nodoctype,two three");
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
