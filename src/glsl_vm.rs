//! The virtual machine that runs compiled GLSL shaders (see `glsl_gen`).
//!
//! Every value slot holds four lanes, so one run shades a 2×2 quad of fragments (or four
//! vertices) in lockstep, as a GPU does. That's what gives `dFdx`/`dFdy` and texture level
//! of detail their meaning: lane 1 is the pixel right of lane 0, lane 2 the one below.
//! Branches that disagree between lanes run both ways under an execution mask; `break`,
//! `continue`, `return` and `discard` take lanes out of the mask until their construct ends.
//!
//! Slots are 32 bits: floats as `f32`, ints and uints as their bits, bools as 0/1 ints.

use crate::glsl::Samp;

pub const LANES: usize = 4;
pub type Slot = u32;
pub type Lanes = [f32; LANES];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum F1 {
    FNeg,
    INeg,
    Not,
    BitNot,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    FAbs,
    IAbs,
    FSign,
    ISign,
    Floor,
    Ceil,
    Fract,
    Trunc,
    Round,
    RoundEven,
    Radians,
    Degrees,
    Exp,
    Log,
    Exp2,
    Log2,
    Sqrt,
    InvSqrt,
    IsNan,
    IsInf,
    F2I,
    F2U,
    I2F,
    U2F,
    B2F,
    F2B,
    I2B,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum F2 {
    FAdd,
    FSub,
    FMul,
    FDiv,
    IAdd,
    ISub,
    IMul,
    IDiv,
    IMod,
    UDiv,
    UMod,
    UMul,
    Shl,
    IShr,
    UShr,
    And,
    Or,
    Xor,
    FLt,
    FLe,
    FGt,
    FGe,
    ILt,
    ILe,
    IGt,
    IGe,
    ULt,
    ULe,
    UGt,
    UGe,
    FEq,
    FNe,
    IEq,
    INe,
    Pow,
    Atan2,
    FMod,
    FMin,
    FMax,
    IMin,
    IMax,
    UMin,
    UMax,
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum F3 {
    FClamp,
    IClamp,
    UClamp,
    Mix,
    /// mix(x, y, bvec): y where the bool is set.
    Select,
    Smoothstep,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TexKind {
    /// texture()/texture2D()/textureCube(); `arg` is the bias slot (or none).
    Sample,
    Lod,
    Grad,
    /// texelFetch(sampler, ivec, lod).
    Fetch,
    Size,
}

#[derive(Clone, Debug)]
pub struct TexOp {
    pub d: Slot,
    pub samp: Slot,
    pub samp_ty: Samp,
    pub coord: Slot,
    /// Components in `coord` (including the array layer / projective divisor / compare value).
    pub ncoord: u8,
    pub kind: TexKind,
    /// Divide by the last coordinate first (textureProj).
    pub proj: bool,
    /// Bias, explicit lod, or texelFetch lod (u32::MAX = none).
    pub arg: Slot,
    pub dx: Slot,
    pub dy: Slot,
    /// A constant texel offset (textureOffset), or none.
    pub offset: Option<[i32; 3]>,
    /// The compare value's slot for shadow samplers (u32::MAX = the last coordinate).
    pub compare: Slot,
}

pub const NONE: Slot = u32::MAX;

#[derive(Clone, Debug)]
pub enum Op {
    Mov { d: Slot, s: Slot, n: u32 },
    Splat { d: Slot, s: Slot, n: u32 },
    /// d[i] = s[i] for each listed source slot.
    Gather { d: Slot, s: Box<[Slot]> },
    /// d[i] = s + i for each listed destination slot.
    Scatter { d: Box<[Slot]>, s: Slot },
    Un { f: F1, d: Slot, a: Slot, n: u32 },
    /// Elementwise; `sa`/`sb` are 0 for a scalar operand used for every component.
    Bin { f: F2, d: Slot, a: Slot, b: Slot, n: u32, sa: u8, sb: u8 },
    Tri { f: F3, d: Slot, a: Slot, b: Slot, c: Slot, n: u32, sa: u8, sb: u8, sc: u8 },
    Dot { d: Slot, a: Slot, b: Slot, n: u32 },
    Cross { d: Slot, a: Slot, b: Slot },
    Length { d: Slot, a: Slot, n: u32 },
    Normalize { d: Slot, a: Slot, n: u32 },
    Reflect { d: Slot, a: Slot, b: Slot, n: u32 },
    Refract { d: Slot, a: Slot, b: Slot, eta: Slot, n: u32 },
    Faceforward { d: Slot, a: Slot, b: Slot, c: Slot, n: u32 },
    /// d (ar × bc) = a (ar × inner) · b (inner × bc), column-major.
    MatMul { d: Slot, a: Slot, b: Slot, ar: u8, inner: u8, bc: u8 },
    Transpose { d: Slot, a: Slot, c: u8, r: u8 },
    Det { d: Slot, a: Slot, n: u8 },
    Inverse { d: Slot, a: Slot, n: u8 },
    /// any()/all() over n bools.
    Any { d: Slot, a: Slot, n: u32 },
    All { d: Slot, a: Slot, n: u32 },
    /// d = base[clamp(idx) * stride .. + n]
    LoadDyn { d: Slot, base: Slot, idx: Slot, stride: u32, count: u32, n: u32 },
    /// base[clamp(idx) * stride + offs[i]] = s + i
    StoreDyn { base: Slot, idx: Slot, stride: u32, count: u32, offs: Box<[u32]>, s: Slot },
    Tex(Box<TexOp>),
    /// 0 = dFdx, 1 = dFdy, 2 = fwidth.
    Deriv { d: Slot, a: Slot, n: u32, kind: u8 },
    /// Bit-level helpers for 3.00 (packHalf2x16 and friends).
    Pack { d: Slot, a: Slot, kind: u8 },
    Unpack { d: Slot, a: Slot, kind: u8 },
    Modf { d: Slot, i: Slot, a: Slot, n: u32 },
    If { c: Slot, else_pc: u32 },
    Else { end_pc: u32 },
    EndIf,
    LoopBegin,
    LoopCond { c: Slot, end_pc: u32 },
    LoopCont,
    LoopBack { pc: u32 },
    LoopEnd,
    Break,
    Continue,
    FnBegin,
    Return,
    FnEnd,
    Discard,
    SwBegin,
    SwCase { c: Slot },
    SwEnd,
}

/// Texture lookups, answered by the WebGL context.
pub struct TexArgs {
    pub coord: [f32; 4],
    pub dx: [f32; 4],
    pub dy: [f32; 4],
    /// An explicit level of detail (textureLod), or None to use the derivatives.
    pub lod: Option<f32>,
    pub bias: f32,
    pub compare: Option<f32>,
    pub offset: [i32; 3],
}

pub trait TexEnv {
    fn sample(&self, unit: i32, samp: Samp, a: &TexArgs) -> [f32; 4];
    fn fetch(&self, unit: i32, samp: Samp, coord: [i32; 3], lod: i32) -> [f32; 4];
    fn size(&self, unit: i32, samp: Samp, lod: i32) -> [i32; 3];
}

/// Nothing bound: every lookup reads transparent black (tests).
#[cfg(test)]
pub struct NoTex;
#[cfg(test)]
impl TexEnv for NoTex {
    fn sample(&self, _: i32, _: Samp, _: &TexArgs) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
    fn fetch(&self, _: i32, _: Samp, _: [i32; 3], _: i32) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
    fn size(&self, _: i32, _: Samp, _: i32) -> [i32; 3] {
        [0, 0, 0]
    }
}

#[derive(Clone, Copy)]
struct Entry {
    kind: u8,
    restore: u8,
    other: u8,
    brk: u8,
    cont: u8,
}
const K_IF: u8 = 0;
const K_LOOP: u8 = 1;
const K_FN: u8 = 2;
const K_SW: u8 = 3;

#[inline]
fn bits(x: f32) -> u32 {
    x.to_bits()
}
#[inline]
fn fb(x: u32) -> f32 {
    f32::from_bits(x)
}
#[inline]
fn ib(x: i32) -> f32 {
    f32::from_bits(x as u32)
}
#[inline]
fn bi(x: f32) -> i32 {
    x.to_bits() as i32
}
#[inline]
fn bool_f(b: bool) -> f32 {
    f32::from_bits(b as u32)
}
#[inline]
fn truthy(x: f32) -> bool {
    x.to_bits() != 0
}

/// The most loop iterations one run may take before its lanes are stopped.
const MAX_ITER: u32 = 1 << 20;

/// Run a shader over the lanes in `mask`; returns the lanes that weren't discarded.
pub fn run(code: &[Op], mem: &mut [Lanes], mask: u8, env: &dyn TexEnv, fragment: bool) -> u8 {
    let mut m = mask;
    let mut alive = mask;
    let mut stack: Vec<Entry> = Vec::with_capacity(16);
    let mut pc = 0usize;
    let mut iters = 0u32;
    while pc < code.len() {
        let op = &code[pc];
        pc += 1;
        match op {
            Op::If { c, else_pc } => {
                let cm = lane_mask(&mem[*c as usize]);
                stack.push(Entry { kind: K_IF, restore: m, other: m & !cm, brk: 0, cont: 0 });
                m &= cm;
                if m == 0 {
                    pc = *else_pc as usize;
                }
                continue;
            }
            Op::Else { end_pc } => {
                let e = stack.last().expect("else");
                m = e.other;
                if m == 0 {
                    pc = *end_pc as usize;
                }
                continue;
            }
            Op::EndIf | Op::LoopEnd | Op::FnEnd | Op::SwEnd => {
                let e = stack.pop().expect("end");
                m = e.restore & alive;
                continue;
            }
            Op::LoopBegin => {
                stack.push(Entry { kind: K_LOOP, restore: m, other: 0, brk: 0, cont: 0 });
                continue;
            }
            Op::LoopCond { c, end_pc } => {
                m &= lane_mask(&mem[*c as usize]);
                if m == 0 {
                    pc = *end_pc as usize;
                }
                continue;
            }
            Op::LoopCont => {
                let e = stack.last_mut().expect("loop");
                m = (m | e.cont) & alive;
                e.cont = 0;
                continue;
            }
            Op::LoopBack { pc: to } => {
                iters += 1;
                if iters > MAX_ITER {
                    // A runaway loop: stop every lane.
                    for e in stack.iter_mut() {
                        e.restore = 0;
                        e.other = 0;
                    }
                    alive = 0;
                    m = 0;
                }
                if m != 0 {
                    pc = *to as usize;
                }
                continue;
            }
            Op::FnBegin => {
                stack.push(Entry { kind: K_FN, restore: m, other: 0, brk: 0, cont: 0 });
                continue;
            }
            Op::SwBegin => {
                stack.push(Entry { kind: K_SW, restore: m, other: m, brk: 0, cont: 0 });
                m = 0;
                continue;
            }
            Op::SwCase { c } => {
                let e = stack.last_mut().expect("switch");
                let cm = lane_mask(&mem[*c as usize]) & e.other;
                e.other &= !cm;
                m |= cm;
                continue;
            }
            Op::Break | Op::Continue => {
                let is_break = matches!(op, Op::Break);
                for e in stack.iter_mut().rev() {
                    if e.kind == K_LOOP || (e.kind == K_SW && is_break) {
                        if is_break {
                            e.brk |= m;
                        } else {
                            e.cont |= m;
                        }
                        break;
                    }
                    e.restore &= !m;
                    e.other &= !m;
                }
                m = 0;
                continue;
            }
            Op::Return => {
                for e in stack.iter_mut().rev() {
                    if e.kind == K_FN {
                        break;
                    }
                    e.restore &= !m;
                    e.other &= !m;
                }
                m = 0;
                continue;
            }
            Op::Discard => {
                alive &= !m;
                for e in stack.iter_mut() {
                    e.restore &= !m;
                    e.other &= !m;
                }
                m = 0;
                continue;
            }
            _ => {}
        }
        if m == 0 {
            continue;
        }
        exec(op, mem, m, env, fragment);
    }
    alive
}

#[inline]
fn lane_mask(l: &Lanes) -> u8 {
    let mut m = 0;
    for (i, v) in l.iter().enumerate() {
        if truthy(*v) {
            m |= 1 << i;
        }
    }
    m
}

#[inline]
fn put(mem: &mut [Lanes], d: usize, v: Lanes, m: u8) {
    if m == 0xF {
        mem[d] = v;
    } else {
        let t = &mut mem[d];
        for l in 0..LANES {
            if m & (1 << l) != 0 {
                t[l] = v[l];
            }
        }
    }
}

#[inline]
fn un(mem: &mut [Lanes], d: Slot, a: Slot, n: u32, m: u8, f: impl Fn(f32) -> f32) {
    for k in 0..n as usize {
        let x = mem[a as usize + k];
        put(mem, d as usize + k, x.map(&f), m);
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn bin(mem: &mut [Lanes], d: Slot, a: Slot, b: Slot, n: u32, sa: u8, sb: u8, m: u8, f: impl Fn(f32, f32) -> f32) {
    for k in 0..n as usize {
        let x = mem[a as usize + k * sa as usize];
        let y = mem[b as usize + k * sb as usize];
        let v = [f(x[0], y[0]), f(x[1], y[1]), f(x[2], y[2]), f(x[3], y[3])];
        put(mem, d as usize + k, v, m);
    }
}

fn fsign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn f1(f: F1) -> fn(f32) -> f32 {
    use F1::*;
    match f {
        FNeg => |x| -x,
        INeg => |x| ib(bi(x).wrapping_neg()),
        Not => |x| fb(bits(x) ^ 1),
        BitNot => |x| fb(!bits(x)),
        Sin => f32::sin,
        Cos => f32::cos,
        Tan => f32::tan,
        Asin => f32::asin,
        Acos => f32::acos,
        Atan => f32::atan,
        Sinh => f32::sinh,
        Cosh => f32::cosh,
        Tanh => f32::tanh,
        Asinh => f32::asinh,
        Acosh => f32::acosh,
        Atanh => f32::atanh,
        FAbs => f32::abs,
        IAbs => |x| ib(bi(x).wrapping_abs()),
        FSign => fsign,
        ISign => |x| ib(bi(x).signum()),
        Floor => f32::floor,
        Ceil => f32::ceil,
        Fract => |x| x - x.floor(),
        Trunc => f32::trunc,
        Round => f32::round,
        RoundEven => f32::round_ties_even,
        Radians => |x| x * (std::f32::consts::PI / 180.0),
        Degrees => |x| x * (180.0 / std::f32::consts::PI),
        Exp => f32::exp,
        Log => f32::ln,
        Exp2 => f32::exp2,
        Log2 => f32::log2,
        Sqrt => f32::sqrt,
        InvSqrt => |x| 1.0 / x.sqrt(),
        IsNan => |x| bool_f(x.is_nan()),
        IsInf => |x| bool_f(x.is_infinite()),
        F2I => |x| ib(if x.is_nan() { 0 } else { x as i32 }),
        F2U => |x| fb(if x < 0.0 { (x as i32) as u32 } else { x as u32 }),
        I2F => |x| bi(x) as f32,
        U2F => |x| bits(x) as f32,
        B2F => |x| if truthy(x) { 1.0 } else { 0.0 },
        F2B => |x| bool_f(x != 0.0),
        I2B => |x| bool_f(bits(x) != 0),
    }
}

fn f2(f: F2) -> fn(f32, f32) -> f32 {
    use F2::*;
    match f {
        FAdd => |a, b| a + b,
        FSub => |a, b| a - b,
        FMul => |a, b| a * b,
        FDiv => |a, b| a / b,
        IAdd => |a, b| ib(bi(a).wrapping_add(bi(b))),
        ISub => |a, b| ib(bi(a).wrapping_sub(bi(b))),
        IMul => |a, b| ib(bi(a).wrapping_mul(bi(b))),
        UMul => |a, b| fb(bits(a).wrapping_mul(bits(b))),
        IDiv => |a, b| ib(if bi(b) == 0 { 0 } else { bi(a).wrapping_div(bi(b)) }),
        IMod => |a, b| ib(if bi(b) == 0 { 0 } else { bi(a).wrapping_rem(bi(b)) }),
        UDiv => |a, b| fb(if bits(b) == 0 { u32::MAX } else { bits(a) / bits(b) }),
        UMod => |a, b| fb(if bits(b) == 0 { 0 } else { bits(a) % bits(b) }),
        Shl => |a, b| fb(bits(a).wrapping_shl(bits(b))),
        IShr => |a, b| ib(bi(a).wrapping_shr(bits(b))),
        UShr => |a, b| fb(bits(a).wrapping_shr(bits(b))),
        And => |a, b| fb(bits(a) & bits(b)),
        Or => |a, b| fb(bits(a) | bits(b)),
        Xor => |a, b| fb(bits(a) ^ bits(b)),
        FLt => |a, b| bool_f(a < b),
        FLe => |a, b| bool_f(a <= b),
        FGt => |a, b| bool_f(a > b),
        FGe => |a, b| bool_f(a >= b),
        ILt => |a, b| bool_f(bi(a) < bi(b)),
        ILe => |a, b| bool_f(bi(a) <= bi(b)),
        IGt => |a, b| bool_f(bi(a) > bi(b)),
        IGe => |a, b| bool_f(bi(a) >= bi(b)),
        ULt => |a, b| bool_f(bits(a) < bits(b)),
        ULe => |a, b| bool_f(bits(a) <= bits(b)),
        UGt => |a, b| bool_f(bits(a) > bits(b)),
        UGe => |a, b| bool_f(bits(a) >= bits(b)),
        FEq => |a, b| bool_f(a == b),
        FNe => |a, b| bool_f(a != b),
        IEq => |a, b| bool_f(bits(a) == bits(b)),
        INe => |a, b| bool_f(bits(a) != bits(b)),
        Pow => f32::powf,
        Atan2 => f32::atan2,
        FMod => |a, b| a - b * (a / b).floor(),
        FMin => |a, b| if b < a { b } else { a },
        FMax => |a, b| if a < b { b } else { a },
        IMin => |a, b| ib(bi(a).min(bi(b))),
        IMax => |a, b| ib(bi(a).max(bi(b))),
        UMin => |a, b| fb(bits(a).min(bits(b))),
        UMax => |a, b| fb(bits(a).max(bits(b))),
        Step => |edge, x| if x < edge { 0.0 } else { 1.0 },
    }
}

fn f3(f: F3) -> fn(f32, f32, f32) -> f32 {
    use F3::*;
    match f {
        FClamp => |x, lo, hi| x.max(lo).min(hi),
        IClamp => |x, lo, hi| ib(bi(x).max(bi(lo)).min(bi(hi))),
        UClamp => |x, lo, hi| fb(bits(x).max(bits(lo)).min(bits(hi))),
        Mix => |x, y, a| x * (1.0 - a) + y * a,
        Select => |x, y, c| if truthy(c) { y } else { x },
        Smoothstep => |e0, e1, x| {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        },
    }
}

/// Gather `n` slots starting at `a` into per-lane arrays.
fn vecs(mem: &[Lanes], a: Slot, n: u32) -> [[f32; 4]; LANES] {
    let mut out = [[0.0; 4]; LANES];
    for k in 0..n.min(4) as usize {
        let s = mem[a as usize + k];
        for l in 0..LANES {
            out[l][k] = s[l];
        }
    }
    out
}

fn put_vecs(mem: &mut [Lanes], d: Slot, n: u32, v: &[[f32; 4]; LANES], m: u8) {
    for k in 0..n.min(4) as usize {
        put(mem, d as usize + k, [v[0][k], v[1][k], v[2][k], v[3][k]], m);
    }
}

fn dot(a: &[f32; 4], b: &[f32; 4], n: usize) -> f32 {
    let mut s = 0.0;
    for i in 0..n {
        s += a[i] * b[i];
    }
    s
}

fn det(a: &[f32], n: usize) -> f32 {
    match n {
        2 => a[0] * a[3] - a[2] * a[1],
        3 => {
            a[0] * (a[4] * a[8] - a[7] * a[5]) - a[3] * (a[1] * a[8] - a[7] * a[2]) + a[6] * (a[1] * a[5] - a[4] * a[2])
        }
        _ => {
            let mut s = 0.0;
            for c in 0..4 {
                let mut minor = [0.0f32; 9];
                let mut k = 0;
                for cc in 0..4 {
                    if cc == c {
                        continue;
                    }
                    for r in 1..4 {
                        minor[k] = a[cc * 4 + r];
                        k += 1;
                    }
                }
                // minor is column-major 3x3 from the remaining columns and rows 1..3.
                let mm = [minor[0], minor[1], minor[2], minor[3], minor[4], minor[5], minor[6], minor[7], minor[8]];
                let sign = if c % 2 == 0 { 1.0 } else { -1.0 };
                s += sign * a[c * 4] * det(&mm, 3);
            }
            s
        }
    }
}

fn inverse(a: &[f32], n: usize) -> Vec<f32> {
    // Gauss-Jordan on a row-major copy.
    let mut m = vec![0.0f64; n * 2 * n];
    for r in 0..n {
        for c in 0..n {
            m[r * 2 * n + c] = a[c * n + r] as f64;
        }
        m[r * 2 * n + n + r] = 1.0;
    }
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n {
            if m[r * 2 * n + col].abs() > m[piv * 2 * n + col].abs() {
                piv = r;
            }
        }
        if m[piv * 2 * n + col] == 0.0 {
            return vec![f32::NAN; n * n];
        }
        if piv != col {
            for k in 0..2 * n {
                m.swap(piv * 2 * n + k, col * 2 * n + k);
            }
        }
        let p = m[col * 2 * n + col];
        for k in 0..2 * n {
            m[col * 2 * n + k] /= p;
        }
        for r in 0..n {
            if r != col {
                let f = m[r * 2 * n + col];
                if f != 0.0 {
                    for k in 0..2 * n {
                        m[r * 2 * n + k] -= f * m[col * 2 * n + k];
                    }
                }
            }
        }
    }
    let mut out = vec![0.0f32; n * n];
    for r in 0..n {
        for c in 0..n {
            out[c * n + r] = m[r * 2 * n + n + c] as f32;
        }
    }
    out
}

fn half_from_f32(f: f32) -> u16 {
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
    // Round to nearest.
    (h + ((mant >> 12) & 1)) as u16
}

pub fn f32_from_half(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            let mut e = 127 - 15 + 1;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 31 {
        sign | 0x7f80_0000 | (mant << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

fn exec(op: &Op, mem: &mut [Lanes], m: u8, env: &dyn TexEnv, fragment: bool) {
    match op {
        Op::Mov { d, s, n } => {
            if d == s {
                return;
            }
            if d < s {
                for k in 0..*n as usize {
                    let v = mem[*s as usize + k];
                    put(mem, *d as usize + k, v, m);
                }
            } else {
                for k in (0..*n as usize).rev() {
                    let v = mem[*s as usize + k];
                    put(mem, *d as usize + k, v, m);
                }
            }
        }
        Op::Splat { d, s, n } => {
            let v = mem[*s as usize];
            for k in 0..*n as usize {
                put(mem, *d as usize + k, v, m);
            }
        }
        Op::Gather { d, s } => {
            let mut tmp = [[0.0f32; 4]; 16];
            if s.len() <= 16 {
                for (i, src) in s.iter().enumerate() {
                    tmp[i] = mem[*src as usize];
                }
                for i in 0..s.len() {
                    put(mem, *d as usize + i, tmp[i], m);
                }
            } else {
                let v: Vec<Lanes> = s.iter().map(|x| mem[*x as usize]).collect();
                for (i, x) in v.into_iter().enumerate() {
                    put(mem, *d as usize + i, x, m);
                }
            }
        }
        Op::Scatter { d, s } => {
            let v: Vec<Lanes> = (0..d.len()).map(|i| mem[*s as usize + i]).collect();
            for (i, dst) in d.iter().enumerate() {
                put(mem, *dst as usize, v[i], m);
            }
        }
        Op::Un { f, d, a, n } => un(mem, *d, *a, *n, m, f1(*f)),
        Op::Bin { f, d, a, b, n, sa, sb } => bin(mem, *d, *a, *b, *n, *sa, *sb, m, f2(*f)),
        Op::Tri { f, d, a, b, c, n, sa, sb, sc } => {
            let g = f3(*f);
            for k in 0..*n as usize {
                let x = mem[*a as usize + k * *sa as usize];
                let y = mem[*b as usize + k * *sb as usize];
                let z = mem[*c as usize + k * *sc as usize];
                let v = [g(x[0], y[0], z[0]), g(x[1], y[1], z[1]), g(x[2], y[2], z[2]), g(x[3], y[3], z[3])];
                put(mem, *d as usize + k, v, m);
            }
        }
        Op::Dot { d, a, b, n } => {
            let (x, y) = (vecs(mem, *a, *n), vecs(mem, *b, *n));
            let v = [0, 1, 2, 3].map(|l| dot(&x[l], &y[l], *n as usize));
            put(mem, *d as usize, v, m);
        }
        Op::Cross { d, a, b } => {
            let (x, y) = (vecs(mem, *a, 3), vecs(mem, *b, 3));
            let v = [0, 1, 2, 3].map(|l| {
                let (p, q) = (x[l], y[l]);
                [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0], 0.0]
            });
            put_vecs(mem, *d, 3, &v, m);
        }
        Op::Length { d, a, n } => {
            let x = vecs(mem, *a, *n);
            let v = [0, 1, 2, 3].map(|l| dot(&x[l], &x[l], *n as usize).sqrt());
            put(mem, *d as usize, v, m);
        }
        Op::Normalize { d, a, n } => {
            let mut x = vecs(mem, *a, *n);
            for v in x.iter_mut() {
                let len = dot(v, v, *n as usize).sqrt();
                for c in v.iter_mut() {
                    *c /= len;
                }
            }
            put_vecs(mem, *d, *n, &x, m);
        }
        Op::Reflect { d, a, b, n } => {
            let (i, nn) = (vecs(mem, *a, *n), vecs(mem, *b, *n));
            let v = [0, 1, 2, 3].map(|l| {
                let k = 2.0 * dot(&nn[l], &i[l], *n as usize);
                let mut o = [0.0; 4];
                for c in 0..4 {
                    o[c] = i[l][c] - k * nn[l][c];
                }
                o
            });
            put_vecs(mem, *d, *n, &v, m);
        }
        Op::Refract { d, a, b, eta, n } => {
            let (i, nn) = (vecs(mem, *a, *n), vecs(mem, *b, *n));
            let e = mem[*eta as usize];
            let v = [0, 1, 2, 3].map(|l| {
                let dn = dot(&nn[l], &i[l], *n as usize);
                let k = 1.0 - e[l] * e[l] * (1.0 - dn * dn);
                let mut o = [0.0; 4];
                if k >= 0.0 {
                    for c in 0..4 {
                        o[c] = e[l] * i[l][c] - (e[l] * dn + k.sqrt()) * nn[l][c];
                    }
                }
                o
            });
            put_vecs(mem, *d, *n, &v, m);
        }
        Op::Faceforward { d, a, b, c, n } => {
            let (nn, i, nr) = (vecs(mem, *a, *n), vecs(mem, *b, *n), vecs(mem, *c, *n));
            let v = [0, 1, 2, 3].map(|l| {
                let s = if dot(&nr[l], &i[l], *n as usize) < 0.0 { 1.0 } else { -1.0 };
                nn[l].map(|x| x * s)
            });
            put_vecs(mem, *d, *n, &v, m);
        }
        Op::MatMul { d, a, b, ar, inner, bc } => {
            let (ar, inner, bc) = (*ar as usize, *inner as usize, *bc as usize);
            let mut out = [[0.0f32; 4]; 16];
            for j in 0..bc {
                for i in 0..ar {
                    let mut s = [0.0f32; 4];
                    for k in 0..inner {
                        let x = mem[*a as usize + k * ar + i];
                        let y = mem[*b as usize + j * inner + k];
                        for l in 0..LANES {
                            s[l] += x[l] * y[l];
                        }
                    }
                    out[j * ar + i] = s;
                }
            }
            for k in 0..ar * bc {
                put(mem, *d as usize + k, out[k], m);
            }
        }
        Op::Transpose { d, a, c, r } => {
            let (c, r) = (*c as usize, *r as usize);
            let mut out = [[0.0f32; 4]; 16];
            for j in 0..c {
                for i in 0..r {
                    out[i * c + j] = mem[*a as usize + j * r + i];
                }
            }
            for k in 0..c * r {
                put(mem, *d as usize + k, out[k], m);
            }
        }
        Op::Det { d, a, n } => {
            let n = *n as usize;
            let v = [0, 1, 2, 3].map(|l| {
                let x: Vec<f32> = (0..n * n).map(|k| mem[*a as usize + k][l]).collect();
                det(&x, n)
            });
            put(mem, *d as usize, v, m);
        }
        Op::Inverse { d, a, n } => {
            let n = *n as usize;
            let mut out = vec![[0.0f32; 4]; n * n];
            for l in 0..LANES {
                let x: Vec<f32> = (0..n * n).map(|k| mem[*a as usize + k][l]).collect();
                for (k, v) in inverse(&x, n).into_iter().enumerate() {
                    out[k][l] = v;
                }
            }
            for (k, v) in out.into_iter().enumerate() {
                put(mem, *d as usize + k, v, m);
            }
        }
        Op::Any { d, a, n } | Op::All { d, a, n } => {
            let all = matches!(op, Op::All { .. });
            let mut v = [bool_f(all); 4];
            for k in 0..*n as usize {
                let x = mem[*a as usize + k];
                for l in 0..LANES {
                    if all {
                        v[l] = bool_f(truthy(v[l]) && truthy(x[l]));
                    } else {
                        v[l] = bool_f(truthy(v[l]) || truthy(x[l]));
                    }
                }
            }
            put(mem, *d as usize, v, m);
        }
        Op::LoadDyn { d, base, idx, stride, count, n } => {
            let ix = mem[*idx as usize];
            for k in 0..*n as usize {
                let mut v = [0.0; 4];
                for l in 0..LANES {
                    let i = (bi(ix[l]).max(0) as u32).min(count.saturating_sub(1));
                    v[l] = mem[(*base + i * stride) as usize + k][l];
                }
                put(mem, *d as usize + k, v, m);
            }
        }
        Op::StoreDyn { base, idx, stride, count, offs, s } => {
            let ix = mem[*idx as usize];
            for (k, off) in offs.iter().enumerate() {
                let src = mem[*s as usize + k];
                for l in 0..LANES {
                    if m & (1 << l) == 0 {
                        continue;
                    }
                    let i = (bi(ix[l]).max(0) as u32).min(count.saturating_sub(1));
                    mem[(*base + i * stride + off) as usize][l] = src[l];
                }
            }
        }
        Op::Tex(t) => tex(t, mem, m, env, fragment),
        Op::Deriv { d, a, n, kind } => {
            for k in 0..*n as usize {
                let x = mem[*a as usize + k];
                let dx = [x[1] - x[0], x[1] - x[0], x[3] - x[2], x[3] - x[2]];
                let dy = [x[2] - x[0], x[3] - x[1], x[2] - x[0], x[3] - x[1]];
                let v = match kind {
                    0 => dx,
                    1 => dy,
                    _ => [0, 1, 2, 3].map(|l| dx[l].abs() + dy[l].abs()),
                };
                let v = if fragment { v } else { [0.0; 4] };
                put(mem, *d as usize + k, v, m);
            }
        }
        Op::Pack { d, a, kind } => {
            let n = if *kind == 0 { 2 } else { 4 };
            let x = vecs(mem, *a, n);
            let v = [0, 1, 2, 3].map(|l| {
                let p = x[l];
                fb(match kind {
                    0 => half_from_f32(p[0]) as u32 | ((half_from_f32(p[1]) as u32) << 16),
                    1 => {
                        let q = |f: f32| ((f.clamp(0.0, 1.0) * 65535.0).round() as u32) & 0xffff;
                        q(p[0]) | (q(p[1]) << 16)
                    }
                    2 => {
                        let q = |f: f32| ((f.clamp(-1.0, 1.0) * 32767.0).round() as i32 as u32) & 0xffff;
                        q(p[0]) | (q(p[1]) << 16)
                    }
                    _ => 0,
                })
            });
            put(mem, *d as usize, v, m);
        }
        Op::Unpack { d, a, kind } => {
            let x = mem[*a as usize];
            let v = [0, 1, 2, 3].map(|l| {
                let u = bits(x[l]);
                let (lo, hi) = (u & 0xffff, u >> 16);
                match kind {
                    0 => [f32_from_half(lo as u16), f32_from_half(hi as u16), 0.0, 0.0],
                    1 => [lo as f32 / 65535.0, hi as f32 / 65535.0, 0.0, 0.0],
                    _ => [((lo as u16 as i16) as f32 / 32767.0).max(-1.0), ((hi as u16 as i16) as f32 / 32767.0).max(-1.0), 0.0, 0.0],
                }
            });
            put_vecs(mem, *d, 2, &v, m);
        }
        Op::Modf { d, i, a, n } => {
            for k in 0..*n as usize {
                let x = mem[*a as usize + k];
                put(mem, *i as usize + k, x.map(f32::trunc), m);
                put(mem, *d as usize + k, x.map(|v| v - v.trunc()), m);
            }
        }
        _ => {}
    }
}

fn tex(t: &TexOp, mem: &mut [Lanes], m: u8, env: &dyn TexEnv, fragment: bool) {
    let unit = mem[t.samp as usize].map(bi);
    let nc = t.ncoord as u32;
    match t.kind {
        TexKind::Size => {
            let lod = if t.arg == NONE { [0.0; 4] } else { mem[t.arg as usize] };
            let mut out = [[0.0f32; 4]; LANES];
            for l in 0..LANES {
                let s = env.size(unit[l], t.samp_ty, bi(lod[l]));
                out[l] = [ib(s[0]), ib(s[1]), ib(s[2]), 0.0];
            }
            put_vecs(mem, t.d, nc, &out, m);
            return;
        }
        TexKind::Fetch => {
            let c = vecs(mem, t.coord, nc);
            let lod = if t.arg == NONE { [0.0; 4] } else { mem[t.arg as usize] };
            let mut out = [[0.0f32; 4]; LANES];
            for l in 0..LANES {
                if m & (1 << l) == 0 {
                    continue;
                }
                let mut ic = [bi(c[l][0]), bi(c[l][1]), bi(c[l][2])];
                if let Some(o) = t.offset {
                    for k in 0..3 {
                        ic[k] += o[k];
                    }
                }
                out[l] = env.fetch(unit[l], t.samp_ty, ic, bi(lod[l]));
            }
            put_vecs(mem, t.d, 4, &out, m);
            return;
        }
        _ => {}
    }
    let mut c = vecs(mem, t.coord, nc);
    if t.proj {
        for v in c.iter_mut() {
            let q = v[nc as usize - 1];
            for k in 0..(nc as usize - 1) {
                v[k] /= q;
            }
        }
    }
    let shadow = t.samp_ty.shadow;
    // Which coordinates are for addressing (the rest: compare value, divisor).
    let spatial = match t.samp_ty.dim {
        crate::glsl::Dim::D2 => 2,
        crate::glsl::Dim::D3 | crate::glsl::Dim::Cube | crate::glsl::Dim::D2Array => 3,
    };
    let compare: [Option<f32>; LANES] = if !shadow {
        [None; LANES]
    } else if t.compare != NONE {
        let v = mem[t.compare as usize];
        [Some(v[0]), Some(v[1]), Some(v[2]), Some(v[3])]
    } else {
        // Shadow lookups carry the compare value after the coordinates; with proj it's
        // the one before the divisor.
        let at = if t.proj { nc as usize - 2 } else { spatial.min(nc as usize - 1) };
        [Some(c[0][at]), Some(c[1][at]), Some(c[2][at]), Some(c[3][at])]
    };
    let arg = if t.arg == NONE { None } else { Some(mem[t.arg as usize]) };
    let (gx, gy) = if t.kind == TexKind::Grad { (Some(vecs(mem, t.dx, spatial as u32)), Some(vecs(mem, t.dy, spatial as u32))) } else { (None, None) };
    let mut out = [[0.0f32; 4]; LANES];
    for l in 0..LANES {
        if m & (1 << l) == 0 {
            continue;
        }
        let mut coord = [0.0; 4];
        coord[..spatial.min(4)].copy_from_slice(&c[l][..spatial.min(4)]);
        let (mut dx, mut dy) = ([0.0; 4], [0.0; 4]);
        if let (Some(gx), Some(gy)) = (&gx, &gy) {
            dx = gx[l];
            dy = gy[l];
        } else if fragment && t.kind == TexKind::Sample {
            let (r0, r1) = if l < 2 { (0, 1) } else { (2, 3) };
            let (c0, c1) = if l % 2 == 0 { (0, 2) } else { (1, 3) };
            for k in 0..spatial {
                dx[k] = c[r1][k] - c[r0][k];
                dy[k] = c[c1][k] - c[c0][k];
            }
        }
        let a = TexArgs {
            coord,
            dx,
            dy,
            lod: match t.kind {
                TexKind::Lod => Some(arg.map_or(0.0, |v| v[l])),
                TexKind::Sample if !fragment => Some(arg.map_or(0.0, |v| v[l])),
                _ => None,
            },
            bias: if t.kind == TexKind::Sample && fragment { arg.map_or(0.0, |v| v[l]) } else { 0.0 },
            compare: compare[l],
            offset: t.offset.unwrap_or([0; 3]),
        };
        out[l] = env.sample(unit[l], t.samp_ty, &a);
    }
    put_vecs(mem, t.d, 4, &out, m);
}
