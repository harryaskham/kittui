//! kittui-overlay
//!
//! Transient, always-on-top surfaces over kittui. Designed for command
//! palettes, notifications, IME ribbons, and similar UI bits that are
//! summoned briefly and dismissed.
//!
//! An overlay is just a `kittui::Composition` rendered through its own
//! `Composer`, plus a stronger default chrome (heavier shadow, brighter
//! glow) and a placement policy that biases toward higher kitty `z` so
//! it visually stacks on top of whatever the host is rendering
//! underneath.
//!
//! Overlays do not own the terminal write side. Hosts retrieve a
//! `DiffResult` from `Overlay::render` and write its bytes whenever
//! they want — typically at the very end of their frame, after the
//! ratatui buffer flush and any background kittui placements.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use kittui::{CellRect, Composer, Composition, CompositionEntry, DiffResult, Runtime, Rgba, Scene};
use ratakittui::{Background, Border, Chrome, Glow, Padding, Pulse, Shadow};

/// Pre-baked overlay chrome themed for transient surfaces. Hosts can
/// build their own [`Chrome`] and skip this if they want full control.
pub fn default_overlay_chrome() -> Chrome {
    let shadow_color = Rgba::parse("#000000aa").unwrap();
    Chrome::default()
        .background(Background::Solid(Rgba::parse("#0b1626ee").unwrap()))
        .border(Border::rounded(Rgba::parse("#00d8ff").unwrap(), 1.5, 8.0))
        .glow(Glow {
            color: Rgba::parse("#00d8ffaa").unwrap(),
            cx: 0.5,
            cy: 0.5,
            radius: 0.6,
            intensity: 0.6,
            pulse: Some(Pulse {
                frames: 8,
                cycle_ms: 1200,
            }),
        })
        .shadow(Shadow {
            dx_px: 4.0,
            dy_px: 6.0,
            color: shadow_color,
        })
        .padding(Padding::trbl(1, 2, 1, 2))
}

/// Stateful overlay surface. Holds a [`Composer`] so the diff-driven
/// upload/place/delete protocol applies even when the same overlay is
/// shown and hidden repeatedly.
pub struct Overlay {
    composer: Composer,
}

impl Default for Overlay {
    fn default() -> Self {
        Self::new()
    }
}

impl Overlay {
    /// Construct a fresh overlay.
    pub fn new() -> Self {
        Self {
            composer: Composer::new(),
        }
    }

    /// Render the supplied composition through this overlay's
    /// composer. Returns the diff the host should write to the
    /// terminal.
    pub fn render(
        &self,
        composition: &Composition,
        runtime: &Runtime,
    ) -> Result<DiffResult, kittui::KittuiError> {
        self.composer.apply(composition, runtime)
    }

    /// Hide the overlay: drain every retained placement and return the
    /// delete escapes.
    pub fn hide(&self, runtime: &Runtime) -> String {
        self.composer.drain(runtime)
    }

    /// Convenience: build a single-entry composition from a chromed
    /// rectangle. Inner text is the host's responsibility (write it
    /// into the ratatui buffer or directly to the terminal).
    pub fn entry_from_chrome(
        key: impl Into<String>,
        footprint: CellRect,
        chrome: &Chrome,
    ) -> Option<CompositionEntry> {
        let area = ratatui::layout::Rect {
            x: footprint.x,
            y: footprint.y,
            width: footprint.cols,
            height: footprint.rows,
        };
        let scene: Scene = chrome.to_scene(area)?;
        Some(CompositionEntry {
            key: Some(key.into()),
            footprint,
            scene,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui::{CellRect, RendererKind, Runtime};

    fn tempdir() -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("kittui-overlay-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn rt() -> Runtime {
        Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap()
    }

    #[test]
    fn render_then_hide_emits_placement_then_delete() {
        let overlay = Overlay::new();
        let runtime = rt();
        let chrome = default_overlay_chrome();
        let entry = Overlay::entry_from_chrome("palette", CellRect::new(5, 5, 40, 10), &chrome)
            .unwrap();
        let mut comp = Composition::new();
        comp.push(entry);
        let diff = overlay.render(&comp, &runtime).unwrap();
        assert!(!diff.placement.is_empty());

        let deletes = overlay.hide(&runtime);
        assert!(deletes.contains("\x1b_Ga=d"));
    }
}
