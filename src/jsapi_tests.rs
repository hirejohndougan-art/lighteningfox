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
