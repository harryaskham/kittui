use std::path::PathBuf;

use kittui_ghostty_vt::{render_snapshot_preview_png, GhosttyVtTerminal, PreviewOptions};

fn main() -> anyhow::Result<()> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/kittui-ghostty-vt-preview.png"));
    let mut term = GhosttyVtTerminal::new(48, 10, 200)?;
    term.write(
        b"kittui-ghostty headless preview\n\
          powered by portable libghostty-vt\n\
          \x1b[32mVT state is Ghostty-owned\x1b[0m\n\
          \x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m styles extracted\n\
          kittui/kittwm can render the surface\n",
    );
    let snapshot = term.render_snapshot()?;
    let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
    std::fs::write(&out, png)?;
    println!("wrote {}", out.display());
    Ok(())
}
