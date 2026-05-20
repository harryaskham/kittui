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

    /// Transport override (`direct`, `tmux`, `file`, `memory`). Default: auto-detect.
    #[arg(long)]
    transport: Option<String>,

    /// Number of columns in the host terminal (for `%` resolution).
    #[arg(long)]
    terminal_cols: Option<u16>,

    /// Number of rows in the host terminal (for `%` resolution).
    #[arg(long)]
    terminal_rows: Option<u16>,

    /// Emit JSON describing the placement instead of raw escapes.
    #[arg(long)]
    json: bool,

    /// Print only the upload escape bytes.
    #[arg(long, group = "channels")]
    upload_only: bool,

    /// Print only the placement escape bytes.
    #[arg(long, group = "channels")]
    placement_only: bool,

    /// Print only the embed placeholder grid.
    #[arg(long, group = "channels")]
    embed_only: bool,

    /// Build the scene + side effects but do not write any bytes.
    #[arg(long)]
    dry_run: bool,

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
    /// Render an image from a path/bytes through Node::Image.
    Image(ImageArgs),
    /// Cache management subcommands.
    #[command(subcommand)]
    Cache(CacheCmd),
    /// Probe terminal capabilities.
    Probe(ProbeArgs),
    /// Walk the full kitty graphics protocol surface and emit labelled output.
    Proof(ProofArgs),
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

#[derive(clap::Args, Clone)]
struct ImageArgs {
    /// Path to a PNG or JPEG image.
    #[arg(long)]
    src: PathBuf,
    /// Width in cells.
    #[arg(short = 'w', long, default_value_t = 20)]
    width: u16,
    /// Height in cells.
    #[arg(short = 'h', long, default_value_t = 8)]
    height: u16,
    /// Fit mode: contain, cover, stretch, none.
    #[arg(long, default_value = "contain")]
    fit: String,
    /// Optional multiplicative tint (e.g. "#ff0000").
    #[arg(long)]
    tint: Option<String>,
}

#[derive(clap::Args, Clone)]
struct ProbeArgs {
    /// Invalidate the cached probe.json and re-detect.
    #[arg(long)]
    force: bool,
}

#[derive(clap::Args, Clone)]
struct ProofArgs {
    /// Emit the raw escape bytes to stdout in addition to the labelled report.
    #[arg(long)]
    emit: bool,
    /// Restrict the matrix to a single section name (substring match).
    #[arg(long)]
    only: Option<String>,
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

fn build_runtime(global: &GlobalConfig, transport_override: Option<&str>) -> Result<Runtime> {
    let mut terminal = TerminalInfo {
        columns: Some(global.terminal_cols.value),
        rows: Some(global.terminal_rows.value),
        ..TerminalInfo::detect()
    };
    if let Some(t) = transport_override {
        terminal.transport = match t.to_ascii_lowercase().as_str() {
            "direct" => kittui_core::terminal::Transport::Direct,
            "tmux" | "tmux_passthrough" => kittui_core::terminal::Transport::TmuxPassthrough,
            "file" => kittui_core::terminal::Transport::File,
            "memory" | "shm" | "shared" => kittui_core::terminal::Transport::Memory,
            other => return Err(anyhow!("unknown transport {other:?}")),
        };
    }
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
    let runtime = build_runtime(&global, cli.transport.as_deref())?;
    let emit_mode = EmitMode {
        upload_only: cli.upload_only,
        placement_only: cli.placement_only,
        embed_only: cli.embed_only,
        dry_run: cli.dry_run,
    };
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
            run_box(&global, &runtime, &config, emit_mode)
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
            run_gradient(&global, &runtime, &config, emit_mode)
        }
        Cmd::Glow(args) => {
            let config = layers.resolve_glow(GlowFlagValues {
                width: args.width.clone(),
                height: args.height.clone(),
                color: args.color.clone(),
                intensity: args.intensity,
            });
            run_glow(&global, &runtime, &config, emit_mode)
        }
        Cmd::Compose(args) => run_compose(&global, &runtime, args, emit_mode),
        Cmd::Image(args) => run_image(&global, &runtime, args, emit_mode),
        Cmd::Cache(sub) => run_cache(&global, &layers, sub),
        Cmd::Probe(args) => run_probe(&global, args),
        Cmd::Proof(args) => run_proof(&global, args),
    }
}

fn run_box(global: &GlobalConfig, runtime: &Runtime, args: &ResolvedBoxConfig, mode: EmitMode) -> Result<()> {
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
    emit_with_mode(global, runtime, &scene, Some(args.source_json()), mode)
}

fn run_gradient(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &ResolvedGradientConfig,
    mode: EmitMode,
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
    emit_with_mode(global, runtime, &scene, Some(args.source_json()), mode)
}

fn run_glow(global: &GlobalConfig, runtime: &Runtime, args: &ResolvedGlowConfig, mode: EmitMode) -> Result<()> {
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
    emit_with_mode(global, runtime, &scene, Some(args.source_json()), mode)
}

fn run_compose(global: &GlobalConfig, runtime: &Runtime, args: &ComposeArgs, mode: EmitMode) -> Result<()> {
    let bytes = std::fs::read(&args.path)?;
    let scene: Scene = serde_json::from_slice(&bytes)?;
    emit_with_mode(global, runtime, &scene, None, mode)
}

fn run_image(global: &GlobalConfig, runtime: &Runtime, args: &ImageArgs, mode: EmitMode) -> Result<()> {
    use kittui_core::node::{Fit, ImageRef};
    let fit = match args.fit.to_ascii_lowercase().as_str() {
        "contain" => Fit::Contain,
        "cover" => Fit::Cover,
        "stretch" => Fit::Stretch,
        "none" => Fit::None,
        other => return Err(anyhow!("invalid --fit {other:?}")),
    };
    let tint = match args.tint.as_deref() {
        Some(s) => Some(Rgba::parse(s)?),
        None => None,
    };
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, args.width, args.height);
    let rect = footprint.to_pixels(cell);
    let scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![Layer::anon(Node::Image {
            rect,
            src: ImageRef::Path {
                path: args.src.to_string_lossy().into_owned(),
            },
            fit,
            tint,
        })],
        animation: None,
    };
    emit_with_mode(global, runtime, &scene, None, mode)
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

fn run_probe(global: &GlobalConfig, args: &ProbeArgs) -> Result<()> {
    let cache_root = global.cache_dir.value.clone();
    if args.force {
        let probe_path = cache_root.join("probe.json");
        let _ = std::fs::remove_file(&probe_path);
    }
    let probe = serde_json::json!({
        "supports_kitty": true,
        "supports_unicode_placeholders": true,
        "renderer": global.renderer.value.to_string(),
        "version": env!("CARGO_PKG_VERSION"),
        "config_sources": { "global": global.source_json() },
        "force_invalidated": args.force,
    });
    println!("{}", serde_json::to_string_pretty(&probe)?);
    Ok(())
}

fn run_proof(global: &GlobalConfig, args: &ProofArgs) -> Result<()> {
    use kittui::scene::{background_solid, rounded_rect};
    use kittui_kitty::{
        delete, delete_placement, placement_command, placement_command_ex, placeholder_text,
        upload_animation, upload_still, upload_still_ex, PlacementOptions, Quiet, SubcellOffset,
        UploadMedium,
    };
    use kittui_core::terminal::Transport;

    // Build a single small still scene and one tiny animation through the
    // CPU renderer so the proof commands carry real PNG bytes.
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, 8, 3);
    let rect = footprint.to_pixels(cell);
    let bg = Rgba::parse("#08111fcc")?;
    let fg = Rgba::parse("#00d8ff")?;
    let still_scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![
            background_solid(footprint, cell, bg),
            rounded_rect(rect, bg, fg, 1.5, 6.0),
        ],
        animation: None,
    };
    let anim_scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![
            background_solid(footprint, cell, bg),
            rounded_rect(rect, bg, fg, 1.5, 6.0),
        ],
        animation: Some(Animation {
            frames: 3,
            cycle_ms: 600,
            curve: PhaseCurve::Pulse { harmonics: 0 },
            loops: 0,
        }),
    };
    let renderer = kittui_render_cpu::render_still(&still_scene)?;
    let still_png = renderer.png;
    let anim = kittui_render_cpu::render_animation(&anim_scene)?;
    let frames = anim.frames;
    let delays: Vec<u32> = anim.frame_delays_ms;

    let mut sections: Vec<(String, String)> = Vec::new();
    let mut add = |label: &str, body: String| {
        sections.push((label.to_string(), body));
    };

    // 1) Direct transport, default quiet (q=2).
    add(
        "upload still + unicode placement (Direct, q=2)",
        format!(
            "{}{}{}",
            upload_still(0x00112233, &still_png, Transport::Direct),
            placement_command(0x00112233, footprint, Transport::Direct),
            placeholder_text(0x00112233, footprint),
        ),
    );

    // 2) Tmux passthrough transport.
    add(
        "upload still + unicode placement (TmuxPassthrough)",
        format!(
            "{}{}{}",
            upload_still(0x00112233, &still_png, Transport::TmuxPassthrough),
            placement_command(0x00112233, footprint, Transport::TmuxPassthrough),
            placeholder_text(0x00112233, footprint),
        ),
    );

    // 3) File-medium upload, plus placement.
    let tmp = std::env::temp_dir().join("kittui-proof.png");
    std::fs::write(&tmp, &still_png)?;
    add(
        "upload still via File medium",
        format!(
            "{}{}{}",
            upload_still_ex(
                0x4400aa00,
                UploadMedium::File { path: &tmp },
                Quiet::SuppressAll,
                Transport::Direct,
            ),
            placement_command(0x4400aa00, footprint, Transport::Direct),
            placeholder_text(0x4400aa00, footprint),
        ),
    );

    // 4) Shared-memory medium upload (name only; terminal would shm_open).
    add(
        "upload still via SharedMemory medium",
        upload_still_ex(
            0x55005500,
            UploadMedium::SharedMemory {
                name: "/kittui-proof",
            },
            Quiet::SuppressAll,
            Transport::Direct,
        ),
    );

    // 5) Absolute (non-placeholder) placement.
    let abs_opts = PlacementOptions::absolute();
    add(
        "absolute placement (no unicode placeholder)",
        format!(
            "{}{}",
            upload_still(0x66006600, &still_png, Transport::Direct),
            placement_command_ex(0x66006600, footprint, &abs_opts, Transport::Direct),
        ),
    );

    // 6) Placement id + subcell offset.
    let p_opts = PlacementOptions {
        placement_id: Some(7),
        offset: SubcellOffset { x_px: 4, y_px: 2 },
        quiet: Quiet::SuppressAll,
        unicode_placeholder: true,
        z_index: 1,
    };
    add(
        "placement with id=7, X=4, Y=2, z=1",
        format!(
            "{}{}{}",
            upload_still(0x77007700, &still_png, Transport::Direct),
            placement_command_ex(0x77007700, footprint, &p_opts, Transport::Direct),
            placeholder_text(0x77007700, footprint),
        ),
    );

    // 7) Animation: 3 frames, real PNG bytes.
    add(
        "animated upload + placement (3 frames)",
        format!(
            "{}{}{}",
            upload_animation(0x88008800, &frames, &delays, 0, Transport::Direct),
            placement_command(0x88008800, footprint, Transport::Direct),
            placeholder_text(0x88008800, footprint),
        ),
    );

    // 8) Delete: by image id, then by placement id.
    add(
        "delete image / delete placement",
        format!(
            "{}{}",
            delete(0x77007700, Transport::Direct),
            delete_placement(0x77007700, 7, Transport::Direct),
        ),
    );

    // 9) HiDPI cell-size override: same scene at 16x32 cells.
    let hidpi_cell = CellSize::new(16, 32);
    let hidpi_footprint = CellRect::new(0, 0, 4, 2);
    let hidpi_rect = hidpi_footprint.to_pixels(hidpi_cell);
    let hidpi_scene = Scene {
        footprint: hidpi_footprint,
        cell_size: hidpi_cell,
        layers: vec![
            background_solid(hidpi_footprint, hidpi_cell, bg),
            rounded_rect(hidpi_rect, bg, fg, 2.0, 8.0),
        ],
        animation: None,
    };
    let hidpi_png = kittui_render_cpu::render_still(&hidpi_scene)?.png;
    add(
        "HiDPI 16x32 cell override",
        format!(
            "{}{}{}",
            upload_still(0x99009900, &hidpi_png, Transport::Direct),
            placement_command(0x99009900, hidpi_footprint, Transport::Direct),
            placeholder_text(0x99009900, hidpi_footprint),
        ),
    );

    // Filtering and emission.
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if global.json.value {
        let payload: Vec<_> = sections
            .iter()
            .filter(|(label, _)| args.only.as_deref().map(|s| label.contains(s)).unwrap_or(true))
            .map(|(label, body)| {
                serde_json::json!({
                    "label": label,
                    "bytes_len": body.len(),
                    "hex_prefix": body.as_bytes().iter().take(48).map(|b| format!("{:02x}", b)).collect::<String>(),
                })
            })
            .collect();
        writeln!(handle, "{}", serde_json::to_string_pretty(&payload)?)?;
        return Ok(());
    }
    for (label, body) in &sections {
        if let Some(filter) = args.only.as_deref() {
            if !label.contains(filter) {
                continue;
            }
        }
        writeln!(handle, "\x1b[1m== {label} ==\x1b[0m")?;
        if args.emit {
            handle.write_all(body.as_bytes())?;
            writeln!(handle)?;
        } else {
            // Default: print labelled hex prefix so it is safe to view in any terminal.
            let prefix: String = body.as_bytes().iter().take(48).map(|b| format!("{:02x}", b)).collect();
            writeln!(handle, "  bytes_len={}", body.len())?;
            writeln!(handle, "  hex_prefix={}", prefix)?;
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

#[derive(Copy, Clone, Debug, Default)]
struct EmitMode {
    upload_only: bool,
    placement_only: bool,
    embed_only: bool,
    dry_run: bool,
}

fn emit(
    global: &GlobalConfig,
    runtime: &Runtime,
    scene: &Scene,
    command_sources: Option<serde_json::Value>,
) -> Result<()> {
    emit_with_mode(global, runtime, scene, command_sources, EmitMode::default())
}

fn emit_with_mode(
    global: &GlobalConfig,
    runtime: &Runtime,
    scene: &Scene,
    command_sources: Option<serde_json::Value>,
    mode: EmitMode,
) -> Result<()> {
    let placement = runtime.place(scene)?;
    if mode.dry_run {
        // Always JSON shape for dry-run so callers can compare.
        let payload = serde_json::json!({
            "dry_run": true,
            "image_id": format!("0x{:08x}", placement.image_id),
            "footprint": placement.footprint,
            "upload_bytes": placement.upload.len(),
            "placement_bytes": placement.placement.len(),
            "embed_bytes": placement.embed.len(),
            "config_sources": {
                "global": global.source_json(),
                "command": command_sources,
            },
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
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
        let any_filter = mode.upload_only || mode.placement_only || mode.embed_only;
        if !any_filter || mode.upload_only {
            handle.write_all(placement.upload.as_bytes())?;
        }
        if !any_filter || mode.placement_only {
            handle.write_all(placement.placement.as_bytes())?;
        }
        if !any_filter || mode.embed_only {
            handle.write_all(placement.embed.as_bytes())?;
        }
    }
    Ok(())
}
