//! Compile a parsed GLSL shader (`glsl`) into code for the 4-lane machine (`glsl_vm`).
//!
//! Every variable and temporary gets fixed slots (GLSL has no recursion, so nothing needs a
//! stack): user functions are inlined at each call, with their own slots. A `Compiled`
//! shader lists where its uniforms, inputs and outputs live so `webgl` can link and feed it.

use crate::glsl::*;
use crate::glsl_vm::{Lanes, Op, Slot, TexKind, TexOp, F1, F2, F3, NONE};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
}

#[derive(Clone, Debug)]
pub struct UniformVar {
    /// The name WebGL reports: `u`, `a[0]`, `s.f`, `s[1].f[0]`.
    pub name: String,
    /// The element type (an array's element; samplers included).
    pub ty: Ty,
    pub slot: Slot,
    /// Array length (1 for a plain uniform).
    pub count: u32,
    pub is_array: bool,
}

#[derive(Clone, Debug)]
pub struct IoVar {
    pub name: String,
    pub ty: Ty,
    pub slot: Slot,
    pub location: Option<i64>,
    pub flat: bool,
}

/// One uniform block (WebGL 2): where each member's slots sit, and its std140 offsets.
#[derive(Clone, Debug)]
pub struct BlockVar {
    pub name: String,
    pub size: u32,
    /// (name, element type, first slot, array count, std140 byte offset, array stride)
    pub members: Vec<(String, Ty, Slot, u32, u32, u32)>,
}

#[derive(Clone, Debug, Default)]
pub struct Builtins {
    pub position: Option<Slot>,
    pub point_size: Option<Slot>,
    pub vertex_id: Option<Slot>,
    pub instance_id: Option<Slot>,
    pub frag_coord: Option<Slot>,
    pub front_facing: Option<Slot>,
    pub point_coord: Option<Slot>,
    pub frag_color: Option<Slot>,
    /// gl_FragData's first slot and length.
    pub frag_data: Option<(Slot, u32)>,
    pub frag_depth: Option<Slot>,
}

pub struct Compiled {
    pub version: u16,
    pub code: Vec<Op>,
    pub mem: Vec<Lanes>,
    pub uniforms: Vec<UniformVar>,
    pub inputs: Vec<IoVar>,
    /// Every declared input, read or not (names and types: attribute aliasing checks).
    pub declared_inputs: Vec<(String, Ty)>,
    pub outputs: Vec<IoVar>,
    pub blocks: Vec<BlockVar>,
    pub b: Builtins,
    pub uses_discard: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Local,
    Global,
    Uniform,
    Input,
    Output,
    Builtin,
    Const,
}

#[derive(Clone)]
struct Sym {
    ty: Ty,
    slot: Slot,
    kind: Kind,
    cval: Option<Rc<Vec<u32>>>,
    used: Rc<Cell<bool>>,
    readonly: bool,
}

#[derive(Clone, Debug)]
struct Val {
    ty: Ty,
    slot: Slot,
}

#[derive(Clone, Debug)]
enum LVal {
    Static { ty: Ty, slots: Vec<Slot> },
    Dyn { ty: Ty, base: Slot, idx: Slot, stride: u32, count: u32, offs: Vec<u32> },
}

impl LVal {
    fn ty(&self) -> &Ty {
        match self {
            LVal::Static { ty, .. } | LVal::Dyn { ty, .. } => ty,
        }
    }
}

struct FnCtx {
    ret: Ty,
    slot: Slot,
}

type R<T> = Result<T, String>;

struct Gen {
    stage: Stage,
    version: u16,
    code: Vec<Op>,
    mem: Vec<Lanes>,
    konsts: HashMap<u32, Slot>,
    scopes: Vec<HashMap<String, Sym>>,
    structs: Vec<HashMap<String, Rc<StructTy>>>,
    funcs: HashMap<String, Vec<Rc<FuncDef>>>,
    fns: Vec<FnCtx>,
    depth: u32,
    uniforms: Vec<(String, Ty, Slot, Rc<Cell<bool>>)>,
    inputs: Vec<(IoVar, Rc<Cell<bool>>)>,
    outputs: Vec<IoVar>,
    blocks: Vec<BlockVar>,
    b: Builtins,
    builtin_used: HashMap<&'static str, Rc<Cell<bool>>>,
    uses_discard: bool,
    line: u32,
}

/// The extensions shaders may enable (their `GL_` macros are defined).
pub const EXTENSIONS: &[&str] = &["GL_OES_standard_derivatives", "GL_EXT_shader_texture_lod", "GL_EXT_frag_depth", "GL_EXT_draw_buffers"];

/// Compile shader source for one stage.
pub fn compile(src: &str, stage: Stage, webgl2: bool) -> Result<Compiled, String> {
    let unit = parse(src, EXTENSIONS).map_err(|e| format!("ERROR: 0:{e}"))?;
    if unit.version == 300 && !webgl2 {
        return Err("ERROR: 0:1: GLSL ES 3.00 needs WebGL 2".into());
    }
    let mut g = Gen {
        stage,
        version: unit.version,
        code: Vec::new(),
        mem: Vec::new(),
        konsts: HashMap::new(),
        scopes: vec![HashMap::new()],
        structs: vec![HashMap::new()],
        funcs: HashMap::new(),
        fns: Vec::new(),
        depth: 0,
        uniforms: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        blocks: Vec::new(),
        b: Builtins::default(),
        builtin_used: HashMap::new(),
        uses_discard: false,
        line: 0,
    };
    g.builtins(&unit);
    g.unit(&unit).map_err(|e| if e.starts_with("ERROR") { e } else { format!("ERROR: 0:{}: {e}", g.line) })?;
    let used = |g: &Gen, n: &str| g.builtin_used.get(n).is_some_and(|c| c.get());
    if stage == Stage::Fragment {
        if !used(&g, "gl_FragColor") {
            g.b.frag_color = None;
        }
        if !used(&g, "gl_FragData") {
            g.b.frag_data = None;
        }
        if g.b.frag_color.is_some() && g.b.frag_data.is_some() {
            return Err("ERROR: 0:0: a shader can't write both gl_FragColor and gl_FragData".into());
        }
        if g.version == 300 && (g.b.frag_color.is_some() || g.b.frag_data.is_some()) {
            return Err("ERROR: 0:0: gl_FragColor and gl_FragData aren't available in GLSL ES 3.00".into());
        }
        if !used(&g, "gl_FragDepth") && !used(&g, "gl_FragDepthEXT") {
            g.b.frag_depth = None;
        }
    }
    let mut uniforms = Vec::new();
    for (name, ty, slot, u) in &g.uniforms {
        if u.get() {
            flatten_uniform(name, ty, *slot, &mut uniforms);
        }
    }
    // Only inputs the shader reads count (an unread varying needn't be written by the
    // vertex shader).
    let inputs = g.inputs.iter().filter(|(_, u)| u.get()).map(|(v, _)| v.clone()).collect();
    let declared_inputs = g.inputs.iter().map(|(v, _)| (v.name.clone(), v.ty.clone())).collect();
    Ok(Compiled {
        declared_inputs,
        version: g.version,
        code: g.code,
        mem: g.mem,
        uniforms,
        inputs,
        outputs: g.outputs,
        blocks: g.blocks,
        b: g.b,
        uses_discard: g.uses_discard,
    })
}

fn flatten_uniform(name: &str, ty: &Ty, slot: Slot, out: &mut Vec<UniformVar>) {
    match ty {
        Ty::Struct(s) => {
            let mut off = 0;
            for (f, fty) in &s.fields {
                flatten_uniform(&format!("{name}.{f}"), fty, slot + off, out);
                off += fty.size();
            }
        }
        Ty::Array(el, n) => match &**el {
            Ty::Struct(_) | Ty::Array(..) => {
                for i in 0..*n {
                    flatten_uniform(&format!("{name}[{i}]"), el, slot + i * el.size(), out);
                }
            }
            _ => out.push(UniformVar { name: format!("{name}[0]"), ty: (**el).clone(), slot, count: *n, is_array: true }),
        },
        _ => out.push(UniformVar { name: name.to_string(), ty: ty.clone(), slot, count: 1, is_array: false }),
    }
}

fn swizzle_index(c: char) -> Option<(u32, u8)> {
    Some(match c {
        'x' => (0, 0),
        'y' => (1, 0),
        'z' => (2, 0),
        'w' => (3, 0),
        'r' => (0, 1),
        'g' => (1, 1),
        'b' => (2, 1),
        'a' => (3, 1),
        's' => (0, 2),
        't' => (1, 2),
        'p' => (2, 2),
        'q' => (3, 2),
        _ => return None,
    })
}

fn parse_swizzle(f: &str, n: u8) -> R<Vec<u32>> {
    if f.is_empty() || f.len() > 4 {
        return Err(format!("bad swizzle '.{f}'"));
    }
    let mut set = None;
    let mut out = Vec::new();
    for c in f.chars() {
        let (i, s) = swizzle_index(c).ok_or(format!("bad swizzle '.{f}'"))?;
        if set.is_some_and(|x| x != s) {
            return Err(format!("mixed swizzle sets in '.{f}'"));
        }
        set = Some(s);
        if i >= n as u32 {
            return Err(format!("swizzle '.{f}' is out of range"));
        }
        out.push(i);
    }
    Ok(out)
}

fn conv_op(from: Base, to: Base) -> Option<F1> {
    use Base::*;
    match (from, to) {
        (a, b) if a == b => None,
        (F, I) => Some(F1::F2I),
        (F, U) => Some(F1::F2U),
        (F, B) => Some(F1::F2B),
        (I, F) => Some(F1::I2F),
        (U, F) => Some(F1::U2F),
        (B, F) => Some(F1::B2F),
        (I, B) | (U, B) => Some(F1::I2B),
        // bool → int/uint and int ↔ uint keep their bits.
        _ => None,
    }
}

impl Gen {
    fn err<T>(&self, m: impl std::fmt::Display) -> R<T> {
        Err(format!("ERROR: 0:{}: {m}", self.line))
    }

    fn alloc(&mut self, n: u32) -> Slot {
        let s = self.mem.len() as Slot;
        self.mem.extend(std::iter::repeat_n([0.0; 4], n.max(1) as usize));
        if self.mem.len() > 1 << 20 {
            // Caught by the caller as an over-large shader.
            self.mem.truncate(1 << 20);
        }
        s
    }
    fn temp(&mut self, ty: &Ty) -> Val {
        let slot = self.alloc(ty.size());
        Val { ty: ty.clone(), slot }
    }
    fn konst(&mut self, bits: u32) -> Slot {
        if let Some(s) = self.konsts.get(&bits) {
            return *s;
        }
        let s = self.alloc(1);
        self.mem[s as usize] = [f32::from_bits(bits); 4];
        self.konsts.insert(bits, s);
        s
    }
    fn kf(&mut self, f: f32) -> Val {
        Val { ty: Ty::FLOAT, slot: self.konst(f.to_bits()) }
    }
    fn ki(&mut self, i: i32) -> Val {
        Val { ty: Ty::INT, slot: self.konst(i as u32) }
    }
    fn emit(&mut self, op: Op) -> usize {
        self.code.push(op);
        self.code.len() - 1
    }

    fn declare(&mut self, name: &str, sym: Sym) -> R<()> {
        let deep = self.scopes.len() > 1;
        let scope = self.scopes.last_mut().unwrap();
        if scope.contains_key(name) && deep {
            return self.err(format!("'{name}' is already declared"));
        }
        scope.insert(name.to_string(), sym);
        Ok(())
    }
    fn lookup(&self, name: &str) -> Option<&Sym> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn builtins(&mut self, unit: &Unit) {
        let v4 = Ty::Vec(Base::F, 4);
        let add = |g: &mut Gen, name: &'static str, ty: Ty, readonly: bool| -> Slot {
            let slot = g.alloc(ty.size());
            let used = Rc::new(Cell::new(false));
            g.builtin_used.insert(name, used.clone());
            g.scopes[0].insert(name.into(), Sym { ty, slot, kind: Kind::Builtin, cval: None, used, readonly });
            slot
        };
        match self.stage {
            Stage::Vertex => {
                let p = add(self, "gl_Position", v4.clone(), false);
                self.mem[p as usize + 3] = [1.0; 4];
                self.b.position = Some(p);
                let ps = add(self, "gl_PointSize", Ty::FLOAT, false);
                self.mem[ps as usize] = [1.0; 4];
                self.b.point_size = Some(ps);
                if unit.version == 300 {
                    self.b.vertex_id = Some(add(self, "gl_VertexID", Ty::INT, true));
                    self.b.instance_id = Some(add(self, "gl_InstanceID", Ty::INT, true));
                }
            }
            Stage::Fragment => {
                self.b.frag_coord = Some(add(self, "gl_FragCoord", v4.clone(), true));
                self.b.front_facing = Some(add(self, "gl_FrontFacing", Ty::BOOL, true));
                self.b.point_coord = Some(add(self, "gl_PointCoord", Ty::Vec(Base::F, 2), true));
                if unit.version == 100 {
                    self.b.frag_color = Some(add(self, "gl_FragColor", v4.clone(), false));
                    let n = 4;
                    self.b.frag_data = Some((add(self, "gl_FragData", Ty::Array(Box::new(v4.clone()), n), false), n));
                    let d = add(self, "gl_FragDepthEXT", Ty::FLOAT, false);
                    self.b.frag_depth = Some(d);
                } else {
                    let d = add(self, "gl_FragDepth", Ty::FLOAT, false);
                    self.b.frag_depth = Some(d);
                }
            }
        }
        // WebGL 2 (ESSL 3.00) has 8 draw buffers; WebGL 1's EXT_draw_buffers has 4.
        let max_draw = if unit.version == 100 { 4 } else { 8 };
        let consts: &[(&str, i32)] = &[
            ("gl_MaxVertexAttribs", 16),
            ("gl_MaxVertexUniformVectors", 256),
            ("gl_MaxVaryingVectors", 15),
            ("gl_MaxVertexTextureImageUnits", 16),
            ("gl_MaxCombinedTextureImageUnits", 32),
            ("gl_MaxTextureImageUnits", 16),
            ("gl_MaxFragmentUniformVectors", 256),
            ("gl_MaxDrawBuffers", max_draw),
            ("gl_MaxVertexOutputVectors", 16),
            ("gl_MaxFragmentInputVectors", 15),
            ("gl_MinProgramTexelOffset", -8),
            ("gl_MaxProgramTexelOffset", 7),
        ];
        for (n, v) in consts {
            let slot = self.konst(*v as u32);
            self.scopes[0].insert(
                n.to_string(),
                Sym { ty: Ty::INT, slot, kind: Kind::Const, cval: Some(Rc::new(vec![*v as u32])), used: Rc::new(Cell::new(false)), readonly: true },
            );
        }
    }

    // ---------------------------------------------------------------- types

    fn find_struct(&self, n: &str) -> Option<Rc<StructTy>> {
        self.structs.iter().rev().find_map(|s| s.get(n).cloned())
    }

    fn base_ty(&mut self, t: &TyAst) -> R<Ty> {
        Ok(match t {
            TyAst::Builtin(t) => t.clone(),
            TyAst::Named(n) => match self.find_struct(n) {
                Some(s) => Ty::Struct(s),
                None => return self.err(format!("unknown type '{n}'")),
            },
            TyAst::Def(d) => {
                let mut fields = Vec::new();
                for (fts, fname, dims) in &d.fields {
                    let fty = self.resolve(fts, dims, None)?;
                    if fields.iter().any(|(n, _): &(String, Ty)| n == fname) {
                        return self.err(format!("duplicate field '{fname}'"));
                    }
                    fields.push((fname.clone(), fty));
                }
                let name = d.name.clone().unwrap_or_else(|| format!("__anon{}", self.mem.len()));
                let st = Rc::new(StructTy { name: name.clone(), fields });
                if let Some(n) = &d.name {
                    self.structs.last_mut().unwrap().insert(n.clone(), st.clone());
                }
                Ty::Struct(st)
            }
        })
    }

    /// A full type: the base, then the type's own dims, then the declarator's dims.
    fn resolve(&mut self, ts: &TypeSpec, dims: &[Option<Expr>], init: Option<&Ty>) -> R<Ty> {
        let mut ty = self.base_ty(&ts.ty)?;
        let all: Vec<&Option<Expr>> = ts.dims.iter().chain(dims.iter()).collect();
        if all.len() > 1 {
            return self.err("arrays of arrays aren't supported");
        }
        for d in all {
            let n = match d {
                Some(e) => {
                    let Some((t, v)) = self.const_eval(e) else { return self.err("array size must be a constant integer expression") };
                    if !matches!(t, Ty::Scalar(Base::I) | Ty::Scalar(Base::U)) {
                        return self.err("array size must be an integer");
                    }
                    let n = v[0] as i32;
                    if n <= 0 {
                        return self.err("array size must be greater than zero");
                    }
                    n as u32
                }
                None => match init {
                    Some(Ty::Array(_, n)) => *n,
                    _ => return self.err("unsized array needs an initializer"),
                },
            };
            ty = Ty::Array(Box::new(ty), n);
        }
        Ok(ty)
    }

    // ---------------------------------------------------------------- constants

    fn const_eval(&mut self, e: &Expr) -> Option<(Ty, Vec<u32>)> {
        use Base::*;
        Some(match e {
            Expr::Int(v) => (Ty::INT, vec![*v as i32 as u32]),
            Expr::UInt(v) => (Ty::Scalar(U), vec![*v as u32]),
            Expr::Float(f) => (Ty::FLOAT, vec![(*f as f32).to_bits()]),
            Expr::Bool(b) => (Ty::BOOL, vec![*b as u32]),
            Expr::Var(n) => {
                let s = self.lookup(n)?;
                (s.ty.clone(), (**s.cval.as_ref()?).clone())
            }
            Expr::Un(op, a) => {
                let (t, v) = self.const_eval(a)?;
                let b = t.base()?;
                let v = v
                    .iter()
                    .map(|&x| match (op, b) {
                        (UnOp::Plus, _) => x,
                        (UnOp::Neg, F) => (-f32::from_bits(x)).to_bits(),
                        (UnOp::Neg, _) => (x as i32).wrapping_neg() as u32,
                        (UnOp::Not, _) => x ^ 1,
                        (UnOp::BitNot, _) => !x,
                    })
                    .collect();
                (t, v)
            }
            Expr::Bin(op, a, b) => {
                let (ta, va) = self.const_eval(a)?;
                let (tb, vb) = self.const_eval(b)?;
                let base = ta.base()?;
                if matches!(ta, Ty::Mat(..)) || matches!(tb, Ty::Mat(..)) {
                    return None;
                }
                let n = va.len().max(vb.len());
                if !(va.len() == n || va.len() == 1) || !(vb.len() == n || vb.len() == 1) {
                    return None;
                }
                let rty = if va.len() >= vb.len() { ta.clone() } else { tb.clone() };
                let cmp = matches!(op, BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge);
                let eq = matches!(op, BinOp::Eq | BinOp::Ne);
                let mut out = Vec::new();
                for k in 0..n {
                    let x = va[if va.len() == 1 { 0 } else { k }];
                    let y = vb[if vb.len() == 1 { 0 } else { k }];
                    let r = match base {
                        F => {
                            let (x, y) = (f32::from_bits(x), f32::from_bits(y));
                            match op {
                                BinOp::Add => (x + y).to_bits(),
                                BinOp::Sub => (x - y).to_bits(),
                                BinOp::Mul => (x * y).to_bits(),
                                BinOp::Div => (x / y).to_bits(),
                                BinOp::Lt => (x < y) as u32,
                                BinOp::Gt => (x > y) as u32,
                                BinOp::Le => (x <= y) as u32,
                                BinOp::Ge => (x >= y) as u32,
                                BinOp::Eq => (x == y) as u32,
                                BinOp::Ne => (x != y) as u32,
                                _ => return None,
                            }
                        }
                        I | U | B => {
                            let s = base == I;
                            let (xi, yi) = (x as i32, y as i32);
                            match op {
                                BinOp::Add => x.wrapping_add(y),
                                BinOp::Sub => x.wrapping_sub(y),
                                BinOp::Mul => x.wrapping_mul(y),
                                BinOp::Div if y == 0 => return None,
                                BinOp::Div if s => xi.wrapping_div(yi) as u32,
                                BinOp::Div => x / y,
                                BinOp::Mod if y == 0 => return None,
                                BinOp::Mod if s => xi.wrapping_rem(yi) as u32,
                                BinOp::Mod => x % y,
                                BinOp::Shl => x.wrapping_shl(y),
                                BinOp::Shr if s => xi.wrapping_shr(y) as u32,
                                BinOp::Shr => x.wrapping_shr(y),
                                BinOp::BitAnd | BinOp::And => x & y,
                                BinOp::BitOr | BinOp::Or => x | y,
                                BinOp::BitXor | BinOp::Xor => x ^ y,
                                BinOp::Lt if s => (xi < yi) as u32,
                                BinOp::Gt if s => (xi > yi) as u32,
                                BinOp::Le if s => (xi <= yi) as u32,
                                BinOp::Ge if s => (xi >= yi) as u32,
                                BinOp::Lt => (x < y) as u32,
                                BinOp::Gt => (x > y) as u32,
                                BinOp::Le => (x <= y) as u32,
                                BinOp::Ge => (x >= y) as u32,
                                BinOp::Eq => (x == y) as u32,
                                BinOp::Ne => (x != y) as u32,
                            }
                        }
                    };
                    out.push(r);
                }
                if eq {
                    let r = if *op == BinOp::Eq { out.iter().all(|&r| r == 1) } else { out.iter().any(|&r| r == 1) };
                    return Some((Ty::BOOL, vec![r as u32]));
                }
                if cmp {
                    return Some((Ty::BOOL, out));
                }
                if matches!(op, BinOp::And | BinOp::Or | BinOp::Xor) {
                    return Some((Ty::BOOL, out));
                }
                (rty, out)
            }
            Expr::Cond(c, a, b) => {
                let (_, cv) = self.const_eval(c)?;
                if cv[0] != 0 { self.const_eval(a)? } else { self.const_eval(b)? }
            }
            Expr::Call(Callee::Type(ts), args) if ts.dims.is_empty() => {
                let TyAst::Builtin(t) = &ts.ty else { return None };
                let tb = t.base()?;
                let mut comps = Vec::new();
                for a in args {
                    let (at, av) = self.const_eval(a)?;
                    if matches!(at, Ty::Mat(..)) && matches!(t, Ty::Mat(..)) {
                        return None;
                    }
                    let ab = at.base()?;
                    for x in av {
                        comps.push(const_convert(x, ab, tb));
                    }
                }
                let n = t.comps() as usize;
                if comps.is_empty() {
                    return None;
                }
                let v = match t {
                    Ty::Mat(c, r) if comps.len() == 1 => {
                        let mut v = vec![0u32; n];
                        for i in 0..(*c.min(r) as usize) {
                            v[i * *r as usize + i] = comps[0];
                        }
                        v
                    }
                    _ if comps.len() == 1 => vec![comps[0]; n],
                    _ => {
                        if comps.len() < n {
                            return None;
                        }
                        comps[..n].to_vec()
                    }
                };
                (t.clone(), v)
            }
            Expr::Field(a, f) => {
                let (t, v) = self.const_eval(a)?;
                match &t {
                    Ty::Vec(b, _) | Ty::Scalar(b) => {
                        let n = if let Ty::Vec(_, n) = t { n } else { 1 };
                        let idx = parse_swizzle(f, n).ok()?;
                        (Ty::vec(*b, idx.len() as u8), idx.iter().map(|&i| v[i as usize]).collect())
                    }
                    _ => return None,
                }
            }
            Expr::Index(a, i) => {
                let (t, v) = self.const_eval(a)?;
                let (_, iv) = self.const_eval(i)?;
                let i = iv[0] as usize;
                match &t {
                    Ty::Vec(b, n) if i < *n as usize => (Ty::Scalar(*b), vec![v[i]]),
                    Ty::Mat(_, r) => {
                        let r = *r as usize;
                        (Ty::Vec(Base::F, r as u8), v.get(i * r..i * r + r)?.to_vec())
                    }
                    Ty::Array(el, n) if (i as u32) < *n => {
                        let sz = el.size() as usize;
                        ((**el).clone(), v.get(i * sz..i * sz + sz)?.to_vec())
                    }
                    _ => return None,
                }
            }
            Expr::Length(a) => {
                let t = self.type_of_simple(a)?;
                let n = match t {
                    Ty::Array(_, n) => n,
                    Ty::Vec(_, n) => n as u32,
                    Ty::Mat(c, _) => c as u32,
                    _ => return None,
                };
                (Ty::INT, vec![n])
            }
            _ => return None,
        })
    }

    /// The type of a variable expression, for `.length()` on arrays.
    fn type_of_simple(&self, e: &Expr) -> Option<Ty> {
        match e {
            Expr::Var(n) => self.lookup(n).map(|s| s.ty.clone()),
            Expr::Field(a, f) => match self.type_of_simple(a)? {
                Ty::Struct(s) => s.fields.iter().find(|x| &x.0 == f).map(|x| x.1.clone()),
                _ => None,
            },
            Expr::Index(a, _) => match self.type_of_simple(a)? {
                Ty::Array(el, _) => Some(*el),
                _ => None,
            },
            _ => None,
        }
    }

    // ---------------------------------------------------------------- top level

    fn unit(&mut self, unit: &Unit) -> R<()> {
        for gl in &unit.globals {
            match gl {
                Global::Func(f) => {
                    let list = self.funcs.entry(f.name.clone()).or_default();
                    if f.body.is_some() {
                        // A definition replaces its own prototype.
                        list.retain(|x| x.body.is_some() || x.params.len() != f.params.len());
                    }
                    if list.iter().any(|x| x.body.is_some() && x.params.len() == f.params.len() && f.body.is_some() && same_sig(x, f)) {
                        self.line = f.line;
                        return self.err(format!("function '{}' is defined twice", f.name));
                    }
                    list.push(f.clone());
                }
                Global::Decl(d) => {
                    self.line = d.line;
                    self.global_decl(d)?;
                }
                Global::Block(b) => {
                    self.line = b.line;
                    self.block(b)?;
                }
            }
        }
        let Some(main) = self.funcs.get("main").and_then(|l| l.iter().find(|f| f.body.is_some()).cloned()) else {
            return self.err("missing main()");
        };
        if !main.params.is_empty() || !matches!(main.ret.ty, TyAst::Builtin(Ty::Void)) {
            return self.err("main() must be 'void main()'");
        }
        self.inline_call(&main, Vec::new())?;
        if self.mem.len() >= 1 << 20 {
            return self.err("shader is too large");
        }
        Ok(())
    }

    fn global_decl(&mut self, d: &Decl) -> R<()> {
        let st = d.quals.storage;
        let mut ty0 = None;
        if d.vars.is_empty() {
            // struct S { … }; on its own defines the type.
            self.base_ty(&d.ty.ty)?;
            return Ok(());
        }
        for v in &d.vars {
            let init_val = match (&v.init, st) {
                (Some(_), Storage::Uniform | Storage::Attribute | Storage::Varying | Storage::In | Storage::Out) => {
                    return self.err(format!("'{}' can't have an initializer", v.name));
                }
                _ => None::<Val>,
            };
            let _ = init_val;
            // Resolve the base type once (struct definitions must only be defined once).
            let base = match &ty0 {
                Some(t) => t,
                None => {
                    let t = self.base_ty(&d.ty.ty)?;
                    ty0 = Some(t);
                    ty0.as_ref().unwrap()
                }
            }
            .clone();
            let ts = TypeSpec { ty: TyAst::Builtin(base), dims: d.ty.dims.clone() };
            let (ty, init) = self.decl_type(&ts, v)?;
            let kind = match (st, self.stage) {
                (Storage::Uniform, _) => Kind::Uniform,
                (Storage::Attribute, Stage::Vertex) | (Storage::In, Stage::Vertex) => Kind::Input,
                (Storage::Varying, Stage::Vertex) | (Storage::Out, _) => Kind::Output,
                (Storage::Varying, Stage::Fragment) | (Storage::In, Stage::Fragment) => Kind::Input,
                (Storage::Attribute, Stage::Fragment) => return self.err("'attribute' is only allowed in vertex shaders"),
                (Storage::Const, _) => Kind::Const,
                _ => Kind::Global,
            };
            if ty.has_sampler() && kind != Kind::Uniform && kind != Kind::Local {
                return self.err("samplers must be uniforms");
            }
            if self.version == 100 && matches!(st, Storage::In | Storage::Out) {
                return self.err("'in'/'out' globals need GLSL ES 3.00");
            }
            if self.version == 300 && matches!(st, Storage::Attribute | Storage::Varying) {
                return self.err("'attribute'/'varying' aren't allowed in GLSL ES 3.00");
            }
            let slot = self.alloc(ty.size());
            let used = Rc::new(Cell::new(false));
            let mut cval = None;
            match kind {
                Kind::Uniform => self.uniforms.push((v.name.clone(), ty.clone(), slot, used.clone())),
                Kind::Input => {
                    if self.stage == Stage::Vertex && matches!(ty, Ty::Struct(_) | Ty::Array(..) | Ty::Scalar(Base::B) | Ty::Vec(Base::B, _)) {
                        return self.err("attributes can't be structs, arrays or bools");
                    }
                    self.inputs.push((IoVar { name: v.name.clone(), ty: ty.clone(), slot, location: d.quals.location, flat: d.quals.flat }, used.clone()));
                }
                Kind::Output => {
                    if self.stage == Stage::Fragment {
                        if !matches!(ty.base(), Some(_)) && !matches!(&ty, Ty::Array(el, _) if el.base().is_some()) {
                            return self.err("fragment outputs must be vectors or scalars");
                        }
                    }
                    self.outputs.push(IoVar { name: v.name.clone(), ty: ty.clone(), slot, location: d.quals.location, flat: d.quals.flat });
                }
                Kind::Const => {
                    let Some(e) = &v.init else { return self.err(format!("const '{}' needs an initializer", v.name)) };
                    if let Some((ct, cv)) = self.const_eval(e) {
                        let cv = convert_const(&ct, &ty, cv);
                        for (k, b) in cv.iter().enumerate() {
                            self.mem[slot as usize + k] = [f32::from_bits(*b); 4];
                        }
                        cval = Some(Rc::new(cv));
                    } else {
                        self.init_var(slot, &ty, e)?;
                    }
                }
                _ => {
                    if let Some(e) = init {
                        self.init_var(slot, &ty, e)?;
                    }
                }
            }
            self.declare(&v.name, Sym { ty, slot, kind, cval, used, readonly: matches!(kind, Kind::Uniform | Kind::Const) || (kind == Kind::Input) })?;
        }
        Ok(())
    }

    /// The type of a declarator, sizing unsized arrays from the initializer.
    fn decl_type<'e>(&mut self, ts: &TypeSpec, v: &'e Declarator) -> R<(Ty, Option<&'e Expr>)> {
        let open_size = ts.dims.iter().chain(v.dims.iter()).any(|d| d.is_none());
        let init_ty = if open_size {
            match &v.init {
                Some(Expr::Call(Callee::Type(cts), args)) if !cts.dims.is_empty() => Some(Ty::Array(Box::new(Ty::Void), args.len() as u32)),
                Some(e) => self.type_of_simple(e),
                None => None,
            }
        } else {
            None
        };
        let ty = self.resolve(ts, &v.dims, init_ty.as_ref())?;
        Ok((ty, v.init.as_ref()))
    }

    fn init_var(&mut self, slot: Slot, ty: &Ty, e: &Expr) -> R<()> {
        let v = self.expr(e)?;
        let v = self.coerce(v, ty)?;
        self.emit(Op::Mov { d: slot, s: v.slot, n: ty.size() });
        Ok(())
    }

    fn block(&mut self, b: &Block) -> R<()> {
        if b.storage != Storage::Uniform {
            return self.err("only uniform blocks are supported");
        }
        if self.version != 300 {
            return self.err("uniform blocks need GLSL ES 3.00");
        }
        let mut members = Vec::new();
        let mut fields = Vec::new();
        let mut off = 0u32;
        for (ts, name, dims) in &b.fields {
            let ty = self.resolve(ts, dims, None)?;
            let slot = self.alloc(ty.size());
            let (el, count) = match &ty {
                Ty::Array(el, n) => ((**el).clone(), *n),
                t => (t.clone(), 1),
            };
            let (align, size) = std140(&el);
            let is_array = matches!(ty, Ty::Array(..));
            let stride = if is_array { round_up(size, 16) } else { size };
            let a = if is_array { round_up(align, 16) } else { align };
            off = round_up(off, a);
            let full = match &b.instance {
                Some(_) => format!("{}.{}", b.name, name),
                None => name.clone(),
            };
            members.push((full, el, slot, count, off, stride));
            off += stride * count;
            if is_array || matches!(ty, Ty::Struct(_)) {
                off = round_up(off, 16);
            }
            fields.push((name.clone(), ty, slot));
        }
        let size = round_up(off, 16);
        match &b.instance {
            Some((inst, _)) => {
                // Members live in consecutive slots, so the instance is a struct over them.
                let st = Rc::new(StructTy { name: b.name.clone(), fields: fields.iter().map(|(n, t, _)| (n.clone(), t.clone())).collect() });
                let slot = fields.first().map_or(0, |f| f.2);
                self.declare(inst, Sym { ty: Ty::Struct(st), slot, kind: Kind::Uniform, cval: None, used: Rc::new(Cell::new(true)), readonly: true })?;
            }
            None => {
                for (n, t, s) in fields {
                    self.declare(&n, Sym { ty: t, slot: s, kind: Kind::Uniform, cval: None, used: Rc::new(Cell::new(true)), readonly: true })?;
                }
            }
        }
        self.blocks.push(BlockVar { name: b.name.clone(), size, members });
        Ok(())
    }

    // ---------------------------------------------------------------- functions

    fn inline_call(&mut self, f: &Rc<FuncDef>, args: Vec<(Val, Option<LVal>)>) -> R<Val> {
        if self.depth > 24 {
            return self.err(format!("'{}': recursion or calls nested too deep", f.name));
        }
        let Some(body) = &f.body else { return self.err(format!("function '{}' has no body", f.name)) };
        let ret = self.resolve(&f.ret, &[], None)?;
        let ret_slot = self.alloc(ret.size());
        self.scopes.push(HashMap::new());
        self.structs.push(HashMap::new());
        let mut outs = Vec::new();
        for (p, (a, lv)) in f.params.iter().zip(args) {
            let pty = self.resolve(&p.ty, &p.dims, None)?;
            let slot = self.alloc(pty.size());
            if p.storage != Storage::Out {
                let a = self.coerce(a, &pty)?;
                self.emit(Op::Mov { d: slot, s: a.slot, n: pty.size() });
            }
            if matches!(p.storage, Storage::Out | Storage::InOut) {
                if let Some(lv) = lv {
                    outs.push((Val { ty: pty.clone(), slot }, lv));
                }
            }
            if let Some(n) = &p.name {
                self.declare(n, Sym { ty: pty, slot, kind: Kind::Local, cval: None, used: Rc::new(Cell::new(true)), readonly: p.storage == Storage::Const })?;
            }
        }
        self.emit(Op::FnBegin);
        self.fns.push(FnCtx { ret: ret.clone(), slot: ret_slot });
        self.depth += 1;
        let line = self.line;
        for s in body {
            self.stmt(s)?;
        }
        self.line = line;
        self.depth -= 1;
        self.fns.pop();
        self.emit(Op::FnEnd);
        self.scopes.pop();
        self.structs.pop();
        for (v, lv) in outs {
            let v = self.coerce(v, &lv.ty().clone())?;
            self.store(&lv, &v);
        }
        Ok(Val { ty: ret, slot: ret_slot })
    }

    fn call_user(&mut self, name: &str, args: &[Expr]) -> R<Val> {
        let cands = self.funcs.get(name).cloned().unwrap_or_default();
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.expr(a)?);
        }
        // Exact match first, then any with the right count (converting).
        let mut pick = None;
        for f in cands.iter().filter(|f| f.params.len() == args.len()) {
            let mut ok = true;
            for (p, v) in f.params.iter().zip(&vals) {
                let pty = self.resolve(&p.ty, &p.dims, None)?;
                if pty != v.ty {
                    ok = false;
                    break;
                }
            }
            if ok {
                pick = Some(f.clone());
                break;
            }
        }
        if pick.is_none() {
            pick = cands.iter().find(|f| f.params.len() == args.len()).cloned();
        }
        let Some(mut f) = pick else { return self.err(format!("no function '{name}' takes these arguments")) };
        if f.body.is_none() {
            // The prototype's definition, declared later.
            if let Some(d) = cands.iter().find(|x| x.body.is_some() && x.params.len() == f.params.len() && same_sig(x, &f)) {
                f = d.clone();
            } else if let Some(d) = cands.iter().find(|x| x.body.is_some() && x.params.len() == f.params.len()) {
                f = d.clone();
            }
        }
        let mut full = Vec::new();
        for ((p, v), a) in f.params.iter().zip(vals).zip(args) {
            let lv = if matches!(p.storage, Storage::Out | Storage::InOut) { Some(self.lvalue(a)?) } else { None };
            full.push((v, lv));
        }
        self.inline_call(&f, full)
    }

    // ---------------------------------------------------------------- statements

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match s {
            Stmt::Empty => {}
            Stmt::Decl(d) => {
                self.line = d.line;
                self.local_decl(d)?;
            }
            Stmt::Expr(e, line) => {
                self.line = *line;
                self.expr(e)?;
            }
            Stmt::Block(b) => {
                self.scopes.push(HashMap::new());
                self.structs.push(HashMap::new());
                for s in b {
                    self.stmt(s)?;
                }
                self.scopes.pop();
                self.structs.pop();
            }
            Stmt::If(c, a, b, line) => {
                self.line = *line;
                let c = self.expr(c)?;
                let c = self.need_bool(c)?;
                let at = self.emit(Op::If { c: c.slot, else_pc: 0 });
                self.stmt(a)?;
                if let Some(b) = b {
                    let el = self.emit(Op::Else { end_pc: 0 });
                    self.code[at] = Op::If { c: c.slot, else_pc: el as u32 };
                    self.stmt(b)?;
                    let end = self.emit(Op::EndIf);
                    self.code[el] = Op::Else { end_pc: end as u32 };
                } else {
                    let end = self.emit(Op::EndIf);
                    self.code[at] = Op::If { c: c.slot, else_pc: end as u32 };
                }
            }
            Stmt::For(init, cond, step, body, line) => {
                self.line = *line;
                self.scopes.push(HashMap::new());
                if let Some(i) = init {
                    self.stmt(i)?;
                }
                self.emit(Op::LoopBegin);
                let top = self.code.len();
                let lc = match cond {
                    Some(c) => Some(self.loop_cond(c)?),
                    None => None,
                };
                self.stmt(body)?;
                self.emit(Op::LoopCont);
                if let Some(s) = step {
                    self.expr(s)?;
                }
                self.emit(Op::LoopBack { pc: top as u32 });
                let end = self.emit(Op::LoopEnd);
                if let Some((at, c)) = lc {
                    self.code[at] = Op::LoopCond { c, end_pc: end as u32 };
                }
                self.scopes.pop();
            }
            Stmt::While(cond, body, line) => {
                self.line = *line;
                self.scopes.push(HashMap::new());
                self.emit(Op::LoopBegin);
                let top = self.code.len();
                let (at, c) = self.loop_cond(cond)?;
                self.stmt(body)?;
                self.emit(Op::LoopCont);
                self.emit(Op::LoopBack { pc: top as u32 });
                let end = self.emit(Op::LoopEnd);
                self.code[at] = Op::LoopCond { c, end_pc: end as u32 };
                self.scopes.pop();
            }
            Stmt::Do(body, cond, line) => {
                self.line = *line;
                self.emit(Op::LoopBegin);
                let top = self.code.len();
                self.stmt(body)?;
                self.emit(Op::LoopCont);
                let c = self.expr(cond)?;
                let c = self.need_bool(c)?;
                let at = self.emit(Op::LoopCond { c: c.slot, end_pc: 0 });
                self.emit(Op::LoopBack { pc: top as u32 });
                let end = self.emit(Op::LoopEnd);
                self.code[at] = Op::LoopCond { c: c.slot, end_pc: end as u32 };
            }
            Stmt::Switch(sel, body, line) => {
                self.line = *line;
                let s = self.expr(sel)?;
                if !matches!(s.ty, Ty::Scalar(Base::I) | Ty::Scalar(Base::U)) {
                    return self.err("switch needs an integer");
                }
                // Each label's condition, worked out before any lanes enter.
                let mut conds = Vec::new();
                let mut any = None;
                for st in body {
                    if let Stmt::Case(Some(e), _) = st {
                        let Some((_, v)) = self.const_eval(e) else { return self.err("case labels must be constant") };
                        let k = self.konst(v[0]);
                        let c = self.alloc(1);
                        self.emit(Op::Bin { f: F2::IEq, d: c, a: s.slot, b: k, n: 1, sa: 1, sb: 1 });
                        any = Some(match any {
                            None => c,
                            Some(p) => {
                                let o = self.alloc(1);
                                self.emit(Op::Bin { f: F2::Or, d: o, a: p, b: c, n: 1, sa: 1, sb: 1 });
                                o
                            }
                        });
                        conds.push(c);
                    }
                }
                let default = self.alloc(1);
                match any {
                    Some(a) => {
                        self.emit(Op::Un { f: F1::Not, d: default, a, n: 1 });
                    }
                    None => self.mem[default as usize] = [f32::from_bits(1); 4],
                }
                self.emit(Op::SwBegin);
                self.scopes.push(HashMap::new());
                let mut ci = 0;
                for st in body {
                    match st {
                        Stmt::Case(Some(_), _) => {
                            self.emit(Op::SwCase { c: conds[ci] });
                            ci += 1;
                        }
                        Stmt::Case(None, _) => {
                            self.emit(Op::SwCase { c: default });
                        }
                        s => self.stmt(s)?,
                    }
                }
                self.scopes.pop();
                self.emit(Op::SwEnd);
            }
            Stmt::Case(_, line) => {
                self.line = *line;
                return self.err("case label outside a switch");
            }
            Stmt::Break(line) => {
                self.line = *line;
                self.emit(Op::Break);
            }
            Stmt::Continue(line) => {
                self.line = *line;
                self.emit(Op::Continue);
            }
            Stmt::Discard(line) => {
                self.line = *line;
                if self.stage != Stage::Fragment {
                    return self.err("discard is only allowed in fragment shaders");
                }
                self.uses_discard = true;
                self.emit(Op::Discard);
            }
            Stmt::Return(e, line) => {
                self.line = *line;
                let Some(fc) = self.fns.last() else { return self.err("return outside a function") };
                let (rty, rs) = (fc.ret.clone(), fc.slot);
                match e {
                    Some(e) => {
                        if rty == Ty::Void {
                            return self.err("void function can't return a value");
                        }
                        let v = self.expr(e)?;
                        let v = self.coerce(v, &rty)?;
                        self.emit(Op::Mov { d: rs, s: v.slot, n: rty.size() });
                    }
                    None if rty != Ty::Void => return self.err("function must return a value"),
                    None => {}
                }
                self.emit(Op::Return);
            }
        }
        Ok(())
    }

    fn loop_cond(&mut self, c: &Stmt) -> R<(usize, Slot)> {
        let v = match c {
            Stmt::Expr(e, _) => self.expr(e)?,
            Stmt::Decl(d) => {
                self.local_decl(d)?;
                let s = self.lookup(&d.vars[0].name).unwrap().clone();
                Val { ty: s.ty, slot: s.slot }
            }
            _ => return self.err("bad loop condition"),
        };
        let v = self.need_bool(v)?;
        let at = self.emit(Op::LoopCond { c: v.slot, end_pc: 0 });
        Ok((at, v.slot))
    }

    fn local_decl(&mut self, d: &Decl) -> R<()> {
        if !matches!(d.quals.storage, Storage::None | Storage::Const) {
            return self.err("storage qualifiers aren't allowed on local variables");
        }
        if d.vars.is_empty() {
            self.base_ty(&d.ty.ty)?;
            return Ok(());
        }
        let base = self.base_ty(&d.ty.ty)?;
        for v in &d.vars {
            let ts = TypeSpec { ty: TyAst::Builtin(base.clone()), dims: d.ty.dims.clone() };
            let (ty, init) = self.decl_type(&ts, v)?;
            if ty.has_sampler() {
                return self.err("samplers must be uniforms");
            }
            let slot = self.alloc(ty.size());
            let mut cval = None;
            let is_const = d.quals.storage == Storage::Const;
            if let Some(e) = init {
                if is_const {
                    if let Some((ct, cv)) = self.const_eval(e) {
                        cval = Some(Rc::new(convert_const(&ct, &ty, cv)));
                    }
                }
                self.init_var(slot, &ty, e)?;
            } else if is_const {
                return self.err(format!("const '{}' needs an initializer", v.name));
            }
            self.declare(&v.name, Sym { ty, slot, kind: Kind::Local, cval, used: Rc::new(Cell::new(true)), readonly: is_const })?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- values

    fn need_bool(&mut self, v: Val) -> R<Val> {
        if v.ty != Ty::BOOL {
            return self.err(format!("expected a bool, found {}", v.ty.name()));
        }
        Ok(v)
    }

    /// Convert numeric bases (lenient: ints become floats where needed).
    fn convert(&mut self, v: Val, to: Base) -> Val {
        let Some(from) = v.ty.base() else { return v };
        if from == to {
            return v;
        }
        let ty = match &v.ty {
            Ty::Scalar(_) => Ty::Scalar(to),
            Ty::Vec(_, n) => Ty::Vec(to, *n),
            _ => return v,
        };
        let t = self.temp(&ty);
        match conv_op(from, to) {
            Some(f) => self.emit(Op::Un { f, d: t.slot, a: v.slot, n: ty.size() }),
            None => self.emit(Op::Mov { d: t.slot, s: v.slot, n: ty.size() }),
        };
        t
    }

    /// Make `v` fit type `ty` (for assignment, initialization, arguments, returns).
    fn coerce(&mut self, v: Val, ty: &Ty) -> R<Val> {
        if &v.ty == ty {
            return Ok(v);
        }
        match (&v.ty, ty) {
            (Ty::Scalar(_), Ty::Scalar(b)) => Ok(self.convert(v, *b)),
            (Ty::Vec(_, n), Ty::Vec(b, m)) if n == m => Ok(self.convert(v, *b)),
            (Ty::Array(a, n), Ty::Array(b, m)) if n == m && (**a == Ty::Void || a.size() == b.size()) => Ok(Val { ty: ty.clone(), slot: v.slot }),
            _ => self.err(format!("can't convert {} to {}", v.ty.name(), ty.name())),
        }
    }

    fn store(&mut self, lv: &LVal, v: &Val) {
        match lv {
            LVal::Static { slots, .. } => {
                let contiguous = slots.windows(2).all(|w| w[1] == w[0] + 1);
                if contiguous && !slots.is_empty() {
                    self.emit(Op::Mov { d: slots[0], s: v.slot, n: slots.len() as u32 });
                } else {
                    self.emit(Op::Scatter { d: slots.clone().into_boxed_slice(), s: v.slot });
                }
            }
            LVal::Dyn { base, idx, stride, count, offs, .. } => {
                self.emit(Op::StoreDyn { base: *base, idx: *idx, stride: *stride, count: *count, offs: offs.clone().into_boxed_slice(), s: v.slot });
            }
        }
    }

    fn load(&mut self, lv: &LVal) -> Val {
        match lv {
            LVal::Static { ty, slots } => {
                if slots.windows(2).all(|w| w[1] == w[0] + 1) && !slots.is_empty() {
                    return Val { ty: ty.clone(), slot: slots[0] };
                }
                let t = self.temp(ty);
                self.emit(Op::Gather { d: t.slot, s: slots.clone().into_boxed_slice() });
                t
            }
            LVal::Dyn { ty, base, idx, stride, count, offs } => {
                let t = self.temp(ty);
                let contiguous = offs.windows(2).all(|w| w[1] == w[0] + 1);
                if contiguous && !offs.is_empty() {
                    self.emit(Op::LoadDyn { d: t.slot, base: *base + offs[0], idx: *idx, stride: *stride, count: *count, n: offs.len() as u32 });
                } else {
                    for (k, o) in offs.iter().enumerate() {
                        self.emit(Op::LoadDyn { d: t.slot + k as u32, base: *base + o, idx: *idx, stride: *stride, count: *count, n: 1 });
                    }
                }
                t
            }
        }
    }

    fn lvalue(&mut self, e: &Expr) -> R<LVal> {
        match e {
            Expr::Var(n) => {
                let Some(s) = self.lookup(n).cloned() else { return self.err(format!("'{n}' isn't declared")) };
                s.used.set(true);
                if s.readonly || (s.kind == Kind::Builtin && s.readonly) {
                    return self.err(format!("'{n}' is read-only"));
                }
                if s.kind == Kind::Input {
                    return self.err(format!("'{n}' is an input and can't be written"));
                }
                Ok(LVal::Static { ty: s.ty.clone(), slots: (s.slot..s.slot + s.ty.size()).collect() })
            }
            Expr::Field(a, f) => {
                let base = self.lvalue(a)?;
                self.select_lv(base, f)
            }
            Expr::Index(a, i) => {
                let base = self.lvalue(a)?;
                self.index_lv(base, i)
            }
            _ => self.err("expression can't be assigned to"),
        }
    }

    fn select_lv(&mut self, base: LVal, f: &str) -> R<LVal> {
        let ty = base.ty().clone();
        let (fty, offs): (Ty, Vec<u32>) = match &ty {
            Ty::Struct(s) => {
                let mut off = 0;
                let mut found = None;
                for (n, t) in &s.fields {
                    if n == f {
                        found = Some((t.clone(), (off..off + t.size()).collect()));
                        break;
                    }
                    off += t.size();
                }
                match found {
                    Some(x) => x,
                    None => return self.err(format!("no field '{f}'")),
                }
            }
            Ty::Vec(b, _) | Ty::Scalar(b) => {
                let n = if let Ty::Vec(_, n) = ty { n } else { 1 };
                let idx = parse_swizzle(f, n).map_err(|e| format!("ERROR: 0:{}: {e}", self.line))?;
                let mut seen = 0u8;
                for i in &idx {
                    if seen & (1 << i) != 0 {
                        return self.err(format!("swizzle '.{f}' repeats a component and can't be assigned to"));
                    }
                    seen |= 1 << i;
                }
                (Ty::vec(*b, idx.len() as u8), idx)
            }
            _ => return self.err(format!("'.{f}' on a {}", ty.name())),
        };
        Ok(match base {
            LVal::Static { slots, .. } => LVal::Static { ty: fty, slots: offs.iter().map(|&o| slots[o as usize]).collect() },
            LVal::Dyn { base, idx, stride, count, offs: bo, .. } => LVal::Dyn { ty: fty, base, idx, stride, count, offs: offs.iter().map(|&o| bo[o as usize]).collect() },
        })
    }

    fn element(&self, ty: &Ty) -> R<(Ty, u32, u32)> {
        Ok(match ty {
            Ty::Array(el, n) => ((**el).clone(), el.size(), *n),
            Ty::Vec(b, n) => (Ty::Scalar(*b), 1, *n as u32),
            Ty::Mat(c, r) => (Ty::Vec(Base::F, *r), *r as u32, *c as u32),
            _ => return self.err(format!("can't index a {}", ty.name())),
        })
    }

    fn index_lv(&mut self, base: LVal, i: &Expr) -> R<LVal> {
        let (el, sz, count) = self.element(base.ty())?;
        if let Some((_, v)) = self.const_eval(i) {
            let k = v[0] as i32;
            if k < 0 || k as u32 >= count {
                return self.err(format!("index {k} is out of range"));
            }
            let k = k as u32;
            return Ok(match base {
                LVal::Static { slots, .. } => LVal::Static { ty: el, slots: slots[(k * sz) as usize..((k + 1) * sz) as usize].to_vec() },
                LVal::Dyn { base, idx, stride, count: c, offs, .. } => {
                    LVal::Dyn { ty: el, base, idx, stride, count: c, offs: offs[(k * sz) as usize..((k + 1) * sz) as usize].to_vec() }
                }
            });
        }
        let iv = self.expr(i)?;
        let iv = self.convert(iv, Base::I);
        match base {
            LVal::Static { slots, .. } => {
                // A contiguous base can be indexed per lane directly.
                let contiguous = slots.windows(2).all(|w| w[1] == w[0] + 1);
                if contiguous || sz == 1 && slots.windows(2).all(|w| w[1] == w[0] + 1) {
                    return Ok(LVal::Dyn { ty: el, base: slots[0], idx: iv.slot, stride: sz, count, offs: (0..sz).collect() });
                }
                self.err("dynamic index into a swizzle")
            }
            LVal::Dyn { base, idx, stride, count: c, offs, .. } => {
                // Fold both indices into one: idx*stride + offs[0] + i*sz.
                let first = offs[0];
                let t1 = self.alloc(1);
                let st = self.ki(stride as i32);
                self.emit(Op::Bin { f: F2::IMul, d: t1, a: idx, b: st.slot, n: 1, sa: 1, sb: 1 });
                let szk = self.ki(sz as i32);
                let t2 = self.alloc(1);
                self.emit(Op::Bin { f: F2::IMul, d: t2, a: iv.slot, b: szk.slot, n: 1, sa: 1, sb: 1 });
                let t3 = self.alloc(1);
                self.emit(Op::Bin { f: F2::IAdd, d: t3, a: t1, b: t2, n: 1, sa: 1, sb: 1 });
                Ok(LVal::Dyn { ty: el, base: base + first, idx: t3, stride: 1, count: c * stride, offs: (0..sz).collect() })
            }
        }
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self, e: &Expr) -> R<Val> {
        match e {
            Expr::Int(v) => Ok(self.ki(*v as i32)),
            Expr::UInt(v) => Ok(Val { ty: Ty::Scalar(Base::U), slot: self.konst(*v as u32) }),
            Expr::Float(f) => Ok(self.kf(*f as f32)),
            Expr::Bool(b) => Ok(Val { ty: Ty::BOOL, slot: self.konst(*b as u32) }),
            Expr::Var(n) => {
                let Some(s) = self.lookup(n).cloned() else { return self.err(format!("'{n}' isn't declared")) };
                s.used.set(true);
                if self.stage == Stage::Vertex && n == "gl_FragColor" {
                    return self.err("gl_FragColor in a vertex shader");
                }
                Ok(Val { ty: s.ty, slot: s.slot })
            }
            Expr::Comma(a, b) => {
                self.expr(a)?;
                self.expr(b)
            }
            Expr::Assign(op, l, r) => {
                let lv = self.lvalue(l)?;
                let rv = self.expr(r)?;
                let v = match op {
                    None => self.coerce(rv, &lv.ty().clone())?,
                    Some(op) => {
                        let cur = self.load(&lv);
                        let res = self.binary(*op, cur, rv)?;
                        self.coerce(res, &lv.ty().clone())?
                    }
                };
                // Copy first so later writes to the target don't change this value.
                let out = self.temp(&v.ty);
                self.emit(Op::Mov { d: out.slot, s: v.slot, n: v.ty.size() });
                self.store(&lv, &out);
                Ok(out)
            }
            Expr::Step(pre, delta, x) => {
                let lv = self.lvalue(x)?;
                let cur = self.load(&lv);
                let old = self.temp(&cur.ty);
                self.emit(Op::Mov { d: old.slot, s: cur.slot, n: cur.ty.size() });
                let one = match cur.ty.base() {
                    Some(Base::F) => self.kf(1.0),
                    Some(Base::I) => self.ki(1),
                    Some(Base::U) => Val { ty: Ty::Scalar(Base::U), slot: self.konst(1) },
                    _ => return self.err("++/-- needs a number"),
                };
                let op = if *delta > 0 { BinOp::Add } else { BinOp::Sub };
                let nv = self.binary(op, old.clone(), one)?;
                let nv = self.coerce(nv, &cur.ty)?;
                self.store(&lv, &nv);
                Ok(if *pre { nv } else { old })
            }
            Expr::Un(op, a) => {
                let v = self.expr(a)?;
                match op {
                    UnOp::Plus => Ok(v),
                    UnOp::Neg => {
                        let f = match v.ty.base() {
                            Some(Base::F) => F1::FNeg,
                            Some(Base::I) | Some(Base::U) => F1::INeg,
                            _ => return self.err("'-' needs a number"),
                        };
                        let t = self.temp(&v.ty);
                        self.emit(Op::Un { f, d: t.slot, a: v.slot, n: v.ty.size() });
                        Ok(t)
                    }
                    UnOp::Not => {
                        let v = self.need_bool(v)?;
                        let t = self.temp(&v.ty);
                        self.emit(Op::Un { f: F1::Not, d: t.slot, a: v.slot, n: 1 });
                        Ok(t)
                    }
                    UnOp::BitNot => {
                        if !matches!(v.ty.base(), Some(Base::I) | Some(Base::U)) {
                            return self.err("'~' needs an integer");
                        }
                        let t = self.temp(&v.ty);
                        self.emit(Op::Un { f: F1::BitNot, d: t.slot, a: v.slot, n: v.ty.size() });
                        Ok(t)
                    }
                }
            }
            Expr::Bin(op @ (BinOp::And | BinOp::Or), a, b) => {
                let x = self.expr(a)?;
                let x = self.need_bool(x)?;
                let r = self.temp(&Ty::BOOL);
                self.emit(Op::Mov { d: r.slot, s: x.slot, n: 1 });
                let c = if *op == BinOp::And {
                    r.slot
                } else {
                    let n = self.alloc(1);
                    self.emit(Op::Un { f: F1::Not, d: n, a: r.slot, n: 1 });
                    n
                };
                let at = self.emit(Op::If { c, else_pc: 0 });
                let y = self.expr(b)?;
                let y = self.need_bool(y)?;
                self.emit(Op::Mov { d: r.slot, s: y.slot, n: 1 });
                let end = self.emit(Op::EndIf);
                self.code[at] = Op::If { c, else_pc: end as u32 };
                Ok(r)
            }
            Expr::Bin(op, a, b) => {
                let x = self.expr(a)?;
                let y = self.expr(b)?;
                self.binary(*op, x, y)
            }
            Expr::Cond(c, a, b) => {
                let c = self.expr(c)?;
                let c = self.need_bool(c)?;
                let at = self.emit(Op::If { c: c.slot, else_pc: 0 });
                let x = self.expr(a)?;
                let r = self.temp(&x.ty);
                self.emit(Op::Mov { d: r.slot, s: x.slot, n: x.ty.size() });
                let el = self.emit(Op::Else { end_pc: 0 });
                self.code[at] = Op::If { c: c.slot, else_pc: el as u32 };
                let y = self.expr(b)?;
                let y = self.coerce(y, &r.ty)?;
                self.emit(Op::Mov { d: r.slot, s: y.slot, n: r.ty.size() });
                let end = self.emit(Op::EndIf);
                self.code[el] = Op::Else { end_pc: end as u32 };
                Ok(r)
            }
            Expr::Field(a, f) => {
                let v = self.expr(a)?;
                self.select(v, f)
            }
            Expr::Index(a, i) => {
                let v = self.expr(a)?;
                let (el, sz, count) = self.element(&v.ty)?;
                if let Some((_, k)) = self.const_eval(i) {
                    let k = k[0] as i32;
                    if k < 0 || k as u32 >= count {
                        return self.err(format!("index {k} is out of range"));
                    }
                    return Ok(Val { ty: el, slot: v.slot + k as u32 * sz });
                }
                let iv = self.expr(i)?;
                if !matches!(iv.ty, Ty::Scalar(Base::I) | Ty::Scalar(Base::U)) {
                    return self.err("index must be an integer");
                }
                let t = self.temp(&el);
                self.emit(Op::LoadDyn { d: t.slot, base: v.slot, idx: iv.slot, stride: sz, count, n: sz });
                Ok(t)
            }
            Expr::Length(a) => {
                let ty = match self.type_of_simple(a) {
                    Some(t) => t,
                    None => self.expr(a)?.ty,
                };
                let n = match ty {
                    Ty::Array(_, n) => n,
                    Ty::Vec(_, n) => n as u32,
                    Ty::Mat(c, _) => c as u32,
                    _ => return self.err(".length() needs an array"),
                };
                Ok(self.ki(n as i32))
            }
            Expr::Call(Callee::Type(ts), args) => self.construct(ts, args),
            Expr::Call(Callee::Name(n), args) => {
                if self.funcs.contains_key(n) {
                    return self.call_user(n, args);
                }
                if let Some(s) = self.find_struct(n) {
                    let ts = TypeSpec { ty: TyAst::Builtin(Ty::Struct(s)), dims: Vec::new() };
                    return self.construct(&ts, args);
                }
                self.builtin(n, args)
            }
        }
    }

    fn select(&mut self, v: Val, f: &str) -> R<Val> {
        match &v.ty {
            Ty::Struct(s) => {
                let mut off = 0;
                for (n, t) in &s.fields {
                    if n == f {
                        return Ok(Val { ty: t.clone(), slot: v.slot + off });
                    }
                    off += t.size();
                }
                self.err(format!("no field '{f}' in {}", s.name))
            }
            Ty::Vec(b, _) | Ty::Scalar(b) => {
                let n = if let Ty::Vec(_, n) = v.ty { n } else { 1 };
                let idx = parse_swizzle(f, n).map_err(|e| format!("ERROR: 0:{}: {e}", self.line))?;
                let ty = Ty::vec(*b, idx.len() as u8);
                if idx.windows(2).all(|w| w[1] == w[0] + 1) {
                    return Ok(Val { ty, slot: v.slot + idx[0] });
                }
                let t = self.temp(&ty);
                let src: Vec<Slot> = idx.iter().map(|&i| v.slot + i).collect();
                self.emit(Op::Gather { d: t.slot, s: src.into_boxed_slice() });
                Ok(t)
            }
            t => self.err(format!("'.{f}' on a {}", t.name())),
        }
    }

    fn binary(&mut self, op: BinOp, x: Val, y: Val) -> R<Val> {
        use BinOp::*;
        // Equality works on anything of one type.
        if matches!(op, Eq | Ne) {
            let (x, y) = self.unify(x, y)?;
            if x.ty != y.ty {
                return self.err(format!("can't compare {} and {}", x.ty.name(), y.ty.name()));
            }
            let n = x.ty.size();
            let float_mask = float_slots(&x.ty);
            let t = self.alloc(n);
            for (k, is_f) in float_mask.iter().enumerate() {
                let f = if *is_f { F2::FEq } else { F2::IEq };
                self.emit(Op::Bin { f, d: t + k as u32, a: x.slot + k as u32, b: y.slot + k as u32, n: 1, sa: 1, sb: 1 });
            }
            let r = self.temp(&Ty::BOOL);
            self.emit(Op::All { d: r.slot, a: t, n });
            if op == Ne {
                self.emit(Op::Un { f: F1::Not, d: r.slot, a: r.slot, n: 1 });
            }
            return Ok(r);
        }
        if op == Xor {
            let x = self.need_bool(x)?;
            let y = self.need_bool(y)?;
            let r = self.temp(&Ty::BOOL);
            self.emit(Op::Bin { f: F2::Xor, d: r.slot, a: x.slot, b: y.slot, n: 1, sa: 1, sb: 1 });
            return Ok(r);
        }
        let (x, y) = self.unify(x, y)?;
        let base = x.ty.base().ok_or_else(|| format!("ERROR: 0:{}: operator on a {}", self.line, x.ty.name()))?;
        if y.ty.base() != Some(base) {
            return self.err(format!("mismatched types {} and {}", x.ty.name(), y.ty.name()));
        }
        if base == Base::B && !matches!(op, BitAnd | BitOr | BitXor) {
            return self.err("arithmetic on bools");
        }
        // Linear algebra.
        if op == Mul {
            match (&x.ty, &y.ty) {
                (Ty::Mat(c, r), Ty::Vec(_, n)) if c == n => {
                    let t = self.temp(&Ty::Vec(Base::F, *r));
                    self.emit(Op::MatMul { d: t.slot, a: x.slot, b: y.slot, ar: *r, inner: *c, bc: 1 });
                    return Ok(t);
                }
                (Ty::Vec(_, n), Ty::Mat(c, r)) if r == n => {
                    let t = self.temp(&Ty::Vec(Base::F, *c));
                    self.emit(Op::MatMul { d: t.slot, a: x.slot, b: y.slot, ar: 1, inner: *r, bc: *c });
                    return Ok(t);
                }
                (Ty::Mat(c1, r1), Ty::Mat(c2, r2)) if c1 == r2 => {
                    let t = self.temp(&Ty::Mat(*c2, *r1));
                    self.emit(Op::MatMul { d: t.slot, a: x.slot, b: y.slot, ar: *r1, inner: *c1, bc: *c2 });
                    return Ok(t);
                }
                (Ty::Mat(..), Ty::Mat(..)) | (Ty::Mat(..), Ty::Vec(..)) | (Ty::Vec(..), Ty::Mat(..)) => {
                    return self.err(format!("can't multiply {} by {}", x.ty.name(), y.ty.name()));
                }
                _ => {}
            }
        }
        let (n, rty) = match (&x.ty, &y.ty) {
            (a, b) if a == b => (a.size(), a.clone()),
            (Ty::Scalar(_), b) => (b.size(), b.clone()),
            (a, Ty::Scalar(_)) => (a.size(), a.clone()),
            _ => return self.err(format!("mismatched types {} and {}", x.ty.name(), y.ty.name())),
        };
        let (sa, sb) = (if x.ty.is_scalar() && n > 1 { 0 } else { 1 }, if y.ty.is_scalar() && n > 1 { 0 } else { 1 });
        let cmp = matches!(op, Lt | Gt | Le | Ge);
        if cmp && !x.ty.is_scalar() {
            return self.err("<, >, <=, >= need scalars (use lessThan() for vectors)");
        }
        let f = match (op, base) {
            (Add, Base::F) => F2::FAdd,
            (Sub, Base::F) => F2::FSub,
            (Mul, Base::F) => F2::FMul,
            (Div, Base::F) => F2::FDiv,
            (Add, _) => F2::IAdd,
            (Sub, _) => F2::ISub,
            (Mul, Base::I) => F2::IMul,
            (Mul, _) => F2::UMul,
            (Div, Base::I) => F2::IDiv,
            (Div, _) => F2::UDiv,
            (Mod, Base::I) => F2::IMod,
            (Mod, Base::U) => F2::UMod,
            (Mod, Base::F) => return self.err("'%' needs integers (use mod())"),
            (Shl, Base::I | Base::U) => F2::Shl,
            (Shr, Base::I) => F2::IShr,
            (Shr, Base::U) => F2::UShr,
            (BitAnd, _) => F2::And,
            (BitOr, _) => F2::Or,
            (BitXor, _) => F2::Xor,
            (Lt, Base::F) => F2::FLt,
            (Gt, Base::F) => F2::FGt,
            (Le, Base::F) => F2::FLe,
            (Ge, Base::F) => F2::FGe,
            (Lt, Base::I) => F2::ILt,
            (Gt, Base::I) => F2::IGt,
            (Le, Base::I) => F2::ILe,
            (Ge, Base::I) => F2::IGe,
            (Lt, _) => F2::ULt,
            (Gt, _) => F2::UGt,
            (Le, _) => F2::ULe,
            (Ge, _) => F2::UGe,
            _ => return self.err("bad operator"),
        };
        if matches!(op, BitAnd | BitOr | BitXor | Shl | Shr | Mod) && self.version == 100 {
            return self.err("bit operators and '%' need GLSL ES 3.00");
        }
        if cmp {
            let r = self.temp(&Ty::BOOL);
            self.emit(Op::Bin { f, d: r.slot, a: x.slot, b: y.slot, n: 1, sa: 1, sb: 1 });
            return Ok(r);
        }
        let r = self.temp(&rty);
        self.emit(Op::Bin { f, d: r.slot, a: x.slot, b: y.slot, n, sa, sb });
        Ok(r)
    }

    /// Lenient numeric promotion: int/uint with float becomes float.
    fn unify(&mut self, x: Val, y: Val) -> R<(Val, Val)> {
        let (bx, by) = (x.ty.base(), y.ty.base());
        match (bx, by) {
            (Some(a), Some(b)) if a != b => {
                if a == Base::F && b != Base::B {
                    let y = self.convert(y, Base::F);
                    return Ok((x, y));
                }
                if b == Base::F && a != Base::B {
                    let x = self.convert(x, Base::F);
                    return Ok((x, y));
                }
                if (a == Base::I && b == Base::U) || (a == Base::U && b == Base::I) {
                    let y = self.convert(y, a);
                    return Ok((x, y));
                }
                Ok((x, y))
            }
            _ => Ok((x, y)),
        }
    }

    /// Flatten values into component slots of base `to`.
    fn components(&mut self, vals: &[Val], to: Base) -> R<Vec<Slot>> {
        let mut out = Vec::new();
        for v in vals {
            let Some(b) = v.ty.base() else { return self.err(format!("can't build a vector from a {}", v.ty.name())) };
            let n = v.ty.size();
            let src = if b == to {
                v.slot
            } else {
                let t = self.alloc(n);
                match conv_op(b, to) {
                    Some(f) => self.emit(Op::Un { f, d: t, a: v.slot, n }),
                    None => self.emit(Op::Mov { d: t, s: v.slot, n }),
                };
                t
            };
            out.extend(src..src + n);
        }
        Ok(out)
    }

    fn construct(&mut self, ts: &TypeSpec, args: &[Expr]) -> R<Val> {
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.expr(a)?);
        }
        if !ts.dims.is_empty() {
            // Array constructor: float[3](a, b, c) / float[](…).
            let el = self.base_ty(&ts.ty)?;
            let n = match &ts.dims[0] {
                Some(e) => match self.const_eval(e) {
                    Some((_, v)) => v[0],
                    None => return self.err("array size must be constant"),
                },
                None => vals.len() as u32,
            };
            if n as usize != vals.len() {
                return self.err("wrong number of array constructor arguments");
            }
            let ty = Ty::Array(Box::new(el.clone()), n);
            let t = self.temp(&ty);
            for (i, v) in vals.into_iter().enumerate() {
                let v = self.coerce(v, &el)?;
                self.emit(Op::Mov { d: t.slot + i as u32 * el.size(), s: v.slot, n: el.size() });
            }
            return Ok(t);
        }
        let ty = self.base_ty(&ts.ty)?;
        if vals.is_empty() {
            return self.err(format!("{} constructor needs arguments", ty.name()));
        }
        match &ty {
            Ty::Struct(s) => {
                if vals.len() != s.fields.len() {
                    return self.err(format!("{} takes {} values", s.name, s.fields.len()));
                }
                let t = self.temp(&ty);
                let mut off = 0;
                for ((_, fty), v) in s.fields.iter().zip(vals) {
                    let v = self.coerce(v, fty)?;
                    self.emit(Op::Mov { d: t.slot + off, s: v.slot, n: fty.size() });
                    off += fty.size();
                }
                Ok(t)
            }
            Ty::Scalar(b) => {
                let c = self.components(&vals[..1], *b)?;
                let t = self.temp(&ty);
                self.emit(Op::Mov { d: t.slot, s: c[0], n: 1 });
                Ok(t)
            }
            Ty::Vec(b, n) => {
                let t = self.temp(&ty);
                if vals.len() == 1 && vals[0].ty.is_scalar() {
                    let c = self.components(&vals, *b)?;
                    self.emit(Op::Splat { d: t.slot, s: c[0], n: *n as u32 });
                    return Ok(t);
                }
                let c = self.components(&vals, *b)?;
                if c.len() < *n as usize {
                    return self.err(format!("not enough values for {}", ty.name()));
                }
                self.emit(Op::Gather { d: t.slot, s: c[..*n as usize].to_vec().into_boxed_slice() });
                Ok(t)
            }
            Ty::Mat(c, r) => {
                let (c, r) = (*c as u32, *r as u32);
                let t = self.temp(&ty);
                if vals.len() == 1 && vals[0].ty.is_scalar() {
                    let s = self.components(&vals, Base::F)?[0];
                    let z = self.kf(0.0);
                    let src: Vec<Slot> = (0..c * r).map(|k| if k / r == k % r { s } else { z.slot }).collect();
                    self.emit(Op::Gather { d: t.slot, s: src.into_boxed_slice() });
                    return Ok(t);
                }
                if vals.len() == 1 {
                    if let Ty::Mat(c2, r2) = vals[0].ty {
                        let (c2, r2) = (c2 as u32, r2 as u32);
                        let (z, o) = (self.kf(0.0).slot, self.kf(1.0).slot);
                        let src: Vec<Slot> = (0..c * r)
                            .map(|k| {
                                let (j, i) = (k / r, k % r);
                                if j < c2 && i < r2 {
                                    vals[0].slot + j * r2 + i
                                } else if i == j {
                                    o
                                } else {
                                    z
                                }
                            })
                            .collect();
                        self.emit(Op::Gather { d: t.slot, s: src.into_boxed_slice() });
                        return Ok(t);
                    }
                }
                let comps = self.components(&vals, Base::F)?;
                if comps.len() < (c * r) as usize {
                    return self.err(format!("not enough values for {}", ty.name()));
                }
                self.emit(Op::Gather { d: t.slot, s: comps[..(c * r) as usize].to_vec().into_boxed_slice() });
                Ok(t)
            }
            _ => self.err(format!("can't construct a {}", ty.name())),
        }
    }

    // ---------------------------------------------------------------- built-in functions

    fn genf(&mut self, f: F1, a: &Val) -> R<Val> {
        let a = self.convert(a.clone(), Base::F);
        let t = self.temp(&a.ty);
        self.emit(Op::Un { f, d: t.slot, a: a.slot, n: a.ty.size() });
        Ok(t)
    }

    /// An elementwise call over arguments that are all the result's shape or scalars.
    fn elementwise2(&mut self, f: F2, a: Val, b: Val, rbase: Option<Base>) -> R<Val> {
        let (a, b) = self.unify(a, b)?;
        let n = a.ty.size().max(b.ty.size());
        let shape = if a.ty.size() >= b.ty.size() { a.ty.clone() } else { b.ty.clone() };
        if (a.ty.size() != n && a.ty.size() != 1) || (b.ty.size() != n && b.ty.size() != 1) {
            return self.err("mismatched argument sizes");
        }
        let rty = match (rbase, &shape) {
            (Some(rb), Ty::Scalar(_)) => Ty::Scalar(rb),
            (Some(rb), Ty::Vec(_, n)) => Ty::Vec(rb, *n),
            _ => shape,
        };
        let t = self.temp(&rty);
        let sa = if a.ty.size() == 1 && n > 1 { 0 } else { 1 };
        let sb = if b.ty.size() == 1 && n > 1 { 0 } else { 1 };
        self.emit(Op::Bin { f, d: t.slot, a: a.slot, b: b.slot, n, sa, sb });
        Ok(t)
    }

    fn elementwise3(&mut self, f: F3, a: Val, b: Val, c: Val) -> R<Val> {
        let n = a.ty.size().max(b.ty.size()).max(c.ty.size());
        let shape = [&a, &b, &c].iter().find(|v| v.ty.size() == n).unwrap().ty.clone();
        for v in [&a, &b, &c] {
            if v.ty.size() != n && v.ty.size() != 1 {
                return self.err("mismatched argument sizes");
            }
        }
        let s = |v: &Val| if v.ty.size() == 1 && n > 1 { 0 } else { 1 };
        let t = self.temp(&shape);
        self.emit(Op::Tri { f, d: t.slot, a: a.slot, b: b.slot, c: c.slot, n, sa: s(&a), sb: s(&b), sc: s(&c) });
        Ok(t)
    }

    fn builtin(&mut self, name: &str, args: &[Expr]) -> R<Val> {
        let mut v = Vec::new();
        for a in args {
            v.push(self.expr(a)?);
        }
        let n = v.len();
        let want = |k: usize| -> R<()> { if n == k { Ok(()) } else { Err(format!("ERROR: 0:{}: {name}() takes {k} arguments", self.line)) } };
        let fl = |g: &mut Gen, x: &Val| g.convert(x.clone(), Base::F);
        macro_rules! un {
            ($f:expr) => {{
                want(1)?;
                return self.genf($f, &v[0]);
            }};
        }
        match name {
            "radians" => un!(F1::Radians),
            "degrees" => un!(F1::Degrees),
            "sin" => un!(F1::Sin),
            "cos" => un!(F1::Cos),
            "tan" => un!(F1::Tan),
            "asin" => un!(F1::Asin),
            "acos" => un!(F1::Acos),
            "sinh" => un!(F1::Sinh),
            "cosh" => un!(F1::Cosh),
            "tanh" => un!(F1::Tanh),
            "asinh" => un!(F1::Asinh),
            "acosh" => un!(F1::Acosh),
            "atanh" => un!(F1::Atanh),
            "exp" => un!(F1::Exp),
            "log" => un!(F1::Log),
            "exp2" => un!(F1::Exp2),
            "log2" => un!(F1::Log2),
            "sqrt" => un!(F1::Sqrt),
            "inversesqrt" => un!(F1::InvSqrt),
            "floor" => un!(F1::Floor),
            "ceil" => un!(F1::Ceil),
            "fract" => un!(F1::Fract),
            "trunc" => un!(F1::Trunc),
            "round" => un!(F1::Round),
            "roundEven" => un!(F1::RoundEven),
            "atan" if n == 1 => un!(F1::Atan),
            "atan" => {
                want(2)?;
                let (a, b) = (fl(self, &v[0]), fl(self, &v[1]));
                return self.elementwise2(F2::Atan2, a, b, None);
            }
            "abs" | "sign" => {
                want(1)?;
                let int = v[0].ty.base() == Some(Base::I);
                let f = match (name, int) {
                    ("abs", false) => F1::FAbs,
                    ("abs", true) => F1::IAbs,
                    (_, false) => F1::FSign,
                    _ => F1::ISign,
                };
                if int {
                    let t = self.temp(&v[0].ty);
                    self.emit(Op::Un { f, d: t.slot, a: v[0].slot, n: v[0].ty.size() });
                    return Ok(t);
                }
                return self.genf(f, &v[0]);
            }
            "isnan" | "isinf" => {
                want(1)?;
                let a = fl(self, &v[0]);
                let ty = match a.ty {
                    Ty::Vec(_, k) => Ty::Vec(Base::B, k),
                    _ => Ty::BOOL,
                };
                let t = self.temp(&ty);
                self.emit(Op::Un { f: if name == "isnan" { F1::IsNan } else { F1::IsInf }, d: t.slot, a: a.slot, n: a.ty.size() });
                return Ok(t);
            }
            "pow" | "mod" | "step" => {
                want(2)?;
                let f = match name {
                    "pow" => F2::Pow,
                    "mod" => F2::FMod,
                    _ => F2::Step,
                };
                let (a, b) = (fl(self, &v[0]), fl(self, &v[1]));
                return self.elementwise2(f, a, b, None);
            }
            "min" | "max" => {
                want(2)?;
                let (a, b) = self.unify(v[0].clone(), v[1].clone())?;
                let f = match (name, a.ty.base()) {
                    ("min", Some(Base::I)) => F2::IMin,
                    ("max", Some(Base::I)) => F2::IMax,
                    ("min", Some(Base::U)) => F2::UMin,
                    ("max", Some(Base::U)) => F2::UMax,
                    ("min", _) => F2::FMin,
                    _ => F2::FMax,
                };
                return self.elementwise2(f, a, b, None);
            }
            "clamp" => {
                want(3)?;
                let f = match v[0].ty.base() {
                    Some(Base::I) => F3::IClamp,
                    Some(Base::U) => F3::UClamp,
                    _ => F3::FClamp,
                };
                let (a, b, c) = if f == F3::FClamp { (fl(self, &v[0]), fl(self, &v[1]), fl(self, &v[2])) } else { (v[0].clone(), v[1].clone(), v[2].clone()) };
                return self.elementwise3(f, a, b, c);
            }
            "mix" => {
                want(3)?;
                if v[2].ty.base() == Some(Base::B) {
                    return self.elementwise3(F3::Select, v[0].clone(), v[1].clone(), v[2].clone());
                }
                let (a, b, c) = (fl(self, &v[0]), fl(self, &v[1]), fl(self, &v[2]));
                return self.elementwise3(F3::Mix, a, b, c);
            }
            "smoothstep" => {
                want(3)?;
                let (a, b, c) = (fl(self, &v[0]), fl(self, &v[1]), fl(self, &v[2]));
                return self.elementwise3(F3::Smoothstep, a, b, c);
            }
            "length" => {
                want(1)?;
                let a = fl(self, &v[0]);
                let t = self.temp(&Ty::FLOAT);
                self.emit(Op::Length { d: t.slot, a: a.slot, n: a.ty.size() });
                return Ok(t);
            }
            "distance" => {
                want(2)?;
                let (a, b) = (fl(self, &v[0]), fl(self, &v[1]));
                let dv = self.elementwise2(F2::FSub, a, b, None)?;
                let t = self.temp(&Ty::FLOAT);
                self.emit(Op::Length { d: t.slot, a: dv.slot, n: dv.ty.size() });
                return Ok(t);
            }
            "dot" => {
                want(2)?;
                let (a, b) = (fl(self, &v[0]), fl(self, &v[1]));
                if a.ty.size() != b.ty.size() {
                    return self.err("dot() needs two vectors of one size");
                }
                let t = self.temp(&Ty::FLOAT);
                self.emit(Op::Dot { d: t.slot, a: a.slot, b: b.slot, n: a.ty.size() });
                return Ok(t);
            }
            "cross" => {
                want(2)?;
                let t = self.temp(&Ty::Vec(Base::F, 3));
                self.emit(Op::Cross { d: t.slot, a: v[0].slot, b: v[1].slot });
                return Ok(t);
            }
            "normalize" => {
                want(1)?;
                let a = fl(self, &v[0]);
                let t = self.temp(&a.ty);
                self.emit(Op::Normalize { d: t.slot, a: a.slot, n: a.ty.size() });
                return Ok(t);
            }
            "faceforward" => {
                want(3)?;
                let t = self.temp(&v[0].ty);
                self.emit(Op::Faceforward { d: t.slot, a: v[0].slot, b: v[1].slot, c: v[2].slot, n: v[0].ty.size() });
                return Ok(t);
            }
            "reflect" => {
                want(2)?;
                let t = self.temp(&v[0].ty);
                self.emit(Op::Reflect { d: t.slot, a: v[0].slot, b: v[1].slot, n: v[0].ty.size() });
                return Ok(t);
            }
            "refract" => {
                want(3)?;
                let e = fl(self, &v[2]);
                let t = self.temp(&v[0].ty);
                self.emit(Op::Refract { d: t.slot, a: v[0].slot, b: v[1].slot, eta: e.slot, n: v[0].ty.size() });
                return Ok(t);
            }
            "matrixCompMult" => {
                want(2)?;
                return self.elementwise2(F2::FMul, v[0].clone(), v[1].clone(), None);
            }
            "outerProduct" => {
                want(2)?;
                let (r, c) = (v[0].ty.size() as u8, v[1].ty.size() as u8);
                let t = self.temp(&Ty::Mat(c, r));
                self.emit(Op::MatMul { d: t.slot, a: v[0].slot, b: v[1].slot, ar: r, inner: 1, bc: c });
                return Ok(t);
            }
            "transpose" => {
                want(1)?;
                let Ty::Mat(c, r) = v[0].ty else { return self.err("transpose() needs a matrix") };
                let t = self.temp(&Ty::Mat(r, c));
                self.emit(Op::Transpose { d: t.slot, a: v[0].slot, c, r });
                return Ok(t);
            }
            "determinant" | "inverse" => {
                want(1)?;
                let Ty::Mat(c, r) = v[0].ty else { return self.err(format!("{name}() needs a square matrix")) };
                if c != r {
                    return self.err(format!("{name}() needs a square matrix"));
                }
                if name == "determinant" {
                    let t = self.temp(&Ty::FLOAT);
                    self.emit(Op::Det { d: t.slot, a: v[0].slot, n: c });
                    return Ok(t);
                }
                let t = self.temp(&v[0].ty);
                self.emit(Op::Inverse { d: t.slot, a: v[0].slot, n: c });
                return Ok(t);
            }
            "lessThan" | "lessThanEqual" | "greaterThan" | "greaterThanEqual" | "equal" | "notEqual" => {
                want(2)?;
                let (a, b) = self.unify(v[0].clone(), v[1].clone())?;
                let base = a.ty.base().unwrap_or(Base::F);
                let f = match (name, base) {
                    ("lessThan", Base::F) => F2::FLt,
                    ("lessThanEqual", Base::F) => F2::FLe,
                    ("greaterThan", Base::F) => F2::FGt,
                    ("greaterThanEqual", Base::F) => F2::FGe,
                    ("lessThan", Base::I) => F2::ILt,
                    ("lessThanEqual", Base::I) => F2::ILe,
                    ("greaterThan", Base::I) => F2::IGt,
                    ("greaterThanEqual", Base::I) => F2::IGe,
                    ("lessThan", _) => F2::ULt,
                    ("lessThanEqual", _) => F2::ULe,
                    ("greaterThan", _) => F2::UGt,
                    ("greaterThanEqual", _) => F2::UGe,
                    ("equal", Base::F) => F2::FEq,
                    ("notEqual", Base::F) => F2::FNe,
                    ("equal", _) => F2::IEq,
                    _ => F2::INe,
                };
                return self.elementwise2(f, a, b, Some(Base::B));
            }
            "any" | "all" => {
                want(1)?;
                let t = self.temp(&Ty::BOOL);
                let (d, a, k) = (t.slot, v[0].slot, v[0].ty.size());
                self.emit(if name == "any" { Op::Any { d, a, n: k } } else { Op::All { d, a, n: k } });
                return Ok(t);
            }
            "not" => {
                want(1)?;
                let t = self.temp(&v[0].ty);
                self.emit(Op::Un { f: F1::Not, d: t.slot, a: v[0].slot, n: v[0].ty.size() });
                return Ok(t);
            }
            "dFdx" | "dFdy" | "fwidth" | "dFdxFine" | "dFdyFine" | "dFdxCoarse" | "dFdyCoarse" | "fwidthFine" | "fwidthCoarse" => {
                want(1)?;
                if self.stage != Stage::Fragment {
                    return self.err(format!("{name}() is only available in fragment shaders"));
                }
                let a = fl(self, &v[0]);
                let t = self.temp(&a.ty);
                let kind = if name.starts_with("dFdx") {
                    0
                } else if name.starts_with("dFdy") {
                    1
                } else {
                    2
                };
                self.emit(Op::Deriv { d: t.slot, a: a.slot, n: a.ty.size(), kind });
                return Ok(t);
            }
            "modf" => {
                want(2)?;
                let lv = self.lvalue(&args[1])?;
                let t = self.temp(&v[0].ty);
                let ip = self.temp(&v[0].ty);
                self.emit(Op::Modf { d: t.slot, i: ip.slot, a: v[0].slot, n: v[0].ty.size() });
                self.store(&lv, &ip);
                return Ok(t);
            }
            "floatBitsToInt" | "floatBitsToUint" | "intBitsToFloat" | "uintBitsToFloat" => {
                want(1)?;
                let b = match name {
                    "floatBitsToInt" => Base::I,
                    "floatBitsToUint" => Base::U,
                    _ => Base::F,
                };
                let ty = match v[0].ty {
                    Ty::Vec(_, k) => Ty::Vec(b, k),
                    _ => Ty::Scalar(b),
                };
                return Ok(Val { ty, slot: v[0].slot });
            }
            "packHalf2x16" | "packUnorm2x16" | "packSnorm2x16" => {
                want(1)?;
                let t = self.temp(&Ty::Scalar(Base::U));
                let kind = match name {
                    "packHalf2x16" => 0,
                    "packUnorm2x16" => 1,
                    _ => 2,
                };
                self.emit(Op::Pack { d: t.slot, a: v[0].slot, kind });
                return Ok(t);
            }
            "unpackHalf2x16" | "unpackUnorm2x16" | "unpackSnorm2x16" => {
                want(1)?;
                let t = self.temp(&Ty::Vec(Base::F, 2));
                let kind = match name {
                    "unpackHalf2x16" => 0,
                    "unpackUnorm2x16" => 1,
                    _ => 2,
                };
                self.emit(Op::Unpack { d: t.slot, a: v[0].slot, kind });
                return Ok(t);
            }
            _ => {}
        }
        if let Some(r) = self.texture_fn(name, &v, args)? {
            return Ok(r);
        }
        self.err(format!("no function '{name}'"))
    }

    fn texture_fn(&mut self, name: &str, v: &[Val], args: &[Expr]) -> R<Option<Val>> {
        // (kind, proj, offset-arg index, legacy sampler dim)
        let (kind, proj, has_offset) = match name {
            "texture2D" | "textureCube" | "texture" | "shadow2DEXT" => (TexKind::Sample, false, false),
            "texture2DProj" | "textureProj" => (TexKind::Sample, true, false),
            "texture2DLod" | "textureCubeLod" | "texture2DLodEXT" | "textureCubeLodEXT" | "textureLod" => (TexKind::Lod, false, false),
            "texture2DProjLod" | "texture2DProjLodEXT" | "textureProjLod" => (TexKind::Lod, true, false),
            "texture2DGradEXT" | "textureCubeGradEXT" | "textureGrad" => (TexKind::Grad, false, false),
            "texture2DProjGradEXT" | "textureProjGrad" => (TexKind::Grad, true, false),
            "textureOffset" => (TexKind::Sample, false, true),
            "textureProjOffset" => (TexKind::Sample, true, true),
            "textureLodOffset" => (TexKind::Lod, false, true),
            "textureProjLodOffset" => (TexKind::Lod, true, true),
            "textureGradOffset" => (TexKind::Grad, false, true),
            "textureProjGradOffset" => (TexKind::Grad, true, true),
            "texelFetch" => (TexKind::Fetch, false, false),
            "texelFetchOffset" => (TexKind::Fetch, false, true),
            "textureSize" => (TexKind::Size, false, false),
            _ => return Ok(None),
        };
        let legacy = name.starts_with("texture2D") || name.starts_with("textureCube") || name.starts_with("shadow2D");
        if legacy && self.version == 300 {
            return self.err(format!("{name}() isn't available in GLSL ES 3.00 (use texture())"));
        }
        if !legacy && self.version == 100 {
            return self.err(format!("{name}() needs GLSL ES 3.00"));
        }
        let Some(s) = v.first() else { return self.err(format!("{name}() needs a sampler")) };
        let Ty::Sampler(samp) = s.ty else { return self.err(format!("{name}() needs a sampler first")) };
        if kind == TexKind::Lod && self.stage == Stage::Fragment && name.ends_with("Lod") && legacy && !name.contains("EXT") {
            return self.err(format!("{name}() is only available in vertex shaders"));
        }
        let ret = match (samp.shadow, samp.base) {
            (true, _) => Ty::FLOAT,
            (_, Base::I) => Ty::Vec(Base::I, 4),
            (_, Base::U) => Ty::Vec(Base::U, 4),
            _ => Ty::Vec(Base::F, 4),
        };
        if kind == TexKind::Size {
            let dims = match samp.dim {
                Dim::D2 | Dim::Cube => 2,
                _ => 3,
            };
            let t = self.temp(&Ty::Vec(Base::I, dims));
            let arg = v.get(1).map_or(NONE, |x| x.slot);
            self.emit(Op::Tex(Box::new(TexOp {
                d: t.slot,
                samp: s.slot,
                samp_ty: samp,
                coord: 0,
                ncoord: dims,
                kind,
                proj: false,
                arg,
                dx: NONE,
                dy: NONE,
                offset: None,
                compare: NONE,
            })));
            return Ok(Some(t));
        }
        let Some(c) = v.get(1) else { return self.err(format!("{name}() needs coordinates")) };
        let c = if kind == TexKind::Fetch { c.clone() } else { self.convert(c.clone(), Base::F) };
        let mut i = 2;
        let (mut dx, mut dy) = (NONE, NONE);
        if kind == TexKind::Grad {
            let (Some(a), Some(b)) = (v.get(2), v.get(3)) else { return self.err(format!("{name}() needs gradients")) };
            dx = a.slot;
            dy = b.slot;
            i = 4;
        }
        let mut arg = NONE;
        if matches!(kind, TexKind::Lod | TexKind::Fetch) {
            let Some(a) = v.get(i) else { return self.err(format!("{name}() needs a level")) };
            arg = if kind == TexKind::Fetch { self.convert(a.clone(), Base::I).slot } else { self.convert(a.clone(), Base::F).slot };
            i += 1;
        }
        let mut offset = None;
        if has_offset {
            let Some(e) = args.get(i) else { return self.err(format!("{name}() needs an offset")) };
            let Some((_, o)) = self.const_eval(e) else { return self.err("texture offsets must be constant") };
            let mut off = [0i32; 3];
            for (k, x) in o.iter().take(3).enumerate() {
                off[k] = *x as i32;
            }
            offset = Some(off);
            i += 1;
        }
        let compare = NONE;
        if kind == TexKind::Sample {
            if let Some(b) = v.get(i) {
                // samplerCubeShadow with vec4 coords takes no bias; texture(sampler2DArrayShadow…) neither.
                arg = self.convert(b.clone(), Base::F).slot;
            }
        }
        let t = self.temp(&ret);
        let tmp4 = self.temp(&Ty::Vec(Base::F, 4));
        let d = if samp.shadow { tmp4.slot } else { t.slot };
        self.emit(Op::Tex(Box::new(TexOp {
            d,
            samp: s.slot,
            samp_ty: samp,
            coord: c.slot,
            ncoord: c.ty.size() as u8,
            kind,
            proj,
            arg,
            dx,
            dy,
            offset,
            compare,
        })));
        if samp.shadow {
            self.emit(Op::Mov { d: t.slot, s: tmp4.slot, n: 1 });
        }
        Ok(Some(t))
    }
}

fn same_sig(a: &FuncDef, b: &FuncDef) -> bool {
    a.params.len() == b.params.len()
        && a.params.iter().zip(&b.params).all(|(x, y)| format!("{:?}{:?}", x.ty.ty, x.dims.len()) == format!("{:?}{:?}", y.ty.ty, y.dims.len()))
}

fn const_convert(x: u32, from: Base, to: Base) -> u32 {
    use Base::*;
    match (from, to) {
        (a, b) if a == b => x,
        (F, I) => (f32::from_bits(x) as i32) as u32,
        (F, U) => f32::from_bits(x) as u32,
        (F, B) => (f32::from_bits(x) != 0.0) as u32,
        (I, F) => (x as i32 as f32).to_bits(),
        (U, F) => (x as f32).to_bits(),
        (B, F) => if x != 0 { 1.0f32 } else { 0.0 }.to_bits(),
        (_, B) => (x != 0) as u32,
        _ => x,
    }
}

fn convert_const(from: &Ty, to: &Ty, v: Vec<u32>) -> Vec<u32> {
    match (from.base(), to.base()) {
        (Some(a), Some(b)) if a != b => v.into_iter().map(|x| const_convert(x, a, b)).collect(),
        _ => v,
    }
}

/// Which slots of a type hold floats (for == on structs and arrays).
fn float_slots(t: &Ty) -> Vec<bool> {
    match t {
        Ty::Scalar(b) => vec![*b == Base::F],
        Ty::Vec(b, n) => vec![*b == Base::F; *n as usize],
        Ty::Mat(c, r) => vec![true; (*c * *r) as usize],
        Ty::Struct(s) => s.fields.iter().flat_map(|f| float_slots(&f.1)).collect(),
        Ty::Array(el, n) => (0..*n).flat_map(|_| float_slots(el)).collect(),
        _ => vec![false; t.size() as usize],
    }
}

fn round_up(x: u32, a: u32) -> u32 {
    x.div_ceil(a) * a
}

/// std140 base alignment and size (bytes) of a non-array member.
fn std140(t: &Ty) -> (u32, u32) {
    match t {
        Ty::Scalar(_) => (4, 4),
        Ty::Vec(_, 2) => (8, 8),
        Ty::Vec(_, 3) => (16, 12),
        Ty::Vec(_, _) => (16, 16),
        // Matrices: an array of column vectors, each padded to 16 bytes.
        Ty::Mat(c, _) => (16, 16 * *c as u32),
        Ty::Struct(s) => {
            let mut off = 0;
            for (_, f) in &s.fields {
                let (a, sz) = std140(f);
                off = round_up(off, a.max(16).min(if matches!(f, Ty::Struct(_)) { 16 } else { a }));
                off += sz;
            }
            (16, round_up(off, 16))
        }
        Ty::Array(el, n) => {
            let (_, sz) = std140(el);
            (16, round_up(sz, 16) * n)
        }
        _ => (4, 4),
    }
}

/// Copy std140 block data into a block member's slots.
pub fn read_std140(el: &Ty, data: &[u8], mut off: usize, stride: usize, count: u32, mem: &mut [Lanes], slot: Slot) {
    let rd = |o: usize| -> f32 {
        data.get(o..o + 4).map_or(0.0, |b| f32::from_bits(u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
    };
    let mut s = slot as usize;
    for _ in 0..count {
        match el {
            Ty::Mat(c, r) => {
                for j in 0..*c as usize {
                    for i in 0..*r as usize {
                        mem[s] = [rd(off + j * 16 + i * 4); 4];
                        s += 1;
                    }
                }
            }
            Ty::Struct(st) => {
                let mut fo = 0u32;
                for (_, f) in &st.fields {
                    let (a, sz) = std140(f);
                    fo = round_up(fo, a);
                    let (fel, fc, fstride) = match f {
                        Ty::Array(e, n) => ((**e).clone(), *n, round_up(std140(e).1, 16)),
                        t => (t.clone(), 1, sz),
                    };
                    read_std140(&fel, data, off + fo as usize, fstride as usize, fc, mem, s as Slot);
                    s += f.size() as usize;
                    fo += sz;
                }
            }
            t => {
                for k in 0..t.size() as usize {
                    mem[s] = [rd(off + k * 4); 4];
                    s += 1;
                }
            }
        }
        off += stride;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glsl_vm::{run, NoTex};

    /// Compile a fragment shader and run it on one quad; returns gl_FragColor per lane.
    fn frag(src: &str) -> [[f32; 4]; 4] {
        let c = compile(src, Stage::Fragment, true).unwrap_or_else(|e| panic!("{e}"));
        let mut mem = c.mem.clone();
        if let Some(fc) = c.b.frag_coord {
            for l in 0..4 {
                mem[fc as usize][l] = (l % 2) as f32 + 0.5;
                mem[fc as usize + 1][l] = (l / 2) as f32 + 0.5;
            }
        }
        run(&c.code, &mut mem, 0xF, &NoTex, true);
        let out = c.b.frag_color.or(c.outputs.first().map(|o| o.slot)).expect("output");
        let mut r = [[0.0; 4]; 4];
        for l in 0..4 {
            for k in 0..4 {
                r[l][k] = mem[out as usize + k][l];
            }
        }
        r
    }

    #[test]
    fn arithmetic_swizzles_and_matrices() {
        let r = frag("precision mediump float; void main() { vec3 a = vec3(1.0, 2.0, 3.0); mat2 m = mat2(1.0, 2.0, 3.0, 4.0); vec2 v = m * vec2(1.0, 1.0); gl_FragColor = vec4(a.zy, v.x / 4.0, dot(a, a)); }");
        assert_eq!(r[0], [3.0, 2.0, 1.0, 14.0]);
    }

    #[test]
    fn divergent_branches_loops_and_functions() {
        let src = "precision mediump float;
            float f(float x) { if (x > 1.0) return 10.0; return x; }
            void main() {
                float s = 0.0;
                for (int i = 0; i < 10; i++) { if (float(i) >= gl_FragCoord.x * 2.0) break; s += 1.0; }
                float b = gl_FragCoord.y > 1.0 ? 1.0 : 0.0;
                gl_FragColor = vec4(s, b, f(gl_FragCoord.x), 0.0);
            }";
        let r = frag(src);
        // x = 0.5 → loop runs i = 0 only; x = 1.5 → i = 0, 1, 2.
        assert_eq!(r[0], [1.0, 0.0, 0.5, 0.0]);
        assert_eq!(r[1], [3.0, 0.0, 10.0, 0.0]);
        assert_eq!(r[2], [1.0, 1.0, 0.5, 0.0]);
    }

    #[test]
    fn derivatives_and_dynamic_indexing() {
        let src = "precision mediump float; uniform float k[3];
            void main() { float v[3]; v[0] = 1.0; v[1] = 2.0; v[2] = 3.0; int i = int(gl_FragCoord.x);
                gl_FragColor = vec4(dFdx(gl_FragCoord.x * 3.0), dFdy(gl_FragCoord.y), v[i + 1], 0.0); }";
        let r = frag(src);
        assert_eq!(r[0], [3.0, 1.0, 2.0, 0.0]);
        assert_eq!(r[1], [3.0, 1.0, 3.0, 0.0]);
    }

    #[test]
    fn glsl3_ints_and_outputs() {
        let src = "#version 300 es\nprecision highp float; out vec4 o;
            void main() { uint u = 7u; int x = -7 / 2; ivec2 b = ivec2(5, 3) & ivec2(3); o = vec4(float(u >> 1u), float(x), float(b.x + b.y), float(x % 2)); }";
        let r = frag(src);
        assert_eq!(r[0], [3.0, -3.0, 4.0, -1.0]);
    }

    #[test]
    fn discards_and_switch() {
        let src = "#version 300 es\nprecision highp float; out vec4 o;
            void main() { int i = int(gl_FragCoord.x); float r = 0.0;
              switch (i) { case 0: r = 5.0; break; case 1: r = 6.0; default: r += 1.0; }
              o = vec4(r); }";
        let r = frag(src);
        assert_eq!(r[0][0], 5.0);
        assert_eq!(r[1][0], 7.0);
    }

    #[test]
    fn unread_inputs_arent_listed() {
        let c = compile("#version 300 es\nprecision highp float; in vec3 vNormal; in vec2 vUv; out vec4 o; void main() { o = vec4(vUv, 0.0, 1.0); }", Stage::Fragment, true).unwrap();
        let names: Vec<&str> = c.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["vUv"]);
    }

    #[test]
    fn rejects_bad_shaders() {
        assert!(compile("void main() { gl_FragColor = vec4(undefinedVar); }", Stage::Fragment, false).is_err());
        assert!(compile("void main() { vec3 a; a.xx = vec2(1.0); }", Stage::Fragment, false).is_err());
        assert!(compile("void f(); void main() { gl_Position = vec4(1.0); }", Stage::Vertex, false).is_ok());
    }
}
