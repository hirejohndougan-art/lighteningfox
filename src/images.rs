//! Images: fetch in parallel, then decode only what's drawn, at the size it's drawn.
//!
//! - A downloaded image is kept in its compressed form; only its size is read up front.
//!   Painting asks for it at its display size, which decodes it, scales it, and caches just
//!   that copy (decode at display size).
//! - Decoded copies share a memory budget. The least recently drawn go first, and copies
//!   not drawn for a while (off-screen images, closed pages) are dropped; they're decoded
//!   again if they scroll back into view.
//! - Formats: PNG, JPEG, GIF, WebP, BMP, ICO, AVIF and SVG. Animated GIF and WebP play.
//! - `loading=lazy` images wait until they come near the window (see `lazy_wanted`).
//! - `srcset`, `sizes` and `<picture>` pick the source (see `srcset.rs`).

use crate::dom::{Dom, NodeId};
use crate::net;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Decoded images are capped at this many pixels per side to save memory.
const MAX_SIDE: u32 = 2048;
const MAX_PARALLEL: usize = 8;
/// Memory for decoded copies, per page.
const BUDGET_BYTES: usize = 96 << 20;
/// Copies not drawn for this long are dropped even under the budget.
const UNUSED_DROP: Duration = Duration::from_secs(30);
/// An animation's frames together may take this much memory; bigger ones are scaled down.
const ANIMATION_BYTES: usize = 48 << 20;
/// Lazy images start loading when they're this close to the window (as in Chrome).
pub const LAZY_MARGIN: f32 = 1250.0;

enum Data {
    /// A still image in its downloaded (compressed) form.
    Encoded(Arc<Vec<u8>>),
    /// Already decoded pixels (an iframe's rendering).
    Pixels(Arc<image::RgbaImage>),
    Svg(Arc<resvg::usvg::Tree>),
    /// Frames (decoded, maybe scaled down) and when each one ends, in ms from the start.
    Animated { frames: Vec<Arc<image::RgbaImage>>, ends: Vec<u32> },
    /// Drawn on demand at the size it's painted (a `<video>` with its controls).
    Drawn(Drawer),
}

/// Draws a picture at exactly w×h device pixels.
pub type Drawer = Arc<dyn Fn(u32, u32) -> Option<image::RgbaImage> + Send + Sync>;

pub struct Img {
    /// The image's own size (CSS px), whatever it's decoded at.
    pub w: u32,
    pub h: u32,
    data: Data,
}

impl Img {
    #[cfg(test)]
    fn animated(&self) -> bool {
        matches!(self.data, Data::Animated { .. })
    }
}

type Key = (String, u32, u32, u32);

struct Cached {
    img: Arc<image::RgbaImage>,
    used: Cell<Instant>,
}

#[derive(Default)]
struct Cache {
    map: HashMap<Key, Cached>,
    bytes: usize,
    last_sweep: Option<Instant>,
}

impl Cache {
    fn insert(&mut self, key: Key, img: Arc<image::RgbaImage>) {
        let size = img.as_raw().len();
        if let Some(old) = self.map.insert(key, Cached { img, used: Cell::new(Instant::now()) }) {
            self.bytes -= old.img.as_raw().len();
        }
        self.bytes += size;
        self.trim();
    }

    /// Drop copies unused for a while, then the least recently used until under budget.
    fn trim(&mut self) {
        let now = Instant::now();
        if self.last_sweep.is_none_or(|t| now - t > Duration::from_secs(2)) {
            self.last_sweep = Some(now);
            let before = self.map.len();
            let mut freed = 0;
            self.map.retain(|_, c| {
                let keep = now - c.used.get() < UNUSED_DROP;
                if !keep {
                    freed += c.img.as_raw().len();
                }
                keep
            });
            self.bytes -= freed;
            if self.map.len() < before {
                crate::release_free_memory();
            }
        }
        while self.bytes > BUDGET_BYTES && self.map.len() > 1 {
            let Some(oldest) = self.map.iter().min_by_key(|(_, c)| c.used.get()).map(|(k, _)| k.clone()) else { break };
            if let Some(c) = self.map.remove(&oldest) {
                self.bytes -= c.img.as_raw().len();
            }
        }
    }
}

#[derive(Default)]
pub struct Images {
    /// Keyed by the image's URL as chosen for the element (`srcset::current_src`).
    by_src: HashMap<String, Arc<Img>>,
    failed: HashSet<String>,
    cache: RefCell<Cache>,
    /// Lazy images known but not loaded yet, and ones being fetched in the background.
    lazy: HashSet<String>,
    in_flight: HashSet<String>,
    arrivals: Option<std::sync::mpsc::Receiver<(String, Option<Img>)>>,
    arrivals_tx: Option<std::sync::mpsc::Sender<(String, Option<Img>)>>,
    /// When the next animation frame of an image drawn last time is due.
    next_frame: Cell<Option<Instant>>,
}

/// Snapshots (--png, screenshot tests) show every animation's first frame.
static FROZEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn freeze_animations() {
    FROZEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Is this a snapshot (animations, video and sound held still)?
pub fn animations_frozen() -> bool {
    FROZEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn clock() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

impl Images {
    /// Changes whenever an image arrives or fails (layout sizes depend on them).
    pub fn layout_stamp(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        // Layout reads an image's size only (paint fetches the pixels by key), so a canvas,
        // video frame or live iframe redrawn at the same size leaves laid-out boxes reusable.
        let mut sum = 0u64;
        for (k, v) in &self.by_src {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            k.hash(&mut h);
            (v.w, v.h).hash(&mut h);
            sum = sum.wrapping_add(h.finish());
        }
        sum ^ (self.failed.len() as u64).rotate_left(40)
    }

    pub fn get(&self, src: &str) -> Option<&Arc<Img>> {
        self.by_src.get(src)
    }

    pub fn size(&self, src: &str) -> Option<(f32, f32)> {
        self.get(src).map(|i| (i.w as f32, i.h as f32))
    }

    /// Add an already-decoded picture (an iframe's rendering) under `key`.
    pub fn insert(&mut self, key: &str, rgba: image::RgbaImage) {
        // A new picture under the same key (a live frame redrawn): drop copies of the old one.
        {
            let mut c = self.cache.borrow_mut();
            let mut freed = 0;
            c.map.retain(|k, v| {
                let keep = k.0 != key;
                if !keep {
                    freed += v.img.as_raw().len();
                }
                keep
            });
            c.bytes -= freed;
        }
        let (w, h) = rgba.dimensions();
        self.by_src.insert(key.to_string(), Arc::new(Img { w, h, data: Data::Pixels(Arc::new(rgba)) }));
        // A redrawn picture (a canvas) must not show an old scaled copy.
        let mut cache = self.cache.borrow_mut();
        let stale: Vec<Key> = cache.map.keys().filter(|k| k.0 == key).cloned().collect();
        for k in stale {
            if let Some(c) = cache.map.remove(&k) {
                cache.bytes -= c.img.as_raw().len();
            }
        }
    }

    /// Add a picture that's drawn when painted, at the painted size (`w`×`h` is its own size,
    /// for layout; 0×0 if it has none).
    pub fn insert_drawn(&mut self, key: &str, w: u32, h: u32, draw: Drawer) {
        self.insert(key, image::RgbaImage::new(1, 1));
        self.by_src.insert(key.to_string(), Arc::new(Img { w, h, data: Data::Drawn(draw) }));
    }

    /// Add an SVG document (an inline `<svg>` written out by svg_inline.rs) under `key`,
    /// replacing what was there. False (and nothing kept) if it doesn't parse.
    pub fn insert_svg(&mut self, key: &str, svg: &[u8]) -> bool {
        {
            let mut cache = self.cache.borrow_mut();
            let stale: Vec<Key> = cache.map.keys().filter(|k| k.0 == key).cloned().collect();
            for k in stale {
                if let Some(c) = cache.map.remove(&k) {
                    cache.bytes -= c.img.as_raw().len();
                }
            }
        }
        let Some(tree) = parse_svg(svg) else {
            self.by_src.remove(key);
            return false;
        };
        let s = tree.size();
        let (w, h) = (s.width().round().max(1.0) as u32, s.height().round().max(1.0) as u32);
        self.by_src.insert(key.to_string(), Arc::new(Img { w, h, data: Data::Svg(Arc::new(tree)) }));
        true
    }

    /// A copy sharing the downloaded images (cheap), for laying the page out on demand.
    pub fn share(&self) -> Images {
        Images { by_src: self.by_src.clone(), failed: self.failed.clone(), lazy: self.lazy.clone(), ..Default::default() }
    }

    /// Bytes held by decoded copies right now.
    #[cfg(test)]
    pub fn decoded_bytes(&self) -> usize {
        self.cache.borrow().bytes
    }

    /// True once (then false until the next paint) when an animated image on screen needs
    /// its next frame drawn.
    pub fn frame_due(&self) -> bool {
        match self.next_frame.get() {
            Some(t) if Instant::now() >= t => {
                self.next_frame.set(None);
                true
            }
            _ => false,
        }
    }

    /// The image at exactly w×h, decoded and scaled on first use and cached within the
    /// memory budget. Animated images give the frame for the current time.
    pub fn scaled(&self, src: &str, w: u32, h: u32) -> Option<Arc<image::RgbaImage>> {
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            return None;
        }
        let img = self.get(src)?;
        let frame = match &img.data {
            Data::Animated { .. } if FROZEN.load(std::sync::atomic::Ordering::Relaxed) => 0,
            Data::Animated { ends, .. } => {
                let total = *ends.last().unwrap_or(&1) as u128;
                let t = (clock().elapsed().as_millis() % total.max(1)) as u32;
                let i = ends.iter().position(|&e| t < e).unwrap_or(0);
                // Ask for a redraw when this frame ends.
                let due = Instant::now() + Duration::from_millis((ends[i] - t).max(10) as u64);
                if self.next_frame.get().is_none_or(|d| due < d) {
                    self.next_frame.set(Some(due));
                }
                i as u32
            }
            _ => 0,
        };
        let key = (src.to_string(), w, h, frame);
        if let Some(c) = self.cache.borrow().map.get(&key) {
            c.used.set(Instant::now());
            return Some(c.img.clone());
        }
        let out = match &img.data {
            Data::Svg(tree) => render_svg(tree, w, h)?,
            Data::Drawn(draw) => draw(w, h)?,
            Data::Pixels(p) => resize(p, w, h),
            Data::Animated { frames, .. } => resize(&frames[frame as usize], w, h),
            Data::Encoded(bytes) => {
                // The full decode is cached too (briefly), so several sizes of one image
                // decode it once.
                let full_key = (src.to_string(), 0, 0, 0);
                let cached = self.cache.borrow().map.get(&full_key).map(|c| c.img.clone());
                let full = match cached {
                    Some(f) => f,
                    None => {
                        let f = Arc::new(decode_still(bytes)?);
                        self.cache.borrow_mut().insert(full_key, f.clone());
                        f
                    }
                };
                resize(&full, w, h)
            }
        };
        let out = Arc::new(out);
        self.cache.borrow_mut().insert(key, out.clone());
        Some(out)
    }

    /// Load the page's images that aren't loaded yet, in parallel: `<img>` sources (as chosen
    /// by srcset/`<picture>`) plus `extra` (CSS background images). Lazy images are only
    /// noted; `load_lazy` fetches them when they come near the window.
    pub fn load_missing(&mut self, dom: &Dom, base_url: &str, extra: &[String]) {
        let mut wanted: Vec<String> = Vec::new();
        for id in dom.find_all("img") {
            let Some(src) = crate::srcset::current_src(dom, id) else { continue };
            if self.by_src.contains_key(&src) || self.failed.contains(&src) {
                continue;
            }
            if is_lazy(dom, id) {
                self.lazy.insert(src);
            } else if !wanted.contains(&src) {
                wanted.push(src);
            }
        }
        for src in extra {
            if !src.is_empty() && !self.by_src.contains_key(src) && !self.failed.contains(src) && !wanted.contains(src) {
                wanted.push(src.clone());
            }
        }
        wanted.retain(|s| !self.in_flight.contains(s));
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
                self.store(src, img);
            }
        }
    }

    fn store(&mut self, src: String, img: Option<Img>) {
        self.lazy.remove(&src);
        self.in_flight.remove(&src);
        match img {
            Some(img) => {
                self.by_src.insert(src, Arc::new(img));
            }
            None => {
                self.failed.insert(src);
            }
        }
    }

    /// Lazy images being fetched right now.
    pub fn waiting_count(&self) -> usize {
        self.in_flight.len()
    }

    /// Start background fetches for lazy images in `srcs`. Returns how many started.
    pub fn load_lazy(&mut self, srcs: &[String], base_url: &str) -> usize {
        let mut started = 0;
        for src in srcs {
            if !self.lazy.contains(src) || self.in_flight.contains(src) {
                continue;
            }
            if self.arrivals_tx.is_none() {
                let (tx, rx) = std::sync::mpsc::channel();
                self.arrivals_tx = Some(tx);
                self.arrivals = Some(rx);
            }
            let tx = self.arrivals_tx.clone().unwrap();
            let (src_c, url, page) = (src.clone(), net::resolve(base_url, src), base_url.to_string());
            self.in_flight.insert(src.clone());
            let _ = std::thread::Builder::new().name("lazy-image".into()).spawn(move || {
                let _ = tx.send((src_c, fetch_and_decode(&url, &page)));
            });
            started += 1;
        }
        started
    }

    /// Lazy images that finished loading since the last call (the page should lay out again).
    pub fn poll_lazy(&mut self) -> bool {
        let got: Vec<(String, Option<Img>)> = match &self.arrivals {
            Some(rx) => rx.try_iter().collect(),
            None => return false,
        };
        let any = !got.is_empty();
        for (src, img) in got {
            self.store(src, img);
        }
        any
    }
}

/// `<img loading=lazy>` (scripts that turn scripting off don't matter here: we always run them).
pub fn is_lazy(dom: &Dom, id: NodeId) -> bool {
    dom.attr(id, "loading").is_some_and(|v| v.trim().eq_ignore_ascii_case("lazy"))
}

/// The lazy images to start loading: those whose box (or, with no box yet, whose nearest
/// laid-out ancestor) is within `LAZY_MARGIN` of the window. `boxes` are page-coordinate
/// rectangles by node id, `view` the visible page range.
pub fn lazy_wanted(dom: &Dom, images: &Images, boxes: &dyn Fn(NodeId) -> Option<(f32, f32)>, view: (f32, f32)) -> Vec<String> {
    if images.lazy.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for id in dom.find_all("img") {
        let Some(src) = crate::srcset::current_src(dom, id) else { continue };
        if !images.lazy.contains(&src) || out.contains(&src) {
            continue;
        }
        let mut n = Some(id);
        let mut span = None;
        while let Some(k) = n {
            if let Some(s) = boxes(k) {
                span = Some(s);
                break;
            }
            n = dom.nodes[k].parent;
        }
        // No box at all (hidden): it isn't needed.
        let Some((top, bottom)) = span else { continue };
        if bottom >= view.0 - LAZY_MARGIN && top <= view.1 + LAZY_MARGIN {
            out.push(src);
        }
    }
    out
}

fn resize(img: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
    if img.width() == w && img.height() == h {
        return img.clone();
    }
    // Shrinking a lot: a quick halving pass first keeps big photos cheap and smooth.
    let mut src = std::borrow::Cow::Borrowed(img);
    while src.width() / 2 >= w.max(1) * 2 && src.height() / 2 >= h.max(1) * 2 {
        src = std::borrow::Cow::Owned(image::imageops::resize(&*src, src.width() / 2, src.height() / 2, image::imageops::FilterType::Triangle));
    }
    image::imageops::resize(&*src, w, h, image::imageops::FilterType::Triangle)
}

fn decode_still(bytes: &[u8]) -> Option<image::RgbaImage> {
    if looks_avif(bytes) {
        return decode_avif(bytes);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 << 20);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| log!("[img] decode: {e}")).ok()?;
    let img = if img.width() > MAX_SIDE || img.height() > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    Some(img.to_rgba8())
}

// ---------------------------------------------------------------- formats

fn looks_svg(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return true; // svgz (only reached for image/svg+xml or .svgz)
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let t = head.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--") || t.starts_with("<!doctype svg")) && head.contains("<svg")
}

fn looks_avif(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[4..8] == b"ftyp" && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis" || bytes[8..bytes.len().min(64)].windows(4).any(|w| w == b"avif"))
}

fn svg_options() -> &'static resvg::usvg::Options<'static> {
    static OPT: OnceLock<resvg::usvg::Options<'static>> = OnceLock::new();
    OPT.get_or_init(|| {
        let mut o = resvg::usvg::Options::default();
        // Fonts for <text>: the system's, loaded once.
        o.fontdb_mut().load_system_fonts();
        o.font_family = "DejaVu Sans".into();
        // External files referenced from inside an SVG image aren't loaded (as in browsers).
        o.image_href_resolver.resolve_string = Box::new(|_, _| None);
        o
    })
}

fn parse_svg(bytes: &[u8]) -> Option<resvg::usvg::Tree> {
    resvg::usvg::Tree::from_data(bytes, svg_options()).map_err(|e| log!("[img] svg: {e}")).ok()
}

/// Draw an SVG at exactly w×h (its viewBox stretched as `preserveAspectRatio` says).
fn render_svg(tree: &resvg::usvg::Tree, w: u32, h: u32) -> Option<image::RgbaImage> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let size = tree.size();
    let t = resvg::tiny_skia::Transform::from_scale(w as f32 / size.width().max(1e-3), h as f32 / size.height().max(1e-3));
    resvg::render(tree, t, &mut pixmap.as_mut());
    // tiny-skia stores premultiplied alpha; images here are straight.
    let mut out = image::RgbaImage::new(w, h);
    for (dst, p) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = p.demultiply();
        *dst = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(out)
}

fn decode_avif(bytes: &[u8]) -> Option<image::RgbaImage> {
    use zenpixels::PixelDescriptor as D;
    let config = zenavif::DecoderConfig::new().threads(1);
    let img = zenavif::decode_with(bytes, &config, &zenavif::Unstoppable).map_err(|e| log!("[img] avif: {e}")).ok()?;
    let desc = img.descriptor();
    let mut out: Vec<u8> = Vec::new();
    let (w, h);
    // 8-bit and high-bit-depth (10/12-bit, delivered as 16-bit) pictures, with or without alpha.
    if desc.layout_compatible(D::RGBA8) {
        let b = img.try_as_imgref::<rgb::Rgba<u8>>()?;
        (w, h) = (b.width() as u32, b.height() as u32);
        out.extend(b.rows().flatten().flat_map(|p| [p.r, p.g, p.b, p.a]));
    } else if desc.layout_compatible(D::RGB8) {
        let b = img.try_as_imgref::<rgb::Rgb<u8>>()?;
        (w, h) = (b.width() as u32, b.height() as u32);
        out.extend(b.rows().flatten().flat_map(|p| [p.r, p.g, p.b, 255]));
    } else if desc.layout_compatible(D::RGBA16) {
        let b = img.try_as_imgref::<rgb::Rgba<u16>>()?;
        (w, h) = (b.width() as u32, b.height() as u32);
        out.extend(b.rows().flatten().flat_map(|p| [(p.r >> 8) as u8, (p.g >> 8) as u8, (p.b >> 8) as u8, (p.a >> 8) as u8]));
    } else if desc.layout_compatible(D::RGB16) {
        let b = img.try_as_imgref::<rgb::Rgb<u16>>()?;
        (w, h) = (b.width() as u32, b.height() as u32);
        out.extend(b.rows().flatten().flat_map(|p| [(p.r >> 8) as u8, (p.g >> 8) as u8, (p.b >> 8) as u8, 255]));
    } else {
        log!("[img] avif: unsupported pixel layout {desc:?}");
        return None;
    }
    let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, out)?);
    let img = if img.width() > MAX_SIDE || img.height() > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    Some(img.to_rgba8())
}

/// Frames of an animated GIF or WebP, or None for a still image.
fn decode_animation(bytes: &[u8]) -> Option<(Vec<Arc<image::RgbaImage>>, Vec<u32>)> {
    use image::AnimationDecoder;
    let cursor = std::io::Cursor::new(bytes);
    let frames: Vec<image::Frame> = if bytes.starts_with(b"GIF8") {
        image::codecs::gif::GifDecoder::new(cursor).ok()?.into_frames().take(1000).collect::<Result<_, _>>().ok()?
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let d = image::codecs::webp::WebPDecoder::new(cursor).ok()?;
        if !d.has_animation() {
            return None;
        }
        d.into_frames().take(1000).collect::<Result<_, _>>().ok()?
    } else {
        return None;
    };
    if frames.len() < 2 {
        return None;
    }
    let (fw, fh) = frames[0].buffer().dimensions();
    let total = fw as usize * fh as usize * 4 * frames.len();
    let scale = if total > ANIMATION_BYTES { (ANIMATION_BYTES as f64 / total as f64).sqrt() } else { 1.0 };
    let (sw, sh) = (((fw as f64 * scale) as u32).max(1), ((fh as f64 * scale) as u32).max(1));
    let mut ends = Vec::with_capacity(frames.len());
    let mut t = 0u32;
    let mut out = Vec::with_capacity(frames.len());
    for f in frames {
        let (n, d) = f.delay().numer_denom_ms();
        let ms = if d == 0 { 100 } else { n / d };
        // Browsers treat 0–10 ms frames as 100 ms (many GIFs rely on it).
        t += if ms <= 10 { 100 } else { ms };
        ends.push(t);
        let buf = f.into_buffer();
        out.push(Arc::new(if scale < 1.0 { resize(&buf, sw, sh) } else { buf }));
    }
    Some((out, ends))
}

/// Fetch one image and decode it at its own size, for scripts (`new Image()`): what canvas
/// and WebGL draw from. None if it can't be loaded or decoded.
pub fn fetch_pixels(url: &str, page: &str) -> Option<image::RgbaImage> {
    let img = fetch_and_decode(url, page)?;
    let (w, h) = (img.w.max(1), img.h.max(1));
    let mut tmp = Images::default();
    tmp.by_src.insert(url.to_string(), Arc::new(img));
    tmp.scaled(url, w, h).map(|a| (*a).clone())
}

fn fetch_and_decode(url: &str, page: &str) -> Option<Img> {
    let bytes = net::fetch_sub_bytes(url, page, crate::security::Dest::Image).map_err(|e| log!("[img] {url}: {e}")).ok()?.1;
    from_bytes(bytes, url)
}

/// Read an image's size (and, for SVG and animations, parse it); pixels wait until drawn.
fn from_bytes(bytes: Vec<u8>, url: &str) -> Option<Img> {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    if looks_svg(&bytes) || path.ends_with(".svg") || path.ends_with(".svgz") || url.starts_with("data:image/svg") {
        let tree = parse_svg(&bytes)?;
        let s = tree.size();
        return Some(Img { w: s.width().round().max(1.0) as u32, h: s.height().round().max(1.0) as u32, data: Data::Svg(Arc::new(tree)) });
    }
    if let Some((frames, ends)) = decode_animation(&bytes) {
        let first = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok());
        let (w, h) = first.unwrap_or(frames[0].dimensions());
        return Some(Img { w, h, data: Data::Animated { frames, ends } });
    }
    let dims = if looks_avif(&bytes) {
        decode_avif(&bytes).map(|i| i.dimensions())
    } else {
        image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok())
    };
    let Some((w, h)) = dims else {
        log!("[img] {url}: unknown or broken image");
        return None;
    };
    if w == 0 || h == 0 {
        return None;
    }
    Some(Img { w, h, data: Data::Encoded(Arc::new(bytes)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba(c)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn store(images: &mut Images, src: &str, bytes: Vec<u8>) {
        let img = from_bytes(bytes, src);
        images.store(src.into(), img);
    }

    #[test]
    fn decodes_at_display_size_only_when_drawn() {
        let mut im = Images::default();
        store(&mut im, "a.png", png(400, 300, [255, 0, 0, 255]));
        assert_eq!(im.size("a.png"), Some((400.0, 300.0)));
        assert_eq!(im.decoded_bytes(), 0, "nothing decoded before painting");
        let s = im.scaled("a.png", 40, 30).unwrap();
        assert_eq!(s.dimensions(), (40, 30));
        assert_eq!(s.get_pixel(5, 5).0, [255, 0, 0, 255]);
    }

    #[test]
    fn svg_renders_sharp_at_any_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 20 10"><rect width="10" height="10" fill="#00f"/><rect x="10" width="10" height="10" fill="#0f0"/></svg>"##;
        let mut im = Images::default();
        store(&mut im, "x.svg", svg.to_vec());
        assert_eq!(im.size("x.svg"), Some((20.0, 10.0)));
        let s = im.scaled("x.svg", 200, 100).unwrap();
        assert_eq!(s.get_pixel(50, 50).0, [0, 0, 255, 255]);
        assert_eq!(s.get_pixel(150, 50).0, [0, 255, 0, 255]);
        // Sharp edge: the pixels either side of the middle are pure.
        assert_eq!(s.get_pixel(98, 50).0, [0, 0, 255, 255]);
        assert_eq!(s.get_pixel(101, 50).0, [0, 255, 0, 255]);
    }

    #[test]
    fn animated_gif_has_frames_and_timing() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let mut bytes = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut bytes);
            enc.set_repeat(Repeat::Infinite).unwrap();
            for c in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                let f = image::Frame::from_parts(image::RgbaImage::from_pixel(8, 8, image::Rgba(c)), 0, 0, image::Delay::from_numer_denom_ms(50, 1));
                enc.encode_frame(f).unwrap();
            }
        }
        let img = from_bytes(bytes, "a.gif").unwrap();
        assert!(img.animated());
        let Data::Animated { frames, ends } = &img.data else { panic!() };
        assert_eq!(frames.len(), 2);
        assert_eq!(ends, &vec![50, 100]);
        let mut im = Images::default();
        im.by_src.insert("a.gif".into(), Arc::new(img));
        assert!(im.scaled("a.gif", 8, 8).is_some());
        assert!(im.next_frame.get().is_some(), "an animation asks for its next frame");
    }

    #[test]
    fn budget_evicts_least_recently_drawn() {
        let mut im = Images::default();
        for i in 0..6 {
            store(&mut im, &format!("{i}.png"), png(10, 10, [i * 40, 0, 0, 255]));
        }
        // Six 2000×2000 copies = 96 MB of RGBA each 16 MB: the budget keeps it near 96 MB.
        for i in 0..6 {
            im.scaled(&format!("{i}.png"), 2000, 2000).unwrap();
        }
        im.scaled("0.png", 2000, 2000).unwrap();
        im.scaled("6.png", 1, 1);
        store(&mut im, "7.png", png(10, 10, [0, 0, 0, 255]));
        im.scaled("7.png", 2000, 2000).unwrap();
        assert!(im.decoded_bytes() <= BUDGET_BYTES, "{} bytes", im.decoded_bytes());
        // 0.png was drawn recently, so it survived; 1.png was the oldest and went.
        let c = im.cache.borrow();
        assert!(c.map.contains_key(&("0.png".to_string(), 2000, 2000, 0)));
        assert!(!c.map.contains_key(&("1.png".to_string(), 2000, 2000, 0)));
    }

    #[test]
    fn broken_and_unknown_data_fail_cleanly() {
        assert!(from_bytes(b"not an image".to_vec(), "x.png").is_none());
        assert!(from_bytes(b"<svg".to_vec(), "x.svg").is_none());
        let mut im = Images::default();
        store(&mut im, "bad.jpg", b"\xff\xd8\xff garbage".to_vec());
        assert!(im.scaled("bad.jpg", 10, 10).is_none());
    }
}
