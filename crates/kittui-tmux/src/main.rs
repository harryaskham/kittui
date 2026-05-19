//! `kittui-tmux` binary.
//!
//! Reads `tmux list-panes -F` from stdin (or a file) and writes the
//! kittui escape stream that repaints the pane separators as kittui
//! chrome to stdout. Intended to be invoked from a tmux hook:
//!
//! ```sh
//! tmux list-panes -F '#{pane_id} #{pane_left} #{pane_top} #{pane_width} #{pane_height}' \
//!   | kittui-tmux > /dev/tty
//! ```
//!
//! Hosts wanting different chrome can pass flags or pipe a JSON
//! configuration via `--config`.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use clap::Parser;

use kittui::{RendererKind, Rgba, Runtime};
use kittui_tmux::{compose_pane_chrome, parse_list_panes, ComposeOptions};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Read pane list from this file instead of stdin.
    #[arg(long)]
    file: Option<PathBuf>,
    /// Border color (CSS hex).
    #[arg(long, default_value = "#00d8ff")]
    border_color: String,
    /// Border width in pixels.
    #[arg(long, default_value_t = 1.5)]
    border_width: f32,
    /// Corner radius in pixels.
    #[arg(long, default_value_t = 6.0)]
    corner_radius: f32,
    /// Cache directory override.
    #[arg(long, env = "KITTUI_CACHE_DIR")]
    cache_dir: Option<PathBuf>,
    /// Renderer: cpu (default) or gpu.
    #[arg(long, default_value = "cpu")]
    renderer: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let input = if let Some(path) = &cli.file {
        std::fs::read_to_string(path)?
    } else {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        buf
    };
    let panes = parse_list_panes(&input)?;
    if panes.is_empty() {
        return Ok(());
    }

    let renderer = match cli.renderer.as_str() {
        "gpu" => RendererKind::Gpu,
        "auto" => RendererKind::Auto,
        _ => RendererKind::Cpu,
    };
    let mut builder = Runtime::builder().renderer(renderer);
    if let Some(dir) = cli.cache_dir.as_ref() {
        builder = builder.cache_dir(dir.clone());
    }
    let runtime = builder.build()?;

    let options = ComposeOptions {
        border_color: Rgba::parse(&cli.border_color)?,
        border_width_px: cli.border_width,
        corner_radius_px: cli.corner_radius,
    };
    let output = compose_pane_chrome(&runtime, &panes, &options);
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(output.bytes.as_bytes())?;
    eprintln!(
        "kittui-tmux: composed {} pane chrome scenes ({} bytes)",
        output.placements,
        output.bytes.len()
    );
    Ok(())
}
