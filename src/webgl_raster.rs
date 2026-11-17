//! The WebGL drawing pipeline: fetch vertex attributes, run the vertex shader four vertices at
//! a time, assemble and clip primitives, rasterize them in 2×2 quads, run the fragment shader
//! on each quad, then the per-fragment tests (scissor, stencil, depth) and blending.
//!
//! Images use GL's orientation: window and texture row 0 is the bottom row.

use crate::glsl_vm::{run, Lanes, Op, Slot, TexEnv};
use crate::webgl_tex::{e, store, Image, Num};
use std::collections::BTreeMap;

pub mod g {
    pub const POINTS: u32 = 0;
    pub const LINES: u32 = 1;
    pub const LINE_LOOP: u32 = 2;
    pub const LINE_STRIP: u32 = 3;
    pub const TRIANGLES: u32 = 4;
    pub const TRIANGLE_STRIP: u32 = 5;
    pub const TRIANGLE_FAN: u32 = 6;
    pub const FRONT: u32 = 0x404;
    pub const BACK: u32 = 0x405;
    pub const FRONT_AND_BACK: u32 = 0x408;
    pub const KEEP: u32 = 0x1E00;
    pub const REPLACE: u32 = 0x1E01;
    pub const INCR: u32 = 0x1E02;
    pub const DECR: u32 = 0x1E03;
    pub const INVERT: u32 = 0x150A;
    pub const INCR_WRAP: u32 = 0x8507;
    pub const DECR_WRAP: u32 = 0x8508;
    pub const ZERO: u32 = 0;
    pub const ONE: u32 = 1;
    pub const SRC_COLOR: u32 = 0x300;
    pub const ONE_MINUS_SRC_COLOR: u32 = 0x301;
    pub const SRC_ALPHA: u32 = 0x302;
    pub const ONE_MINUS_SRC_ALPHA: u32 = 0x303;
    pub const DST_ALPHA: u32 = 0x304;
    pub const ONE_MINUS_DST_ALPHA: u32 = 0x305;
    pub const DST_COLOR: u32 = 0x306;
    pub const ONE_MINUS_DST_COLOR: u32 = 0x307;
    pub const SRC_ALPHA_SATURATE: u32 = 0x308;
    pub const CONSTANT_COLOR: u32 = 0x8001;
    pub const ONE_MINUS_CONSTANT_COLOR: u32 = 0x8002;
    pub const CONSTANT_ALPHA: u32 = 0x8003;
    pub const ONE_MINUS_CONSTANT_ALPHA: u32 = 0x8004;
    pub const FUNC_ADD: u32 = 0x8006;
    pub const MIN: u32 = 0x8007;
    pub const MAX: u32 = 0x8008;
    pub const FUNC_SUBTRACT: u32 = 0x800A;
    pub const FUNC_REVERSE_SUBTRACT: u32 = 0x800B;
}

#[derive(Clone, Copy, Debug)]
pub struct StencilFace {
    pub func: u32,
    pub reference: i32,
    pub mask: u32,
    pub fail: u32,
    pub zfail: u32,
    pub zpass: u32,
    pub writemask: u32,
}

impl Default for StencilFace {
    fn default() -> Self {
        StencilFace { func: e::ALWAYS, reference: 0, mask: 0xFFFF_FFFF, fail: g::KEEP, zfail: g::KEEP, zpass: g::KEEP, writemask: 0xFFFF_FFFF }
    }
}

/// Fixed-function state for one draw.
#[derive(Clone, Debug)]
pub struct RState {
    pub viewport: [i32; 4],
    pub depth_range: [f32; 2],
    pub scissor: Option<[i32; 4]>,
    /// The face to cull (FRONT, BACK, FRONT_AND_BACK) when culling is on.
    pub cull: Option<u32>,
    pub front_ccw: bool,
    pub depth_test: bool,
    pub depth_func: u32,
    pub depth_mask: bool,
    /// Front and back stencil state when the stencil test is on.
    pub stencil: Option<[StencilFace; 2]>,
    /// Stencil write masks apply even with the test off (they don't, but keeping them here
    /// lets clear() use them).
    pub blend: bool,
    pub blend_eq: [u32; 2],
    pub blend_src: [u32; 2],
    pub blend_dst: [u32; 2],
    pub blend_color: [f32; 4],
    pub color_mask: [bool; 4],
    pub poly_offset: Option<(f32, f32)>,
    pub discard_raster: bool,
}

pub enum AttrSrc<'a> {
    Const([f32; 4]),
    Buf { data: &'a [u8], ty: u32, size: u32, norm: bool, stride: usize, offset: usize, divisor: u32, int: bool },
}

pub struct Attr<'a> {
    pub slot: Slot,
    pub rows: u32,
    /// One source per column (matrices use one attribute location per column).
    pub cols: Vec<AttrSrc<'a>>,
    /// The shader reads ints: integer data is written as bits.
    pub int_attr: bool,
}

pub struct Target {
    pub w: u32,
    pub h: u32,
    /// Images for each draw buffer (None = not drawn to).
    pub color: Vec<Option<Image>>,
    pub depth: Option<Vec<f32>>,
    pub stencil: Option<Vec<u8>>,
}

pub struct Pipeline<'a> {
    pub vs_code: &'a [Op],
    pub vs_mem: Vec<Lanes>,
    pub fs_code: &'a [Op],
    pub fs_mem: Vec<Lanes>,
    pub attrs: Vec<Attr<'a>>,
    /// (vertex shader slot, fragment shader slot, slots, flat)
    pub vary: Vec<(Slot, Slot, u32, bool)>,
    pub pos: Slot,
    pub psize: Option<Slot>,
    pub vertex_id: Option<Slot>,
    pub instance_id: Option<Slot>,
    pub frag_coord: Option<Slot>,
    pub front_facing: Option<Slot>,
    pub point_coord: Option<Slot>,
    pub frag_depth: Option<Slot>,
    /// For each draw buffer, the fragment shader slot (vec4) that feeds it.
    pub outs: Vec<Option<Slot>>,
    pub discards: bool,
    pub tex: &'a dyn TexEnv,
    pub st: &'a RState,
}

/// One shaded vertex: clip position, point size, then varyings.
struct Shaded {
    stride: usize,
    data: Vec<f32>,
}

fn read_attr(src: &AttrSrc, vertex: u32, instance: u32) -> [f32; 4] {
    match src {
        AttrSrc::Const(v) => *v,
        AttrSrc::Buf { data, ty, size, norm, stride, offset, divisor, int } => {
            let idx = if *divisor > 0 { instance / divisor } else { vertex };
            let tsz = match *ty {
                e::BYTE | e::UNSIGNED_BYTE => 1,
                e::SHORT | e::UNSIGNED_SHORT | e::HALF_FLOAT | e::HALF_FLOAT_OES => 2,
                _ => 4,
            };
            let packed = matches!(*ty, 0x8D9F | 0x8368);
            let st = if *stride == 0 { if packed { 4 } else { tsz * *size as usize } } else { *stride };
            let at = offset + idx as usize * st;
            let mut v = [0.0, 0.0, 0.0, if *int { f32::from_bits(1) } else { 1.0 }];
            if packed {
                let Some(b) = data.get(at..at + 4) else { return v };
                let x = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let signed = *ty == 0x8D9F;
                let c = |sh: u32, bits: u32| -> f32 {
                    let raw = (x >> sh) & ((1 << bits) - 1);
                    if signed {
                        let s = ((raw << (32 - bits)) as i32) >> (32 - bits);
                        if *norm { (s as f32 / ((1 << (bits - 1)) - 1) as f32).max(-1.0) } else { s as f32 }
                    } else if *norm {
                        raw as f32 / ((1 << bits) - 1) as f32
                    } else {
                        raw as f32
                    }
                };
                return [c(0, 10), c(10, 10), c(20, 10), c(30, 2)];
            }
            for k in 0..(*size as usize).min(4) {
                let o = at + k * tsz;
                let Some(b) = data.get(o..o + tsz) else { return v };
                v[k] = match *ty {
                    e::BYTE => {
                        let x = b[0] as i8;
                        if *int {
                            f32::from_bits(x as i32 as u32)
                        } else if *norm {
                            (x as f32 / 127.0).max(-1.0)
                        } else {
                            x as f32
                        }
                    }
                    e::UNSIGNED_BYTE => {
                        if *int {
                            f32::from_bits(b[0] as u32)
                        } else if *norm {
                            b[0] as f32 / 255.0
                        } else {
                            b[0] as f32
                        }
                    }
                    e::SHORT => {
                        let x = i16::from_le_bytes([b[0], b[1]]);
                        if *int {
                            f32::from_bits(x as i32 as u32)
                        } else if *norm {
                            (x as f32 / 32767.0).max(-1.0)
                        } else {
                            x as f32
                        }
                    }
                    e::UNSIGNED_SHORT => {
                        let x = u16::from_le_bytes([b[0], b[1]]);
                        if *int {
                            f32::from_bits(x as u32)
                        } else if *norm {
                            x as f32 / 65535.0
                        } else {
                            x as f32
                        }
                    }
                    e::INT => {
                        let x = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        if *int {
                            f32::from_bits(x as u32)
                        } else if *norm {
                            (x as f64 / i32::MAX as f64).max(-1.0) as f32
                        } else {
                            x as f32
                        }
                    }
                    e::UNSIGNED_INT => {
                        let x = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        if *int {
                            f32::from_bits(x)
                        } else if *norm {
                            (x as f64 / u32::MAX as f64) as f32
                        } else {
                            x as f32
                        }
                    }
                    e::HALF_FLOAT | e::HALF_FLOAT_OES => crate::glsl_vm::f32_from_half(u16::from_le_bytes([b[0], b[1]])),
                    _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                };
            }
            v
        }
    }
}

struct Win {
    x: f32,
    y: f32,
    z: f32,
    inv_w: f32,
}

impl Pipeline<'_> {
    fn nvary(&self) -> usize {
        self.vary.iter().map(|v| v.2 as usize).sum()
    }

    /// Run the vertex shader for each listed vertex id.
    fn shade_vertices(&mut self, ids: &[u32], instance: u32) -> Shaded {
        let nv = self.nvary();
        let stride = 5 + nv;
        let mut data = vec![0.0f32; ids.len() * stride];
        if let Some(s) = self.instance_id {
            self.vs_mem[s as usize] = [f32::from_bits(instance); 4];
        }
        for (chunk_i, chunk) in ids.chunks(4).enumerate() {
            let mask = (1u8 << chunk.len()) - 1;
            for a in &self.attrs {
                for (c, src) in a.cols.iter().enumerate() {
                    for (l, &vid) in chunk.iter().enumerate() {
                        let mut v = read_attr(src, vid, instance);
                        match (a.int_attr, src) {
                            // Float data feeding an int attribute: convert the values.
                            (true, AttrSrc::Buf { int: false, .. }) => v = v.map(|x| f32::from_bits(x as i32 as u32)),
                            // Integer data feeding a float attribute.
                            (false, AttrSrc::Buf { int: true, .. }) => v = v.map(|x| x.to_bits() as i32 as f32),
                            _ => {}
                        }
                        for k in 0..a.rows as usize {
                            self.vs_mem[a.slot as usize + c * a.rows as usize + k][l] = v[k];
                        }
                    }
                }
            }
            if let Some(s) = self.vertex_id {
                for (l, &vid) in chunk.iter().enumerate() {
                    self.vs_mem[s as usize][l] = f32::from_bits(vid);
                }
            }
            run(self.vs_code, &mut self.vs_mem, mask, self.tex, false);
            for (l, _) in chunk.iter().enumerate() {
                let o = (chunk_i * 4 + l) * stride;
                for k in 0..4 {
                    data[o + k] = self.vs_mem[self.pos as usize + k][l];
                }
                data[o + 4] = self.psize.map_or(1.0, |s| self.vs_mem[s as usize][l]);
                let mut at = o + 5;
                for (vs, _, n, _) in &self.vary {
                    for k in 0..*n as usize {
                        data[at] = self.vs_mem[*vs as usize + k][l];
                        at += 1;
                    }
                }
            }
        }
        Shaded { stride, data }
    }

    /// Draw `count` vertices (by id) `instances` times.
    pub fn draw(&mut self, t: &mut Target, mode: u32, ids: &[u32], instances: u32) {
        if self.st.discard_raster || ids.is_empty() {
            return;
        }
        // Shade each distinct vertex once per instance.
        let mut uniq: Vec<u32> = Vec::new();
        let mut at: Vec<usize> = Vec::with_capacity(ids.len());
        let mut seen: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for &id in ids {
            let k = *seen.entry(id).or_insert_with(|| {
                uniq.push(id);
                uniq.len() - 1
            });
            at.push(k);
        }
        for inst in 0..instances.max(1) {
            let sh = self.shade_vertices(&uniq, inst);
            let v = |i: usize| &sh.data[at[i] * sh.stride..(at[i] + 1) * sh.stride];
            let n = at.len();
            match mode {
                g::TRIANGLES => {
                    for i in (0..n.saturating_sub(2)).step_by(3) {
                        self.triangle(t, [v(i), v(i + 1), v(i + 2)]);
                    }
                }
                g::TRIANGLE_STRIP => {
                    for i in 0..n.saturating_sub(2) {
                        if i % 2 == 0 {
                            self.triangle(t, [v(i), v(i + 1), v(i + 2)]);
                        } else {
                            self.triangle(t, [v(i + 1), v(i), v(i + 2)]);
                        }
                    }
                }
                g::TRIANGLE_FAN => {
                    for i in 1..n.saturating_sub(1) {
                        self.triangle(t, [v(0), v(i), v(i + 1)]);
                    }
                }
                g::LINES => {
                    for i in (0..n.saturating_sub(1)).step_by(2) {
                        self.line(t, v(i), v(i + 1));
                    }
                }
                g::LINE_STRIP | g::LINE_LOOP => {
                    for i in 0..n.saturating_sub(1) {
                        self.line(t, v(i), v(i + 1));
                    }
                    if mode == g::LINE_LOOP && n > 1 {
                        self.line(t, v(n - 1), v(0));
                    }
                }
                _ => {
                    for i in 0..n {
                        self.point(t, v(i));
                    }
                }
            }
        }
    }

    fn to_window(&self, c: &[f32]) -> Win {
        let w = c[3];
        let [vx, vy, vw, vh] = self.st.viewport;
        let (n, f) = (self.st.depth_range[0], self.st.depth_range[1]);
        let (x, y, z) = (c[0] / w, c[1] / w, c[2] / w);
        Win { x: (x + 1.0) * 0.5 * vw as f32 + vx as f32, y: (y + 1.0) * 0.5 * vh as f32 + vy as f32, z: z * 0.5 * (f - n) + 0.5 * (f + n), inv_w: 1.0 / w }
    }

    /// The pixels fragments may land on: the framebuffer, cut by the scissor box.
    fn bounds(&self, t: &Target) -> [i32; 4] {
        let mut b = [0, 0, t.w as i32, t.h as i32];
        if let Some([x, y, w, h]) = self.st.scissor {
            b = [b[0].max(x), b[1].max(y), b[2].min(x + w), b[3].min(y + h)];
        }
        b
    }

    fn triangle(&mut self, t: &mut Target, v: [&[f32]; 3]) {
        let stride = v[0].len();
        // Flat varyings take the last (provoking) vertex's values.
        let mut verts: Vec<Vec<f32>> = v.iter().map(|x| x.to_vec()).collect();
        let mut at = 5;
        for (_, _, n, flat) in &self.vary {
            if *flat {
                for k in at..at + *n as usize {
                    let pv = verts[2][k];
                    verts[0][k] = pv;
                    verts[1][k] = pv;
                }
            }
            at += *n as usize;
        }
        // Clip against the view volume.
        let poly = clip_polygon(verts, stride);
        if poly.len() < 3 {
            return;
        }
        let win: Vec<Win> = poly.iter().map(|p| self.to_window(p)).collect();
        for i in 1..poly.len() - 1 {
            self.raster_tri(t, [&win[0], &win[i], &win[i + 1]], [&poly[0], &poly[i], &poly[i + 1]]);
        }
    }

    fn raster_tri(&mut self, t: &mut Target, w: [&Win; 3], c: [&Vec<f32>; 3]) {
        let area = (w[1].x - w[0].x) as f64 * (w[2].y - w[0].y) as f64 - (w[2].x - w[0].x) as f64 * (w[1].y - w[0].y) as f64;
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let ccw = area > 0.0;
        let front = ccw == self.st.front_ccw;
        if let Some(face) = self.st.cull {
            if face == g::FRONT_AND_BACK || (face == g::FRONT && front) || (face == g::BACK && !front) {
                return;
            }
        }
        // Order counter-clockwise so inside is where every edge function is positive.
        let (w, c) = if ccw { (w, c) } else { ([w[0], w[2], w[1]], [c[0], c[2], c[1]]) };
        let area = area.abs();
        let p = |i: usize| (w[i].x as f64, w[i].y as f64);
        let edges = [(p(1), p(2)), (p(2), p(0)), (p(0), p(1))];
        let top_left: [bool; 3] = edges.map(|(a, b)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            dy < 0.0 || (dy == 0.0 && dx < 0.0)
        });
        let ef = |k: usize, x: f64, y: f64| {
            let (a, b) = edges[k];
            (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0)
        };
        let b = self.bounds(t);
        let minx = (w.iter().map(|v| v.x).fold(f32::INFINITY, f32::min).floor() as i32).max(b[0]);
        let maxx = (w.iter().map(|v| v.x).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(b[2] - 1);
        let miny = (w.iter().map(|v| v.y).fold(f32::INFINITY, f32::min).floor() as i32).max(b[1]);
        let maxy = (w.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(b[3] - 1);
        if minx > maxx || miny > maxy {
            return;
        }
        // Polygon offset from the depth slope.
        let mut zoff = 0.0f32;
        if let Some((factor, units)) = self.st.poly_offset {
            let (x0, y0, z0) = (w[0].x, w[0].y, w[0].z);
            let det = (w[1].x - x0) * (w[2].y - y0) - (w[2].x - x0) * (w[1].y - y0);
            let dzdx = ((w[1].z - z0) * (w[2].y - y0) - (w[2].z - z0) * (w[1].y - y0)) / det;
            let dzdy = ((w[1].x - x0) * (w[2].z - z0) - (w[2].x - x0) * (w[1].z - z0)) / det;
            zoff = factor * dzdx.abs().max(dzdy.abs()) + units / 16_777_216.0;
        }
        let nv = self.nvary();
        let interp = |px: f32, py: f32, out: &mut [f32]| -> (f32, f32) {
            let (x, y) = (px as f64, py as f64);
            let bs = [ef(0, x, y) / area, ef(1, x, y) / area, ef(2, x, y) / area];
            let pw = [bs[0] as f32 * w[0].inv_w, bs[1] as f32 * w[1].inv_w, bs[2] as f32 * w[2].inv_w];
            let sum = pw[0] + pw[1] + pw[2];
            let k = if sum != 0.0 { 1.0 / sum } else { 0.0 };
            for (i, o) in out.iter_mut().enumerate().take(nv) {
                *o = (pw[0] * c[0][5 + i] + pw[1] * c[1][5 + i] + pw[2] * c[2][5 + i]) * k;
            }
            let z = bs[0] as f32 * w[0].z + bs[1] as f32 * w[1].z + bs[2] as f32 * w[2].z + zoff;
            (z, sum)
        };
        let qx0 = minx & !1;
        let qy0 = miny & !1;
        let mut y = qy0;
        while y <= maxy {
            let mut x = qx0;
            while x <= maxx {
                let mut cover = 0u8;
                for l in 0..4 {
                    let (px, py) = (x + (l & 1) as i32, y + (l >> 1) as i32);
                    if px < minx || px > maxx || py < miny || py > maxy {
                        continue;
                    }
                    let (cx, cy) = (px as f64 + 0.5, py as f64 + 0.5);
                    let mut inside = true;
                    for k in 0..3 {
                        let ev = ef(k, cx, cy);
                        if ev < 0.0 || (ev == 0.0 && !top_left[k]) {
                            inside = false;
                            break;
                        }
                    }
                    if inside {
                        cover |= 1 << l;
                    }
                }
                if cover != 0 {
                    self.shade_quad(t, x, y, cover, front, &interp, None);
                }
                x += 2;
            }
            y += 2;
        }
    }

    fn line(&mut self, t: &mut Target, a: &[f32], b: &[f32]) {
        let stride = a.len();
        let (mut a, mut b) = (a.to_vec(), b.to_vec());
        // Flat varyings come from the second vertex.
        let mut at = 5;
        for (_, _, n, flat) in &self.vary {
            if *flat {
                for k in at..at + *n as usize {
                    a[k] = b[k];
                }
            }
            at += *n as usize;
        }
        let Some((a, b)) = clip_line(a, b, stride) else { return };
        let (wa, wb) = (self.to_window(&a), self.to_window(&b));
        let (dx, dy) = (wb.x - wa.x, wb.y - wa.y);
        let len2 = dx * dx + dy * dy;
        if len2 == 0.0 {
            return;
        }
        let bd = self.bounds(t);
        let mut pixels: BTreeMap<(i32, i32), u8> = BTreeMap::new();
        let mut add = |px: i32, py: i32| {
            if px >= bd[0] && px < bd[2] && py >= bd[1] && py < bd[3] {
                let l = ((py & 1) << 1 | (px & 1)) as u8;
                *pixels.entry((px & !1, py & !1)).or_insert(0) |= 1 << l;
            }
        };
        if dx.abs() >= dy.abs() {
            let (x0, x1) = (wa.x.min(wb.x), wa.x.max(wb.x));
            let start = (x0 - 0.5).ceil() as i32;
            let end = (x1 - 0.5).ceil() as i32;
            for px in start..end {
                let cx = px as f32 + 0.5;
                let yy = wa.y + (cx - wa.x) * dy / dx;
                add(px, yy.floor() as i32);
            }
        } else {
            let (y0, y1) = (wa.y.min(wb.y), wa.y.max(wb.y));
            let start = (y0 - 0.5).ceil() as i32;
            let end = (y1 - 0.5).ceil() as i32;
            for py in start..end {
                let cy = py as f32 + 0.5;
                let xx = wa.x + (cy - wa.y) * dx / dy;
                add(xx.floor() as i32, py);
            }
        }
        let nv = self.nvary();
        let interp = |px: f32, py: f32, out: &mut [f32]| -> (f32, f32) {
            let tt = ((px - wa.x) * dx + (py - wa.y) * dy) / len2;
            let (p0, p1) = ((1.0 - tt) * wa.inv_w, tt * wb.inv_w);
            let sum = p0 + p1;
            let k = if sum != 0.0 { 1.0 / sum } else { 0.0 };
            for (i, o) in out.iter_mut().enumerate().take(nv) {
                *o = (p0 * a[5 + i] + p1 * b[5 + i]) * k;
            }
            (wa.z + (wb.z - wa.z) * tt, sum)
        };
        for ((qx, qy), cover) in pixels {
            self.shade_quad(t, qx, qy, cover, true, &interp, None);
        }
    }

    fn point(&mut self, t: &mut Target, v: &[f32]) {
        let (x, y, z, w) = (v[0], v[1], v[2], v[3]);
        if w <= 0.0 || z < -w || z > w || x < -w || x > w || y < -w || y > w {
            return;
        }
        let win = self.to_window(v);
        let size = v[4].clamp(1.0, 1024.0);
        let half = size * 0.5;
        let bd = self.bounds(t);
        let x0 = ((win.x - half - 0.5).ceil() as i32).max(bd[0]);
        let x1 = ((win.x + half - 0.5).ceil() as i32).min(bd[2]);
        let y0 = ((win.y - half - 0.5).ceil() as i32).max(bd[1]);
        let y1 = ((win.y + half - 0.5).ceil() as i32).min(bd[3]);
        let mut pixels: BTreeMap<(i32, i32), u8> = BTreeMap::new();
        for py in y0..y1 {
            for px in x0..x1 {
                let l = ((py & 1) << 1 | (px & 1)) as u8;
                *pixels.entry((px & !1, py & !1)).or_insert(0) |= 1 << l;
            }
        }
        let nv = self.nvary();
        let interp = |_: f32, _: f32, out: &mut [f32]| -> (f32, f32) {
            out[..nv].copy_from_slice(&v[5..5 + nv]);
            (win.z, win.inv_w)
        };
        for ((qx, qy), cover) in pixels {
            self.shade_quad(t, qx, qy, cover, true, &interp, Some((win.x, win.y, size)));
        }
    }

    /// Shade one 2×2 quad (lanes: (x,y) (x+1,y) (x,y+1) (x+1,y+1)) and write the covered,
    /// surviving fragments.
    #[allow(clippy::too_many_arguments)]
    fn shade_quad(&mut self, t: &mut Target, qx: i32, qy: i32, mut cover: u8, front: bool, interp: &dyn Fn(f32, f32, &mut [f32]) -> (f32, f32), point: Option<(f32, f32, f32)>) {
        let nv = self.nvary();
        let mut vals = [[0.0f32; 64]; 4];
        let mut big: Vec<Vec<f32>> = Vec::new();
        if nv > 64 {
            big = vec![vec![0.0; nv]; 4];
        }
        let mut zs = [0.0f32; 4];
        let mut ws = [1.0f32; 4];
        for l in 0..4 {
            let (px, py) = ((qx + (l & 1) as i32) as f32 + 0.5, (qy + (l >> 1) as i32) as f32 + 0.5);
            let out: &mut [f32] = if nv > 64 { &mut big[l] } else { &mut vals[l][..nv.max(1)] };
            let (z, w) = interp(px, py, out);
            zs[l] = z.clamp(0.0, 1.0);
            ws[l] = w;
        }
        let w = t.w as i32;
        // Early depth test when the shader can't change depth or discard.
        let early = self.st.depth_test && t.depth.is_some() && self.st.stencil.is_none() && self.frag_depth.is_none() && !self.discards;
        if early {
            let db = t.depth.as_ref().unwrap();
            for l in 0..4 {
                if cover & (1 << l) == 0 {
                    continue;
                }
                let (px, py) = (qx + (l & 1) as i32, qy + (l >> 1) as i32);
                let d = db[(py * w + px) as usize];
                if !depth_pass(self.st.depth_func, zs[l], d) {
                    cover &= !(1 << l);
                }
            }
            if cover == 0 {
                return;
            }
        }
        // Fragment shader inputs.
        let mem = &mut self.fs_mem;
        for l in 0..4 {
            let (px, py) = ((qx + (l & 1) as i32) as f32 + 0.5, (qy + (l >> 1) as i32) as f32 + 0.5);
            if let Some(s) = self.frag_coord {
                mem[s as usize][l] = px;
                mem[s as usize + 1][l] = py;
                mem[s as usize + 2][l] = zs[l];
                mem[s as usize + 3][l] = ws[l];
            }
            if let Some(s) = self.front_facing {
                mem[s as usize][l] = f32::from_bits(front as u32);
            }
            if let (Some(s), Some((cx, cy, size))) = (self.point_coord, point) {
                mem[s as usize][l] = 0.5 + (px - cx) / size;
                mem[s as usize + 1][l] = 0.5 - (py - cy) / size;
            }
            let src: &[f32] = if nv > 64 { &big[l] } else { &vals[l][..nv] };
            let mut k = 0;
            for (_, fs, n, _) in &self.vary {
                for c in 0..*n as usize {
                    mem[*fs as usize + c][l] = src[k];
                    k += 1;
                }
            }
        }
        if let Some(s) = self.frag_depth {
            for l in 0..4 {
                mem[s as usize][l] = zs[l];
            }
        }
        let alive = run(self.fs_code, mem, 0xF, self.tex, true);
        let cover = cover & alive;
        if cover == 0 {
            return;
        }
        for l in 0..4 {
            if cover & (1 << l) == 0 {
                continue;
            }
            let (px, py) = (qx + (l & 1) as i32, qy + (l >> 1) as i32);
            let i = (py * w + px) as usize;
            let z = match self.frag_depth {
                Some(s) => self.fs_mem[s as usize][l].clamp(0.0, 1.0),
                None => zs[l],
            };
            // Stencil and depth tests.
            let mut stencil_ok = true;
            if let (Some(sf), Some(sb)) = (self.st.stencil, t.stencil.as_mut()) {
                let f = if front { sf[0] } else { sf[1] };
                let cur = sb[i];
                stencil_ok = stencil_pass(&f, cur);
                if !stencil_ok {
                    sb[i] = stencil_op(f.fail, cur, &f);
                    continue;
                }
            }
            let mut depth_ok = true;
            if self.st.depth_test {
                if let Some(db) = t.depth.as_ref() {
                    depth_ok = depth_pass(self.st.depth_func, z, db[i]);
                }
            }
            if let (Some(sf), Some(sb)) = (self.st.stencil, t.stencil.as_mut()) {
                let f = if front { sf[0] } else { sf[1] };
                let cur = sb[i];
                sb[i] = stencil_op(if depth_ok { f.zpass } else { f.zfail }, cur, &f);
            }
            let _ = stencil_ok;
            if !depth_ok {
                continue;
            }
            if self.st.depth_test && self.st.depth_mask {
                if let Some(db) = t.depth.as_mut() {
                    db[i] = z;
                }
            }
            for (bi, img) in t.color.iter_mut().enumerate() {
                let (Some(img), Some(Some(slot))) = (img.as_mut(), self.outs.get(bi)) else { continue };
                let s = *slot as usize;
                let src = [self.fs_mem[s][l], self.fs_mem[s + 1][l], self.fs_mem[s + 2][l], self.fs_mem[s + 3][l]];
                write_color(img, i, src, self.st);
            }
        }
    }
}

pub fn write_color(img: &mut Image, i: usize, src: [f32; 4], st: &RState) {
    let dst = img.px[i];
    let int = img.fmt.is_integer();
    let mut v = if st.blend && !int { blend(src, dst, st, matches!(img.fmt.num, Num::Unorm(_) | Num::Snorm)) } else { src };
    if !int {
        v = store(&img.fmt, v);
    }
    for c in 0..4 {
        if !st.color_mask[c] {
            v[c] = dst[c];
        }
    }
    // Channels the format lacks keep their defaults.
    img.px[i] = if int { crate::webgl_tex::expand(&img.fmt, v) } else { v };
}

fn blend(s: [f32; 4], d: [f32; 4], st: &RState, clamp: bool) -> [f32; 4] {
    let (s, cc) = if clamp { (s.map(|x| x.clamp(0.0, 1.0)), st.blend_color.map(|x| x.clamp(0.0, 1.0))) } else { (s, st.blend_color) };
    let mut out = [0.0; 4];
    for c in 0..4 {
        let a = c == 3;
        let (eq, sf, df) = if a { (st.blend_eq[1], st.blend_src[1], st.blend_dst[1]) } else { (st.blend_eq[0], st.blend_src[0], st.blend_dst[0]) };
        let fs = factor(sf, s, d, cc, c);
        let fd = factor(df, s, d, cc, c);
        out[c] = match eq {
            g::FUNC_SUBTRACT => s[c] * fs - d[c] * fd,
            g::FUNC_REVERSE_SUBTRACT => d[c] * fd - s[c] * fs,
            g::MIN => s[c].min(d[c]),
            g::MAX => s[c].max(d[c]),
            _ => s[c] * fs + d[c] * fd,
        };
    }
    out
}

fn factor(f: u32, s: [f32; 4], d: [f32; 4], k: [f32; 4], c: usize) -> f32 {
    match f {
        g::ZERO => 0.0,
        g::ONE => 1.0,
        g::SRC_COLOR => s[c],
        g::ONE_MINUS_SRC_COLOR => 1.0 - s[c],
        g::DST_COLOR => d[c],
        g::ONE_MINUS_DST_COLOR => 1.0 - d[c],
        g::SRC_ALPHA => s[3],
        g::ONE_MINUS_SRC_ALPHA => 1.0 - s[3],
        g::DST_ALPHA => d[3],
        g::ONE_MINUS_DST_ALPHA => 1.0 - d[3],
        g::CONSTANT_COLOR => k[c],
        g::ONE_MINUS_CONSTANT_COLOR => 1.0 - k[c],
        g::CONSTANT_ALPHA => k[3],
        g::ONE_MINUS_CONSTANT_ALPHA => 1.0 - k[3],
        g::SRC_ALPHA_SATURATE => {
            if c == 3 {
                1.0
            } else {
                s[3].min(1.0 - d[3])
            }
        }
        _ => 0.0,
    }
}

pub fn depth_pass(func: u32, z: f32, d: f32) -> bool {
    match func {
        e::NEVER => false,
        e::LESS => z < d,
        e::EQUAL => z == d,
        e::LEQUAL => z <= d,
        e::GREATER => z > d,
        e::NOTEQUAL => z != d,
        e::GEQUAL => z >= d,
        _ => true,
    }
}

fn stencil_pass(f: &StencilFace, cur: u8) -> bool {
    let r = (f.reference.clamp(0, 255) as u32) & f.mask & 0xff;
    let s = cur as u32 & f.mask & 0xff;
    match f.func {
        e::NEVER => false,
        e::LESS => r < s,
        e::LEQUAL => r <= s,
        e::GREATER => r > s,
        e::GEQUAL => r >= s,
        e::EQUAL => r == s,
        e::NOTEQUAL => r != s,
        _ => true,
    }
}

fn stencil_op(op: u32, cur: u8, f: &StencilFace) -> u8 {
    let v = match op {
        g::ZERO => 0,
        g::REPLACE => f.reference.clamp(0, 255) as u8,
        g::INCR => cur.saturating_add(1),
        g::DECR => cur.saturating_sub(1),
        g::INVERT => !cur,
        g::INCR_WRAP => cur.wrapping_add(1),
        g::DECR_WRAP => cur.wrapping_sub(1),
        _ => cur,
    };
    let m = f.writemask as u8;
    (cur & !m) | (v & m)
}

/// Signed distances to the six clip planes.
fn plane_dist(v: &[f32], p: usize) -> f32 {
    let w = v[3];
    match p {
        0 => w + v[0],
        1 => w - v[0],
        2 => w + v[1],
        3 => w - v[1],
        4 => w + v[2],
        _ => w - v[2],
    }
}

fn lerp_vert(a: &[f32], b: &[f32], t: f32) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + (y - x) * t).collect()
}

fn clip_polygon(mut poly: Vec<Vec<f32>>, _stride: usize) -> Vec<Vec<f32>> {
    // Quick accept when every vertex is inside.
    if poly.iter().all(|v| (0..6).all(|p| plane_dist(v, p) >= 0.0)) {
        return poly;
    }
    for p in 0..6 {
        if poly.is_empty() {
            break;
        }
        let mut out = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let a = &poly[i];
            let b = &poly[(i + 1) % poly.len()];
            let (da, db) = (plane_dist(a, p), plane_dist(b, p));
            if da >= 0.0 {
                out.push(a.clone());
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                out.push(lerp_vert(a, b, t));
            }
        }
        poly = out;
    }
    poly
}

fn clip_line(mut a: Vec<f32>, mut b: Vec<f32>, _stride: usize) -> Option<(Vec<f32>, Vec<f32>)> {
    for p in 0..6 {
        let (da, db) = (plane_dist(&a, p), plane_dist(&b, p));
        if da < 0.0 && db < 0.0 {
            return None;
        }
        if da < 0.0 {
            a = lerp_vert(&a, &b, da / (da - db));
        } else if db < 0.0 {
            b = lerp_vert(&b, &a, db / (db - da));
        }
    }
    Some((a, b))
}
