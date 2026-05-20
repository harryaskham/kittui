//! Shader pipelines used by the GPU renderer.
//!
//! The v1 design (per DESIGN.md `## Renderer architecture`) collapses every
//! node type into three pipelines:
//!
//! - `rounded_rect_sdf`: rect fills + strokes + per-corner radii.
//! - `gradient`: linear / vertical / horizontal / diagonal / radial.
//! - `glow_radial`: smoothstep radial falloff with phase-aware intensity.
//!
//! For v0.4 we ship only the shared full-screen-triangle pipeline plus
//! the rounded-rect SDF shader. Gradient and glow currently route through
//! the same fragment shader by switching on a `kind` uniform — this keeps
//! the pipeline count at one until the parity gate forces more. Scanlines
//! are produced by the gradient shader with a stripe-period uniform.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use parking_lot::Mutex;

use crate::device::GpuDevice;

/// Uniform buffer that controls what the unified fragment shader paints.
/// The `kind` field selects which branch the shader follows:
///
/// | kind | meaning                                                    |
/// |------|------------------------------------------------------------|
/// |   0  | rounded-rect fill + optional stroke + per-corner radii     |
/// |   1  | gradient (linear via `dir`)                                |
/// |   2  | radial glow                                                |
/// |   3  | scanlines (stripe period in `period_px`)                   |
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Uniforms {
    /// Viewport size in pixels.
    pub viewport: [f32; 2],
    /// Pad to vec4 alignment.
    pub _pad0: [f32; 2],
    /// Node rect: x, y, w, h in pixels.
    pub rect: [f32; 4],
    /// Fill RGBA in [0,1].
    pub fill: [f32; 4],
    /// Stroke RGBA in [0,1].
    pub stroke: [f32; 4],
    /// `stroke_width_px, corner_tl, corner_tr, corner_bl`.
    pub stroke_radii_a: [f32; 4],
    /// `corner_br, kind, dir, intensity`.
    pub stroke_radii_b: [f32; 4],
    /// Gradient start color (for kind=1).
    pub grad_start: [f32; 4],
    /// Gradient end color.
    pub grad_end: [f32; 4],
    /// `center_x_frac, center_y_frac, radius_frac, phase`.
    pub glow: [f32; 4],
    /// `scanline_alpha, scanline_period_px, _, _`.
    pub scan: [f32; 4],
}

impl Uniforms {
    /// Zero-initialised uniforms.
    pub fn zeroed() -> Self {
        Self {
            viewport: [0.0; 2],
            _pad0: [0.0; 2],
            rect: [0.0; 4],
            fill: [0.0; 4],
            stroke: [0.0; 4],
            stroke_radii_a: [0.0; 4],
            stroke_radii_b: [0.0; 4],
            grad_start: [0.0; 4],
            grad_end: [0.0; 4],
            glow: [0.0; 4],
            scan: [0.0; 4],
        }
    }
}

/// Pipeline bundle. The DESIGN promises one pipeline per shape
/// family (rounded_rect_sdf / gradient / glow_radial); v0.5 honors
/// that by compiling three pipelines that share the same uniform
/// layout but have distinct fragment entry points. Scanlines are a
/// fourth small pipeline (cheap to keep separate; lets us drop the
/// `kind` branch out of the hot shader path).
pub struct Pipelines {
    /// Pipeline for `Node::Rect` (rounded rectangle SDF + stroke).
    pub rect_pipeline: wgpu::RenderPipeline,
    /// Pipeline for `Node::Gradient`.
    pub gradient_pipeline: wgpu::RenderPipeline,
    /// Pipeline for `Node::Glow`.
    pub glow_pipeline: wgpu::RenderPipeline,
    /// Pipeline for `Node::Scanlines`.
    pub scanlines_pipeline: wgpu::RenderPipeline,
    /// Shared bind group layout.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Dynamic cache of user-shader pipelines keyed by WGSL source hash.
    pub shader_cache: Mutex<HashMap<u64, wgpu::RenderPipeline>>,
}

impl Pipelines {
    /// Compile shaders and build the four shape pipelines.
    pub fn new(device: &GpuDevice) -> Self {
        let shader = device
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kittui-shader"),
                source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
            });

        let bind_group_layout =
            device
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("kittui-bg-layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let pipeline_layout =
            device
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("kittui-pl-layout"),
                    bind_group_layouts: &[&bind_group_layout],
                    push_constant_ranges: &[],
                });

        let make = |label: &str, fs: &str| {
            device
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fs),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                })
        };

        Self {
            rect_pipeline: make("kittui-pipeline-rect", "fs_rect"),
            gradient_pipeline: make("kittui-pipeline-gradient", "fs_gradient"),
            glow_pipeline: make("kittui-pipeline-glow", "fs_glow"),
            scanlines_pipeline: make("kittui-pipeline-scanlines", "fs_scanlines"),
            bind_group_layout,
            shader_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Compile (or fetch from cache) a user-supplied WGSL fragment shader
    /// keyed by a stable hash of `source`. Errors are propagated to the
    /// caller via [`ShaderError`].
    pub fn compile_user_shader(
        &self,
        device: &GpuDevice,
        source: &str,
    ) -> Result<wgpu::RenderPipeline, ShaderError> {
        let key = stable_hash(source);
        {
            let cache = self.shader_cache.lock();
            if let Some(pipeline) = cache.get(&key) {
                // Cheaply clone the pipeline handle.
                return Ok(pipeline.clone());
            }
        }

        let wrapped = wrap_user_shader(source);
        let module = device
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kittui-user-shader"),
                source: wgpu::ShaderSource::Wgsl(wrapped.into()),
            });

        // Single-bind-group layout, same as the canonical pipelines.
        let pipeline_layout = device
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kittui-user-pl-layout"),
                bind_group_layouts: &[&self.bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = device
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("kittui-user-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        self.shader_cache.lock().insert(key, pipeline.clone());
        Ok(pipeline)
    }
}

/// Errors compiling a user shader.
#[derive(Debug, thiserror::Error)]
pub enum ShaderError {
    /// WGSL parse or validation failed before pipeline creation.
    #[error("shader compile failed: {0}")]
    Compile(String),
}

fn stable_hash(source: &str) -> u64 {
    // FxHash-style mix; we only need stability within a process so a small
    // xorshift roll over bytes is enough.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in source.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn wrap_user_shader(user_source: &str) -> String {
    // Wrap the user's fragment with the standard vertex stage, uniform
    // binding, and an `fs_main` that calls into the user-supplied function.
    // The user is expected to declare:
    //
    //     @fragment fn user(@location(0) frag: vec2<f32>) -> @location(0) vec4<f32>
    //
    // ...and may read `U.user0..U.user3` to receive up to 4 vec4 uniforms.
    format!(
        r#"
struct Uniforms {{
  viewport: vec2<f32>,
  _pad0: vec2<f32>,
  rect: vec4<f32>,
  fill: vec4<f32>,
  stroke: vec4<f32>,
  stroke_radii_a: vec4<f32>,
  stroke_radii_b: vec4<f32>,
  grad_start: vec4<f32>,
  grad_end: vec4<f32>,
  glow: vec4<f32>,
  scan: vec4<f32>,
}};

@group(0) @binding(0) var<uniform> U: Uniforms;

struct VsOut {{
  @builtin(position) pos: vec4<f32>,
  @location(0) frag: vec2<f32>,
}};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {{
  var out: VsOut;
  let x = f32((vid & 1u) << 1u) - 1.0;
  let y = 1.0 - f32((vid & 2u));
  out.pos = vec4<f32>(x, y, 0.0, 1.0);
  out.frag = vec2<f32>(
    (x * 0.5 + 0.5) * U.viewport.x,
    (1.0 - (y * 0.5 + 0.5)) * U.viewport.y
  );
  return out;
}}

{user_source}

@fragment
fn fs_main(in_frag: VsOut) -> @location(0) vec4<f32> {{
  return user(in_frag.frag);
}}
"#
    )
}

const SHADER_WGSL: &str = r#"
struct Uniforms {
  viewport: vec2<f32>,
  _pad0: vec2<f32>,
  rect: vec4<f32>,
  fill: vec4<f32>,
  stroke: vec4<f32>,
  stroke_radii_a: vec4<f32>,
  stroke_radii_b: vec4<f32>,
  grad_start: vec4<f32>,
  grad_end: vec4<f32>,
  glow: vec4<f32>,
  scan: vec4<f32>,
};

@group(0) @binding(0) var<uniform> U: Uniforms;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) frag: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
  var out: VsOut;
  let x = f32((vid & 1u) << 1u) - 1.0;
  let y = 1.0 - f32((vid & 2u));
  out.pos = vec4<f32>(x, y, 0.0, 1.0);
  out.frag = vec2<f32>(
    (x * 0.5 + 0.5) * U.viewport.x,
    (1.0 - (y * 0.5 + 0.5)) * U.viewport.y
  );
  return out;
}

fn sd_rounded_rect(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>,
                   tl: f32, tr: f32, bl: f32, br: f32) -> f32 {
  let q = p - c;
  let r = select(
    select(bl, br, q.x > 0.0),
    select(tl, tr, q.x > 0.0),
    q.y < 0.0
  );
  let d = abs(q) - h + vec2<f32>(r, r);
  return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0))) - r;
}

fn over(src: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
  let a = src.a + dst.a * (1.0 - src.a);
  if (a <= 0.0) { return vec4<f32>(0.0); }
  let rgb = (src.rgb * src.a + dst.rgb * dst.a * (1.0 - src.a)) / a;
  return vec4<f32>(rgb, a);
}

fn rect_bounds(p: vec2<f32>) -> bool {
  return p.x >= U.rect.x && p.x <= U.rect.x + U.rect.z
      && p.y >= U.rect.y && p.y <= U.rect.y + U.rect.w;
}

// rounded_rect_sdf pipeline
@fragment
fn fs_rect(in: VsOut) -> @location(0) vec4<f32> {
  let p = in.frag;
  let cx = U.rect.x + U.rect.z * 0.5;
  let cy = U.rect.y + U.rect.w * 0.5;
  let half = vec2<f32>(U.rect.z * 0.5, U.rect.w * 0.5);
  let tl = U.stroke_radii_a.y;
  let tr = U.stroke_radii_a.z;
  let bl = U.stroke_radii_a.w;
  let br = U.stroke_radii_b.x;
  let sd = sd_rounded_rect(p, vec2<f32>(cx, cy), half, tl, tr, bl, br);
  let inside = clamp(0.5 - sd, 0.0, 1.0);
  var color = vec4<f32>(U.fill.rgb, U.fill.a * inside);
  let sw = U.stroke_radii_a.x;
  if (sw > 0.0 && U.stroke.a > 0.0) {
    let band = clamp(0.5 - abs(sd + sw * 0.5) / max(sw * 0.5, 0.001), 0.0, 1.0);
    let stroke_color = vec4<f32>(U.stroke.rgb, U.stroke.a * band);
    color = over(stroke_color, color);
  }
  return color;
}

// gradient pipeline (linear: horizontal / vertical / diagonal)
@fragment
fn fs_gradient(in: VsOut) -> @location(0) vec4<f32> {
  if (!rect_bounds(in.frag)) { return vec4<f32>(0.0); }
  let dir = i32(U.stroke_radii_b.z);
  let nx = (in.frag.x - U.rect.x) / max(U.rect.z, 0.0001);
  let ny = (in.frag.y - U.rect.y) / max(U.rect.w, 0.0001);
  var t: f32 = 0.0;
  if (dir == 0) { t = nx; }
  else if (dir == 1) { t = ny; }
  else { t = (nx + ny) * 0.5; }
  return mix(U.grad_start, U.grad_end, clamp(t, 0.0, 1.0));
}

// glow_radial pipeline
@fragment
fn fs_glow(in: VsOut) -> @location(0) vec4<f32> {
  let cx = U.rect.x + U.rect.z * U.glow.x;
  let cy = U.rect.y + U.rect.w * U.glow.y;
  let r = min(U.rect.z, U.rect.w) * U.glow.z;
  if (r <= 0.0) { return vec4<f32>(0.0); }
  let d = distance(in.frag, vec2<f32>(cx, cy));
  if (d > r) { return vec4<f32>(0.0); }
  let t = 1.0 - (d / r);
  let weight = t * t * (3.0 - 2.0 * t);
  let phase = U.glow.w;
  let pulse = 0.5 + 0.5 * sin(phase * 6.2831853);
  let a = U.fill.a * weight * U.stroke_radii_b.w * pulse;
  return vec4<f32>(U.fill.rgb, clamp(a, 0.0, 1.0));
}

// scanlines pipeline
@fragment
fn fs_scanlines(in: VsOut) -> @location(0) vec4<f32> {
  if (!rect_bounds(in.frag)) { return vec4<f32>(0.0); }
  let period = max(U.scan.y, 1.0);
  let on = step(0.5, fract((in.frag.y - U.rect.y) / period) * period - (period - 1.0));
  return vec4<f32>(0.0, 0.0, 0.0, U.scan.x * on);
}
"#;
