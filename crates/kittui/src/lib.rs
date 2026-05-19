//! kittui — public Rust facade.
//!
//! Ties together the scene model (`kittui-core`), the renderers
//! (`kittui-render-cpu`, `kittui-render-gpu`), the kitty graphics protocol
//! encoder (`kittui-kitty`), and the content-addressed cache
//! (`kittui-cache`). Library users build a `Scene` (using the helpers in
//! `kittui::scene`) and call `Runtime::place` to render, cache, upload and
//! receive embeddable text in one step.
//!
//! The crate intentionally exposes only general primitives. Affordance-level
//! "draw a panel / chip / divider" helpers live in the CLI and the showcase
//! example, where they belong as consumers of the library rather than part
//! of its API surface.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

pub mod scene;

pub use kittui_core::{
    Animation, BlendMode, CellRect, CellSize, Corners, Direction, Fit, ImageRef, Layer, Node,
    Paint, PhaseCurve, Px, PxRect, Rgba, Scene, SceneId, Stop, Stroke, TerminalInfo,
};
pub use kittui_core::terminal::Transport;

use std::path::PathBuf;

use parking_lot::Mutex;

use kittui_cache::{Cache, CacheEntryMeta};
use kittui_kitty as kitty;
use kittui_render_cpu as cpu;

/// Errors surfaced by the facade.
#[derive(Debug, thiserror::Error)]
pub enum KittuiError {
    /// CPU renderer error.
    #[error(transparent)]
    Render(#[from] cpu::RenderError),
    /// Cache error.
    #[error(transparent)]
    Cache(#[from] kittui_cache::CacheError),
}

/// Renderer selection.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RendererKind {
    /// CPU renderer (always available; reference oracle).
    Cpu,
    /// GPU renderer (wgpu). Falls back to CPU until the GPU backend lands.
    Gpu,
    /// Choose GPU if available, otherwise CPU.
    Auto,
}

impl Default for RendererKind {
    fn default() -> Self {
        Self::Auto
    }
}

/// Long-lived state shared across `Runtime::place` calls.
pub struct Runtime {
    terminal: TerminalInfo,
    cache: Cache,
    renderer: RendererKind,
    // Currently placed image ids → footprint, so re-place calls don't emit
    // redundant escapes. Wrapped for cheap interior mutability.
    placed: Mutex<std::collections::HashMap<u32, CellRect>>,
}

impl Runtime {
    /// Build a runtime with explicit configuration.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }

    /// Render, cache and place a scene. Returns a `Placement` containing the
    /// upload bytes (empty if already cached + uploaded), the placement
    /// escape, and the embeddable text grid.
    pub fn place(&self, scene: &Scene) -> Result<Placement, KittuiError> {
        let id = scene.id();
        let image_id = id.kitty_image_id();
        let transport = self.terminal.transport;

        let mut upload = String::new();

        if scene.animation.is_none() {
            let png = if self.cache.contains_still(&id) {
                self.cache.get_still(&id)?
            } else {
                let frame = match self.renderer {
                    RendererKind::Cpu | RendererKind::Auto => cpu::render_still(scene)?,
                    RendererKind::Gpu => match kittui_render_gpu::render_still(scene) {
                        Ok(_) => cpu::render_still(scene)?, // not implemented yet
                        Err(_) => cpu::render_still(scene)?,
                    },
                };
                self.cache.put_still(
                    &id,
                    &frame.png,
                    &CacheEntryMeta {
                        footprint: scene.footprint,
                        width_px: frame.width_px,
                        height_px: frame.height_px,
                        frames: 1,
                        frame_delays_ms: vec![],
                        kitty_image_id: image_id,
                        loops: 0,
                    },
                )?;
                frame.png
            };
            if !self.has_already_placed(image_id, scene.footprint) {
                upload.push_str(&kitty::upload_still(image_id, &png, transport));
            }
        } else {
            let animation = scene.animation.as_ref().expect("checked above");
            if !self.cache.contains_animation(&id, animation.frames as u32) {
                let raster = cpu::render_animation(scene)?;
                self.cache.put_animation(
                    &id,
                    &raster.frames,
                    &CacheEntryMeta {
                        footprint: scene.footprint,
                        width_px: raster.width_px,
                        height_px: raster.height_px,
                        frames: raster.frames.len() as u32,
                        frame_delays_ms: raster.frame_delays_ms.clone(),
                        kitty_image_id: image_id,
                        loops: raster.loops,
                    },
                )?;
            }
            let meta = self.cache.get_meta(&id)?;
            let frames = self.cache.get_animation(&id, meta.frames)?;
            if !self.has_already_placed(image_id, scene.footprint) {
                upload.push_str(&kitty::upload_animation(
                    image_id,
                    &frames,
                    &meta.frame_delays_ms,
                    meta.loops,
                    transport,
                ));
            }
        }

        let placement = kitty::placement_command(image_id, scene.footprint, transport);
        let embed = kitty::placeholder_text(image_id, scene.footprint);
        self.mark_placed(image_id, scene.footprint);

        Ok(Placement {
            image_id,
            upload,
            placement,
            embed,
            footprint: scene.footprint,
        })
    }

    /// Delete an image from the terminal and forget it locally.
    pub fn unplace(&self, image_id: u32) -> String {
        self.placed.lock().remove(&image_id);
        kitty::delete(image_id, self.terminal.transport)
    }

    fn has_already_placed(&self, image_id: u32, footprint: CellRect) -> bool {
        matches!(self.placed.lock().get(&image_id), Some(prev) if *prev == footprint)
    }

    fn mark_placed(&self, image_id: u32, footprint: CellRect) {
        self.placed.lock().insert(image_id, footprint);
    }
}

/// Builder for [`Runtime`].
#[derive(Default)]
pub struct RuntimeBuilder {
    terminal: Option<TerminalInfo>,
    cache_dir: Option<PathBuf>,
    renderer: RendererKind,
}

impl RuntimeBuilder {
    /// Provide terminal capabilities (overrides probing).
    pub fn terminal(mut self, terminal: TerminalInfo) -> Self {
        self.terminal = Some(terminal);
        self
    }

    /// Override the cache directory. Defaults to platform-specific cache
    /// home (see [`kittui_cache::default_cache_dir`]).
    pub fn cache_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(path.into());
        self
    }

    /// Choose the renderer backend.
    pub fn renderer(mut self, kind: RendererKind) -> Self {
        self.renderer = kind;
        self
    }

    /// Build the runtime.
    pub fn build(self) -> Result<Runtime, KittuiError> {
        let cache = Cache::open(
            self.cache_dir
                .unwrap_or_else(kittui_cache::default_cache_dir),
        )?;
        Ok(Runtime {
            terminal: self.terminal.unwrap_or_default(),
            cache,
            renderer: self.renderer,
            placed: Mutex::new(Default::default()),
        })
    }
}

/// Result of [`Runtime::place`].
pub struct Placement {
    /// kitty graphics image id assigned to the scene.
    pub image_id: u32,
    /// Upload escape sequence(s). Empty on cache + placement hit.
    pub upload: String,
    /// Placement escape sequence positioning the image under the cursor.
    pub placement: String,
    /// Unicode-placeholder grid to write into the terminal cells.
    pub embed: String,
    /// Cell footprint the image occupies.
    pub footprint: CellRect,
}

impl Placement {
    /// Convenience: the full bytes to write to the terminal in one call.
    ///
    /// Hosts that want finer control (e.g. write upload at the top of a
    /// frame, then placement+embed at the widget origin) should use the
    /// individual fields.
    pub fn to_bytes(&self) -> String {
        let mut out = String::with_capacity(self.upload.len() + self.placement.len() + self.embed.len());
        out.push_str(&self.upload);
        out.push_str(&self.placement);
        out.push_str(&self.embed);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::builders;

    fn tempdir() -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("kittui-runtime-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn place_caches_then_skips_reupload() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let scene = builders::simple_solid_box(4, 2, "#00d8ff");
        let first = runtime.place(&scene).unwrap();
        assert!(!first.upload.is_empty());
        let second = runtime.place(&scene).unwrap();
        // Same footprint + same image id ⇒ no re-upload.
        assert!(second.upload.is_empty());
    }
}
