//! kittui-render-gpu
//!
//! wgpu-backed renderer that produces the same RGBA8 PNG output as the CPU
//! oracle. Designed for the project's hard-throughput regime: one render
//! pass per scene, one pipeline per shape family, instanced draws batched
//! across all nodes in the same family.
//!
//! The GPU renderer is selected by `Runtime::builder().renderer(Gpu)` or
//! `Auto`. The facade calls into this crate when selected and falls back
//! to the CPU renderer on any failure (adapter init, shader compile,
//! parity gate, runtime).
//!
//! The public API mirrors `kittui-render-cpu`:
//!
//! ```ignore
//! let mut renderer = GpuRenderer::new()?;
//! let frame = renderer.render_still(&scene)?;
//! let anim = renderer.render_animation(&scene)?;
//! ```
//!
//! Internals are split into three modules: `device` owns the wgpu adapter
//! and queue; `pipelines` owns the three shape-family pipelines; `encode`
//! drives a single render pass per frame.

#![warn(missing_docs, rust_2018_idioms)]

mod device;
mod encode;
pub mod pipelines;

pub use device::{GpuDevice, GpuDeviceOptions, GpuInitError, GpuPowerPreference};
pub use pipelines::ShaderError;

use kittui_core::Scene;
use kittui_render_cpu::{encode_png, Pixmap, RasterAnimation, RasterFrame};

/// Errors produced by the GPU renderer.
#[derive(Debug, thiserror::Error)]
pub enum GpuRenderError {
    /// Underlying wgpu initialization failed.
    #[error(transparent)]
    Init(#[from] GpuInitError),
    /// Animation curve does not close the loop.
    #[error("animation phase curve does not close the loop")]
    AnimationDoesNotLoop,
    /// Buffer mapping failed during readback.
    #[error("gpu readback failed: {0}")]
    Readback(String),
}

/// Options for constructing a long-lived [`GpuRenderer`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct GpuRendererOptions {
    /// Adapter/device selection options.
    pub device: GpuDeviceOptions,
}

/// Long-lived GPU renderer. Owns the wgpu adapter, queue, pipelines, and
/// reusable offscreen/readback resources for deterministic repeated renders.
pub struct GpuRenderer {
    device: GpuDevice,
    pipelines: pipelines::Pipelines,
    scratch: encode::RenderScratch,
}

impl GpuRenderer {
    /// Construct a new GPU renderer. May block briefly on adapter init.
    pub fn new() -> Result<Self, GpuRenderError> {
        Self::with_options(GpuRendererOptions::default())
    }

    /// Construct a renderer with explicit adapter/device options.
    pub fn with_options(options: GpuRendererOptions) -> Result<Self, GpuRenderError> {
        let device = GpuDevice::new_with_options(options.device)?;
        let pipelines = pipelines::Pipelines::new(&device);
        let scratch = encode::RenderScratch::new();
        Ok(Self {
            device,
            pipelines,
            scratch,
        })
    }

    /// Return adapter diagnostics for logs, probe caches, and support reports.
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.device.adapter_info
    }

    /// GPU features that are accepted by the scene model but intentionally
    /// rendered by CPU fallback until their dedicated pipelines land.
    pub fn unsupported_features(&self) -> &'static [&'static str] {
        &[
            "image atlas nodes",
            "custom shader nodes",
            "true mask/clip intermediate passes",
        ]
    }

    /// Render a still scene into an RGBA PNG. Output is byte-identical
    /// across runs for the same scene id (post-PNG-encode).
    pub fn render_still(&mut self, scene: &Scene) -> Result<RasterFrame, GpuRenderError> {
        let mut pixmap = Pixmap::new(scene.pixel_width(), scene.pixel_height());
        encode::render_scene(
            &self.device,
            &self.pipelines,
            &mut self.scratch,
            scene,
            0.0,
            &mut pixmap,
        )?;
        let png = encode_png(&pixmap);
        Ok(RasterFrame {
            png,
            width_px: pixmap.width(),
            height_px: pixmap.height(),
        })
    }

    /// Render every frame of an animated scene. Each frame uses the same
    /// GPU storage; only the phase uniform changes between frames.
    pub fn render_animation(&mut self, scene: &Scene) -> Result<RasterAnimation, GpuRenderError> {
        let Some(animation) = scene.animation.clone() else {
            // Treat as a single-frame animation for symmetry.
            let frame = self.render_still(scene)?;
            return Ok(RasterAnimation {
                frames: vec![frame.png],
                frame_delays_ms: vec![0],
                width_px: frame.width_px,
                height_px: frame.height_px,
                loops: 0,
            });
        };
        if !animation.curve.closes_loop() {
            return Err(GpuRenderError::AnimationDoesNotLoop);
        }
        let mut frames = Vec::with_capacity(animation.frames as usize);
        let mut pixmap = Pixmap::new(scene.pixel_width(), scene.pixel_height());
        for phase in animation.phases() {
            pixmap.clear();
            encode::render_scene(
                &self.device,
                &self.pipelines,
                &mut self.scratch,
                scene,
                phase,
                &mut pixmap,
            )?;
            frames.push(encode_png(&pixmap));
        }
        let delays: Vec<u32> = (0..animation.frames)
            .map(|i| animation.delay_ms(i))
            .collect();
        Ok(RasterAnimation {
            frames,
            frame_delays_ms: delays,
            width_px: pixmap.width(),
            height_px: pixmap.height(),
            loops: animation.loops,
        })
    }
}

/// Stateless convenience: build a renderer and render a still scene. For
/// long-lived hosts, prefer constructing one `GpuRenderer` and reusing it.
pub fn render_still(scene: &Scene) -> Result<RasterFrame, GpuRenderError> {
    GpuRenderer::new()?.render_still(scene)
}

/// Stateless convenience for animations.
pub fn render_animation(scene: &Scene) -> Result<RasterAnimation, GpuRenderError> {
    GpuRenderer::new()?.render_animation(scene)
}

#[cfg(test)]
mod tests {
    // GPU smoke coverage lives in `tests/parity.rs`, which gates on adapter
    // availability and asserts CPU↔GPU output agreement. Running an
    // additional in-crate test that also opens a wgpu device in parallel
    // deadlocks on Metal under cargo's default test runner, so we keep the
    // GPU exercise to the dedicated parity test process.
}
