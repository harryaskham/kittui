//! Encode a scene into one render pass + readback.

use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use kittui_core::color::Rgba;
use kittui_core::node::{BlendMode, Direction, Layer, Node};
use kittui_core::paint::{LinearGradient, Paint, RadialGradient};
use kittui_core::Scene;
use kittui_render_cpu::Pixmap;

use crate::device::GpuDevice;
use crate::pipelines::{Pipelines, Uniforms};
use crate::GpuRenderError;

const ALIGN: u32 = 256; // wgpu copy_texture_to_buffer alignment requirement.

/// Reusable offscreen render/readback resources for a single renderer.
pub(crate) struct RenderScratch {
    width: u32,
    height: u32,
    bytes_per_row: u32,
    target: Option<wgpu::Texture>,
    readback: Option<wgpu::Buffer>,
}

impl RenderScratch {
    /// Create an empty scratch bundle. Resources are allocated lazily on the
    /// first render and then reused until the scene dimensions change.
    pub(crate) fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            bytes_per_row: 0,
            target: None,
            readback: None,
        }
    }

    fn ensure(&mut self, device: &GpuDevice, width: u32, height: u32) {
        let bytes_per_row = align_up(width * 4, ALIGN);
        if self.target.is_some()
            && self.readback.is_some()
            && self.width == width
            && self.height == height
            && self.bytes_per_row == bytes_per_row
        {
            return;
        }

        self.width = width;
        self.height = height;
        self.bytes_per_row = bytes_per_row;
        self.target = Some(device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kittui-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }));
        self.readback = Some(device.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kittui-readback"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
    }
}

/// Render `scene` at the given animation phase into `pixmap`. Each top-level
/// layer becomes one draw call into a shared offscreen color target; after
/// all draws complete we copy the texture back to a CPU-mapped buffer.
pub fn render_scene(
    device: &GpuDevice,
    pipelines: &Pipelines,
    scratch: &mut RenderScratch,
    scene: &Scene,
    phase: f32,
    pixmap: &mut Pixmap,
) -> Result<(), GpuRenderError> {
    let width = pixmap.width();
    let height = pixmap.height();
    if width == 0 || height == 0 {
        return Ok(());
    }

    scratch.ensure(device, width, height);
    let texture = scratch.target.as_ref().expect("scratch target initialized");
    let readback = scratch
        .readback
        .as_ref()
        .expect("scratch readback initialized");
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kittui-encoder"),
        });

    // First clear pass.
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("kittui-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,

                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    // One draw call per node.
    for layer in &scene.layers {
        draw_node(
            device,
            pipelines,
            &view,
            &mut encoder,
            &layer.root,
            width,
            height,
            phase,
            1.0,
            BlendMode::Normal,
        );
    }

    // Copy texture → buffer for readback.
    let bytes_per_row = scratch.bytes_per_row;
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    device.queue.submit(std::iter::once(encoder.finish()));

    // Map and copy out into the pixmap.
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    device.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|e| GpuRenderError::Readback(e.to_string()))?
        .map_err(|e| GpuRenderError::Readback(e.to_string()))?;
    {
        let data = slice.get_mapped_range();
        let dst = pixmap.data_mut();
        let row_bytes = (width * 4) as usize;
        for y in 0..height as usize {
            let src_off = y * bytes_per_row as usize;
            let dst_off = y * row_bytes;
            dst[dst_off..dst_off + row_bytes].copy_from_slice(&data[src_off..src_off + row_bytes]);
        }
    }
    readback.unmap();

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    device: &GpuDevice,
    pipelines: &Pipelines,
    view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
    node: &Node,
    width: u32,
    height: u32,
    phase: f32,
    opacity: f32,
    _blend: BlendMode,
) {
    match node {
        Node::Rect {
            rect,
            fill,
            stroke,
            corners,
        } => {
            let mut u = Uniforms::zeroed();
            u.viewport = [width as f32, height as f32];
            u.rect = [rect.origin.0, rect.origin.1, rect.width, rect.height];
            let (rgba, _) = paint_to_solid(fill);
            u.fill = scale_alpha(rgba, opacity);
            if let Some(s) = stroke {
                let (srgba, _) = paint_to_solid(&s.paint);
                u.stroke = scale_alpha(srgba, opacity);
                u.stroke_radii_a[0] = s.width_px;
            }
            u.stroke_radii_a[1] = corners.tl;
            u.stroke_radii_a[2] = corners.tr;
            u.stroke_radii_a[3] = corners.bl;
            u.stroke_radii_b[0] = corners.br;
            u.stroke_radii_b[1] = 0.0; // kind=rect
            submit_draw(device, pipelines, view, encoder, &u, NodeKind::Rect);
        }
        Node::Gradient {
            rect,
            stops,
            direction,
        } => {
            let mut u = Uniforms::zeroed();
            u.viewport = [width as f32, height as f32];
            u.rect = [rect.origin.0, rect.origin.1, rect.width, rect.height];
            let (start, end) = endpoint_stops(stops);
            u.grad_start = scale_alpha(start, opacity);
            u.grad_end = scale_alpha(end, opacity);
            u.stroke_radii_b[1] = 1.0; // kind=gradient
            u.stroke_radii_b[2] = match direction {
                Direction::Horizontal => 0.0,
                Direction::Vertical => 1.0,
                Direction::Diagonal => 2.0,
            };
            submit_draw(device, pipelines, view, encoder, &u, NodeKind::Gradient);
        }
        Node::Glow {
            rect,
            center_x_frac,
            center_y_frac,
            radius_frac,
            color,
            intensity,
        } => {
            let mut u = Uniforms::zeroed();
            u.viewport = [width as f32, height as f32];
            u.rect = [rect.origin.0, rect.origin.1, rect.width, rect.height];
            u.fill = scale_alpha(rgba_to_vec4(*color), opacity);
            u.glow = [*center_x_frac, *center_y_frac, *radius_frac, phase];
            u.stroke_radii_b[1] = 2.0; // kind=glow
            u.stroke_radii_b[3] = (*intensity).clamp(0.0, 1.0);
            submit_draw(device, pipelines, view, encoder, &u, NodeKind::Glow);
        }
        Node::Scanlines {
            rect,
            alpha,
            period_px,
        } => {
            let mut u = Uniforms::zeroed();
            u.viewport = [width as f32, height as f32];
            u.rect = [rect.origin.0, rect.origin.1, rect.width, rect.height];
            u.scan = [
                (*alpha as f32 / 255.0) * opacity,
                *period_px as f32,
                0.0,
                0.0,
            ];
            u.stroke_radii_b[1] = 3.0; // kind=scanlines
            submit_draw(device, pipelines, view, encoder, &u, NodeKind::Scanlines);
        }
        Node::Image { .. } => {
            // Image support arrives with the atlas pipeline.
        }
        Node::Shader { .. } => {
            // User-shader nodes need per-scene pipeline compilation +
            // caching. v0.6 GPU backend accepts the node in the type
            // system but draws nothing; the next revision wires the
            // dynamic pipeline cache and lights this up.
        }
        Node::Group {
            opacity: o,
            children,
        } => {
            let combined = (opacity * o.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            for child in children {
                draw_node(
                    device,
                    pipelines,
                    view,
                    encoder,
                    child,
                    width,
                    height,
                    phase,
                    combined,
                    BlendMode::Normal,
                );
            }
        }
        Node::Composite { mode, children } => {
            for child in children {
                draw_node(
                    device, pipelines, view, encoder, child, width, height, phase, opacity, *mode,
                );
            }
        }
        Node::Mask { child, .. } => {
            draw_node(
                device,
                pipelines,
                view,
                encoder,
                child,
                width,
                height,
                phase,
                opacity,
                BlendMode::Normal,
            );
        }
        Node::Clip { child, .. } => {
            draw_node(
                device,
                pipelines,
                view,
                encoder,
                child,
                width,
                height,
                phase,
                opacity,
                BlendMode::Normal,
            );
        }
    }
}

#[derive(Copy, Clone)]
enum NodeKind {
    Rect,
    Gradient,
    Glow,
    Scanlines,
}

#[allow(dead_code)]
fn submit_draw(
    device: &GpuDevice,
    pipelines: &Pipelines,
    view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
    u: &Uniforms,
    kind: NodeKind,
) {
    let buffer = device
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kittui-uniforms"),
            contents: bytemuck::bytes_of(u),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    let bind_group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("kittui-bg"),
        layout: &pipelines.bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64),
            }),
        }],
    });
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("kittui-draw"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,

            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(match kind {
        NodeKind::Rect => &pipelines.rect_pipeline,
        NodeKind::Gradient => &pipelines.gradient_pipeline,
        NodeKind::Glow => &pipelines.glow_pipeline,
        NodeKind::Scanlines => &pipelines.scanlines_pipeline,
    });
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..4, 0..1);
}

fn paint_to_solid(paint: &Paint) -> ([f32; 4], bool) {
    match paint {
        Paint::Solid { color } => (rgba_to_vec4(*color), true),
        Paint::Linear(LinearGradient { stops, .. }) => {
            let (start, _) = endpoint_stops(stops);
            (start, false)
        }
        Paint::Radial(RadialGradient { stops, .. }) => {
            let (start, _) = endpoint_stops(stops);
            (start, false)
        }
    }
}

fn endpoint_stops(stops: &[kittui_core::node::Stop]) -> ([f32; 4], [f32; 4]) {
    let first = stops.first().map(|s| s.color).unwrap_or_default();
    let last = stops.last().map(|s| s.color).unwrap_or(first);
    (rgba_to_vec4(first), rgba_to_vec4(last))
}

fn rgba_to_vec4(c: Rgba) -> [f32; 4] {
    [
        c.0 as f32 / 255.0,
        c.1 as f32 / 255.0,
        c.2 as f32 / 255.0,
        c.3 as f32 / 255.0,
    ]
}

fn scale_alpha(mut c: [f32; 4], scale: f32) -> [f32; 4] {
    c[3] *= scale.clamp(0.0, 1.0);
    c
}

fn align_up(value: u32, align: u32) -> u32 {
    (value + align - 1) & !(align - 1)
}

#[allow(dead_code)]
fn _layer_unused(_: Layer) {}
