//! `<canvas>` 2D drawing. Each canvas's pixels live here, in the page's script engine, and
//! are drawn with tiny-skia (anti-aliased paths, strokes, gradients, patterns, blend modes).
//!
//! The JavaScript side (`canvas.js`) keeps the drawing state (styles, transform, the current
//! path in canvas pixels) and calls the `__cv*` natives below to rasterize. Before each
//! layout, `publish` hands canvases that changed to the page's images as `canvas:<node id>`,
//! which layout and painting treat like any other picture.

use crate::dom::NodeId;
use crate::images::Images;
use boa_engine::object::builtins::{JsArray, JsFloat64Array, JsUint8ClampedArray};
use boa_engine::{js_string, Context, JsResult, JsString, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, PathBuilder, Pattern, Pixmap,
    PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

/// The most pixels one canvas may have (browsers cap similarly; this is 64 MB of RGBA).
const MAX_PIXELS: u64 = 16_384 * 1024;
const MAX_SIDE: u32 = 16_384;

struct Cv {
    pix: Pixmap,
    /// The clip in force (None = whole canvas) and the ones saved by save().
    clip: Option<Mask>,
    saved: Vec<Option<Mask>>,
    /// Bumped by every change; `published` is the generation the page's images hold.
    version: u64,
    published: u64,
}

#[derive(Default)]
pub struct Store {
    canvases: HashMap<NodeId, Cv>,
    /// Decoded page images as premultiplied pixmaps, for drawImage and patterns.
    sources: HashMap<String, Pixmap>,
}

thread_local! {
    /// The page's decoded images (shared copies), so drawImage can find an <img>'s pixels.
    static PAGE_IMAGES: RefCell<Option<Images>> = const { RefCell::new(None) };
}

/// Make the page's decoded images available to canvas drawImage.
pub fn set_images(images: Images) {
    PAGE_IMAGES.with(|p| *p.borrow_mut() = Some(images));
}

/// Set up canvas support in a page's script engine.
pub fn install(ctx: &mut Context) {
    ctx.insert_data(RefCell::new(Store::default()));
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
        ("__cvInit", 3, n_init),
        ("__cvFill", 6, n_fill),
        ("__cvStroke", 6, n_stroke),
        ("__cvClip", 3, n_clip),
        ("__cvSave", 1, n_save),
        ("__cvRestore", 1, n_restore),
        ("__cvText", 8, n_text),
        ("__cvMeasure", 3, n_measure),
        ("__cvImage", 6, n_image),
        ("__cvSourceSize", 2, n_source_size),
        ("__cvGetData", 5, n_get_data),
        ("__cvPutData", 8, n_put_data),
        ("__cvDataURL", 3, n_data_url),
        ("__cvInPath", 4, n_in_path),
        ("__cvColor", 1, n_color),
    ];
    for (name, len, f) in natives {
        ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f)).expect("register canvas native");
    }
    if let Err(e) = ctx.eval(Source::from_bytes(include_str!("dommatrix.js").as_bytes())) {
        log!("[ironswift] DOMMatrix setup failed: {e}");
    }
    if let Err(e) = ctx.eval(Source::from_bytes(include_str!("canvas.js").as_bytes())) {
        log!("[ironswift] canvas setup failed: {e}");
    }
}

/// Copy canvases drawn on since the last call into the page's images.
pub fn publish(ctx: &Context, images: &mut Images) {
    let Some(store) = ctx.get_data::<RefCell<Store>>() else { return };
    let mut store = store.borrow_mut();
    for (id, cv) in store.canvases.iter_mut() {
        if cv.version == cv.published {
            continue;
        }
        cv.published = cv.version;
        let (w, h) = (cv.pix.width(), cv.pix.height());
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for p in cv.pix.pixels() {
            let c = p.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        if let Some(img) = image::RgbaImage::from_raw(w, h, rgba) {
            images.insert(&format!("canvas:{id}"), img);
        }
    }
}

fn store(ctx: &Context) -> std::cell::RefMut<'_, Store> {
    ctx.get_data::<RefCell<Store>>().expect("canvas store").borrow_mut()
}

// ---------------------------------------------------------------- argument helpers

fn num(a: &[JsValue], i: usize, ctx: &mut Context) -> f64 {
    a.get(i).map_or(Ok(f64::NAN), |v| v.to_number(ctx)).unwrap_or(f64::NAN)
}

fn id_of(a: &[JsValue], ctx: &mut Context) -> NodeId {
    num(a, 0, ctx) as NodeId
}

fn floats(a: &[JsValue], i: usize, ctx: &mut Context) -> Vec<f64> {
    let Some(obj) = a.get(i).and_then(|v| v.as_object()) else { return Vec::new() };
    if let Ok(arr) = JsFloat64Array::from_object(obj.clone()) {
        if let Some(bytes) = typed_bytes(&arr, ctx) {
            return bytes.chunks_exact(8).map(|c| f64::from_ne_bytes(c.try_into().unwrap())).collect();
        }
    }
    if let Ok(arr) = JsArray::from_object(obj.clone()) {
        let n = arr.length(ctx).unwrap_or(0);
        return (0..n).map(|i| arr.get(i, ctx).and_then(|v| v.to_number(ctx)).unwrap_or(f64::NAN)).collect();
    }
    Vec::new()
}

/// The bytes a typed array views.
fn typed_bytes(arr: &boa_engine::object::builtins::JsTypedArray, ctx: &mut Context) -> Option<Vec<u8>> {
    let buf = arr.buffer(ctx).ok()?;
    let buf = boa_engine::object::builtins::JsArrayBuffer::from_object(buf.as_object()?.clone()).ok()?;
    let (off, len) = (arr.byte_offset(ctx).ok()?, arr.byte_length(ctx).ok()?);
    let data = buf.data()?;
    data.get(off..off + len).map(|b| b.to_vec())
}

fn string(a: &[JsValue], i: usize, ctx: &mut Context) -> String {
    a.get(i).and_then(|v| v.to_string(ctx).ok()).map(|s| s.to_std_string_escaped()).unwrap_or_default()
}

/// A path from `[op, …]` commands: 0 moveTo x y · 1 lineTo x y · 2 quad cx cy x y ·
/// 3 cubic c1x c1y c2x c2y x y · 4 close.
fn build_path(cmds: &[f64]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let f = |v: f64| v as f32;
    let mut i = 0;
    while i < cmds.len() {
        let need = match cmds[i] as u8 {
            0 | 1 => 2,
            2 => 4,
            3 => 6,
            _ => 0,
        };
        if i + need >= cmds.len() + usize::from(need == 0) {
            break;
        }
        let p = &cmds[i + 1..i + 1 + need];
        if p.iter().any(|v| !v.is_finite()) {
            i += 1 + need;
            continue;
        }
        match cmds[i] as u8 {
            0 => pb.move_to(f(p[0]), f(p[1])),
            1 => pb.line_to(f(p[0]), f(p[1])),
            2 => pb.quad_to(f(p[0]), f(p[1]), f(p[2]), f(p[3])),
            3 => pb.cubic_to(f(p[0]), f(p[1]), f(p[2]), f(p[3]), f(p[4]), f(p[5])),
            _ => pb.close(),
        }
        i += 1 + need;
    }
    pb.finish()
}

/// Drawing state from canvas.js: [globalAlpha, composite op, a b c d e f (transform),
/// image smoothing, shadow blur, shadow dx, shadow dy, shadow r g b a].
struct State {
    alpha: f32,
    blend: BlendMode,
    ctm: Transform,
    smooth: bool,
    shadow: Option<(f32, f32, f32, Color)>,
}

fn state(v: &[f64]) -> State {
    let g = |i: usize, d: f64| v.get(i).copied().filter(|x| x.is_finite()).unwrap_or(d) as f32;
    let blend = match v.get(1).copied().unwrap_or(0.0) as u8 {
        1 => BlendMode::SourceIn,
        2 => BlendMode::SourceOut,
        3 => BlendMode::SourceAtop,
        4 => BlendMode::DestinationOver,
        5 => BlendMode::DestinationIn,
        6 => BlendMode::DestinationOut,
        7 => BlendMode::DestinationAtop,
        8 => BlendMode::Plus,
        9 => BlendMode::Source,
        10 => BlendMode::Xor,
        11 => BlendMode::Multiply,
        12 => BlendMode::Screen,
        13 => BlendMode::Overlay,
        14 => BlendMode::Darken,
        15 => BlendMode::Lighten,
        16 => BlendMode::ColorDodge,
        17 => BlendMode::ColorBurn,
        18 => BlendMode::HardLight,
        19 => BlendMode::SoftLight,
        20 => BlendMode::Difference,
        21 => BlendMode::Exclusion,
        22 => BlendMode::Hue,
        23 => BlendMode::Saturation,
        24 => BlendMode::Color,
        25 => BlendMode::Luminosity,
        26 => BlendMode::Clear,
        _ => BlendMode::SourceOver,
    };
    let ctm = Transform::from_row(g(2, 1.0), g(3, 0.0), g(4, 0.0), g(5, 1.0), g(6, 0.0), g(7, 0.0));
    let shadow_color = Color::from_rgba(g(12, 0.0).clamp(0.0, 1.0), g(13, 0.0).clamp(0.0, 1.0), g(14, 0.0).clamp(0.0, 1.0), g(15, 0.0).clamp(0.0, 1.0));
    let shadow = match shadow_color {
        Some(c) if c.alpha() > 0.0 && (g(9, 0.0) > 0.0 || g(10, 0.0) != 0.0 || g(11, 0.0) != 0.0) => Some((g(9, 0.0).max(0.0), g(10, 0.0), g(11, 0.0), c)),
        _ => None,
    };
    State { alpha: g(0, 1.0).clamp(0.0, 1.0), blend, ctm, smooth: g(8, 1.0) != 0.0, shadow }
}

fn rgba(v: &[f64], i: usize) -> Color {
    let g = |k: usize| v.get(i + k).copied().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    Color::from_rgba(g(0), g(1), g(2), g(3)).unwrap_or(Color::BLACK)
}

/// A fill or stroke style: [0 r g b a] color · [1 x0 y0 x1 y1 n stops…] linear ·
/// [2 x0 y0 r0 x1 y1 r1 n stops…] radial · [3 repeatX repeatY] pattern (its image passed
/// separately) · [4 angle x y n stops…] conic. Stops are (offset r g b a).
fn shader<'a>(spec: &[f64], st: &State, pattern: Option<&'a Pixmap>) -> Option<Shader<'a>> {
    let stops = |at: usize, shrink: Option<f32>| -> Vec<GradientStop> {
        let n = spec.get(at).copied().unwrap_or(0.0) as usize;
        let mut out = Vec::new();
        for k in 0..n {
            let b = at + 1 + k * 5;
            if b + 4 >= spec.len() + 1 {
                break;
            }
            let mut off = spec[b].clamp(0.0, 1.0) as f32;
            if let Some(r0) = shrink {
                off = r0 + off * (1.0 - r0);
            }
            out.push(GradientStop::new(off, rgba(spec, b + 1)));
        }
        out
    };
    let p = |i: usize| spec.get(i).copied().unwrap_or(0.0) as f32;
    match spec.first().copied().unwrap_or(0.0) as u8 {
        0 => Some(Shader::SolidColor(rgba(spec, 1))),
        1 => {
            let s = stops(5, None);
            match s.len() {
                0 => None,
                1 => Some(Shader::SolidColor(first_color(spec, 6))),
                _ => LinearGradient::new(Point::from_xy(p(1), p(2)), Point::from_xy(p(3), p(4)), s, SpreadMode::Pad, st.ctm).or(Some(Shader::SolidColor(first_color(spec, 6)))),
            }
        }
        2 => {
            // tiny-skia's radial gradient grows from a point (radius 0) at the start to
            // `r1` at the end; an inner radius r0 is folded into the stop offsets.
            let (r0, r1) = (p(3).max(0.0), p(6).max(0.0));
            let shrink = if r1 > 0.0 { Some((r0 / r1).clamp(0.0, 1.0)) } else { None };
            let s = stops(7, shrink);
            match s.len() {
                0 => None,
                1 => Some(Shader::SolidColor(first_color(spec, 8))),
                _ => RadialGradient::new(Point::from_xy(p(1), p(2)), Point::from_xy(p(4), p(5)), r1, s, SpreadMode::Pad, st.ctm).or(Some(Shader::SolidColor(first_color(spec, 8)))),
            }
        }
        3 => {
            let px = pattern?;
            let mode = if p(1) != 0.0 || p(2) != 0.0 { SpreadMode::Repeat } else { SpreadMode::Pad };
            let q = if st.smooth { FilterQuality::Bilinear } else { FilterQuality::Nearest };
            Some(Pattern::new(px.as_ref(), mode, q, 1.0, st.ctm))
        }
        // Conic gradients: drawn with their first color for now.
        4 => Some(Shader::SolidColor(first_color(spec, 5))),
        _ => None,
    }
}

fn first_color(spec: &[f64], at: usize) -> Color {
    rgba(spec, at)
}

fn paint_of<'a>(spec: &[f64], st: &State, pattern: Option<&'a Pixmap>) -> Option<Paint<'a>> {
    let mut shader = shader(spec, st, pattern)?;
    shader.apply_opacity(st.alpha);
    Some(Paint { shader, blend_mode: st.blend, anti_alias: true, force_hq_pipeline: false })
}

/// The image URL an `<img>` shows (its src, or what srcset/<picture> chose).
fn img_url(key: &str) -> Option<String> {
    let id: NodeId = key.parse().ok()?;
    crate::js::with_dom(|d| (id < d.nodes.len()).then(|| crate::srcset::current_src(d, id)).flatten())
}

/// The pixels of an image source: 0 = an <img> (by node id), 1 = a canvas (by node id).
fn source_pixmap(store: &mut Store, kind: u8, key: &str) -> Option<Pixmap> {
    let url;
    let key = if kind == 0 {
        url = img_url(key)?;
        url.as_str()
    } else {
        key
    };
    if kind == 1 {
        let id: NodeId = key.parse().ok()?;
        return store.canvases.get(&id).map(|c| c.pix.clone());
    }
    if let Some(p) = store.sources.get(key) {
        return Some(p.clone());
    }
    let pix = PAGE_IMAGES.with(|imgs| {
        let imgs = imgs.borrow();
        let imgs = imgs.as_ref()?;
        let img = imgs.get(key)?;
        let rgba = imgs.scaled(key, img.w.max(1), img.h.max(1))?;
        let (w, h) = rgba.dimensions();
        let mut pix = Pixmap::new(w, h)?;
        for (dst, s) in pix.pixels_mut().iter_mut().zip(rgba.pixels()) {
            let [r, g, b, a] = s.0;
            *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
        }
        Some(pix)
    })?;
    if store.sources.len() > 64 {
        store.sources.clear();
    }
    store.sources.insert(key.to_string(), pix.clone());
    Some(pix)
}

/// Draw with `f`, with a shadow underneath when the state has one. `f` draws into the
/// pixmap it's given, through the mask and blend mode it's given.
fn draw(cv: &mut Cv, st: &State, f: impl Fn(&mut Pixmap, Option<&Mask>, BlendMode)) {
    cv.version += 1;
    let Some((blur, dx, dy, color)) = st.shadow else {
        f(&mut cv.pix, cv.clip.as_ref(), st.blend);
        return;
    };
    let (w, h) = (cv.pix.width(), cv.pix.height());
    let Some(mut layer) = Pixmap::new(w, h) else { return };
    f(&mut layer, None, BlendMode::SourceOver);
    // The shadow: the shape's coverage in the shadow color, blurred, moved.
    let mut alpha: Vec<f32> = layer.pixels().iter().map(|p| p.alpha() as f32 / 255.0).collect();
    blur_alpha(&mut alpha, w as usize, h as usize, blur / 2.0);
    if let Some(mut shadow) = Pixmap::new(w, h) {
        for (d, a) in shadow.pixels_mut().iter_mut().zip(&alpha) {
            let a = (a * color.alpha()).clamp(0.0, 1.0);
            if a > 0.0 {
                if let Some(c) = Color::from_rgba(color.red(), color.green(), color.blue(), a) {
                    *d = c.premultiply().to_color_u8();
                }
            }
        }
        let pp = PixmapPaint { opacity: 1.0, blend_mode: st.blend, quality: FilterQuality::Nearest };
        cv.pix.draw_pixmap(dx.round() as i32, dy.round() as i32, shadow.as_ref(), &pp, Transform::identity(), cv.clip.as_ref());
    }
    let pp = PixmapPaint { opacity: 1.0, blend_mode: st.blend, quality: FilterQuality::Nearest };
    cv.pix.draw_pixmap(0, 0, layer.as_ref(), &pp, Transform::identity(), cv.clip.as_ref());
}

/// Three box blurs approximate a Gaussian of standard deviation `sigma`.
fn blur_alpha(a: &mut [f32], w: usize, h: usize, sigma: f32) {
    if sigma < 0.5 || w == 0 || h == 0 {
        return;
    }
    let r = ((sigma * 3.0_f32.sqrt()).round() as usize).clamp(1, 200);
    let mut tmp = vec![0.0f32; a.len()];
    for _ in 0..3 {
        for y in 0..h {
            box_line(&a[y * w..(y + 1) * w], &mut tmp[y * w..(y + 1) * w], r);
        }
        let mut col = vec![0.0f32; h];
        let mut out = vec![0.0f32; h];
        for x in 0..w {
            for y in 0..h {
                col[y] = tmp[y * w + x];
            }
            box_line(&col, &mut out, r);
            for y in 0..h {
                a[y * w + x] = out[y];
            }
        }
    }
}

fn box_line(src: &[f32], dst: &mut [f32], r: usize) {
    let n = src.len();
    let mut sum = 0.0f32;
    let width = (2 * r + 1) as f32;
    for i in 0..=r.min(n - 1) {
        sum += src[i];
    }
    for i in 0..n {
        dst[i] = sum / width;
        if i + r + 1 < n {
            sum += src[i + r + 1];
        }
        if i >= r {
            sum -= src[i - r];
        }
    }
}

// ---------------------------------------------------------------- natives

/// __cvInit(id, w, h): give a canvas a fresh, transparent bitmap (also when its size changes).
fn n_init(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let mut w = num(a, 1, ctx).max(0.0).min(MAX_SIDE as f64) as u32;
    let mut h = num(a, 2, ctx).max(0.0).min(MAX_SIDE as f64) as u32;
    if w as u64 * h as u64 > MAX_PIXELS {
        w = 0;
        h = 0;
    }
    let mut s = store(ctx);
    let version = s.canvases.get(&id).map_or(1, |c| c.version + 1);
    // A 0×0 canvas still exists; tiny-skia needs at least one pixel.
    let pix = Pixmap::new(w.max(1), h.max(1)).expect("pixmap");
    s.canvases.insert(id, Cv { pix, clip: None, saved: Vec::new(), version, published: 0 });
    drop(s);
    crate::js::with_dom_mut(|_| ());
    Ok(JsValue::undefined())
}

/// __cvFill(id, path, style, state, evenOdd, patternKind, patternKey): fill a path (or the
/// whole canvas with clearRect's composite op).
fn n_fill(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let path = floats(a, 1, ctx);
    let spec = floats(a, 2, ctx);
    let st = state(&floats(a, 3, ctx));
    let rule = if num(a, 4, ctx) == 1.0 { FillRule::EvenOdd } else { FillRule::Winding };
    let pkind = num(a, 5, ctx);
    let pkey = string(a, 6, ctx);
    let Some(path) = build_path(&path) else { return Ok(JsValue::undefined()) };
    let mut s = store(ctx);
    let pattern = if spec.first() == Some(&3.0) { source_pixmap(&mut s, pkind as u8, &pkey) } else { None };
    let Some(paint) = paint_of(&spec, &st, pattern.as_ref()) else { return Ok(JsValue::undefined()) };
    if let Some(cv) = s.canvases.get_mut(&id) {
        draw(cv, &st, |pix, mask, blend| {
            let mut p = paint.clone();
            p.blend_mode = blend;
            pix.fill_path(&path, &p, rule, Transform::identity(), mask);
        });
    }
    drop(s);
    crate::js::with_dom_mut(|_| ());
    Ok(JsValue::undefined())
}

/// __cvStroke(id, path, style, state, strokeSpec, patternKind, patternKey). The path is in
/// canvas pixels; the stroke is drawn in user space (so a scaled transform widens the line).
fn n_stroke(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let path = floats(a, 1, ctx);
    let spec = floats(a, 2, ctx);
    let st = state(&floats(a, 3, ctx));
    let sk = floats(a, 4, ctx);
    let pkind = num(a, 5, ctx);
    let pkey = string(a, 6, ctx);
    let Some(path) = build_path(&path) else { return Ok(JsValue::undefined()) };
    let Some(inv) = st.ctm.invert() else { return Ok(JsValue::undefined()) };
    let Some(user_path) = path.transform(inv) else { return Ok(JsValue::undefined()) };
    let g = |i: usize, d: f64| sk.get(i).copied().filter(|v| v.is_finite()).unwrap_or(d) as f32;
    let mut stroke = Stroke {
        width: g(0, 1.0).max(0.0),
        line_cap: match g(1, 0.0) as u8 {
            1 => LineCap::Round,
            2 => LineCap::Square,
            _ => LineCap::Butt,
        },
        line_join: match g(2, 0.0) as u8 {
            1 => LineJoin::Round,
            2 => LineJoin::Bevel,
            _ => LineJoin::Miter,
        },
        miter_limit: g(3, 10.0).max(1.0),
        dash: None,
    };
    if stroke.width <= 0.0 {
        return Ok(JsValue::undefined());
    }
    let mut dashes: Vec<f32> = sk.iter().skip(5).map(|&v| v as f32).filter(|v| v.is_finite() && *v >= 0.0).collect();
    if !dashes.is_empty() {
        if dashes.len() % 2 == 1 {
            dashes.extend(dashes.clone());
        }
        stroke.dash = StrokeDash::new(dashes, g(4, 0.0));
    }
    let user = State { ctm: Transform::identity(), ..state(&floats(a, 3, ctx)) };
    let mut s = store(ctx);
    // Patterns and gradients are given in user space: drawn through the same transform.
    let pattern = if spec.first() == Some(&3.0) { source_pixmap(&mut s, pkind as u8, &pkey) } else { None };
    let Some(paint) = paint_of(&spec, &user, pattern.as_ref()) else { return Ok(JsValue::undefined()) };
    if let Some(cv) = s.canvases.get_mut(&id) {
        draw(cv, &st, |pix, mask, blend| {
            let mut p = paint.clone();
            p.blend_mode = blend;
            pix.stroke_path(&user_path, &p, &stroke, st.ctm, mask);
        });
    }
    drop(s);
    crate::js::with_dom_mut(|_| ());
    Ok(JsValue::undefined())
}

/// __cvClip(id, path, evenOdd): narrow the clip to a path.
fn n_clip(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let path = floats(a, 1, ctx);
    let rule = if num(a, 2, ctx) == 1.0 { FillRule::EvenOdd } else { FillRule::Winding };
    let mut s = store(ctx);
    if let Some(cv) = s.canvases.get_mut(&id) {
        let (w, h) = (cv.pix.width(), cv.pix.height());
        match build_path(&path) {
            Some(p) => match cv.clip.as_mut() {
                Some(m) => m.intersect_path(&p, rule, true, Transform::identity()),
                None => {
                    let mut m = Mask::new(w, h).expect("mask");
                    m.fill_path(&p, rule, true, Transform::identity());
                    cv.clip = Some(m);
                }
            },
            // Clipping to an empty path hides everything.
            None => cv.clip = Mask::new(w, h),
        }
    }
    Ok(JsValue::undefined())
}

fn n_save(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    if let Some(cv) = store(ctx).canvases.get_mut(&id) {
        let c = cv.clip.clone();
        cv.saved.push(c);
    }
    Ok(JsValue::undefined())
}

fn n_restore(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    if let Some(cv) = store(ctx).canvases.get_mut(&id) {
        if let Some(c) = cv.saved.pop() {
            cv.clip = c;
        }
    }
    Ok(JsValue::undefined())
}

/// Font from canvas.js: [size px, bold, italic] and a family list.
fn font_of(f: &[f64], family: &str) -> (u8, f32) {
    let size = f.first().copied().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(10.0) as f32;
    let bold = f.get(1).copied().unwrap_or(0.0) != 0.0;
    let italic = f.get(2).copied().unwrap_or(0.0) != 0.0;
    let mono = family.to_ascii_lowercase().contains("monospace");
    let fam = if family.trim().is_empty() { None } else { Some(family) };
    (crate::font::resolve(fam, bold, italic, mono), size.min(1000.0))
}

/// __cvMeasure(text, font, family) -> [width, ascent, descent]
fn n_measure(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = string(a, 0, ctx);
    let f = floats(a, 1, ctx);
    let family = string(a, 2, ctx);
    let (font, size) = font_of(&f, &family);
    let (w, asc) = match crate::load_fonts() {
        Some(fonts) => (fonts.shape(font, &text, size).width, fonts.ascent(font, size)),
        None => (text.chars().count() as f32 * size * 0.55, size * 0.8),
    };
    let arr = JsArray::from_iter([JsValue::from(w as f64), JsValue::from(asc as f64), JsValue::from((size * 0.24) as f64)], ctx);
    Ok(arr.into())
}

/// __cvText(id, text, [x, y, maxWidth, align, baseline, stroke, lineWidth], font, family,
/// style, state, unused): fillText / strokeText.
fn n_text(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let text = string(a, 1, ctx);
    let pos = floats(a, 2, ctx);
    let f = floats(a, 3, ctx);
    let family = string(a, 4, ctx);
    let spec = floats(a, 5, ctx);
    let st = state(&floats(a, 6, ctx));
    let Some(fonts) = crate::load_fonts() else { return Ok(JsValue::undefined()) };
    if text.is_empty() {
        return Ok(JsValue::undefined());
    }
    let g = |i: usize, d: f64| pos.get(i).copied().filter(|v| v.is_finite()).unwrap_or(d) as f32;
    let (font, size) = font_of(&f, &family);
    // Draw the glyphs at the size they'll be seen (a scaled transform), not stretched.
    let scale = (st.ctm.sx * st.ctm.sy - st.ctm.kx * st.ctm.ky).abs().sqrt().clamp(0.05, 16.0);
    let dsize = size * scale;
    let shaped = fonts.shape(font, &text, dsize);
    let ascent = fonts.ascent(font, dsize);
    let descent = dsize * 0.24;
    let width = shaped.width;
    if width <= 0.0 {
        return Ok(JsValue::undefined());
    }
    let pad = 2.0f32;
    let (tw, th) = ((width + 2.0 * pad).ceil() as u32 + 2, (ascent + descent + 2.0 * pad).ceil() as u32 + 2);
    let Some(mut mask) = Mask::new(tw, th) else { return Ok(JsValue::undefined()) };
    {
        let data = mask.data_mut();
        let baseline = pad + ascent;
        for gl in &shaped.glyphs {
            fonts.with_glyph_id(gl.face, gl.id, dsize, 0, |m, bmp| {
                let gx = (pad + gl.x).round() as i32 + m.xmin;
                let gy = (baseline - gl.y).round() as i32 - m.height as i32 - m.ymin;
                for row in 0..m.height {
                    for col in 0..m.width {
                        let (x, y) = (gx + col as i32, gy + row as i32);
                        if x >= 0 && y >= 0 && (x as u32) < tw && (y as u32) < th {
                            let i = y as usize * tw as usize + x as usize;
                            // Color glyphs (emoji) are RGBA; their coverage is the alpha.
                            let cov = if m.color { bmp.get((row * m.width + col) * 4 + 3).copied().unwrap_or(0) } else { bmp[row * m.width + col] };
                            data[i] = data[i].max(cov);
                        }
                    }
                }
            });
        }
    }
    // Where the text box goes in user space.
    let w_user = width / scale;
    let squeeze = match pos.get(2).copied().filter(|v| v.is_finite() && *v > 0.0) {
        Some(mw) if (mw as f32) < w_user => mw as f32 / w_user,
        _ => 1.0,
    };
    let left = match g(3, 0.0) as u8 {
        1 => -w_user * squeeze / 2.0,
        2 => -w_user * squeeze,
        _ => 0.0,
    };
    let (asc_u, desc_u) = (ascent / scale, descent / scale);
    let top_of_baseline = match g(4, 0.0) as u8 {
        1 => asc_u,                       // top
        2 => asc_u * 0.8,                 // hanging
        3 => (asc_u - desc_u) / 2.0,      // middle
        4 | 5 => -desc_u,                 // bottom, ideographic
        _ => 0.0,                         // alphabetic
    };
    // Text pixmap → user space: scale back down, then place.
    let place = Transform::from_row(squeeze / scale, 0.0, 0.0, 1.0 / scale, g(0, 0.0) + left - pad / scale * squeeze, g(1, 0.0) + top_of_baseline - (pad + ascent) / scale);
    let to_canvas = st.ctm.pre_concat(place);
    let user = State { ctm: Transform::identity(), ..state(&floats(a, 6, ctx)) };
    let Some(paint) = paint_of(&spec, &user, None) else { return Ok(JsValue::undefined()) };
    let Some(mut layer) = Pixmap::new(tw, th) else { return Ok(JsValue::undefined()) };
    let mut p = paint.clone();
    p.blend_mode = BlendMode::SourceOver;
    // Gradients are given in user space: map them into the text pixmap's frame.
    if let Some(inv) = place.invert() {
        p.shader.transform(inv);
    }
    if let Some(r) = Rect::from_xywh(0.0, 0.0, tw as f32, th as f32) {
        layer.fill_rect(r, &p, Transform::identity(), Some(&mask));
    }
    let quality = FilterQuality::Bilinear;
    let mut s = store(ctx);
    if let Some(cv) = s.canvases.get_mut(&id) {
        draw(cv, &st, |pix, m, blend| {
            let pp = PixmapPaint { opacity: 1.0, blend_mode: blend, quality };
            pix.draw_pixmap(0, 0, layer.as_ref(), &pp, to_canvas, m);
        });
    }
    drop(s);
    crate::js::with_dom_mut(|_| ());
    Ok(JsValue::undefined())
}

/// __cvSourceSize(kind, key) -> [w, h] or null: an image source's own size.
fn n_source_size(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let kind = num(a, 0, ctx) as u8;
    let key = string(a, 1, ctx);
    let key = if kind == 0 { img_url(&key).unwrap_or_default() } else { key };
    let size = if kind == 1 {
        key.parse::<NodeId>().ok().and_then(|id| store(ctx).canvases.get(&id).map(|c| (c.pix.width(), c.pix.height())))
    } else {
        PAGE_IMAGES.with(|i| i.borrow().as_ref().and_then(|i| i.get(&key)).map(|i| (i.w, i.h)))
    };
    Ok(match size {
        Some((w, h)) => JsArray::from_iter([JsValue::from(w), JsValue::from(h)], ctx).into(),
        None => JsValue::null(),
    })
}

/// __cvImage(id, kind, key, [sx sy sw sh dx dy dw dh], state): drawImage.
fn n_image(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let kind = num(a, 1, ctx) as u8;
    let key = string(a, 2, ctx);
    let r = floats(a, 3, ctx);
    let st = state(&floats(a, 4, ctx));
    if r.len() < 8 || r.iter().any(|v| !v.is_finite()) {
        return Ok(JsValue::from(false));
    }
    let [sx, sy, sw, sh, dx, dy, dw, dh] = [r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]].map(|v| v as f32);
    if sw == 0.0 || sh == 0.0 || dw == 0.0 || dh == 0.0 {
        return Ok(JsValue::from(true));
    }
    let mut s = store(ctx);
    let Some(src) = source_pixmap(&mut s, kind, &key) else { return Ok(JsValue::from(false)) };
    // Only the source rectangle shows: clamp it to the image, and the destination with it.
    let (iw, ih) = (src.width() as f32, src.height() as f32);
    let (x0, y0) = (sx.min(sx + sw).max(0.0), sy.min(sy + sh).max(0.0));
    let (x1, y1) = (sx.max(sx + sw).min(iw), sy.max(sy + sh).min(ih));
    if x1 <= x0 || y1 <= y0 {
        return Ok(JsValue::from(true));
    }
    let (kx, ky) = (dw / sw, dh / sh);
    let dest = Rect::from_ltrb(dx + (x0 - sx) * kx, dy + (y0 - sy) * ky, dx + (x1 - sx) * kx, dy + (y1 - sy) * ky).or_else(|| Rect::from_ltrb(dx + (x1 - sx) * kx, dy + (y1 - sy) * ky, dx + (x0 - sx) * kx, dy + (y0 - sy) * ky));
    let Some(dest) = dest else { return Ok(JsValue::from(true)) };
    // Image pixel → user space → canvas.
    let img_to_user = Transform::from_row(kx, 0.0, 0.0, ky, dx - sx * kx, dy - sy * ky);
    let quality = if st.smooth { FilterQuality::Bilinear } else { FilterQuality::Nearest };
    let shader = Pattern::new(src.as_ref(), SpreadMode::Pad, quality, st.alpha, st.ctm.pre_concat(img_to_user));
    let paint = Paint { shader, blend_mode: st.blend, anti_alias: true, force_hq_pipeline: false };
    if let Some(cv) = s.canvases.get_mut(&id) {
        draw(cv, &st, |pix, m, blend| {
            let mut p = paint.clone();
            p.blend_mode = blend;
            pix.fill_rect(dest, &p, st.ctm, m);
        });
    }
    drop(s);
    crate::js::with_dom_mut(|_| ());
    Ok(JsValue::from(true))
}

/// The fingerprinting-noise seed for this page (0 = off).
fn fp_seed(ctx: &Context) -> u32 {
    ctx.get_data::<RefCell<crate::js::PageCtx>>().map_or(0, |p| crate::security::fingerprint_seed(&p.borrow().top))
}

/// Straight (not premultiplied) RGBA of a region; outside the canvas reads as transparent.
fn read_region(cv: &Cv, x: i64, y: i64, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    let (cw, ch) = (cv.pix.width() as i64, cv.pix.height() as i64);
    let px = cv.pix.pixels();
    for row in 0..h as i64 {
        let sy = y + row;
        if sy < 0 || sy >= ch {
            continue;
        }
        for col in 0..w as i64 {
            let sx = x + col;
            if sx < 0 || sx >= cw {
                continue;
            }
            let c = px[(sy * cw + sx) as usize].demultiply();
            let o = ((row * w as i64 + col) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
    }
    out
}

/// __cvGetData(id, x, y, w, h) -> Uint8ClampedArray (with per-site noise when protection is on).
fn n_get_data(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let (x, y) = (num(a, 1, ctx).floor() as i64, num(a, 2, ctx).floor() as i64);
    let (w, h) = (num(a, 3, ctx).max(0.0) as u32, num(a, 4, ctx).max(0.0) as u32);
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(boa_engine::JsNativeError::range().with_message("getImageData: too large").into());
    }
    let seed = fp_seed(ctx);
    let mut data = match store(ctx).canvases.get(&id) {
        Some(cv) => read_region(cv, x, y, w, h),
        None => vec![0u8; (w * h * 4) as usize],
    };
    crate::security::noise_rgba(seed, &mut data);
    Ok(JsUint8ClampedArray::from_iter(data, ctx)?.into())
}

/// __cvPutData(id, data, w, h, dx, dy, [dirtyX dirtyY dirtyW dirtyH]): write pixels as they
/// are (no transform, blending or clip).
fn n_put_data(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let bytes = match a.get(1).and_then(|v| v.as_object()) {
        Some(o) => match JsUint8ClampedArray::from_object(o.clone()) {
            Ok(arr) => typed_bytes(&arr, ctx).unwrap_or_default(),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };
    let (w, h) = (num(a, 2, ctx) as i64, num(a, 3, ctx) as i64);
    let (dx, dy) = (num(a, 4, ctx).floor() as i64, num(a, 5, ctx).floor() as i64);
    let dirty = floats(a, 6, ctx);
    let (mut rx, mut ry, mut rw, mut rh) = (0i64, 0i64, w, h);
    if dirty.len() == 4 && dirty.iter().all(|v| v.is_finite()) {
        let (mut x, mut y, mut ww, mut hh) = (dirty[0] as i64, dirty[1] as i64, dirty[2] as i64, dirty[3] as i64);
        if ww < 0 {
            x += ww;
            ww = -ww;
        }
        if hh < 0 {
            y += hh;
            hh = -hh;
        }
        rx = x.max(0);
        ry = y.max(0);
        rw = (x + ww).min(w) - rx;
        rh = (y + hh).min(h) - ry;
    }
    if (w * h * 4) as usize > bytes.len() || rw <= 0 || rh <= 0 {
        return Ok(JsValue::undefined());
    }
    if let Some(cv) = store(ctx).canvases.get_mut(&id) {
        let (cw, ch) = (cv.pix.width() as i64, cv.pix.height() as i64);
        let px = cv.pix.pixels_mut();
        for row in ry..ry + rh {
            let ty = dy + row;
            if ty < 0 || ty >= ch {
                continue;
            }
            for col in rx..rx + rw {
                let tx = dx + col;
                if tx < 0 || tx >= cw {
                    continue;
                }
                let o = ((row * w + col) * 4) as usize;
                px[(ty * cw + tx) as usize] = tiny_skia::ColorU8::from_rgba(bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]).premultiply();
            }
        }
        cv.version += 1;
    }
    crate::js::with_dom_mut(|_| ());
    Ok(JsValue::undefined())
}

/// __cvDataURL(id, type, quality) -> "data:image/png;base64,…" (with the same noise as
/// getImageData). Only PNG is produced, as browsers do for types they don't support.
fn n_data_url(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let seed = fp_seed(ctx);
    let (w, h, mut data) = match store(ctx).canvases.get(&id) {
        Some(cv) => (cv.pix.width(), cv.pix.height(), read_region(cv, 0, 0, cv.pix.width(), cv.pix.height())),
        None => return Ok(JsValue::from(js_string!("data:,"))),
    };
    crate::security::noise_rgba(seed, &mut data);
    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let ok = enc.write_header().and_then(|mut wr| wr.write_image_data(&data));
        if ok.is_err() {
            return Ok(JsValue::from(js_string!("data:,")));
        }
    }
    use base64::Engine;
    let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png_bytes));
    Ok(JsValue::from(JsString::from(url.as_str())))
}

/// __cvInPath(path, x, y, evenOdd) -> is canvas point (x, y) inside the path?
fn n_in_path(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let path = floats(a, 0, ctx);
    let (x, y) = (num(a, 1, ctx) as f32, num(a, 2, ctx) as f32);
    let rule = if num(a, 3, ctx) == 1.0 { FillRule::EvenOdd } else { FillRule::Winding };
    let Some(p) = build_path(&path) else { return Ok(JsValue::from(false)) };
    let Some(mut m) = Mask::new(1, 1) else { return Ok(JsValue::from(false)) };
    m.fill_path(&p, rule, false, Transform::from_translate(0.5 - x, 0.5 - y));
    Ok(JsValue::from(m.data()[0] > 127))
}

/// __cvColor(css) -> [r, g, b, a] (0–255) or null: CSS colors, parsed as stylesheets are.
fn n_color(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let s = string(a, 0, ctx);
    let c = match s.trim().to_ascii_lowercase().as_str() {
        "currentcolor" => Some(crate::style::Color::BLACK),
        t => crate::style::parse_color(t),
    };
    Ok(match c {
        Some(c) => JsArray::from_iter([c.r, c.g, c.b, c.a].map(|v| JsValue::from(v as u32)), ctx).into(),
        None => JsValue::null(),
    })
}
