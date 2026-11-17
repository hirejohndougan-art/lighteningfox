//! WebGL images: pixel formats, textures and renderbuffers, uploads from JavaScript data,
//! and texture sampling (filtering, wrapping, mipmaps, cube maps, depth compare).
//!
//! Every image is stored as RGBA `f32` texels in GL row order (row 0 at the bottom), already
//! expanded the way sampling reads them (LUMINANCE → L,L,L,1, RGB → r,g,b,1 …). Integer
//! formats keep their integers' bits in the floats.

use crate::glsl::{Base, Dim, Samp};
use crate::glsl_vm::{f32_from_half, TexArgs, TexEnv};

pub mod e {
    pub const NONE: u32 = 0;
    pub const BYTE: u32 = 0x1400;
    pub const UNSIGNED_BYTE: u32 = 0x1401;
    pub const SHORT: u32 = 0x1402;
    pub const UNSIGNED_SHORT: u32 = 0x1403;
    pub const INT: u32 = 0x1404;
    pub const UNSIGNED_INT: u32 = 0x1405;
    pub const FLOAT: u32 = 0x1406;
    pub const HALF_FLOAT: u32 = 0x140B;
    pub const HALF_FLOAT_OES: u32 = 0x8D61;
    pub const UNSIGNED_SHORT_4_4_4_4: u32 = 0x8033;
    pub const UNSIGNED_SHORT_5_5_5_1: u32 = 0x8034;
    pub const UNSIGNED_SHORT_5_6_5: u32 = 0x8363;
    pub const UNSIGNED_INT_24_8: u32 = 0x84FA;
    pub const UNSIGNED_INT_2_10_10_10_REV: u32 = 0x8368;
    pub const UNSIGNED_INT_10F_11F_11F_REV: u32 = 0x8C3B;
    pub const UNSIGNED_INT_5_9_9_9_REV: u32 = 0x8C3E;
    pub const FLOAT_32_UNSIGNED_INT_24_8_REV: u32 = 0x8DAD;
    pub const DEPTH_COMPONENT: u32 = 0x1902;
    pub const RED: u32 = 0x1903;
    pub const ALPHA: u32 = 0x1906;
    pub const RGB: u32 = 0x1907;
    pub const RGBA: u32 = 0x1908;
    pub const LUMINANCE: u32 = 0x1909;
    pub const LUMINANCE_ALPHA: u32 = 0x190A;
    pub const RG: u32 = 0x8227;
    pub const RG_INTEGER: u32 = 0x8228;
    pub const RED_INTEGER: u32 = 0x8D94;
    pub const RGB_INTEGER: u32 = 0x8D98;
    pub const RGBA_INTEGER: u32 = 0x8D99;
    pub const DEPTH_STENCIL: u32 = 0x84F9;
    pub const STENCIL_INDEX8: u32 = 0x8D48;
    pub const RGBA4: u32 = 0x8056;
    pub const RGB5_A1: u32 = 0x8057;
    pub const RGB565: u32 = 0x8D62;
    pub const RGB8: u32 = 0x8051;
    pub const RGBA8: u32 = 0x8058;
    pub const RGB10_A2: u32 = 0x8059;
    pub const RGB10_A2UI: u32 = 0x906F;
    pub const SRGB8: u32 = 0x8C41;
    pub const SRGB8_ALPHA8: u32 = 0x8C43;
    pub const R8: u32 = 0x8229;
    pub const RG8: u32 = 0x822B;
    pub const R16F: u32 = 0x822D;
    pub const R32F: u32 = 0x822E;
    pub const RG16F: u32 = 0x822F;
    pub const RG32F: u32 = 0x8230;
    pub const R8I: u32 = 0x8231;
    pub const R8UI: u32 = 0x8232;
    pub const R16I: u32 = 0x8233;
    pub const R16UI: u32 = 0x8234;
    pub const R32I: u32 = 0x8235;
    pub const R32UI: u32 = 0x8236;
    pub const RG8I: u32 = 0x8237;
    pub const RG8UI: u32 = 0x8238;
    pub const RG16I: u32 = 0x8239;
    pub const RG16UI: u32 = 0x823A;
    pub const RG32I: u32 = 0x823B;
    pub const RG32UI: u32 = 0x823C;
    pub const RGBA32F: u32 = 0x8814;
    pub const RGB32F: u32 = 0x8815;
    pub const RGBA16F: u32 = 0x881A;
    pub const RGB16F: u32 = 0x881B;
    pub const R11F_G11F_B10F: u32 = 0x8C3A;
    pub const RGB9_E5: u32 = 0x8C3D;
    pub const RGBA32UI: u32 = 0x8D70;
    pub const RGB32UI: u32 = 0x8D71;
    pub const RGBA16UI: u32 = 0x8D76;
    pub const RGB16UI: u32 = 0x8D77;
    pub const RGBA8UI: u32 = 0x8D7C;
    pub const RGB8UI: u32 = 0x8D7D;
    pub const RGBA32I: u32 = 0x8D82;
    pub const RGB32I: u32 = 0x8D83;
    pub const RGBA16I: u32 = 0x8D88;
    pub const RGB16I: u32 = 0x8D89;
    pub const RGBA8I: u32 = 0x8D8E;
    pub const RGB8I: u32 = 0x8D8F;
    pub const R8_SNORM: u32 = 0x8F94;
    pub const RG8_SNORM: u32 = 0x8F95;
    pub const RGB8_SNORM: u32 = 0x8F96;
    pub const RGBA8_SNORM: u32 = 0x8F97;
    pub const DEPTH_COMPONENT16: u32 = 0x81A5;
    pub const DEPTH_COMPONENT24: u32 = 0x81A6;
    pub const DEPTH_COMPONENT32F: u32 = 0x8CAC;
    pub const DEPTH24_STENCIL8: u32 = 0x88F0;
    pub const DEPTH32F_STENCIL8: u32 = 0x8CAD;
    pub const SRGB_EXT: u32 = 0x8C40;
    pub const SRGB_ALPHA_EXT: u32 = 0x8C42;

    pub const NEAREST: u32 = 0x2600;
    pub const LINEAR: u32 = 0x2601;
    pub const NEAREST_MIPMAP_NEAREST: u32 = 0x2700;
    pub const LINEAR_MIPMAP_NEAREST: u32 = 0x2701;
    pub const NEAREST_MIPMAP_LINEAR: u32 = 0x2702;
    pub const LINEAR_MIPMAP_LINEAR: u32 = 0x2703;
    pub const REPEAT: u32 = 0x2901;
    pub const CLAMP_TO_EDGE: u32 = 0x812F;
    pub const MIRRORED_REPEAT: u32 = 0x8370;
    pub const COMPARE_REF_TO_TEXTURE: u32 = 0x884E;
    pub const NEVER: u32 = 0x200;
    pub const LESS: u32 = 0x201;
    pub const EQUAL: u32 = 0x202;
    pub const LEQUAL: u32 = 0x203;
    pub const GREATER: u32 = 0x204;
    pub const NOTEQUAL: u32 = 0x205;
    pub const GEQUAL: u32 = 0x206;
    pub const ALWAYS: u32 = 0x207;

    pub const TEXTURE_2D: u32 = 0x0DE1;
    pub const TEXTURE_CUBE_MAP: u32 = 0x8513;
    pub const TEXTURE_CUBE_MAP_POSITIVE_X: u32 = 0x8515;
    pub const TEXTURE_3D: u32 = 0x806F;
    pub const TEXTURE_2D_ARRAY: u32 = 0x8C1A;
}

/// How texels are stored and converted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Num {
    /// Normalized to [0, 1] with this many bits per channel (0 = the bits vary: 565 etc.).
    Unorm(u8),
    Snorm,
    Float,
    Half,
    Int,
    Uint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fmt {
    /// The internal format as WebGL reports it.
    pub internal: u32,
    /// The unsized format (RGBA, RGB, RED, LUMINANCE …) that decides channel layout.
    pub layout: u32,
    pub num: Num,
    pub depth: bool,
    pub stencil: bool,
    /// Bits per channel for getRenderbufferParameter / getParameter(RED_BITS…): r g b a d s.
    pub bits: [u8; 6],
}

impl Fmt {
    pub fn is_integer(&self) -> bool {
        matches!(self.num, Num::Int | Num::Uint)
    }
    pub fn color_renderable(&self, webgl2: bool) -> bool {
        if self.depth || self.stencil {
            return false;
        }
        if matches!(self.layout, e::LUMINANCE | e::ALPHA | e::LUMINANCE_ALPHA) {
            return false;
        }
        if !webgl2 {
            return matches!(self.layout, e::RGBA | e::RGB) || self.internal == e::RGBA4 || self.internal == e::RGB5_A1 || self.internal == e::RGB565;
        }
        !matches!(self.num, Num::Snorm) && self.internal != e::RGB9_E5
    }
    pub fn filterable(&self) -> bool {
        !self.is_integer()
    }
}

/// Look up the storage format for `texImage` arguments (internal format, format, type).
pub fn texture_format(internal: u32, format: u32, ty: u32, webgl2: bool) -> Option<Fmt> {
    use e::*;
    let f = |internal, layout, num, bits: [u8; 6]| Some(Fmt { internal, layout, num, depth: false, stencil: false, bits });
    // Unsized (WebGL 1 style): internal format must equal format.
    if matches!(internal, RGBA | RGB | LUMINANCE | ALPHA | LUMINANCE_ALPHA | DEPTH_COMPONENT | DEPTH_STENCIL | SRGB_EXT | SRGB_ALPHA_EXT) {
        if internal != format && !(internal == SRGB_EXT && format == RGB) && !(internal == SRGB_ALPHA_EXT && format == RGBA) {
            return None;
        }
        let layout = match internal {
            SRGB_EXT => RGB,
            SRGB_ALPHA_EXT => RGBA,
            x => x,
        };
        let num = match ty {
            UNSIGNED_BYTE => Num::Unorm(8),
            UNSIGNED_SHORT_5_6_5 if layout == RGB => Num::Unorm(0),
            UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 if layout == RGBA => Num::Unorm(0),
            FLOAT => Num::Float,
            HALF_FLOAT_OES | HALF_FLOAT => Num::Half,
            UNSIGNED_SHORT | UNSIGNED_INT if layout == DEPTH_COMPONENT => Num::Unorm(0),
            UNSIGNED_INT_24_8 if layout == DEPTH_STENCIL => Num::Unorm(0),
            _ => return None,
        };
        if layout == DEPTH_COMPONENT || layout == DEPTH_STENCIL {
            let d = if ty == UNSIGNED_SHORT { 16 } else { 24 };
            return Some(Fmt { internal, layout, num: Num::Unorm(0), depth: true, stencil: layout == DEPTH_STENCIL, bits: [0, 0, 0, 0, d, if layout == DEPTH_STENCIL { 8 } else { 0 }] });
        }
        let b = match (num, ty) {
            (Num::Float, _) => 32,
            (Num::Half, _) => 16,
            (_, UNSIGNED_SHORT_5_6_5) => 5,
            (_, UNSIGNED_SHORT_4_4_4_4) => 4,
            (_, UNSIGNED_SHORT_5_5_5_1) => 5,
            _ => 8,
        };
        let bits = match layout {
            RGBA => [b, b, b, b, 0, 0],
            RGB => [b, b, b, 0, 0, 0],
            ALPHA => [0, 0, 0, b, 0, 0],
            LUMINANCE => [b, 0, 0, 0, 0, 0],
            _ => [b, 0, 0, b, 0, 0],
        };
        return f(internal, layout, num, bits);
    }
    if !webgl2 {
        return None;
    }
    // Sized formats (WebGL 2): check the format/type pair is one the spec allows.
    let ok = |fmts: &[u32], tys: &[u32]| fmts.contains(&format) && tys.contains(&ty);
    let fl = [FLOAT, HALF_FLOAT];
    let r = match internal {
        R8 if ok(&[RED], &[UNSIGNED_BYTE]) => f(R8, RED, Num::Unorm(8), [8, 0, 0, 0, 0, 0]),
        R8_SNORM if ok(&[RED], &[BYTE]) => f(R8_SNORM, RED, Num::Snorm, [8, 0, 0, 0, 0, 0]),
        RG8 if ok(&[RG], &[UNSIGNED_BYTE]) => f(RG8, RG, Num::Unorm(8), [8, 8, 0, 0, 0, 0]),
        RG8_SNORM if ok(&[RG], &[BYTE]) => f(RG8_SNORM, RG, Num::Snorm, [8, 8, 0, 0, 0, 0]),
        RGB8 | SRGB8 if ok(&[RGB], &[UNSIGNED_BYTE]) => f(internal, RGB, Num::Unorm(8), [8, 8, 8, 0, 0, 0]),
        RGB8_SNORM if ok(&[RGB], &[BYTE]) => f(RGB8_SNORM, RGB, Num::Snorm, [8, 8, 8, 0, 0, 0]),
        RGB565 if ok(&[RGB], &[UNSIGNED_BYTE, UNSIGNED_SHORT_5_6_5]) => f(RGB565, RGB, Num::Unorm(0), [5, 6, 5, 0, 0, 0]),
        RGBA8 | SRGB8_ALPHA8 if ok(&[RGBA], &[UNSIGNED_BYTE]) => f(internal, RGBA, Num::Unorm(8), [8, 8, 8, 8, 0, 0]),
        RGBA8_SNORM if ok(&[RGBA], &[BYTE]) => f(RGBA8_SNORM, RGBA, Num::Snorm, [8, 8, 8, 8, 0, 0]),
        RGB5_A1 if ok(&[RGBA], &[UNSIGNED_BYTE, UNSIGNED_SHORT_5_5_5_1, UNSIGNED_INT_2_10_10_10_REV]) => f(RGB5_A1, RGBA, Num::Unorm(0), [5, 5, 5, 1, 0, 0]),
        RGBA4 if ok(&[RGBA], &[UNSIGNED_BYTE, UNSIGNED_SHORT_4_4_4_4]) => f(RGBA4, RGBA, Num::Unorm(0), [4, 4, 4, 4, 0, 0]),
        RGB10_A2 if ok(&[RGBA], &[UNSIGNED_INT_2_10_10_10_REV]) => f(RGB10_A2, RGBA, Num::Unorm(0), [10, 10, 10, 2, 0, 0]),
        RGB10_A2UI if ok(&[RGBA_INTEGER], &[UNSIGNED_INT_2_10_10_10_REV]) => f(RGB10_A2UI, RGBA, Num::Uint, [10, 10, 10, 2, 0, 0]),
        R16F if ok(&[RED], &fl) => f(R16F, RED, Num::Half, [16, 0, 0, 0, 0, 0]),
        R32F if ok(&[RED], &[FLOAT]) => f(R32F, RED, Num::Float, [32, 0, 0, 0, 0, 0]),
        RG16F if ok(&[RG], &fl) => f(RG16F, RG, Num::Half, [16, 16, 0, 0, 0, 0]),
        RG32F if ok(&[RG], &[FLOAT]) => f(RG32F, RG, Num::Float, [32, 32, 0, 0, 0, 0]),
        RGB16F if ok(&[RGB], &fl) => f(RGB16F, RGB, Num::Half, [16, 16, 16, 0, 0, 0]),
        RGB32F if ok(&[RGB], &[FLOAT]) => f(RGB32F, RGB, Num::Float, [32, 32, 32, 0, 0, 0]),
        R11F_G11F_B10F if ok(&[RGB], &[FLOAT, HALF_FLOAT, UNSIGNED_INT_10F_11F_11F_REV]) => f(internal, RGB, Num::Half, [11, 11, 10, 0, 0, 0]),
        RGB9_E5 if ok(&[RGB], &[FLOAT, HALF_FLOAT, UNSIGNED_INT_5_9_9_9_REV]) => f(internal, RGB, Num::Half, [9, 9, 9, 0, 0, 0]),
        RGBA16F if ok(&[RGBA], &fl) => f(RGBA16F, RGBA, Num::Half, [16, 16, 16, 16, 0, 0]),
        RGBA32F if ok(&[RGBA], &[FLOAT]) => f(RGBA32F, RGBA, Num::Float, [32, 32, 32, 32, 0, 0]),
        R8UI if ok(&[RED_INTEGER], &[UNSIGNED_BYTE]) => f(R8UI, RED, Num::Uint, [8, 0, 0, 0, 0, 0]),
        R8I if ok(&[RED_INTEGER], &[BYTE]) => f(R8I, RED, Num::Int, [8, 0, 0, 0, 0, 0]),
        R16UI if ok(&[RED_INTEGER], &[UNSIGNED_SHORT]) => f(R16UI, RED, Num::Uint, [16, 0, 0, 0, 0, 0]),
        R16I if ok(&[RED_INTEGER], &[SHORT]) => f(R16I, RED, Num::Int, [16, 0, 0, 0, 0, 0]),
        R32UI if ok(&[RED_INTEGER], &[UNSIGNED_INT]) => f(R32UI, RED, Num::Uint, [32, 0, 0, 0, 0, 0]),
        R32I if ok(&[RED_INTEGER], &[INT]) => f(R32I, RED, Num::Int, [32, 0, 0, 0, 0, 0]),
        RG8UI if ok(&[RG_INTEGER], &[UNSIGNED_BYTE]) => f(RG8UI, RG, Num::Uint, [8, 8, 0, 0, 0, 0]),
        RG8I if ok(&[RG_INTEGER], &[BYTE]) => f(RG8I, RG, Num::Int, [8, 8, 0, 0, 0, 0]),
        RG16UI if ok(&[RG_INTEGER], &[UNSIGNED_SHORT]) => f(RG16UI, RG, Num::Uint, [16, 16, 0, 0, 0, 0]),
        RG16I if ok(&[RG_INTEGER], &[SHORT]) => f(RG16I, RG, Num::Int, [16, 16, 0, 0, 0, 0]),
        RG32UI if ok(&[RG_INTEGER], &[UNSIGNED_INT]) => f(RG32UI, RG, Num::Uint, [32, 32, 0, 0, 0, 0]),
        RG32I if ok(&[RG_INTEGER], &[INT]) => f(RG32I, RG, Num::Int, [32, 32, 0, 0, 0, 0]),
        RGB8UI if ok(&[RGB_INTEGER], &[UNSIGNED_BYTE]) => f(RGB8UI, RGB, Num::Uint, [8, 8, 8, 0, 0, 0]),
        RGB8I if ok(&[RGB_INTEGER], &[BYTE]) => f(RGB8I, RGB, Num::Int, [8, 8, 8, 0, 0, 0]),
        RGB16UI if ok(&[RGB_INTEGER], &[UNSIGNED_SHORT]) => f(RGB16UI, RGB, Num::Uint, [16, 16, 16, 0, 0, 0]),
        RGB16I if ok(&[RGB_INTEGER], &[SHORT]) => f(RGB16I, RGB, Num::Int, [16, 16, 16, 0, 0, 0]),
        RGB32UI if ok(&[RGB_INTEGER], &[UNSIGNED_INT]) => f(RGB32UI, RGB, Num::Uint, [32, 32, 32, 0, 0, 0]),
        RGB32I if ok(&[RGB_INTEGER], &[INT]) => f(RGB32I, RGB, Num::Int, [32, 32, 32, 0, 0, 0]),
        RGBA8UI if ok(&[RGBA_INTEGER], &[UNSIGNED_BYTE]) => f(RGBA8UI, RGBA, Num::Uint, [8, 8, 8, 8, 0, 0]),
        RGBA8I if ok(&[RGBA_INTEGER], &[BYTE]) => f(RGBA8I, RGBA, Num::Int, [8, 8, 8, 8, 0, 0]),
        RGBA16UI if ok(&[RGBA_INTEGER], &[UNSIGNED_SHORT]) => f(RGBA16UI, RGBA, Num::Uint, [16, 16, 16, 16, 0, 0]),
        RGBA16I if ok(&[RGBA_INTEGER], &[SHORT]) => f(RGBA16I, RGBA, Num::Int, [16, 16, 16, 16, 0, 0]),
        RGBA32UI if ok(&[RGBA_INTEGER], &[UNSIGNED_INT]) => f(RGBA32UI, RGBA, Num::Uint, [32, 32, 32, 32, 0, 0]),
        RGBA32I if ok(&[RGBA_INTEGER], &[INT]) => f(RGBA32I, RGBA, Num::Int, [32, 32, 32, 32, 0, 0]),
        DEPTH_COMPONENT16 if ok(&[DEPTH_COMPONENT], &[UNSIGNED_SHORT, UNSIGNED_INT]) => depth_fmt(internal),
        DEPTH_COMPONENT24 if ok(&[DEPTH_COMPONENT], &[UNSIGNED_INT]) => depth_fmt(internal),
        DEPTH_COMPONENT32F if ok(&[DEPTH_COMPONENT], &[FLOAT]) => depth_fmt(internal),
        DEPTH24_STENCIL8 if ok(&[DEPTH_STENCIL], &[UNSIGNED_INT_24_8]) => depth_fmt(internal),
        DEPTH32F_STENCIL8 if ok(&[DEPTH_STENCIL], &[FLOAT_32_UNSIGNED_INT_24_8_REV]) => depth_fmt(internal),
        _ => None,
    };
    r
}

/// The format for a sized internal format alone (texStorage, renderbufferStorage).
pub fn sized_format(internal: u32, webgl2: bool) -> Option<Fmt> {
    use e::*;
    match internal {
        DEPTH_COMPONENT16 | DEPTH_COMPONENT24 | DEPTH_COMPONENT32F | DEPTH24_STENCIL8 | DEPTH32F_STENCIL8 | STENCIL_INDEX8 => depth_fmt(internal),
        // WebGL 1 renderbuffers: DEPTH_STENCIL means DEPTH24_STENCIL8.
        DEPTH_STENCIL => depth_fmt(DEPTH24_STENCIL8).map(|mut f| {
            f.internal = DEPTH_STENCIL;
            f
        }),
        RGBA4 => texture_format(internal, RGBA, UNSIGNED_SHORT_4_4_4_4, true),
        RGB5_A1 => texture_format(internal, RGBA, UNSIGNED_SHORT_5_5_5_1, true),
        RGB565 => texture_format(internal, RGB, UNSIGNED_SHORT_5_6_5, true),
        _ if !webgl2 => match internal {
            SRGB_ALPHA_EXT | 0x8C43 => texture_format(SRGB8_ALPHA8, RGBA, UNSIGNED_BYTE, true),
            RGBA32F | RGBA16F | RGB16F | RGB32F => sized_format(internal, true),
            _ => None,
        },
        _ => {
            for (fmt, ty) in [
                (RGBA, UNSIGNED_BYTE),
                (RGB, UNSIGNED_BYTE),
                (RG, UNSIGNED_BYTE),
                (RED, UNSIGNED_BYTE),
                (RGBA, BYTE),
                (RGB, BYTE),
                (RG, BYTE),
                (RED, BYTE),
                (RGBA, FLOAT),
                (RGB, FLOAT),
                (RG, FLOAT),
                (RED, FLOAT),
                (RGBA, UNSIGNED_INT_2_10_10_10_REV),
                (RGBA_INTEGER, UNSIGNED_INT_2_10_10_10_REV),
                (RGB, UNSIGNED_INT_10F_11F_11F_REV),
                (RGB, UNSIGNED_INT_5_9_9_9_REV),
            ] {
                if let Some(f) = texture_format(internal, fmt, ty, true) {
                    return Some(f);
                }
            }
            for fmt in [RED_INTEGER, RG_INTEGER, RGB_INTEGER, RGBA_INTEGER] {
                for ty in [UNSIGNED_BYTE, BYTE, UNSIGNED_SHORT, SHORT, UNSIGNED_INT, INT] {
                    if let Some(f) = texture_format(internal, fmt, ty, true) {
                        return Some(f);
                    }
                }
            }
            None
        }
    }
}

fn depth_fmt(internal: u32) -> Option<Fmt> {
    use e::*;
    let (d, s, layout) = match internal {
        DEPTH_COMPONENT16 => (16, 0, DEPTH_COMPONENT),
        DEPTH_COMPONENT24 => (24, 0, DEPTH_COMPONENT),
        DEPTH_COMPONENT32F => (32, 0, DEPTH_COMPONENT),
        DEPTH24_STENCIL8 => (24, 8, DEPTH_STENCIL),
        DEPTH32F_STENCIL8 => (32, 8, DEPTH_STENCIL),
        STENCIL_INDEX8 => (0, 8, STENCIL_INDEX8),
        _ => return None,
    };
    Some(Fmt { internal, layout, num: if d == 32 { Num::Float } else { Num::Unorm(0) }, depth: d > 0, stencil: s > 0, bits: [0, 0, 0, 0, d, s] })
}

/// Bytes per pixel for a client format/type pair.
pub fn pixel_size(format: u32, ty: u32) -> Option<usize> {
    use e::*;
    let comps = match format {
        RED | RED_INTEGER | ALPHA | LUMINANCE | DEPTH_COMPONENT => 1,
        RG | RG_INTEGER | LUMINANCE_ALPHA => 2,
        RGB | RGB_INTEGER | SRGB_EXT => 3,
        RGBA | RGBA_INTEGER | SRGB_ALPHA_EXT => 4,
        DEPTH_STENCIL => 1,
        _ => return None,
    };
    Some(match ty {
        UNSIGNED_BYTE | BYTE => comps,
        UNSIGNED_SHORT | SHORT | HALF_FLOAT | HALF_FLOAT_OES => comps * 2,
        UNSIGNED_INT | INT | FLOAT => comps * 4,
        UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => 2,
        UNSIGNED_INT_24_8 | UNSIGNED_INT_2_10_10_10_REV | UNSIGNED_INT_10F_11F_11F_REV | UNSIGNED_INT_5_9_9_9_REV => 4,
        FLOAT_32_UNSIGNED_INT_24_8_REV => 8,
        _ => return None,
    })
}

#[derive(Clone, Debug)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub d: u32,
    pub fmt: Fmt,
    pub px: Vec<[f32; 4]>,
    /// Stencil values for depth-stencil and stencil formats.
    pub st: Vec<u8>,
}

impl Image {
    pub fn new(w: u32, h: u32, d: u32, fmt: Fmt, webgl2: bool) -> Image {
        let n = (w as usize) * (h as usize) * (d.max(1) as usize);
        let fill = default_texel(&fmt, webgl2);
        Image { w, h, d: d.max(1), fmt, px: vec![fill; n], st: if fmt.stencil { vec![0; n] } else { Vec::new() } }
    }
}

/// What an all-zero texel of this format reads as.
fn default_texel(f: &Fmt, _webgl2: bool) -> [f32; 4] {
    expand(f, [0.0; 4])
}

/// Fill in the channels a format doesn't have, the way sampling reads them.
pub fn expand(f: &Fmt, v: [f32; 4]) -> [f32; 4] {
    let one = if f.is_integer() { f32::from_bits(1) } else { 1.0 };
    let zero = 0.0;
    match f.layout {
        e::RGBA | e::DEPTH_STENCIL => v,
        e::RGB => [v[0], v[1], v[2], one],
        e::RG => [v[0], v[1], zero, one],
        e::RED => [v[0], zero, zero, one],
        e::LUMINANCE => [v[0], v[0], v[0], one],
        e::ALPHA => [zero, zero, zero, v[3]],
        e::LUMINANCE_ALPHA => [v[0], v[0], v[0], v[3]],
        e::DEPTH_COMPONENT => v,
        _ => v,
    }
}

fn to_half(f: f32) -> f32 {
    // Round through half precision.
    let h = half::from_f32(f);
    f32_from_half(h)
}

mod half {
    pub fn from_f32(f: f32) -> u16 {
        let x = f.to_bits();
        let sign = ((x >> 16) & 0x8000) as u16;
        let exp = ((x >> 23) & 0xff) as i32;
        let mant = x & 0x7f_ffff;
        if exp == 255 {
            return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
        }
        let e = exp - 127 + 15;
        if e >= 31 {
            return sign | 0x7c00;
        }
        if e <= 0 {
            if e < -10 {
                return sign;
            }
            let m = (mant | 0x80_0000) >> (1 - e);
            return sign | ((m + 0x1000) >> 13) as u16;
        }
        let h = sign as u32 | ((e as u32) << 10) | (mant >> 13);
        (h + ((mant >> 12) & 1)) as u16
    }
}

/// Quantize a value written by rendering (or a clear) into this format.
pub fn store(f: &Fmt, v: [f32; 4]) -> [f32; 4] {
    let q = |x: f32, bits: u8| -> f32 {
        if bits == 0 {
            return x.clamp(0.0, 1.0);
        }
        let m = ((1u32 << bits) - 1) as f32;
        (x.clamp(0.0, 1.0) * m).round() / m
    };
    let out = match f.num {
        Num::Unorm(_) => [q(v[0], f.bits[0]), q(v[1], f.bits[1]), q(v[2], f.bits[2]), q(v[3], if f.bits[3] == 0 { 8 } else { f.bits[3] })],
        Num::Snorm => v.map(|x| (x.clamp(-1.0, 1.0) * 127.0).round() / 127.0),
        Num::Half => v.map(to_half),
        _ => v,
    };
    expand(f, out)
}

pub struct Unpack {
    pub alignment: usize,
    pub flip_y: bool,
    pub premultiply: bool,
    pub row_length: usize,
    pub skip_rows: usize,
    pub skip_pixels: usize,
    pub image_height: usize,
    pub skip_images: usize,
}

/// Convert client pixel data to texels for format `f`. Returns None if the data is too short.
pub fn decode(data: &[u8], w: u32, h: u32, d: u32, format: u32, ty: u32, f: &Fmt, up: &Unpack) -> Option<(Vec<[f32; 4]>, Vec<u8>)> {
    let bpp = pixel_size(format, ty)?;
    let (w, h, d) = (w as usize, h as usize, d.max(1) as usize);
    let row_px = if up.row_length > 0 { up.row_length } else { w };
    let row = (row_px * bpp).div_ceil(up.alignment) * up.alignment;
    let img_rows = if up.image_height > 0 { up.image_height } else { h };
    let start = up.skip_images * img_rows * row + up.skip_rows * row + up.skip_pixels * bpp;
    let need = if w == 0 || h == 0 { 0 } else { start + ((d - 1) * img_rows + h - 1) * row + w * bpp };
    if data.len() < need {
        return None;
    }
    let mut px = vec![[0.0f32; 4]; w * h * d];
    let mut st = if f.stencil { vec![0u8; w * h * d] } else { Vec::new() };
    let int = f.is_integer();
    for z in 0..d {
        for y in 0..h {
            let sy = if up.flip_y { h - 1 - y } else { y };
            let base = start + (z * img_rows + sy) * row;
            for x in 0..w {
                let o = base + x * bpp;
                let p = &data[o..o + bpp];
                let (v, s) = read_pixel(p, format, ty, int);
                let mut v = v;
                if up.premultiply && !int && matches!(format, e::RGBA | e::LUMINANCE_ALPHA) {
                    v[0] *= v[3];
                    v[1] *= v[3];
                    v[2] *= v[3];
                }
                let i = (z * h + y) * w + x;
                px[i] = expand(f, place(format, v));
                if f.layout == e::DEPTH_COMPONENT || f.layout == e::DEPTH_STENCIL {
                    px[i] = [v[0], v[0], v[0], 1.0];
                }
                if f.stencil {
                    st[i] = s;
                }
            }
        }
    }
    Some((px, st))
}

/// Put read channels where the layout keeps them (ALPHA's one value is alpha).
fn place(format: u32, v: [f32; 4]) -> [f32; 4] {
    match format {
        e::ALPHA => [0.0, 0.0, 0.0, v[0]],
        e::LUMINANCE_ALPHA => [v[0], v[0], v[0], v[1]],
        _ => v,
    }
}

fn read_pixel(p: &[u8], format: u32, ty: u32, int: bool) -> ([f32; 4], u8) {
    use e::*;
    let comps = match format {
        RED | RED_INTEGER | ALPHA | LUMINANCE | DEPTH_COMPONENT => 1,
        RG | RG_INTEGER | LUMINANCE_ALPHA => 2,
        RGB | RGB_INTEGER | SRGB_EXT => 3,
        _ => 4,
    };
    let mut v = [0.0f32; 4];
    let u16_at = |i: usize| u16::from_le_bytes([p[i], p[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
    match ty {
        UNSIGNED_SHORT_5_6_5 => {
            let x = u16_at(0);
            return ([((x >> 11) & 31) as f32 / 31.0, ((x >> 5) & 63) as f32 / 63.0, (x & 31) as f32 / 31.0, 1.0], 0);
        }
        UNSIGNED_SHORT_4_4_4_4 => {
            let x = u16_at(0);
            return ([((x >> 12) & 15) as f32 / 15.0, ((x >> 8) & 15) as f32 / 15.0, ((x >> 4) & 15) as f32 / 15.0, (x & 15) as f32 / 15.0], 0);
        }
        UNSIGNED_SHORT_5_5_5_1 => {
            let x = u16_at(0);
            return ([((x >> 11) & 31) as f32 / 31.0, ((x >> 6) & 31) as f32 / 31.0, ((x >> 1) & 31) as f32 / 31.0, (x & 1) as f32], 0);
        }
        UNSIGNED_INT_24_8 => {
            let x = u32_at(0);
            return ([(x >> 8) as f32 / 16_777_215.0, 0.0, 0.0, 0.0], (x & 0xff) as u8);
        }
        FLOAT_32_UNSIGNED_INT_24_8_REV => {
            let dv = f32::from_bits(u32_at(0));
            return ([dv, 0.0, 0.0, 0.0], (u32_at(4) & 0xff) as u8);
        }
        UNSIGNED_INT_2_10_10_10_REV => {
            let x = u32_at(0);
            let c = [x & 1023, (x >> 10) & 1023, (x >> 20) & 1023, x >> 30];
            if int {
                return (c.map(f32::from_bits), 0);
            }
            return ([c[0] as f32 / 1023.0, c[1] as f32 / 1023.0, c[2] as f32 / 1023.0, c[3] as f32 / 3.0], 0);
        }
        UNSIGNED_INT_10F_11F_11F_REV => {
            let x = u32_at(0);
            let f11 = |b: u32| -> f32 {
                let (e, m) = ((b >> 6) & 31, b & 63);
                f32_from_half(((e << 10) | (m << 4)) as u16)
            };
            let f10 = |b: u32| -> f32 {
                let (e, m) = ((b >> 5) & 31, b & 31);
                f32_from_half(((e << 10) | (m << 5)) as u16)
            };
            return ([f11(x & 2047), f11((x >> 11) & 2047), f10(x >> 22), 1.0], 0);
        }
        UNSIGNED_INT_5_9_9_9_REV => {
            let x = u32_at(0);
            let ex = (x >> 27) as i32 - 15 - 9;
            let s = 2f32.powi(ex);
            return ([(x & 511) as f32 * s, ((x >> 9) & 511) as f32 * s, ((x >> 18) & 511) as f32 * s, 1.0], 0);
        }
        _ => {}
    }
    for c in 0..comps {
        v[c] = match ty {
            UNSIGNED_BYTE if int => f32::from_bits(p[c] as u32),
            UNSIGNED_BYTE => p[c] as f32 / 255.0,
            BYTE if int => f32::from_bits(p[c] as i8 as i32 as u32),
            BYTE => (p[c] as i8 as f32 / 127.0).max(-1.0),
            UNSIGNED_SHORT if int => f32::from_bits(u16_at(c * 2) as u32),
            UNSIGNED_SHORT => u16_at(c * 2) as f32 / 65535.0,
            SHORT => f32::from_bits(u16_at(c * 2) as i16 as i32 as u32),
            UNSIGNED_INT if int => f32::from_bits(u32_at(c * 4)),
            UNSIGNED_INT => (u32_at(c * 4) as f64 / u32::MAX as f64) as f32,
            INT => f32::from_bits(u32_at(c * 4)),
            FLOAT => f32::from_bits(u32_at(c * 4)),
            HALF_FLOAT | HALF_FLOAT_OES => f32_from_half(u16_at(c * 2)),
            _ => 0.0,
        };
    }
    if comps < 4 && !matches!(format, ALPHA | LUMINANCE_ALPHA) {
        v[3] = if int { f32::from_bits(1) } else { 1.0 };
    }
    (v, 0)
}

/// Convert straight RGBA8 pixels (from an <img>, canvas or ImageData) into texels.
pub fn from_rgba8(rgba: &[u8], w: u32, h: u32, format: u32, ty: u32, f: &Fmt, flip_y: bool, premultiply: bool) -> Vec<[f32; 4]> {
    let (w, h) = (w as usize, h as usize);
    let mut px = vec![[0.0f32; 4]; w * h];
    let int = f.is_integer();
    for y in 0..h {
        // DOM images are top row first; without flipping, the top row becomes GL row 0.
        let sy = if flip_y { h - 1 - y } else { y };
        for x in 0..w {
            let o = (sy * w + x) * 4;
            let mut v = [rgba[o] as f32 / 255.0, rgba[o + 1] as f32 / 255.0, rgba[o + 2] as f32 / 255.0, rgba[o + 3] as f32 / 255.0];
            if premultiply {
                for c in 0..3 {
                    v[c] *= v[3];
                }
            }
            if int {
                v = [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]].map(|b| f32::from_bits(b as u32));
            }
            let v = match format {
                e::ALPHA => [0.0, 0.0, 0.0, v[3]],
                e::LUMINANCE => [v[0], v[0], v[0], 1.0],
                e::LUMINANCE_ALPHA => [v[0], v[0], v[0], v[3]],
                _ => v,
            };
            let v = match ty {
                e::UNSIGNED_SHORT_5_6_5 | e::UNSIGNED_SHORT_4_4_4_4 | e::UNSIGNED_SHORT_5_5_5_1 => store(f, v),
                e::HALF_FLOAT | e::HALF_FLOAT_OES => v.map(to_half),
                _ => v,
            };
            px[y * w + x] = expand(f, v);
        }
    }
    px
}

// ---------------------------------------------------------------- textures

#[derive(Clone, Debug)]
pub struct TexParams {
    pub min: u32,
    pub mag: u32,
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub wrap_r: u32,
    pub base: u32,
    pub max: u32,
    pub min_lod: f32,
    pub max_lod: f32,
    pub compare_mode: u32,
    pub compare_func: u32,
    pub anisotropy: f32,
}

impl Default for TexParams {
    fn default() -> Self {
        TexParams {
            min: e::NEAREST_MIPMAP_LINEAR,
            mag: e::LINEAR,
            wrap_s: e::REPEAT,
            wrap_t: e::REPEAT,
            wrap_r: e::REPEAT,
            base: 0,
            max: 1000,
            min_lod: -1000.0,
            max_lod: 1000.0,
            compare_mode: e::NONE,
            compare_func: e::LEQUAL,
            anisotropy: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Texture {
    /// 0 until first bound.
    pub target: u32,
    /// [face][level]: one face, or six for cube maps (+X −X +Y −Y +Z −Z).
    pub faces: Vec<Vec<Option<Image>>>,
    pub p: TexParams,
    pub immutable: bool,
    pub immutable_levels: u32,
}

impl Texture {
    pub fn new() -> Texture {
        Texture { target: 0, faces: Vec::new(), p: TexParams::default(), immutable: false, immutable_levels: 0 }
    }
    pub fn level(&self, face: usize, level: usize) -> Option<&Image> {
        self.faces.get(face)?.get(level)?.as_ref()
    }
    pub fn set_level(&mut self, face: usize, level: usize, img: Image) {
        let nf = if self.target == e::TEXTURE_CUBE_MAP { 6 } else { 1 };
        if self.faces.len() < nf {
            self.faces.resize(nf, Vec::new());
        }
        let f = &mut self.faces[face];
        if f.len() <= level {
            f.resize(level + 1, None);
        }
        f[level] = Some(img);
    }
    pub fn base_image(&self) -> Option<&Image> {
        self.level(0, self.p.base as usize)
    }
    /// The last mipmap level (`q`) of a full chain from base.
    pub fn max_level(&self) -> usize {
        let Some(b) = self.base_image() else { return 0 };
        let mut size = b.w.max(b.h);
        if self.target == e::TEXTURE_3D {
            size = size.max(b.d);
        }
        let mut q = self.p.base as usize;
        while size > 1 {
            size /= 2;
            q += 1;
        }
        let q = if self.immutable { q.min(self.immutable_levels.saturating_sub(1) as usize) } else { q };
        q.min(self.p.max as usize)
    }
    /// Is the texture complete for sampling with its parameters (or a sampler's)?
    pub fn complete(&self, p: &TexParams, webgl2: bool) -> bool {
        let Some(b) = self.base_image() else { return false };
        if b.w == 0 || b.h == 0 {
            return false;
        }
        let nf = if self.target == e::TEXTURE_CUBE_MAP { 6 } else { 1 };
        for f in 0..nf {
            let Some(i) = self.level(f, p.base as usize) else { return false };
            if i.w != b.w || i.h != b.h || i.fmt != b.fmt || (nf == 6 && i.w != i.h) {
                return false;
            }
        }
        let mip = !matches!(p.min, e::NEAREST | e::LINEAR);
        if !b.fmt.filterable() && (!matches!(p.min, e::NEAREST | e::NEAREST_MIPMAP_NEAREST) || p.mag != e::NEAREST) {
            return false;
        }
        if !webgl2 {
            let pot = |x: u32| x.is_power_of_two();
            if (!pot(b.w) || !pot(b.h)) && (mip || p.wrap_s != e::CLAMP_TO_EDGE || p.wrap_t != e::CLAMP_TO_EDGE) {
                return false;
            }
        }
        if mip {
            let q = self.max_level();
            for f in 0..nf {
                let (mut w, mut h, mut d) = (b.w, b.h, b.d);
                for lv in p.base as usize..=q {
                    let Some(i) = self.level(f, lv) else { return false };
                    if i.w != w || i.h != h || i.fmt != b.fmt || (self.target == e::TEXTURE_3D && i.d != d) {
                        return false;
                    }
                    w = (w / 2).max(1);
                    h = (h / 2).max(1);
                    if self.target == e::TEXTURE_3D {
                        d = (d / 2).max(1);
                    }
                }
            }
        }
        true
    }
    /// Build the mipmap chain from the base level (a box filter).
    pub fn generate_mipmap(&mut self) -> bool {
        let nf = self.faces.len();
        if nf == 0 {
            return false;
        }
        let base = self.p.base as usize;
        let q = self.max_level_unclamped();
        for f in 0..nf {
            let Some(mut prev) = self.level(f, base).cloned() else { return false };
            for lv in base + 1..=q {
                let (w, h) = ((prev.w / 2).max(1), (prev.h / 2).max(1));
                let d = if self.target == e::TEXTURE_3D { (prev.d / 2).max(1) } else { prev.d };
                let mut img = Image { w, h, d, fmt: prev.fmt, px: vec![[0.0; 4]; (w * h * d) as usize], st: Vec::new() };
                for z in 0..d {
                    for y in 0..h {
                        for x in 0..w {
                            let mut acc = [0.0f32; 4];
                            let mut n = 0.0;
                            let zs = if self.target == e::TEXTURE_3D { [z * 2, (z * 2 + 1).min(prev.d - 1)] } else { [z, z] };
                            for sz in zs {
                                for sy in [y * 2, (y * 2 + 1).min(prev.h - 1)] {
                                    for sx in [x * 2, (x * 2 + 1).min(prev.w - 1)] {
                                        let t = prev.px[((sz * prev.h + sy) * prev.w + sx) as usize];
                                        for c in 0..4 {
                                            acc[c] += t[c];
                                        }
                                        n += 1.0;
                                    }
                                }
                            }
                            img.px[((z * h + y) * w + x) as usize] = store(&prev.fmt, acc.map(|v| v / n));
                        }
                    }
                }
                self.set_level(f, lv, img.clone());
                prev = img;
            }
        }
        true
    }
    fn max_level_unclamped(&self) -> usize {
        let Some(b) = self.base_image() else { return 0 };
        let mut size = b.w.max(b.h);
        if self.target == e::TEXTURE_3D {
            size = size.max(b.d);
        }
        let mut q = self.p.base as usize;
        while size > 1 {
            size /= 2;
            q += 1;
        }
        if self.immutable {
            q = q.min(self.immutable_levels.saturating_sub(1) as usize);
        }
        q.min(self.p.max as usize)
    }
}

// ---------------------------------------------------------------- sampling

fn wrap(i: i32, size: i32, mode: u32) -> i32 {
    match mode {
        e::REPEAT => i.rem_euclid(size),
        e::MIRRORED_REPEAT => {
            let m = i.rem_euclid(2 * size);
            if m >= size { 2 * size - 1 - m } else { m }
        }
        _ => i.clamp(0, size - 1),
    }
}

fn texel(img: &Image, x: i32, y: i32, z: i32) -> [f32; 4] {
    img.px[((z as usize * img.h as usize) + y as usize) * img.w as usize + x as usize]
}

fn compare(func: u32, r: f32, d: f32) -> f32 {
    let r = r.clamp(0.0, 1.0);
    let pass = match func {
        e::LEQUAL => r <= d,
        e::GEQUAL => r >= d,
        e::LESS => r < d,
        e::GREATER => r > d,
        e::EQUAL => r == d,
        e::NOTEQUAL => r != d,
        e::ALWAYS => true,
        _ => false,
    };
    if pass { 1.0 } else { 0.0 }
}

/// Sample one level of a 2D (or one layer of an array / slice of a 3D) image.
fn sample_level(img: &Image, s: f32, t: f32, r: f32, linear: bool, p: &TexParams, layer: Option<i32>, cmp: Option<f32>, off: [i32; 3], is3d: bool) -> [f32; 4] {
    let (w, h, d) = (img.w as i32, img.h as i32, img.d as i32);
    let fetch = |x: i32, y: i32, z: i32| -> [f32; 4] {
        let t = texel(img, wrap(x + off[0], w, p.wrap_s), wrap(y + off[1], h, p.wrap_t), z);
        match cmp {
            Some(rf) => {
                let c = compare(p.compare_func, rf, t[0]);
                [c, c, c, 1.0]
            }
            None => t,
        }
    };
    let z_of = |zz: i32| -> i32 {
        if is3d {
            wrap(zz + off[2], d, p.wrap_r)
        } else {
            layer.unwrap_or(0).clamp(0, d - 1)
        }
    };
    if !linear {
        let x = (s * w as f32).floor() as i32;
        let y = (t * h as f32).floor() as i32;
        let z = z_of((r * d as f32).floor() as i32);
        return fetch(x, y, z);
    }
    let u = s * w as f32 - 0.5;
    let v = t * h as f32 - 0.5;
    let (x0, y0) = (u.floor() as i32, v.floor() as i32);
    let (a, b) = (u - u.floor(), v - v.floor());
    let lerp4 = |p: [f32; 4], q: [f32; 4], k: f32| [0, 1, 2, 3].map(|i| p[i] + (q[i] - p[i]) * k);
    let plane = |z: i32| {
        let top = lerp4(fetch(x0, y0, z), fetch(x0 + 1, y0, z), a);
        let bot = lerp4(fetch(x0, y0 + 1, z), fetch(x0 + 1, y0 + 1, z), a);
        lerp4(top, bot, b)
    };
    if is3d {
        let ww = r * d as f32 - 0.5;
        let z0 = ww.floor() as i32;
        let c = ww - ww.floor();
        return lerp4(plane(z_of(z0)), plane(z_of(z0 + 1)), c);
    }
    plane(z_of(0))
}

pub struct UnitView<'a> {
    pub tex: &'a Texture,
    pub p: &'a TexParams,
    pub complete: bool,
}

/// Textures bound to each unit, by sampler dimension, for one draw.
pub struct Sampler<'a> {
    /// [unit] → views for 2D, 3D, cube, 2D array.
    pub units: Vec<[Option<UnitView<'a>>; 4]>,
    pub webgl2: bool,
}

fn dim_index(d: Dim) -> usize {
    match d {
        Dim::D2 => 0,
        Dim::D3 => 1,
        Dim::Cube => 2,
        Dim::D2Array => 3,
    }
}

fn cube_face(c: [f32; 3]) -> (usize, f32, f32, f32) {
    let (x, y, z) = (c[0], c[1], c[2]);
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, sc, tc, ma) = if ax >= ay && ax >= az {
        if x >= 0.0 { (0, -z, -y, ax) } else { (1, z, -y, ax) }
    } else if ay >= az {
        if y >= 0.0 { (2, x, z, ay) } else { (3, x, -z, ay) }
    } else if z >= 0.0 {
        (4, x, -y, az)
    } else {
        (5, -x, -y, az)
    };
    let ma = if ma == 0.0 { 1.0 } else { ma };
    (face, (sc / ma + 1.0) * 0.5, (tc / ma + 1.0) * 0.5, ma)
}

impl Sampler<'_> {
    fn view(&self, unit: i32, samp: Samp) -> Option<&UnitView<'_>> {
        self.units.get(unit as usize)?[dim_index(samp.dim)].as_ref()
    }
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

impl TexEnv for Sampler<'_> {
    fn sample(&self, unit: i32, samp: Samp, a: &TexArgs) -> [f32; 4] {
        let Some(v) = self.view(unit, samp) else { return BLACK };
        if !v.complete {
            return if samp.base == Base::F { BLACK } else { [0.0, 0.0, 0.0, f32::from_bits(1)] };
        }
        let tex = v.tex;
        let p = v.p;
        let Some(b) = tex.base_image() else { return BLACK };
        let cmp = if samp.shadow || p.compare_mode == e::COMPARE_REF_TO_TEXTURE && b.fmt.depth { a.compare.or(Some(a.coord[2])) } else { None };
        let cmp = if p.compare_mode == e::COMPARE_REF_TO_TEXTURE { cmp } else { None };
        // Face and 2D coordinates.
        let (face, s, t, r, ma) = match samp.dim {
            Dim::Cube => {
                let (f, s, t, ma) = cube_face([a.coord[0], a.coord[1], a.coord[2]]);
                (f, s, t, 0.0, ma)
            }
            _ => (0, a.coord[0], a.coord[1], a.coord[2], 1.0),
        };
        // Level of detail.
        let lambda = match a.lod {
            Some(l) => l,
            None => {
                let (w, h, d) = (b.w as f32, b.h as f32, b.d as f32);
                let (dx, dy) = if samp.dim == Dim::Cube {
                    // Scale the direction's change to face texels.
                    let k = 0.5 * w / ma;
                    (
                        [a.dx[0] * k, a.dx[1] * k, a.dx[2] * k, 0.0],
                        [a.dy[0] * k, a.dy[1] * k, a.dy[2] * k, 0.0],
                    )
                } else {
                    let dz = if samp.dim == Dim::D3 { d } else { 0.0 };
                    ([a.dx[0] * w, a.dx[1] * h, a.dx[2] * dz, 0.0], [a.dy[0] * w, a.dy[1] * h, a.dy[2] * dz, 0.0])
                };
                let rx = (dx[0] * dx[0] + dx[1] * dx[1] + dx[2] * dx[2]).sqrt();
                let ry = (dy[0] * dy[0] + dy[1] * dy[1] + dy[2] * dy[2]).sqrt();
                let rho = rx.max(ry);
                let mut l = if rho > 0.0 { rho.log2() } else { -1000.0 };
                if p.anisotropy > 1.0 && rho > 0.0 {
                    let minr = rx.min(ry).max(rho / p.anisotropy);
                    l = minr.log2();
                }
                l + a.bias
            }
        };
        let lambda = lambda.clamp(p.min_lod, p.max_lod);
        let layer = if samp.dim == Dim::D2Array { Some(r.round() as i32) } else { None };
        let is3d = samp.dim == Dim::D3;
        let off = a.offset;
        let lv = |l: usize, linear: bool| -> [f32; 4] {
            match tex.level(face, l) {
                Some(img) => sample_level(img, s, t, r, linear, p, layer, cmp, off, is3d),
                None => BLACK,
            }
        };
        let base = p.base as usize;
        let q = tex.max_level();
        if lambda <= 0.0 {
            return lv(base, p.mag == e::LINEAR);
        }
        match p.min {
            e::NEAREST => lv(base, false),
            e::LINEAR => lv(base, true),
            e::NEAREST_MIPMAP_NEAREST | e::LINEAR_MIPMAP_NEAREST => {
                let l = if lambda <= 0.5 { base } else { (base + (lambda + 0.5).ceil() as usize - 1).min(q) };
                lv(l, p.min == e::LINEAR_MIPMAP_NEAREST)
            }
            _ => {
                let lin = p.min == e::LINEAR_MIPMAP_LINEAR;
                let l0 = (base + lambda.floor() as usize).min(q);
                let l1 = (l0 + 1).min(q);
                let k = lambda - lambda.floor();
                let (x, y) = (lv(l0, lin), lv(l1, lin));
                if l0 == l1 {
                    return x;
                }
                [0, 1, 2, 3].map(|i| x[i] + (y[i] - x[i]) * k)
            }
        }
    }

    fn fetch(&self, unit: i32, samp: Samp, c: [i32; 3], lod: i32) -> [f32; 4] {
        let Some(v) = self.view(unit, samp) else { return [0.0; 4] };
        if !v.complete && !v.tex.base_image().is_some() {
            return [0.0; 4];
        }
        let Some(img) = v.tex.level(0, (v.p.base as i32 + lod).max(0) as usize) else { return [0.0; 4] };
        let z = if samp.dim == Dim::D2 { 0 } else { c[2] };
        if c[0] < 0 || c[1] < 0 || z < 0 || c[0] >= img.w as i32 || c[1] >= img.h as i32 || z >= img.d as i32 {
            return [0.0; 4];
        }
        texel(img, c[0], c[1], z)
    }

    fn size(&self, unit: i32, samp: Samp, lod: i32) -> [i32; 3] {
        let Some(v) = self.view(unit, samp) else { return [0; 3] };
        match v.tex.level(0, (v.p.base as i32 + lod).max(0) as usize) {
            Some(i) => [i.w as i32, i.h as i32, i.d as i32],
            None => [0; 3],
        }
    }
}

// ---------------------------------------------------------------- compressed textures (S3TC)

pub const COMPRESSED_RGB_S3TC_DXT1: u32 = 0x83F0;
pub const COMPRESSED_RGBA_S3TC_DXT1: u32 = 0x83F1;
pub const COMPRESSED_RGBA_S3TC_DXT3: u32 = 0x83F2;
pub const COMPRESSED_RGBA_S3TC_DXT5: u32 = 0x83F3;
pub const COMPRESSED_SRGB_S3TC_DXT1: u32 = 0x8C4C;
pub const COMPRESSED_SRGB_ALPHA_S3TC_DXT1: u32 = 0x8C4D;
pub const COMPRESSED_SRGB_ALPHA_S3TC_DXT3: u32 = 0x8C4E;
pub const COMPRESSED_SRGB_ALPHA_S3TC_DXT5: u32 = 0x8C4F;

/// The storage format and block size (bytes per 4×4 block) of a compressed format.
pub fn compressed_format(internal: u32) -> Option<(Fmt, usize)> {
    let (layout, block) = match internal {
        COMPRESSED_RGB_S3TC_DXT1 | COMPRESSED_SRGB_S3TC_DXT1 => (e::RGB, 8),
        COMPRESSED_RGBA_S3TC_DXT1 | COMPRESSED_SRGB_ALPHA_S3TC_DXT1 => (e::RGBA, 8),
        COMPRESSED_RGBA_S3TC_DXT3 | COMPRESSED_RGBA_S3TC_DXT5 | COMPRESSED_SRGB_ALPHA_S3TC_DXT3 | COMPRESSED_SRGB_ALPHA_S3TC_DXT5 => (e::RGBA, 16),
        _ => return None,
    };
    Some((Fmt { internal, layout, num: Num::Unorm(8), depth: false, stencil: false, bits: [5, 6, 5, if layout == e::RGBA { 8 } else { 0 }, 0, 0] }, block))
}

/// The byte size compressed data of this format must have for a w×h image.
pub fn compressed_size(internal: u32, w: u32, h: u32) -> Option<usize> {
    let (_, block) = compressed_format(internal)?;
    Some(w.div_ceil(4) as usize * h.div_ceil(4) as usize * block)
}

fn rgb565(c: u16) -> [f32; 3] {
    [((c >> 11) & 31) as f32 / 31.0, ((c >> 5) & 63) as f32 / 63.0, (c & 31) as f32 / 31.0]
}

/// Decode S3TC (DXT1/3/5) blocks into texels (GL row order: the first block row is row 0).
pub fn decode_s3tc(data: &[u8], w: u32, h: u32, internal: u32) -> Option<Vec<[f32; 4]>> {
    let (fmt, block) = compressed_format(internal)?;
    if data.len() < compressed_size(internal, w, h)? {
        return None;
    }
    let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
    let mut px = vec![[0.0f32, 0.0, 0.0, 1.0]; (w * h) as usize];
    let dxt1_alpha = matches!(internal, COMPRESSED_RGBA_S3TC_DXT1 | COMPRESSED_SRGB_ALPHA_S3TC_DXT1);
    for by in 0..bh {
        for bx in 0..bw {
            let b = &data[(by * bw + bx) * block..(by * bw + bx + 1) * block];
            let (alpha, color) = if block == 16 { (&b[..8], &b[8..]) } else { (&b[..0], b) };
            let c0 = u16::from_le_bytes([color[0], color[1]]);
            let c1 = u16::from_le_bytes([color[2], color[3]]);
            let (p0, p1) = (rgb565(c0), rgb565(c1));
            let mut pal = [[p0[0], p0[1], p0[2], 1.0], [p1[0], p1[1], p1[2], 1.0], [0.0; 4], [0.0; 4]];
            if c0 > c1 || block == 16 {
                for k in 0..3 {
                    pal[2][k] = (2.0 * p0[k] + p1[k]) / 3.0;
                    pal[3][k] = (p0[k] + 2.0 * p1[k]) / 3.0;
                }
                pal[2][3] = 1.0;
                pal[3][3] = 1.0;
            } else {
                for k in 0..3 {
                    pal[2][k] = (p0[k] + p1[k]) / 2.0;
                }
                pal[2][3] = 1.0;
                pal[3] = [0.0, 0.0, 0.0, if dxt1_alpha { 0.0 } else { 1.0 }];
            }
            let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
            // DXT5 alpha palette.
            let mut apal = [0.0f32; 8];
            let mut abits = 0u64;
            if matches!(internal, COMPRESSED_RGBA_S3TC_DXT5 | COMPRESSED_SRGB_ALPHA_S3TC_DXT5) {
                let (a0, a1) = (alpha[0] as f32 / 255.0, alpha[1] as f32 / 255.0);
                apal[0] = a0;
                apal[1] = a1;
                if alpha[0] > alpha[1] {
                    for k in 1..7 {
                        apal[k + 1] = ((7 - k) as f32 * a0 + k as f32 * a1) / 7.0;
                    }
                } else {
                    for k in 1..5 {
                        apal[k + 1] = ((5 - k) as f32 * a0 + k as f32 * a1) / 5.0;
                    }
                    apal[6] = 0.0;
                    apal[7] = 1.0;
                }
                for k in 0..6 {
                    abits |= (alpha[2 + k] as u64) << (8 * k);
                }
            }
            for y in 0..4 {
                for x in 0..4 {
                    let (px_x, px_y) = (bx * 4 + x, by * 4 + y);
                    if px_x >= w as usize || px_y >= h as usize {
                        continue;
                    }
                    let i = y * 4 + x;
                    let mut c = pal[((bits >> (2 * i)) & 3) as usize];
                    match internal {
                        COMPRESSED_RGBA_S3TC_DXT3 | COMPRESSED_SRGB_ALPHA_S3TC_DXT3 => {
                            let nib = (alpha[i / 2] >> ((i % 2) * 4)) & 15;
                            c[3] = nib as f32 / 15.0;
                        }
                        COMPRESSED_RGBA_S3TC_DXT5 | COMPRESSED_SRGB_ALPHA_S3TC_DXT5 => c[3] = apal[((abits >> (3 * i)) & 7) as usize],
                        _ => {}
                    }
                    px[px_y * w as usize + px_x] = expand(&fmt, c);
                }
            }
        }
    }
    Some(px)
}
