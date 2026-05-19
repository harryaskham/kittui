//! kittui-render-gpu
//!
//! Placeholder for the wgpu-backed renderer. Lives in the workspace so the
//! `kittui` facade can keep its `RendererKind::Gpu` variant compilable while
//! the shader/encoder work lands incrementally.
//!
//! The intent of this crate is to expose the same surface as `kittui-render-cpu`
//! (`render_still` / `render_animation`) so the facade picks one at runtime.
//! Until shaders land, attempting to render through the GPU backend returns
//! [`GpuRenderError::NotImplemented`] and the facade falls back to CPU.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use kittui_core::Scene;

/// Errors returned by the GPU renderer placeholder.
#[derive(Debug, thiserror::Error)]
pub enum GpuRenderError {
    /// The GPU backend has not been implemented yet.
    #[error("kittui-render-gpu is a scaffold; falling back to CPU")]
    NotImplemented,
}

/// Pretend to render a still scene. Always returns [`GpuRenderError::NotImplemented`]
/// today so callers know to fall back. The signature mirrors the CPU renderer
/// so swapping in a real implementation is mechanical.
pub fn render_still(_scene: &Scene) -> Result<Vec<u8>, GpuRenderError> {
    Err(GpuRenderError::NotImplemented)
}
