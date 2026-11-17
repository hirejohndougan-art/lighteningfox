//! Tests for the JavaScript APIs pages use: load a test page from `tests/`, let its
//! scripts run, then check what they did to the document.

use crate::js;

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
    let _page = open("tests/modules.html");
    assert_eq!(text_of("out"), "hello modules 42 7 modules.html");
    assert_eq!(text_of("dyn"), "dyn 42");
    // Modules wait for the parser, like deferred scripts.
    assert_eq!(text_of("order"), "classic;classic2;module;");
    let _ = eval;
}

#[test]
fn custom_elements() {
    let _page = open("tests/custom-elements.html");
    assert_eq!(
        text_of("log"),
        "name:null>Ada;connected Ada;upgraded true true;created true;name:null>Grace;connected Grace;\
         name:null>Linus;connected Linus;disconnected Linus;name:null>Alan;connected Alan;bad SyntaxError get true;whenDefined;"
    );
}

#[test]
fn intersection_and_resize_observers() {
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
