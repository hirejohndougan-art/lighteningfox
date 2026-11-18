//! X31: MutationObserver records after the hooks stopped copying child lists (the neighbours
//! come from `__sibling`, and changes nobody watches skip the work).

use crate::jsapi_tests::{open, serial, text_of};

#[test]
fn records_name_the_right_neighbours_and_filters() {
    let _g = serial();
    let _page = open("tests/sites/mutation-records.html");
    let out = text_of("out");
    let (all, rest) = out.split_once(" || ONLY ").expect("three lists");
    let (only, loose) = rest.split_once(" || LOOSE ").expect("three lists");
    assert_eq!(
        all.split(" | ").collect::<Vec<_>>(),
        [
            "childList@a[+n - prev a2 next -]",
            // insertBefore of a2 before a1: out with its old neighbours, in at the front.
            "childList@a[+ -a2 prev a1 next n]",
            "childList@a[+a2 - prev - next a1]",
            // a1 moves to b: removed from a, added to b.
            "childList@a[+ -a1 prev a2 next n]",
            "childList@b[+a1 - prev b1 next -]",
            "childList@b[+ -b1 prev - next a1]",
            // attributeFilter: class only, with old values; the title change isn't there.
            "attributes@n[class=null]",
            "attributes@n[class=x]",
        ]
    );
    assert_eq!(only, "childList@b[+a1 - prev b1 next -] | childList@b[+ -b1 prev - next a1]", "no subtree: b only");
    assert_eq!(loose, "childList@DIV[+k - prev - next -]", "an observer on a detached element");
}

#[test]
fn appends_to_a_watched_parent_stay_linear() {
    let _g = serial();
    let _page = open("tests/sites/mutation-records.html");
    let timing = text_of("timing");
    assert!(timing.ends_with("children 4000"), "{timing}");
    let ms: u64 = timing.split_whitespace().nth(1).and_then(|n| n.parse().ok()).expect("a time");
    // Quadratic (each append copying the child list) took several seconds here; linear takes
    // well under one, even with every function compiled first (BOA_JIT=always in CI).
    assert!(ms < 3000, "4000 watched appends took {ms} ms");
}
