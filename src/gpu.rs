//! GPU presenter (wgpu: Vulkan or OpenGL). The CPU still paints, but only what changed:
//! the toolbar strip and newly painted page tiles are uploaded as textures, and each frame
//! is a handful of textured rectangles. Scrolling moves the tile rectangles; nothing is
//! painted or uploaded.
//!
//! If no usable graphics adapter is found, `Gpu::new` returns None and the window falls back
//! to software presenting. By default only real GPUs are used; IRONSWIFT_GPU=1 also accepts software
//! adapters (useful for testing), IRONSWIFT_GPU=0 turns the GPU off.

use crate::tiles::{PageTiles, TILE_H};
use std::collections::HashMap;

const SHADER: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    return o;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs(i: VsOut) -> @location(0) vec4<f32> {
    // Canvas pixels are 0RGB; the alpha byte is unused.
    return vec4<f32>(textureSample(tex, samp, i.uv).rgb, 1.0);
}
"#;

struct Tex {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    w: u32,
    h: u32,
}

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vertices: wgpu::Buffer,
    /// The whole window (software-composed frames, and the toolbar strip in tiled mode).
    frame: Option<Tex>,
    tiles: HashMap<usize, Tex>,
    tiles_version: u64,
    /// Which paint of each page strip its texture holds.
    tile_gens: HashMap<usize, u64>,
    pub name: String,
}

/// Two triangles for a rectangle in pixels, with texture coordinates.
fn quad(out: &mut Vec<f32>, (x, y, w, h): (f32, f32, f32, f32), (u0, v0, u1, v1): (f32, f32, f32, f32), sw: f32, sh: f32) {
    let nx = |px: f32| px / sw * 2.0 - 1.0;
    let ny = |py: f32| 1.0 - py / sh * 2.0;
    let (x0, y0, x1, y1) = (nx(x), ny(y), nx(x + w), ny(y + h));
    for (px, py, u, v) in [(x0, y0, u0, v0), (x1, y0, u1, v0), (x0, y1, u0, v1), (x0, y1, u0, v1), (x1, y0, u1, v0), (x1, y1, u1, v1)] {
        out.extend_from_slice(&[px, py, u, v]);
    }
}

fn as_bytes(px: &[u32]) -> &[u8] {
    // SAFETY: u32 has no padding and any byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(px.as_ptr() as *const u8, std::mem::size_of_val(px)) }
}

const MAX_QUADS: u64 = 64;

impl Gpu {
    /// Set up drawing into `window`. The window must outlive the returned Gpu.
    pub fn new(window: &crate::window::Window, w: u32, h: u32) -> Option<Gpu> {
        match Self::setup(window, w, h) {
            Ok(g) => Some(g),
            Err(why) => {
                log!("[gpu] {why}");
                None
            }
        }
    }

    fn setup(window: &crate::window::Window, w: u32, h: u32) -> Result<Gpu, String> {
        let mode = std::env::var("IRONSWIFT_GPU").unwrap_or_default();
        if mode == "0" {
            return Err("turned off (IRONSWIFT_GPU=0)".into());
        }
        if mode != "1" && !crate::settings::get().use_gpu {
            return Err("turned off in settings".into());
        }
        let allow_software = mode == "1";
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        // SAFETY: the window lives for the whole run of the window loop, which owns this Gpu
        // and drops it before the window.
        let surface = unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_display_and_window(window, window).map_err(|e| format!("window handle: {e}"))?) }.map_err(|e| format!("surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no adapter: {e}"))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu && !allow_software {
            return Err(format!("only a software adapter ({}); set IRONSWIFT_GPU=1 to use it", info.name));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ironswift"),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("device: {e}"))?;
        let caps = surface.get_capabilities(&adapter);
        // Canvas pixels are plain (not gamma-encoded) 8-bit values: prefer a non-sRGB format so
        // colors come out exactly as painted.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
            .ok_or("surface has no formats")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: w.max(1),
            height: h.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], ..Default::default() });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        // Nearest sampling: textures are drawn at exactly their own size, pixel for pixel.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: MAX_QUADS * 6 * 16,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let name = format!("{} ({:?})", info.name, info.backend);
        Ok(Gpu { surface, device, queue, config, pipeline, layout, sampler, vertices, frame: None, tiles: HashMap::new(), tiles_version: 0, tile_gens: HashMap::new(), name })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if (w, h) != (self.config.width, self.config.height) && w > 0 && h > 0 {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn make_tex(&self, w: u32, h: u32) -> Tex {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        Tex { texture, bind, w, h }
    }

    /// Copy `rows` rows of 0RGB pixels (row length `w`) into the top of a texture.
    fn upload(&self, tex: &Tex, px: &[u32], w: u32, rows: u32) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            as_bytes(&px[..(w * rows) as usize]),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
        );
    }

    fn frame_tex(&mut self, w: u32, h: u32) {
        if self.frame.as_ref().is_none_or(|f| f.w != w || f.h != h) {
            self.frame = Some(self.make_tex(w, h));
        }
    }

    /// Present a frame painted entirely on the CPU (`top_rows` = how much of it to upload;
    /// the rest of the window shows page tiles).
    pub fn present(&mut self, canvas: &[u32], w: u32, h: u32, top_rows: u32, tiles: Option<(&PageTiles, u64, f32)>) {
        self.resize(w, h);
        self.frame_tex(w, h);
        let frame = self.frame.as_ref().expect("frame texture");
        self.upload(frame, canvas, w, top_rows.min(h));

        // Page tiles: drop the GPU copies of tiles the page no longer has, upload new ones.
        let mut quads: Vec<(f32, f32, f32, f32, f32, f32, f32, f32, Option<usize>)> = Vec::new();
        quads.push((0.0, 0.0, w as f32, top_rows as f32, 0.0, 0.0, 1.0, top_rows as f32 / h as f32, None));
        if let Some((pt, version, scroll)) = tiles {
            // Strips repainted since they were uploaded are uploaded again; unchanged ones stay.
            if pt.width() != w as usize {
                self.tiles.clear();
            }
            self.tiles_version = version;
            let gens = &self.tile_gens;
            self.tiles.retain(|i, _| pt.get(*i).is_some() && pt.generation(*i).is_some_and(|g| gens.get(i) == Some(&g)));
            let view_h = h.saturating_sub(top_rows) as usize;
            for idx in PageTiles::visible(scroll, view_h) {
                let Some(px) = pt.get(idx) else { continue };
                if !self.tiles.contains_key(&idx) {
                    let t = self.make_tex(w, TILE_H as u32);
                    self.upload(&t, px, w, TILE_H as u32);
                    self.tiles.insert(idx, t);
                    if let Some(g) = pt.generation(idx) {
                        self.tile_gens.insert(idx, g);
                    }
                }
                let y = top_rows as f32 + (idx * TILE_H) as f32 - scroll;
                quads.push((0.0, y, w as f32, TILE_H as f32, 0.0, 0.0, 1.0, 1.0, Some(idx)));
            }
        }

        let mut verts: Vec<f32> = Vec::new();
        for q in quads.iter().take(MAX_QUADS as usize) {
            quad(&mut verts, (q.0, q.1, q.2, q.3), (q.4, q.5, q.6, q.7), w as f32, h as f32);
        }
        self.queue.write_buffer(&self.vertices, 0, as_f32_bytes(&verts));

        let out = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
        let view = out.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            for (i, q) in quads.iter().take(MAX_QUADS as usize).enumerate() {
                let bind = match q.8 {
                    None => &frame.bind,
                    Some(idx) => &self.tiles[&idx].bind,
                };
                // Page tiles stay below the toolbar.
                if q.8.is_some() {
                    pass.set_scissor_rect(0, top_rows.min(h - 1), w, h - top_rows.min(h - 1));
                } else {
                    pass.set_scissor_rect(0, 0, w, h);
                }
                pass.set_bind_group(0, bind, &[]);
                let v = (i * 6) as u32;
                pass.draw(v..v + 6, 0..1);
            }
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(out);
    }
}

fn as_f32_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and any byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}
