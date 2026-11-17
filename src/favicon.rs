//! Site icons for the tab strip. Each icon is fetched once per address on a background
//! thread (the page's `<link rel="icon">`, else `/favicon.ico`), shrunk to 16×16 and
//! kept for the rest of the run. Tabs draw nothing until it arrives.

use crate::dom::{Dom, NodeData};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const SIZE: usize = 16;

/// 16×16 pixels, 0xAARRGGBB.
pub type Icon = Arc<Vec<u32>>;

/// None while loading or when the site has no usable icon.
fn cache() -> &'static Mutex<HashMap<String, Option<Icon>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<Icon>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

static ARRIVED: AtomicBool = AtomicBool::new(false);

/// An icon finished loading since the last call (the window redraws its tabs).
pub fn arrived() -> bool {
    ARRIVED.swap(false, Ordering::Relaxed)
}

/// Where a page's icon lives: its last `<link rel="icon">` with an href (browsers use the
/// last one), else /favicon.ico on the page's origin. None for pages without an origin.
pub fn icon_url(dom: &Dom, page_url: &str) -> Option<String> {
    if !page_url.starts_with("http://") && !page_url.starts_with("https://") {
        return None;
    }
    let mut found = None;
    for n in dom.descendants(Dom::ROOT) {
        if let NodeData::Element { tag, attrs } = &dom.nodes[n].data {
            if tag != "link" {
                continue;
            }
            let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
            let rel = get("rel").unwrap_or("").to_ascii_lowercase();
            if !rel.split_whitespace().any(|r| r == "icon") {
                continue;
            }
            // SVG icons can't be decoded here; prefer a raster one.
            if get("type").is_some_and(|t| t.contains("svg")) || get("href").is_some_and(|h| h.ends_with(".svg")) {
                continue;
            }
            if let Some(h) = get("href").filter(|h| !h.trim().is_empty()) {
                found = Some(crate::net::resolve(page_url, h.trim()));
            }
        }
    }
    found.or_else(|| Some(format!("{}/favicon.ico", crate::net::origin_of(page_url))))
}

/// The icon at `url`: Some when loaded; starts fetching it the first time it's asked for.
pub fn get(url: &str, page_url: &str) -> Option<Icon> {
    {
        let mut c = cache().lock().unwrap();
        if let Some(v) = c.get(url) {
            return v.clone();
        }
        c.insert(url.to_string(), None);
    }
    let (url, page) = (url.to_string(), page_url.to_string());
    std::thread::spawn(move || {
        let icon = crate::net::fetch_sub_bytes(&url, &page).ok().and_then(|(_, bytes)| decode(&bytes));
        if let Some(icon) = icon {
            cache().lock().unwrap().insert(url, Some(icon));
            ARRIVED.store(true, Ordering::Relaxed);
        }
    });
    None
}

/// Decode an ICO/PNG/GIF/… and shrink it to 16×16. An .ico holds several sizes; the
/// decoder picks the largest, which shrinks cleanly.
fn decode(bytes: &[u8]) -> Option<Icon> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.resize_exact(SIZE as u32, SIZE as u32, image::imageops::FilterType::Triangle).to_rgba8();
    let px = img.pixels().map(|p| (p[3] as u32) << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
    Some(Arc::new(px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_url_prefers_link_then_favicon_ico() {
        let dom = crate::html::parse(r#"<head><link rel="shortcut icon" href="/a.png"><link rel="icon" href="b.svg"></head>"#);
        assert_eq!(icon_url(&dom, "https://ex.com/x/y").as_deref(), Some("https://ex.com/a.png"));
        let dom = crate::html::parse("<p>hi</p>");
        assert_eq!(icon_url(&dom, "https://ex.com/x/y").as_deref(), Some("https://ex.com/favicon.ico"));
        assert_eq!(icon_url(&dom, "file:///tmp/a.html"), None);
    }

    #[test]
    fn decodes_and_shrinks_a_png() {
        let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let icon = decode(&bytes).unwrap();
        assert_eq!(icon.len(), SIZE * SIZE);
        assert_eq!(icon[0], 0xFFFF0000);
    }
}
