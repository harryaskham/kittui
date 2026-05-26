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
        let path = out_dir.join(format!("frame-{idx:03}.png"));
        std::fs::write(&path, png)?;
        frames.push((idx, path, snapshot.cursor_x, snapshot.cursor_y));
    }

    write_manifest(&out_dir, &frames)?;
    println!("wrote {} frames to {}", frames.len(), out_dir.display());
    Ok(())
}

fn write_manifest(out_dir: &Path, frames: &[(usize, PathBuf, u16, u16)]) -> anyhow::Result<()> {
    let files = frames
        .iter()
        .map(|(idx, path, cursor_x, cursor_y)| {
            format!(
                "{{\"index\":{idx},\"path\":{:?},\"cursor_x\":{cursor_x},\"cursor_y\":{cursor_y}}}",
                path.display().to_string()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n  ");
    std::fs::write(
        out_dir.join("manifest.json"),
        format!(
            "{{\n  \"kind\": \"kittui-ghostty-vt-timelapse\",\n  \"frame_count\": {},\n  \"frames\": [\n  {}\n  ]\n}}\n",
            frames.len(), files
        ),
    )?;
    Ok(())
}
