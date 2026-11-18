//! ETC1, ETC2/EAC (WEBGL_compressed_texture_etc1 / _etc) and RGTC (EXT_texture_compression_rgtc)
//! block decoders. Each format stores 4×4 texel blocks; decoded texels come out in GL row
//! order (block row 0 is texture row 0), values in [0, 1] (or [-1, 1] for the signed ones).

use crate::webgl_tex::{e, Fmt, Num};

pub const COMPRESSED_RGB_ETC1: u32 = 0x8D64;
pub const COMPRESSED_R11_EAC: u32 = 0x9270;
pub const COMPRESSED_SIGNED_R11_EAC: u32 = 0x9271;
pub const COMPRESSED_RG11_EAC: u32 = 0x9272;
pub const COMPRESSED_SIGNED_RG11_EAC: u32 = 0x9273;
pub const COMPRESSED_RGB8_ETC2: u32 = 0x9274;
pub const COMPRESSED_SRGB8_ETC2: u32 = 0x9275;
pub const COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2: u32 = 0x9276;
pub const COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2: u32 = 0x9277;
pub const COMPRESSED_RGBA8_ETC2_EAC: u32 = 0x9278;
pub const COMPRESSED_SRGB8_ALPHA8_ETC2_EAC: u32 = 0x9279;
pub const COMPRESSED_RED_RGTC1: u32 = 0x8DBB;
pub const COMPRESSED_SIGNED_RED_RGTC1: u32 = 0x8DBC;
pub const COMPRESSED_RED_GREEN_RGTC2: u32 = 0x8DBD;
pub const COMPRESSED_SIGNED_RED_GREEN_RGTC2: u32 = 0x8DBE;

/// Which extension a format belongs to.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Family {
    Etc1,
    Etc,
    Rgtc,
}

/// Storage format, bytes per block and extension family.
pub fn format(internal: u32) -> Option<(Fmt, usize, Family)> {
    use Family::*;
    let (layout, num, block, fam) = match internal {
        COMPRESSED_RGB_ETC1 => (e::RGB, Num::Unorm(8), 8, Etc1),
        COMPRESSED_RGB8_ETC2 | COMPRESSED_SRGB8_ETC2 => (e::RGB, Num::Unorm(8), 8, Etc),
        COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 | COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 => (e::RGBA, Num::Unorm(8), 8, Etc),
        COMPRESSED_RGBA8_ETC2_EAC | COMPRESSED_SRGB8_ALPHA8_ETC2_EAC => (e::RGBA, Num::Unorm(8), 16, Etc),
        COMPRESSED_R11_EAC => (e::RED, Num::Unorm(0), 8, Etc),
        COMPRESSED_SIGNED_R11_EAC => (e::RED, Num::Snorm, 8, Etc),
        COMPRESSED_RG11_EAC => (e::RG, Num::Unorm(0), 16, Etc),
        COMPRESSED_SIGNED_RG11_EAC => (e::RG, Num::Snorm, 16, Etc),
        COMPRESSED_RED_RGTC1 => (e::RED, Num::Unorm(8), 8, Rgtc),
        COMPRESSED_SIGNED_RED_RGTC1 => (e::RED, Num::Snorm, 8, Rgtc),
        COMPRESSED_RED_GREEN_RGTC2 => (e::RG, Num::Unorm(8), 16, Rgtc),
        COMPRESSED_SIGNED_RED_GREEN_RGTC2 => (e::RG, Num::Snorm, 16, Rgtc),
        _ => return None,
    };
    let bits = match layout {
        e::RED => [8, 0, 0, 0, 0, 0],
        e::RG => [8, 8, 0, 0, 0, 0],
        e::RGB => [8, 8, 8, 0, 0, 0],
        _ => [8, 8, 8, 8, 0, 0],
    };
    Some((Fmt { internal, layout, num, depth: false, stencil: false, bits }, block, fam))
}

/// Decode a w×h image (whole blocks are read; texels outside w×h are dropped).
pub fn decode(data: &[u8], w: u32, h: u32, internal: u32) -> Option<Vec<[f32; 4]>> {
    let (fmt, block, _) = format(internal)?;
    let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
    if data.len() < bw * bh * block {
        return None;
    }
    let mut px = vec![[0.0, 0.0, 0.0, 1.0]; (w * h) as usize];
    for by in 0..bh {
        for bx in 0..bw {
            let b = &data[(by * bw + bx) * block..(by * bw + bx + 1) * block];
            let texels = decode_block(b, internal, &fmt);
            for y in 0..4 {
                for x in 0..4 {
                    let (tx, ty) = (bx * 4 + x, by * 4 + y);
                    if tx < w as usize && ty < h as usize {
                        px[ty * w as usize + tx] = texels[y * 4 + x];
                    }
                }
            }
        }
    }
    Some(px)
}

/// One block's 16 texels, row by row.
fn decode_block(b: &[u8], internal: u32, fmt: &Fmt) -> [[f32; 4]; 16] {
    let mut out = [[0.0, 0.0, 0.0, 1.0]; 16];
    let signed = matches!(fmt.num, Num::Snorm);
    match internal {
        COMPRESSED_R11_EAC | COMPRESSED_SIGNED_R11_EAC => {
            let r = eac11(b, signed);
            (0..16).for_each(|i| out[i][0] = r[i]);
        }
        COMPRESSED_RG11_EAC | COMPRESSED_SIGNED_RG11_EAC => {
            let (r, g) = (eac11(&b[..8], signed), eac11(&b[8..], signed));
            (0..16).for_each(|i| (out[i][0], out[i][1]) = (r[i], g[i]));
        }
        COMPRESSED_RED_RGTC1 | COMPRESSED_SIGNED_RED_RGTC1 => {
            let r = bc4(b, signed);
            (0..16).for_each(|i| out[i][0] = r[i]);
        }
        COMPRESSED_RED_GREEN_RGTC2 | COMPRESSED_SIGNED_RED_GREEN_RGTC2 => {
            let (r, g) = (bc4(&b[..8], signed), bc4(&b[8..], signed));
            (0..16).for_each(|i| (out[i][0], out[i][1]) = (r[i], g[i]));
        }
        COMPRESSED_RGBA8_ETC2_EAC | COMPRESSED_SRGB8_ALPHA8_ETC2_EAC => {
            let a = eac_alpha(&b[..8]);
            let c = etc_rgb(&b[8..], internal != COMPRESSED_RGB_ETC1, false);
            (0..16).for_each(|i| out[i] = [c[i][0], c[i][1], c[i][2], a[i]]);
        }
        COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 | COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 => {
            out = etc_rgb(b, true, true);
        }
        _ => {
            let c = etc_rgb(b, internal != COMPRESSED_RGB_ETC1, false);
            (0..16).for_each(|i| out[i] = [c[i][0], c[i][1], c[i][2], 1.0]);
        }
    }
    out
}

const ETC_MODIFIERS: [[i32; 2]; 8] = [[2, 8], [5, 17], [9, 29], [13, 42], [18, 60], [24, 80], [33, 106], [47, 183]];
const ETC_DISTANCES: [i32; 8] = [3, 6, 11, 16, 23, 32, 41, 64];

fn ext4(v: u64) -> i32 {
    (v as i32 & 15) * 17
}
fn ext5(v: i32) -> i32 {
    (v << 3) | (v >> 2)
}
fn ext6(v: u64) -> i32 {
    let v = v as i32 & 63;
    (v << 2) | (v >> 4)
}
fn ext7(v: u64) -> i32 {
    let v = v as i32 & 127;
    (v << 1) | (v >> 6)
}

/// An ETC1/ETC2 RGB block (8 bytes, big-endian). `etc2` allows the T, H and planar modes;
/// `punch` is RGB8_PUNCHTHROUGH_ALPHA1 (the differential bit is the "opaque" flag).
fn etc_rgb(b: &[u8], etc2: bool, punch: bool) -> [[f32; 4]; 16] {
    let v = u64::from_be_bytes(b[..8].try_into().unwrap());
    let bits = |hi: u32, lo: u32| (v >> lo) & ((1u64 << (hi - lo + 1)) - 1);
    let flip = bits(32, 32) == 1;
    let diff = bits(33, 33) == 1;
    let opaque = !punch || diff;
    // Pixel index (0..3) of texel x, y: column-major, MSB plane in bits 31..16.
    let index = |x: usize, y: usize| {
        let i = x * 4 + y;
        (((v >> (16 + i)) & 1) << 1 | ((v >> i) & 1)) as usize
    };
    let to = |c: [i32; 3], a: f32| [c[0].clamp(0, 255) as f32 / 255.0, c[1].clamp(0, 255) as f32 / 255.0, c[2].clamp(0, 255) as f32 / 255.0, a];
    let mut out = [[0.0f32; 4]; 16];
    if punch || diff {
        let r = bits(63, 59) as i32;
        let g = bits(55, 51) as i32;
        let bl = bits(47, 43) as i32;
        let d = |hi: u32, lo: u32| ((bits(hi, lo) as i32) << 29) >> 29;
        let (r2, g2, b2) = (r + d(58, 56), g + d(50, 48), bl + d(42, 40));
        if etc2 && !(0..32).contains(&r2) {
            // T mode.
            let c1 = [ext4(bits(60, 59) << 2 | bits(57, 56)), ext4(bits(55, 52)), ext4(bits(51, 48))];
            let c2 = [ext4(bits(47, 44)), ext4(bits(43, 40)), ext4(bits(39, 36))];
            let dist = ETC_DISTANCES[(bits(35, 34) << 1 | bits(32, 32)) as usize];
            let paint = [c1, c2.map(|c| c + dist), c2, c2.map(|c| c - dist)];
            return paint_block(&paint, index, opaque, to);
        }
        if etc2 && !(0..32).contains(&g2) {
            // H mode.
            let c1 = [ext4(bits(62, 59)), ext4(bits(58, 56) << 1 | bits(52, 52)), ext4(bits(51, 51) << 3 | bits(49, 47))];
            let c2 = [ext4(bits(46, 43)), ext4(bits(42, 39)), ext4(bits(38, 35))];
            let key = |c: [i32; 3]| (c[0] << 16) | (c[1] << 8) | c[2];
            let di = (bits(34, 34) << 2 | bits(32, 32) << 1) as usize | (key(c1) >= key(c2)) as usize;
            let dist = ETC_DISTANCES[di];
            let paint = [c1.map(|c| c + dist), c1.map(|c| c - dist), c2.map(|c| c + dist), c2.map(|c| c - dist)];
            return paint_block(&paint, index, opaque, to);
        }
        if etc2 && !(0..32).contains(&b2) {
            // Planar mode: a gradient from three colors.
            let o = [ext6(bits(62, 57)), ext7(bits(56, 56) << 6 | bits(54, 49)), ext6(bits(48, 48) << 5 | bits(44, 43) << 3 | bits(41, 39))];
            let hc = [ext6(bits(38, 34) << 1 | bits(32, 32)), ext7(bits(31, 25)), ext6(bits(24, 19))];
            let vc = [ext6(bits(18, 13)), ext7(bits(12, 6)), ext6(bits(5, 0))];
            for y in 0..4 {
                for x in 0..4 {
                    let c: [i32; 3] = std::array::from_fn(|k| (x as i32 * (hc[k] - o[k]) + y as i32 * (vc[k] - o[k]) + 4 * o[k] + 2) >> 2);
                    out[y * 4 + x] = to(c, 1.0);
                }
            }
            return out;
        }
        let bases = [[ext5(r), ext5(g), ext5(bl)], [ext5(r2.clamp(0, 31)), ext5(g2.clamp(0, 31)), ext5(b2.clamp(0, 31))]];
        return individual(bases, [bits(39, 37) as usize, bits(36, 34) as usize], flip, index, opaque, to);
    }
    let bases = [[ext4(bits(63, 60)), ext4(bits(55, 52)), ext4(bits(47, 44))], [ext4(bits(59, 56)), ext4(bits(51, 48)), ext4(bits(43, 40))]];
    individual(bases, [bits(39, 37) as usize, bits(36, 34) as usize], flip, index, true, to)
}

/// The two-sub-block modes (individual and differential): base color plus a table modifier.
fn individual(bases: [[i32; 3]; 2], tables: [usize; 2], flip: bool, index: impl Fn(usize, usize) -> usize, opaque: bool, to: impl Fn([i32; 3], f32) -> [f32; 4]) -> [[f32; 4]; 16] {
    let mut out = [[0.0f32; 4]; 16];
    for y in 0..4 {
        for x in 0..4 {
            let sub = if flip { (y >= 2) as usize } else { (x >= 2) as usize };
            let [a, bm] = ETC_MODIFIERS[tables[sub]];
            let idx = index(x, y);
            if !opaque && idx == 2 {
                out[y * 4 + x] = [0.0; 4];
                continue;
            }
            // Punch-through blocks without the opaque bit use 0 for index 0's modifier.
            let m = match idx {
                0 if !opaque => 0,
                0 => a,
                1 => bm,
                2 => -a,
                _ => -bm,
            };
            out[y * 4 + x] = to(bases[sub].map(|c| c + m), 1.0);
        }
    }
    out
}

fn paint_block(paint: &[[i32; 3]; 4], index: impl Fn(usize, usize) -> usize, opaque: bool, to: impl Fn([i32; 3], f32) -> [f32; 4]) -> [[f32; 4]; 16] {
    let mut out = [[0.0f32; 4]; 16];
    for y in 0..4 {
        for x in 0..4 {
            let idx = index(x, y);
            out[y * 4 + x] = if !opaque && idx == 2 { [0.0; 4] } else { to(paint[idx], 1.0) };
        }
    }
    out
}

const EAC_TABLES: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

/// The 3-bit index of texel x, y in an EAC block (column-major, first texel in the top bits).
fn eac_index(v: u64, x: usize, y: usize) -> usize {
    ((v >> (45 - 3 * (x * 4 + y))) & 7) as usize
}

fn eac_alpha(b: &[u8]) -> [f32; 16] {
    let v = u64::from_be_bytes(b[..8].try_into().unwrap());
    let (base, mult, table) = (b[0] as i32, (b[1] >> 4) as i32, &EAC_TABLES[(b[1] & 15) as usize]);
    std::array::from_fn(|i| {
        let (x, y) = (i % 4, i / 4);
        (base + table[eac_index(v, x, y)] * mult).clamp(0, 255) as f32 / 255.0
    })
}

/// An 11-bit EAC channel (R11 / RG11, unsigned or signed).
fn eac11(b: &[u8], signed: bool) -> [f32; 16] {
    let v = u64::from_be_bytes(b[..8].try_into().unwrap());
    let mult = (b[1] >> 4) as i32;
    let table = &EAC_TABLES[(b[1] & 15) as usize];
    std::array::from_fn(|i| {
        let (x, y) = (i % 4, i / 4);
        let m = table[eac_index(v, x, y)];
        let step = if mult == 0 { m } else { m * mult * 8 };
        if signed {
            let base = (b[0] as i8 as i32).max(-127);
            (base * 8 + step).clamp(-1023, 1023) as f32 / 1023.0
        } else {
            (b[0] as i32 * 8 + 4 + step).clamp(0, 2047) as f32 / 2047.0
        }
    })
}

/// A BC4 (RGTC1) channel: two endpoints and 3-bit indices, like DXT5 alpha.
fn bc4(b: &[u8], signed: bool) -> [f32; 16] {
    let (e0, e1) = if signed {
        ((b[0] as i8 as f32).max(-127.0) / 127.0, (b[1] as i8 as f32).max(-127.0) / 127.0)
    } else {
        (b[0] as f32 / 255.0, b[1] as f32 / 255.0)
    };
    let greater = if signed { (b[0] as i8) > (b[1] as i8) } else { b[0] > b[1] };
    let mut pal = [e0, e1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    if greater {
        for k in 1..7 {
            pal[k + 1] = ((7 - k) as f32 * e0 + k as f32 * e1) / 7.0;
        }
    } else {
        for k in 1..5 {
            pal[k + 1] = ((5 - k) as f32 * e0 + k as f32 * e1) / 5.0;
        }
        pal[6] = if signed { -1.0 } else { 0.0 };
        pal[7] = 1.0;
    }
    let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
    std::array::from_fn(|i| pal[((bits >> (3 * i)) & 7) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etc1_individual_block() {
        // Individual mode, both sub-blocks base 0x88 gray, table 0, all indices 0 (+2).
        let b = [0x88, 0x88, 0x88, 0x00, 0, 0, 0, 0];
        let px = decode(&b, 4, 4, COMPRESSED_RGB_ETC1).unwrap();
        assert!(px.iter().all(|p| (p[0] * 255.0).round() == 138.0 && p[3] == 1.0));
    }

    #[test]
    fn eac_alpha_and_r11() {
        // Base 128, multiplier 1, table 0, every index 4 (+2).
        let idx4: u64 = (0..16).fold(0, |a, _| (a << 3) | 4);
        let mut b = [0u8; 8];
        b[0] = 128;
        b[1] = 0x10;
        b[2..].copy_from_slice(&idx4.to_be_bytes()[2..]);
        assert!(eac_alpha(&b).iter().all(|&a| (a * 255.0).round() == 130.0));
        let r = eac11(&b, false);
        assert!(r.iter().all(|&v| (v - (128.0 * 8.0 + 4.0 + 16.0) / 2047.0).abs() < 1e-6));
    }

    #[test]
    fn rgtc1_endpoints() {
        // e0 = 255 > e1 = 0: texel 0 has index 0 (1.0), texel 1 index 2 (6/7), texel 2 index 1.
        let b = [255, 0, 0b0101_0000, 0, 0, 0, 0, 0];
        let r = bc4(&b, false);
        assert_eq!(r[0], 1.0);
        assert!((r[1] - 6.0 / 7.0).abs() < 1e-6);
        assert_eq!(r[2], 0.0);
    }
}
