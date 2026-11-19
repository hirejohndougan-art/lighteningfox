//! X33: every built-in script a page's JavaScript is set up with goes through the compiled-
//! script cache, so only a thread's first page parses them. webgl.js, media.js and the rest were
//! evaluated without it: a page's setup took ~70 ms warm, ~18 ms now (140 ms cold either way).

#[test]
fn built_in_scripts_are_compiled_once_per_thread() {
    std::thread::Builder::new()
        .stack_size(crate::MAIN_STACK)
        .spawn(|| {
            drop(crate::js::Js::new("https://example.com/"));
            for (name, src) in [
                ("webgl.js", include_str!("webgl.js")),
                ("media.js", include_str!("media.js")),
                ("webaudio.js", include_str!("webaudio.js")),
                ("mse.js", include_str!("mse.js")),
                ("canvas.js", include_str!("canvas.js")),
                ("dommatrix.js", include_str!("dommatrix.js")),
                ("img.js", include_str!("img.js")),
            ] {
                assert!(boa_engine::script_cache::contains(src.as_bytes()), "{name} isn't cached");
            }
            // A second page on the thread parses none of the big ones again.
            let (_, misses, _) = boa_engine::script_cache::stats();
            drop(crate::js::Js::new("https://example.org/"));
            assert_eq!(boa_engine::script_cache::stats().1, misses, "a second page parsed built-in scripts again");
        })
        .unwrap()
        .join()
        .unwrap();
}
