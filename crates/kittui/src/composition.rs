//! Diff-driven composition.
//!
//! A `Composition` is the set of scenes a host wants visible this frame.
//! Hosts mutate the composition between frames; `Composition::diff` returns
//! the minimal set of uploads, placements, and deletions to apply against
//! the previous frame's state.
//!
//! This is the library-level cousin of `ratakittui::LifecycleTracker`.
//! ratakittui-specific glue stays in ratakittui; this module is for
//! non-ratatui hosts (Pi panels, kittui-wm, kittui-tmux, FFI consumers)
//! that want the same upload-once / place-while-visible / delete-when-gone
//! contract without re-implementing it.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::{CellRect, Placement, Runtime, Scene, SceneId};

/// One entry in a composition: a scene + the cell rect to place it at.
#[derive(Clone, Debug)]
pub struct CompositionEntry {
    /// Optional stable key the host can use to refer to this entry
    /// later. If `None`, the scene id is used as the key.
    pub key: Option<String>,
    /// Cell-space placement footprint. Must match `scene.footprint`.
    pub footprint: CellRect,
    /// Scene to render.
    pub scene: Scene,
}

/// A composition the host wants on-screen this frame.
#[derive(Default, Clone, Debug)]
pub struct Composition {
    entries: Vec<CompositionEntry>,
}

impl Composition {
    /// Construct an empty composition.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a scene to the composition.
    pub fn push(&mut self, entry: CompositionEntry) -> &mut Self {
        self.entries.push(entry);
        self
    }

    /// Borrow the entries.
    pub fn entries(&self) -> &[CompositionEntry] {
        &self.entries
    }
}

/// State retained between frames so `diff` can compute what changed.
/// Hosts hold a `Composer` for the lifetime of the application.
pub struct Composer {
    state: Mutex<HashMap<String, PlacedRecord>>,
}

#[derive(Clone, Debug)]
struct PlacedRecord {
    scene_id: SceneId,
    image_id: u32,
    footprint: CellRect,
}

impl Default for Composer {
    fn default() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
        }
    }
}

impl Composer {
    /// Construct a fresh composer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a composition against `runtime`. Returns the byte streams
    /// the host should write to the terminal:
    /// * `upload` first (escape sequences that copy bytes to the kitty
    ///   image registry),
    /// * `placement` next (escape sequences + unicode placeholders that
    ///   anchor the image at its target cells),
    /// * `deletes` last (escape sequences for placements that fell out
    ///   of this frame).
    ///
    /// On cache + unchanged-placement hits, `upload` is empty. On no-op
    /// frames (no changes at all), all three are empty.
    pub fn apply(
        &self,
        composition: &Composition,
        runtime: &Runtime,
    ) -> Result<DiffResult, crate::KittuiError> {
        let mut prev = self.state.lock();
        let mut next: HashMap<String, PlacedRecord> = HashMap::new();
        let mut upload = String::new();
        let mut placement = String::new();
        let mut placements_emitted = 0usize;

        for entry in &composition.entries {
            let placed: Placement = runtime.place_at(&entry.scene, entry.footprint)?;
            let scene_id = entry.scene.id();
            let key = entry.key.clone().unwrap_or_else(|| scene_id.0.clone());

            let needs_placement = match prev.get(&key) {
                Some(prev_record) => {
                    prev_record.scene_id != scene_id || prev_record.footprint != entry.footprint
                }
                None => true,
            };

            if !placed.upload.is_empty() {
                upload.push_str(&placed.upload);
            }
            if needs_placement {
                placement.push_str(&placed.placement);
                placement.push_str(&placed.embed);
                placements_emitted += 1;
            }

            next.insert(
                key,
                PlacedRecord {
                    scene_id,
                    image_id: placed.image_id,
                    footprint: entry.footprint,
                },
            );
        }

        // Anything in prev that didn't survive into next must be deleted.
        let mut deletes = String::new();
        let mut deleted = 0usize;
        for (key, record) in prev.iter() {
            if !next.contains_key(key) {
                deletes.push_str(&runtime.unplace(record.image_id));
                deleted += 1;
            }
        }
        *prev = next;

        Ok(DiffResult {
            upload,
            placement,
            deletes,
            placements_emitted,
            deleted,
        })
    }

    /// Drop all retained state and return the delete escapes for every
    /// previously-placed image. Useful for clean-shutdown paths.
    pub fn drain(&self, runtime: &Runtime) -> String {
        let mut prev = self.state.lock();
        let mut out = String::new();
        for record in prev.values() {
            out.push_str(&runtime.unplace(record.image_id));
        }
        prev.clear();
        out
    }
}

/// Result of [`Composer::apply`].
#[derive(Clone, Debug, Default)]
pub struct DiffResult {
    /// Concatenated upload escape sequences. Empty on cache hits.
    pub upload: String,
    /// Concatenated placement + embed strings for entries that need to
    /// move (new entries, moved footprints, or changed scene ids).
    pub placement: String,
    /// Delete escape sequences for entries that fell out of the frame.
    pub deletes: String,
    /// Number of placement escapes emitted.
    pub placements_emitted: usize,
    /// Number of entries deleted.
    pub deleted: usize,
}

impl DiffResult {
    /// Whether this diff has any bytes to write at all.
    pub fn is_empty(&self) -> bool {
        self.upload.is_empty() && self.placement.is_empty() && self.deletes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        scene::{background_solid, scene},
        CellRect, CellSize, RendererKind, Rgba, Runtime,
    };
    use std::fmt::Write as FmtWrite;

    fn tempdir() -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(composer_test_temp_dir_name(pid, nanos));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn composer_test_temp_dir_name(pid: u32, nanos: u128) -> String {
        let mut name = String::with_capacity(
            "kittui-composer-".len() + decimal_len(pid as u128) + 1 + decimal_len(nanos),
        );
        name.push_str("kittui-composer-");
        write!(name, "{pid}-{nanos}").expect("write to string");
        name
    }

    fn decimal_len(mut value: u128) -> usize {
        let mut digits = 1;
        while value >= 10 {
            value /= 10;
            digits += 1;
        }
        digits
    }

    fn rt() -> Runtime {
        Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap()
    }

    fn entry(key: &str, color: Rgba, x: u16, y: u16) -> CompositionEntry {
        let cell = CellSize::default();
        let footprint = CellRect::new(x, y, 4, 2);
        let s = scene(
            footprint,
            cell,
            vec![background_solid(footprint, cell, color)],
        );
        CompositionEntry {
            key: Some(key.to_owned()),
            footprint,
            scene: s,
        }
    }

    #[test]
    fn composer_test_temp_dir_name_builds_directly() {
        let name = composer_test_temp_dir_name(1234, 5678);
        assert_eq!(name, "kittui-composer-1234-5678");
        assert_eq!(name.capacity(), name.len());
        assert_eq!(decimal_len(0), 1);
        assert_eq!(decimal_len(9), 1);
        assert_eq!(decimal_len(10), 2);
    }

    #[test]
    fn first_apply_uploads_and_places() {
        let runtime = rt();
        let composer = Composer::new();
        let mut comp = Composition::new();
        comp.push(entry("a", Rgba::rgb(0, 216, 255), 0, 0));
        let diff = composer.apply(&comp, &runtime).unwrap();
        assert!(!diff.upload.is_empty());
        assert!(!diff.placement.is_empty());
        assert_eq!(diff.placements_emitted, 1);
        assert_eq!(diff.deleted, 0);
    }

    #[test]
    fn re_applying_identical_composition_is_a_noop() {
        let runtime = rt();
        let composer = Composer::new();
        let mut comp = Composition::new();
        comp.push(entry("a", Rgba::rgb(0, 216, 255), 0, 0));
        composer.apply(&comp, &runtime).unwrap();
        let diff = composer.apply(&comp, &runtime).unwrap();
        assert!(diff.is_empty(), "expected no-op diff, got {:?}", diff);
    }

    #[test]
    fn moved_entry_re_emits_placement_only_when_scene_unchanged() {
        // If the host wants "same scene, new position", it must keep
        // the scene's footprint fixed (since footprint is part of the
        // scene id) and supply the new placement footprint via the
        // CompositionEntry's `footprint` field. v0.6 of the diff
        // detects the placement-only delta.
        let runtime = rt();
        let composer = Composer::new();
        let cell = CellSize::default();
        let fixed_footprint = CellRect::new(0, 0, 4, 2);
        let s = scene(
            fixed_footprint,
            cell,
            vec![background_solid(
                fixed_footprint,
                cell,
                Rgba::rgb(0, 216, 255),
            )],
        );
        let first = CompositionEntry {
            key: Some("a".to_owned()),
            footprint: fixed_footprint,
            scene: s.clone(),
        };
        let mut comp = Composition::new();
        comp.push(first);
        composer.apply(&comp, &runtime).unwrap();

        let moved = CompositionEntry {
            key: Some("a".to_owned()),
            footprint: CellRect::new(10, 5, 4, 2),
            scene: s,
        };
        let mut moved_comp = Composition::new();
        moved_comp.push(moved);
        let diff = composer.apply(&moved_comp, &runtime).unwrap();
        assert!(
            diff.upload.is_empty(),
            "expected upload to be empty on placement-only move, got {:?}",
            diff
        );
        assert!(!diff.placement.is_empty());
        assert!(
            diff.placement.contains("\x1b[6;11H"),
            "{:?}",
            diff.placement
        );
        assert_eq!(diff.placements_emitted, 1);
    }

    #[test]
    fn dropping_entry_emits_delete() {
        let runtime = rt();
        let composer = Composer::new();
        let mut comp = Composition::new();
        comp.push(entry("a", Rgba::rgb(0, 216, 255), 0, 0));
        comp.push(entry("b", Rgba::rgb(255, 0, 0), 4, 0));
        composer.apply(&comp, &runtime).unwrap();
        let mut smaller = Composition::new();
        smaller.push(entry("a", Rgba::rgb(0, 216, 255), 0, 0));
        let diff = composer.apply(&smaller, &runtime).unwrap();
        assert_eq!(diff.deleted, 1);
        assert!(!diff.deletes.is_empty());
    }

    #[test]
    fn drain_returns_deletes_for_every_placed_entry() {
        let runtime = rt();
        let composer = Composer::new();
        let mut comp = Composition::new();
        comp.push(entry("a", Rgba::rgb(0, 216, 255), 0, 0));
        comp.push(entry("b", Rgba::rgb(255, 0, 0), 4, 0));
        composer.apply(&comp, &runtime).unwrap();
        let out = composer.drain(&runtime);
        assert!(out.contains("\x1b_Ga=d"));
    }
}
