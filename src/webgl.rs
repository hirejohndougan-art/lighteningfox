//! WebGL 1 and 2 for `<canvas>`: the context state, objects (buffers, textures, shaders,
//! programs, framebuffers, renderbuffers, vertex arrays, samplers) and the `__gl*` natives
//! that `webgl.js` calls. Shaders compile with `glsl_gen`, drawing runs in `webgl_raster`,
//! textures and pixel formats live in `webgl_tex`.
//!
//! Rendering is in software, so it works the same everywhere (headless too). The drawing
//! buffer is handed to the canvas's pixels when the page is painted (`present`), or when
//! a script reads the canvas (toDataURL, drawImage).

use crate::dom::NodeId;
use crate::glsl::{Base, Dim, Ty};
use crate::glsl_gen::{compile, read_std140, Compiled, Stage};
use crate::glsl_vm::Lanes;
use crate::webgl_raster::{g, Attr, AttrSrc, Pipeline, RState, StencilFace, Target};
use crate::webgl_tex::{self as tex, e, Fmt, Image, Num, Sampler, TexParams, Texture, Unpack, UnitView};
use boa_engine::object::builtins::{JsArray, JsArrayBuffer, JsDataView, JsFloat32Array, JsInt32Array, JsTypedArray, JsUint32Array};
use boa_engine::{js_string, Context, JsResult, JsString, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// ---------------------------------------------------------------- GL enums (not in webgl_tex)

const INVALID_ENUM: u32 = 0x500;
const INVALID_VALUE: u32 = 0x501;
const INVALID_OPERATION: u32 = 0x502;
const INVALID_FRAMEBUFFER_OPERATION: u32 = 0x506;
const DEPTH_BUFFER_BIT: u32 = 0x100;
const STENCIL_BUFFER_BIT: u32 = 0x400;
const COLOR_BUFFER_BIT: u32 = 0x4000;
const ARRAY_BUFFER: u32 = 0x8892;
const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const COPY_READ_BUFFER: u32 = 0x8F36;
const COPY_WRITE_BUFFER: u32 = 0x8F37;
const PIXEL_PACK_BUFFER: u32 = 0x88EB;
const PIXEL_UNPACK_BUFFER: u32 = 0x88EC;
const UNIFORM_BUFFER: u32 = 0x8A11;
const TRANSFORM_FEEDBACK_BUFFER: u32 = 0x8C8E;
const CULL_FACE: u32 = 0xB44;
const BLEND: u32 = 0xBE2;
const DITHER: u32 = 0xBD0;
const STENCIL_TEST: u32 = 0xB90;
const DEPTH_TEST: u32 = 0xB71;
const SCISSOR_TEST: u32 = 0xC11;
const POLYGON_OFFSET_FILL: u32 = 0x8037;
const SAMPLE_ALPHA_TO_COVERAGE: u32 = 0x809E;
const SAMPLE_COVERAGE: u32 = 0x80A0;
const RASTERIZER_DISCARD: u32 = 0x8C89;
const FRAMEBUFFER: u32 = 0x8D40;
const READ_FRAMEBUFFER: u32 = 0x8CA8;
const DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const RENDERBUFFER: u32 = 0x8D41;
const COLOR_ATTACHMENT0: u32 = 0x8CE0;
const DEPTH_ATTACHMENT: u32 = 0x8D00;
const STENCIL_ATTACHMENT: u32 = 0x8D20;
const DEPTH_STENCIL_ATTACHMENT: u32 = 0x821A;
const FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const FRAMEBUFFER_INCOMPLETE_ATTACHMENT: u32 = 0x8CD6;
const FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT: u32 = 0x8CD7;
const FRAMEBUFFER_INCOMPLETE_DIMENSIONS: u32 = 0x8CD9;
const FRAMEBUFFER_UNSUPPORTED: u32 = 0x8CDD;
const BACK: u32 = 0x405;
const COLOR: u32 = 0x1800;
const DEPTH: u32 = 0x1801;
const STENCIL: u32 = 0x1802;
const DEPTH_STENCIL: u32 = 0x84F9;
const VERTEX_SHADER: u32 = 0x8B31;
const FRAGMENT_SHADER: u32 = 0x8B30;
const TEXTURE0: u32 = 0x84C0;

const MAX_UNITS: usize = 32;
const MAX_ATTRIBS: usize = 16;
const MAX_TEXTURE_SIZE: u32 = 8192;
const MAX_RENDERBUFFER_SIZE: u32 = 8192;
const MAX_COLOR_ATTACHMENTS: usize = 8;
const MAX_UNIFORM_BINDINGS: usize = 24;

// ---------------------------------------------------------------- objects

#[derive(Default)]
struct Buffer {
    data: Vec<u8>,
    usage: u32,
    /// 0 = never bound, 1 = ELEMENT_ARRAY_BUFFER, 2 = anything else (WebGL forbids mixing).
    kind: u8,
}

struct Shader {
    ty: u32,
    src: String,
    compiled: Option<Rc<Compiled>>,
    log: String,
    deleted: bool,
}

#[derive(Clone)]
struct ULoc {
    name: String,
    ty: Ty,
    count: u32,
    is_array: bool,
    /// A uniform block member: (block index, std140 offset, array stride). No location.
    block: Option<(u32, u32, u32)>,
    /// (stage: 0 vertex / 1 fragment, first slot)
    slots: Vec<(u8, u32)>,
}

struct LBlock {
    name: String,
    size: u32,
    /// (stage, member slot list as in BlockVar)
    stages: Vec<(u8, usize)>,
    /// Member names (for getUniformIndices) — first stage's members.
    members: Vec<String>,
}

struct Linked {
    vs: Rc<Compiled>,
    fs: Rc<Compiled>,
    vs_mem: Vec<Lanes>,
    fs_mem: Vec<Lanes>,
    uniforms: Vec<ULoc>,
    /// (name, type, location)
    attribs: Vec<(String, Ty, u32, u32)>,
    vary: Vec<(u32, u32, u32, bool)>,
    /// draw buffer index → fragment slot
    outs: Vec<Option<u32>>,
    out_names: Vec<(String, u32)>,
    blocks: Vec<LBlock>,
    block_binding: Vec<u32>,
}

struct Program {
    shaders: Vec<u32>,
    linked: Option<Linked>,
    link_ok: bool,
    log: String,
    bound_attribs: HashMap<String, u32>,
    deleted: bool,
    validate: bool,
}

#[derive(Clone, Copy)]
struct VAttr {
    enabled: bool,
    buffer: u32,
    size: u32,
    ty: u32,
    norm: bool,
    stride: u32,
    offset: u32,
    divisor: u32,
    int: bool,
}

impl Default for VAttr {
    fn default() -> Self {
        VAttr { enabled: false, buffer: 0, size: 4, ty: e::FLOAT, norm: false, stride: 0, offset: 0, divisor: 0, int: false }
    }
}

#[derive(Clone)]
struct Vao {
    attribs: [VAttr; MAX_ATTRIBS],
    ebo: u32,
}

impl Default for Vao {
    fn default() -> Self {
        Vao { attribs: [VAttr::default(); MAX_ATTRIBS], ebo: 0 }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Att {
    Tex { id: u32, level: u32, face: u32, layer: u32 },
    Rbo(u32),
}

#[derive(Clone)]
struct Fbo {
    color: [Option<Att>; MAX_COLOR_ATTACHMENTS],
    depth: Option<Att>,
    stencil: Option<Att>,
    /// Attached with DEPTH_STENCIL_ATTACHMENT.
    depth_stencil: bool,
    draw_buffers: Vec<u32>,
    read_buffer: u32,
}

impl Default for Fbo {
    fn default() -> Self {
        let mut db = vec![0; MAX_COLOR_ATTACHMENTS];
        db[0] = COLOR_ATTACHMENT0;
        Fbo { color: [None; MAX_COLOR_ATTACHMENTS], depth: None, stencil: None, depth_stencil: false, draw_buffers: db, read_buffer: COLOR_ATTACHMENT0 }
    }
}

#[derive(Default)]
struct Rbo {
    img: Option<Image>,
    samples: u32,
    internal: u32,
}

enum Obj {
    Buffer(Buffer),
    Texture(Texture),
    Shader(Shader),
    Program(Program),
    Fbo(Fbo),
    Rbo(Rbo),
    Vao(Vao),
    Sampler(TexParams),
    Query(u32),
    Sync,
}

#[derive(Clone, Copy)]
struct Attrs {
    alpha: bool,
    depth: bool,
    stencil: bool,
    antialias: bool,
    premultiplied: bool,
    preserve: bool,
}

pub struct Gl {
    v2: bool,
    attrs: Attrs,
    w: u32,
    h: u32,
    err: u32,
    objs: HashMap<u32, Obj>,
    next: u32,
    // The default framebuffer.
    fb_color: Image,
    fb_depth: Option<Vec<f32>>,
    fb_stencil: Option<Vec<u8>>,
    fb_draw_buffer: u32,
    fb_read_buffer: u32,
    /// Presented and not preserved: clear before the next drawing operation.
    needs_clear: bool,
    /// Changed since last copied to the canvas.
    dirty: bool,
    // Bindings.
    array_buffer: u32,
    bound: HashMap<u32, u32>,
    ubo_bind: Vec<(u32, u32, u32)>,
    default_vao: Vao,
    vao: u32,
    units: Vec<[u32; 4]>,
    samplers: Vec<u32>,
    active: usize,
    program: u32,
    draw_fbo: u32,
    read_fbo: u32,
    rbo: u32,
    generic: [([f32; 4], u8); MAX_ATTRIBS],
    // State.
    clear_color: [f32; 4],
    clear_depth: f32,
    clear_stencil: i32,
    viewport: [i32; 4],
    scissor: [i32; 4],
    caps: HashMap<u32, bool>,
    blend_eq: [u32; 2],
    blend_src: [u32; 2],
    blend_dst: [u32; 2],
    blend_color: [f32; 4],
    color_mask: [bool; 4],
    depth_func: u32,
    depth_mask: bool,
    depth_range: [f32; 2],
    stencil: [StencilFace; 2],
    cull_face: u32,
    front_face: u32,
    line_width: f32,
    poly_offset: (f32, f32),
    sample_coverage: (f32, bool),
    pack_alignment: usize,
    unpack: Unpack,
    pack_row_length: usize,
    pack_skip_rows: usize,
    pack_skip_pixels: usize,
    colorspace: u32,
    hints: HashMap<u32, u32>,
    seed: u32,
    /// WEBGL_compressed_texture_s3tc (and _srgb) enabled.
    s3tc: bool,
}

fn fb_fmt(alpha: bool) -> Fmt {
    if alpha {
        tex::texture_format(e::RGBA, e::RGBA, e::UNSIGNED_BYTE, false).unwrap()
    } else {
        tex::texture_format(e::RGB, e::RGB, e::UNSIGNED_BYTE, false).unwrap()
    }
}

impl Gl {
    fn new(w: u32, h: u32, attrs: Attrs, v2: bool, seed: u32) -> Gl {
        let (w, h) = (w.min(MAX_TEXTURE_SIZE), h.min(MAX_TEXTURE_SIZE));
        let mut gl = Gl {
            v2,
            attrs,
            w,
            h,
            err: 0,
            objs: HashMap::new(),
            next: 1,
            fb_color: Image::new(w, h, 1, fb_fmt(attrs.alpha), v2),
            fb_depth: None,
            fb_stencil: None,
            fb_draw_buffer: BACK,
            fb_read_buffer: BACK,
            needs_clear: false,
            dirty: true,
            array_buffer: 0,
            bound: HashMap::new(),
            ubo_bind: vec![(0, 0, 0); MAX_UNIFORM_BINDINGS],
            default_vao: Vao::default(),
            vao: 0,
            units: vec![[0; 4]; MAX_UNITS],
            samplers: vec![0; MAX_UNITS],
            active: 0,
            program: 0,
            draw_fbo: 0,
            read_fbo: 0,
            rbo: 0,
            generic: [([0.0, 0.0, 0.0, 1.0], 0); MAX_ATTRIBS],
            clear_color: [0.0; 4],
            clear_depth: 1.0,
            clear_stencil: 0,
            viewport: [0, 0, w as i32, h as i32],
            scissor: [0, 0, w as i32, h as i32],
            caps: HashMap::from([(DITHER, true)]),
            blend_eq: [g::FUNC_ADD; 2],
            blend_src: [g::ONE; 2],
            blend_dst: [g::ZERO; 2],
            blend_color: [0.0; 4],
            color_mask: [true; 4],
            depth_func: e::LESS,
            depth_mask: true,
            depth_range: [0.0, 1.0],
            stencil: [StencilFace::default(); 2],
            cull_face: g::BACK,
            front_face: 0x901,
            line_width: 1.0,
            poly_offset: (0.0, 0.0),
            sample_coverage: (1.0, false),
            pack_alignment: 4,
            unpack: Unpack { alignment: 4, flip_y: false, premultiply: false, row_length: 0, skip_rows: 0, skip_pixels: 0, image_height: 0, skip_images: 0 },
            pack_row_length: 0,
            pack_skip_rows: 0,
            pack_skip_pixels: 0,
            colorspace: 0x9244,
            hints: HashMap::new(),
            seed,
            s3tc: false,
        };
        gl.alloc_default();
        gl
    }

    fn alloc_default(&mut self) {
        let n = (self.w * self.h) as usize;
        self.fb_color = Image::new(self.w, self.h, 1, fb_fmt(self.attrs.alpha), self.v2);
        self.fb_color.px.iter_mut().for_each(|p| *p = if self.attrs.alpha { [0.0; 4] } else { [0.0, 0.0, 0.0, 1.0] });
        self.fb_depth = self.attrs.depth.then(|| vec![1.0; n]);
        self.fb_stencil = self.attrs.stencil.then(|| vec![0; n]);
        self.dirty = true;
    }

    fn error(&mut self, code: u32) {
        if self.err == 0 {
            self.err = code;
        }
    }

    fn cap(&self, c: u32) -> bool {
        self.caps.get(&c).copied().unwrap_or(false)
    }

    fn new_id(&mut self, o: Obj) -> u32 {
        let id = self.next;
        self.next += 1;
        self.objs.insert(id, o);
        id
    }

    fn vao_ref(&self) -> &Vao {
        match self.objs.get(&self.vao) {
            Some(Obj::Vao(v)) if self.vao != 0 => v,
            _ => &self.default_vao,
        }
    }
    fn vao_mut(&mut self) -> &mut Vao {
        if self.vao != 0 {
            if let Some(Obj::Vao(v)) = self.objs.get_mut(&self.vao) {
                return v;
            }
        }
        &mut self.default_vao
    }

    fn buffer_target(&self, target: u32) -> Option<u32> {
        match target {
            ARRAY_BUFFER => Some(self.array_buffer),
            ELEMENT_ARRAY_BUFFER => Some(self.vao_ref().ebo),
            COPY_READ_BUFFER | COPY_WRITE_BUFFER | PIXEL_PACK_BUFFER | PIXEL_UNPACK_BUFFER | UNIFORM_BUFFER | TRANSFORM_FEEDBACK_BUFFER if self.v2 => {
                Some(self.bound.get(&target).copied().unwrap_or(0))
            }
            _ => None,
        }
    }

    fn buf(&self, id: u32) -> Option<&Buffer> {
        match self.objs.get(&id) {
            Some(Obj::Buffer(b)) => Some(b),
            _ => None,
        }
    }
    fn buf_mut(&mut self, id: u32) -> Option<&mut Buffer> {
        match self.objs.get_mut(&id) {
            Some(Obj::Buffer(b)) => Some(b),
            _ => None,
        }
    }
    fn texture(&self, id: u32) -> Option<&Texture> {
        match self.objs.get(&id) {
            Some(Obj::Texture(t)) => Some(t),
            _ => None,
        }
    }
    fn texture_mut(&mut self, id: u32) -> Option<&mut Texture> {
        match self.objs.get_mut(&id) {
            Some(Obj::Texture(t)) => Some(t),
            _ => None,
        }
    }
    fn fbo(&self, id: u32) -> Option<&Fbo> {
        match self.objs.get(&id) {
            Some(Obj::Fbo(f)) => Some(f),
            _ => None,
        }
    }
    fn fbo_mut(&mut self, id: u32) -> Option<&mut Fbo> {
        match self.objs.get_mut(&id) {
            Some(Obj::Fbo(f)) => Some(f),
            _ => None,
        }
    }
    fn program_ref(&self, id: u32) -> Option<&Program> {
        match self.objs.get(&id) {
            Some(Obj::Program(p)) => Some(p),
            _ => None,
        }
    }
    fn program_mut(&mut self, id: u32) -> Option<&mut Program> {
        match self.objs.get_mut(&id) {
            Some(Obj::Program(p)) => Some(p),
            _ => None,
        }
    }

    /// Clear lazily after an unpreserved frame was shown.
    fn before_draw(&mut self) {
        if self.needs_clear {
            self.needs_clear = false;
            self.fb_color.px.iter_mut().for_each(|p| *p = if self.attrs.alpha { [0.0; 4] } else { [0.0, 0.0, 0.0, 1.0] });
            if let Some(d) = self.fb_depth.as_mut() {
                d.iter_mut().for_each(|x| *x = 1.0);
            }
            if let Some(s) = self.fb_stencil.as_mut() {
                s.iter_mut().for_each(|x| *x = 0);
            }
        }
    }

    // ---- textures

    /// The unit slot (0 2D, 1 3D, 2 cube, 3 2D array) for a texture target.
    fn tex_slot(&self, target: u32) -> Option<usize> {
        match target {
            e::TEXTURE_2D => Some(0),
            e::TEXTURE_3D if self.v2 => Some(1),
            e::TEXTURE_CUBE_MAP => Some(2),
            e::TEXTURE_2D_ARRAY if self.v2 => Some(3),
            _ => None,
        }
    }

    /// The bound texture and face index for a texImage target.
    fn image_target(&mut self, target: u32) -> Option<(u32, usize)> {
        let (slot, face) = match target {
            e::TEXTURE_2D => (0, 0),
            t if (e::TEXTURE_CUBE_MAP_POSITIVE_X..e::TEXTURE_CUBE_MAP_POSITIVE_X + 6).contains(&t) => (2, (t - e::TEXTURE_CUBE_MAP_POSITIVE_X) as usize),
            e::TEXTURE_3D if self.v2 => (1, 0),
            e::TEXTURE_2D_ARRAY if self.v2 => (3, 0),
            _ => {
                self.error(INVALID_ENUM);
                return None;
            }
        };
        let id = self.units[self.active][slot];
        if id == 0 {
            self.error(INVALID_OPERATION);
            return None;
        }
        Some((id, face))
    }

    // ---- framebuffers

    fn att_image(&self, a: &Att) -> Option<&Image> {
        match a {
            Att::Tex { id, level, face, .. } => self.texture(*id)?.level(*face as usize, *level as usize),
            Att::Rbo(id) => match self.objs.get(id) {
                Some(Obj::Rbo(r)) => r.img.as_ref(),
                _ => None,
            },
        }
    }

    fn fbo_status(&self, id: u32) -> u32 {
        let Some(f) = self.fbo(id) else { return FRAMEBUFFER_COMPLETE };
        let mut size: Option<(u32, u32)> = None;
        let mut any = false;
        let mut check = |img: Option<&Image>, want: u8| -> Option<u32> {
            let Some(img) = img else { return Some(FRAMEBUFFER_INCOMPLETE_ATTACHMENT) };
            if img.w == 0 || img.h == 0 {
                return Some(FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
            }
            let ok = match want {
                0 => img.fmt.color_renderable(self.v2),
                1 => img.fmt.depth,
                _ => img.fmt.stencil,
            };
            if !ok {
                return Some(FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
            }
            match size {
                None => size = Some((img.w, img.h)),
                Some(s) if s != (img.w, img.h) && !self.v2 => return Some(FRAMEBUFFER_INCOMPLETE_DIMENSIONS),
                _ => {}
            }
            any = true;
            None
        };
        for a in f.color.iter().flatten() {
            if let Some(s) = check(self.att_image(a), 0) {
                return s;
            }
        }
        if let Some(a) = &f.depth {
            if let Some(s) = check(self.att_image(a), 1) {
                return s;
            }
        }
        if let Some(a) = &f.stencil {
            if let Some(s) = check(self.att_image(a), 2) {
                return s;
            }
        }
        if !any {
            return FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT;
        }
        if !self.v2 && f.depth.is_some() && f.stencil.is_some() && f.depth != f.stencil {
            return FRAMEBUFFER_UNSUPPORTED;
        }
        if self.v2 && f.depth.is_some() && f.stencil.is_some() && f.depth != f.stencil {
            return FRAMEBUFFER_UNSUPPORTED;
        }
        FRAMEBUFFER_COMPLETE
    }

    /// Move the draw (or read) framebuffer's images out for rendering.
    fn take_target(&mut self, read: bool) -> Option<(Target, Vec<Option<Att>>, Option<Att>, Option<Att>)> {
        let fid = if read { self.read_fbo } else { self.draw_fbo };
        if fid == 0 {
            let w = self.w;
            let h = self.h;
            let buf = if read { self.fb_read_buffer } else { self.fb_draw_buffer };
            let color = if buf == BACK { Some(std::mem::replace(&mut self.fb_color, Image::new(0, 0, 1, fb_fmt(true), self.v2))) } else { None };
            let t = Target { w, h, color: vec![color], depth: self.fb_depth.take(), stencil: self.fb_stencil.take() };
            return Some((t, vec![None], None, None));
        }
        if self.fbo_status(fid) != FRAMEBUFFER_COMPLETE {
            self.error(INVALID_FRAMEBUFFER_OPERATION);
            return None;
        }
        let f = self.fbo(fid)?.clone();
        let (mut w, mut h) = (u32::MAX, u32::MAX);
        for a in f.color.iter().flatten().chain(f.depth.iter()).chain(f.stencil.iter()) {
            if let Some(i) = self.att_image(a) {
                w = w.min(i.w);
                h = h.min(i.h);
            }
        }
        let mut colors = Vec::new();
        let mut restore = Vec::new();
        let n = if read { 1 } else { f.draw_buffers.len() };
        for i in 0..n {
            let buf = if read { f.read_buffer } else { f.draw_buffers[i] };
            let att = if buf >= COLOR_ATTACHMENT0 && buf < COLOR_ATTACHMENT0 + MAX_COLOR_ATTACHMENTS as u32 { f.color[(buf - COLOR_ATTACHMENT0) as usize] } else { None };
            // The same attachment twice can't be taken twice; later copies are skipped.
            let dup = att.is_some() && restore.contains(&att);
            match (att, dup) {
                (Some(a), false) => {
                    colors.push(self.take_image(&a));
                    restore.push(Some(a));
                }
                _ => {
                    colors.push(None);
                    restore.push(None);
                }
            }
        }
        let depth = f.depth.and_then(|a| self.att_image(&a).map(|i| i.px.iter().map(|p| p[0]).collect::<Vec<f32>>()));
        let stencil = f.stencil.and_then(|a| self.att_image(&a).map(|i| if i.st.is_empty() { vec![0; i.px.len()] } else { i.st.clone() }));
        let t = Target { w, h, color: colors, depth: depth.map(|d| crop_f(&d, self.att_w(&f.depth), w, h)), stencil: stencil.map(|s| crop_u8(&s, self.att_w(&f.stencil), w, h)) };
        Some((t, restore, f.depth, f.stencil))
    }

    fn att_w(&self, a: &Option<Att>) -> u32 {
        a.as_ref().and_then(|a| self.att_image(a)).map_or(0, |i| i.w)
    }

    fn take_image(&mut self, a: &Att) -> Option<Image> {
        match a {
            Att::Tex { id, level, face, layer } => {
                let t = self.texture_mut(*id)?;
                let img = t.faces.get_mut(*face as usize)?.get_mut(*level as usize)?.as_mut()?;
                if img.d > 1 {
                    // One layer of a 3D or array texture.
                    let n = (img.w * img.h) as usize;
                    let start = n * (*layer).min(img.d - 1) as usize;
                    return Some(Image { w: img.w, h: img.h, d: 1, fmt: img.fmt, px: img.px[start..start + n].to_vec(), st: Vec::new() });
                }
                Some(std::mem::replace(img, Image { w: 0, h: 0, d: 1, fmt: img.fmt, px: Vec::new(), st: Vec::new() }))
            }
            Att::Rbo(id) => match self.objs.get_mut(id) {
                Some(Obj::Rbo(r)) => r.img.take(),
                _ => None,
            },
        }
    }

    fn put_image(&mut self, a: &Att, img: Image) {
        match a {
            Att::Tex { id, level, face, layer } => {
                if let Some(t) = self.texture_mut(*id) {
                    if let Some(Some(slot)) = t.faces.get_mut(*face as usize).and_then(|f| f.get_mut(*level as usize)) {
                        if slot.d > 1 {
                            let n = (slot.w * slot.h) as usize;
                            let start = n * (*layer).min(slot.d - 1) as usize;
                            slot.px[start..start + n].copy_from_slice(&img.px[..n]);
                        } else {
                            *slot = img;
                        }
                    }
                }
            }
            Att::Rbo(id) => {
                if let Some(Obj::Rbo(r)) = self.objs.get_mut(id) {
                    r.img = Some(img);
                }
            }
        }
    }

    fn put_target(&mut self, t: Target, restore: Vec<Option<Att>>, depth: Option<Att>, stencil: Option<Att>, read: bool) {
        let fid = if read { self.read_fbo } else { self.draw_fbo };
        if fid == 0 {
            let mut c = t.color;
            if let Some(Some(img)) = c.pop() {
                self.fb_color = img;
            }
            self.fb_depth = t.depth;
            self.fb_stencil = t.stencil;
            self.dirty = true;
            return;
        }
        for (img, a) in t.color.into_iter().zip(restore) {
            if let (Some(img), Some(a)) = (img, a) {
                self.put_image(&a, img);
            }
        }
        let v2 = self.v2;
        if let (Some(d), Some(a)) = (t.depth, depth) {
            let w = t.w;
            if let Some(img) = self.att_image_mut(&a) {
                for y in 0..t.h.min(img.h) {
                    for x in 0..w.min(img.w) {
                        let v = d[(y * w + x) as usize];
                        img.px[(y * img.w + x) as usize] = if v2 { [v, 0.0, 0.0, 1.0] } else { [v, v, v, 1.0] };
                    }
                }
            }
        }
        if let (Some(s), Some(a)) = (t.stencil, stencil) {
            let w = t.w;
            if let Some(img) = self.att_image_mut(&a) {
                if img.st.len() == img.px.len() {
                    for y in 0..t.h.min(img.h) {
                        for x in 0..w.min(img.w) {
                            img.st[(y * img.w + x) as usize] = s[(y * w + x) as usize];
                        }
                    }
                }
            }
        }
    }

    fn att_image_mut(&mut self, a: &Att) -> Option<&mut Image> {
        match a {
            Att::Tex { id, level, face, .. } => self.texture_mut(*id)?.faces.get_mut(*face as usize)?.get_mut(*level as usize)?.as_mut(),
            Att::Rbo(id) => match self.objs.get_mut(id) {
                Some(Obj::Rbo(r)) => r.img.as_mut(),
                _ => None,
            },
        }
    }

    fn rstate(&self) -> RState {
        let stencil_on = self.cap(STENCIL_TEST);
        RState {
            viewport: self.viewport,
            depth_range: self.depth_range,
            scissor: self.cap(SCISSOR_TEST).then_some(self.scissor),
            cull: self.cap(CULL_FACE).then_some(self.cull_face),
            front_ccw: self.front_face == 0x901,
            depth_test: self.cap(DEPTH_TEST),
            depth_func: self.depth_func,
            depth_mask: self.depth_mask,
            stencil: stencil_on.then_some(self.stencil),
            blend: self.cap(BLEND),
            blend_eq: self.blend_eq,
            blend_src: self.blend_src,
            blend_dst: self.blend_dst,
            blend_color: self.blend_color,
            color_mask: self.color_mask,
            poly_offset: self.cap(POLYGON_OFFSET_FILL).then_some(self.poly_offset),
            discard_raster: self.cap(RASTERIZER_DISCARD),
        }
    }

    // ---- clear

    fn clear(&mut self, mask: u32) {
        if mask & !(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            return self.error(INVALID_VALUE);
        }
        self.before_draw();
        if self.cap(RASTERIZER_DISCARD) {
            return;
        }
        let Some((mut t, restore, d, s)) = self.take_target(false) else { return };
        let st = self.rstate();
        let [x0, y0, x1, y1] = clip_rect(&st, t.w, t.h);
        if mask & COLOR_BUFFER_BIT != 0 {
            let cc = self.clear_color;
            for img in t.color.iter_mut().flatten() {
                if img.fmt.is_integer() {
                    continue;
                }
                let v = tex::store(&img.fmt, cc);
                fill_rect(img, [x0, y0, x1, y1], |dst| {
                    let mut o = *dst;
                    for c in 0..4 {
                        if st.color_mask[c] {
                            o[c] = v[c];
                        }
                    }
                    o
                });
            }
        }
        if mask & DEPTH_BUFFER_BIT != 0 && self.depth_mask {
            if let Some(db) = t.depth.as_mut() {
                let v = self.clear_depth.clamp(0.0, 1.0);
                for y in y0..y1 {
                    for x in x0..x1 {
                        db[(y * t.w as i32 + x) as usize] = v;
                    }
                }
            }
        }
        if mask & STENCIL_BUFFER_BIT != 0 {
            if let Some(sb) = t.stencil.as_mut() {
                let m = self.stencil[0].writemask as u8;
                let v = self.clear_stencil as u8;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = &mut sb[(y * t.w as i32 + x) as usize];
                        *p = (*p & !m) | (v & m);
                    }
                }
            }
        }
        self.put_target(t, restore, d, s, false);
        self.dirty = true;
    }

    /// clearBuffer{fv,iv,uiv,fi} (WebGL 2).
    fn clear_buffer(&mut self, buffer: u32, index: i32, vals: &[f32], kind: u8) {
        self.before_draw();
        let Some((mut t, restore, d, s)) = self.take_target(false) else { return };
        let st = self.rstate();
        let r = clip_rect(&st, t.w, t.h);
        match buffer {
            COLOR => {
                if let Some(Some(img)) = t.color.get_mut(index.max(0) as usize) {
                    let int = img.fmt.is_integer();
                    if (kind == 0) == int {
                        self.error(INVALID_OPERATION);
                    } else {
                        let v = [vals[0], vals[1], vals[2], vals[3]];
                        let v = if int { tex::expand(&img.fmt, v) } else { tex::store(&img.fmt, v) };
                        fill_rect(img, r, |dst| {
                            let mut o = *dst;
                            for c in 0..4 {
                                if st.color_mask[c] {
                                    o[c] = v[c];
                                }
                            }
                            o
                        });
                    }
                }
            }
            DEPTH | DEPTH_STENCIL => {
                if self.depth_mask {
                    if let Some(db) = t.depth.as_mut() {
                        for y in r[1]..r[3] {
                            for x in r[0]..r[2] {
                                db[(y * t.w as i32 + x) as usize] = vals[0].clamp(0.0, 1.0);
                            }
                        }
                    }
                }
                if buffer == DEPTH_STENCIL {
                    if let Some(sb) = t.stencil.as_mut() {
                        let v = vals[1] as i32 as u8;
                        let m = self.stencil[0].writemask as u8;
                        for y in r[1]..r[3] {
                            for x in r[0]..r[2] {
                                let p = &mut sb[(y * t.w as i32 + x) as usize];
                                *p = (*p & !m) | (v & m);
                            }
                        }
                    }
                }
            }
            STENCIL => {
                if let Some(sb) = t.stencil.as_mut() {
                    let v = vals[0].to_bits() as i32 as u8;
                    let m = self.stencil[0].writemask as u8;
                    for y in r[1]..r[3] {
                        for x in r[0]..r[2] {
                            let p = &mut sb[(y * t.w as i32 + x) as usize];
                            *p = (*p & !m) | (v & m);
                        }
                    }
                }
            }
            _ => self.error(INVALID_ENUM),
        }
        self.put_target(t, restore, d, s, false);
        self.dirty = true;
    }

    // ---- programs

    fn link(&mut self, pid: u32) {
        let v2 = self.v2;
        let (shaders, bound) = match self.program_ref(pid) {
            Some(p) => (p.shaders.clone(), p.bound_attribs.clone()),
            None => return,
        };
        let mut vs = None;
        let mut fs = None;
        for s in shaders {
            if let Some(Obj::Shader(sh)) = self.objs.get(&s) {
                match sh.ty {
                    VERTEX_SHADER => vs = sh.compiled.clone(),
                    _ => fs = sh.compiled.clone(),
                }
            }
        }
        let result = (|| -> Result<Linked, String> {
            let vs = vs.ok_or("no compiled vertex shader is attached")?;
            let fs = fs.ok_or("no compiled fragment shader is attached")?;
            if vs.version != fs.version {
                return Err("the shaders' GLSL versions differ".into());
            }
            // Varyings, by name.
            let mut vary = Vec::new();
            for i in &fs.inputs {
                match vs.outputs.iter().find(|o| o.name == i.name) {
                    Some(o) => {
                        if o.ty != i.ty {
                            return Err(format!("varying '{}' has different types in the two shaders", i.name));
                        }
                        vary.push((o.slot, i.slot, i.ty.size(), i.flat || o.flat));
                    }
                    None => {
                        if fs.version == 300 {
                            return Err(format!("fragment input '{}' isn't written by the vertex shader", i.name));
                        }
                    }
                }
            }
            if vary.iter().map(|v| v.2).sum::<u32>() > 15 * 4 + 4 {
                return Err("too many varyings".into());
            }
            // Uniforms, merged.
            let mut uniforms: Vec<ULoc> = Vec::new();
            for (stage, c) in [(0u8, &vs), (1u8, &fs)] {
                for u in &c.uniforms {
                    match uniforms.iter_mut().find(|x| x.name == u.name) {
                        Some(x) => {
                            if x.ty != u.ty || x.count != u.count {
                                return Err(format!("uniform '{}' has different types in the two shaders", u.name));
                            }
                            x.slots.push((stage, u.slot));
                        }
                        None => uniforms.push(ULoc { name: u.name.clone(), ty: u.ty.clone(), count: u.count, is_array: u.is_array, block: None, slots: vec![(stage, u.slot)] }),
                    }
                }
            }
            // Attribute locations.
            let mut used = [false; MAX_ATTRIBS];
            let mut attribs: Vec<(String, Ty, u32, u32)> = Vec::new();
            let cols = |t: &Ty| match t {
                Ty::Mat(c, _) => *c as u32,
                _ => 1,
            };
            let mut pending = Vec::new();
            for a in &vs.inputs {
                let loc = bound.get(&a.name).copied().or(a.location.map(|l| l as u32));
                match loc {
                    Some(l) => {
                        let n = cols(&a.ty);
                        if (l + n) as usize > MAX_ATTRIBS {
                            return Err(format!("attribute '{}' location is out of range", a.name));
                        }
                        for k in l..l + n {
                            used[k as usize] = true;
                        }
                        attribs.push((a.name.clone(), a.ty.clone(), l, a.slot));
                    }
                    None => pending.push(a),
                }
            }
            for a in pending {
                let n = cols(&a.ty) as usize;
                let Some(l) = (0..=MAX_ATTRIBS - n).find(|&l| (l..l + n).all(|k| !used[k])) else { return Err("too many attributes".into()) };
                for k in l..l + n {
                    used[k] = true;
                }
                attribs.push((a.name.clone(), a.ty.clone(), l as u32, a.slot));
            }
            // Fragment outputs.
            let mut outs = vec![None; MAX_COLOR_ATTACHMENTS];
            let mut out_names = Vec::new();
            if let Some(s) = fs.b.frag_color {
                for o in outs.iter_mut() {
                    *o = Some(s);
                }
            } else if let Some((s, n)) = fs.b.frag_data {
                for i in 0..n as usize {
                    outs[i] = Some(s + i as u32 * 4);
                }
            } else {
                let single = fs.outputs.len() == 1;
                for o in &fs.outputs {
                    let loc = o.location.map(|l| l as usize).or(if single { Some(0) } else { None });
                    let Some(loc) = loc else { return Err(format!("output '{}' needs a layout(location)", o.name)) };
                    let (n, sz) = match &o.ty {
                        Ty::Array(el, n) => (*n as usize, el.size()),
                        t => (1, t.size()),
                    };
                    for k in 0..n {
                        if loc + k < MAX_COLOR_ATTACHMENTS {
                            outs[loc + k] = Some(o.slot + (k as u32) * sz);
                        }
                    }
                    out_names.push((o.name.clone(), loc as u32));
                }
            }
            // Uniform blocks.
            let mut blocks: Vec<LBlock> = Vec::new();
            for (stage, c) in [(0u8, &vs), (1u8, &fs)] {
                for (bi, b) in c.blocks.iter().enumerate() {
                    match blocks.iter_mut().find(|x| x.name == b.name) {
                        Some(x) => x.stages.push((stage, bi)),
                        None => blocks.push(LBlock { name: b.name.clone(), size: b.size, stages: vec![(stage, bi)], members: b.members.iter().map(|m| m.0.clone()).collect() }),
                    }
                }
            }
            let _ = v2;
            // Block members are active uniforms too (without locations).
            for (bi, b) in blocks.iter().enumerate() {
                let (stage, idx) = b.stages[0];
                let c = if stage == 0 { &vs } else { &fs };
                for (name, el, _, count, off, stride) in &c.blocks[idx].members {
                    let arr = *count > 1;
                    let name = if arr { format!("{name}[0]") } else { name.clone() };
                    uniforms.push(ULoc { name, ty: el.clone(), count: *count, is_array: arr, block: Some((bi as u32, *off, if arr { *stride } else { 0 })), slots: Vec::new() });
                }
            }
            let nb = blocks.len();
            Ok(Linked { vs_mem: vs.mem.clone(), fs_mem: fs.mem.clone(), vs, fs, uniforms, attribs, vary, outs, out_names, blocks, block_binding: vec![0; nb] })
        })();
        if let Some(p) = self.program_mut(pid) {
            match result {
                Ok(l) => {
                    p.linked = Some(l);
                    p.link_ok = true;
                    p.log.clear();
                }
                Err(m) => {
                    p.linked = None;
                    p.link_ok = false;
                    p.log = m;
                }
            }
        }
    }

    // ---- drawing

    fn draw(&mut self, mode: u32, first: i32, count: i32, index: Option<(u32, u32)>, instances: i32) {
        if mode > g::TRIANGLE_FAN {
            return self.error(INVALID_ENUM);
        }
        if first < 0 || count < 0 || instances < 0 {
            return self.error(INVALID_VALUE);
        }
        let Some(prog) = self.program_ref(self.program) else { return self.error(INVALID_OPERATION) };
        let Some(l) = prog.linked.as_ref() else { return self.error(INVALID_OPERATION) };
        let attribs_needed: Vec<(u32, Ty, u32)> = l.attribs.iter().map(|a| (a.2, a.1.clone(), a.3)).collect();
        if count == 0 || instances == 0 {
            return;
        }
        // Vertex ids.
        let ids: Vec<u32> = match index {
            None => (first as u32..(first + count) as u32).collect(),
            Some((ty, offset)) => {
                let ebo = self.vao_ref().ebo;
                let Some(b) = self.buf(ebo) else { return self.error(INVALID_OPERATION) };
                let sz = match ty {
                    e::UNSIGNED_BYTE => 1,
                    e::UNSIGNED_SHORT => 2,
                    e::UNSIGNED_INT => 4,
                    _ => return self.error(INVALID_ENUM),
                };
                if offset % sz != 0 {
                    return self.error(INVALID_OPERATION);
                }
                let end = offset as usize + count as usize * sz as usize;
                if end > b.data.len() {
                    return self.error(INVALID_OPERATION);
                }
                let d = &b.data[offset as usize..end];
                let mut v: Vec<u32> = match sz {
                    1 => d.iter().map(|&x| x as u32).collect(),
                    2 => d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect(),
                    _ => d.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
                };
                if self.v2 {
                    // Primitive restart is always on in WebGL 2: the all-ones index breaks the strip.
                    let restart = if sz == 1 { 0xFF } else if sz == 2 { 0xFFFF } else { u32::MAX };
                    if v.contains(&restart) {
                        return self.draw_restart(mode, &v, restart, instances);
                    }
                }
                v.shrink_to_fit();
                v
            }
        };
        self.draw_ids(mode, &ids, instances as u32, &attribs_needed);
    }

    fn draw_restart(&mut self, mode: u32, v: &[u32], restart: u32, instances: i32) {
        let attribs_needed: Vec<(u32, Ty, u32)> = match self.program_ref(self.program).and_then(|p| p.linked.as_ref()) {
            Some(l) => l.attribs.iter().map(|a| (a.2, a.1.clone(), a.3)).collect(),
            None => return,
        };
        for part in v.split(|&x| x == restart) {
            if !part.is_empty() {
                self.draw_ids(mode, part, instances as u32, &attribs_needed);
            }
        }
    }

    fn draw_ids(&mut self, mode: u32, ids: &[u32], instances: u32, attribs_needed: &[(u32, Ty, u32)]) {
        // Check the attribute buffers cover every vertex read.
        let max_id = ids.iter().copied().max().unwrap_or(0);
        {
            let vao = self.vao_ref().clone();
            for (loc, ty, _) in attribs_needed {
                let n = match ty {
                    Ty::Mat(c, _) => *c as u32,
                    _ => 1,
                };
                for k in 0..n {
                    let a = vao.attribs[(*loc + k) as usize];
                    if !a.enabled {
                        continue;
                    }
                    let Some(b) = self.buf(a.buffer) else { return self.error(INVALID_OPERATION) };
                    let tsz = type_size(a.ty);
                    let packed = matches!(a.ty, 0x8D9F | 0x8368);
                    let elem = if packed { 4 } else { a.size * tsz };
                    let stride = if a.stride == 0 { elem } else { a.stride } as u64;
                    let last = if a.divisor > 0 { (instances.max(1) - 1) / a.divisor } else { max_id } as u64;
                    if a.offset as u64 + last * stride + elem as u64 > b.data.len() as u64 {
                        return self.error(INVALID_OPERATION);
                    }
                }
            }
        }
        self.before_draw();
        let Some((mut target, restore, dep, sten)) = self.take_target(false) else { return };
        let st = self.rstate();
        {
            let gl = &*self;
            let p = gl.program_ref(gl.program).unwrap();
            let l = p.linked.as_ref().unwrap();
            // Textures for each unit.
            let mut units: Vec<[Option<UnitView>; 4]> = Vec::with_capacity(MAX_UNITS);
            for u in 0..MAX_UNITS {
                let samp = gl.samplers[u];
                let sp = match gl.objs.get(&samp) {
                    Some(Obj::Sampler(sp)) if samp != 0 => Some(sp),
                    _ => None,
                };
                let mut row: [Option<UnitView>; 4] = [None, None, None, None];
                for (k, slot) in row.iter_mut().enumerate() {
                    let id = gl.units[u][k];
                    if let Some(t) = gl.texture(id) {
                        let params = sp.unwrap_or(&t.p);
                        let mut params_full = params;
                        // Base/max level always come from the texture.
                        if sp.is_some() {
                            params_full = params;
                        }
                        *slot = Some(UnitView { tex: t, p: params_full, complete: t.complete(&TexParams { base: t.p.base, max: t.p.max, ..params_full.clone() }, gl.v2) });
                    }
                }
                units.push(row);
            }
            let sampler = Sampler { units, webgl2: gl.v2 };
            // Uniform block data.
            let mut vs_mem = l.vs_mem.clone();
            let mut fs_mem = l.fs_mem.clone();
            for (bi, b) in l.blocks.iter().enumerate() {
                let (buf, off, size) = gl.ubo_bind.get(l.block_binding[bi] as usize).copied().unwrap_or((0, 0, 0));
                let Some(data) = gl.buf(buf).map(|b| &b.data) else { continue };
                let start = (off as usize).min(data.len());
                let end = if size == 0 { data.len() } else { (start + size as usize).min(data.len()) };
                let bytes = &data[start..end];
                let _ = b.size;
                for (stage, idx) in &b.stages {
                    let (c, mem) = if *stage == 0 { (&l.vs, &mut vs_mem) } else { (&l.fs, &mut fs_mem) };
                    for (_, el, slot, cnt, off, stride) in &c.blocks[*idx].members {
                        read_std140(el, bytes, *off as usize, *stride as usize, *cnt, mem, *slot);
                    }
                }
            }
            // Attributes.
            let vao = gl.vao_ref();
            let mut attrs = Vec::new();
            for (loc, ty, slot) in attribs_needed {
                let (n, rows) = match ty {
                    Ty::Mat(c, r) => (*c as u32, *r as u32),
                    t => (1, t.size()),
                };
                let int_attr = matches!(ty.base(), Some(Base::I) | Some(Base::U));
                let mut cols = Vec::new();
                for k in 0..n {
                    let li = (*loc + k) as usize;
                    let a = vao.attribs[li];
                    if a.enabled {
                        let data = gl.buf(a.buffer).map(|b| b.data.as_slice()).unwrap_or(&[]);
                        cols.push(AttrSrc::Buf { data, ty: a.ty, size: a.size, norm: a.norm, stride: a.stride as usize, offset: a.offset as usize, divisor: a.divisor, int: a.int });
                    } else {
                        let (v, kind) = gl.generic[li];
                        // Generic values are kept as floats or, for vertexAttribI, as bits.
                        let v = match (int_attr, kind) {
                            (true, 0) => v.map(|x| f32::from_bits(x as i32 as u32)),
                            (false, 1) => v.map(|x| x.to_bits() as i32 as f32),
                            (false, 2) => v.map(|x| x.to_bits() as f32),
                            _ => v,
                        };
                        cols.push(AttrSrc::Const(v));
                    }
                }
                attrs.push(Attr { slot: *slot, rows, cols, int_attr });
            }
            // Which draw buffer gets which output.
            let outs: Vec<Option<u32>> = (0..target.color.len()).map(|i| l.outs.get(i).copied().flatten()).collect();
            let mut pipe = Pipeline {
                vs_code: &l.vs.code,
                vs_mem,
                fs_code: &l.fs.code,
                fs_mem,
                attrs,
                vary: l.vary.clone(),
                pos: l.vs.b.position.unwrap_or(0),
                psize: l.vs.b.point_size,
                vertex_id: l.vs.b.vertex_id,
                instance_id: l.vs.b.instance_id,
                frag_coord: l.fs.b.frag_coord,
                front_facing: l.fs.b.front_facing,
                point_coord: l.fs.b.point_coord,
                frag_depth: l.fs.b.frag_depth,
                outs,
                discards: l.fs.uses_discard,
                tex: &sampler,
                st: &st,
            };
            pipe.draw(&mut target, mode, ids, instances);
        }
        self.put_target(target, restore, dep, sten, false);
        self.dirty = true;
    }

    // ---- reading pixels

    /// Pixels of the read framebuffer's color buffer, bottom row first.
    fn read_rect(&mut self, x: i32, y: i32, w: i32, h: i32) -> Option<(Vec<Option<[f32; 4]>>, Fmt)> {
        self.before_draw();
        let fid = self.read_fbo;
        let (img, fw, fh) = if fid == 0 {
            if self.fb_read_buffer == 0 {
                self.error(INVALID_OPERATION);
                return None;
            }
            (&self.fb_color, self.w, self.h)
        } else {
            if self.fbo_status(fid) != FRAMEBUFFER_COMPLETE {
                self.error(INVALID_FRAMEBUFFER_OPERATION);
                return None;
            }
            let f = self.fbo(fid)?;
            let rb = f.read_buffer;
            if rb == 0 {
                self.error(INVALID_OPERATION);
                return None;
            }
            let Some(a) = f.color[(rb - COLOR_ATTACHMENT0) as usize] else {
                self.error(INVALID_OPERATION);
                return None;
            };
            let img = self.att_image(&a)?;
            (img, img.w, img.h)
        };
        let mut out = Vec::with_capacity((w.max(0) * h.max(0)) as usize);
        for j in 0..h {
            for i in 0..w {
                let (px, py) = (x + i, y + j);
                if px < 0 || py < 0 || px >= fw as i32 || py >= fh as i32 {
                    out.push(None);
                } else {
                    out.push(Some(img.px[(py as u32 * img.w + px as u32) as usize]));
                }
            }
        }
        Some((out, img.fmt))
    }

    /// The default framebuffer as premultiplied RGBA8, top row first (for the canvas).
    fn frame_rgba(&self) -> Vec<u8> {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut out = vec![0u8; w * h * 4];
        if self.fb_color.px.len() < w * h {
            return out;
        }
        for y in 0..h {
            let src_row = h - 1 - y;
            for x in 0..w {
                let p = self.fb_color.px[src_row * w + x];
                let a = if self.attrs.alpha { p[3].clamp(0.0, 1.0) } else { 1.0 };
                let (mut r, mut gg, mut b) = (p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0));
                if !self.attrs.premultiplied || !self.attrs.alpha {
                    r *= a;
                    gg *= a;
                    b *= a;
                } else {
                    r = r.min(a);
                    gg = gg.min(a);
                    b = b.min(a);
                }
                let o = (y * w + x) * 4;
                out[o] = (r * 255.0).round() as u8;
                out[o + 1] = (gg * 255.0).round() as u8;
                out[o + 2] = (b * 255.0).round() as u8;
                out[o + 3] = (a * 255.0).round() as u8;
            }
        }
        out
    }
}

/// Is `ty` a pixel type texImage accepts in this WebGL version?
fn valid_pixel_type(ty: u32, v2: bool) -> bool {
    if v2 {
        matches!(ty, e::BYTE | e::UNSIGNED_BYTE | e::SHORT | e::UNSIGNED_SHORT | e::INT | e::UNSIGNED_INT | e::HALF_FLOAT | e::FLOAT | 0x8363 | 0x8033 | 0x8034 | 0x84FA | 0x8368 | 0x8C3B | 0x8C3E | 0x8DAD)
    } else {
        matches!(ty, e::UNSIGNED_BYTE | e::UNSIGNED_SHORT | e::UNSIGNED_INT | e::FLOAT | e::HALF_FLOAT_OES | 0x8363 | 0x8033 | 0x8034 | 0x84FA)
    }
}

/// Check texture dimensions against the limits for `target` at `level`.
fn size_ok(target: u32, level: i32, w: i32, h: i32, d: i32) -> bool {
    let max2 = MAX_TEXTURE_SIZE as i32;
    let max3 = 2048;
    let (mw, md) = match target {
        e::TEXTURE_3D => (max3, max3),
        e::TEXTURE_2D_ARRAY => (max2, 2048),
        _ => (max2, 1),
    };
    let levels = 31 - (mw as u32).leading_zeros() as i32;
    if level < 0 || level > levels {
        return false;
    }
    let lw = mw >> level;
    let ld = if target == e::TEXTURE_3D { md >> level } else { md };
    w >= 0 && h >= 0 && d >= 0 && w <= lw && h <= lw && d <= ld.max(1)
}

fn type_size(ty: u32) -> u32 {
    match ty {
        e::BYTE | e::UNSIGNED_BYTE => 1,
        e::SHORT | e::UNSIGNED_SHORT | e::HALF_FLOAT | e::HALF_FLOAT_OES => 2,
        _ => 4,
    }
}

fn crop_f(v: &[f32], src_w: u32, w: u32, h: u32) -> Vec<f32> {
    if src_w == w {
        return v[..(w * h) as usize].to_vec();
    }
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        out.extend_from_slice(&v[(y * src_w) as usize..(y * src_w + w) as usize]);
    }
    out
}

fn crop_u8(v: &[u8], src_w: u32, w: u32, h: u32) -> Vec<u8> {
    if src_w == w {
        return v[..(w * h) as usize].to_vec();
    }
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        out.extend_from_slice(&v[(y * src_w) as usize..(y * src_w + w) as usize]);
    }
    out
}

fn clip_rect(st: &RState, w: u32, h: u32) -> [i32; 4] {
    let mut r = [0, 0, w as i32, h as i32];
    if let Some([x, y, sw, sh]) = st.scissor {
        r = [r[0].max(x), r[1].max(y), r[2].min(x + sw), r[3].min(y + sh)];
    }
    if r[2] < r[0] {
        r[2] = r[0];
    }
    if r[3] < r[1] {
        r[3] = r[1];
    }
    r
}

fn fill_rect(img: &mut Image, r: [i32; 4], f: impl Fn(&[f32; 4]) -> [f32; 4]) {
    for y in r[1]..r[3].min(img.h as i32) {
        for x in r[0]..r[2].min(img.w as i32) {
            let i = (y as u32 * img.w + x as u32) as usize;
            img.px[i] = f(&img.px[i]);
        }
    }
}

// ---------------------------------------------------------------- the page's store

#[derive(Default)]
pub struct Store {
    ctxs: HashMap<NodeId, Gl>,
}

fn store(ctx: &Context) -> std::cell::RefMut<'_, Store> {
    ctx.get_data::<RefCell<Store>>().expect("webgl store").borrow_mut()
}

/// Copy changed WebGL drawing buffers to their canvases (before the page is painted).
pub fn present(ctx: &Context) {
    let Some(s) = ctx.get_data::<RefCell<Store>>() else { return };
    let mut frames = Vec::new();
    for (id, gl) in s.borrow_mut().ctxs.iter_mut() {
        if gl.dirty {
            gl.dirty = false;
            frames.push((*id, gl.w, gl.h, gl.frame_rgba()));
            if !gl.attrs.preserve {
                gl.needs_clear = true;
            }
        }
    }
    for (id, w, h, px) in frames {
        crate::canvas::set_frame(ctx, id, w, h, &px);
    }
}

/// Copy one canvas's drawing buffer to its pixels (for toDataURL / drawImage).
fn sync_canvas(ctx: &Context, id: NodeId) {
    let Some(s) = ctx.get_data::<RefCell<Store>>() else { return };
    let frame = {
        let mut s = s.borrow_mut();
        match s.ctxs.get_mut(&id) {
            Some(gl) if gl.dirty => {
                gl.dirty = false;
                Some((gl.w, gl.h, gl.frame_rgba()))
            }
            _ => None,
        }
    };
    if let Some((w, h, px)) = frame {
        crate::canvas::set_frame(ctx, id, w, h, &px);
    }
}

// ---------------------------------------------------------------- argument helpers

fn num(a: &[JsValue], i: usize, ctx: &mut Context) -> f64 {
    a.get(i).map_or(Ok(f64::NAN), |v| v.to_number(ctx)).unwrap_or(f64::NAN)
}
fn u(a: &[JsValue], i: usize, ctx: &mut Context) -> u32 {
    let v = num(a, i, ctx);
    if v.is_finite() { v as i64 as u32 } else { 0 }
}
fn int(a: &[JsValue], i: usize, ctx: &mut Context) -> i32 {
    let v = num(a, i, ctx);
    if v.is_finite() { v as i64 as i32 } else { 0 }
}
fn fl(a: &[JsValue], i: usize, ctx: &mut Context) -> f32 {
    num(a, i, ctx) as f32
}
fn boolean(a: &[JsValue], i: usize) -> bool {
    a.get(i).is_some_and(|v| v.to_boolean())
}
fn string(a: &[JsValue], i: usize, ctx: &mut Context) -> String {
    a.get(i).and_then(|v| v.to_string(ctx).ok()).map(|s| s.to_std_string_escaped()).unwrap_or_default()
}

/// The bytes an ArrayBuffer, typed array or DataView covers.
fn view_bytes(v: Option<&JsValue>, ctx: &mut Context) -> Option<Vec<u8>> {
    let o = v?.as_object()?;
    if let Ok(arr) = JsTypedArray::from_object(o.clone()) {
        let buf = arr.buffer(ctx).ok()?;
        let buf = JsArrayBuffer::from_object(buf.as_object()?.clone()).ok()?;
        let (off, len) = (arr.byte_offset(ctx).ok()?, arr.byte_length(ctx).ok()?);
        let data = buf.data()?;
        return data.get(off..off + len).map(|b| b.to_vec());
    }
    if let Ok(dv) = JsDataView::from_object(o.clone()) {
        let buf = dv.buffer(ctx).ok()?;
        let buf = JsArrayBuffer::from_object(buf.as_object()?.clone()).ok()?;
        let (off, len) = (dv.byte_offset(ctx).ok()? as usize, dv.byte_length(ctx).ok()? as usize);
        let data = buf.data()?;
        return data.get(off..off + len).map(|b| b.to_vec());
    }
    if let Ok(buf) = JsArrayBuffer::from_object(o.clone()) {
        return buf.data().map(|d| d.to_vec());
    }
    None
}

/// Write bytes into a typed array view starting at byte `at`.
fn view_write(v: Option<&JsValue>, at: usize, bytes: &[u8], ctx: &mut Context) -> bool {
    let Some(o) = v.and_then(|v| v.as_object()) else { return false };
    let Ok(arr) = JsTypedArray::from_object(o.clone()) else { return false };
    let (Ok(buf), Ok(off), Ok(len)) = (arr.buffer(ctx), arr.byte_offset(ctx), arr.byte_length(ctx)) else { return false };
    let Some(bo) = buf.as_object() else { return false };
    let Ok(buf) = JsArrayBuffer::from_object(bo.clone()) else { return false };
    let Some(mut data) = buf.data_mut() else { return false };
    if at + bytes.len() > len {
        return false;
    }
    data[off + at..off + at + bytes.len()].copy_from_slice(bytes);
    true
}

type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// Run `f` on the context for canvas `a[0]`.
fn with_gl<R>(a: &[JsValue], ctx: &mut Context, f: impl FnOnce(&mut Gl) -> R) -> Option<R> {
    let id = num(a, 0, ctx) as NodeId;
    let mut s = store(ctx);
    s.ctxs.get_mut(&id).map(f)
}

fn undef() -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn jarr(vals: impl IntoIterator<Item = JsValue>, ctx: &mut Context) -> JsValue {
    JsArray::from_iter(vals, ctx).into()
}

// ---------------------------------------------------------------- natives

pub fn install(ctx: &mut Context) {
    ctx.insert_data(RefCell::new(Store::default()));
    let natives: &[(&str, usize, Native)] = &[
        ("__glCreate", 5, n_create),
        ("__glResize", 3, n_resize),
        ("__glFlush", 1, n_flush),
        ("__glHas", 1, n_has),
        ("__glError", 1, n_error),
        ("__glSetError", 2, n_set_error),
        ("__glGen", 3, n_gen),
        ("__glDel", 3, n_del),
        ("__glIs", 3, n_is),
        ("__glBindBuffer", 3, n_bind_buffer),
        ("__glBindBufferRange", 6, n_bind_buffer_range),
        ("__glBufferData", 6, n_buffer_data),
        ("__glBufferSubData", 6, n_buffer_sub_data),
        ("__glCopyBufferSubData", 6, n_copy_buffer_sub_data),
        ("__glGetBufferSubData", 6, n_get_buffer_sub_data),
        ("__glGetBufferParam", 3, n_get_buffer_param),
        ("__glBindTexture", 3, n_bind_texture),
        ("__glActiveTexture", 2, n_active_texture),
        ("__glTexImage", 13, n_tex_image),
        ("__glTexImageSource", 14, n_tex_image_source),
        ("__glTexStorage", 7, n_tex_storage),
        ("__glCopyTexImage", 10, n_copy_tex_image),
        ("__glTexParam", 4, n_tex_param),
        ("__glGetTexParam", 3, n_get_tex_param),
        ("__glGenerateMipmap", 2, n_generate_mipmap),
        ("__glShaderSource", 3, n_shader_source),
        ("__glCompileShader", 2, n_compile_shader),
        ("__glGetShaderParam", 3, n_get_shader_param),
        ("__glGetShaderInfoLog", 2, n_get_shader_info_log),
        ("__glGetShaderSource", 2, n_get_shader_source),
        ("__glAttachShader", 3, n_attach_shader),
        ("__glDetachShader", 3, n_detach_shader),
        ("__glLinkProgram", 2, n_link_program),
        ("__glUseProgram", 2, n_use_program),
        ("__glValidateProgram", 2, n_validate_program),
        ("__glGetProgramParam", 3, n_get_program_param),
        ("__glGetProgramInfoLog", 2, n_get_program_info_log),
        ("__glGetAttachedShaders", 2, n_get_attached_shaders),
        ("__glBindAttribLocation", 4, n_bind_attrib_location),
        ("__glGetAttribLocation", 3, n_get_attrib_location),
        ("__glGetFragDataLocation", 3, n_get_frag_data_location),
        ("__glGetUniformLocation", 3, n_get_uniform_location),
        ("__glGetActive", 4, n_get_active),
        ("__glUniform", 7, n_uniform),
        ("__glGetUniform", 4, n_get_uniform),
        ("__glUniformBlock", 5, n_uniform_block),
        ("__glEnableVA", 3, n_enable_va),
        ("__glVAPointer", 8, n_va_pointer),
        ("__glVADivisor", 3, n_va_divisor),
        ("__glVertexAttrib", 4, n_vertex_attrib),
        ("__glGetVertexAttrib", 3, n_get_vertex_attrib),
        ("__glBindVao", 2, n_bind_vao),
        ("__glDrawArrays", 5, n_draw_arrays),
        ("__glDrawElements", 6, n_draw_elements),
        ("__glClear", 2, n_clear),
        ("__glClearBuffer", 5, n_clear_buffer),
        ("__glEnable", 3, n_enable),
        ("__glIsEnabled", 2, n_is_enabled),
        ("__glState", 6, n_state),
        ("__glPixelStore", 3, n_pixel_store),
        ("__glGetParameter", 2, n_get_parameter),
        ("__glReadPixels", 9, n_read_pixels),
        ("__glBindFramebuffer", 3, n_bind_framebuffer),
        ("__glFramebufferTexture", 7, n_framebuffer_texture),
        ("__glFramebufferRenderbuffer", 4, n_framebuffer_renderbuffer),
        ("__glCheckFramebufferStatus", 2, n_check_framebuffer_status),
        ("__glGetFramebufferAttachmentParam", 4, n_get_fb_attachment_param),
        ("__glBindRenderbuffer", 3, n_bind_renderbuffer),
        ("__glRenderbufferStorage", 6, n_renderbuffer_storage),
        ("__glGetRenderbufferParam", 3, n_get_renderbuffer_param),
        ("__glDrawBuffers", 2, n_draw_buffers),
        ("__glReadBuffer", 2, n_read_buffer),
        ("__glBlitFramebuffer", 11, n_blit_framebuffer),
        ("__glBindSampler", 3, n_bind_sampler),
        ("__glSamplerParam", 4, n_sampler_param),
        ("__glEnableExt", 2, n_enable_ext),
        ("__glCompressedTexImage", 11, n_compressed_tex_image),
        ("__glGetSamplerParam", 3, n_get_sampler_param),
    ];
    for (name, len, f) in natives {
        ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f)).expect("register webgl native");
    }
    if let Err(e) = ctx.eval(Source::from_bytes(include_str!("webgl.js").as_bytes())) {
        log!("[ironswift] WebGL setup failed: {e}");
    }
}

fn seed_of(ctx: &Context) -> u32 {
    ctx.get_data::<RefCell<crate::js::PageCtx>>().map_or(0, |p| crate::security::fingerprint_seed(&p.borrow().top))
}

/// __glCreate(canvasId, w, h, [alpha, depth, stencil, antialias, premultipliedAlpha, preserveDrawingBuffer], v2)
fn n_create(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as NodeId;
    let (w, h) = (u(a, 1, ctx), u(a, 2, ctx));
    let mut flags = [true, true, false, true, true, false];
    if let Some(o) = a.get(3).and_then(|v| v.as_object()) {
        if let Ok(arr) = JsArray::from_object(o.clone()) {
            for (k, f) in flags.iter_mut().enumerate() {
                *f = arr.get(k as u32, ctx).map(|v| v.to_boolean()).unwrap_or(*f);
            }
        }
    }
    let attrs = Attrs { alpha: flags[0], depth: flags[1], stencil: flags[2], antialias: flags[3], premultiplied: flags[4], preserve: flags[5] };
    let v2 = boolean(a, 4);
    let seed = seed_of(ctx);
    store(ctx).ctxs.insert(id, Gl::new(w, h, attrs, v2, seed));
    crate::js::with_dom_mut(|_| ());
    undef()
}

fn n_resize(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (w, h) = (u(a, 1, ctx), u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        gl.w = w.min(MAX_TEXTURE_SIZE);
        gl.h = h.min(MAX_TEXTURE_SIZE);
        gl.alloc_default();
        // The viewport isn't changed by a resize (per spec), but a fresh context has one.
    });
    crate::js::with_dom_mut(|_| ());
    undef()
}

fn n_flush(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as NodeId;
    sync_canvas(ctx, id);
    undef()
}

fn n_has(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as NodeId;
    Ok(JsValue::from(store(ctx).ctxs.contains_key(&id)))
}

fn n_error(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let e = with_gl(a, ctx, |gl| std::mem::take(&mut gl.err)).unwrap_or(0);
    Ok(JsValue::from(e))
}

fn n_set_error(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let code = u(a, 1, ctx);
    with_gl(a, ctx, |gl| gl.error(code));
    undef()
}

/// __glGen(cid, kind, shaderType): 0 buffer, 1 texture, 2 framebuffer, 3 renderbuffer,
/// 4 shader, 5 program, 6 vertex array, 7 sampler, 8 query, 9 sync.
fn n_gen(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let kind = u(a, 1, ctx);
    let sty = u(a, 2, ctx);
    let id = with_gl(a, ctx, |gl| {
        let o = match kind {
            0 => Obj::Buffer(Buffer::default()),
            1 => Obj::Texture(Texture::new()),
            2 => Obj::Fbo(Fbo::default()),
            3 => Obj::Rbo(Rbo::default()),
            4 => {
                if sty != VERTEX_SHADER && sty != FRAGMENT_SHADER {
                    gl.error(INVALID_ENUM);
                    return 0;
                }
                Obj::Shader(Shader { ty: sty, src: String::new(), compiled: None, log: String::new(), deleted: false })
            }
            5 => Obj::Program(Program { shaders: Vec::new(), linked: None, link_ok: false, log: String::new(), bound_attribs: HashMap::new(), deleted: false, validate: false }),
            6 => Obj::Vao(Vao::default()),
            7 => Obj::Sampler(TexParams::default()),
            8 => Obj::Query(0),
            _ => Obj::Sync,
        };
        gl.new_id(o)
    })
    .unwrap_or(0);
    Ok(JsValue::from(id))
}

fn n_del(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let obj = u(a, 2, ctx);
    with_gl(a, ctx, |gl| {
        let Some(o) = gl.objs.get(&obj) else { return };
        match o {
            Obj::Buffer(_) => {
                if gl.array_buffer == obj {
                    gl.array_buffer = 0;
                }
                let vao = gl.vao_mut();
                if vao.ebo == obj {
                    vao.ebo = 0;
                }
                for at in vao.attribs.iter_mut() {
                    if at.buffer == obj {
                        at.buffer = 0;
                    }
                }
                gl.bound.retain(|_, v| *v != obj);
                for b in gl.ubo_bind.iter_mut() {
                    if b.0 == obj {
                        *b = (0, 0, 0);
                    }
                }
            }
            Obj::Texture(_) => {
                for unit in gl.units.iter_mut() {
                    for s in unit.iter_mut() {
                        if *s == obj {
                            *s = 0;
                        }
                    }
                }
                detach_everywhere(gl, |at| matches!(at, Att::Tex { id, .. } if *id == obj));
            }
            Obj::Rbo(_) => {
                if gl.rbo == obj {
                    gl.rbo = 0;
                }
                detach_everywhere(gl, |at| *at == Att::Rbo(obj));
            }
            Obj::Fbo(_) => {
                if gl.draw_fbo == obj {
                    gl.draw_fbo = 0;
                }
                if gl.read_fbo == obj {
                    gl.read_fbo = 0;
                }
            }
            Obj::Vao(_) => {
                if gl.vao == obj {
                    gl.vao = 0;
                }
            }
            Obj::Program(_) => {
                if gl.program == obj {
                    // A program in use stays until it's replaced.
                    if let Some(p) = gl.program_mut(obj) {
                        p.deleted = true;
                    }
                    return;
                }
            }
            Obj::Shader(_) => {
                let attached = gl.objs.values().any(|o| matches!(o, Obj::Program(p) if p.shaders.contains(&obj)));
                if attached {
                    if let Some(Obj::Shader(s)) = gl.objs.get_mut(&obj) {
                        s.deleted = true;
                    }
                    return;
                }
            }
            Obj::Sampler(_) => {
                for s in gl.samplers.iter_mut() {
                    if *s == obj {
                        *s = 0;
                    }
                }
            }
            _ => {}
        }
        gl.objs.remove(&obj);
    });
    undef()
}

fn detach_everywhere(gl: &mut Gl, f: impl Fn(&Att) -> bool) {
    let bound = [gl.draw_fbo, gl.read_fbo];
    for fid in bound {
        if let Some(fb) = gl.fbo_mut(fid) {
            for c in fb.color.iter_mut() {
                if c.as_ref().is_some_and(&f) {
                    *c = None;
                }
            }
            if fb.depth.as_ref().is_some_and(&f) {
                fb.depth = None;
            }
            if fb.stencil.as_ref().is_some_and(&f) {
                fb.stencil = None;
            }
        }
    }
}

fn n_is(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let kind = u(a, 1, ctx);
    let obj = u(a, 2, ctx);
    let r = with_gl(a, ctx, |gl| match gl.objs.get(&obj) {
        Some(Obj::Buffer(b)) => kind == 0 && b.kind != 0,
        Some(Obj::Texture(t)) => kind == 1 && t.target != 0,
        Some(Obj::Fbo(_)) => kind == 2,
        Some(Obj::Rbo(_)) => kind == 3,
        Some(Obj::Shader(_)) => kind == 4,
        Some(Obj::Program(_)) => kind == 5,
        Some(Obj::Vao(_)) => kind == 6,
        Some(Obj::Sampler(_)) => kind == 7,
        Some(Obj::Query(_)) => kind == 8,
        Some(Obj::Sync) => kind == 9,
        None => false,
    })
    .unwrap_or(false);
    Ok(JsValue::from(r))
}

fn n_bind_buffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = u(a, 1, ctx);
    let obj = u(a, 2, ctx);
    with_gl(a, ctx, |gl| {
        if gl.buffer_target(target).is_none() {
            return gl.error(INVALID_ENUM);
        }
        if obj != 0 {
            let want = if target == ELEMENT_ARRAY_BUFFER { 1 } else { 2 };
            let v2 = gl.v2;
            let Some(b) = gl.buf_mut(obj) else { return gl.error(INVALID_OPERATION) };
            if b.kind != 0 && b.kind != want && !(v2 && want == 2 && b.kind == 1 && false) {
                // WebGL never lets one buffer be both index data and anything else.
                return gl.error(INVALID_OPERATION);
            }
            b.kind = want;
        }
        match target {
            ARRAY_BUFFER => gl.array_buffer = obj,
            ELEMENT_ARRAY_BUFFER => gl.vao_mut().ebo = obj,
            t => {
                gl.bound.insert(t, obj);
            }
        }
    });
    undef()
}

/// __glBindBufferRange(cid, target, index, obj, offset, size) — size 0 = whole buffer.
fn n_bind_buffer_range(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, index, obj, off, size) = (u(a, 1, ctx), u(a, 2, ctx), u(a, 3, ctx), u(a, 4, ctx), u(a, 5, ctx));
    with_gl(a, ctx, |gl| {
        if target != UNIFORM_BUFFER && target != TRANSFORM_FEEDBACK_BUFFER {
            return gl.error(INVALID_ENUM);
        }
        if target == UNIFORM_BUFFER {
            if index as usize >= MAX_UNIFORM_BINDINGS {
                return gl.error(INVALID_VALUE);
            }
            if off % 256 != 0 {
                return gl.error(INVALID_VALUE);
            }
            gl.ubo_bind[index as usize] = (obj, off, size);
        }
        if let Some(b) = gl.buf_mut(obj) {
            if b.kind == 1 {
                return gl.error(INVALID_OPERATION);
            }
            b.kind = 2;
        }
        gl.bound.insert(target, obj);
    });
    undef()
}

/// __glBufferData(cid, target, sizeOrData, usage, srcOffset, length)
fn n_buffer_data(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = u(a, 1, ctx);
    let usage = u(a, 3, ctx);
    let data = if a.get(2).is_some_and(|v| v.is_number()) {
        let n = num(a, 2, ctx);
        if n < 0.0 || !n.is_finite() {
            with_gl(a, ctx, |gl| gl.error(INVALID_VALUE));
            return undef();
        }
        Some(vec![0u8; n as usize])
    } else {
        view_bytes(a.get(2), ctx)
    };
    let (src_off, len) = (u(a, 4, ctx) as usize, a.get(5).filter(|v| v.is_number()).map(|_| u(a, 5, ctx) as usize));
    with_gl(a, ctx, |gl| {
        let Some(data) = data else { return gl.error(INVALID_VALUE) };
        let Some(id) = gl.buffer_target(target) else { return gl.error(INVALID_ENUM) };
        if !matches!(usage, 0x88E0 | 0x88E4 | 0x88E8 | 0x88E1 | 0x88E2 | 0x88E5 | 0x88E6 | 0x88E9 | 0x88EA) {
            return gl.error(INVALID_ENUM);
        }
        let Some(b) = gl.buf_mut(id) else { return gl.error(INVALID_OPERATION) };
        let start = src_off.min(data.len());
        let end = match len {
            Some(l) if l > 0 => start + l,
            _ => data.len(),
        };
        if end > data.len() {
            return gl.error(INVALID_VALUE);
        }
        b.data = data[start..end].to_vec();
        b.usage = usage;
    });
    undef()
}

/// __glBufferSubData(cid, target, offset, data, srcOffset, length)
fn n_buffer_sub_data(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = u(a, 1, ctx);
    let off = num(a, 2, ctx);
    let data = view_bytes(a.get(3), ctx);
    let (src_off, len) = (u(a, 4, ctx) as usize, u(a, 5, ctx) as usize);
    with_gl(a, ctx, |gl| {
        let Some(data) = data else { return gl.error(INVALID_VALUE) };
        if off < 0.0 {
            return gl.error(INVALID_VALUE);
        }
        let Some(id) = gl.buffer_target(target) else { return gl.error(INVALID_ENUM) };
        let Some(b) = gl.buf_mut(id) else { return gl.error(INVALID_OPERATION) };
        let start = src_off.min(data.len());
        let end = if len > 0 { start + len } else { data.len() };
        if end > data.len() {
            return gl.error(INVALID_VALUE);
        }
        let src = &data[start..end];
        let o = off as usize;
        if o + src.len() > b.data.len() {
            return gl.error(INVALID_VALUE);
        }
        b.data[o..o + src.len()].copy_from_slice(src);
    });
    undef()
}

fn n_copy_buffer_sub_data(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (rt, wt, ro, wo, size) = (u(a, 1, ctx), u(a, 2, ctx), u(a, 3, ctx) as usize, u(a, 4, ctx) as usize, u(a, 5, ctx) as usize);
    with_gl(a, ctx, |gl| {
        let (Some(r), Some(w)) = (gl.buffer_target(rt), gl.buffer_target(wt)) else { return gl.error(INVALID_ENUM) };
        let Some(src) = gl.buf(r).map(|b| b.data.clone()) else { return gl.error(INVALID_OPERATION) };
        if ro + size > src.len() {
            return gl.error(INVALID_VALUE);
        }
        let Some(dst) = gl.buf_mut(w) else { return gl.error(INVALID_OPERATION) };
        if wo + size > dst.data.len() {
            return gl.error(INVALID_VALUE);
        }
        dst.data[wo..wo + size].copy_from_slice(&src[ro..ro + size]);
    });
    undef()
}

/// __glGetBufferSubData(cid, target, srcByteOffset, dstView, dstByteOffset, length)
fn n_get_buffer_sub_data(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, so) = (u(a, 1, ctx), u(a, 2, ctx) as usize);
    let (doff, len) = (u(a, 4, ctx) as usize, u(a, 5, ctx) as usize);
    let total = view_bytes(a.get(3), ctx).map_or(0, |b| b.len());
    let bytes = with_gl(a, ctx, |gl| {
        let Some(id) = gl.buffer_target(target) else {
            gl.error(INVALID_ENUM);
            return None;
        };
        let data = gl.buf(id).map(|b| b.data.clone())?;
        let n = if len == 0 { total.saturating_sub(doff) } else { len };
        if so + n > data.len() || doff + n > total {
            gl.error(INVALID_VALUE);
            return None;
        }
        Some(data[so..so + n].to_vec())
    })
    .flatten();
    if let Some(b) = bytes {
        view_write(a.get(3), doff, &b, ctx);
    }
    undef()
}

fn n_get_buffer_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, pname) = (u(a, 1, ctx), u(a, 2, ctx));
    let r = with_gl(a, ctx, |gl| {
        let Some(id) = gl.buffer_target(target) else {
            gl.error(INVALID_ENUM);
            return JsValue::null();
        };
        let Some(b) = gl.buf(id) else {
            gl.error(INVALID_OPERATION);
            return JsValue::null();
        };
        match pname {
            0x8764 => JsValue::from(b.data.len() as u32),
            0x8765 => JsValue::from(if b.usage == 0 { 0x88E4 } else { b.usage }),
            _ => {
                gl.error(INVALID_ENUM);
                JsValue::null()
            }
        }
    });
    Ok(r.unwrap_or(JsValue::null()))
}

fn n_bind_texture(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, obj) = (u(a, 1, ctx), u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        let Some(slot) = gl.tex_slot(target) else { return gl.error(INVALID_ENUM) };
        if obj != 0 {
            let Some(t) = gl.texture_mut(obj) else { return gl.error(INVALID_OPERATION) };
            if t.target != 0 && t.target != target {
                return gl.error(INVALID_OPERATION);
            }
            if t.target == 0 {
                t.target = target;
                if target == e::TEXTURE_CUBE_MAP {
                    t.faces = vec![Vec::new(); 6];
                } else {
                    t.faces = vec![Vec::new()];
                }
            }
        }
        let u = gl.active;
        gl.units[u][slot] = obj;
    });
    undef()
}

fn n_active_texture(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        if t < TEXTURE0 || t >= TEXTURE0 + MAX_UNITS as u32 {
            return gl.error(INVALID_ENUM);
        }
        gl.active = (t - TEXTURE0) as usize;
    });
    undef()
}

/// __glTexImage(cid, target, level, internal, w, h, d, border, format, type, data, srcOffset, mode)
/// mode: 0 texImage, 1 texSubImage (internal/border are x/y offsets packed: see JS), 2 = 3D image.
fn n_tex_image(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, level) = (u(a, 1, ctx), int(a, 2, ctx));
    let internal = u(a, 3, ctx);
    let (w, h, d) = (int(a, 4, ctx), int(a, 5, ctx), int(a, 6, ctx));
    let border = int(a, 7, ctx);
    let (format, ty) = (u(a, 8, ctx), u(a, 9, ctx));
    let data = if a.get(10).is_some_and(|v| v.is_object()) { view_bytes(a.get(10), ctx) } else { None };
    let has_data = a.get(10).is_some_and(|v| v.is_object());
    let src_off = u(a, 11, ctx) as usize;
    let sub = u(a, 12, ctx);
    let offs = if sub == 1 {
        a.get(13).and_then(|v| v.as_object()).and_then(|o| JsArray::from_object(o.clone()).ok()).map(|arr| (0..3).map(|i| arr.get(i, ctx).ok().and_then(|v| v.to_number(ctx).ok()).unwrap_or(0.0) as i32).collect::<Vec<i32>>())
    } else {
        None
    };
    with_gl(a, ctx, |gl| {
        if level < 0 || w < 0 || h < 0 || d < 0 || border != 0 && sub == 0 {
            return gl.error(INVALID_VALUE);
        }
        let v2 = gl.v2;
        if !valid_pixel_type(ty, v2) {
            return gl.error(INVALID_ENUM);
        }
        if sub != 1 && !size_ok(target, level, w, h, if sub == 2 { d } else { 1 }) {
            return gl.error(INVALID_VALUE);
        }
        let Some((id, face)) = gl.image_target(target) else { return };
        let mut unpack = Unpack { ..gl.unpack };
        let bytes: Option<Vec<u8>> = if has_data {
            let Some(b) = data.as_ref() else { return gl.error(INVALID_VALUE) };
            let es = match ty {
                e::UNSIGNED_SHORT | e::SHORT | e::HALF_FLOAT | e::HALF_FLOAT_OES | 0x8363 | 0x8033 | 0x8034 => 2,
                e::UNSIGNED_INT | e::INT | e::FLOAT | 0x84FA | 0x8368 | 0x8C3B | 0x8C3E => 4,
                _ => 1,
            };
            Some(b.get(src_off * es..).unwrap_or(&[]).to_vec())
        } else {
            None
        };
        if sub == 1 {
            let o = offs.unwrap_or(vec![0, 0, 0]);
            let Some(t) = gl.texture(id) else { return };
            let Some(img) = t.level(face, level as usize) else { return gl.error(INVALID_OPERATION) };
            let (x, y, z) = (o[0], o[1], o[2]);
            if x < 0 || y < 0 || z < 0 || x + w > img.w as i32 || y + h > img.h as i32 || z + d.max(1) > img.d as i32 {
                return gl.error(INVALID_VALUE);
            }
            let fmt = img.fmt;
            let Some(check) = tex::texture_format(if fmt.internal == fmt.layout || !v2 { fmt.internal } else { fmt.internal }, format, ty, v2).or(tex::texture_format(fmt.layout, format, ty, v2)) else {
                return gl.error(INVALID_OPERATION);
            };
            let _ = check;
            let Some(bytes) = bytes else { return gl.error(INVALID_VALUE) };
            let Some((px, st)) = tex::decode(&bytes, w as u32, h as u32, d.max(1) as u32, format, ty, &fmt, &unpack) else { return gl.error(INVALID_OPERATION) };
            let Some(t) = gl.texture_mut(id) else { return };
            let Some(Some(img)) = t.faces.get_mut(face).and_then(|f| f.get_mut(level as usize)) else { return };
            for zz in 0..d.max(1) {
                for yy in 0..h {
                    for xx in 0..w {
                        let si = ((zz * h + yy) * w + xx) as usize;
                        let di = (((z + zz) as u32 * img.h + (y + yy) as u32) * img.w + (x + xx) as u32) as usize;
                        img.px[di] = px[si];
                        if !st.is_empty() && !img.st.is_empty() {
                            img.st[di] = st[si];
                        }
                    }
                }
            }
            return;
        }
        let is3d = sub == 2;
        let Some(fmt) = tex::texture_format(internal, format, ty, v2) else {
            return gl.error(if v2 { INVALID_OPERATION } else { INVALID_ENUM });
        };
        if fmt.depth && !v2 && target != e::TEXTURE_2D {
            return gl.error(INVALID_OPERATION);
        }
        if target != e::TEXTURE_2D && !is3d && w != h {
            return gl.error(INVALID_VALUE);
        }
        if gl.texture(id).is_some_and(|t| t.immutable) {
            return gl.error(INVALID_OPERATION);
        }
        let dd = if is3d { d.max(1) as u32 } else { 1 };
        let img = match bytes {
            Some(b) => {
                if fmt.depth && !v2 {
                    // WebGL 1 depth textures can't take data.
                    return gl.error(INVALID_OPERATION);
                }
                unpack.flip_y = unpack.flip_y && !is3d;
                match tex::decode(&b, w as u32, h as u32, dd, format, ty, &fmt, &unpack) {
                    Some((px, st)) => Image { w: w as u32, h: h as u32, d: dd, fmt, px, st },
                    None => return gl.error(INVALID_OPERATION),
                }
            }
            None => Image::new(w as u32, h as u32, dd, fmt, v2),
        };
        if let Some(t) = gl.texture_mut(id) {
            t.set_level(face, level as usize, img);
        }
    });
    undef()
}

/// __glTexImageSource(cid, target, level, internal, format, type, kind, key, data, w, h, sub, [x, y, z], is3d)
/// kind: 0 <img>, 1 canvas, 2 raw RGBA8 data (ImageData) with w×h.
fn n_tex_image_source(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, level, internal, format, ty) = (u(a, 1, ctx), int(a, 2, ctx), u(a, 3, ctx), u(a, 4, ctx), u(a, 5, ctx));
    let kind = u(a, 6, ctx) as u8;
    let key = string(a, 7, ctx);
    let sub = boolean(a, 11);
    let offs: Vec<i32> = a
        .get(12)
        .and_then(|v| v.as_object())
        .and_then(|o| JsArray::from_object(o.clone()).ok())
        .map(|arr| (0..3).map(|i| arr.get(i, ctx).ok().and_then(|v| v.to_number(ctx).ok()).unwrap_or(0.0) as i32).collect())
        .unwrap_or(vec![0, 0, 0]);
    let is3d = boolean(a, 13);
    // The source's pixels, straight RGBA, top row first.
    let src: Option<(u32, u32, Vec<u8>)> = match kind {
        2 => {
            let (w, h) = (u(a, 9, ctx), u(a, 10, ctx));
            view_bytes(a.get(8), ctx).map(|b| (w, h, b))
        }
        k => {
            if k == 1 {
                if let Ok(id) = key.parse::<NodeId>() {
                    sync_canvas(ctx, id);
                }
            }
            crate::canvas::source_rgba(ctx, k, &key)
        }
    };
    with_gl(a, ctx, |gl| {
        // An image that hasn't loaded uploads nothing (and isn't an error).
        let Some((w, h, rgba)) = src else { return };
        if !valid_pixel_type(ty, gl.v2) {
            return gl.error(INVALID_ENUM);
        }
        if rgba.len() < (w * h * 4) as usize {
            return gl.error(INVALID_VALUE);
        }
        let Some((id, face)) = gl.image_target(target) else { return };
        let v2 = gl.v2;
        let up = (gl.unpack.flip_y, gl.unpack.premultiply);
        if sub {
            let Some(t) = gl.texture(id) else { return };
            let Some(img) = t.level(face, level.max(0) as usize) else { return gl.error(INVALID_OPERATION) };
            let fmt = img.fmt;
            let (x, y, z) = (offs[0], offs[1], offs[2]);
            if x < 0 || y < 0 || x + w as i32 > img.w as i32 || y + h as i32 > img.h as i32 || z < 0 || z >= img.d as i32 {
                return gl.error(INVALID_VALUE);
            }
            let px = tex::from_rgba8(&rgba, w, h, format, ty, &fmt, up.0, up.1);
            let Some(t) = gl.texture_mut(id) else { return };
            let Some(Some(img)) = t.faces.get_mut(face).and_then(|f| f.get_mut(level as usize)) else { return };
            for yy in 0..h as i32 {
                for xx in 0..w as i32 {
                    let di = ((z as u32 * img.h + (y + yy) as u32) * img.w + (x + xx) as u32) as usize;
                    img.px[di] = px[(yy as u32 * w + xx as u32) as usize];
                }
            }
            return;
        }
        let Some(fmt) = tex::texture_format(internal, format, ty, v2) else { return gl.error(if v2 { INVALID_OPERATION } else { INVALID_ENUM }) };
        if fmt.depth || fmt.is_integer() && false {
            return gl.error(INVALID_OPERATION);
        }
        if target != e::TEXTURE_2D && !is3d && w != h {
            return gl.error(INVALID_VALUE);
        }
        if gl.texture(id).is_some_and(|t| t.immutable) {
            return gl.error(INVALID_OPERATION);
        }
        let px = tex::from_rgba8(&rgba, w, h, format, ty, &fmt, up.0, up.1);
        if let Some(t) = gl.texture_mut(id) {
            t.set_level(face, level.max(0) as usize, Image { w, h, d: 1, fmt, px, st: Vec::new() });
        }
    });
    undef()
}

/// __glTexStorage(cid, target, levels, internal, w, h, d)
fn n_tex_storage(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, levels, internal) = (u(a, 1, ctx), int(a, 2, ctx), u(a, 3, ctx));
    let (w, h, d) = (int(a, 4, ctx), int(a, 5, ctx), int(a, 6, ctx).max(1));
    with_gl(a, ctx, |gl| {
        let Some(slot) = gl.tex_slot(target) else { return gl.error(INVALID_ENUM) };
        let id = gl.units[gl.active][slot];
        if id == 0 {
            return gl.error(INVALID_OPERATION);
        }
        if levels < 1 || w < 1 || h < 1 {
            return gl.error(INVALID_VALUE);
        }
        let compressed = if gl.s3tc { tex::compressed_format(internal).map(|c| c.0) } else { None };
        let Some(fmt) = tex::sized_format(internal, gl.v2).or(compressed) else { return gl.error(INVALID_ENUM) };
        if target == e::TEXTURE_3D && (fmt.depth || fmt.stencil || compressed.is_some()) {
            return gl.error(INVALID_OPERATION);
        }
        let v2 = gl.v2;
        let Some(t) = gl.texture_mut(id) else { return };
        if t.immutable {
            return gl.error(INVALID_OPERATION);
        }
        let nf = if target == e::TEXTURE_CUBE_MAP { 6 } else { 1 };
        let (mut lw, mut lh, mut ld) = (w as u32, h as u32, d as u32);
        for lv in 0..levels as usize {
            for f in 0..nf {
                t.set_level(f, lv, Image::new(lw, lh, ld, fmt, v2));
            }
            lw = (lw / 2).max(1);
            lh = (lh / 2).max(1);
            if target == e::TEXTURE_3D {
                ld = (ld / 2).max(1);
            }
        }
        t.immutable = true;
        t.immutable_levels = levels as u32;
    });
    undef()
}

/// __glCopyTexImage(cid, target, level, internal, x, y, w, h, sub, [xoff, yoff, zoff])
fn n_copy_tex_image(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, level, internal) = (u(a, 1, ctx), int(a, 2, ctx), u(a, 3, ctx));
    let (x, y, w, h) = (int(a, 4, ctx), int(a, 5, ctx), int(a, 6, ctx), int(a, 7, ctx));
    let sub = boolean(a, 8);
    let offs: Vec<i32> = a
        .get(9)
        .and_then(|v| v.as_object())
        .and_then(|o| JsArray::from_object(o.clone()).ok())
        .map(|arr| (0..3).map(|i| arr.get(i, ctx).ok().and_then(|v| v.to_number(ctx).ok()).unwrap_or(0.0) as i32).collect())
        .unwrap_or(vec![0, 0, 0]);
    with_gl(a, ctx, |gl| {
        if w < 0 || h < 0 || level < 0 {
            return gl.error(INVALID_VALUE);
        }
        let Some((id, face)) = gl.image_target(target) else { return };
        let Some((px, src_fmt)) = gl.read_rect(x, y, w, h) else { return };
        let v2 = gl.v2;
        let px: Vec<[f32; 4]> = px.into_iter().map(|p| p.unwrap_or([0.0; 4])).collect();
        if sub {
            let Some(t) = gl.texture_mut(id) else { return };
            let Some(Some(img)) = t.faces.get_mut(face).and_then(|f| f.get_mut(level as usize)) else { return gl.error(INVALID_OPERATION) };
            let (ox, oy, oz) = (offs[0], offs[1], offs[2]);
            if ox < 0 || oy < 0 || ox + w > img.w as i32 || oy + h > img.h as i32 || oz < 0 || oz >= img.d as i32 {
                return gl.error(INVALID_VALUE);
            }
            let fmt = img.fmt;
            for yy in 0..h {
                for xx in 0..w {
                    let di = ((oz as u32 * img.h + (oy + yy) as u32) * img.w + (ox + xx) as u32) as usize;
                    img.px[di] = tex::store(&fmt, px[(yy * w + xx) as usize]);
                }
            }
            return;
        }
        let is_unsized = matches!(internal, e::RGBA | e::RGB | e::ALPHA | e::LUMINANCE | e::LUMINANCE_ALPHA);
        let fmt = if is_unsized { tex::texture_format(internal, internal, e::UNSIGNED_BYTE, false) } else { tex::sized_format(internal, v2) };
        let Some(mut fmt) = fmt else { return gl.error(INVALID_ENUM) };
        if src_fmt.is_integer() != fmt.is_integer() || fmt.depth {
            return gl.error(INVALID_OPERATION);
        }
        // Channels the copy keeps (r g b a by layout).
        let keep: [bool; 4] = match fmt.layout {
            e::RGBA => [true; 4],
            e::RGB => [true, true, true, false],
            e::RG => [true, true, false, false],
            e::RED | e::LUMINANCE => [true, false, false, false],
            e::ALPHA => [false, false, false, true],
            _ => [true, false, false, true],
        };
        if src_fmt.layout == e::RGB && keep[3] {
            return gl.error(INVALID_OPERATION);
        }
        if is_unsized {
            // The effective format: the source's component sizes.
            for c in 0..4 {
                fmt.bits[c] = if keep[c] { src_fmt.bits[c] } else { 0 };
            }
            if src_fmt.bits[..4].iter().any(|&b| b != 8 && b != 0) {
                fmt.num = src_fmt.num;
            }
        } else if v2 {
            // A sized copy must match the source's component sizes.
            for c in 0..4 {
                if keep[c] && fmt.bits[c] != src_fmt.bits[c] && !(matches!(fmt.layout, e::LUMINANCE | e::ALPHA | e::LUMINANCE_ALPHA)) {
                    return gl.error(INVALID_OPERATION);
                }
            }
        }
        if (fmt.layout == e::RGBA || fmt.layout == e::ALPHA || fmt.layout == e::LUMINANCE_ALPHA) && src_fmt.layout == e::RGB && !v2 {
            return gl.error(INVALID_OPERATION);
        }
        let img = Image { w: w as u32, h: h as u32, d: 1, fmt, px: px.into_iter().map(|p| tex::store(&fmt, p)).collect(), st: Vec::new() };
        if let Some(t) = gl.texture_mut(id) {
            if t.immutable {
                return;
            }
            t.set_level(face, level as usize, img);
        }
    });
    undef()
}

fn set_param(p: &mut TexParams, pname: u32, v: f32, v2: bool) -> Result<(), u32> {
    let vi = v as u32;
    match pname {
        0x2801 => {
            if !matches!(vi, e::NEAREST | e::LINEAR | e::NEAREST_MIPMAP_NEAREST | e::LINEAR_MIPMAP_NEAREST | e::NEAREST_MIPMAP_LINEAR | e::LINEAR_MIPMAP_LINEAR) {
                return Err(INVALID_ENUM);
            }
            p.min = vi
        }
        0x2800 => {
            if !matches!(vi, e::NEAREST | e::LINEAR) {
                return Err(INVALID_ENUM);
            }
            p.mag = vi
        }
        0x2802 | 0x2803 | 0x8072 => {
            if !matches!(vi, e::REPEAT | e::CLAMP_TO_EDGE | e::MIRRORED_REPEAT) || (pname == 0x8072 && !v2) {
                return Err(INVALID_ENUM);
            }
            match pname {
                0x2802 => p.wrap_s = vi,
                0x2803 => p.wrap_t = vi,
                _ => p.wrap_r = vi,
            }
        }
        0x813C if v2 => p.base = vi,
        0x813D if v2 => p.max = vi,
        0x813A if v2 => p.min_lod = v,
        0x813B if v2 => p.max_lod = v,
        0x884C if v2 => p.compare_mode = vi,
        0x884D if v2 => p.compare_func = vi,
        // TEXTURE_MAX_ANISOTROPY_EXT
        0x84FE => p.anisotropy = v.max(1.0),
        _ => return Err(INVALID_ENUM),
    }
    Ok(())
}

fn get_param(p: &TexParams, pname: u32) -> Option<JsValue> {
    Some(match pname {
        0x2801 => JsValue::from(p.min),
        0x2800 => JsValue::from(p.mag),
        0x2802 => JsValue::from(p.wrap_s),
        0x2803 => JsValue::from(p.wrap_t),
        0x8072 => JsValue::from(p.wrap_r),
        0x813C => JsValue::from(p.base),
        0x813D => JsValue::from(p.max),
        0x813A => JsValue::from(p.min_lod as f64),
        0x813B => JsValue::from(p.max_lod as f64),
        0x884C => JsValue::from(p.compare_mode),
        0x884D => JsValue::from(p.compare_func),
        0x84FE => JsValue::from(p.anisotropy as f64),
        _ => return None,
    })
}

fn n_tex_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, pname, v) = (u(a, 1, ctx), u(a, 2, ctx), fl(a, 3, ctx));
    with_gl(a, ctx, |gl| {
        let Some(slot) = gl.tex_slot(target) else { return gl.error(INVALID_ENUM) };
        let id = gl.units[gl.active][slot];
        let v2 = gl.v2;
        let Some(t) = gl.texture_mut(id) else { return gl.error(INVALID_OPERATION) };
        if pname == 0x912F || pname == 0x82DF {
            return gl.error(INVALID_ENUM);
        }
        if let Err(e) = set_param(&mut t.p, pname, v, v2) {
            gl.error(e);
        }
    });
    undef()
}

fn n_get_tex_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, pname) = (u(a, 1, ctx), u(a, 2, ctx));
    let r = with_gl(a, ctx, |gl| {
        let Some(slot) = gl.tex_slot(target) else {
            gl.error(INVALID_ENUM);
            return JsValue::null();
        };
        let id = gl.units[gl.active][slot];
        let Some(t) = gl.texture(id) else {
            gl.error(INVALID_OPERATION);
            return JsValue::null();
        };
        if pname == 0x912F && gl.v2 {
            return JsValue::from(t.immutable);
        }
        if pname == 0x82DF && gl.v2 {
            return JsValue::from(t.immutable_levels);
        }
        match get_param(&t.p, pname) {
            Some(v) => v,
            None => {
                gl.error(INVALID_ENUM);
                JsValue::null()
            }
        }
    });
    Ok(r.unwrap_or(JsValue::null()))
}

fn n_generate_mipmap(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        let Some(slot) = gl.tex_slot(target) else { return gl.error(INVALID_ENUM) };
        let id = gl.units[gl.active][slot];
        let v2 = gl.v2;
        let Some(t) = gl.texture_mut(id) else { return gl.error(INVALID_OPERATION) };
        let Some(b) = t.base_image() else { return gl.error(INVALID_OPERATION) };
        if !v2 && (!b.w.is_power_of_two() || !b.h.is_power_of_two()) {
            return gl.error(INVALID_OPERATION);
        }
        if b.fmt.depth || b.fmt.is_integer() {
            return gl.error(INVALID_OPERATION);
        }
        if !t.generate_mipmap() {
            gl.error(INVALID_OPERATION);
        }
    });
    undef()
}

fn shader_mut(gl: &mut Gl, id: u32) -> Option<&mut Shader> {
    match gl.objs.get_mut(&id) {
        Some(Obj::Shader(s)) => Some(s),
        _ => None,
    }
}

fn n_shader_source(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = u(a, 1, ctx);
    let src = string(a, 2, ctx);
    with_gl(a, ctx, |gl| match shader_mut(gl, id) {
        Some(s) => s.src = src,
        None => gl.error(INVALID_OPERATION),
    });
    undef()
}

fn n_compile_shader(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        let v2 = gl.v2;
        let Some(s) = shader_mut(gl, id) else { return gl.error(INVALID_OPERATION) };
        let stage = if s.ty == VERTEX_SHADER { Stage::Vertex } else { Stage::Fragment };
        match compile(&s.src, stage, v2) {
            Ok(c) => {
                s.compiled = Some(Rc::new(c));
                s.log.clear();
            }
            Err(m) => {
                s.compiled = None;
                s.log = format!("{m}\n");
            }
        }
    });
    undef()
}

fn n_get_shader_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (id, pname) = (u(a, 1, ctx), u(a, 2, ctx));
    let r = with_gl(a, ctx, |gl| {
        let Some(s) = shader_mut(gl, id) else {
            gl.error(INVALID_OPERATION);
            return JsValue::null();
        };
        match pname {
            0x8B4F => JsValue::from(s.ty),
            0x8B80 => JsValue::from(s.deleted),
            0x8B81 => JsValue::from(s.compiled.is_some()),
            _ => {
                gl.error(INVALID_ENUM);
                JsValue::null()
            }
        }
    });
    Ok(r.unwrap_or(JsValue::null()))
}

fn n_get_shader_info_log(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = u(a, 1, ctx);
    let r = with_gl(a, ctx, |gl| shader_mut(gl, id).map(|s| s.log.clone())).flatten();
    Ok(r.map_or(JsValue::null(), |s| JsValue::from(JsString::from(s.as_str()))))
}

fn n_get_shader_source(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = u(a, 1, ctx);
    let r = with_gl(a, ctx, |gl| shader_mut(gl, id).map(|s| s.src.clone())).flatten();
    Ok(r.map_or(JsValue::null(), |s| JsValue::from(JsString::from(s.as_str()))))
}

fn n_attach_shader(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, s) = (u(a, 1, ctx), u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        let Some(sty) = shader_mut(gl, s).map(|x| x.ty) else { return gl.error(INVALID_OPERATION) };
        let types: Vec<u32> = match gl.program_ref(p) {
            Some(pr) => pr.shaders.iter().filter_map(|x| match gl.objs.get(x) {
                Some(Obj::Shader(sh)) => Some(sh.ty),
                _ => None,
            }).collect(),
            None => return gl.error(INVALID_OPERATION),
        };
        if types.contains(&sty) {
            return gl.error(INVALID_OPERATION);
        }
        if let Some(pr) = gl.program_mut(p) {
            pr.shaders.push(s);
        }
    });
    undef()
}

fn n_detach_shader(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, s) = (u(a, 1, ctx), u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        let Some(pr) = gl.program_mut(p) else { return gl.error(INVALID_OPERATION) };
        if !pr.shaders.contains(&s) {
            return gl.error(INVALID_OPERATION);
        }
        pr.shaders.retain(|x| *x != s);
        if let Some(Obj::Shader(sh)) = gl.objs.get(&s) {
            if sh.deleted {
                gl.objs.remove(&s);
            }
        }
    });
    undef()
}

fn n_link_program(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        if gl.program_ref(p).is_none() {
            return gl.error(INVALID_OPERATION);
        }
        gl.link(p);
    });
    undef()
}

fn n_use_program(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        if p != 0 && !gl.program_ref(p).is_some_and(|x| x.link_ok) {
            return gl.error(INVALID_OPERATION);
        }
        let old = gl.program;
        gl.program = p;
        if old != p && gl.program_ref(old).is_some_and(|x| x.deleted) {
            gl.objs.remove(&old);
        }
    });
    undef()
}

fn n_validate_program(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        if let Some(pr) = gl.program_mut(p) {
            pr.validate = pr.link_ok;
        } else {
            gl.error(INVALID_OPERATION);
        }
    });
    undef()
}

fn n_get_program_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, pname) = (u(a, 1, ctx), u(a, 2, ctx));
    let r = with_gl(a, ctx, |gl| {
        let Some(pr) = gl.program_ref(p) else {
            gl.error(INVALID_OPERATION);
            return JsValue::null();
        };
        let l = pr.linked.as_ref();
        match pname {
            0x8B80 => JsValue::from(pr.deleted),
            0x8B82 => JsValue::from(pr.link_ok),
            0x8B83 => JsValue::from(pr.validate),
            0x8B85 => JsValue::from(pr.shaders.len() as u32),
            0x8B86 => JsValue::from(l.map_or(0, |l| l.uniforms.len() as u32)),
            0x8B89 => JsValue::from(l.map_or(0, |l| l.attribs.len() as u32)),
            0x8A36 if gl.v2 => JsValue::from(l.map_or(0, |l| l.blocks.len() as u32)),
            0x8C7F if gl.v2 => JsValue::from(0x8C8C),
            0x8C83 if gl.v2 => JsValue::from(0),
            _ => {
                gl.error(INVALID_ENUM);
                JsValue::null()
            }
        }
    });
    Ok(r.unwrap_or(JsValue::null()))
}

fn n_get_program_info_log(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    let r = with_gl(a, ctx, |gl| gl.program_ref(p).map(|x| x.log.clone())).flatten();
    Ok(r.map_or(JsValue::null(), |s| JsValue::from(JsString::from(s.as_str()))))
}

fn n_get_attached_shaders(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    let r = with_gl(a, ctx, |gl| gl.program_ref(p).map(|x| x.shaders.clone())).flatten();
    Ok(match r {
        Some(v) => jarr(v.into_iter().map(JsValue::from), ctx),
        None => JsValue::null(),
    })
}

fn n_bind_attrib_location(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, idx) = (u(a, 1, ctx), u(a, 2, ctx));
    let name = string(a, 3, ctx);
    with_gl(a, ctx, |gl| {
        if idx as usize >= MAX_ATTRIBS {
            return gl.error(INVALID_VALUE);
        }
        if name.starts_with("gl_") || name.starts_with("webgl_") || name.starts_with("_webgl_") {
            return gl.error(INVALID_OPERATION);
        }
        match gl.program_mut(p) {
            Some(pr) => {
                pr.bound_attribs.insert(name, idx);
            }
            None => gl.error(INVALID_OPERATION),
        }
    });
    undef()
}

fn n_get_attrib_location(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    let name = string(a, 2, ctx);
    let r = with_gl(a, ctx, |gl| {
        let Some(pr) = gl.program_ref(p) else {
            gl.error(INVALID_OPERATION);
            return -1;
        };
        let Some(l) = pr.linked.as_ref() else {
            gl.error(INVALID_OPERATION);
            return -1;
        };
        l.attribs.iter().find(|x| x.0 == name).map_or(-1, |x| x.2 as i32)
    })
    .unwrap_or(-1);
    Ok(JsValue::from(r))
}

fn n_get_frag_data_location(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    let name = string(a, 2, ctx);
    let r = with_gl(a, ctx, |gl| {
        let l = gl.program_ref(p).and_then(|x| x.linked.as_ref());
        match l {
            Some(l) => l.out_names.iter().find(|x| x.0 == name).map_or(-1, |x| x.1 as i32),
            None => {
                gl.error(INVALID_OPERATION);
                -1
            }
        }
    })
    .unwrap_or(-1);
    Ok(JsValue::from(r))
}

/// Find a uniform by name: "u", "a", "a[0]", "a[2]", "s.f", "s[1].f". Returns (index, element).
fn find_uniform(l: &Linked, name: &str) -> Option<(usize, u32)> {
    for (i, u) in l.uniforms.iter().enumerate() {
        if u.block.is_some() {
            continue;
        }
        if u.name == name {
            return Some((i, 0));
        }
        if u.is_array {
            let base = &u.name[..u.name.len() - 3];
            if name == base {
                return Some((i, 0));
            }
            if let Some(rest) = name.strip_prefix(base) {
                if let Some(n) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    if let Ok(k) = n.parse::<u32>() {
                        if k < u.count {
                            return Some((i, k));
                        }
                    }
                }
            }
        }
    }
    None
}

fn n_get_uniform_location(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let p = u(a, 1, ctx);
    let name = string(a, 2, ctx);
    let r = with_gl(a, ctx, |gl| {
        let Some(pr) = gl.program_ref(p) else {
            gl.error(INVALID_OPERATION);
            return None;
        };
        let Some(l) = pr.linked.as_ref() else {
            gl.error(INVALID_OPERATION);
            return None;
        };
        find_uniform(l, &name)
    })
    .flatten();
    Ok(match r {
        Some((i, k)) => jarr([JsValue::from(i as u32), JsValue::from(k)], ctx),
        None => JsValue::null(),
    })
}

/// The GL type enum for a GLSL type.
fn gl_type(t: &Ty) -> u32 {
    use crate::glsl::Samp;
    match t {
        Ty::Scalar(Base::F) => e::FLOAT,
        Ty::Scalar(Base::I) => e::INT,
        Ty::Scalar(Base::U) => e::UNSIGNED_INT,
        Ty::Scalar(Base::B) => 0x8B56,
        Ty::Vec(Base::F, n) => 0x8B50 + (*n as u32 - 2),
        Ty::Vec(Base::I, n) => 0x8B53 + (*n as u32 - 2),
        Ty::Vec(Base::B, n) => 0x8B57 + (*n as u32 - 2),
        Ty::Vec(Base::U, n) => 0x8DC6 + (*n as u32 - 2),
        Ty::Mat(2, 2) => 0x8B5A,
        Ty::Mat(3, 3) => 0x8B5B,
        Ty::Mat(4, 4) => 0x8B5C,
        Ty::Mat(2, 3) => 0x8B65,
        Ty::Mat(2, 4) => 0x8B66,
        Ty::Mat(3, 2) => 0x8B67,
        Ty::Mat(3, 4) => 0x8B68,
        Ty::Mat(4, 2) => 0x8B69,
        Ty::Mat(4, 3) => 0x8B6A,
        Ty::Sampler(Samp { dim, shadow, base }) => match (dim, shadow, base) {
            (Dim::D2, false, Base::F) => 0x8B5E,
            (Dim::D3, false, Base::F) => 0x8B5F,
            (Dim::Cube, false, Base::F) => 0x8B60,
            (Dim::D2, true, _) => 0x8B62,
            (Dim::D2Array, false, Base::F) => 0x8DC1,
            (Dim::D2Array, true, _) => 0x8DC4,
            (Dim::Cube, true, _) => 0x8DC5,
            (Dim::D2, _, Base::I) => 0x8DCA,
            (Dim::D3, _, Base::I) => 0x8DCB,
            (Dim::Cube, _, Base::I) => 0x8DCC,
            (Dim::D2Array, _, Base::I) => 0x8DCF,
            (Dim::D2, _, _) => 0x8DD2,
            (Dim::D3, _, _) => 0x8DD3,
            (Dim::Cube, _, _) => 0x8DD4,
            (Dim::D2Array, _, _) => 0x8DD7,
        },
        _ => 0,
    }
}

/// __glGetActive(cid, program, index, attrib?) -> [size, type, name] or null
fn n_get_active(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, idx) = (u(a, 1, ctx), u(a, 2, ctx) as usize);
    let attrib = boolean(a, 3);
    let r = with_gl(a, ctx, |gl| {
        let Some(l) = gl.program_ref(p).and_then(|x| x.linked.as_ref()) else {
            gl.error(INVALID_OPERATION);
            return None;
        };
        if attrib {
            match l.attribs.get(idx) {
                Some(x) => Some((1u32, gl_type(&x.1), x.0.clone())),
                None => {
                    gl.error(INVALID_VALUE);
                    None
                }
            }
        } else {
            match l.uniforms.get(idx) {
                Some(x) => Some((x.count, gl_type(&x.ty), x.name.clone())),
                None => {
                    gl.error(INVALID_VALUE);
                    None
                }
            }
        }
    })
    .flatten();
    Ok(match r {
        Some((size, ty, name)) => jarr([JsValue::from(size), JsValue::from(ty), JsValue::from(JsString::from(name.as_str()))], ctx),
        None => JsValue::null(),
    })
}

/// __glUniform(cid, program, index, element, kind, data, transpose)
/// kind = base*100 + cols*10 + rows (base 0 float, 1 int, 2 uint); vectors have cols 1.
fn n_uniform(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, idx, el, kind) = (u(a, 1, ctx), u(a, 2, ctx) as usize, u(a, 3, ctx), u(a, 4, ctx));
    let bytes = view_bytes(a.get(5), ctx).unwrap_or_default();
    let transpose = boolean(a, 6);
    with_gl(a, ctx, |gl| {
        let v2 = gl.v2;
        if p != gl.program {
            return gl.error(INVALID_OPERATION);
        }
        let Some(l) = gl.program_mut(p).and_then(|x| x.linked.as_mut()) else { return gl.error(INVALID_OPERATION) };
        let Some(un) = l.uniforms.get(idx).cloned().filter(|u| u.block.is_none()) else { return gl.error(INVALID_OPERATION) };
        let (base, cols, rows) = (kind / 100, (kind / 10) % 10, kind % 10);
        let comps = (cols * rows) as usize;
        // Type check.
        let ok = match &un.ty {
            Ty::Scalar(Base::B) | Ty::Vec(Base::B, _) => cols == 1 && rows == un.ty.size() && base <= 2,
            Ty::Scalar(b) | Ty::Vec(b, _) => cols == 1 && rows == un.ty.size() && *b as u32 == [Base::F, Base::I, Base::U][base as usize] as u32,
            Ty::Mat(c, r) => cols == *c as u32 && rows == *r as u32 && base == 0,
            Ty::Sampler(_) => base == 1 && cols == 1 && rows == 1,
            _ => false,
        };
        if !ok {
            return gl.error(INVALID_OPERATION);
        }
        if transpose && !v2 {
            return gl.error(INVALID_VALUE);
        }
        let words: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        if words.is_empty() || words.len() % comps != 0 {
            return gl.error(INVALID_VALUE);
        }
        let n = words.len() / comps;
        if n > 1 && !un.is_array {
            return gl.error(INVALID_OPERATION);
        }
        let n = n.min((un.count - el) as usize);
        if let Ty::Sampler(_) = un.ty {
            if words.iter().any(|&w| w as i32 >= MAX_UNITS as i32 || (w as i32) < 0) {
                return gl.error(INVALID_VALUE);
            }
        }
        let size = un.ty.size() as usize;
        for k in 0..n {
            let mut vals: Vec<u32> = words[k * comps..(k + 1) * comps].to_vec();
            if transpose {
                let mut t = vals.clone();
                for c in 0..cols as usize {
                    for r in 0..rows as usize {
                        t[c * rows as usize + r] = vals[r * cols as usize + c];
                    }
                }
                vals = t;
            }
            if matches!(un.ty.base(), Some(Base::B)) {
                vals = vals.iter().map(|&w| if base == 0 { (f32::from_bits(w) != 0.0) as u32 } else { (w != 0) as u32 }).collect();
            }
            for &(stage, slot) in &un.slots {
                let mem = if stage == 0 { &mut l.vs_mem } else { &mut l.fs_mem };
                let at = slot as usize + (el as usize + k) * size;
                for (c, w) in vals.iter().enumerate() {
                    if let Some(m) = mem.get_mut(at + c) {
                        *m = [f32::from_bits(*w); 4];
                    }
                }
            }
        }
    });
    undef()
}

/// __glGetUniform(cid, program, index, element) -> number | bool | array
fn n_get_uniform(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (p, idx, el) = (u(a, 1, ctx), u(a, 2, ctx) as usize, u(a, 3, ctx));
    let r = with_gl(a, ctx, |gl| {
        let Some(l) = gl.program_ref(p).and_then(|x| x.linked.as_ref()) else {
            gl.error(INVALID_OPERATION);
            return None;
        };
        let un = l.uniforms.get(idx)?;
        let (stage, slot) = *un.slots.first()?;
        let mem = if stage == 0 { &l.vs_mem } else { &l.fs_mem };
        let size = un.ty.size() as usize;
        let at = slot as usize + el as usize * size;
        let words: Vec<u32> = (0..size).map(|k| mem[at + k][0].to_bits()).collect();
        Some((un.ty.clone(), words))
    })
    .flatten();
    let Some((ty, words)) = r else { return Ok(JsValue::null()) };
    let conv = |w: u32, b: Option<Base>| -> JsValue {
        match b {
            Some(Base::F) => JsValue::from(f32::from_bits(w) as f64),
            Some(Base::I) => JsValue::from(w as i32),
            Some(Base::U) => JsValue::from(w),
            Some(Base::B) => JsValue::from(w != 0),
            None => JsValue::from(w as i32),
        }
    };
    let b = ty.base();
    if words.len() == 1 {
        return Ok(conv(words[0], b));
    }
    Ok(match b {
        Some(Base::F) => JsFloat32Array::from_iter(words.iter().map(|w| f32::from_bits(*w)), ctx)?.into(),
        Some(Base::I) => JsInt32Array::from_iter(words.iter().map(|w| *w as i32), ctx)?.into(),
        Some(Base::U) => JsUint32Array::from_iter(words.iter().copied(), ctx)?.into(),
        _ => jarr(words.iter().map(|w| JsValue::from(*w != 0)), ctx),
    })
}

/// __glUniformBlock(cid, op, program, a, b): uniform block queries and bindings.
/// op 0: getUniformBlockIndex(name=a) · 1: uniformBlockBinding(index=a, binding=b) ·
/// 2: getActiveUniformBlockName(a) · 3: getActiveUniformBlockParameter(a, pname=b) ·
/// 4: getUniformIndices(names=a) · 5: getActiveUniforms(indices=a, pname=b)
fn n_uniform_block(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let op = u(a, 1, ctx);
    let p = u(a, 2, ctx);
    let sname = string(a, 3, ctx);
    let (x, y) = (u(a, 3, ctx), u(a, 4, ctx));
    let list: Vec<JsValue> = a
        .get(3)
        .and_then(|v| v.as_object())
        .and_then(|o| JsArray::from_object(o.clone()).ok())
        .map(|arr| {
            let n = arr.length(ctx).unwrap_or(0);
            (0..n).map(|i| arr.get(i, ctx).unwrap_or_default()).collect()
        })
        .unwrap_or_default();
    let names: Vec<String> = list.iter().map(|v| v.to_string(ctx).map(|s| s.to_std_string_escaped()).unwrap_or_default()).collect();
    let nums: Vec<u32> = list.iter().map(|v| v.to_number(ctx).unwrap_or(0.0) as u32).collect();
    enum Out {
        N(u32),
        S(String),
        B(bool),
        L(Vec<u32>),
        Li(Vec<i32>),
        Null,
    }
    let r = with_gl(a, ctx, |gl| {
        let Some(l) = gl.program_mut(p).and_then(|x| x.linked.as_mut()) else {
            gl.error(INVALID_OPERATION);
            return Out::Null;
        };
        match op {
            0 => Out::N(l.blocks.iter().position(|b| b.name == sname).map_or(u32::MAX, |i| i as u32)),
            1 => {
                if (x as usize) < l.block_binding.len() && (y as usize) < MAX_UNIFORM_BINDINGS {
                    l.block_binding[x as usize] = y;
                    Out::Null
                } else {
                    gl.error(INVALID_VALUE);
                    Out::Null
                }
            }
            2 => match l.blocks.get(x as usize) {
                Some(b) => Out::S(b.name.clone()),
                None => {
                    gl.error(INVALID_VALUE);
                    Out::Null
                }
            },
            3 => {
                let Some(b) = l.blocks.get(x as usize) else {
                    gl.error(INVALID_VALUE);
                    return Out::Null;
                };
                match y {
                    0x8A3F => Out::N(l.block_binding[x as usize]),
                    0x8A40 => Out::N(b.size),
                    0x8A42 => Out::N(b.members.len() as u32),
                    0x8A43 => Out::L(b.members.iter().filter_map(|m| l.uniforms.iter().position(|u| &u.name == m).map(|i| i as u32)).collect()),
                    0x8A44 => Out::B(b.stages.iter().any(|s| s.0 == 0)),
                    0x8A46 => Out::B(b.stages.iter().any(|s| s.0 == 1)),
                    _ => {
                        gl.error(INVALID_ENUM);
                        Out::Null
                    }
                }
            }
            4 => Out::L(names.iter().map(|n| find_uniform(l, n).map_or(u32::MAX, |(i, _)| i as u32)).collect()),
            5 => {
                let mut v = Vec::new();
                for &i in &nums {
                    let Some(un) = l.uniforms.get(i as usize) else {
                        gl.error(INVALID_VALUE);
                        return Out::Null;
                    };
                    v.push(match y {
                        0x8A37 => gl_type(&un.ty) as i32,
                        0x8A38 => un.count as i32,
                        0x8A3A => un.block.map_or(-1, |b| b.0 as i32),
                        0x8A3B => un.block.map_or(-1, |b| b.1 as i32),
                        0x8A3C => un.block.map_or(-1, |b| b.2 as i32),
                        0x8A3D => un.block.map_or(-1, |_| if matches!(un.ty, Ty::Mat(..)) { 16 } else { 0 }),
                        0x8A3E => 0,
                        _ => {
                            gl.error(INVALID_ENUM);
                            return Out::Null;
                        }
                    });
                }
                Out::Li(v)
            }
            _ => Out::Null,
        }
    });
    Ok(match r {
        Some(Out::N(n)) => JsValue::from(n),
        Some(Out::S(s)) => JsValue::from(JsString::from(s.as_str())),
        Some(Out::B(b)) => JsValue::from(b),
        Some(Out::L(l)) => jarr(l.into_iter().map(JsValue::from), ctx),
        Some(Out::Li(l)) => jarr(l.into_iter().map(JsValue::from), ctx),
        _ => JsValue::null(),
    })
}

fn n_enable_va(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (i, on) = (u(a, 1, ctx) as usize, boolean(a, 2));
    with_gl(a, ctx, |gl| {
        if i >= MAX_ATTRIBS {
            return gl.error(INVALID_VALUE);
        }
        gl.vao_mut().attribs[i].enabled = on;
    });
    undef()
}

/// __glVAPointer(cid, index, size, type, normalized, stride, offset, integer)
fn n_va_pointer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (i, size, ty) = (u(a, 1, ctx) as usize, int(a, 2, ctx), u(a, 3, ctx));
    let norm = boolean(a, 4);
    let (stride, offset) = (num(a, 5, ctx), num(a, 6, ctx));
    let integer = boolean(a, 7);
    with_gl(a, ctx, |gl| {
        if i >= MAX_ATTRIBS || !(1..=4).contains(&size) || !(0.0..=255.0).contains(&stride) || offset < 0.0 {
            return gl.error(INVALID_VALUE);
        }
        let packed = matches!(ty, 0x8D9F | 0x8368);
        let ok = match ty {
            e::BYTE | e::UNSIGNED_BYTE | e::SHORT | e::UNSIGNED_SHORT => true,
            e::FLOAT => !integer,
            e::INT | e::UNSIGNED_INT => gl.v2,
            e::HALF_FLOAT => gl.v2 && !integer,
            _ if packed => gl.v2 && !integer,
            _ => false,
        };
        if !ok {
            return gl.error(INVALID_ENUM);
        }
        if packed && size != 4 {
            return gl.error(INVALID_OPERATION);
        }
        let ts = type_size(ty);
        if (offset as u32) % ts != 0 || (stride as u32) % ts != 0 {
            return gl.error(INVALID_OPERATION);
        }
        let buf = gl.array_buffer;
        if buf == 0 && offset != 0.0 {
            return gl.error(INVALID_OPERATION);
        }
        let at = &mut gl.vao_mut().attribs[i];
        at.buffer = buf;
        at.size = size as u32;
        at.ty = ty;
        at.norm = norm && !integer;
        at.stride = stride as u32;
        at.offset = offset as u32;
        at.int = integer;
    });
    undef()
}

fn n_va_divisor(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (i, d) = (u(a, 1, ctx) as usize, u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        if i >= MAX_ATTRIBS {
            return gl.error(INVALID_VALUE);
        }
        gl.vao_mut().attribs[i].divisor = d;
    });
    undef()
}

/// __glVertexAttrib(cid, index, [x y z w], kind: 0 float, 1 int, 2 uint)
fn n_vertex_attrib(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let i = u(a, 1, ctx) as usize;
    let kind = u(a, 3, ctx) as u8;
    let mut v = [0.0f32, 0.0, 0.0, 1.0];
    if let Some(arr) = a.get(2).and_then(|x| x.as_object()).and_then(|o| JsArray::from_object(o.clone()).ok()) {
        for (k, slot) in v.iter_mut().enumerate() {
            if let Ok(x) = arr.get(k as u32, ctx) {
                if !x.is_undefined() {
                    let n = x.to_number(ctx).unwrap_or(0.0);
                    *slot = match kind {
                        1 => f32::from_bits(n as i64 as i32 as u32),
                        2 => f32::from_bits(n as i64 as u32),
                        _ => n as f32,
                    };
                }
            }
        }
        if kind != 0 {
            // Missing components default to (0, 0, 0, 1) as integers.
            let n = arr.length(ctx).unwrap_or(4);
            if n < 4 {
                v[3] = f32::from_bits(1);
            }
        }
    }
    with_gl(a, ctx, |gl| {
        if i >= MAX_ATTRIBS {
            return gl.error(INVALID_VALUE);
        }
        gl.generic[i] = (v, kind);
    });
    undef()
}

fn n_get_vertex_attrib(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (i, pname) = (u(a, 1, ctx) as usize, u(a, 2, ctx));
    enum Out {
        N(u32),
        B(bool),
        Cur([f32; 4], u8),
        Null,
    }
    let r = with_gl(a, ctx, |gl| {
        if i >= MAX_ATTRIBS {
            gl.error(INVALID_VALUE);
            return Out::Null;
        }
        let at = gl.vao_ref().attribs[i];
        match pname {
            0x8622 => Out::B(at.enabled),
            0x8623 => Out::N(at.size),
            0x8624 => Out::N(at.stride),
            0x8625 => Out::N(at.ty),
            0x886A => Out::B(at.norm),
            0x889F => Out::N(at.buffer),
            0x88FE => Out::N(at.divisor),
            0x88FD if gl.v2 => Out::B(at.int),
            0x8645 => Out::N(at.offset),
            0x8626 => Out::Cur(gl.generic[i].0, gl.generic[i].1),
            _ => {
                gl.error(INVALID_ENUM);
                Out::Null
            }
        }
    });
    Ok(match r {
        Some(Out::N(n)) => JsValue::from(n),
        Some(Out::B(b)) => JsValue::from(b),
        Some(Out::Cur(v, 0)) => JsFloat32Array::from_iter(v, ctx)?.into(),
        Some(Out::Cur(v, 1)) => JsInt32Array::from_iter(v.map(|x| x.to_bits() as i32), ctx)?.into(),
        Some(Out::Cur(v, _)) => JsUint32Array::from_iter(v.map(|x| x.to_bits()), ctx)?.into(),
        _ => JsValue::null(),
    })
}

fn n_bind_vao(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        if id != 0 && !matches!(gl.objs.get(&id), Some(Obj::Vao(_))) {
            return gl.error(INVALID_OPERATION);
        }
        gl.vao = id;
    });
    undef()
}

fn n_draw_arrays(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (mode, first, count, inst) = (u(a, 1, ctx), int(a, 2, ctx), int(a, 3, ctx), a.get(4).filter(|v| v.is_number()).map_or(1, |_| int(a, 4, ctx)));
    with_gl(a, ctx, |gl| gl.draw(mode, first, count, None, inst));
    crate::js::with_dom_mut(|_| ());
    undef()
}

fn n_draw_elements(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (mode, count, ty) = (u(a, 1, ctx), int(a, 2, ctx), u(a, 3, ctx));
    let off = num(a, 4, ctx);
    let inst = a.get(5).filter(|v| v.is_number()).map_or(1, |_| int(a, 5, ctx));
    with_gl(a, ctx, |gl| {
        if off < 0.0 {
            return gl.error(INVALID_VALUE);
        }
        if ty == e::UNSIGNED_INT && false {
            return gl.error(INVALID_ENUM);
        }
        gl.draw(mode, 0, count, Some((ty, off as u32)), inst)
    });
    crate::js::with_dom_mut(|_| ());
    undef()
}

fn n_clear(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mask = u(a, 1, ctx);
    with_gl(a, ctx, |gl| gl.clear(mask));
    crate::js::with_dom_mut(|_| ());
    undef()
}

/// __glClearBuffer(cid, buffer, drawbuffer, values, kind: 0 float, 1 int, 2 uint, 3 depth-stencil)
fn n_clear_buffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (buffer, db) = (u(a, 1, ctx), int(a, 2, ctx));
    let kind = u(a, 4, ctx) as u8;
    let mut vals = [0.0f32; 4];
    if let Some(arr) = a.get(3).and_then(|x| x.as_object()).and_then(|o| JsArray::from_object(o.clone()).ok()) {
        for (k, slot) in vals.iter_mut().enumerate() {
            let n = arr.get(k as u32, ctx).ok().and_then(|x| x.to_number(ctx).ok()).unwrap_or(0.0);
            *slot = match kind {
                1 => f32::from_bits(n as i64 as i32 as u32),
                2 => f32::from_bits(n as i64 as u32),
                _ => n as f32,
            };
        }
    }
    with_gl(a, ctx, |gl| gl.clear_buffer(buffer, db, &vals, kind));
    crate::js::with_dom_mut(|_| ());
    undef()
}

fn n_enable(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (cap, on) = (u(a, 1, ctx), boolean(a, 2));
    with_gl(a, ctx, |gl| {
        let ok = matches!(cap, CULL_FACE | BLEND | DITHER | STENCIL_TEST | DEPTH_TEST | SCISSOR_TEST | POLYGON_OFFSET_FILL | SAMPLE_ALPHA_TO_COVERAGE | SAMPLE_COVERAGE)
            || (gl.v2 && cap == RASTERIZER_DISCARD);
        if !ok {
            return gl.error(INVALID_ENUM);
        }
        gl.caps.insert(cap, on);
    });
    undef()
}

fn n_is_enabled(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let cap = u(a, 1, ctx);
    let r = with_gl(a, ctx, |gl| {
        let ok = matches!(cap, CULL_FACE | BLEND | DITHER | STENCIL_TEST | DEPTH_TEST | SCISSOR_TEST | POLYGON_OFFSET_FILL | SAMPLE_ALPHA_TO_COVERAGE | SAMPLE_COVERAGE)
            || (gl.v2 && cap == RASTERIZER_DISCARD);
        if !ok {
            gl.error(INVALID_ENUM);
            return false;
        }
        gl.cap(cap)
    })
    .unwrap_or(false);
    Ok(JsValue::from(r))
}

fn valid_func(f: u32) -> bool {
    (e::NEVER..=e::ALWAYS).contains(&f)
}
fn valid_blend_factor(f: u32) -> bool {
    matches!(f, 0 | 1 | 0x300..=0x308 | 0x8001..=0x8004)
}
fn valid_stencil_op(o: u32) -> bool {
    matches!(o, 0 | g::KEEP | g::REPLACE | g::INCR | g::DECR | g::INVERT | g::INCR_WRAP | g::DECR_WRAP)
}

/// __glState(cid, op, a, b, c, d): the small state setters.
fn n_state(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let op = u(a, 1, ctx);
    let v = [num(a, 2, ctx), num(a, 3, ctx), num(a, 4, ctx), num(a, 5, ctx)];
    let ui = v.map(|x| if x.is_finite() { x as i64 as u32 } else { 0 });
    with_gl(a, ctx, |gl| {
        match op {
            // clearColor
            0 => gl.clear_color = v.map(|x| x as f32),
            1 => gl.clear_depth = (v[0] as f32).clamp(0.0, 1.0),
            2 => gl.clear_stencil = v[0] as i32,
            // blendColor
            3 => gl.blend_color = v.map(|x| x as f32),
            // blendEquationSeparate(rgb, alpha)
            4 => {
                let ok = |m: u32| matches!(m, g::FUNC_ADD | g::FUNC_SUBTRACT | g::FUNC_REVERSE_SUBTRACT | g::MIN | g::MAX);
                if !ok(ui[0]) || !ok(ui[1]) {
                    return gl.error(INVALID_ENUM);
                }
                gl.blend_eq = [ui[0], ui[1]];
            }
            // blendFuncSeparate(srcRGB, dstRGB, srcA, dstA)
            5 => {
                if !ui.iter().all(|f| valid_blend_factor(*f)) {
                    return gl.error(INVALID_ENUM);
                }
                let constant_color = |f: u32| f == g::CONSTANT_COLOR || f == g::ONE_MINUS_CONSTANT_COLOR;
                let constant_alpha = |f: u32| f == g::CONSTANT_ALPHA || f == g::ONE_MINUS_CONSTANT_ALPHA;
                if !gl.v2 && ((constant_color(ui[0]) && constant_alpha(ui[1])) || (constant_alpha(ui[0]) && constant_color(ui[1]))) {
                    return gl.error(INVALID_OPERATION);
                }
                gl.blend_src = [ui[0], ui[2]];
                gl.blend_dst = [ui[1], ui[3]];
            }
            6 => gl.color_mask = [v[0] != 0.0, v[1] != 0.0, v[2] != 0.0, v[3] != 0.0],
            7 => {
                if !matches!(ui[0], g::FRONT | g::BACK | g::FRONT_AND_BACK) {
                    return gl.error(INVALID_ENUM);
                }
                gl.cull_face = ui[0];
            }
            8 => {
                if ui[0] != 0x900 && ui[0] != 0x901 {
                    return gl.error(INVALID_ENUM);
                }
                gl.front_face = ui[0];
            }
            9 => {
                if !valid_func(ui[0]) {
                    return gl.error(INVALID_ENUM);
                }
                gl.depth_func = ui[0];
            }
            10 => gl.depth_mask = v[0] != 0.0,
            11 => {
                let (n, f) = (v[0] as f32, v[1] as f32);
                if n > f {
                    return gl.error(INVALID_OPERATION);
                }
                gl.depth_range = [n.clamp(0.0, 1.0), f.clamp(0.0, 1.0)];
            }
            // stencilFuncSeparate(face, func, ref, mask)
            12 => {
                if !valid_func(ui[1]) || !matches!(ui[0], g::FRONT | g::BACK | g::FRONT_AND_BACK) {
                    return gl.error(INVALID_ENUM);
                }
                for (k, f) in gl.stencil.iter_mut().enumerate() {
                    if ui[0] == g::FRONT_AND_BACK || (k == 0) == (ui[0] == g::FRONT) {
                        f.func = ui[1];
                        f.reference = v[2] as i32;
                        f.mask = ui[3];
                    }
                }
            }
            // stencilOpSeparate(face, fail, zfail, zpass)
            13 => {
                if !valid_stencil_op(ui[1]) || !valid_stencil_op(ui[2]) || !valid_stencil_op(ui[3]) || !matches!(ui[0], g::FRONT | g::BACK | g::FRONT_AND_BACK) {
                    return gl.error(INVALID_ENUM);
                }
                for (k, f) in gl.stencil.iter_mut().enumerate() {
                    if ui[0] == g::FRONT_AND_BACK || (k == 0) == (ui[0] == g::FRONT) {
                        f.fail = ui[1];
                        f.zfail = ui[2];
                        f.zpass = ui[3];
                    }
                }
            }
            // stencilMaskSeparate(face, mask)
            14 => {
                if !matches!(ui[0], g::FRONT | g::BACK | g::FRONT_AND_BACK) {
                    return gl.error(INVALID_ENUM);
                }
                for (k, f) in gl.stencil.iter_mut().enumerate() {
                    if ui[0] == g::FRONT_AND_BACK || (k == 0) == (ui[0] == g::FRONT) {
                        f.writemask = ui[1];
                    }
                }
            }
            15 | 16 => {
                let (w, h) = (v[2] as i32, v[3] as i32);
                if w < 0 || h < 0 {
                    return gl.error(INVALID_VALUE);
                }
                let r = [v[0] as i32, v[1] as i32, w.min(8192), h.min(8192)];
                if op == 15 {
                    gl.viewport = r;
                } else {
                    gl.scissor = r;
                }
            }
            17 => {
                if v[0].is_nan() || v[0] <= 0.0 {
                    return gl.error(INVALID_VALUE);
                }
                gl.line_width = v[0] as f32;
            }
            18 => gl.poly_offset = (v[0] as f32, v[1] as f32),
            19 => gl.sample_coverage = (v[0].clamp(0.0, 1.0) as f32, v[1] != 0.0),
            // hint(target, mode)
            20 => {
                if !matches!(ui[1], 0x1100..=0x1102) {
                    return gl.error(INVALID_ENUM);
                }
                if !matches!(ui[0], 0x8192 | 0x8B8B) {
                    return gl.error(INVALID_ENUM);
                }
                gl.hints.insert(ui[0], ui[1]);
            }
            _ => gl.error(INVALID_ENUM),
        }
    });
    undef()
}

fn n_pixel_store(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (pname, v) = (u(a, 1, ctx), num(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        let vi = v as i64;
        let align_ok = matches!(vi, 1 | 2 | 4 | 8);
        match pname {
            0x0D05 => {
                if !align_ok {
                    return gl.error(INVALID_VALUE);
                }
                gl.pack_alignment = vi as usize
            }
            0x0CF5 => {
                if !align_ok {
                    return gl.error(INVALID_VALUE);
                }
                gl.unpack.alignment = vi as usize
            }
            0x9240 => gl.unpack.flip_y = v != 0.0,
            0x9241 => gl.unpack.premultiply = v != 0.0,
            0x9243 => gl.colorspace = vi as u32,
            0x0CF2 if gl.v2 => gl.unpack.row_length = vi.max(0) as usize,
            0x0CF3 if gl.v2 => gl.unpack.skip_rows = vi.max(0) as usize,
            0x0CF4 if gl.v2 => gl.unpack.skip_pixels = vi.max(0) as usize,
            0x806E if gl.v2 => gl.unpack.image_height = vi.max(0) as usize,
            0x806D if gl.v2 => gl.unpack.skip_images = vi.max(0) as usize,
            0x0D02 if gl.v2 => gl.pack_row_length = vi.max(0) as usize,
            0x0D03 if gl.v2 => gl.pack_skip_rows = vi.max(0) as usize,
            0x0D04 if gl.v2 => gl.pack_skip_pixels = vi.max(0) as usize,
            _ => gl.error(INVALID_ENUM),
        }
    });
    undef()
}

fn n_get_parameter(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let pname = u(a, 1, ctx);
    enum P {
        N(f64),
        B(bool),
        S(&'static str),
        F(Vec<f32>),
        I(Vec<i32>),
        U(Vec<u32>),
        Bs(Vec<bool>),
        Null,
    }
    let r = with_gl(a, ctx, |gl| {
        let read_img: Option<Fmt> = if gl.read_fbo == 0 {
            Some(gl.fb_color.fmt)
        } else {
            gl.fbo(gl.read_fbo).and_then(|f| f.color[0]).and_then(|at| gl.att_image(&at)).map(|i| i.fmt)
        };
        let draw_bits = |k: usize| -> f64 {
            if gl.draw_fbo == 0 {
                match k {
                    0..=2 => 8.0,
                    3 => {
                        if gl.attrs.alpha {
                            8.0
                        } else {
                            0.0
                        }
                    }
                    4 => {
                        if gl.attrs.depth {
                            24.0
                        } else {
                            0.0
                        }
                    }
                    _ => {
                        if gl.attrs.stencil {
                            8.0
                        } else {
                            0.0
                        }
                    }
                }
            } else {
                let f = gl.fbo(gl.draw_fbo);
                let att = match k {
                    0..=3 => f.and_then(|f| f.color[0]),
                    4 => f.and_then(|f| f.depth),
                    _ => f.and_then(|f| f.stencil),
                };
                att.and_then(|at| gl.att_image(&at)).map_or(0.0, |i| i.fmt.bits[k] as f64)
            }
        };
        let v2 = gl.v2;
        let n = |x: u32| P::N(x as f64);
        match pname {
            0x1F00 => P::S("WebKit"),
            0x1F01 => P::S("WebKit WebGL"),
            0x1F02 => P::S(if v2 { "WebGL 2.0 (Ironswift)" } else { "WebGL 1.0 (Ironswift)" }),
            0x8B8C => P::S(if v2 { "WebGL GLSL ES 3.00 (Ironswift)" } else { "WebGL GLSL ES 1.0 (Ironswift)" }),
            0x0D33 | 0x84E8 => n(MAX_TEXTURE_SIZE),
            0x851C => n(MAX_TEXTURE_SIZE),
            0x8073 if v2 => n(2048),
            0x88FF if v2 => n(2048),
            0x0D3A => P::I(vec![MAX_RENDERBUFFER_SIZE as i32; 2]),
            0x8869 => n(MAX_ATTRIBS as u32),
            0x8DFB => n(256),
            0x8DFD => n(256),
            0x8DFC => n(15),
            0x8B4D => n(MAX_UNITS as u32),
            0x8B4C | 0x8872 => n(16),
            0x0D50 => n(4),
            0x846D => P::F(vec![1.0, 1024.0]),
            0x846E => P::F(vec![1.0, 1.0]),
            0x0D52 => P::N(draw_bits(0)),
            0x0D53 => P::N(draw_bits(1)),
            0x0D54 => P::N(draw_bits(2)),
            0x0D55 => P::N(draw_bits(3)),
            0x0D56 => P::N(draw_bits(4)),
            0x0D57 => P::N(draw_bits(5)),
            0x0BA2 => P::I(gl.viewport.to_vec()),
            0x0C10 => P::I(gl.scissor.to_vec()),
            0x0C22 => P::F(gl.clear_color.to_vec()),
            0x0B73 => P::N(gl.clear_depth as f64),
            0x0B91 => P::N(gl.clear_stencil as f64),
            0x0C23 => P::Bs(gl.color_mask.to_vec()),
            0x8005 => P::F(gl.blend_color.to_vec()),
            0x8009 => n(gl.blend_eq[0]),
            0x883D => n(gl.blend_eq[1]),
            0x80C9 => n(gl.blend_src[0]),
            0x80C8 => n(gl.blend_dst[0]),
            0x80CB => n(gl.blend_src[1]),
            0x80CA => n(gl.blend_dst[1]),
            0x0B74 => n(gl.depth_func),
            0x0B72 => P::B(gl.depth_mask),
            0x0B70 => P::F(gl.depth_range.to_vec()),
            0x0B45 => n(gl.cull_face),
            0x0B46 => n(gl.front_face),
            0x0B21 => P::N(gl.line_width as f64),
            0x8038 => P::N(gl.poly_offset.0 as f64),
            0x2A00 => P::N(gl.poly_offset.1 as f64),
            0x80AA => P::N(gl.sample_coverage.0 as f64),
            0x80AB => P::B(gl.sample_coverage.1),
            0x80A8 => n(0),
            0x80A9 => n(0),
            0x0B92 => n(gl.stencil[0].func),
            0x0B94 => n(gl.stencil[0].fail),
            0x0B95 => n(gl.stencil[0].zfail),
            0x0B96 => n(gl.stencil[0].zpass),
            0x0B97 => P::N(gl.stencil[0].reference as f64),
            0x0B93 => n(gl.stencil[0].mask),
            0x0B98 => n(gl.stencil[0].writemask),
            0x8800 => n(gl.stencil[1].func),
            0x8801 => n(gl.stencil[1].fail),
            0x8802 => n(gl.stencil[1].zfail),
            0x8803 => n(gl.stencil[1].zpass),
            0x8CA3 => P::N(gl.stencil[1].reference as f64),
            0x8CA4 => n(gl.stencil[1].mask),
            0x8CA5 => n(gl.stencil[1].writemask),
            0x0CF5 => n(gl.unpack.alignment as u32),
            0x0D05 => n(gl.pack_alignment as u32),
            0x9240 => P::B(gl.unpack.flip_y),
            0x9241 => P::B(gl.unpack.premultiply),
            0x9243 => n(gl.colorspace),
            0x84E0 => n(TEXTURE0 + gl.active as u32),
            0x8192 => n(gl.hints.get(&0x8192).copied().unwrap_or(0x1100)),
            0x8B8B => n(gl.hints.get(&0x8B8B).copied().unwrap_or(0x1100)),
            0x86A3 => P::U(Vec::new()),
            0x8B9A => n(match read_img.map(|f| f.num) {
                Some(Num::Float) | Some(Num::Half) => e::FLOAT,
                Some(Num::Int) => e::INT,
                Some(Num::Uint) => e::UNSIGNED_INT,
                _ => e::UNSIGNED_BYTE,
            }),
            0x8B9B => n(match read_img {
                Some(f) if f.is_integer() => e::RGBA_INTEGER,
                _ => e::RGBA,
            }),
            // Object bindings: the ids, which webgl.js turns back into objects.
            0x8894 => n(gl.array_buffer),
            0x8895 => n(gl.vao_ref().ebo),
            0x8B8D => n(gl.program),
            0x8CA6 => n(gl.draw_fbo),
            0x8CAA if v2 => n(gl.read_fbo),
            0x8CA7 => n(gl.rbo),
            0x8069 => n(gl.units[gl.active][0]),
            0x806A if v2 => n(gl.units[gl.active][1]),
            0x8514 => n(gl.units[gl.active][2]),
            0x8C1D if v2 => n(gl.units[gl.active][3]),
            0x85B5 => n(gl.vao),
            0x8A28 if v2 => n(gl.bound.get(&UNIFORM_BUFFER).copied().unwrap_or(0)),
            0x8F36 if v2 => n(gl.bound.get(&COPY_READ_BUFFER).copied().unwrap_or(0)),
            0x8F37 if v2 => n(gl.bound.get(&COPY_WRITE_BUFFER).copied().unwrap_or(0)),
            0x88ED if v2 => n(gl.bound.get(&PIXEL_PACK_BUFFER).copied().unwrap_or(0)),
            0x88EF if v2 => n(gl.bound.get(&PIXEL_UNPACK_BUFFER).copied().unwrap_or(0)),
            0x8C8F if v2 => n(gl.bound.get(&TRANSFORM_FEEDBACK_BUFFER).copied().unwrap_or(0)),
            0x8919 if v2 => n(gl.samplers[gl.active]),
            0x8E25 if v2 => n(0),
            // WebGL 2 limits and state.
            0x8824 => n(if v2 { 8 } else { 4 }),
            0x8CDF => n(if v2 { 8 } else { 4 }),
            x if (0x8825..0x8825 + 8).contains(&x) => {
                let i = (x - 0x8825) as usize;
                if gl.draw_fbo == 0 {
                    n(if i == 0 { gl.fb_draw_buffer } else { 0 })
                } else {
                    n(gl.fbo(gl.draw_fbo).map_or(0, |f| f.draw_buffers.get(i).copied().unwrap_or(0)))
                }
            }
            0x0C02 if v2 => n(if gl.read_fbo == 0 { gl.fb_read_buffer } else { gl.fbo(gl.read_fbo).map_or(0, |f| f.read_buffer) }),
            0x8C89 if v2 => P::B(gl.cap(RASTERIZER_DISCARD)),
            0x0CF2 if v2 => n(gl.unpack.row_length as u32),
            0x0CF3 if v2 => n(gl.unpack.skip_rows as u32),
            0x0CF4 if v2 => n(gl.unpack.skip_pixels as u32),
            0x806E if v2 => n(gl.unpack.image_height as u32),
            0x806D if v2 => n(gl.unpack.skip_images as u32),
            0x0D02 if v2 => n(gl.pack_row_length as u32),
            0x0D03 if v2 => n(gl.pack_skip_rows as u32),
            0x0D04 if v2 => n(gl.pack_skip_pixels as u32),
            0x80E8 | 0x80E9 if v2 => n(1 << 20),
            0x84FD if v2 => n(16),
            0x8B49 | 0x8B4A if v2 => n(1024),
            0x8B4B if v2 => n(60),
            0x8904 if v2 => P::N(-8.0),
            0x8905 if v2 => n(7),
            0x9122 if v2 => n(64),
            0x9125 if v2 => n(60),
            0x9111 if v2 => n(0),
            0x8D6B if v2 => P::N(u32::MAX as f64),
            0x8D57 if v2 => n(1),
            0x8A2B | 0x8A2D if v2 => n(12),
            0x8A2E if v2 => n(24),
            0x8A2F if v2 => n(MAX_UNIFORM_BINDINGS as u32),
            0x8A30 if v2 => n(16384),
            0x8A31 | 0x8A33 if v2 => n(1024 + 12 * 4096),
            0x8A34 if v2 => n(256),
            0x8C8A | 0x8C8B | 0x8C80 if v2 => n(4),
            0x9247 if v2 => n(0),
            // EXT_texture_filter_anisotropic
            0x84FF => n(16),
            _ => {
                gl.error(INVALID_ENUM);
                P::Null
            }
        }
    });
    Ok(match r {
        Some(P::N(x)) => JsValue::from(x),
        Some(P::B(b)) => JsValue::from(b),
        Some(P::S(s)) => JsValue::from(js_string!(s)),
        Some(P::F(v)) => JsFloat32Array::from_iter(v, ctx)?.into(),
        Some(P::I(v)) => JsInt32Array::from_iter(v, ctx)?.into(),
        Some(P::U(v)) => JsUint32Array::from_iter(v, ctx)?.into(),
        Some(P::Bs(v)) => jarr(v.into_iter().map(JsValue::from), ctx),
        _ => JsValue::null(),
    })
}

/// __glReadPixels(cid, x, y, w, h, format, type, view, dstOffset)
fn n_read_pixels(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (x, y, w, h) = (int(a, 1, ctx), int(a, 2, ctx), int(a, 3, ctx), int(a, 4, ctx));
    let (format, ty) = (u(a, 5, ctx), u(a, 6, ctx));
    let doff = u(a, 8, ctx) as usize;
    let existing = view_bytes(a.get(7), ctx);
    let bytes = with_gl(a, ctx, |gl| -> Option<Vec<u8>> {
        if w < 0 || h < 0 {
            gl.error(INVALID_VALUE);
            return None;
        }
        let Some(mut dst) = existing else {
            gl.error(INVALID_VALUE);
            return None;
        };
        let (pixels, fmt) = gl.read_rect(x, y, w, h)?;
        let int = fmt.is_integer();
        let float = matches!(fmt.num, Num::Float | Num::Half);
        // Allowed: RGBA/UNSIGNED_BYTE (normalized), RGBA/FLOAT (float), RGBA_INTEGER/INT|UINT (integer).
        let ok = match (format, ty) {
            (e::RGBA, e::UNSIGNED_BYTE) => !int,
            (e::RGBA, e::FLOAT) => float || !int && gl.v2 && false,
            (e::RGBA_INTEGER, e::INT) => int && fmt.num == Num::Int,
            (e::RGBA_INTEGER, e::UNSIGNED_INT) => int && fmt.num == Num::Uint,
            (e::RGB, e::UNSIGNED_BYTE) | (e::ALPHA, e::UNSIGNED_BYTE) => false,
            _ => false,
        };
        let known = matches!(format, e::RGBA | e::RGB | e::ALPHA | e::RGBA_INTEGER | e::RED | e::RG | e::LUMINANCE | e::LUMINANCE_ALPHA);
        if !known {
            gl.error(INVALID_ENUM);
            return None;
        }
        if !ok {
            gl.error(INVALID_OPERATION);
            return None;
        }
        let bpp = if ty == e::UNSIGNED_BYTE { 4 } else { 16 };
        let row_px = if gl.pack_row_length > 0 { gl.pack_row_length } else { w as usize };
        let row = (row_px * bpp).div_ceil(gl.pack_alignment) * gl.pack_alignment;
        let start = doff + gl.pack_skip_rows * row + gl.pack_skip_pixels * bpp;
        let need = if w == 0 || h == 0 { 0 } else { start + (h as usize - 1) * row + w as usize * bpp };
        if need > dst.len() {
            gl.error(INVALID_OPERATION);
            return None;
        }
        let mut rgba8 = Vec::new();
        if ty == e::UNSIGNED_BYTE {
            for p in &pixels {
                let v = p.unwrap_or([0.0; 4]);
                let a = if fmt.layout == e::RGB { 1.0 } else { v[3] };
                rgba8.extend_from_slice(&[v[0], v[1], v[2], a].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
            }
            crate::security::noise_rgba(gl.seed, &mut rgba8);
        }
        for j in 0..h as usize {
            for i in 0..w as usize {
                let k = j * w as usize + i;
                if pixels[k].is_none() {
                    continue;
                }
                let o = start + j * row + i * bpp;
                if ty == e::UNSIGNED_BYTE {
                    dst[o..o + 4].copy_from_slice(&rgba8[k * 4..k * 4 + 4]);
                } else {
                    let v = pixels[k].unwrap();
                    for c in 0..4 {
                        dst[o + c * 4..o + c * 4 + 4].copy_from_slice(&v[c].to_bits().to_le_bytes());
                    }
                }
            }
        }
        Some(dst)
    })
    .flatten();
    if let Some(b) = bytes {
        view_write(a.get(7), 0, &b, ctx);
    }
    undef()
}

fn n_bind_framebuffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, obj) = (u(a, 1, ctx), u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        if obj != 0 && gl.fbo(obj).is_none() {
            return gl.error(INVALID_OPERATION);
        }
        match target {
            FRAMEBUFFER => {
                gl.draw_fbo = obj;
                gl.read_fbo = obj;
            }
            DRAW_FRAMEBUFFER if gl.v2 => gl.draw_fbo = obj,
            READ_FRAMEBUFFER if gl.v2 => gl.read_fbo = obj,
            _ => gl.error(INVALID_ENUM),
        }
    });
    undef()
}

fn fbo_for_target(gl: &mut Gl, target: u32) -> Option<u32> {
    match target {
        FRAMEBUFFER | DRAW_FRAMEBUFFER if target == FRAMEBUFFER || gl.v2 => Some(gl.draw_fbo),
        READ_FRAMEBUFFER if gl.v2 => Some(gl.read_fbo),
        _ => {
            gl.error(INVALID_ENUM);
            None
        }
    }
}

fn attach(gl: &mut Gl, target: u32, attachment: u32, att: Option<Att>) {
    let Some(fid) = fbo_for_target(gl, target) else { return };
    if fid == 0 {
        return gl.error(INVALID_OPERATION);
    }
    let max = if gl.v2 { MAX_COLOR_ATTACHMENTS as u32 } else { 4 };
    let Some(f) = gl.fbo_mut(fid) else { return };
    match attachment {
        x if x >= COLOR_ATTACHMENT0 && x < COLOR_ATTACHMENT0 + max => f.color[(x - COLOR_ATTACHMENT0) as usize] = att,
        DEPTH_ATTACHMENT => {
            f.depth = att;
            f.depth_stencil = false;
        }
        STENCIL_ATTACHMENT => {
            f.stencil = att;
            f.depth_stencil = false;
        }
        DEPTH_STENCIL_ATTACHMENT => {
            f.depth = att;
            f.stencil = att;
            f.depth_stencil = true;
        }
        _ => gl.error(INVALID_ENUM),
    }
}

/// __glFramebufferTexture(cid, target, attachment, textarget, texture, level, layer)
fn n_framebuffer_texture(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, attachment, textarget, tex_id, level, layer) = (u(a, 1, ctx), u(a, 2, ctx), u(a, 3, ctx), u(a, 4, ctx), int(a, 5, ctx), int(a, 6, ctx));
    with_gl(a, ctx, |gl| {
        if tex_id == 0 {
            return attach(gl, target, attachment, None);
        }
        let Some(t) = gl.texture(tex_id) else { return gl.error(INVALID_OPERATION) };
        let face = if textarget == 0 {
            // framebufferTextureLayer: 3D or array textures.
            if t.target != e::TEXTURE_3D && t.target != e::TEXTURE_2D_ARRAY {
                return gl.error(INVALID_OPERATION);
            }
            0
        } else if textarget == e::TEXTURE_2D {
            if t.target != e::TEXTURE_2D {
                return gl.error(INVALID_OPERATION);
            }
            0
        } else if (e::TEXTURE_CUBE_MAP_POSITIVE_X..e::TEXTURE_CUBE_MAP_POSITIVE_X + 6).contains(&textarget) {
            if t.target != e::TEXTURE_CUBE_MAP {
                return gl.error(INVALID_OPERATION);
            }
            textarget - e::TEXTURE_CUBE_MAP_POSITIVE_X
        } else {
            return gl.error(INVALID_ENUM);
        };
        if level < 0 || (!gl.v2 && level != 0) {
            return gl.error(INVALID_VALUE);
        }
        attach(gl, target, attachment, Some(Att::Tex { id: tex_id, level: level as u32, face, layer: layer.max(0) as u32 }));
    });
    undef()
}

fn n_framebuffer_renderbuffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, attachment, rb_target, rbo) = (u(a, 1, ctx), u(a, 2, ctx), u(a, 3, ctx), u(a, 4, ctx));
    with_gl(a, ctx, |gl| {
        if rb_target != RENDERBUFFER {
            return gl.error(INVALID_ENUM);
        }
        if rbo != 0 && !matches!(gl.objs.get(&rbo), Some(Obj::Rbo(_))) {
            return gl.error(INVALID_OPERATION);
        }
        attach(gl, target, attachment, if rbo == 0 { None } else { Some(Att::Rbo(rbo)) });
    });
    undef()
}

fn n_check_framebuffer_status(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = u(a, 1, ctx);
    let r = with_gl(a, ctx, |gl| match fbo_for_target(gl, target) {
        Some(f) => gl.fbo_status(f),
        None => 0,
    })
    .unwrap_or(0);
    Ok(JsValue::from(r))
}

/// __glGetFramebufferAttachmentParam(cid, target, attachment, pname) → number or [kind, id]
fn n_get_fb_attachment_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, attachment, pname) = (u(a, 1, ctx), u(a, 2, ctx), u(a, 3, ctx));
    enum Out {
        N(f64),
        Obj(u32, u32),
        Null,
    }
    let r = with_gl(a, ctx, |gl| {
        let Some(fid) = fbo_for_target(gl, target) else { return Out::Null };
        let v2 = gl.v2;
        if fid == 0 {
            if !v2 {
                gl.error(INVALID_OPERATION);
                return Out::Null;
            }
            return match pname {
                0x8CD0 => Out::N(0x8218 as f64),
                0x8212..=0x8217 => Out::N(8.0),
                _ => Out::Null,
            };
        }
        let Some(f) = gl.fbo(fid) else { return Out::Null };
        let att = match attachment {
            x if (COLOR_ATTACHMENT0..COLOR_ATTACHMENT0 + MAX_COLOR_ATTACHMENTS as u32).contains(&x) => f.color[(x - COLOR_ATTACHMENT0) as usize],
            DEPTH_ATTACHMENT => f.depth,
            STENCIL_ATTACHMENT => f.stencil,
            DEPTH_STENCIL_ATTACHMENT => {
                if f.depth != f.stencil {
                    gl.error(INVALID_OPERATION);
                    return Out::Null;
                }
                f.depth
            }
            _ => {
                gl.error(INVALID_ENUM);
                return Out::Null;
            }
        };
        let Some(att) = att else {
            return match pname {
                0x8CD0 => Out::N(0.0),
                _ => {
                    gl.error(if pname == 0x8CD1 { INVALID_ENUM } else { INVALID_ENUM });
                    Out::Null
                }
            };
        };
        let img = gl.att_image(&att).map(|i| i.fmt);
        match (pname, att) {
            (0x8CD0, Att::Tex { .. }) => Out::N(0x1702 as f64),
            (0x8CD0, Att::Rbo(_)) => Out::N(RENDERBUFFER as f64),
            (0x8CD1, Att::Tex { id, .. }) => Out::Obj(1, id),
            (0x8CD1, Att::Rbo(id)) => Out::Obj(3, id),
            (0x8CD2, Att::Tex { level, .. }) => Out::N(level as f64),
            (0x8CD3, Att::Tex { face, id, .. }) => {
                let cube = gl.texture(id).is_some_and(|t| t.target == e::TEXTURE_CUBE_MAP);
                Out::N(if cube { (e::TEXTURE_CUBE_MAP_POSITIVE_X + face) as f64 } else { 0.0 })
            }
            (0x8CD4, Att::Tex { layer, .. }) if v2 => Out::N(layer as f64),
            (0x8212..=0x8217, _) if v2 => Out::N(img.map_or(0.0, |f| f.bits[(pname - 0x8212) as usize] as f64)),
            (0x8211, _) if v2 => Out::N(match img.map(|f| f.num) {
                Some(Num::Float) | Some(Num::Half) => e::FLOAT as f64,
                Some(Num::Int) => e::INT as f64,
                Some(Num::Uint) => e::UNSIGNED_INT as f64,
                Some(Num::Snorm) => 0x8F9C as f64,
                _ => 0x8C17 as f64,
            }),
            (0x8210, _) if v2 => Out::N(if img.is_some_and(|f| f.internal == e::SRGB8_ALPHA8) { 0x8C40 as f64 } else { 0x2601 as f64 }),
            _ => {
                gl.error(INVALID_ENUM);
                Out::Null
            }
        }
    });
    Ok(match r {
        Some(Out::N(n)) => JsValue::from(n),
        Some(Out::Obj(k, id)) => jarr([JsValue::from(k), JsValue::from(id)], ctx),
        _ => JsValue::null(),
    })
}

fn n_bind_renderbuffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, obj) = (u(a, 1, ctx), u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        if target != RENDERBUFFER {
            return gl.error(INVALID_ENUM);
        }
        if obj != 0 && !matches!(gl.objs.get(&obj), Some(Obj::Rbo(_))) {
            return gl.error(INVALID_OPERATION);
        }
        gl.rbo = obj;
    });
    undef()
}

/// __glRenderbufferStorage(cid, target, samples, internal, w, h)
fn n_renderbuffer_storage(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, samples, internal, w, h) = (u(a, 1, ctx), int(a, 2, ctx), u(a, 3, ctx), int(a, 4, ctx), int(a, 5, ctx));
    with_gl(a, ctx, |gl| {
        if target != RENDERBUFFER {
            return gl.error(INVALID_ENUM);
        }
        if gl.rbo == 0 {
            return gl.error(INVALID_OPERATION);
        }
        if w < 0 || h < 0 || w as u32 > MAX_RENDERBUFFER_SIZE || h as u32 > MAX_RENDERBUFFER_SIZE || samples < 0 {
            return gl.error(INVALID_VALUE);
        }
        let v2 = gl.v2;
        let fmt = match internal {
            e::RGBA4 | e::RGB565 | e::RGB5_A1 | e::DEPTH_COMPONENT16 | e::STENCIL_INDEX8 | DEPTH_STENCIL => tex::sized_format(internal, v2),
            // EXT_color_buffer_float / half float and sRGB on WebGL 1.
            _ => tex::sized_format(internal, v2).filter(|f| v2 || matches!(f.internal, e::RGBA32F | e::RGBA16F | e::RGB16F | e::SRGB8_ALPHA8)),
        };
        let Some(fmt) = fmt else { return gl.error(INVALID_ENUM) };
        if v2 && fmt.is_integer() && samples > 0 {
            return gl.error(INVALID_OPERATION);
        }
        let img = Image::new(w as u32, h as u32, 1, fmt, v2);
        let id = gl.rbo;
        if let Some(Obj::Rbo(r)) = gl.objs.get_mut(&id) {
            r.img = Some(img);
            r.samples = samples as u32;
            r.internal = internal;
        }
    });
    undef()
}

fn n_get_renderbuffer_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, pname) = (u(a, 1, ctx), u(a, 2, ctx));
    let r = with_gl(a, ctx, |gl| {
        if target != RENDERBUFFER {
            gl.error(INVALID_ENUM);
            return None;
        }
        let Some(Obj::Rbo(r)) = gl.objs.get(&gl.rbo) else {
            gl.error(INVALID_OPERATION);
            return None;
        };
        let img = r.img.as_ref();
        let bits = |k: usize| img.map_or(0, |i| i.fmt.bits[k] as u32);
        Some(match pname {
            0x8D42 => img.map_or(0, |i| i.w),
            0x8D43 => img.map_or(0, |i| i.h),
            0x8D44 => if r.internal == 0 { e::RGBA4 } else { r.internal },
            0x8D50 => bits(0),
            0x8D51 => bits(1),
            0x8D52 => bits(2),
            0x8D53 => bits(3),
            0x8D54 => bits(4),
            0x8D55 => bits(5),
            0x8CAB if gl.v2 => r.samples,
            _ => {
                gl.error(INVALID_ENUM);
                return None;
            }
        })
    })
    .flatten();
    Ok(r.map_or(JsValue::null(), JsValue::from))
}

fn n_draw_buffers(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let list: Vec<u32> = a
        .get(1)
        .and_then(|v| v.as_object())
        .and_then(|o| JsArray::from_object(o.clone()).ok())
        .map(|arr| {
            let n = arr.length(ctx).unwrap_or(0);
            (0..n).map(|i| arr.get(i, ctx).ok().and_then(|v| v.to_number(ctx).ok()).unwrap_or(0.0) as u32).collect()
        })
        .unwrap_or_default();
    with_gl(a, ctx, |gl| {
        if gl.draw_fbo == 0 {
            if list.len() != 1 || !(list[0] == BACK || list[0] == 0) {
                return gl.error(INVALID_OPERATION);
            }
            gl.fb_draw_buffer = list[0];
            return;
        }
        if list.len() > MAX_COLOR_ATTACHMENTS {
            return gl.error(INVALID_VALUE);
        }
        for (i, b) in list.iter().enumerate() {
            if *b != 0 && *b != COLOR_ATTACHMENT0 + i as u32 {
                return gl.error(INVALID_OPERATION);
            }
        }
        let f = gl.draw_fbo;
        if let Some(fb) = gl.fbo_mut(f) {
            let mut db = list;
            db.resize(MAX_COLOR_ATTACHMENTS, 0);
            fb.draw_buffers = db;
        }
    });
    undef()
}

fn n_read_buffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let src = u(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        if gl.read_fbo == 0 {
            if src != BACK && src != 0 {
                return gl.error(INVALID_OPERATION);
            }
            gl.fb_read_buffer = src;
            return;
        }
        if src != 0 && !(COLOR_ATTACHMENT0..COLOR_ATTACHMENT0 + MAX_COLOR_ATTACHMENTS as u32).contains(&src) {
            return gl.error(INVALID_OPERATION);
        }
        let f = gl.read_fbo;
        if let Some(fb) = gl.fbo_mut(f) {
            fb.read_buffer = src;
        }
    });
    undef()
}

/// __glBlitFramebuffer(cid, sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask, filter)
fn n_blit_framebuffer(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let r: Vec<i32> = (1..9).map(|i| int(a, i, ctx)).collect();
    let (mask, filter) = (u(a, 9, ctx), u(a, 10, ctx));
    with_gl(a, ctx, |gl| {
        if mask & COLOR_BUFFER_BIT == 0 {
            // Depth/stencil blits aren't needed by pages in practice; accept them as no-ops.
            return;
        }
        let (sw, sh) = (r[2] - r[0], r[3] - r[1]);
        let (dw, dh) = (r[6] - r[4], r[7] - r[5]);
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
            return;
        }
        let (sx, sy) = (r[0].min(r[2]), r[1].min(r[3]));
        let Some((src, _)) = gl.read_rect(sx, sy, sw.abs(), sh.abs()) else { return };
        let Some((mut t, restore, d, s)) = gl.take_target(false) else { return };
        let st = RState { blend: false, scissor: gl.cap(SCISSOR_TEST).then_some(gl.scissor), ..gl.rstate() };
        let (dx0, dy0) = (r[4].min(r[6]), r[5].min(r[7]));
        let (fx, fy) = (sw < 0, sh < 0);
        let (gx, gy) = (dw < 0, dh < 0);
        let bounds = clip_rect(&st, t.w, t.h);
        for img in t.color.iter_mut().flatten() {
            for y in 0..dh.abs() {
                for x in 0..dw.abs() {
                    let (px, py) = (dx0 + x, dy0 + y);
                    if px < bounds[0] || py < bounds[1] || px >= bounds[2] || py >= bounds[3] {
                        continue;
                    }
                    let mut u0 = ((x as f32 + 0.5) * sw.abs() as f32 / dw.abs() as f32) as i32;
                    let mut v0 = ((y as f32 + 0.5) * sh.abs() as f32 / dh.abs() as f32) as i32;
                    if fx != gx {
                        u0 = sw.abs() - 1 - u0;
                    }
                    if fy != gy {
                        v0 = sh.abs() - 1 - v0;
                    }
                    let _ = filter;
                    if let Some(Some(p)) = src.get((v0 * sw.abs() + u0) as usize) {
                        let i = (py as u32 * img.w + px as u32) as usize;
                        crate::webgl_raster::write_color(img, i, *p, &st);
                    }
                }
            }
        }
        gl.put_target(t, restore, d, s, false);
        gl.dirty = true;
    });
    undef()
}

fn n_bind_sampler(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (unit, s) = (u(a, 1, ctx) as usize, u(a, 2, ctx));
    with_gl(a, ctx, |gl| {
        if unit >= MAX_UNITS {
            return gl.error(INVALID_VALUE);
        }
        gl.samplers[unit] = s;
    });
    undef()
}

fn n_sampler_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (s, pname, v) = (u(a, 1, ctx), u(a, 2, ctx), fl(a, 3, ctx));
    with_gl(a, ctx, |gl| {
        let Some(Obj::Sampler(p)) = gl.objs.get_mut(&s) else { return gl.error(INVALID_OPERATION) };
        if matches!(pname, 0x813C | 0x813D) {
            return gl.error(INVALID_ENUM);
        }
        if let Err(e) = set_param(p, pname, v, true) {
            gl.error(e);
        }
    });
    undef()
}

fn n_get_sampler_param(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (s, pname) = (u(a, 1, ctx), u(a, 2, ctx));
    let r = with_gl(a, ctx, |gl| {
        let Some(Obj::Sampler(p)) = gl.objs.get(&s) else {
            gl.error(INVALID_OPERATION);
            return None;
        };
        let v = get_param(p, pname);
        if v.is_none() {
            gl.error(INVALID_ENUM);
        }
        v
    })
    .flatten();
    Ok(r.unwrap_or(JsValue::null()))
}

fn n_enable_ext(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = string(a, 1, ctx);
    with_gl(a, ctx, |gl| {
        if name.starts_with("WEBGL_compressed_texture_s3tc") {
            gl.s3tc = true;
        }
    });
    undef()
}

/// __glCompressedTexImage(cid, target, level, internal, w, h, d, border, data, sub, [x, y, z])
fn n_compressed_tex_image(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (target, level, internal) = (u(a, 1, ctx), int(a, 2, ctx), u(a, 3, ctx));
    let (w, h, d, border) = (int(a, 4, ctx), int(a, 5, ctx), int(a, 6, ctx).max(1), int(a, 7, ctx));
    let data = view_bytes(a.get(8), ctx);
    let sub = boolean(a, 9);
    let offs: Vec<i32> = a
        .get(10)
        .and_then(|v| v.as_object())
        .and_then(|o| JsArray::from_object(o.clone()).ok())
        .map(|arr| (0..3).map(|i| arr.get(i, ctx).ok().and_then(|v| v.to_number(ctx).ok()).unwrap_or(0.0) as i32).collect())
        .unwrap_or(vec![0, 0, 0]);
    with_gl(a, ctx, |gl| {
        let Some((fmt, _)) = tex::compressed_format(internal).filter(|_| gl.s3tc) else { return gl.error(INVALID_ENUM) };
        if level < 0 || w < 0 || h < 0 || border != 0 || !size_ok(target, level, w, h, 1) {
            return gl.error(INVALID_VALUE);
        }
        let Some(data) = data else { return gl.error(INVALID_VALUE) };
        let Some((id, face)) = gl.image_target(target) else { return };
        let v2 = gl.v2;
        let _ = v2;
        if tex::compressed_size(internal, w as u32, h as u32) != Some(data.len() / d as usize) {
            return gl.error(INVALID_VALUE);
        }
        let Some(px) = tex::decode_s3tc(&data, w as u32, h as u32, internal) else { return gl.error(INVALID_VALUE) };
        if sub {
            let Some(t) = gl.texture_mut(id) else { return };
            let Some(Some(img)) = t.faces.get_mut(face).and_then(|f| f.get_mut(level as usize)) else { return gl.error(INVALID_OPERATION) };
            let (x, y) = (offs[0], offs[1]);
            if img.fmt.internal != internal || x % 4 != 0 || y % 4 != 0 || (w % 4 != 0 && x + w != img.w as i32) || (h % 4 != 0 && y + h != img.h as i32) {
                return gl.error(INVALID_OPERATION);
            }
            if x < 0 || y < 0 || x + w > img.w as i32 || y + h > img.h as i32 {
                return gl.error(INVALID_VALUE);
            }
            for yy in 0..h {
                for xx in 0..w {
                    img.px[((y + yy) as u32 * img.w + (x + xx) as u32) as usize] = px[(yy * w + xx) as usize];
                }
            }
            return;
        }
        if let Some(t) = gl.texture_mut(id) {
            if t.immutable {
                return gl.error(INVALID_OPERATION);
            }
            t.set_level(face, level as usize, Image { w: w as u32, h: h as u32, d: 1, fmt, px, st: Vec::new() });
        }
    });
    undef()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gl() -> Gl {
        Gl::new(8, 8, Attrs { alpha: true, depth: true, stencil: true, antialias: false, premultiplied: true, preserve: true }, false, 0)
    }

    fn program(gl: &mut Gl, vs: &str, fs: &str) -> u32 {
        let mk = |gl: &mut Gl, ty: u32, src: &str| {
            let stage = if ty == VERTEX_SHADER { Stage::Vertex } else { Stage::Fragment };
            let c = compile(src, stage, gl.v2).unwrap_or_else(|e| panic!("{e}"));
            gl.new_id(Obj::Shader(Shader { ty, src: src.into(), compiled: Some(Rc::new(c)), log: String::new(), deleted: false }))
        };
        let v = mk(gl, VERTEX_SHADER, vs);
        let f = mk(gl, FRAGMENT_SHADER, fs);
        let p = gl.new_id(Obj::Program(Program { shaders: vec![v, f], linked: None, link_ok: false, log: String::new(), bound_attribs: HashMap::new(), deleted: false, validate: false }));
        gl.link(p);
        assert!(gl.program_ref(p).unwrap().link_ok, "{}", gl.program_ref(p).unwrap().log);
        gl.program = p;
        p
    }

    fn buffer(gl: &mut Gl, floats: &[f32]) -> u32 {
        let data = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        gl.new_id(Obj::Buffer(Buffer { data, usage: 0x88E4, kind: 2 }))
    }

    #[test]
    fn clears_and_draws_a_triangle() {
        let mut gl = gl();
        gl.clear_color = [0.0, 0.0, 1.0, 1.0];
        gl.clear(COLOR_BUFFER_BIT);
        assert_eq!(gl.fb_color.px[0], [0.0, 0.0, 1.0, 1.0]);
        program(&mut gl, "attribute vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); }", "precision mediump float; void main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }");
        // A triangle covering the lower-left half.
        let b = buffer(&mut gl, &[-1.0, -1.0, 1.0, -1.0, -1.0, 1.0]);
        let vao = gl.vao_mut();
        vao.attribs[0] = VAttr { enabled: true, buffer: b, size: 2, ..VAttr::default() };
        gl.draw(g::TRIANGLES, 0, 3, None, 1);
        assert_eq!(gl.err, 0);
        assert_eq!(gl.fb_color.px[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(gl.fb_color.px[63], [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn varyings_textures_and_depth() {
        let mut gl = gl();
        program(
            &mut gl,
            "attribute vec2 p; varying vec2 uv; void main() { uv = p * 0.5 + 0.5; gl_Position = vec4(p, 0.5, 1.0); }",
            "precision mediump float; varying vec2 uv; uniform sampler2D t; void main() { gl_FragColor = texture2D(t, uv); }",
        );
        // A 2×1 texture: red | green, nearest filtering.
        let fmt = tex::texture_format(e::RGBA, e::RGBA, e::UNSIGNED_BYTE, false).unwrap();
        let mut t = Texture::new();
        t.target = e::TEXTURE_2D;
        t.p.min = e::NEAREST;
        t.p.mag = e::NEAREST;
        t.p.wrap_s = e::CLAMP_TO_EDGE;
        t.p.wrap_t = e::CLAMP_TO_EDGE;
        t.set_level(0, 0, Image { w: 2, h: 1, d: 1, fmt, px: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]], st: Vec::new() });
        let tid = gl.new_id(Obj::Texture(t));
        gl.units[0][0] = tid;
        let b = buffer(&mut gl, &[-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]);
        gl.vao_mut().attribs[0] = VAttr { enabled: true, buffer: b, size: 2, ..VAttr::default() };
        gl.caps.insert(DEPTH_TEST, true);
        gl.draw(g::TRIANGLES, 0, 3, None, 1);
        assert_eq!(gl.err, 0);
        assert_eq!(gl.fb_color.px[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(gl.fb_color.px[7], [0.0, 1.0, 0.0, 1.0]);
        assert!((gl.fb_depth.as_ref().unwrap()[0] - 0.75).abs() < 1e-6);
    }
}
