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

pub mod composition;
pub mod scene;

pub use composition::{Composer, Composition, CompositionEntry, DiffResult};

pub use kittui_core::terminal::Transport;
pub use kittui_core::{
    Animation, BlendMode, CellRect, CellSize, Corners, Direction, Fit, ImageRef, Layer, Node,
    Paint, PhaseCurve, Px, PxRect, Rgba, Scene, SceneId, Stop, Stroke, TerminalInfo,
};

use std::path::PathBuf;

use parking_lot::Mutex;

use kittui_cache::{Cache, CacheEntryMeta};
use kittui_kitty as kitty;
use kittui_render_cpu as cpu;
use kittui_render_gpu as gpu;

enum BackendState {
    Cpu,
    Gpu(gpu::GpuRenderer),
    GpuFailed,
}

/// Errors surfaced by the facade.
#[derive(Debug, thiserror::Error)]
pub enum KittuiError {
    /// CPU renderer error.
    #[error(transparent)]
    Render(#[from] cpu::RenderError),
    /// Cache error.
    #[error(transparent)]
    Cache(#[from] kittui_cache::CacheError),
    /// Invalid placement override.
    #[error("invalid placement: {0}")]
    InvalidPlacement(String),
    /// Terminal capabilities do not support high-level kittui placement.
    #[error("unsupported terminal: {0}")]
    UnsupportedTerminal(String),
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
    backend: Mutex<BackendState>,
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
        self.place_at(scene, scene.footprint)
    }

    /// Render/cache `scene` using its scene-local footprint, but place the
    /// resulting image at `placement_footprint` in the host terminal.
    ///
    /// This lets hosts move an already-rendered scene without mutating the
    /// scene itself. The placement footprint must have the same dimensions as
    /// the scene footprint; only `x`/`y` may differ.
    pub fn place_at(
        &self,
        scene: &Scene,
        placement_footprint: CellRect,
    ) -> Result<Placement, KittuiError> {
        self.ensure_terminal_support()?;
        if scene.footprint.cols != placement_footprint.cols
            || scene.footprint.rows != placement_footprint.rows
        {
            return Err(KittuiError::InvalidPlacement(format!(
                "placement footprint dimensions {}x{} must match scene footprint {}x{}",
                placement_footprint.cols,
                placement_footprint.rows,
                scene.footprint.cols,
                scene.footprint.rows
            )));
        }
        let id = scene.id();
        let image_id = id.kitty_image_id();
        let transport = self.terminal.transport;

        let mut upload = String::new();

        if scene.animation.is_none() {
            let png = if self.cache.contains_still(&id) {
                self.cache.get_still(&id)?
            } else {
                let frame = self.render_still_with_backend(scene)?;
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
            if !self.has_image_uploaded(image_id) {
                upload.push_str(&kitty::upload_still(image_id, &png, transport));
            }
        } else {
            let animation = scene.animation.as_ref().expect("checked above");
            if !self.cache.contains_animation(&id, animation.frames as u32) {
                let raster = self.render_animation_with_backend(scene)?;
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
            if !self.has_image_uploaded(image_id) {
                upload.push_str(&kitty::upload_animation(
                    image_id,
                    &frames,
                    &meta.frame_delays_ms,
                    meta.loops,
                    transport,
                ));
            }
        }

        let placement = {
            let mv = kitty::cursor_move(placement_footprint.x, placement_footprint.y, transport);
            let p = kitty::placement_command(image_id, placement_footprint, transport);
            format!("{mv}{p}")
        };
        let embed = kitty::placeholder_text(image_id, placement_footprint);
        self.mark_placed(image_id, placement_footprint);

        Ok(Placement {
            image_id,
            upload,
            placement,
            embed,
            footprint: placement_footprint,
        })
    }

    /// Delete an image from the terminal and forget it locally.
    pub fn unplace(&self, image_id: u32) -> String {
        self.placed.lock().remove(&image_id);
        kitty::delete(image_id, self.terminal.transport)
    }

    /// WM hot path: emit a `Placement` for a raw RGBA frame without ever
    /// constructing a `Scene` or running the renderer/cache. Uses kitty's
    /// `f=32` upload (raw RGBA bytes; no PNG encode) so the per-frame cost
    /// drops from PNG-encode time to a single base64 + write.
    ///
    /// `image_id` should be stable across frames for the same logical
    /// window (so kitty keeps the placement under the same id). The caller
    /// is responsible for emitting `unplace` when the window goes away.
    pub fn place_raw_frame(
        &self,
        image_id: u32,
        rgba: &[u8],
        width: u32,
        height: u32,
        footprint: CellRect,
    ) -> Placement {
        let transport = self.terminal.transport;
        let mut upload = String::new();
        if !self.has_already_placed(image_id, footprint) {
            upload.push_str(&kitty::upload_still_rgba(
                image_id, rgba, width, height, transport,
            ));
        }
        // Always re-upload pixels for raw frames — the WM compositor
        // changes pixels every tick. The placement + embed strings stay
        // cached behind has_already_placed for the no-resize case.
        if !upload.is_empty() {
            // For raw frames we want to overwrite the previous image: a
            // fresh upload on the same id triggers kitty to replace.
        } else {
            upload.push_str(&kitty::upload_still_rgba(
                image_id, rgba, width, height, transport,
            ));
        }
        let placement = {
            let mv = kitty::cursor_move(footprint.x, footprint.y, transport);
            let p = kitty::placement_command(image_id, footprint, transport);
            format!("{mv}{p}")
        };
        let embed = kitty::placeholder_text(image_id, footprint);
        self.mark_placed(image_id, footprint);
        Placement {
            image_id,
            upload,
            placement,
            embed,
            footprint,
        }
    }

    /// Renderer kind chosen at build time.
    pub fn renderer_kind(&self) -> RendererKind {
        self.renderer
    }

    /// Effective transport for this runtime (auto-detected or host-supplied).
    pub fn transport(&self) -> kittui_core::terminal::Transport {
        self.terminal.transport
    }

    /// Render a batch of scenes through the same runtime/cache, returning one
    /// `Placement` per scene in input order. This is the documented batch
    /// entrypoint for hosts that need to place many scenes in one tick — it
    /// reuses the cache, the upload registry, and a single transport hint
    /// without forcing callers to call `place` in a loop themselves.
    pub fn place_many(&self, scenes: &[Scene]) -> Result<Vec<Placement>, KittuiError> {
        let mut out = Vec::with_capacity(scenes.len());
        for scene in scenes {
            out.push(self.place(scene)?);
        }
        Ok(out)
    }

    /// Convenience that concatenates all upload bytes, placement escapes, and
    /// embed placeholders for a batch of scenes into a single `BatchPlacement`
    /// the host can write to its terminal stream in three contiguous writes.
    pub fn place_batch(&self, scenes: &[Scene]) -> Result<BatchPlacement, KittuiError> {
        let mut batch = BatchPlacement::default();
        for scene in scenes {
            let p = self.place(scene)?;
            batch.upload.push_str(&p.upload);
            batch.placement.push_str(&p.placement);
            batch.embed.push_str(&p.embed);
            batch.image_ids.push(p.image_id);
            batch.footprints.push(p.footprint);
        }
        Ok(batch)
    }

    fn ensure_terminal_support(&self) -> Result<(), KittuiError> {
        if !self.terminal.supports_kitty {
            return Err(KittuiError::UnsupportedTerminal(
                "kitty graphics protocol is not supported".to_string(),
            ));
        }
        if !self.terminal.supports_unicode_placeholders {
            return Err(KittuiError::UnsupportedTerminal(
                "kitty unicode placeholders are not supported".to_string(),
            ));
        }
        Ok(())
    }

    fn record_gpu_probe(&self, status: &str, adapter: Option<String>) {
        let record = kittui_cache::ProbeRecord {
            kittui_version: env!("CARGO_PKG_VERSION").to_string(),
            gpu_status: status.to_string(),
            gpu_adapter: adapter,
            gpu_parity_ssim: None,
            checked_at: now_rfc3339(),
        };
        let _ = self.cache.write_probe(&record);
    }

    fn render_still_with_backend(&self, scene: &Scene) -> Result<cpu::RasterFrame, KittuiError> {
        let try_gpu = matches!(self.renderer, RendererKind::Gpu | RendererKind::Auto);
        if try_gpu {
            let mut backend = self.backend.lock();
            // Initialize lazily on first attempt.
            if matches!(*backend, BackendState::Cpu) {
                match gpu::GpuRenderer::new() {
                    Ok(r) => {
                        let adapter = format!("{:?}", r.adapter_info().name);
                        *backend = BackendState::Gpu(r);
                        self.record_gpu_probe("ok", Some(adapter));
                    }
                    Err(_) => {
                        *backend = BackendState::GpuFailed;
                        self.record_gpu_probe("failed", None);
                    }
                }
            }
            if let BackendState::Gpu(renderer) = &mut *backend {
                match renderer.render_still(scene) {
                    Ok(frame) => return Ok(frame),
                    Err(_) if matches!(self.renderer, RendererKind::Auto) => {
                        *backend = BackendState::GpuFailed;
                        self.record_gpu_probe("failed", None);
                    }
                    Err(e) => {
                        return Err(KittuiError::Render(cpu::RenderError::UnsupportedImage(
                            format!("gpu error: {e}"),
                        )));
                    }
                }
            }
        }
        Ok(cpu::render_still(scene)?)
    }

    fn render_animation_with_backend(
        &self,
        scene: &Scene,
    ) -> Result<cpu::RasterAnimation, KittuiError> {
        let try_gpu = matches!(self.renderer, RendererKind::Gpu | RendererKind::Auto);
        if try_gpu {
            let mut backend = self.backend.lock();
            if matches!(*backend, BackendState::Cpu) {
                match gpu::GpuRenderer::new() {
                    Ok(r) => {
                        let adapter = format!("{:?}", r.adapter_info().name);
                        *backend = BackendState::Gpu(r);
                        self.record_gpu_probe("ok", Some(adapter));
                    }
                    Err(_) => {
                        *backend = BackendState::GpuFailed;
                        self.record_gpu_probe("failed", None);
                    }
                }
            }
            if let BackendState::Gpu(renderer) = &mut *backend {
                match renderer.render_animation(scene) {
                    Ok(anim) => return Ok(anim),
                    Err(_) if matches!(self.renderer, RendererKind::Auto) => {
                        *backend = BackendState::GpuFailed;
                        self.record_gpu_probe("failed", None);
                    }
                    Err(e) => {
                        return Err(KittuiError::Render(cpu::RenderError::UnsupportedImage(
                            format!("gpu error: {e}"),
                        )));
                    }
                }
            }
        }
        Ok(cpu::render_animation(scene)?)
    }

    fn has_already_placed(&self, image_id: u32, footprint: CellRect) -> bool {
        matches!(self.placed.lock().get(&image_id), Some(prev) if *prev == footprint)
    }

    fn has_image_uploaded(&self, image_id: u32) -> bool {
        self.placed.lock().contains_key(&image_id)
    }

    fn mark_placed(&self, image_id: u32, footprint: CellRect) {
        self.placed.lock().insert(image_id, footprint);
    }
}

fn now_rfc3339() -> String {
    // Avoid pulling chrono just for one timestamp; format seconds-since-epoch
    // as a stable, sortable ISO-ish string. Good enough for probe.json freshness.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch:{secs}")
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
        // Consult the persisted probe record: if a previous run determined the
        // GPU is unusable on this host, skip the lazy GPU init so Auto mode
        // does not eat the adapter-request cost on every startup.
        let prior_probe = cache.read_probe().ok().flatten();
        let initial_backend = match self.renderer {
            RendererKind::Cpu => BackendState::Cpu,
            RendererKind::Gpu => BackendState::Cpu, // lazy init; explicit Gpu still tries.
            RendererKind::Auto => {
                if matches!(prior_probe.as_ref(), Some(p) if p.gpu_status == "failed") {
                    BackendState::GpuFailed
                } else {
                    BackendState::Cpu // lazy init on first frame.
                }
            }
        };
        Ok(Runtime {
            terminal: self.terminal.unwrap_or_else(TerminalInfo::detect),
            cache,
            renderer: self.renderer,
            backend: Mutex::new(initial_backend),
            placed: Mutex::new(Default::default()),
        })
    }
}

/// Result of [`Runtime::place_batch`]. Concatenates the upload, placement,
/// and embed bytes for a batch of scenes so hosts can write them in three
/// contiguous writes.
#[derive(Default, Clone, Debug)]
pub struct BatchPlacement {
    /// Concatenated upload escapes for every scene in the batch.
    pub upload: String,
    /// Concatenated placement escapes for every scene in the batch.
    pub placement: String,
    /// Concatenated unicode-placeholder grids for every scene.
    pub embed: String,
    /// kitty image id assigned to each scene, in input order.
    pub image_ids: Vec<u32>,
    /// Cell footprint for each scene, in input order.
    pub footprints: Vec<CellRect>,
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
        let mut out =
            String::with_capacity(self.upload.len() + self.placement.len() + self.embed.len());
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
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("kittui-runtime-{pid}-{nanos}-{seq}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn with_env<F: FnOnce()>(pairs: &[(&str, Option<&str>)], f: F) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let saved = pairs
            .iter()
            .map(|(key, _)| (key.to_string(), std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for (key, value) in pairs {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    #[test]
    fn runtime_builder_detects_terminal_by_default() {
        with_env(
            &[
                ("TMUX", Some("/tmp/tmux,123,0")),
                ("WT_SESSION", None),
                ("TERM_PROGRAM", None),
                ("KITTY_WINDOW_ID", None),
                ("KITTY_PUBLIC_KEY", None),
                ("TERM", Some("xterm-256color")),
            ],
            || {
                let runtime = Runtime::builder()
                    .cache_dir(tempdir())
                    .renderer(RendererKind::Cpu)
                    .build()
                    .unwrap();
                assert_eq!(runtime.transport(), Transport::TmuxPassthrough);
            },
        );
    }

    #[test]
    fn runtime_builder_terminal_override_wins_over_detection() {
        with_env(&[("TMUX", Some("/tmp/tmux,123,0"))], || {
            let runtime = Runtime::builder()
                .cache_dir(tempdir())
                .renderer(RendererKind::Cpu)
                .terminal(TerminalInfo::override_with(
                    None,
                    None,
                    CellSize::default(),
                    true,
                    true,
                    Transport::Direct,
                ))
                .build()
                .unwrap();
            assert_eq!(runtime.transport(), Transport::Direct);
        });
    }

    fn runtime_with_terminal(terminal: TerminalInfo) -> Runtime {
        Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(terminal)
            .build()
            .unwrap()
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

    #[test]
    fn unsupported_terminal_without_kitty_rejects_high_level_place() {
        let runtime = runtime_with_terminal(TerminalInfo::override_with(
            None,
            None,
            CellSize::default(),
            false,
            true,
            Transport::Direct,
        ));
        let scene = builders::simple_solid_box(4, 2, "#00d8ff");
        let err = match runtime.place(&scene) {
            Ok(_) => panic!("unsupported terminal unexpectedly placed scene"),
            Err(err) => err,
        };
        assert!(matches!(err, KittuiError::UnsupportedTerminal(_)));
    }

    #[test]
    fn unsupported_terminal_without_placeholders_rejects_high_level_place() {
        let runtime = runtime_with_terminal(TerminalInfo::override_with(
            None,
            None,
            CellSize::default(),
            true,
            false,
            Transport::Direct,
        ));
        let scene = builders::simple_solid_box(4, 2, "#00d8ff");
        let err = match runtime.place(&scene) {
            Ok(_) => panic!("unsupported placeholder terminal unexpectedly placed scene"),
            Err(err) => err,
        };
        assert!(matches!(err, KittuiError::UnsupportedTerminal(_)));
    }

    #[test]
    fn supported_terminal_override_places_scene() {
        let runtime = runtime_with_terminal(TerminalInfo::override_with(
            None,
            None,
            CellSize::default(),
            true,
            true,
            Transport::Direct,
        ));
        let scene = builders::simple_solid_box(4, 2, "#00d8ff");
        let placement = runtime.place(&scene).unwrap();
        assert_eq!(placement.footprint, scene.footprint);
        assert!(!placement.placement.is_empty());
    }

    #[test]
    fn place_at_moves_scene_without_changing_image_id() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let scene = builders::simple_solid_box(4, 2, "#00d8ff");
        let first = runtime.place(&scene).unwrap();
        let moved = runtime
            .place_at(&scene, CellRect::new(10, 5, 4, 2))
            .unwrap();
        assert_eq!(moved.image_id, first.image_id);
        assert_eq!(moved.footprint, CellRect::new(10, 5, 4, 2));
        assert!(
            moved.upload.is_empty(),
            "move should not re-upload cached image"
        );
        assert!(
            moved.placement.contains("\x1b[6;11H"),
            "{:?}",
            moved.placement
        );
    }

    #[test]
    fn place_at_rejects_dimension_mismatches() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let scene = builders::simple_solid_box(4, 2, "#00d8ff");
        let err = match runtime.place_at(&scene, CellRect::new(0, 0, 5, 2)) {
            Ok(_) => panic!("dimension mismatch unexpectedly succeeded"),
            Err(err) => err,
        };
        assert!(matches!(err, KittuiError::InvalidPlacement(_)));
    }

    #[test]
    fn place_batch_returns_one_placement_per_scene_and_concatenates_bytes() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let scenes = vec![
            builders::simple_solid_box(2, 1, "#ff0000"),
            builders::simple_solid_box(3, 1, "#00ff00"),
            builders::simple_solid_box(4, 1, "#0000ff"),
        ];
        let batch = runtime.place_batch(&scenes).unwrap();
        assert_eq!(batch.image_ids.len(), 3);
        assert_eq!(batch.footprints.len(), 3);
        // Three distinct image ids ⇒ three uploads.
        assert!(!batch.upload.is_empty());
        assert!(!batch.placement.is_empty());
        assert!(!batch.embed.is_empty());
    }

    #[test]
    fn place_many_matches_individual_place_call_for_same_scene() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let scenes = vec![
            builders::simple_solid_box(2, 1, "#ff00ff"),
            builders::simple_solid_box(3, 1, "#00ffff"),
        ];
        let many = runtime.place_many(&scenes).unwrap();
        assert_eq!(many.len(), 2);
        // Second call should hit the cache for both (no new upload).
        let again = runtime.place_many(&scenes).unwrap();
        for p in &again {
            assert!(
                p.upload.is_empty(),
                "cached scene re-uploaded: {:?}",
                p.image_id
            );
        }
    }

    #[test]
    fn auto_mode_honours_persisted_failed_probe() {
        let dir = tempdir();
        // Pre-seed probe.json with a `failed` record.
        let cache = kittui_cache::Cache::open(&dir).unwrap();
        cache
            .write_probe(&kittui_cache::ProbeRecord {
                kittui_version: env!("CARGO_PKG_VERSION").to_string(),
                gpu_status: "failed".to_string(),
                gpu_adapter: None,
                gpu_parity_ssim: None,
                checked_at: super::now_rfc3339(),
            })
            .unwrap();
        let runtime = Runtime::builder()
            .cache_dir(&dir)
            .renderer(RendererKind::Auto)
            .build()
            .unwrap();
        // Render a scene; it must succeed via the CPU fallback even when no
        // adapter is available (the probe pre-decided GPU is unusable).
        let scene = builders::simple_solid_box(2, 1, "#abcdef");
        let p = runtime.place(&scene).unwrap();
        assert!(!p.upload.is_empty());
    }
}
