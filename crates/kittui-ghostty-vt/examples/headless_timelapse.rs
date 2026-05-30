use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kittui_ghostty_vt::{render_snapshot_preview_png, GhosttyVtTerminal, PreviewOptions};

fn main() -> anyhow::Result<()> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/kittui-ghostty-vt-timelapse"));
    std::fs::create_dir_all(&out_dir)?;

    let mut term = GhosttyVtTerminal::new(56, 12, 400)?;
    let steps: &[&[u8]] = &[
        b"kittui-ghostty timelapse\n",
        b"\x1b[32mframe 1:\x1b[0m libghostty-vt owns terminal state\n",
        b"\x1b[33mframe 2:\x1b[0m render-state rows/cells extracted\n",
        b"\x1b[1mframe 3:\x1b[0m bold + \x1b[3mitalic\x1b[0m + \x1b[4munderline\x1b[0m styles\n",
        b"\x1b[36mframe 4:\x1b[0m kittui can own portable headless pixels\n",
    ];

    let mut frames = Vec::new();
    for (idx, bytes) in steps.iter().enumerate() {
        term.write(*bytes);
        let snapshot = term.render_snapshot()?;
        let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
        let path = out_dir.join(frame_png_name(idx));
        std::fs::write(&path, png)?;
        frames.push((idx, path, snapshot.cursor_x, snapshot.cursor_y));
    }

    write_manifest(&out_dir, &frames)?;
    println!("wrote {} frames to {}", frames.len(), out_dir.display());
    Ok(())
}

fn frame_png_name(idx: usize) -> String {
    let mut name = String::with_capacity("frame-000.png".len());
    name.push_str("frame-");
    if idx < 100 {
        name.push('0');
    }
    if idx < 10 {
        name.push('0');
    }
    write!(name, "{idx}.png").expect("write to string");
    name
}

fn write_manifest(out_dir: &Path, frames: &[(usize, PathBuf, u16, u16)]) -> anyhow::Result<()> {
    let mut manifest = String::new();
    manifest.push_str("{\n  \"kind\": \"kittui-ghostty-vt-timelapse\",\n  \"frame_count\": ");
    write!(manifest, "{}", frames.len()).expect("write to string");
    manifest.push_str(",\n  \"frames\": [\n  ");
    for (i, (idx, path, cursor_x, cursor_y)) in frames.iter().enumerate() {
        if i > 0 {
            manifest.push_str(",\n  ");
        }
        manifest.push_str("{\"index\":");
        write!(manifest, "{idx}").expect("write to string");
        manifest.push_str(",\"path\":");
        write!(manifest, "{:?}", path.display().to_string()).expect("write to string");
        manifest.push_str(",\"cursor_x\":");
        write!(manifest, "{cursor_x}").expect("write to string");
        manifest.push_str(",\"cursor_y\":");
        write!(manifest, "{cursor_y}").expect("write to string");
        manifest.push('}');
    }
    manifest.push_str("\n  ]\n}\n");
    std::fs::write(out_dir.join("manifest.json"), manifest)?;
    Ok(())
}
