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
//! └── images/<sha>...                     # external image inputs
//! ```
//!
//! Reads are zero-copy where the OS permits; writes are atomic (write to a
//! tempfile then rename) so concurrent processes can share a cache safely.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use kittui_core::geom::CellRect;
use kittui_core::scene::SceneId;

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

/// Handle to a content-addressed cache rooted at a directory.
#[derive(Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Open (and create if necessary) a cache rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CacheError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("scenes"))?;
        fs::create_dir_all(root.join("images"))?;
        Ok(Self { root })
    }

    /// Borrow the root path.
    pub fn root(&self) -> &Path {
        &self.root
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

    /// Store a still raster atomically.
    pub fn put_still(
        &self,
        id: &SceneId,
        png: &[u8],
        meta: &CacheEntryMeta,
    ) -> Result<(), CacheError> {
        let path = self.still_path(id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, png)?;
        self.put_meta(id, meta)?;
        Ok(())
    }

    /// Read a still raster.
    pub fn get_still(&self, id: &SceneId) -> Result<Vec<u8>, CacheError> {
        Ok(fs::read(self.still_path(id))?)
    }

    /// Store all frames of an animation atomically.
    pub fn put_animation(
        &self,
        id: &SceneId,
        frames: &[Vec<u8>],
        meta: &CacheEntryMeta,
    ) -> Result<(), CacheError> {
        let dir = self.frames_dir(id);
        fs::create_dir_all(&dir)?;
        for (i, frame) in frames.iter().enumerate() {
            atomic_write(&dir.join(format!("{i}.png")), frame)?;
        }
        self.put_meta(id, meta)?;
        Ok(())
    }

    /// Read all frames of an animation.
    pub fn get_animation(&self, id: &SceneId, frames: u32) -> Result<Vec<Vec<u8>>, CacheError> {
        let dir = self.frames_dir(id);
        (0..frames)
            .map(|i| fs::read(dir.join(format!("{i}.png"))).map_err(CacheError::from))
            .collect()
    }

    /// Read metadata for `id`.
    pub fn get_meta(&self, id: &SceneId) -> Result<CacheEntryMeta, CacheError> {
        let bytes = fs::read(self.meta_path(id))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Write metadata for `id` atomically.
    pub fn put_meta(&self, id: &SceneId, meta: &CacheEntryMeta) -> Result<(), CacheError> {
        let path = self.meta_path(id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(meta)?;
        atomic_write(&path, &bytes)?;
        Ok(())
    }

    fn shard(&self, id: &SceneId) -> PathBuf {
        self.root.join("scenes").join(&id.0[..2])
    }

    fn still_path(&self, id: &SceneId) -> PathBuf {
        self.shard(id).join(format!("{}.png", id.0))
    }

    fn frames_dir(&self, id: &SceneId) -> PathBuf {
        self.shard(id).join(format!("{}.frames", id.0))
    }

    fn meta_path(&self, id: &SceneId) -> PathBuf {
        self.shard(id).join(format!("{}.meta.json", id.0))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_cache() -> Cache {
        let dir = tempdir();
        Cache::open(dir).unwrap()
    }

    fn tempdir() -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("kittui-cache-{pid}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
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
}
