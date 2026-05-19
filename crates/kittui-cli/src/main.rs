//! `kittui` CLI.
//!
//! Affordance subcommands (box, gradient, panel, image, compose, place,
//! cache, probe) are intentionally thin wrappers that build a `Scene` from
//! flags and forward to the `kittui::Runtime`. Library users wanting to
//! script kittui from a shell should reach for this binary; library users
//! wanting fine-grained control should use the Rust crate directly.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use kittui::{
    scene::{background_linear, background_solid, glow_layer, rounded_rect},
    Animation, CellRect, CellSize, Direction, Layer, PhaseCurve, Rgba, RendererKind, Runtime,
    Scene,
};
use kittui_core::node::{Corners, Node, StrokeAlign};
use kittui_core::paint::Paint;
use kittui_core::Stroke;

#[derive(Parser)]
#[command(name = "kittui", version, about = "kitty graphics for TUIs")]
struct Cli {
    /// Cache directory override.
    #[arg(long, env = "KITTUI_CACHE_DIR")]
    cache_dir: Option<PathBuf>,

    /// Number of columns in the host terminal (for `%` resolution).
    #[arg(long, env = "COLUMNS")]
    terminal_cols: Option<u16>,

    /// Number of rows in the host terminal (for `%` resolution).
    #[arg(long, env = "LINES")]
    terminal_rows: Option<u16>,

    /// Emit JSON describing the placement instead of raw escapes.
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render a box (filled, stroked, rounded) at the given footprint.
    Box(BoxArgs),
    /// Render a linear gradient strip.
    Gradient(GradientArgs),
    /// Render a glow layer.
    Glow(GlowArgs),
    /// Compose a scene from a JSON file.
    Compose(ComposeArgs),
    /// Cache management subcommands.
    #[command(subcommand)]
    Cache(CacheCmd),
    /// Probe terminal capabilities.
    Probe,
}

#[derive(Subcommand)]
enum CacheCmd {
    /// Print cache directory + stats.
    Info,
    /// Force an eviction pass.
    Gc {
        /// Optional byte budget override for this run.
        #[arg(long)]
        budget: Option<u64>,
    },
    /// Remove every cached scene and image.
    Clear,
}

#[derive(clap::Args)]
struct BoxArgs {
    #[arg(short, long, default_value_t = 0)]
    x: u16,
    #[arg(short, long, default_value_t = 0)]
    y: u16,
    /// Width in cells or as a percentage (`100%`).
    #[arg(short = 'w', long, default_value = "40")]
    width: String,
    /// Height in cells or as a percentage (`100%`).
    #[arg(short = 'h', long, default_value = "8")]
    height: String,
    /// Foreground / border color.
    #[arg(long, default_value = "#00d8ff")]
    fg: String,
    /// Background color.
    #[arg(long, default_value = "#08111fcc")]
    bg: String,
    /// Corner radius in pixels.
    #[arg(long, default_value_t = 6.0)]
    radius: f32,
    /// Border width in pixels.
    #[arg(long, default_value_t = 1.5)]
    border: f32,
    /// Animate with a pulsing glow: `frames@cycle_ms` (e.g. `8@800`).
    #[arg(long)]
    animate: Option<String>,
}

#[derive(clap::Args)]
struct GradientArgs {
    #[arg(short = 'w', long, default_value = "100%")]
    width: String,
    #[arg(short = 'h', long, default_value = "1")]
    height: String,
    #[arg(long, default_value = "#00d8ff")]
    left: String,
    #[arg(long, default_value = "#b48cff")]
    right: String,
    #[arg(long, value_enum, default_value_t = DirectionArg::Horizontal)]
    direction: DirectionArg,
}

#[derive(clap::Args)]
struct GlowArgs {
    #[arg(short = 'w', long, default_value = "40")]
    width: String,
    #[arg(short = 'h', long, default_value = "8")]
    height: String,
    #[arg(long, default_value = "#00d8ff")]
    color: String,
    #[arg(long, default_value_t = 0.6)]
    intensity: f32,
}

#[derive(clap::Args)]
struct ComposeArgs {
    /// Path to a JSON file describing a `kittui::Scene`.
    path: PathBuf,
}

#[derive(Copy, Clone, clap::ValueEnum)]
enum DirectionArg {
    Horizontal,
    Vertical,
    Diagonal,
}

impl From<DirectionArg> for Direction {
    fn from(value: DirectionArg) -> Self {
        match value {
            DirectionArg::Horizontal => Direction::Horizontal,
            DirectionArg::Vertical => Direction::Vertical,
            DirectionArg::Diagonal => Direction::Diagonal,
        }
    }
}

fn resolve_size(input: &str, axis: u16) -> Result<u16> {
    if let Some(percent) = input.strip_suffix('%') {
        let pct: f32 = percent.parse()?;
        let value = (axis as f32 * pct / 100.0).round();
        return Ok(value.max(1.0) as u16);
    }
    Ok(input.parse()?)
}

fn build_runtime(cli: &Cli) -> Result<Runtime> {
    let mut builder = Runtime::builder().renderer(RendererKind::Cpu);
    if let Some(path) = &cli.cache_dir {
        builder = builder.cache_dir(path.clone());
    }
    Ok(builder.build()?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let runtime = build_runtime(&cli)?;
    let Cli { cmd, .. } = &cli;
    match cmd {
        Cmd::Box(args) => run_box(&cli, &runtime, args),
        Cmd::Gradient(args) => run_gradient(&cli, &runtime, args),
        Cmd::Glow(args) => run_glow(&cli, &runtime, args),
        Cmd::Compose(args) => run_compose(&cli, &runtime, args),
        Cmd::Cache(sub) => run_cache(&cli, sub),
        Cmd::Probe => run_probe(&cli),
    }
}

fn run_box(cli: &Cli, runtime: &Runtime, args: &BoxArgs) -> Result<()> {
    let cols = resolve_size(&args.width, cli.terminal_cols.unwrap_or(80))?;
    let rows = resolve_size(&args.height, cli.terminal_rows.unwrap_or(24))?;
    let cell = CellSize::default();
    let footprint = CellRect::new(args.x, args.y, cols, rows);
    let bg = Rgba::parse(&args.bg)?;
    let fg = Rgba::parse(&args.fg)?;
    let rect = footprint.to_pixels(cell);
    let mut layers = vec![
        background_solid(footprint, cell, bg),
        rounded_rect(rect, bg, fg, args.border, args.radius),
    ];
    let animation = if let Some(spec) = args.animate.as_deref() {
        let (frames, cycle) = spec
            .split_once('@')
            .ok_or_else(|| anyhow!("--animate expects `frames@cycle_ms`"))?;
        let anim = Animation {
            frames: frames.parse()?,
            cycle_ms: cycle.parse()?,
            curve: PhaseCurve::Pulse { harmonics: 0 },
            loops: 0,
        };
        layers.push(glow_layer(rect, fg, 0.6));
        Some(anim)
    } else {
        None
    };
    let scene = Scene {
        footprint,
        cell_size: cell,
        layers,
        animation,
    };
    emit(cli, runtime, &scene)
}

fn run_gradient(cli: &Cli, runtime: &Runtime, args: &GradientArgs) -> Result<()> {
    let cols = resolve_size(&args.width, cli.terminal_cols.unwrap_or(80))?;
    let rows = resolve_size(&args.height, cli.terminal_rows.unwrap_or(24))?;
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, rows);
    let scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![background_linear(
            footprint,
            cell,
            args.direction.into(),
            Rgba::parse(&args.left)?,
            Rgba::parse(&args.right)?,
        )],
        animation: None,
    };
    emit(cli, runtime, &scene)
}

fn run_glow(cli: &Cli, runtime: &Runtime, args: &GlowArgs) -> Result<()> {
    let cols = resolve_size(&args.width, cli.terminal_cols.unwrap_or(80))?;
    let rows = resolve_size(&args.height, cli.terminal_rows.unwrap_or(24))?;
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, rows);
    let rect = footprint.to_pixels(cell);
    let scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![
            Layer::anon(Node::Rect {
                rect,
                fill: Paint::Solid {
                    color: Rgba::rgba(0x05, 0x0a, 0x14, 0xee),
                },
                stroke: Some(Stroke {
                    align: StrokeAlign::Inside,
                    width_px: 1.0,
                    paint: Paint::Solid {
                        color: Rgba::parse(&args.color)?,
                    },
                }),
                corners: Corners::uniform(6.0),
            }),
            glow_layer(rect, Rgba::parse(&args.color)?, args.intensity),
        ],
        animation: None,
    };
    emit(cli, runtime, &scene)
}

fn run_compose(cli: &Cli, runtime: &Runtime, args: &ComposeArgs) -> Result<()> {
    let bytes = std::fs::read(&args.path)?;
    let scene: Scene = serde_json::from_slice(&bytes)?;
    emit(cli, runtime, &scene)
}

fn run_cache(cli: &Cli, sub: &CacheCmd) -> Result<()> {
    let dir = cli
        .cache_dir
        .clone()
        .unwrap_or_else(kittui::scene::default_cache_dir);
    match sub {
        CacheCmd::Info => {
            let cache = kittui_cache::Cache::open(&dir)?;
            let stats = cache.stats()?;
            let probe = cache.read_probe()?;
            if cli.json {
                let payload = serde_json::json!({
                    "root": dir.display().to_string(),
                    "scene_bytes": stats.scene_bytes,
                    "scene_count": stats.scene_count,
                    "image_bytes": stats.image_bytes,
                    "budget_bytes": cache.config().budget_bytes,
                    "grace_secs": cache.config().grace_secs,
                    "probe": probe,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("root:          {}", dir.display());
                println!(
                    "scenes:        {} entries, {} bytes",
                    stats.scene_count, stats.scene_bytes
                );
                println!("images:        {} bytes", stats.image_bytes);
                println!("budget:        {} bytes", cache.config().budget_bytes);
                println!("grace:         {} seconds", cache.config().grace_secs);
                if let Some(probe) = probe {
                    println!(
                        "probe:         {} (gpu={}, ssim={:?})",
                        probe.gpu_status,
                        probe.gpu_adapter.as_deref().unwrap_or("-"),
                        probe.gpu_parity_ssim
                    );
                }
            }
        }
        CacheCmd::Gc { budget } => {
            let config = match budget {
                Some(b) => kittui_cache::CacheConfig {
                    budget_bytes: *b,
                    grace_secs: kittui_cache::DEFAULT_GRACE_SECS,
                },
                None => kittui_cache::CacheConfig::default(),
            };
            let cache = kittui_cache::Cache::open_with_config(&dir, config)?;
            let report = cache.gc()?;
            if cli.json {
                let payload = serde_json::json!({
                    "removed_entries": report.removed_entries,
                    "reclaimed_bytes": report.reclaimed_bytes,
                    "skipped_grace": report.skipped_grace,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!(
                    "reclaimed {} bytes across {} entries (skipped {} within grace)",
                    report.reclaimed_bytes, report.removed_entries, report.skipped_grace
                );
            }
        }
        CacheCmd::Clear => {
            let cache = kittui_cache::Cache::open(&dir)?;
            cache.clear()?;
            println!("cleared {}", dir.display());
        }
    }
    Ok(())
}

fn run_probe(_cli: &Cli) -> Result<()> {
    let probe = serde_json::json!({
        "supports_kitty": true,
        "supports_unicode_placeholders": true,
        "renderer": "cpu",
        "version": env!("CARGO_PKG_VERSION"),
    });
    println!("{}", serde_json::to_string_pretty(&probe)?);
    Ok(())
}

fn emit(cli: &Cli, runtime: &Runtime, scene: &Scene) -> Result<()> {
    let placement = runtime.place(scene)?;
    if cli.json {
        let payload = serde_json::json!({
            "image_id": format!("0x{:08x}", placement.image_id),
            "footprint": placement.footprint,
            "upload_bytes": placement.upload.len(),
            "embed": placement.embed,
        });
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(serde_json::to_string_pretty(&payload)?.as_bytes())?;
        handle.write_all(b"\n")?;
    } else {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(placement.upload.as_bytes())?;
        handle.write_all(placement.placement.as_bytes())?;
        handle.write_all(placement.embed.as_bytes())?;
    }
    Ok(())
}
