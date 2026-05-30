//! Renderer-capability probe record.
//!
//! Hosts probe their available rendering backends once per session and
//! cache the result here so subsequent processes can skip the live
//! check. The file lives at `<root>/probe.json` and is rewritten only
//! when the kittui version changes or when an explicit `kittui probe
//! --force` runs.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::CacheError;

/// Probe record schema. Matches `## Cache → probe.json` in DESIGN.md.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProbeRecord {
    /// kittui version that wrote this record.
    pub kittui_version: String,
    /// Adapter name reported by wgpu, if any.
    pub gpu_adapter: Option<String>,
    /// CPU↔GPU SSIM-proxy score on the canonical fixture, if measured.
    pub gpu_parity_ssim: Option<f32>,
    /// Status: `"ok"`, `"fallback"`, or `"unavailable"`.
    pub gpu_status: String,
    /// RFC 3339 timestamp.
    pub checked_at: String,
}

/// Read the probe record if present and non-empty.
pub fn read(root: &Path) -> Result<Option<ProbeRecord>, CacheError> {
    let path = root.join("probe.json");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// Write the probe record atomically.
pub fn write(root: &Path, record: &ProbeRecord) -> Result<(), CacheError> {
    fs::create_dir_all(root)?;
    let path = root.join("probe.json");
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
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
        let path = std::env::temp_dir().join(probe_test_temp_dir_name(pid, nanos, seq));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn probe_test_temp_dir_name(pid: u32, nanos: u128, seq: u64) -> String {
        let mut name = String::with_capacity(
            "kittui-probe---".len()
                + decimal_len(pid as u128)
                + decimal_len(nanos)
                + decimal_len(seq as u128),
        );
        name.push_str("kittui-probe-");
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
    fn probe_test_temp_dir_name_builds_directly() {
        let name = probe_test_temp_dir_name(1234, 5678, 9);
        assert_eq!(name, "kittui-probe-1234-5678-9");
        assert_eq!(name.capacity(), name.len());
        assert_eq!(decimal_len(0), 1);
        assert_eq!(decimal_len(9), 1);
        assert_eq!(decimal_len(10), 2);
    }

    #[test]
    fn round_trip() {
        let root = tempdir();
        assert!(read(&root).unwrap().is_none());
        let record = ProbeRecord {
            kittui_version: "0.1.0".into(),
            gpu_adapter: Some("Apple M2".into()),
            gpu_parity_ssim: Some(0.998),
            gpu_status: "ok".into(),
            checked_at: "2026-05-19T20:00:00Z".into(),
        };
        write(&root, &record).unwrap();
        let read_back = read(&root).unwrap().unwrap();
        assert_eq!(read_back.gpu_status, "ok");
        assert_eq!(read_back.gpu_adapter.as_deref(), Some("Apple M2"));
    }
}
