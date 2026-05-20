//! `kittui` CLI.
//!
//! Affordance subcommands (box, gradient, panel, image, compose, place,
//! cache, probe) are intentionally thin wrappers that build a `Scene` from
//! flags and forward to the `kittui::Runtime`. Library users wanting to
//! script kittui from a shell should reach for this binary; library users
//! wanting fine-grained control should use the Rust crate directly.

mod config;

use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use config::{
    BoxFlagValues, ConfigLayers, GlobalConfig, GlobalFlagValues, GlowFlagValues,
    GradientFlagValues, RendererArg, ResolvedBoxConfig, ResolvedGlowConfig, ResolvedGradientConfig,
};
use kittui::{
    scene::{background_linear, background_solid, glow_layer, rounded_rect},
    Animation, CellRect, CellSize, Direction, Layer, PhaseCurve, Rgba, Runtime, Scene,
    TerminalInfo,
};
use kittui_core::node::{Corners, Node, StrokeAlign};
use kittui_core::paint::Paint;
use kittui_core::Stroke;

#[derive(Parser)]
#[command(name = "kittui", version, about = "kitty graphics for TUIs")]
struct Cli {
    /// Cache directory override.
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Renderer backend (`auto`, `cpu`, or `gpu`).
    #[arg(long, value_enum)]
    renderer: Option<RendererArg>,

    /// Number of columns in the host terminal (for `%` resolution).
    #[arg(long)]
    terminal_cols: Option<u16>,

    /// Number of rows in the host terminal (for `%` resolution).
    #[arg(long)]
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
    #[arg(short, long)]
    x: Option<u16>,
    #[arg(short, long)]
    y: Option<u16>,
    /// Width in cells or as a percentage (`100%`).
    #[arg(short = 'w', long)]
    width: Option<String>,
    /// Height in cells or as a percentage (`100%`).
    #[arg(short = 'h', long)]
    height: Option<String>,
    /// Foreground / border color.
    #[arg(long)]
    fg: Option<String>,
    /// Background color.
    #[arg(long)]
    bg: Option<String>,
    /// Corner radius in pixels.
    #[arg(long)]
    radius: Option<f32>,
    /// Border width in pixels.
    #[arg(long)]
    border: Option<f32>,
    /// Animate with a pulsing glow: `frames@cycle_ms` (e.g. `8@800`).
    #[arg(long)]
    animate: Option<String>,
}

#[derive(clap::Args)]
struct GradientArgs {
    #[arg(short = 'w', long)]
    width: Option<String>,
    #[arg(short = 'h', long)]
    height: Option<String>,
    #[arg(long)]
    left: Option<String>,
    #[arg(long)]
    right: Option<String>,
    #[arg(long, value_enum)]
    direction: Option<DirectionArg>,
}

#[derive(clap::Args)]
struct GlowArgs {
    #[arg(short = 'w', long)]
    width: Option<String>,
    #[arg(short = 'h', long)]
    height: Option<String>,
    #[arg(long)]
    color: Option<String>,
    #[arg(long)]
    intensity: Option<f32>,
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

impl DirectionArg {
    fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().replace('_', "-").as_str() {
            "horizontal" => Ok(Self::Horizontal),
            "vertical" => Ok(Self::Vertical),
            "diagonal" => Ok(Self::Diagonal),
            other => Err(anyhow!(
                "invalid gradient direction {other:?}; expected horizontal, vertical, or diagonal"
            )),
        }
    }
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

fn build_runtime(global: &GlobalConfig) -> Result<Runtime> {
    let terminal = TerminalInfo {
        columns: Some(global.terminal_cols.value),
        rows: Some(global.terminal_rows.value),
        ..TerminalInfo::default()
    };
    Ok(Runtime::builder()
        .renderer(global.renderer.value.into())
        .cache_dir(global.cache_dir.value.clone())
        .terminal(terminal)
        .build()?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let layers = ConfigLayers::load()?;
    let global = layers.resolve_global(GlobalFlagValues {
        cache_dir: cli.cache_dir.clone(),
        renderer: cli.renderer,
        terminal_cols: cli.terminal_cols,
        terminal_rows: cli.terminal_rows,
        json: cli.json,
    });
    let runtime = build_runtime(&global)?;
    match &cli.cmd {
        Cmd::Box(args) => {
            let config = layers.resolve_box(BoxFlagValues {
                x: args.x,
                y: args.y,
                width: args.width.clone(),
                height: args.height.clone(),
                fg: args.fg.clone(),
                bg: args.bg.clone(),
                radius: args.radius,
                border: args.border,
                animate: args.animate.clone(),
            });
            run_box(&global, &runtime, &config)
        }
        Cmd::Gradient(args) => {
            let config = layers.resolve_gradient(GradientFlagValues {
                width: args.width.clone(),
                height: args.height.clone(),
                left: args.left.clone(),
                right: args.right.clone(),
                direction: args.direction.map(|d| match d {
                    DirectionArg::Horizontal => "horizontal".to_string(),
                    DirectionArg::Vertical => "vertical".to_string(),
                    DirectionArg::Diagonal => "diagonal".to_string(),
                }),
            });
            run_gradient(&global, &runtime, &config)
        }
        Cmd::Glow(args) => {
            let config = layers.resolve_glow(GlowFlagValues {
                width: args.width.clone(),
                height: args.height.clone(),
                color: args.color.clone(),
                intensity: args.intensity,
            });
            run_glow(&global, &runtime, &config)
        }
        Cmd::Compose(args) => run_compose(&global, &runtime, args),
        Cmd::Cache(sub) => run_cache(&global, &layers, sub),
        Cmd::Probe => run_probe(&global),
    }
}

fn run_box(global: &GlobalConfig, runtime: &Runtime, args: &ResolvedBoxConfig) -> Result<()> {
    let cols = resolve_size(&args.width.value, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height.value, global.terminal_rows.value)?;
    let cell = CellSize::default();
    let footprint = CellRect::new(args.x.value, args.y.value, cols, rows);
    let bg = Rgba::parse(&args.bg.value)?;
    let fg = Rgba::parse(&args.fg.value)?;
    let rect = footprint.to_pixels(cell);
    let mut layers = vec![
        background_solid(footprint, cell, bg),
        rounded_rect(rect, bg, fg, args.border.value, args.radius.value),
    ];
    let animation = if let Some(spec) = args.animate.value.as_deref() {
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
    emit(global, runtime, &scene, Some(args.source_json()))
}

fn run_gradient(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &ResolvedGradientConfig,
) -> Result<()> {
    let cols = resolve_size(&args.width.value, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height.value, global.terminal_rows.value)?;
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, rows);
    let direction = DirectionArg::parse(&args.direction.value)?;
    let scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![background_linear(
            footprint,
            cell,
            direction.into(),
            Rgba::parse(&args.left.value)?,
            Rgba::parse(&args.right.value)?,
        )],
        animation: None,
    };
    emit(global, runtime, &scene, Some(args.source_json()))
}

fn run_glow(global: &GlobalConfig, runtime: &Runtime, args: &ResolvedGlowConfig) -> Result<()> {
    let cols = resolve_size(&args.width.value, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height.value, global.terminal_rows.value)?;
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
                        color: Rgba::parse(&args.color.value)?,
                    },
                }),
                corners: Corners::uniform(6.0),
            }),
            glow_layer(rect, Rgba::parse(&args.color.value)?, args.intensity.value),
        ],
        animation: None,
    };
    emit(global, runtime, &scene, Some(args.source_json()))
}

fn run_compose(global: &GlobalConfig, runtime: &Runtime, args: &ComposeArgs) -> Result<()> {
    let bytes = std::fs::read(&args.path)?;
    let scene: Scene = serde_json::from_slice(&bytes)?;
    emit(global, runtime, &scene, None)
}

fn run_cache(global: &GlobalConfig, layers: &ConfigLayers, sub: &CacheCmd) -> Result<()> {
    let dir = global.cache_dir.value.clone();
    match sub {
        CacheCmd::Info => {
            let cache = kittui_cache::Cache::open(&dir)?;
            let stats = cache.stats()?;
            let probe = cache.read_probe()?;
            if global.json.value {
                let payload = serde_json::json!({
                    "root": dir.display().to_string(),
                    "scene_bytes": stats.scene_bytes,
                    "scene_count": stats.scene_count,
                    "image_bytes": stats.image_bytes,
                    "budget_bytes": cache.config().budget_bytes,
                    "grace_secs": cache.config().grace_secs,
                    "probe": probe,
                    "config_sources": { "global": global.source_json() },
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
            let budget = layers.resolve_cache_budget(*budget);
            let config = match budget.value {
                Some(b) => kittui_cache::CacheConfig {
                    budget_bytes: b,
                    grace_secs: kittui_cache::DEFAULT_GRACE_SECS,
                },
                None => kittui_cache::CacheConfig::default(),
            };
            let cache = kittui_cache::Cache::open_with_config(&dir, config)?;
            let report = cache.gc()?;
            if global.json.value {
                let payload = serde_json::json!({
                    "removed_entries": report.removed_entries,
                    "reclaimed_bytes": report.reclaimed_bytes,
                    "skipped_grace": report.skipped_grace,
                    "config_sources": {
                        "global": global.source_json(),
                        "cache": { "budget": budget.source },
                    },
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

fn run_probe(global: &GlobalConfig) -> Result<()> {
    let probe = serde_json::json!({
        "supports_kitty": true,
        "supports_unicode_placeholders": true,
        "renderer": global.renderer.value.to_string(),
        "version": env!("CARGO_PKG_VERSION"),
        "config_sources": { "global": global.source_json() },
    });
    println!("{}", serde_json::to_string_pretty(&probe)?);
    Ok(())
}

fn emit(
    global: &GlobalConfig,
    runtime: &Runtime,
    scene: &Scene,
    command_sources: Option<serde_json::Value>,
) -> Result<()> {
    let placement = runtime.place(scene)?;
    if global.json.value {
        let payload = serde_json::json!({
            "image_id": format!("0x{:08x}", placement.image_id),
            "footprint": placement.footprint,
            "upload_bytes": placement.upload.len(),
            "embed": placement.embed,
            "config_sources": {
                "global": global.source_json(),
                "command": command_sources,
            },
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
