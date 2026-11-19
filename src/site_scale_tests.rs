//! Pages in site processes on a HiDPI screen: the browser sends its screen scale with each page
//! and resize, since a site process has no window to ask (Google's pages showed at half size on
//! a 200% Windows screen while about:home, drawn in the browser, was right).

use crate::frames::Event;
use crate::zoom;

#[test]
fn resize_events_carry_the_screen_scale() {
    let ev = Event::Resize { w: 1400, h: 900, scale: 2.0 };
    let bytes = ev.encode();
    assert_eq!(Event::decode(&mut &bytes[..]).expect("decodes"), ev);
}

#[test]
fn a_site_process_draws_at_the_scale_it_was_told() {
    // Before it's told (and with no window), 1; then what the browser sent, kept in 1..=4.
    // Set back to 1 so other tests in this process see what they expect.
    zoom::set_render_scale(2.0);
    assert_eq!(zoom::device_scale(), 2.0);
    zoom::set_render_scale(9.0);
    assert_eq!(zoom::device_scale(), 4.0);
    zoom::set_render_scale(f32::NAN);
    assert_eq!(zoom::device_scale(), 4.0, "nonsense is ignored");
    zoom::set_render_scale(1.0);
    assert_eq!(zoom::device_scale(), 1.0);
}
