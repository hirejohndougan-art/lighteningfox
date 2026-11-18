//! WebGL 2 transform feedback: the state of a transform feedback object, the outputs a linked
//! program captures, and the order vertices are captured in. webgl.rs keeps the objects and
//! validates the calls; the rasterizer (webgl_raster.rs) copies the chosen vertex-shader
//! outputs as it shades, and webgl.rs writes them into the bound buffers.

use crate::glsl::Ty;

/// MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS (and buffer binding points).
pub const MAX_SEPARATE: usize = 4;
/// MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS.
pub const MAX_SEPARATE_COMPONENTS: u32 = 4;
/// MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS.
pub const MAX_INTERLEAVED_COMPONENTS: u32 = 64;
pub const INTERLEAVED_ATTRIBS: u32 = 0x8C8C;
pub const SEPARATE_ATTRIBS: u32 = 0x8C8D;

/// A transform feedback object (id 0 is the context's default one).
#[derive(Clone, Default)]
pub struct Tf {
    /// Buffer bound at each binding point: (buffer, offset, size; 0 = to the buffer's end).
    pub bind: [(u32, u32, u32); MAX_SEPARATE],
    pub active: bool,
    pub paused: bool,
    /// The primitive mode given to beginTransformFeedback (POINTS, LINES, TRIANGLES).
    pub mode: u32,
    /// The program in use when it began.
    pub program: u32,
    /// Bytes written at each binding point since it began.
    pub written: [u64; MAX_SEPARATE],
}

/// One captured output of a linked program.
#[derive(Clone, Debug)]
pub struct TfVar {
    pub name: String,
    pub ty: Ty,
    /// The vertex shader's output slot (None: gl_Position).
    pub slot: Option<u32>,
    pub comps: u32,
}

/// The vertex outputs `names` name, for a program linking with buffer mode `mode`.
pub fn resolve(names: &[String], mode: u32, outputs: &[crate::glsl_gen::IoVar]) -> Result<Vec<TfVar>, String> {
    let mut vars: Vec<TfVar> = Vec::new();
    for n in names {
        if vars.iter().any(|v| &v.name == n) {
            return Err(format!("transform feedback varying '{n}' is listed twice"));
        }
        let var = if n == "gl_Position" {
            TfVar { name: n.clone(), ty: Ty::Vec(crate::glsl::Base::F, 4), slot: None, comps: 4 }
        } else {
            let o = outputs.iter().find(|o| &o.name == n).ok_or_else(|| format!("transform feedback varying '{n}' isn't a vertex shader output"))?;
            TfVar { name: n.clone(), ty: o.ty.clone(), slot: Some(o.slot), comps: o.ty.size() }
        };
        vars.push(var);
    }
    if mode == SEPARATE_ATTRIBS && vars.iter().any(|v| v.comps > MAX_SEPARATE_COMPONENTS) {
        return Err("a separate transform feedback varying has too many components".into());
    }
    if mode == INTERLEAVED_ATTRIBS && vars.iter().map(|v| v.comps).sum::<u32>() > MAX_INTERLEAVED_COMPONENTS {
        return Err("the interleaved transform feedback varyings have too many components".into());
    }
    Ok(vars)
}

/// The transform feedback primitive mode a draw mode belongs to.
pub fn base_mode(draw_mode: u32) -> u32 {
    match draw_mode {
        0 => 0,
        1..=3 => 1,
        _ => 4,
    }
}

/// The vertices (as positions in `at`) captured for a draw, in order: each primitive's
/// vertices, strips and fans broken into separate lines and triangles.
pub fn order(mode: u32, n: usize) -> Vec<usize> {
    let mut out = Vec::new();
    match mode {
        0 => out.extend(0..n),
        1 => out.extend(0..n - n % 2),
        2 => (1..n).for_each(|i| out.extend([i - 1, i])),
        3 => {
            (1..n).for_each(|i| out.extend([i - 1, i]));
            if n >= 2 {
                out.extend([n - 1, 0]);
            }
        }
        4 => out.extend(0..n - n % 3),
        5 => (2..n).for_each(|i| if i % 2 == 0 { out.extend([i - 2, i - 1, i]) } else { out.extend([i - 1, i - 2, i]) }),
        6 => (2..n).for_each(|i| out.extend([0, i - 1, i])),
        _ => {}
    }
    out
}

/// Vertices per primitive for a transform feedback mode.
pub fn per_primitive(mode: u32) -> usize {
    match mode {
        0 => 1,
        1 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_order() {
        assert_eq!(order(5, 5), [0, 1, 2, 2, 1, 3, 2, 3, 4]);
        assert_eq!(order(6, 4), [0, 1, 2, 0, 2, 3]);
        assert_eq!(order(3, 3), [0, 1, 1, 2, 2, 0]);
        assert_eq!(order(4, 7), [0, 1, 2, 3, 4, 5]);
        assert_eq!(base_mode(2), 1);
    }
}
