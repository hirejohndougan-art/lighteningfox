//! Images: fetch in parallel, decode (PNG, JPEG, GIF, WebP, BMP, ICO),
//! and keep scaled copies ready for painting.

use crate::dom::Dom;
use crate::net;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Decoded images are capped at this many pixels per side to save memory.
const MAX_SIDE: u32 = 2048;
const MAX_PARALLEL: usize = 8;

pub struct Img {
    pub w: u32,
    pub h: u32,
    pub rgba: image::RgbaImage,
}

#[derive(Default)]
pub struct Images {
    /// Keyed by the `src` attribute exactly as written in the page.
    by_src: HashMap<String, Arc<Img>>,
    failed: HashSet<String>,
    /// Pre-scaled copies for painting, keyed by (src, width, height).
    scaled: RefCell<HashMap<(String, u32, u32), Arc<image::RgbaImage>>>,
}

impl Images {
    pub fn get(&self, src: &str) -> Option<&Arc<Img>> {
        self.by_src.get(src)
    }

    pub fn size(&self, src: &str) -> Option<(f32, f32)> {
        self.get(src).map(|i| (i.w as f32, i.h as f32))
    }

    /// Add an already-decoded picture (an iframe's rendering) under `key`.
    pub fn insert(&mut self, key: &str, rgba: image::RgbaImage) {
        let (w, h) = rgba.dimensions();
        self.by_src.insert(key.to_string(), Arc::new(Img { w, h, rgba }));
    }

    /// A copy sharing the decoded images (cheap), for laying the page out on demand.
    pub fn share(&self) -> Images {
        Images { by_src: self.by_src.clone(), failed: self.failed.clone(), ..Default::default() }
    }

    /// The image scaled to exactly w×h (cached).
    pub fn scaled(&self, src: &str, w: u32, h: u32) -> Option<Arc<image::RgbaImage>> {
        if w == 0 || h == 0 {
            return None;
        }
        let key = (src.to_string(), w, h);
        if let Some(s) = self.scaled.borrow().get(&key) {
            return Some(s.clone());
        }
        let img = self.get(src)?;
        let out = if img.rgba.width() == w && img.rgba.height() == h {
            img.rgba.clone()
        } else {
            image::imageops::resize(&img.rgba, w, h, image::imageops::FilterType::Triangle)
        };
        let out = Arc::new(out);
        let mut cache = self.scaled.borrow_mut();
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, out.clone());
        Some(out)
    }

    /// Load every <img src> in the DOM that isn't loaded yet. Fetches run in parallel.
    /// `extra` = more image URLs to load, such as CSS background images.
    pub fn load_missing(&mut self, dom: &Dom, base_url: &str, extra: &[String]) {
        let mut wanted: Vec<String> = Vec::new();
        let srcs = dom.find_all("img").into_iter().filter_map(|id| dom.attr(id, "src").map(|s| s.trim().to_string()));
        for src in srcs.chain(extra.iter().cloned()) {
            if !src.is_empty() && !self.by_src.contains_key(&src) && !self.failed.contains(&src) && !wanted.contains(&src) {
                wanted.push(src);
            }
        }
        for chunk in wanted.chunks(MAX_PARALLEL) {
            let results: Vec<(String, Option<Img>)> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|src| {
                        let url = net::resolve(base_url, src);
                        scope.spawn(move || fetch_and_decode(&url, base_url))
                    })
                    .collect();
                chunk.iter().cloned().zip(handles.into_iter().map(|h| h.join().ok().flatten())).collect()
            });
            for (src, img) in results {
                match img {
                    Some(img) => {
                        self.by_src.insert(src, Arc::new(img));
                    }
                    None => {
                        self.failed.insert(src);
                    }
                }
            }
        }
    }
}

fn fetch_and_decode(url: &str, page: &str) -> Option<Img> {
    let bytes = net::fetch_sub_bytes(url, page, crate::security::Dest::Image).map_err(|e| log!("[img] {url}: {e}")).ok()?.1;
    let img = image::load_from_memory(&bytes).map_err(|e| log!("[img] {url}: {e}")).ok()?;
    let (w, h) = (img.width(), img.height());
    let img = if w > MAX_SIDE || h > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    // Report the original size so layout matches other browsers.
    Some(Img { w, h, rgba: img.to_rgba8() })
}
