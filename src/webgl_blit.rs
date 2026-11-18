//! blitFramebuffer (WebGL 2): validation, then color (NEAREST or LINEAR, with sRGB decode and
//! encode), depth and stencil copies between the read and draw framebuffers.
//!
//! A child module of webgl.rs, so it can use the context's private state.

use super::*;
use crate::webgl_tex as tex;

/// Where a blit reads from or writes to: an attachment, or the default framebuffer.
#[derive(Clone, Copy, PartialEq)]
enum Spot {
    Att(Att),
    Default,
}

impl Gl {
    fn att_samples(&self, a: &Att) -> u32 {
        match a {
            Att::Rbo(id) => match self.objs.get(id) {
                Some(Obj::Rbo(r)) => r.samples,
                _ => 0,
            },
            Att::Tex { .. } => 0,
        }
    }

    /// The read framebuffer's color image as a copy, and where it lives.
    fn blit_color_source(&self) -> Option<(Image, Spot)> {
        if self.read_fbo == 0 {
            return (self.fb_read_buffer != 0).then(|| (self.fb_color.clone(), Spot::Default));
        }
        let f = self.fbo(self.read_fbo)?;
        if f.read_buffer == 0 {
            return None;
        }
        let a = f.color[(f.read_buffer - COLOR_ATTACHMENT0) as usize]?;
        let img = self.att_image(&a)?;
        // One layer of a 3D or array texture.
        let img = match a {
            Att::Tex { layer, .. } if img.d > 1 => {
                let n = (img.w * img.h) as usize;
                let s = n * layer.min(img.d - 1) as usize;
                Image { w: img.w, h: img.h, d: 1, fmt: img.fmt, px: img.px[s..s + n].to_vec(), st: Vec::new() }
            }
            _ => img.clone(),
        };
        Some((img, Spot::Att(a)))
    }

    /// The draw framebuffer's color images being written (formats and where they live).
    fn blit_color_dests(&self) -> Vec<(Fmt, Spot, u32)> {
        if self.draw_fbo == 0 {
            return if self.fb_draw_buffer == BACK { vec![(self.fb_color.fmt, Spot::Default, 0)] } else { Vec::new() };
        }
        let Some(f) = self.fbo(self.draw_fbo) else { return Vec::new() };
        let mut out = Vec::new();
        for (i, &b) in f.draw_buffers.iter().enumerate() {
            if b == 0 {
                continue;
            }
            if let Some(a) = f.color[i] {
                if let Some(img) = self.att_image(&a) {
                    out.push((img.fmt, Spot::Att(a), self.att_samples(&a)));
                }
            }
        }
        out
    }

    /// The depth (or stencil) attachment of the read or draw framebuffer: format, place, samples.
    fn blit_ds(&self, read: bool, stencil: bool) -> Option<(Option<Fmt>, Spot, u32)> {
        let fid = if read { self.read_fbo } else { self.draw_fbo };
        if fid == 0 {
            let has = if stencil { self.fb_stencil.is_some() } else { self.fb_depth.is_some() };
            return has.then_some((None, Spot::Default, 0));
        }
        let f = self.fbo(fid)?;
        let a = if stencil { f.stencil } else { f.depth }?;
        let img = self.att_image(&a)?;
        Some((Some(img.fmt), Spot::Att(a), self.att_samples(&a)))
    }

    pub(super) fn blit(&mut self, r: [i64; 8], mask: u32, filter: u32) {
        if mask & !(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            return self.error(INVALID_VALUE);
        }
        // WebGL: a width or height that doesn't fit in 32 bits is INVALID_VALUE.
        if [(0, 2), (1, 3), (4, 6), (5, 7)].iter().any(|&(a, b)| (r[b] - r[a]).abs() > i32::MAX as i64) {
            return self.error(INVALID_VALUE);
        }
        if filter != tex::e::NEAREST && filter != tex::e::LINEAR {
            return self.error(INVALID_ENUM);
        }
        if filter == tex::e::LINEAR && mask & (DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            return self.error(INVALID_OPERATION);
        }
        for fid in [self.read_fbo, self.draw_fbo] {
            if fid != 0 && self.fbo_status(fid) != FRAMEBUFFER_COMPLETE {
                return self.error(INVALID_FRAMEBUFFER_OPERATION);
            }
        }
        let same_rects = r[0..4] == r[4..8];
        let mut src_samples = 0;
        let mut dst_samples = 0;

        let mut color = None;
        if mask & COLOR_BUFFER_BIT != 0 {
            let dests = self.blit_color_dests();
            if let Some((img, spot)) = self.blit_color_source() {
                let sf = img.fmt;
                if let Spot::Att(a) = spot {
                    src_samples = self.att_samples(&a);
                }
                for (df, dspot, ds) in &dests {
                    dst_samples = dst_samples.max(*ds);
                    if sf.is_integer() != df.is_integer() || (sf.is_integer() && sf.num != df.num) {
                        return self.error(INVALID_OPERATION);
                    }
                    if *dspot == spot {
                        return self.error(INVALID_OPERATION);
                    }
                    if src_samples > 0 && (sized(df) != sized(&sf) || !same_rects) {
                        return self.error(INVALID_OPERATION);
                    }
                }
                if filter == tex::e::LINEAR && sf.is_integer() {
                    return self.error(INVALID_OPERATION);
                }
                if !dests.is_empty() {
                    color = Some(img);
                }
            }
        }
        let mut ds_copy = [false; 2];
        for (k, bit) in [(0, DEPTH_BUFFER_BIT), (1, STENCIL_BUFFER_BIT)] {
            if mask & bit == 0 {
                continue;
            }
            let (Some(s), Some(d)) = (self.blit_ds(true, k == 1), self.blit_ds(false, k == 1)) else { continue };
            src_samples = src_samples.max(s.2);
            dst_samples = dst_samples.max(d.2);
            if s.1 == d.1 {
                return self.error(INVALID_OPERATION);
            }
            if let (Some(a), Some(b)) = (s.0, d.0) {
                if a.internal != b.internal {
                    return self.error(INVALID_OPERATION);
                }
            }
            if s.2 > 0 && !same_rects {
                return self.error(INVALID_OPERATION);
            }
            ds_copy[k] = true;
        }
        if dst_samples > 0 {
            return self.error(INVALID_OPERATION);
        }
        if color.is_none() && !ds_copy[0] && !ds_copy[1] {
            return;
        }
        // Source depth and stencil, with the source width.
        let src_depth = ds_copy[0].then(|| self.blit_ds_values(true)).flatten();
        let src_stencil = ds_copy[1].then(|| self.blit_stencil_values()).flatten();

        let Some((mut t, restore, d, s)) = self.take_target(false) else { return };
        let st = RState { scissor: self.cap(SCISSOR_TEST).then_some(self.scissor), ..self.rstate() };
        let bounds = clip_rect(&st, t.w, t.h);
        let map = Map::new(r);
        let (x0, x1) = (r[4].min(r[6]).max(bounds[0] as i64), r[4].max(r[6]).min(bounds[2] as i64));
        let (y0, y1) = (r[5].min(r[7]).max(bounds[1] as i64), r[5].max(r[7]).min(bounds[3] as i64));

        if let Some(src) = &color {
            let linear = filter == tex::e::LINEAR;
            for img in t.color.iter_mut().flatten() {
                let df = img.fmt;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let Some(v) = map.sample(src, x, y, linear) else { continue };
                        let i = (y as u32 * img.w + x as u32) as usize;
                        img.px[i] = if df.is_integer() { tex::expand(&df, v) } else { tex::store(&df, v) };
                    }
                }
            }
        }
        for (vals, k) in [(src_depth.map(|(v, w, h)| (v.into_iter().map(|x| x.to_bits()).collect::<Vec<u32>>(), w, h)), 0), (src_stencil.map(|(v, w, h)| (v.into_iter().map(u32::from).collect(), w, h)), 1)] {
            let Some((vals, sw, sh)) = vals else { continue };
            let tw = t.w;
            for y in y0..y1 {
                for x in x0..x1 {
                    let Some((u, v)) = map.nearest(x, y, sw, sh) else { continue };
                    let val = vals[(v * sw + u) as usize];
                    let i = (y as u32 * tw + x as u32) as usize;
                    if k == 0 {
                        if let Some(dv) = t.depth.as_mut().and_then(|d| d.get_mut(i)) {
                            *dv = f32::from_bits(val);
                        }
                    } else if let Some(sv) = t.stencil.as_mut().and_then(|s| s.get_mut(i)) {
                        *sv = val as u8;
                    }
                }
            }
        }
        self.put_target(t, restore, d, s, false);
        self.dirty = true;
    }

    fn blit_ds_values(&self, _read: bool) -> Option<(Vec<f32>, u32, u32)> {
        if self.read_fbo == 0 {
            return self.fb_depth.clone().map(|d| (d, self.w, self.h));
        }
        let a = self.fbo(self.read_fbo)?.depth?;
        let img = self.att_image(&a)?;
        Some((img.px.iter().take((img.w * img.h) as usize).map(|p| p[0]).collect(), img.w, img.h))
    }

    fn blit_stencil_values(&self) -> Option<(Vec<u8>, u32, u32)> {
        if self.read_fbo == 0 {
            return self.fb_stencil.clone().map(|s| (s, self.w, self.h));
        }
        let a = self.fbo(self.read_fbo)?.stencil?;
        let img = self.att_image(&a)?;
        let n = (img.w * img.h) as usize;
        Some((if img.st.len() >= n { img.st[..n].to_vec() } else { vec![0; n] }, img.w, img.h))
    }
}

/// The sized format of an unsized byte format (the default framebuffer is RGBA or RGB).
fn sized(f: &Fmt) -> u32 {
    match f.internal {
        tex::e::RGBA => tex::e::RGBA8,
        tex::e::RGB => tex::e::RGB8,
        i => i,
    }
}

/// Destination pixel → source position (OpenGL ES 3.0 §4.3.3: pixel centers map linearly;
/// either rectangle may be flipped).
struct Map {
    r: [f64; 8],
    /// Source rectangle, low and high corners.
    lo: [i64; 2],
    hi: [i64; 2],
}

impl Map {
    fn new(r: [i64; 8]) -> Map {
        Map { r: r.map(|v| v as f64), lo: [r[0].min(r[2]), r[1].min(r[3])], hi: [r[0].max(r[2]), r[1].max(r[3])] }
    }

    fn pos(&self, x: i64, y: i64) -> (f64, f64) {
        let r = &self.r;
        let u = r[0] + (x as f64 + 0.5 - r[4]) * (r[2] - r[0]) / (r[6] - r[4]);
        let v = r[1] + (y as f64 + 0.5 - r[5]) * (r[3] - r[1]) / (r[7] - r[5]);
        (u, v)
    }

    /// The source pixel for this destination pixel, if it's inside the source rectangle and image.
    fn nearest(&self, x: i64, y: i64, w: u32, h: u32) -> Option<(u32, u32)> {
        let (u, v) = self.pos(x, y);
        let (u, v) = (u.floor() as i64, v.floor() as i64);
        if u < self.lo[0].max(0) || v < self.lo[1].max(0) || u >= self.hi[0].min(w as i64) || v >= self.hi[1].min(h as i64) {
            return None;
        }
        Some((u as u32, v as u32))
    }

    /// Linear color at the source position (NEAREST, or bilinear between texel centers clamped
    /// to the readable part of the source rectangle). Destination pixels whose source falls
    /// outside the read image are left alone.
    fn sample(&self, img: &Image, x: i64, y: i64, linear: bool) -> Option<[f32; 4]> {
        let (cu, cv) = self.nearest(x, y, img.w, img.h)?;
        let at = |u: i64, v: i64| tex::to_linear(&img.fmt, img.px[(v as u32 * img.w + u as u32) as usize]);
        if !linear {
            return Some(at(cu as i64, cv as i64));
        }
        let (u, v) = self.pos(x, y);
        let (u, v) = (u - 0.5, v - 0.5);
        let (iu, iv) = (u.floor(), v.floor());
        let (fu, fv) = ((u - iu) as f32, (v - iv) as f32);
        let cl = |p: i64, k: usize, size: u32| p.clamp(self.lo[k].max(0), (self.hi[k].min(size as i64) - 1).max(0));
        let (u0, u1) = (cl(iu as i64, 0, img.w), cl(iu as i64 + 1, 0, img.w));
        let (v0, v1) = (cl(iv as i64, 1, img.h), cl(iv as i64 + 1, 1, img.h));
        let (a, b, c, d) = (at(u0, v0), at(u1, v0), at(u0, v1), at(u1, v1));
        Some(std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fu;
            let bot = c[k] + (d[k] - c[k]) * fu;
            top + (bot - top) * fv
        }))
    }
}
