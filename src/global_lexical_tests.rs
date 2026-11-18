//! A function from one script sees a global `let`/`const` a later script declares (a Boa
//! fix in `vendor/boa`, found by WPT `wasm/jsapi`: `instanceTestFactory.js` reads a `let`
//! the test file declares after it).

#[test]
fn later_global_let_is_visible_to_earlier_functions() {
    let mut js = crate::js::Js::new("https://scope.example/");
    js.run("function read() { return typeof later + ':' + later } function write(v) { later = v } function bump() { fixed = 1 }").unwrap();
    js.run("let later = 3; const fixed = 2;").unwrap();
    assert_eq!(js.run("read()").unwrap(), "\"number:3\"");
    js.run("write(9)").unwrap();
    assert_eq!(js.run("later + ':' + ('later' in globalThis)").unwrap(), "\"9:false\"");
    assert!(js.run("bump()").unwrap_err().contains("constant"));
    // Still a ReferenceError before it's initialized, and for names never declared.
    js.run("function early() { try { return tdz } catch (e) { return e.name } }").unwrap();
    assert_eq!(js.run("early()").unwrap(), "\"ReferenceError\"");
    assert_eq!(js.run("try { nowhere } catch (e) { e.name }").unwrap(), "\"ReferenceError\"");
}
