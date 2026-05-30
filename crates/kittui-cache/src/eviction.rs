//! LRU eviction by access mtime with a grace window.
//!
//! `collect_stats` walks `<root>/scenes/` once and returns an ordered
//! list of entries with their on-disk byte counts and last-access
//! mtime. `evict_to_budget` evicts oldest-first until total size
//! falls below the configured budget. Entries newer than the grace
//! window are never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{CacheConfig, CacheError};

/// Snapshot of cache disk usage.
#[derive(Clone, Debug, Default)]
pub struct CacheStats {
    /// Total bytes occupied by `scenes/` (stills + frames + meta).
    pub scene_bytes: u64,
    /// Number of distinct scene entries.
    pub scene_count: u64,
    /// Total bytes occupied by `images/`.
    pub image_bytes: u64,
}

/// Result of an eviction pass.
#[derive(Clone, Debug, Default)]
pub struct EvictionReport {
    /// Number of scene entries removed.
    pub removed_entries: u64,
    /// Bytes reclaimed.
    pub reclaimed_bytes: u64,
    /// Number of entries skipped because they were within the grace
    /// window.
    pub skipped_grace: u64,
}

#[derive(Debug)]
#[allow(dead_code)]
struct Entry {
    sha: String,
    paths: Vec<PathBuf>,
    bytes: u64,
    mtime: SystemTime,
}

/// Walk `<root>/scenes/<shard>/` and return one [`Entry`] per scene id.
fn collect_entries(root: &Path) -> Result<Vec<Entry>, CacheError> {
    let scenes = root.join("scenes");
    if !scenes.is_dir() {
        return Ok(Vec::new());
    }
    let mut by_sha: std::collections::BTreeMap<String, Entry> =
        std::collections::BTreeMap::new();
    for shard in fs::read_dir(&scenes)? {
        let shard = shard?;
        if !shard.file_type()?.is_dir() {
            continue;
        }
        for entry in fs::read_dir(shard.path())? {
            let entry = entry?;
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            // sha is the basename before the first extension or
            // ".frames" / ".meta.json" suffix.
            let sha = name
                .split('.')
                .next()
                .unwrap_or(&name)
                .to_owned();
            if sha.len() != 64 {
                continue;
            }
            let bytes = if path.is_dir() {
                directory_bytes(&path)?
            } else {
                fs::metadata(&path)?.len()
            };
            let mtime = fs::metadata(&path)?
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let e = by_sha.entry(sha.clone()).or_insert_with(|| Entry {
                sha: sha.clone(),
                paths: Vec::new(),
                bytes: 0,
                mtime: SystemTime::UNIX_EPOCH,
            });
            e.paths.push(path);
            e.bytes += bytes;
            if mtime > e.mtime {
                e.mtime = mtime;
            }
        }
    }
    Ok(by_sha.into_values().collect())
}

fn directory_bytes(path: &Path) -> Result<u64, CacheError> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += directory_bytes(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Compute cache statistics without modifying anything.
pub fn collect_stats(root: &Path) -> Result<CacheStats, CacheError> {
    let entries = collect_entries(root)?;
    let scene_bytes: u64 = entries.iter().map(|e| e.bytes).sum();
    let scene_count = entries.len() as u64;
    let image_bytes = if root.join("images").is_dir() {
        directory_bytes(&root.join("images"))?
    } else {
        0
    };
    Ok(CacheStats {
        scene_bytes,
        scene_count,
        image_bytes,
    })
}

/// Evict oldest entries until total size is at or below `budget_bytes`.
/// `force=true` ignores the budget threshold and proceeds anyway, but
/// still respects the grace window.
pub fn evict_to_budget(
    root: &Path,
    config: CacheConfig,
    force: bool,
) -> Result<EvictionReport, CacheError> {
    let mut entries = collect_entries(root)?;
    let total: u64 = entries.iter().map(|e| e.bytes).sum();
    if !force && total <= config.budget_bytes {
        return Ok(EvictionReport::default());
    }
    // Oldest first.
    entries.sort_by_key(|e| e.mtime);

    let now = SystemTime::now();
    let grace = std::time::Duration::from_secs(config.grace_secs);
    let mut report = EvictionReport::default();
    let mut size = total;

    for entry in entries {
        if !force && size <= config.budget_bytes {
            break;
        }
        // Skip entries within the grace window.
        if let Ok(age) = now.duration_since(entry.mtime) {
            if age < grace {
                report.skipped_grace += 1;
                continue;
            }
        }
        for path in &entry.paths {
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        report.removed_entries += 1;
        report.reclaimed_bytes += entry.bytes;
        size = size.saturating_sub(entry.bytes);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui_core::scene::SceneId;

    use crate::{Cache, CacheEntryMeta};
    use kittui_core::geom::CellRect;
    use std::fmt::Write as FmtWrite;

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(eviction_test_temp_dir_name(pid, nanos, seq));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn eviction_test_temp_dir_name(pid: u32, nanos: u128, seq: u64) -> String {
        let mut name = String::with_capacity(
            "kittui-evict---".len()
                + decimal_len(pid as u128)
                + decimal_len(nanos)
                + decimal_len(seq as u128),
        );
        name.push_str("kittui-evict-");
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

    fn put(cache: &Cache, sha: &str, bytes: &[u8]) {
        let id = SceneId(sha.to_owned());
        cache
            .put_still(
                &id,
                bytes,
                &CacheEntryMeta {
                    footprint: CellRect::new(0, 0, 1, 1),
                    width_px: 8,
                    height_px: 16,
                    frames: 1,
                    frame_delays_ms: vec![],
                    kitty_image_id: 0,
                    loops: 0,
                },
            )
            .unwrap();
    }

    #[test]
    fn eviction_test_temp_dir_name_builds_directly() {
        let name = eviction_test_temp_dir_name(1234, 5678, 9);
        assert_eq!(name, "kittui-evict-1234-5678-9");
        assert_eq!(name.capacity(), name.len());
        assert_eq!(decimal_len(0), 1);
        assert_eq!(decimal_len(9), 1);
        assert_eq!(decimal_len(10), 2);
    }

    #[test]
    fn force_gc_with_zero_grace_evicts_all_oldest_entries() {
        // Use a large enough budget that put_still's maybe_evict doesn't fire
        // during writes; then sleep to age the entries past the grace window
        // and call gc(force=true) explicitly.
        let cache = Cache::open_with_config(
            tempdir(),
            CacheConfig {
                budget_bytes: 1_000_000,
                grace_secs: 0,
            },
        )
        .unwrap();
        for i in 0..3 {
            let sha = format!("{:0>64}", i);
            put(&cache, &sha, &vec![0u8; 4096]);
        }
        // Age the entries so age > grace=0.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let report = cache.gc().unwrap();
        assert!(
            report.removed_entries >= 1,
            "expected at least 1 eviction, got {:?}",
            report
        );
        assert!(report.reclaimed_bytes > 0);
    }

    #[test]
    fn grace_window_protects_recent_entries() {
        let cache = Cache::open_with_config(
            tempdir(),
            CacheConfig {
                budget_bytes: 1_000_000,
                grace_secs: 600, // 10 minutes
            },
        )
        .unwrap();
        let sha = "f".repeat(64);
        put(&cache, &sha, &vec![0u8; 8192]);
        // Force gc; should skip the entry because it's within grace.
        let report = cache.gc().unwrap();
        assert_eq!(report.removed_entries, 0);
        assert!(report.skipped_grace >= 1);
    }
}
