//! kittui-cache
//!
//! Content-addressed cache for rendered scenes. Keys are blake3 hashes
//! produced by `kittui-core`. The cache layout is:
//!
//! ```text
//! <root>/
//! ├── scenes/<sha[0..2]>/<sha>.png        # still raster
//! ├── scenes/<sha[0..2]>/<sha>.frames/    # one PNG per animation frame
//! ├── scenes/<sha[0..2]>/<sha>.meta.json  # metadata: footprint, frame count, delays
//! ├── images/<sha>...                     # external image inputs
//! ├── locks/<sha>.lock                    # per-key advisory locks
//! └── probe.json                          # renderer capability cache
//! ```
//!
//! Reads are zero-copy where the OS permits; writes are atomic (write to a
//! tempfile then rename) so concurrent processes can share a cache safely.
//! LRU eviction by access mtime with a configurable byte budget; entries
//! younger than the grace window are never evicted.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

mod eviction;
mod lock;
mod probe;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use kittui_core::geom::CellRect;
use kittui_core::scene::SceneId;

pub use eviction::{CacheStats, EvictionReport};
pub use probe::ProbeRecord;

/// Default eviction byte budget: 256 MiB.
pub const DEFAULT_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// Default eviction grace window in seconds. Entries with mtime newer
/// than `now - GRACE` are never evicted, to avoid churn within an
/// interactive session.
pub const DEFAULT_GRACE_SECS: u64 = 60;

/// Cache errors surfaced to callers.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// IO failure while reading or writing.
    #[error("cache io error: {0}")]
    Io(#[from] io::Error),
    /// Metadata JSON could not be parsed.
    #[error("cache metadata parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Metadata stored alongside each cached scene.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntryMeta {
    /// Scene cell footprint at time of caching.
    pub footprint: CellRect,
    /// Pixel-space width.
    pub width_px: u32,
    /// Pixel-space height.
    pub height_px: u32,
    /// Number of frames. `1` for stills, `>= 2` for animations.
    pub frames: u32,
    /// Per-frame delays in milliseconds (empty for stills).
    pub frame_delays_ms: Vec<u32>,
    /// kitty graphics protocol image id this scene is associated with.
    pub kitty_image_id: u32,
    /// Loop count from the originating animation. `0` means loop forever.
    pub loops: u32,
}

/// Eviction configuration for a cache instance.
#[derive(Copy, Clone, Debug)]
pub struct CacheConfig {
    /// Eviction byte budget. Total `scenes/` size; `images/` are not
    /// counted because they are user inputs.
    pub budget_bytes: u64,
    /// Grace window for newly-written entries. Entries younger than
    /// `now - grace_secs` are skipped during eviction.
    pub grace_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            budget_bytes: env_budget().unwrap_or(DEFAULT_BUDGET_BYTES),
            grace_secs: DEFAULT_GRACE_SECS,
        }
    }
}

fn env_budget() -> Option<u64> {
    std::env::var("KITTUI_CACHE_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
}

/// Handle to a content-addressed cache rooted at a directory.
#[derive(Clone)]
pub struct Cache {
    root: PathBuf,
    config: CacheConfig,
}

impl Cache {
    /// Open (and create if necessary) a cache rooted at `root` with
    /// default config.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CacheError> {
        Self::open_with_config(root, CacheConfig::default())
    }

    /// Open (and create if necessary) a cache rooted at `root` with an
    /// explicit config.
    pub fn open_with_config(
        root: impl Into<PathBuf>,
        config: CacheConfig,
    ) -> Result<Self, CacheError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("scenes"))?;
        fs::create_dir_all(root.join("images"))?;
        fs::create_dir_all(root.join("locks"))?;
        Ok(Self { root, config })
    }

    /// Borrow the root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Borrow the active config.
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    /// Whether the cache has a still PNG entry for `id`.
    pub fn contains_still(&self, id: &SceneId) -> bool {
        self.still_path(id).exists()
    }

    /// Whether the cache has all animation frames for `id`.
    pub fn contains_animation(&self, id: &SceneId, frames: u32) -> bool {
        let dir = self.frames_dir(id);
        (0..frames).all(|i| dir.join(format!("{i}.png")).exists())
    }

    /// Store a still raster atomically. Takes a per-key advisory lock
    /// for the duration of the write so concurrent processes do not
    /// produce inconsistent state.
    pub fn put_still(
        &self,
        id: &SceneId,
        png: &[u8],
        meta: &CacheEntryMeta,
    ) -> Result<(), CacheError> {
        let _guard = lock::lock_key(&self.root, id)?;
        let path = self.still_path(id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, png)?;
        self.put_meta_locked(id, meta)?;
        self.maybe_evict()?;
        Ok(())
    }

    /// Read a still raster. Touches mtime so LRU eviction sees the
    /// recency.
    pub fn get_still(&self, id: &SceneId) -> Result<Vec<u8>, CacheError> {
        let path = self.still_path(id);
        touch(&path).ok();
        Ok(fs::read(path)?)
    }

    /// Store all frames of an animation atomically.
    pub fn put_animation(
        &self,
        id: &SceneId,
        frames: &[Vec<u8>],
        meta: &CacheEntryMeta,
    ) -> Result<(), CacheError> {
        let _guard = lock::lock_key(&self.root, id)?;
        let dir = self.frames_dir(id);
        fs::create_dir_all(&dir)?;
        for (i, frame) in frames.iter().enumerate() {
            atomic_write(&dir.join(format!("{i}.png")), frame)?;
        }
        self.put_meta_locked(id, meta)?;
        self.maybe_evict()?;
        Ok(())
    }

    /// Read all frames of an animation. Touches mtime on the frames
    /// directory so LRU eviction sees the access.
    pub fn get_animation(&self, id: &SceneId, frames: u32) -> Result<Vec<Vec<u8>>, CacheError> {
        let dir = self.frames_dir(id);
        touch(&dir).ok();
        (0..frames)
            .map(|i| fs::read(dir.join(format!("{i}.png"))).map_err(CacheError::from))
            .collect()
    }

    /// Read metadata for `id`.
    pub fn get_meta(&self, id: &SceneId) -> Result<CacheEntryMeta, CacheError> {
        let bytes = fs::read(self.meta_path(id))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Write metadata for `id` atomically. Public for the rare case
    /// where a host updates metadata out-of-band; prefer the put_* paths
    /// for normal usage.
    pub fn put_meta(&self, id: &SceneId, meta: &CacheEntryMeta) -> Result<(), CacheError> {
        let _guard = lock::lock_key(&self.root, id)?;
        self.put_meta_locked(id, meta)
    }

    fn put_meta_locked(&self, id: &SceneId, meta: &CacheEntryMeta) -> Result<(), CacheError> {
        let path = self.meta_path(id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(meta)?;
        atomic_write(&path, &bytes)?;
        Ok(())
    }

    /// Force a gc pass regardless of current size. Useful for tests
    /// and for `kittui cache gc`.
    pub fn gc(&self) -> Result<EvictionReport, CacheError> {
        eviction::evict_to_budget(&self.root, self.config, /*force=*/ true)
    }

    /// Clear the entire cache. Idempotent.
    pub fn clear(&self) -> Result<(), CacheError> {
        for sub in ["scenes", "images"] {
            let path = self.root.join(sub);
            if path.exists() {
                fs::remove_dir_all(&path)?;
                fs::create_dir_all(&path)?;
            }
        }
        let probe = self.root.join("probe.json");
        if probe.exists() {
            fs::remove_file(probe)?;
        }
        Ok(())
    }

    /// Compute cache statistics: total scene bytes, scene count, etc.
    pub fn stats(&self) -> Result<CacheStats, CacheError> {
        eviction::collect_stats(&self.root)
    }

    /// Read the probe record if present.
    pub fn read_probe(&self) -> Result<Option<ProbeRecord>, CacheError> {
        probe::read(&self.root)
    }

    /// Write a probe record.
    pub fn write_probe(&self, record: &ProbeRecord) -> Result<(), CacheError> {
        probe::write(&self.root, record)
    }

    fn maybe_evict(&self) -> Result<(), CacheError> {
        let stats = eviction::collect_stats(&self.root)?;
        if stats.scene_bytes > self.config.budget_bytes {
            eviction::evict_to_budget(&self.root, self.config, /*force=*/ false)?;
        }
        Ok(())
    }

    fn shard(&self, id: &SceneId) -> PathBuf {
        self.root.join("scenes").join(&id.0[..2])
    }

    fn still_path(&self, id: &SceneId) -> PathBuf {
        self.shard(id).join(scene_artifact_name(id, ".png"))
    }

    fn frames_dir(&self, id: &SceneId) -> PathBuf {
        self.shard(id).join(scene_artifact_name(id, ".frames"))
    }

    fn meta_path(&self, id: &SceneId) -> PathBuf {
        self.shard(id).join(scene_artifact_name(id, ".meta.json"))
    }
}

fn scene_artifact_name(id: &SceneId, suffix: &str) -> String {
    let mut name = String::with_capacity(id.0.len() + suffix.len());
    name.push_str(&id.0);
    name.push_str(suffix);
    name
}

/// Default cache root, honouring `KITTUI_CACHE_DIR` and XDG conventions.
pub fn default_cache_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("KITTUI_CACHE_DIR") {
        return PathBuf::from(override_dir);
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("kittui");
    }
    if let Ok(home) = std::env::var("HOME") {
        let path = PathBuf::from(home);
        #[cfg(target_os = "macos")]
        {
            return path.join("Library").join("Caches").join("kittui");
        }
        #[cfg(not(target_os = "macos"))]
        {
            return path.join(".cache").join("kittui");
        }
    }
    std::env::temp_dir().join("kittui-cache")
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn touch(path: &Path) -> io::Result<()> {
    // Refresh mtime to "now" so LRU sees the access. `OpenOptions` +
    // `set_modified` is cross-platform; on systems where this fails we
    // silently accept slightly older recency.
    let f = fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(std::time::SystemTime::now())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as FmtWrite;

    fn tmp_cache() -> Cache {
        let dir = tempdir();
        Cache::open(dir).unwrap()
    }

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(cache_test_temp_dir_name(pid, nanos, seq));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn cache_test_temp_dir_name(pid: u32, nanos: u128, seq: u64) -> String {
        let mut name = String::with_capacity(
            "kittui-cache---".len()
                + decimal_len(pid as u128)
                + decimal_len(nanos)
                + decimal_len(seq as u128),
        );
        name.push_str("kittui-cache-");
        write!(name, "{pid}-{nanos}-{seq}").expect("write to string");
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

    fn meta() -> CacheEntryMeta {
        CacheEntryMeta {
            footprint: CellRect::new(0, 0, 2, 2),
            width_px: 16,
            height_px: 32,
            frames: 1,
            frame_delays_ms: vec![],
            kitty_image_id: 0x1234,
            loops: 0,
        }
    }

    #[test]
    fn scene_artifact_names_build_directly() {
        let id = SceneId("a".repeat(64));
        let still = scene_artifact_name(&id, ".png");
        assert_eq!(
            still,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png"
        );
        assert_eq!(still.capacity(), still.len());
        let frames = scene_artifact_name(&id, ".frames");
        assert!(frames.ends_with(".frames"));
        assert_eq!(frames.capacity(), frames.len());
        let meta = scene_artifact_name(&id, ".meta.json");
        assert!(meta.ends_with(".meta.json"));
        assert_eq!(meta.capacity(), meta.len());
    }

    #[test]
    fn cache_test_temp_dir_name_builds_directly() {
        let name = cache_test_temp_dir_name(1234, 5678, 9);
        assert_eq!(name, "kittui-cache-1234-5678-9");
        assert_eq!(name.capacity(), name.len());
        assert_eq!(decimal_len(0), 1);
        assert_eq!(decimal_len(9), 1);
        assert_eq!(decimal_len(10), 2);
    }

    #[test]
    fn still_round_trip() {
        let cache = tmp_cache();
        let id = SceneId("a".repeat(64));
        cache.put_still(&id, b"png-bytes", &meta()).unwrap();
        assert!(cache.contains_still(&id));
        assert_eq!(cache.get_still(&id).unwrap(), b"png-bytes");
        assert_eq!(cache.get_meta(&id).unwrap().kitty_image_id, 0x1234);
    }

    #[test]
    fn animation_round_trip() {
        let cache = tmp_cache();
        let id = SceneId("b".repeat(64));
        let frames = vec![vec![1u8; 4], vec![2u8; 4], vec![3u8; 4]];
        let mut meta = meta();
        meta.frames = 3;
        meta.frame_delays_ms = vec![100, 100, 100];
        cache.put_animation(&id, &frames, &meta).unwrap();
        assert!(cache.contains_animation(&id, 3));
        assert_eq!(cache.get_animation(&id, 3).unwrap(), frames);
    }

    #[test]
    fn clear_removes_scenes_but_recreates_dir() {
        let cache = tmp_cache();
        let id = SceneId("c".repeat(64));
        cache.put_still(&id, b"xx", &meta()).unwrap();
        assert!(cache.contains_still(&id));
        cache.clear().unwrap();
        assert!(!cache.contains_still(&id));
        assert!(cache.root().join("scenes").is_dir());
    }

    #[test]
    fn stats_reports_total_scene_bytes() {
        let cache = tmp_cache();
        cache
            .put_still(&SceneId("d".repeat(64)), &vec![0u8; 1024], &meta())
            .unwrap();
        let stats = cache.stats().unwrap();
        assert!(stats.scene_bytes >= 1024);
        assert!(stats.scene_count >= 1);
    }
}
