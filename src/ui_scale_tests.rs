//! The browser UI on a HiDPI screen: the toolbar is drawn in logical pixels times the screen's
//! scale, as pages are, so it isn't half size on a 200% screen (John's Windows laptop).

use crate::{PagePaint, TOOL_BOTTOM, Toolbar, draw, images, layout, paint::Canvas, zoom};

/// The toolbar drawn into a window `w` × `h` logical pixels at `scale`.
fn render(scale: f32, w: usize, h: usize) -> Canvas {
    zoom::TEST_SCALE.with(|c| c.set(Some(scale)));
    let fonts = crate::load_fonts().expect("fonts");
    let images = images::Images::default();
    let lay = layout::Layout::default();
    let tabs = vec![("Ironswift".to_string(), false), ("Second tab".to_string(), false)];
    let icons = vec![None, None];
    let ui = Toolbar {
        tabs: &tabs, icons: &icons, active: 0, tab_hover: None, caret: None, bookmarked: false, bookmarks: &[], bm_hover: None,
        downloads: None, address: "about:home", editing: false, can_back: true, can_forward: false, hover: None, loading: None,
        zoom: 1.0, shield: Some((3, true)),
    };
    let mut canvas = Canvas::new((w as f32 * scale) as usize, (h as f32 * scale) as usize);
    draw(&mut canvas, fonts, &images, &lay, 0.0, &ui, PagePaint::Paint);
    zoom::TEST_SCALE.with(|c| c.set(None));
    canvas
}

fn save(c: &Canvas, path: &str) {
    let img = image::RgbImage::from_fn(c.w as u32, c.h as u32, |x, y| {
        let p = c.buf[y as usize * c.w + x as usize];
        image::Rgb([(p >> 16) as u8, (p >> 8) as u8, p as u8])
    });
    let _ = std::fs::create_dir_all("target/ui-scale");
    img.save(path).expect("png");
}

/// Dark pixels (text, glyphs) in rows y0..y1.
fn ink(c: &Canvas, y0: usize, y1: usize) -> usize {
    c.buf[y0 * c.w..y1 * c.w].iter().filter(|&&p| ((p >> 16) & 255) + ((p >> 8) & 255) + (p & 255) < 300).count()
}

#[test]
fn the_toolbar_scales_with_the_screen() {
    let one = render(1.0, 900, 120);
    let two = render(2.0, 900, 120);
    save(&one, "target/ui-scale/100.png");
    save(&two, "target/ui-scale/200.png");
    let px = |c: &Canvas, x: usize, y: usize| c.buf[y * c.w + x] & 0xFFFFFF;
    // The line under the toolbar: one pixel at 100%, two at 200%, twice as far down.
    let line = 0xC8C8CD;
    assert_eq!(px(&one, 450, TOOL_BOTTOM - 1), line);
    assert_ne!(px(&one, 450, TOOL_BOTTOM - 2), line);
    assert_eq!(px(&two, 900, 2 * TOOL_BOTTOM - 1), line);
    assert_eq!(px(&two, 900, 2 * TOOL_BOTTOM - 2), line);
    assert_ne!(px(&two, 900, 2 * TOOL_BOTTOM - 3), line);
    // The text (tab titles, address, buttons) is drawn at twice the size: about four times the
    // dark pixels, not the same count squeezed into the top half.
    let (a, b) = (ink(&one, 0, TOOL_BOTTOM), ink(&two, 0, 2 * TOOL_BOTTOM));
    assert!(a > 50 && (3 * a..6 * a).contains(&b), "ink at 100%: {a}, at 200%: {b}");
    // The page starts under the scaled toolbar, not under the 100% one.
    zoom::TEST_SCALE.with(|c| c.set(Some(2.0)));
    assert_eq!(crate::page_top(), 2 * TOOL_BOTTOM);
    zoom::TEST_SCALE.with(|c| c.set(None));
}
