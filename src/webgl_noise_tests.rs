//! X32: fingerprinting noise on WebGL readouts, now that tests opt into it
//! (`security::TEST_NOISE`) instead of getting it at random positions every run.

use crate::jsapi_tests::{open, serial};

/// Clears a 64×64 framebuffer to mid-grey and reads it back twice: how many colour values
/// aren't 128, the largest difference, and whether both reads agree.
const READ_GREY: &str = r##"(() => {
    const c = document.createElement("canvas"); c.width = 64; c.height = 64;
    const gl = c.getContext("webgl2");
    gl.clearColor(0.5, 0.5, 0.5, 1); gl.clear(gl.COLOR_BUFFER_BIT);
    const read = () => { const px = new Uint8Array(64 * 64 * 4); gl.readPixels(0, 0, 64, 64, gl.RGBA, gl.UNSIGNED_BYTE, px); return px; };
    const a = read(), b = read();
    let changed = 0, most = 0;
    for (let i = 0; i < a.length; i++) {
        if (i % 4 === 3) continue;
        const d = Math.abs(a[i] - 128);
        if (d) changed++;
        most = Math.max(most, d);
    }
    return changed + "," + most + "," + a.every((v, i) => v === b[i]);
})()"##;

fn grey_readout(noise: bool) -> (usize, u32, bool) {
    crate::security::TEST_NOISE.with(|n| n.set(noise));
    let mut page = open("tests/observers.html");
    let r = page.js.as_mut().expect("scripts").run(READ_GREY).expect("runs");
    crate::security::TEST_NOISE.with(|n| n.set(false));
    let parts: Vec<String> = r.trim_matches('"').split(',').map(String::from).collect();
    (parts[0].parse().expect("count"), parts[1].parse().expect("most"), parts[2] == "true")
}

#[test]
fn webgl_readouts_get_fingerprinting_noise_when_on() {
    let _g = serial();
    let (changed, most, same) = grey_readout(true);
    // 12,288 colour values, about one in a hundred nudged by one.
    assert!((20..400).contains(&changed), "{changed} values changed");
    assert_eq!(most, 1, "never more than one step");
    assert!(same, "the same page reads the same values twice");
}

#[test]
fn webgl_readouts_are_exact_without_noise() {
    let _g = serial();
    assert_eq!(grey_readout(false), (0, 0, true));
}
