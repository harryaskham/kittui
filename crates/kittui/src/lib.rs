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

pub use kittui_core::terminal::{GraphicsCompressionMode, Transport, TransportDiagnostics};
pub use kittui_core::{
    Animation, BlendMode, CellRect, CellSize, Corners, Direction, Fit, ImageRef, Layer, Node,
    Paint, PhaseCurve, Px, PxRect, Rgba, Scene, SceneId, Stop, Stroke, TerminalInfo,
    STANDARD_ANIMATION_CYCLE_MS, STANDARD_ANIMATION_FPS, STANDARD_ANIMATION_FRAMES,
};

use std::fmt::Write as FmtWrite;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    // Raw-frame shared-memory backing files kept alive until the next upload
    // for the same image id, explicit unplace, or runtime drop.
    raw_shm: Mutex<std::collections::HashMap<u32, PathBuf>>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        for (_, path) in self.raw_shm.get_mut().drain() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Runtime {
    /// Build a runtime with explicit configuration.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }

    /// Render a scene into PNG bytes without placing it in a terminal.
    ///
    /// This is the render-only substrate for foreign hosts and previews. It
    /// does not check kitty terminal capabilities, mutate placement state, or
    /// emit kitty escape sequences.
    pub fn render_png(&self, scene: &Scene) -> Result<Vec<u8>, KittuiError> {
        Ok(self.render_still_with_backend(scene)?.png)
    }

    /// Render many scenes into PNG bytes, preserving input order.
    ///
    /// Like [`render_png`](Self::render_png), this does not require terminal
    /// placement support and does not mutate placement/upload state.
    pub fn render_many_png(&self, scenes: &[Scene]) -> Result<Vec<Vec<u8>>, KittuiError> {
        let mut out = Vec::with_capacity(scenes.len());
        for scene in scenes {
            out.push(self.render_png(scene)?);
        }
        Ok(out)
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
        self.place_at_with_options(
            scene,
            placement_footprint,
            &kitty::PlacementOptions::unicode(),
        )
    }

    /// Like [`Runtime::place_at`], but with explicit kitty placement options
    /// such as z-index. This lets hosts keep app surfaces and chrome on
    /// separate compositor planes without changing scene content.
    pub fn place_at_with_options(
        &self,
        scene: &Scene,
        placement_footprint: CellRect,
        options: &kitty::PlacementOptions,
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
            let p = kitty::placement_command_ex(image_id, placement_footprint, options, transport);
            placement_escape(&mv, &p)
        };
        let embed = if options.unicode_placeholder {
            kitty::placeholder_text(image_id, placement_footprint)
        } else {
            String::new()
        };
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
        self.remove_raw_shm(image_id);
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
        self.place_raw_frame_with_options(
            image_id,
            rgba,
            width,
            height,
            footprint,
            &kitty::PlacementOptions::unicode(),
        )
    }

    /// Like [`Runtime::place_raw_frame`], but with explicit kitty placement
    /// options such as z-index.
    pub fn place_raw_frame_with_options(
        &self,
        image_id: u32,
        rgba: &[u8],
        width: u32,
        height: u32,
        footprint: CellRect,
        options: &kitty::PlacementOptions,
    ) -> Placement {
        let upload = self.raw_frame_upload(image_id, rgba, width, height);
        self.place_uploaded_image_with_upload_and_options(image_id, footprint, upload, options)
    }

    /// WM/browser hot path: emit a `Placement` for an already encoded PNG
    /// frame without constructing a `Scene` or using unicode placeholder text
    /// at the call site.
    ///
    /// This keeps first-party native apps on the same kittui runtime transport
    /// and placement semantics as scene/raw-frame surfaces while allowing
    /// adapters such as headless browsers to provide their native PNG capture.
    pub fn place_png_frame_with_options(
        &self,
        image_id: u32,
        png: &[u8],
        footprint: CellRect,
        options: &kitty::PlacementOptions,
    ) -> Placement {
        let upload = kitty::upload_still(image_id, png, self.terminal.transport);
        self.place_uploaded_image_with_upload_and_options(image_id, footprint, upload, options)
    }

    /// Emit placement/embed text for an image id that is already uploaded.
    ///
    /// This is useful for WM policies that skip re-uploading byte-identical raw
    /// frames but still need to redraw or move the terminal placement.
    pub fn place_uploaded_image(&self, image_id: u32, footprint: CellRect) -> Placement {
        self.place_uploaded_image_with_options(
            image_id,
            footprint,
            &kitty::PlacementOptions::unicode(),
        )
    }

    /// Like [`Runtime::place_uploaded_image`], but with explicit kitty placement
    /// options such as z-index.
    pub fn place_uploaded_image_with_options(
        &self,
        image_id: u32,
        footprint: CellRect,
        options: &kitty::PlacementOptions,
    ) -> Placement {
        self.place_uploaded_image_with_upload_and_options(
            image_id,
            footprint,
            String::new(),
            options,
        )
    }

    fn raw_frame_upload(&self, image_id: u32, rgba: &[u8], width: u32, height: u32) -> String {
        match self.terminal.transport {
            Transport::File => self
                .write_raw_frame_tempfile(image_id, rgba)
                .map(|path| {
                    kitty::upload_still_rgba_medium(
                        image_id,
                        kitty::UploadMedium::TempFile { path: &path },
                        width,
                        height,
                        kitty::Quiet::SuppressAll,
                        self.terminal.transport,
                    )
                })
                .unwrap_or_else(|| {
                    kitty::upload_still_rgba(image_id, rgba, width, height, Transport::Direct)
                }),
            Transport::Memory => self
                .write_raw_frame_shm(image_id, rgba)
                .map(|name| {
                    kitty::upload_still_rgba_medium(
                        image_id,
                        kitty::UploadMedium::SharedMemory { name: &name },
                        width,
                        height,
                        kitty::Quiet::SuppressAll,
                        Transport::Memory,
                    )
                })
                .or_else(|| {
                    self.write_raw_frame_tempfile(image_id, rgba).map(|path| {
                        kitty::upload_still_rgba_medium(
                            image_id,
                            kitty::UploadMedium::TempFile { path: &path },
                            width,
                            height,
                            kitty::Quiet::SuppressAll,
                            Transport::File,
                        )
                    })
                })
                .unwrap_or_else(|| {
                    kitty::upload_still_rgba(image_id, rgba, width, height, Transport::Direct)
                }),
            transport => kitty::upload_still_rgba(image_id, rgba, width, height, transport),
        }
    }

    fn write_raw_frame_tempfile(&self, image_id: u32, rgba: &[u8]) -> Option<PathBuf> {
        let mut path = std::env::temp_dir();
        let now = raw_frame_unique_suffix();
        path.push(raw_frame_temp_file_name(std::process::id(), image_id, now));
        std::fs::write(&path, rgba).ok()?;
        Some(path)
    }

    fn write_raw_frame_shm(&self, image_id: u32, rgba: &[u8]) -> Option<String> {
        let (name, path) = raw_frame_shm_name_path(image_id, raw_frame_unique_suffix())?;
        std::fs::write(&path, rgba).ok()?;
        if let Some(old) = self.raw_shm.lock().insert(image_id, path) {
            let _ = std::fs::remove_file(old);
        }
        Some(name)
    }

    fn remove_raw_shm(&self, image_id: u32) {
        if let Some(path) = self.raw_shm.lock().remove(&image_id) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn place_uploaded_image_with_upload_and_options(
        &self,
        image_id: u32,
        footprint: CellRect,
        upload: String,
        options: &kitty::PlacementOptions,
    ) -> Placement {
        let transport = self.terminal.transport;
        let placement = {
            let mv = kitty::cursor_move(footprint.x, footprint.y, transport);
            let p = kitty::placement_command_ex(image_id, footprint, options, transport);
            placement_escape(&mv, &p)
        };
        let embed = if options.unicode_placeholder {
            kitty::placeholder_text(image_id, footprint)
        } else {
            String::new()
        };
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
        let placements = self.place_many(scenes)?;
        Ok(BatchPlacement::from_placements(&placements))
    }

    /// Place a batch with its minimum `x`/`y` remapped to `origin_x`/`origin_y`.
    ///
    /// Each scene is rendered and cached using its own scene-local footprint,
    /// but placement escapes and placeholders are emitted at a group origin.
    /// Relative offsets inside the batch are preserved. Empty batches succeed
    /// and return an empty [`BatchPlacement`].
    pub fn place_batch_at_origin(
        &self,
        scenes: &[Scene],
        origin_x: u16,
        origin_y: u16,
    ) -> Result<BatchPlacement, KittuiError> {
        let Some(min_x) = scenes.iter().map(|scene| scene.footprint.x).min() else {
            return Ok(BatchPlacement::default());
        };
        let min_y = scenes
            .iter()
            .map(|scene| scene.footprint.y)
            .min()
            .unwrap_or(0);
        let mut placements = Vec::with_capacity(scenes.len());
        for scene in scenes {
            let rel_x = scene.footprint.x.saturating_sub(min_x);
            let rel_y = scene.footprint.y.saturating_sub(min_y);
            let footprint = CellRect::new(
                origin_x.saturating_add(rel_x),
                origin_y.saturating_add(rel_y),
                scene.footprint.cols,
                scene.footprint.rows,
            );
            placements.push(self.place_at(scene, footprint)?);
        }
        Ok(BatchPlacement::from_placements(&placements))
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

    fn has_image_uploaded(&self, image_id: u32) -> bool {
        self.placed.lock().contains_key(&image_id)
    }

    fn mark_placed(&self, image_id: u32, footprint: CellRect) {
        self.placed.lock().insert(image_id, footprint);
    }
}

fn placement_escape(cursor_move: &str, placement: &str) -> String {
    let mut out = String::with_capacity(cursor_move.len() + placement.len());
    out.push_str(cursor_move);
    out.push_str(placement);
    out
}

fn raw_frame_unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

fn raw_frame_temp_file_name(pid: u32, image_id: u32, suffix: u128) -> String {
    let mut file_name = String::with_capacity(
        "kittui-raw---.rgba".len()
            + decimal_len_u128(pid as u128)
            + decimal_len_u128(image_id as u128)
            + decimal_len_u128(suffix),
    );
    file_name.push_str("kittui-raw-");
    write!(file_name, "{pid}-{image_id}-{suffix}.rgba").expect("write to string");
    file_name
}

fn raw_frame_shm_name_path(image_id: u32, suffix: u128) -> Option<(String, PathBuf)> {
    let dir = PathBuf::from("/dev/shm");
    if !dir.is_dir() {
        return None;
    }
    let file_name = raw_frame_shm_file_name(std::process::id(), image_id, suffix);
    let mut posix_name = String::with_capacity(1 + file_name.len());
    posix_name.push('/');
    posix_name.push_str(&file_name);
    Some((posix_name, dir.join(file_name)))
}

fn raw_frame_shm_file_name(pid: u32, image_id: u32, suffix: u128) -> String {
    let mut file_name = String::with_capacity(
        "kittui-raw-shm---.rgba".len()
            + decimal_len_u128(pid as u128)
            + decimal_len_u128(image_id as u128)
            + decimal_len_u128(suffix),
    );
    file_name.push_str("kittui-raw-shm-");
    write!(file_name, "{pid}-{image_id}-{suffix}.rgba").expect("write to string");
    file_name
}

fn decimal_len_u128(mut value: u128) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

fn now_rfc3339() -> String {
    // Avoid pulling chrono just for one timestamp; format seconds-since-epoch
    // as a stable, sortable ISO-ish string. Good enough for probe.json freshness.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    epoch_timestamp(secs)
}

fn epoch_timestamp(secs: u64) -> String {
    let mut timestamp = String::with_capacity("epoch:".len() + decimal_len_u128(secs as u128));
    timestamp.push_str("epoch:");
    write!(timestamp, "{secs}").expect("write to string");
    timestamp
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
            raw_shm: Mutex::new(Default::default()),
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

impl BatchPlacement {
    fn from_placements(placements: &[Placement]) -> Self {
        let mut batch = Self::default();
        for p in placements {
            batch.upload.push_str(&p.upload);
            batch.placement.push_str(&p.placement);
            batch.embed.push_str(&p.embed);
            batch.image_ids.push(p.image_id);
            batch.footprints.push(p.footprint);
        }
        batch
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
    fn render_png_returns_png_without_terminal_support() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                false,
                false,
                Transport::Direct,
            ))
            .build()
            .unwrap();
        let scene = builders::simple_solid_box(4, 2, "#00d8ff");
        let png = runtime.render_png(&scene).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        match runtime.place(&scene) {
            Ok(_) => panic!("place unexpectedly succeeded without terminal support"),
            Err(err) => assert!(matches!(err, KittuiError::UnsupportedTerminal(_))),
        }
    }

    #[test]
    fn render_many_png_returns_one_png_per_scene() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                false,
                false,
                Transport::Direct,
            ))
            .build()
            .unwrap();
        let scenes = vec![
            builders::simple_solid_box(2, 1, "#ff0000"),
            builders::simple_solid_box(3, 1, "#00ff00"),
        ];
        let pngs = runtime.render_many_png(&scenes).unwrap();
        assert_eq!(pngs.len(), 2);
        assert!(pngs.iter().all(|png| png.starts_with(b"\x89PNG\r\n\x1a\n")));
        assert!(runtime.render_many_png(&[]).unwrap().is_empty());
    }

    #[test]
    fn png_frame_placement_uses_runtime_transport_and_options() {
        let rt = runtime_with_terminal(TerminalInfo::override_with(
            None,
            None,
            CellSize::default(),
            true,
            true,
            Transport::Direct,
        ));
        let mut opts = kitty::PlacementOptions::absolute();
        opts.z_index = 7;
        let placement = rt.place_png_frame_with_options(
            404,
            b"not-a-real-png-but-uploadable-bytes",
            CellRect::new(2, 3, 4, 5),
            &opts,
        );
        assert!(placement.upload.contains("a=t"), "{}", placement.upload);
        assert!(placement.upload.contains("i=404"), "{}", placement.upload);
        assert!(
            placement.placement.contains("a=p"),
            "{}",
            placement.placement
        );
        assert!(
            placement.placement.contains("z=7"),
            "{}",
            placement.placement
        );
        assert!(
            !placement.placement.contains("U=1"),
            "{}",
            placement.placement
        );
        assert!(placement.embed.is_empty());
        assert_eq!(placement.footprint, CellRect::new(2, 3, 4, 5));
    }

    #[test]
    fn placement_options_allow_hosts_to_assign_z_planes() {
        let rt = runtime_with_terminal(TerminalInfo::override_with(
            None,
            None,
            CellSize::default(),
            true,
            true,
            Transport::Direct,
        ));
        let scene = builders::simple_solid_box(2, 1, "#00d8ff");
        let mut opts = kitty::PlacementOptions::unicode();
        opts.z_index = 12;
        let placement = rt
            .place_at_with_options(&scene, scene.footprint, &opts)
            .unwrap();
        assert!(
            placement.placement.contains("z=12"),
            "{}",
            placement.placement
        );

        let rgba = vec![0xff; 4 * 2 * 2];
        opts.z_index = -5;
        let raw =
            rt.place_raw_frame_with_options(777, &rgba, 2, 2, CellRect::new(0, 0, 1, 1), &opts);
        assert!(raw.placement.contains("z=-5"), "{}", raw.placement);

        opts.z_index = -4;
        let moved = rt.place_uploaded_image_with_options(777, CellRect::new(1, 0, 1, 1), &opts);
        assert!(moved.upload.is_empty());
        assert!(moved.placement.contains("z=-4"), "{}", moved.placement);
        assert!(!moved.embed.is_empty());

        let absolute = rt.place_uploaded_image_with_options(
            777,
            CellRect::new(2, 0, 1, 1),
            &kitty::PlacementOptions::absolute(),
        );
        assert!(absolute.embed.is_empty());
        assert!(
            !absolute.placement.contains("U=1"),
            "{}",
            absolute.placement
        );
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
    fn raw_frame_reupload_replaces_without_delete() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                true,
                true,
                Transport::Direct,
            ))
            .build()
            .unwrap();
        let rgba = vec![0xff; 2 * 2 * 4];
        let footprint = CellRect::new(0, 0, 1, 1);
        let first = runtime.place_raw_frame(7, &rgba, 2, 2, footprint);
        assert!(
            !first.upload.contains("a=d"),
            "first upload should not delete"
        );
        let second = runtime.place_raw_frame(7, &rgba, 2, 2, footprint);
        assert!(
            !second.upload.contains("a=d"),
            "same-id reupload should not delete first; deleting before upload causes visible flicker: {:?}",
            second.upload
        );
        assert!(second.upload.starts_with("\x1b_Ga=t,f=32"));
    }

    #[test]
    fn uploaded_raw_frame_repositions_without_upload_or_delete() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                true,
                true,
                Transport::Direct,
            ))
            .build()
            .unwrap();
        let rgba = vec![0xff; 2 * 2 * 4];
        let first = runtime.place_raw_frame(8, &rgba, 2, 2, CellRect::new(0, 0, 1, 1));
        assert!(first.upload.starts_with("\x1b_Ga=t,f=32"));
        let moved = runtime.place_uploaded_image(8, CellRect::new(10, 5, 2, 2));
        assert!(
            moved.upload.is_empty(),
            "move should not re-upload raw frame"
        );
        assert!(
            !moved.placement.contains("a=d"),
            "move should not delete/recreate image and flicker: {:?}",
            moved.placement
        );
        assert_eq!(moved.footprint, CellRect::new(10, 5, 2, 2));
        assert!(
            moved.placement.contains("\x1b[6;11H"),
            "{:?}",
            moved.placement
        );
        assert_eq!(moved.image_id, first.image_id);
    }

    #[test]
    fn raw_frame_file_transport_uses_tempfile_medium() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                true,
                true,
                Transport::File,
            ))
            .build()
            .unwrap();
        let rgba = vec![0x7f; 2 * 2 * 4];
        let placement = runtime.place_raw_frame(17, &rgba, 2, 2, CellRect::new(0, 0, 1, 1));
        assert!(
            placement
                .upload
                .starts_with("\x1b_Ga=t,f=32,s=2,v=2,t=t,i=17,q=2;"),
            "raw frame file transport should use tempfile medium: {:?}",
            placement.upload
        );
        let prefix_name = raw_frame_temp_file_name(std::process::id(), 17, 0);
        let prefix = prefix_name.trim_end_matches("0.rgba");
        if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    #[test]
    fn raw_frame_memory_transport_uses_shm_when_available_or_safe_fallback() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                true,
                true,
                Transport::Memory,
            ))
            .build()
            .unwrap();
        let rgba = vec![0x44; 2 * 2 * 4];
        let placement = runtime.place_raw_frame(18, &rgba, 2, 2, CellRect::new(0, 0, 1, 1));
        if PathBuf::from("/dev/shm").is_dir() {
            assert!(
                placement
                    .upload
                    .starts_with("\x1b_Ga=t,f=32,s=2,v=2,t=s,i=18,q=2;"),
                "memory transport should use shared-memory medium when /dev/shm exists: {:?}",
                placement.upload
            );
            assert!(runtime.raw_shm.lock().contains_key(&18));
            let path = runtime.raw_shm.lock().get(&18).cloned().unwrap();
            assert!(path.exists());
            runtime.unplace(18);
            assert!(
                !path.exists(),
                "unplace should remove raw-frame shm backing file"
            );
        } else {
            assert!(
                placement.upload.contains("t=t") || placement.upload.contains("f=32"),
                "memory transport should fall back safely: {:?}",
                placement.upload
            );
        }
    }

    #[test]
    fn raw_frame_temp_file_name_builds_directly() {
        let file_name = raw_frame_temp_file_name(1234, 17, 5678);
        assert_eq!(file_name, "kittui-raw-1234-17-5678.rgba");
        assert_eq!(file_name.capacity(), file_name.len());
    }

    #[test]
    fn placement_escape_builds_directly() {
        let escape = placement_escape("\x1b[2;3H", "\x1b_Ga=p,i=7\x1b\\");
        assert_eq!(escape, "\x1b[2;3H\x1b_Ga=p,i=7\x1b\\");
        assert_eq!(escape.capacity(), escape.len());
    }

    #[test]
    fn epoch_timestamp_builds_directly() {
        let timestamp = epoch_timestamp(1234567890);
        assert_eq!(timestamp, "epoch:1234567890");
        assert_eq!(timestamp.capacity(), timestamp.len());
    }

    #[test]
    fn raw_frame_shm_file_name_builds_directly() {
        let file_name = raw_frame_shm_file_name(1234, 19, 5678);
        assert_eq!(file_name, "kittui-raw-shm-1234-19-5678.rgba");
        assert_eq!(file_name.capacity(), file_name.len());
        assert_eq!(decimal_len_u128(0), 1);
        assert_eq!(decimal_len_u128(9), 1);
        assert_eq!(decimal_len_u128(10), 2);
        assert_eq!(decimal_len_u128(12345678901234567890), 20);
    }

    #[test]
    fn raw_frame_shm_name_path_uses_posix_name_and_dev_shm_path() {
        if let Some((name, path)) = raw_frame_shm_name_path(19, 123) {
            assert!(name.starts_with('/'));
            assert!(name.contains("kittui-raw-shm-"));
            assert!(name.ends_with("-19-123.rgba"));
            assert_eq!(path.file_name().unwrap().to_string_lossy(), &name[1..]);
            assert_eq!(path.parent().unwrap(), PathBuf::from("/dev/shm"));
        }
    }

    #[test]
    fn place_uploaded_image_emits_no_upload() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .terminal(TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                true,
                true,
                Transport::Direct,
            ))
            .build()
            .unwrap();
        let placement = runtime.place_uploaded_image(9, CellRect::new(1, 2, 3, 4));
        assert!(placement.upload.is_empty());
        assert!(placement.placement.contains("\x1b[3;2H"));
        assert!(!placement.embed.is_empty());
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
    fn place_batch_at_origin_preserves_relative_offsets() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let mut a = builders::simple_solid_box(2, 1, "#ff0000");
        let mut b = builders::simple_solid_box(3, 1, "#00ff00");
        a.footprint.x = 2;
        a.footprint.y = 4;
        b.footprint.x = 7;
        b.footprint.y = 6;
        let batch = runtime.place_batch_at_origin(&[a, b], 10, 20).unwrap();
        assert_eq!(
            batch.footprints,
            vec![CellRect::new(10, 20, 2, 1), CellRect::new(15, 22, 3, 1),]
        );
        assert!(
            batch.placement.contains("\x1b[21;11H"),
            "{:?}",
            batch.placement
        );
        assert!(
            batch.placement.contains("\x1b[23;16H"),
            "{:?}",
            batch.placement
        );
    }

    #[test]
    fn place_batch_at_origin_accepts_empty_batches() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let batch = runtime.place_batch_at_origin(&[], 10, 20).unwrap();
        assert!(batch.upload.is_empty());
        assert!(batch.placement.is_empty());
        assert!(batch.embed.is_empty());
        assert!(batch.image_ids.is_empty());
        assert!(batch.footprints.is_empty());
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
