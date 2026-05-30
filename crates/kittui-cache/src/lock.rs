//! Cross-process advisory locking via `fs2::FileExt`.
//!
//! Each `put_*` call takes an exclusive flock on
//! `<root>/locks/<sha>.lock` for the duration of the write. Other
//! processes that race for the same scene id block on the same file
//! and only one rasterization / encode happens per shared cache.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

use fs2::FileExt;

use kittui_core::scene::SceneId;

use crate::CacheError;

/// RAII guard. Holds the lock file open + flocked for as long as the
/// guard lives; releases on drop.
pub struct KeyLockGuard {
    _file: File,
}

/// Take an exclusive lock on the scene id's lockfile. Blocks until
/// other writers release.
pub fn lock_key(root: &Path, id: &SceneId) -> Result<KeyLockGuard, CacheError> {
    let dir = root.join("locks");
    fs::create_dir_all(&dir)?;
    let path = dir.join(lock_file_name(id));
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    file.lock_exclusive().map_err(io::Error::from)?;
    Ok(KeyLockGuard { _file: file })
}

fn lock_file_name(id: &SceneId) -> String {
    let mut name = String::with_capacity(id.0.len() + ".lock".len());
    name.push_str(&id.0);
    name.push_str(".lock");
    name
}

impl Drop for KeyLockGuard {
    fn drop(&mut self) {
        // fs2's File::unlock is a method on FileExt; flock is released
        // when the file descriptor is closed at drop time regardless.
        let _ = self._file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as FmtWrite;

    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(lock_test_temp_dir_name(pid, nanos, seq));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn lock_test_temp_dir_name(pid: u32, nanos: u128, seq: u64) -> String {
        let mut name = String::with_capacity(
            "kittui-lock---".len()
                + decimal_len(pid as u128)
                + decimal_len(nanos)
                + decimal_len(seq as u128),
        );
        name.push_str("kittui-lock-");
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

    #[test]
    fn lock_test_temp_dir_name_builds_directly() {
        let name = lock_test_temp_dir_name(1234, 5678, 9);
        assert_eq!(name, "kittui-lock-1234-5678-9");
        assert_eq!(name.capacity(), name.len());
        assert_eq!(decimal_len(0), 1);
        assert_eq!(decimal_len(9), 1);
        assert_eq!(decimal_len(10), 2);
    }

    #[test]
    fn lock_file_name_builds_directly() {
        let id = SceneId("a".repeat(64));
        let name = lock_file_name(&id);
        assert_eq!(
            name,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.lock"
        );
        assert_eq!(name.capacity(), name.len());
    }

    #[test]
    fn lock_and_release_within_process() {
        let root = tempdir();
        let id = SceneId("a".repeat(64));
        {
            let _guard = lock_key(&root, &id).unwrap();
            // The lockfile exists and is open while the guard is alive.
            assert!(root.join("locks").join(lock_file_name(&id)).exists());
        }
        // After drop, the file remains on disk (we don't unlink) but is
        // re-lockable.
        let _again = lock_key(&root, &id).unwrap();
    }
}
