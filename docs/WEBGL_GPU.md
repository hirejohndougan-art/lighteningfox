# WebGL on the GPU: findings and plan

O4 asked for a look at drawing WebGL on the GPU instead of in software. This is what that look
found, measured on the build machine (2 cores, no GPU), and how to get there.

## Where things stand

- WebGL 1 and 2 run entirely on the CPU. `glsl.rs` / `glsl_gen.rs` parse and type-check GLSL
  ES 1.00 and 3.00 and compile it to the register VM in `glsl_vm.rs`. `webgl_raster.rs`
  rasterizes and shades 2×2 quads, splitting big draws into bands, one thread each.
- Conformance is high (conformance2 without textures: 173/258 pages, 98% of checks; textures:
  861/926 pages), so correctness is not the problem. Speed is.
- `gpu.rs` already brings up a `wgpu` device (Vulkan, GLES, Metal or DX12) to present the
  window. WebGL does not use it.

## How slow the software path is

Measured with a WebGL 2 page in the test harness (`--release`, 2 threads):

| Draw | Time per draw |
| --- | --- |
| Full-screen 1280×720 quad, one texture fetch and a `sin` per pixel | 77 ms |
| 20,000 small flat-shaded triangles | 26 ms |

One full-screen pass is about 13 frames per second; a typical three.js scene (shadow map, main
pass, a post-processing pass or two) lands at 2–5 frames per second at that size. A GPU does
each of these in well under a millisecond. More threads help linearly at best (8 cores: about
4× faster), so the software path can't reach 60 fps for real 3D pages.

## Shaders: translate the VM, not the source

- naga (already in the build through wgpu) has a GLSL front end, but `front/glsl/parser.rs`
  only accepts `#version 440/450/460` desktop GLSL. WebGL's ES 1.00 and 3.00 are rejected,
  and its semantics differ anyway: `gl_FragColor`, `attribute`/`varying`, combined sampler
  uniforms, precision qualifiers, WebGL's extra validation.
- `glsl_gen.rs` already does all the WebGL-specific parsing and checking. Its output, the VM
  program, is a good input for a GPU backend: control flow is already structured (`If/Else`,
  `LoopBegin…LoopEnd`, `Break`, `Continue`, `SwBegin/SwCase`, `Return`, `Discard`), and every op
  (`Bin`, `Tri`, `Dot`, `MatMul`, `Tex`, `Deriv`, `Pack` …) maps to a WGSL expression or
  built-in.
- So the backend is **VM program → WGSL text**: each slot becomes a `u32` register (floats
  through `bitcast<f32>`, as the VM stores them), each op one statement. naga's WGSL front end
  and validator (both already compiled in) check the result, and wgpu turns it into SPIR-V,
  MSL or HLSL. A program that doesn't translate falls back to the software path, so the
  backend can grow op by op.
- The translator can be tested without a GPU: translate every shader pair in the Khronos suite
  and count how many naga validates.

## Mapping WebGL state onto wgpu

| WebGL | wgpu |
| --- | --- |
| Program + vertex layout + blend/depth/stencil/cull/color-mask/topology/target formats | A `RenderPipeline`, cached by a hash of all of them |
| Viewport, scissor, blend color, stencil reference | Dynamic render-pass state |
| Uniforms (the VM's uniform slots) | One uniform buffer per program, rewritten on change |
| Texture units (texture + sampler state, or a sampler object) | A bind group per draw: texture view + `Sampler` (cached by parameters); shadow samplers are comparison samplers |
| Framebuffers, renderbuffers | Textures with `RENDER_ATTACHMENT`; multisampled ones get real 4× MSAA and resolve, so `antialias: true` can finally be honest |
| Occlusion queries | `QuerySet` of type `Occlusion` |
| `fenceSync` / `clientWaitSync` | `queue.on_submitted_work_done` |
| `readPixels`, `getBufferSubData` | Copy to a buffer, map, wait (slow but rare) |

What WebGL has and wgpu doesn't, with the fix for each:

- **Clip space and origin.** GL depth is −1…1, wgpu 0…1: the vertex shader writes
  `z = (z + w) / 2`. GL's window origin is bottom-left: flip Y in the viewport and in
  `gl_FragCoord`, and flip rows on readback.
- **TRIANGLE_FAN, LINE_LOOP, 8-bit indices, primitive restart with fans.** Rewrite to index
  lists on the CPU; widen `UNSIGNED_BYTE` indices to 16-bit.
- **Points bigger than 1 px.** wgpu draws 1-pixel points only: expand each point to a quad
  (vertex pulling or instancing) and compute `gl_PointCoord` in the shader.
- **Formats wgpu lacks.** RGB8 and the other 3-channel formats are stored as RGBA; LUMINANCE /
  ALPHA / LUMINANCE_ALPHA become R8/RG8 with a swizzle in the generated shader.
- **Transform feedback.** No wgpu equivalent: run the vertex stage as a compute shader that
  writes the captured varyings into a storage buffer. Until then, transform-feedback draws use
  the software path.
- **Robust buffer access.** WebGL requires out-of-range vertex fetches to be safe. Check
  attribute ranges on the CPU before the draw, as the software path does now.

## Keeping CPU and GPU copies in step

During the move, both paths exist. Each image (texture level, renderbuffer, default framebuffer)
gets a "where is the newest copy" flag: CPU, GPU or both. A software-only draw or upload
downloads what it needs first; a GPU draw uploads first. Pages that only use translated
programs never copy back, except for `readPixels` and compositing the canvas.

## Processes

Site processes (X15) are sandboxed and should not open the GPU driver. Two steps:

1. **In-process first.** When a page runs in the browser process (site processes off, as in
   tests), the WebGL context opens its own headless wgpu device. The canvas is handed to the
   page painter as a texture: it's shared with `gpu.rs` when both run in one process, or read
   back otherwise.
2. **A GPU process later**, like the media broker (X23): site processes send a command stream
   (the WebGL call log with buffer contents) over the existing socket. A small GPU process
   replays it on wgpu and shares the canvas image with the browser process. `readPixels` and
   queries become round trips.

## Plan

1. VM → WGSL translator plus naga validation, tested on every Khronos shader pair (no GPU
   needed). Target: the shaders behind 95% of conformance pages translate.
2. Headless wgpu WebGL backend for translated programs, in-process only: buffers, textures,
   framebuffers, pipeline cache, draws, readPixels. Run the Khronos suite with
   `IRONSWIFT_GPU=1` on a software adapter (lavapipe) in CI and compare against the software
   path.
3. MSAA, queries, fences and the compute-shader transform feedback.
4. The GPU process and the command stream, so sandboxed site processes get it too.
5. Make the GPU path the default where an adapter exists. Keep the software path as the
   fallback for machines without one, and for anything not translated.

Each step can be a task of its own; step 1 is the one to start with.
