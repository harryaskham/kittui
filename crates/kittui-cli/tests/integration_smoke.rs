//! Workspace integration smoke for kittui (placed in kittui-cli because
//! the workspace lacks a root crate).
//!
//! Builds a CPU Runtime, renders a small scene, and checks the kitty
//! graphics grammar of the resulting placement bytes. No real terminal
//! is required.

use std::sync::atomic::{AtomicU64, Ordering};

use kittui::scene::builders::simple_solid_box;
use kittui::{CellSize, RendererKind, Runtime, TerminalInfo, Transport};

fn unique_tmp(label: &str) -> std::path::PathBuf {
    static C: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = C.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("kittui-it-{label}-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn workspace_smoke_renders_box_and_emits_kitty_grammar() {
    let runtime = Runtime::builder()
        .cache_dir(unique_tmp("smoke"))
        .renderer(RendererKind::Cpu)
        // Pin Direct transport so the asserted Direct-grammar is deterministic
        // regardless of the ambient terminal (e.g. running inside tmux, which
        // would otherwise wrap placements in tmux passthrough).
        .terminal(TerminalInfo::override_with(
            Some(80),
            Some(24),
            CellSize::default(),
            true,
            true,
            Transport::Direct,
        ))
        .build()
        .unwrap();
    let scene = simple_solid_box(4, 2, "#00d8ff");
    let placement = runtime.place(&scene).unwrap();

    assert!(
        placement.upload.contains("\x1b_Ga=t,"),
        "upload missing a=t verb"
    );
    assert!(placement.upload.ends_with("\x1b\\"));
    // Placement now begins with a CSI cursor-move so footprint.x/y are
    // honoured (bd-12568a); the kitty graphics escape follows it.
    assert!(
        placement.placement.starts_with("\x1b[")
            && placement.placement.contains("H\x1b_Ga=p,"),
        "placement missing CSI-move + a=p prefix: {:?}",
        &placement.placement[..placement.placement.len().min(40)]
    );
    assert!(placement.placement.ends_with("\x1b\\"));

    let footprint = scene.footprint;
    let placeholder = kittui_kitty::PLACEHOLDER_CHAR;
    let count = placement.embed.matches(placeholder).count();
    assert_eq!(count, (footprint.cols as usize) * (footprint.rows as usize));
}

#[test]
fn workspace_smoke_batch_renders_three_scenes() {
    let runtime = Runtime::builder()
        .cache_dir(unique_tmp("batch"))
        .renderer(RendererKind::Cpu)
        .build()
        .unwrap();
    let scenes = vec![
        simple_solid_box(2, 1, "#ff0000"),
        simple_solid_box(3, 1, "#00ff00"),
        simple_solid_box(4, 1, "#0000ff"),
    ];
    let batch = runtime.place_batch(&scenes).unwrap();
    assert_eq!(batch.image_ids.len(), 3);
    assert!(!batch.upload.is_empty());
    assert!(!batch.placement.is_empty());
    assert!(!batch.embed.is_empty());
}

#[test]
fn scene_from_json_fuzz_lite_does_not_panic() {
    // Bounded fuzz: walk a small deterministic xorshift over malformed and
    // partially-valid JSON blobs and assert that `serde_json::from_str`
    // never panics. This mirrors what cargo-fuzz would do on Scene parsing.
    let seed = 0xC0FFEE_u64;
    let mut state = seed;
    let mut step = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let templates = [
        "{}",
        "{\"footprint\":{\"x\":0,\"y\":0,\"cols\":1,\"rows\":1}}",
        "not json",
        "{\"footprint\":1}",
        "[1,2,3]",
    ];
    for _ in 0..1024 {
        let pick = (step() as usize) % templates.len();
        let len = (step() as usize) % 32;
        let payload = format!("{}{}", &templates[pick][..templates[pick].len().min(len + 1)], "");
        // Must not panic, regardless of result.
        let _ = serde_json::from_str::<kittui::Scene>(&payload);
    }
}
