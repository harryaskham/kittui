//! `kittui` CLI.
//!
//! Affordance subcommands (box, gradient, panel, chip, divider, title-bar,
//! image, compose, place, cache, probe) are intentionally thin wrappers that build a `Scene` from
//! flags and forward to the `kittui::Runtime`. Library users wanting to
//! script kittui from a shell should reach for this binary; library users
//! wanting fine-grained control should use the Rust crate directly.

mod config;

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use base64::Engine;
use clap::{Parser, Subcommand, ValueEnum};

use config::{
    BoxFlagValues, ConfigLayers, GlobalConfig, GlobalFlagValues, GlowFlagValues,
    GradientFlagValues, RendererArg, ResolvedBoxConfig, ResolvedGlowConfig, ResolvedGradientConfig,
};
use kittui::{
    scene::{background_linear, background_solid, rounded_rect},
    Animation, CellRect, CellSize, Direction, Layer, PhaseCurve, Rgba, Runtime, Scene,
    TerminalInfo, STANDARD_ANIMATION_FPS, STANDARD_ANIMATION_FRAMES,
};
use kittui_affordances::{
    chip_chrome, divider_chrome, panel_chrome, parse_nord_inline_color, title_chrome,
    InlineChipColors, InlineStyle, InlineTheme, Palette, PanelOptions, Tone,
};
use kittui_cli::update::{self as cli_update, UpdateAction, UpdateOptions};
use kittui_core::node::{BlendMode, Corners, Node, StrokeAlign};
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
    #[arg(long, global = true)]
    json: bool,

    /// Include upload/placement/embed string channels in JSON output.
    #[arg(long, global = true)]
    json_bytes: bool,

    /// Print only the upload escape bytes.
    #[arg(long, global = true, group = "channels")]
    upload_only: bool,

    /// Print only the placement escape bytes.
    #[arg(long, global = true, group = "channels")]
    placement_only: bool,

    /// Print only the embed placeholder grid.
    #[arg(long, global = true, group = "channels")]
    embed_only: bool,

    /// Build and print the generated `kittui::Scene` JSON instead of rendering.
    #[arg(long, global = true)]
    scene_json: bool,

    /// Build the scene + side effects but do not write any bytes.
    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render inline one-line components for prompts/statuslines.
    #[command(subcommand)]
    Inline(InlineCmd),
    /// Render a box (filled, stroked, rounded) at the given footprint.
    Box(BoxArgs),
    /// Render a linear gradient strip.
    Gradient(GradientArgs),
    /// Render a glow layer.
    Glow(GlowArgs),
    /// Render a tonal panel chrome scene.
    Panel(PanelArgs),
    /// Render a pill-shaped chip chrome scene.
    Chip(ChipArgs),
    /// Render a single-row divider chrome scene.
    Divider(DividerArgs),
    /// Render the reusable kittwm window chrome scene.
    WmChrome(WmChromeArgs),
    /// Render a kittwm SESSION_JSON manifest as window chrome scenes.
    WmSession(WmSessionArgs),
    /// Render a title-bar chrome scene.
    TitleBar(TitleBarArgs),
    /// Compose a scene from a JSON file.
    Compose(ComposeArgs),
    /// Render a scene JSON file to PNG bytes without terminal placement.
    Render(RenderArgs),
    /// Render an image from a path/bytes through Node::Image.
    Image(ImageArgs),
    /// Re-place an already-uploaded image id at a terminal footprint.
    Place(PlaceArgs),
    /// Delete an uploaded image or one placement from the terminal.
    Delete(DeleteArgs),
    /// Cache management subcommands.
    #[command(subcommand)]
    Cache(CacheCmd),
    /// Probe terminal capabilities.
    Probe(ProbeArgs),
    /// Walk the full kitty graphics protocol surface and emit labelled output.
    Proof(ProofArgs),
    /// Download and install a released kittui binary.
    Update(UpdateArgs),
    /// Expose shared kittui tools over MCP stdio.
    Mcp,
}

#[derive(clap::Args, Clone, Debug)]
struct UpdateArgs {
    /// Print local install/staged status instead of updating.
    #[arg(long)]
    status: bool,
    /// Check GitHub releases instead of updating.
    #[arg(long)]
    check: bool,
    /// Override owner/repo release source.
    #[arg(long)]
    repository: Option<String>,
    /// Override install directory.
    #[arg(long = "install-dir")]
    install_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum InlineCmd {
    /// Render a one-line text chip for shell prompts or tmux statuslines.
    Chip(InlineChipArgs),
    /// Render a compact badge for labels, modes, or counts.
    Badge(InlineChipArgs),
    /// Render a prompt/status segment.
    Segment(InlineChipArgs),
    /// Render a one-line divider/rule.
    Divider(InlineDividerArgs),
    /// Render several inline components in one process invocation.
    Row(InlineRowArgs),
    /// Print copy/paste prompt, statusline, and fallback examples.
    Examples,
}

#[derive(clap::Args, Clone)]
struct InlineChipArgs {
    /// Text inside the chip.
    #[arg(long)]
    text: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = InlineFormatArg::Kitty)]
    format: InlineFormatArg,
    /// Tone palette used by fallback formats.
    #[arg(long, value_enum, default_value_t = ToneArg::Assistant)]
    tone: ToneArg,
    /// Inline graphics theme.
    #[arg(long, value_enum, default_value_t = InlineThemeArg::Nord)]
    theme: InlineThemeArg,
    /// Inline graphics style.
    #[arg(long, value_enum, default_value_t = InlineStyleArg::Glass)]
    style: InlineStyleArg,
    /// Fill color override as hex or theme index/name.
    #[arg(long)]
    bg_color: Option<String>,
    /// Border color override as hex or theme index/name.
    #[arg(long)]
    border_color: Option<String>,
    /// Text color override as hex or theme index/name.
    #[arg(long)]
    fg_color: Option<String>,
    /// Spaces of horizontal padding around the text.
    #[arg(long, default_value_t = 1)]
    padding: usize,
    /// Inline kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args, Clone, Copy, Debug, PartialEq, Eq)]
struct InlineAnimationArgs {
    /// Render all kitty animation frames up-front and let the terminal loop them.
    #[arg(long)]
    animated: bool,
    /// Animation frames per second when --animated is set.
    #[arg(long, default_value_t = 60)]
    fps: u16,
    /// Frames in one perfectly looping animation period when --animated is set.
    #[arg(long, default_value_t = 180)]
    frames: u16,
}

impl Default for InlineAnimationArgs {
    fn default() -> Self {
        Self {
            animated: false,
            fps: STANDARD_ANIMATION_FPS,
            frames: STANDARD_ANIMATION_FRAMES,
        }
    }
}

impl InlineAnimationArgs {
    fn scene_animation(self) -> Option<Animation> {
        self.scene_animation_when(self.animated)
    }

    fn scene_animation_when(self, enabled: bool) -> Option<Animation> {
        if !enabled {
            return None;
        }
        Some(Animation::pulse_fps(self.frames, self.fps))
    }
}

#[derive(clap::Args, Clone)]
struct InlineDividerArgs {
    /// Divider width in terminal cells.
    #[arg(long, default_value_t = 8)]
    width: u16,
    /// Visible divider glyph for text/prompt fallback width.
    #[arg(long, default_value = "─")]
    glyph: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = InlineFormatArg::Kitty)]
    format: InlineFormatArg,
    /// Tone palette used by fallback formats.
    #[arg(long, value_enum, default_value_t = ToneArg::Assistant)]
    tone: ToneArg,
    /// Inline graphics theme.
    #[arg(long, value_enum, default_value_t = InlineThemeArg::Nord)]
    theme: InlineThemeArg,
    /// Inline graphics style.
    #[arg(long, value_enum, default_value_t = InlineStyleArg::Glass)]
    style: InlineStyleArg,
    /// Rule color override as hex or theme index/name.
    #[arg(long)]
    color: Option<String>,
    /// Inline kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args, Clone)]
struct InlineRowArgs {
    /// Ordered row item: chip:TEXT, badge:TEXT, segment:TEXT, divider:WIDTH, or divider:WIDTH:GLYPH.
    #[arg(long = "item", required = true)]
    items: Vec<String>,
    /// Output format for all row items.
    #[arg(long, value_enum, default_value_t = InlineFormatArg::Kitty)]
    format: InlineFormatArg,
    /// Tone palette used by fallback formats.
    #[arg(long, value_enum, default_value_t = ToneArg::Assistant)]
    tone: ToneArg,
    /// Inline graphics theme.
    #[arg(long, value_enum, default_value_t = InlineThemeArg::Nord)]
    theme: InlineThemeArg,
    /// Inline graphics style.
    #[arg(long, value_enum, default_value_t = InlineStyleArg::Glass)]
    style: InlineStyleArg,
    /// Spaces of horizontal padding around text items.
    #[arg(long, default_value_t = 1)]
    padding: usize,
    /// Visible spaces between row items.
    #[arg(long, default_value_t = 0)]
    gap: usize,
    /// Inline kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
enum InlineThemeArg {
    Nord,
}

impl From<InlineThemeArg> for InlineTheme {
    fn from(value: InlineThemeArg) -> Self {
        match value {
            InlineThemeArg::Nord => InlineTheme::Nord,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
enum InlineStyleArg {
    Glass,
    Chrome,
    Metal,
    Neon,
}

impl From<InlineStyleArg> for InlineStyle {
    fn from(value: InlineStyleArg) -> Self {
        match value {
            InlineStyleArg::Glass => InlineStyle::Glass,
            InlineStyleArg::Chrome => InlineStyle::Chrome,
            InlineStyleArg::Metal => InlineStyle::Metal,
            InlineStyleArg::Neon => InlineStyle::Neon,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
enum InlineFormatArg {
    /// Kitty graphics background with inline width-bearing placeholder/text.
    Kitty,
    /// Kitty graphics for zsh prompts; nonprinting escapes are wrapped in `%{...%}`.
    PromptZsh,
    /// Kitty graphics for bash prompts; nonprinting escapes are wrapped in `\\[...\\]`.
    PromptBash,
    /// ASCII/plain fallback.
    Plain,
    /// 24-bit ANSI styled text fallback.
    Ansi,
    /// tmux statusline style syntax fallback.
    Tmux,
}

impl InlineFormatArg {
    fn uses_kitty_graphics(self) -> bool {
        matches!(
            self,
            InlineFormatArg::Kitty | InlineFormatArg::PromptZsh | InlineFormatArg::PromptBash
        )
    }

    fn prompt_wrapper(self) -> PromptWrapper {
        match self {
            InlineFormatArg::PromptZsh => PromptWrapper::Zsh,
            InlineFormatArg::PromptBash => PromptWrapper::Bash,
            _ => PromptWrapper::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            InlineFormatArg::Kitty => "kitty",
            InlineFormatArg::PromptZsh => "prompt-zsh",
            InlineFormatArg::PromptBash => "prompt-bash",
            InlineFormatArg::Plain => "plain",
            InlineFormatArg::Ansi => "ansi",
            InlineFormatArg::Tmux => "tmux",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PromptWrapper {
    None,
    Zsh,
    Bash,
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
#[command(disable_help_flag = true)]
struct ImageArgs {
    /// Path to a PNG or JPEG image; use `-` to read bytes from stdin.
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
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args, Clone)]
#[command(disable_help_flag = true)]
struct PlaceArgs {
    /// Existing kitty image id, decimal or 0x-prefixed hex.
    #[arg(long)]
    id: String,
    /// X column to place at.
    #[arg(short, long)]
    x: u16,
    /// Y row to place at.
    #[arg(short, long)]
    y: u16,
    /// Width in cells.
    #[arg(short = 'w', long)]
    cols: u16,
    /// Height in cells.
    #[arg(short = 'h', long)]
    rows: u16,
}

#[derive(clap::Args, Clone)]
struct DeleteArgs {
    /// Existing kitty image id, decimal or 0x-prefixed hex.
    #[arg(long)]
    id: String,
    /// Optional placement id to delete instead of the whole image.
    #[arg(long)]
    placement_id: Option<u32>,
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
    /// Milliseconds to dwell on each emitted section (default: 1500).
    #[arg(long)]
    dwell_ms: Option<u64>,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
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
    /// Animate with a pulsing glow: `frames@cycle_ms` (legacy form, e.g. `8@800`).
    #[arg(long)]
    animate: Option<String>,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
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
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
struct GlowArgs {
    #[arg(short = 'w', long)]
    width: Option<String>,
    #[arg(short = 'h', long)]
    height: Option<String>,
    #[arg(long)]
    color: Option<String>,
    #[arg(long)]
    intensity: Option<f32>,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
struct PanelArgs {
    /// Panel tone palette.
    #[arg(long, value_enum, default_value_t = ToneArg::Assistant)]
    tone: ToneArg,
    /// Width in cells or as a percentage (`100%`).
    #[arg(short = 'w', long)]
    width: String,
    /// Height in cells or as a percentage (`100%`).
    #[arg(short = 'h', long)]
    height: String,
    /// Add native kitty-side pulsing glow animation (legacy alias for --animated).
    #[arg(long)]
    animate: bool,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
struct ChipArgs {
    /// Width in cells or as a percentage (`100%`).
    #[arg(short = 'w', long)]
    width: String,
    /// Height in cells.
    #[arg(short = 'h', long, default_value = "1")]
    height: String,
    /// Background color.
    #[arg(long)]
    bg: String,
    /// Border color.
    #[arg(long)]
    border: String,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
struct DividerArgs {
    /// Width in cells or as a percentage (`100%`).
    #[arg(short = 'w', long)]
    width: String,
    /// Left gradient color.
    #[arg(long)]
    left: String,
    /// Right gradient color.
    #[arg(long)]
    right: String,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
struct WmSessionArgs {
    /// Path to a kittwm SESSION_JSON manifest; use `-` for stdin.
    path: PathBuf,
    /// Preview width in cells.
    #[arg(short = 'w', long)]
    width: String,
    /// Preview height in cells.
    #[arg(short = 'h', long)]
    height: String,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
struct WmChromeArgs {
    /// Width in cells or as a percentage (`100%`).
    #[arg(short = 'w', long)]
    width: String,
    /// Height in cells or as a percentage (`100%`).
    #[arg(short = 'h', long)]
    height: String,
    /// Window title used in layer labels.
    #[arg(long, default_value = "window")]
    title: String,
    /// Render focused chrome styling.
    #[arg(long)]
    focused: bool,
    /// Render floating chrome mode. Default is tiled.
    #[arg(long)]
    floating: bool,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(clap::Args)]
#[command(disable_help_flag = true)]
struct TitleBarArgs {
    /// Width in cells or as a percentage (`100%`).
    #[arg(short = 'w', long)]
    width: String,
    /// Height in cells.
    #[arg(short = 'h', long, default_value = "1")]
    height: String,
    /// Left gradient color.
    #[arg(long)]
    left: String,
    /// Right gradient color.
    #[arg(long)]
    right: String,
    /// Kitty-native animation options.
    #[command(flatten)]
    animation: InlineAnimationArgs,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ToneArg {
    Assistant,
    Tool,
    User,
}

impl From<ToneArg> for Tone {
    fn from(value: ToneArg) -> Self {
        match value {
            ToneArg::Assistant => Tone::Assistant,
            ToneArg::Tool => Tone::Tool,
            ToneArg::User => Tone::User,
        }
    }
}

#[derive(clap::Args)]
struct ComposeArgs {
    /// Path to a JSON file describing a `kittui::Scene`; use `-` for stdin.
    path: PathBuf,
    /// Override terminal placement X column without changing scene JSON.
    #[arg(long)]
    x: Option<u16>,
    /// Override terminal placement Y row without changing scene JSON.
    #[arg(long)]
    y: Option<u16>,
}

#[derive(clap::Args)]
struct RenderArgs {
    /// Path to JSON describing one `kittui::Scene` or an array of scenes; use `-` for stdin.
    path: PathBuf,
    /// Write a single-scene PNG to this path instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Directory for rendering a scene array to one PNG per scene.
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Write render metadata JSON to this path in addition to normal output.
    #[arg(long)]
    manifest: Option<PathBuf>,
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
    cli_update::maybe_apply_staged_update("kittui");
    let cli = Cli::parse();
    if let Cmd::Update(args) = &cli.cmd {
        return cli_update::run_update_command("kittui", &update_options(args, cli.json));
    }
    if let Cmd::Mcp = &cli.cmd {
        return cli_update::serve_update_mcp("kittui");
    }
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
        scene_json: cli.scene_json,
        json_bytes: cli.json_bytes,
        dry_run: cli.dry_run,
    };
    match &cli.cmd {
        Cmd::Inline(sub) => run_inline(&global, &runtime, sub, emit_mode),
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
            run_box(&global, &runtime, &config, args.animation, emit_mode)
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
            run_gradient(&global, &runtime, &config, args.animation, emit_mode)
        }
        Cmd::Glow(args) => {
            let config = layers.resolve_glow(GlowFlagValues {
                width: args.width.clone(),
                height: args.height.clone(),
                color: args.color.clone(),
                intensity: args.intensity,
            });
            run_glow(&global, &runtime, &config, args.animation, emit_mode)
        }
        Cmd::Panel(args) => run_panel(&global, &runtime, args, emit_mode),
        Cmd::Chip(args) => run_chip(&global, &runtime, args, emit_mode),
        Cmd::Divider(args) => run_divider(&global, &runtime, args, emit_mode),
        Cmd::WmChrome(args) => run_wm_chrome(&global, &runtime, args, emit_mode),
        Cmd::WmSession(args) => run_wm_session(&global, &runtime, args, emit_mode),
        Cmd::TitleBar(args) => run_title_bar(&global, &runtime, args, emit_mode),
        Cmd::Compose(args) => run_compose(&global, &runtime, args, emit_mode),
        Cmd::Render(args) => run_render(&global, &runtime, args, emit_mode),
        Cmd::Image(args) => run_image(&global, &runtime, args, emit_mode),
        Cmd::Place(args) => run_place(&global, &runtime, args, emit_mode),
        Cmd::Delete(args) => run_delete(&global, &runtime, args, emit_mode),
        Cmd::Cache(sub) => run_cache(&global, &layers, sub),
        Cmd::Probe(args) => run_probe(&global, args),
        Cmd::Proof(args) => run_proof(&global, args),
        Cmd::Update(_) | Cmd::Mcp => unreachable!("handled before runtime construction"),
    }
}

fn update_options(args: &UpdateArgs, json: bool) -> UpdateOptions {
    let action = if args.status {
        UpdateAction::Status
    } else if args.check {
        UpdateAction::Check
    } else {
        UpdateAction::Run
    };
    UpdateOptions {
        action,
        json,
        repository: args.repository.clone(),
        install_dir: args.install_dir.clone(),
    }
}

fn run_inline(
    global: &GlobalConfig,
    runtime: &Runtime,
    cmd: &InlineCmd,
    mode: EmitMode,
) -> Result<()> {
    match cmd {
        InlineCmd::Chip(args) => {
            run_inline_text_component(global, runtime, args, InlineTextComponent::Chip, mode)
        }
        InlineCmd::Badge(args) => {
            run_inline_text_component(global, runtime, args, InlineTextComponent::Badge, mode)
        }
        InlineCmd::Segment(args) => {
            run_inline_text_component(global, runtime, args, InlineTextComponent::Segment, mode)
        }
        InlineCmd::Divider(args) => run_inline_divider(global, runtime, args, mode),
        InlineCmd::Row(args) => run_inline_row(global, runtime, args, mode),
        InlineCmd::Examples => {
            print!("{}", inline_examples_text());
            Ok(())
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum InlineTextComponent {
    Chip,
    Badge,
    Segment,
}

impl InlineTextComponent {
    fn label(self) -> &'static str {
        match self {
            InlineTextComponent::Chip => "chip",
            InlineTextComponent::Badge => "badge",
            InlineTextComponent::Segment => "segment",
        }
    }

    fn radius(self, rect_height: f32) -> f32 {
        match self {
            InlineTextComponent::Chip => (rect_height / 2.0).max(1.0),
            InlineTextComponent::Badge => 4.0,
            InlineTextComponent::Segment => 2.0,
        }
    }
}

fn run_inline_text_component(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &InlineChipArgs,
    component: InlineTextComponent,
    mode: EmitMode,
) -> Result<()> {
    if args.format.uses_kitty_graphics() {
        return run_inline_chip_kitty(global, runtime, args, component, mode);
    }
    print!("{}", render_inline_text_component(args, component));
    Ok(())
}

fn run_inline_chip_kitty(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &InlineChipArgs,
    component: InlineTextComponent,
    mode: EmitMode,
) -> Result<()> {
    let colors = inline_chip_colors(args)?;
    let cols = inline_chip_cols(args);
    let scene = inline_chip_scene(
        cols,
        colors,
        component,
        args.style,
        args.animation.scene_animation(),
    );
    if mode.scene_json {
        println!("{}", serialize_scene_json(&scene)?);
        return Ok(());
    }
    let placement = runtime.place(&scene)?;
    let wrapper = args.format.prompt_wrapper();
    let upload = wrap_prompt_nonprinting(&placement.upload, wrapper);
    let embed = inline_chip_text_embed(&args.text, args.padding, colors.fg, wrapper);
    let inline_placement = wrap_prompt_nonprinting(
        &inline_background_placement(&placement, runtime.transport()),
        wrapper,
    );
    if mode.dry_run || global.json.value {
        let mut payload =
            placement_json_payload(global, &placement, None, mode.dry_run, mode.json_bytes);
        payload["inline_component"] = serde_json::json!(component.label());
        payload["inline_text"] = serde_json::json!(args.text);
        payload["inline_format"] = serde_json::json!(args.format.label());
        payload["upload_bytes"] = serde_json::json!(upload.len());
        payload["placement_bytes"] = serde_json::json!(inline_placement.len());
        payload["embed_bytes"] = serde_json::json!(embed.len());
        add_inline_animation_json(&mut payload, args.animation);
        if mode.json_bytes || mode.dry_run {
            payload["upload"] = serde_json::json!(upload);
            payload["placement"] = serde_json::json!(inline_placement);
            payload["embed"] = serde_json::json!(embed);
        }
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let any_filter = mode.upload_only || mode.placement_only || mode.embed_only;
    if !any_filter || mode.upload_only {
        handle.write_all(upload.as_bytes())?;
    }
    if !any_filter || mode.placement_only {
        handle.write_all(inline_placement.as_bytes())?;
    }
    if !any_filter || mode.embed_only {
        handle.write_all(embed.as_bytes())?;
    }
    Ok(())
}

fn inline_chip_colors(args: &InlineChipArgs) -> Result<InlineChipColors> {
    let fill = args
        .bg_color
        .as_deref()
        .map(parse_nord_inline_color)
        .transpose()?;
    let border = args
        .border_color
        .as_deref()
        .map(parse_nord_inline_color)
        .transpose()?;
    let fg = args
        .fg_color
        .as_deref()
        .map(parse_nord_inline_color)
        .transpose()?;
    Ok(
        InlineChipColors::resolve(args.theme.into(), args.style.into())
            .with_overrides(fill, border, fg),
    )
}

fn inline_chip_scene(
    cols: u16,
    colors: InlineChipColors,
    component: InlineTextComponent,
    style: InlineStyleArg,
    animation: Option<Animation>,
) -> Scene {
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, 1);
    let rect = footprint.to_pixels(cell);
    let radius = component.radius(rect.height as f32);
    let highlight_rect = kittui_core::geom::PxRect::new(
        rect.origin.0,
        rect.origin.1,
        rect.width,
        (rect.height / 2.0).max(1.0),
    );
    let mut layers = vec![
        Layer::anon(Node::Rect {
            rect,
            fill: Paint::Solid { color: colors.fill },
            stroke: Some(Stroke {
                align: StrokeAlign::Inside,
                width_px: 1.0,
                paint: Paint::Solid {
                    color: colors.border,
                },
            }),
            corners: Corners::uniform(radius),
        }),
        Layer::anon(Node::Rect {
            rect: highlight_rect,
            fill: Paint::Solid {
                color: colors.highlight,
            },
            stroke: None,
            corners: Corners::uniform(radius),
        }),
    ];
    if animation.is_some() {
        layers.extend(inline_style_effect_layers(rect, radius, style, colors));
    }
    Scene {
        footprint,
        cell_size: cell,
        layers,
        animation,
    }
}

fn inline_style_effect_layers(
    rect: kittui_core::geom::PxRect,
    radius: f32,
    style: InlineStyleArg,
    colors: InlineChipColors,
) -> Vec<Layer> {
    let glare = kittui_core::geom::PxRect::new(
        rect.origin.0,
        rect.origin.1,
        rect.width,
        (rect.height * 0.62).max(1.0),
    );
    let reflection = kittui_core::geom::PxRect::new(
        rect.origin.0 + rect.width * 0.12,
        rect.origin.1 + rect.height * 0.18,
        (rect.width * 0.76).max(1.0),
        (rect.height * 0.28).max(1.0),
    );
    let (label, center_x_frac, center_y_frac, radius_frac, color, intensity, extra) = match style {
        InlineStyleArg::Glass => (
            "inline-effect-glass-glare",
            0.22,
            0.05,
            1.6,
            Rgba(255, 255, 255, 118),
            0.46,
            Some(Node::Rect {
                rect: glare,
                fill: Paint::Solid {
                    color: Rgba(255, 255, 255, 34),
                },
                stroke: None,
                corners: Corners::uniform(radius),
            }),
        ),
        InlineStyleArg::Neon => (
            "inline-effect-neon-pulse",
            0.5,
            0.5,
            2.6,
            colors.border,
            0.72,
            None,
        ),
        InlineStyleArg::Metal => (
            "inline-effect-metal-reflection",
            0.78,
            0.22,
            2.0,
            Rgba(255, 255, 255, 96),
            0.42,
            Some(Node::Rect {
                rect: reflection,
                fill: Paint::Solid {
                    color: Rgba(255, 255, 255, 28),
                },
                stroke: None,
                corners: Corners::uniform((reflection.height / 2.0).max(1.0)),
            }),
        ),
        InlineStyleArg::Chrome => (
            "inline-effect-chrome-sheen",
            0.5,
            0.0,
            1.8,
            Rgba(255, 255, 255, 104),
            0.38,
            Some(Node::Rect {
                rect: glare,
                fill: Paint::Solid {
                    color: Rgba(255, 255, 255, 26),
                },
                stroke: None,
                corners: Corners::uniform(radius),
            }),
        ),
    };
    let glow = Node::Glow {
        rect,
        center_x_frac,
        center_y_frac,
        radius_frac,
        color,
        intensity,
    };
    let root = match extra {
        Some(extra) => Node::Composite {
            mode: BlendMode::Screen,
            children: vec![extra, glow],
        },
        None => glow,
    };
    vec![Layer::new(label, root)]
}

fn inline_divider_effect_layers(
    rect: kittui_core::geom::PxRect,
    color: Rgba,
    style: InlineStyleArg,
) -> Vec<Layer> {
    let colors = InlineChipColors {
        fill: Rgba(color.0, color.1, color.2, 42),
        border: color,
        highlight: Rgba(255, 255, 255, 64),
        fg: color,
    };
    inline_style_effect_layers(rect, (rect.height / 2.0).max(1.0), style, colors)
}

fn primitive_animation(
    legacy: Option<&str>,
    animation: InlineAnimationArgs,
) -> Result<Option<Animation>> {
    if let Some(spec) = legacy {
        let (frames, cycle) = spec
            .split_once('@')
            .ok_or_else(|| anyhow!("--animate expects `frames@cycle_ms`"))?;
        return Ok(Some(Animation::pulse(frames.parse()?, cycle.parse()?)));
    }
    Ok(animation.scene_animation())
}

fn add_affordance_animation(
    scene: &mut Scene,
    animation: Option<Animation>,
    color: Rgba,
    label: &str,
) {
    let Some(animation) = animation else {
        return;
    };
    let rect = scene.footprint.to_pixels(scene.cell_size);
    scene.layers.push(Layer::new(
        label,
        Node::Glow {
            rect,
            center_x_frac: 0.5,
            center_y_frac: 0.35,
            radius_frac: 2.0,
            color,
            intensity: 0.55,
        },
    ));
    scene.animation = Some(animation);
}

fn add_inline_animation_json(payload: &mut serde_json::Value, animation: InlineAnimationArgs) {
    payload["inline_animated"] = serde_json::json!(animation.animated);
    if let Some(scene_animation) = animation.scene_animation() {
        payload["inline_animation"] = serde_json::json!({
            "fps": animation.fps.max(1),
            "frames": scene_animation.frames,
            "cycle_ms": scene_animation.cycle_ms,
            "loops": scene_animation.loops,
        });
    }
}

fn inline_background_placement(
    placement: &kittui::Placement,
    transport: kittui_core::terminal::Transport,
) -> String {
    let mut options = kittui_kitty::PlacementOptions::absolute();
    options.z_index = -1;
    let mut out = kittui_kitty::placement_command_ex(
        placement.image_id,
        CellRect::new(0, 0, placement.footprint.cols, placement.footprint.rows),
        &options,
        transport,
    );
    out.push_str(&inline_cursor_back(placement.footprint.cols, transport));
    out
}

fn inline_cursor_back(cols: u16, _transport: kittui_core::terminal::Transport) -> String {
    // This must be normal terminal output, not kitty/tmux passthrough: tmux
    // needs to update its own cursor model before the visible text fallback is
    // printed over the z=-1 image placement.
    format!("\x1b[{}D", cols.max(1))
}

fn inline_chip_cols(args: &InlineChipArgs) -> u16 {
    inline_text_cols(&args.text, args.padding)
}

fn inline_text_cols(text: &str, padding: usize) -> u16 {
    (text.chars().count() + padding.saturating_mul(2)).max(1) as u16
}

fn inline_chip_text_embed(text: &str, padding: usize, fg: Rgba, wrapper: PromptWrapper) -> String {
    format!(
        "{}{}{}",
        wrap_prompt_nonprinting(&inline_chip_text_prefix(fg), wrapper),
        inline_chip_visible_text(text, padding),
        wrap_prompt_nonprinting(inline_chip_text_suffix(), wrapper),
    )
}

fn inline_chip_visible_text(text: &str, padding: usize) -> String {
    let pad = " ".repeat(padding);
    format!("{pad}{text}{pad}")
}

fn inline_chip_text_prefix(fg: Rgba) -> String {
    format!("\x1b[38;2;{};{};{}m", fg.0, fg.1, fg.2)
}

fn inline_chip_text_suffix() -> &'static str {
    "\x1b[39m"
}

fn wrap_prompt_nonprinting(text: &str, wrapper: PromptWrapper) -> String {
    match wrapper {
        PromptWrapper::None => text.to_string(),
        PromptWrapper::Zsh => format!("%{{{text}%}}"),
        PromptWrapper::Bash => format!("\\[{text}\\]"),
    }
}

fn inline_examples_text() -> &'static str {
    r##"kittui inline examples — prompt/statusline building blocks

Default kitty graphics chip:
  kittui inline chip --text "main"

zsh prompt-safe graphics chip:
  PROMPT='$(kittui inline chip --format prompt-zsh --text "dev") %~ %# '

bash prompt-safe graphics chip:
  PS1='$(kittui inline chip --format prompt-bash --text "dev") \w \$ '

tmux statusline fallback:
  tmux set -g status-left "$(kittui inline chip --format tmux --text '#S')"

Explicit text fallbacks:
  kittui inline chip --format plain --text "offline"
  kittui inline chip --format ansi --text "ready"

Theme/style knobs:
  kittui inline chip --text "deploy" --style neon
  kittui inline chip --text "warn" --style chrome --border-color orange

Native kitty animation:
  kittui inline chip --text "main" --animated
  kittui inline row --item chip:main --item divider:4 --animated --fps 60 --frames 180

Notes:
  - default `kitty` mode emits kitty graphics plus visible terminal text.
  - --animated uploads all frames once; default period is 180 frames at 60fps (3 seconds).
  - prompt modes wrap only nonprinting bytes; visible text remains width-bearing.
  - use explicit `tmux`, `plain`, or `ansi` fallback formats when graphics are not appropriate.
"##
}

#[cfg(test)]
fn render_inline_chip(args: &InlineChipArgs) -> String {
    render_inline_text_component(args, InlineTextComponent::Chip)
}

fn render_inline_text_component(args: &InlineChipArgs, component: InlineTextComponent) -> String {
    let label = inline_chip_visible_text(&args.text, args.padding);
    let palette = Palette::for_tone(args.tone.into());
    let plain = match component {
        InlineTextComponent::Chip => format!("[{label}]"),
        InlineTextComponent::Badge => format!("<{label}>"),
        InlineTextComponent::Segment => label.clone(),
    };
    match args.format {
        InlineFormatArg::Kitty
        | InlineFormatArg::PromptZsh
        | InlineFormatArg::PromptBash
        | InlineFormatArg::Plain => plain,
        InlineFormatArg::Ansi => format!(
            "\x1b[1;38;2;{};{};{};48;2;{};{};{}m{label}\x1b[0m",
            palette.rail.0,
            palette.rail.1,
            palette.rail.2,
            palette.bg_top.0,
            palette.bg_top.1,
            palette.bg_top.2,
        ),
        InlineFormatArg::Tmux => format!(
            "#[bold,fg={},bg={}]{}#[default]",
            rgba_hex(palette.rail),
            rgba_hex(palette.bg_top),
            tmux_escape(&label),
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InlineRowItem {
    Text {
        component: InlineTextComponent,
        text: String,
    },
    Divider {
        width: u16,
        glyph: String,
    },
}

fn run_inline_row(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &InlineRowArgs,
    mode: EmitMode,
) -> Result<()> {
    let items = parse_inline_row_items(&args.items)?;
    if !args.format.uses_kitty_graphics() {
        print!("{}", render_inline_row_fallback(&items, args)?);
        return Ok(());
    }
    let output = render_inline_row_output(runtime, args, &items)?;
    if mode.scene_json {
        println!("{}", serialize_scene_json(&output.scene)?);
        return Ok(());
    }
    if mode.dry_run || global.json.value {
        let mut payload = placement_json_payload(
            global,
            &output.placement,
            None,
            mode.dry_run,
            mode.json_bytes,
        );
        payload["inline_component"] = serde_json::json!("row");
        payload["inline_format"] = serde_json::json!(args.format.label());
        payload["inline_items"] = serde_json::json!(args.items);
        payload["upload_bytes"] = serde_json::json!(output.upload.len());
        payload["placement_bytes"] = serde_json::json!(output.inline_placement.len());
        payload["embed_bytes"] = serde_json::json!(output.embed.len());
        add_inline_animation_json(&mut payload, args.animation);
        if mode.json_bytes || mode.dry_run {
            payload["upload"] = serde_json::json!(output.upload);
            payload["placement"] = serde_json::json!(output.inline_placement);
            payload["embed"] = serde_json::json!(output.embed);
        }
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let any_filter = mode.upload_only || mode.placement_only || mode.embed_only;
    if !any_filter || mode.upload_only {
        handle.write_all(output.upload.as_bytes())?;
    }
    if !any_filter || mode.placement_only {
        handle.write_all(output.inline_placement.as_bytes())?;
    }
    if !any_filter || mode.embed_only {
        handle.write_all(output.embed.as_bytes())?;
    }
    Ok(())
}

struct InlineRowOutput {
    scene: Scene,
    placement: kittui::Placement,
    upload: String,
    inline_placement: String,
    embed: String,
}

fn render_inline_row_output(
    runtime: &Runtime,
    args: &InlineRowArgs,
    items: &[InlineRowItem],
) -> Result<InlineRowOutput> {
    let colors = InlineChipColors::resolve(args.theme.into(), args.style.into());
    let scene = inline_row_scene(items, args, colors, args.animation.scene_animation())?;
    let placement = runtime.place(&scene)?;
    let wrapper = args.format.prompt_wrapper();
    let upload = wrap_prompt_nonprinting(&placement.upload, wrapper);
    let inline_placement = wrap_prompt_nonprinting(
        &inline_background_placement(&placement, runtime.transport()),
        wrapper,
    );
    let embed = inline_row_embed(items, args, colors, wrapper)?;
    Ok(InlineRowOutput {
        scene,
        placement,
        upload,
        inline_placement,
        embed,
    })
}

fn parse_inline_row_items(values: &[String]) -> Result<Vec<InlineRowItem>> {
    values
        .iter()
        .map(|value| parse_inline_row_item(value))
        .collect()
}

fn parse_inline_row_item(value: &str) -> Result<InlineRowItem> {
    let (kind, rest) = value
        .split_once(':')
        .ok_or_else(|| anyhow!("inline row item must be kind:value, got {value:?}"))?;
    match kind {
        "chip" => Ok(InlineRowItem::Text {
            component: InlineTextComponent::Chip,
            text: rest.to_string(),
        }),
        "badge" => Ok(InlineRowItem::Text {
            component: InlineTextComponent::Badge,
            text: rest.to_string(),
        }),
        "segment" => Ok(InlineRowItem::Text {
            component: InlineTextComponent::Segment,
            text: rest.to_string(),
        }),
        "divider" => {
            let (width, glyph) = rest.split_once(':').unwrap_or((rest, "─"));
            Ok(InlineRowItem::Divider {
                width: width.parse()?,
                glyph: glyph.to_string(),
            })
        }
        other => Err(anyhow!(
            "unknown inline row item kind {other:?}; expected chip, badge, segment, or divider"
        )),
    }
}

fn inline_row_scene(
    items: &[InlineRowItem],
    args: &InlineRowArgs,
    colors: InlineChipColors,
    animation: Option<Animation>,
) -> Result<Scene> {
    let cell = CellSize::default();
    let gap = args.gap as u16;
    let cols = inline_row_cols(items, args.padding, gap);
    let mut layers = Vec::new();
    let mut cursor = 0u16;
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            cursor = cursor.saturating_add(gap);
        }
        match item {
            InlineRowItem::Text { component, text } => {
                let item_cols = inline_text_cols(text, args.padding);
                let mut scene = inline_chip_scene(item_cols, colors, *component, args.style, None);
                for layer in &mut scene.layers {
                    offset_layer_x(layer, cursor, cell);
                }
                layers.extend(scene.layers);
                cursor = cursor.saturating_add(item_cols);
            }
            InlineRowItem::Divider { width, .. } => {
                let mut scene = inline_divider_scene(*width, colors.border, args.style, None);
                for layer in &mut scene.layers {
                    offset_layer_x(layer, cursor, cell);
                }
                layers.extend(scene.layers);
                cursor = cursor.saturating_add((*width).max(1));
            }
        }
    }
    let footprint = CellRect::new(0, 0, cols.max(1), 1);
    if animation.is_some() {
        let rect = footprint.to_pixels(cell);
        layers.extend(inline_style_effect_layers(
            rect,
            (rect.height / 2.0).max(1.0),
            args.style,
            colors,
        ));
    }
    Ok(Scene {
        footprint,
        cell_size: cell,
        layers,
        animation,
    })
}

fn inline_row_cols(items: &[InlineRowItem], padding: usize, gap: u16) -> u16 {
    let item_cols: u16 = items
        .iter()
        .map(|item| match item {
            InlineRowItem::Text { text, .. } => inline_text_cols(text, padding),
            InlineRowItem::Divider { width, .. } => (*width).max(1),
        })
        .sum();
    item_cols.saturating_add(gap.saturating_mul(items.len().saturating_sub(1) as u16))
}

fn inline_row_embed(
    items: &[InlineRowItem],
    args: &InlineRowArgs,
    colors: InlineChipColors,
    wrapper: PromptWrapper,
) -> Result<String> {
    if !args.format.uses_kitty_graphics() {
        return render_inline_row_fallback(items, args);
    }
    let mut out = String::new();
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 && args.gap > 0 {
            out.push_str(&" ".repeat(args.gap));
        }
        match item {
            InlineRowItem::Text { text, .. } => {
                out.push_str(&inline_chip_text_embed(
                    text,
                    args.padding,
                    colors.fg,
                    wrapper,
                ));
            }
            InlineRowItem::Divider { width, glyph } => {
                let divider = InlineDividerArgs {
                    width: *width,
                    glyph: glyph.clone(),
                    format: args.format,
                    tone: args.tone,
                    theme: args.theme,
                    style: args.style,
                    color: None,
                    animation: InlineAnimationArgs::default(),
                };
                out.push_str(&inline_chip_text_embed(
                    &inline_divider_visible_text(&divider),
                    0,
                    colors.border,
                    wrapper,
                ));
            }
        }
    }
    Ok(out)
}

fn render_inline_row_fallback(items: &[InlineRowItem], args: &InlineRowArgs) -> Result<String> {
    let mut out = String::new();
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 && args.gap > 0 {
            out.push_str(&" ".repeat(args.gap));
        }
        match item {
            InlineRowItem::Text { component, text } => {
                let item_args = InlineChipArgs {
                    text: text.clone(),
                    format: args.format,
                    tone: args.tone,
                    theme: args.theme,
                    style: args.style,
                    bg_color: None,
                    border_color: None,
                    fg_color: None,
                    padding: args.padding,
                    animation: InlineAnimationArgs::default(),
                };
                out.push_str(&render_inline_text_component(&item_args, *component));
            }
            InlineRowItem::Divider { width, glyph } => {
                let item_args = InlineDividerArgs {
                    width: *width,
                    glyph: glyph.clone(),
                    format: args.format,
                    tone: args.tone,
                    theme: args.theme,
                    style: args.style,
                    color: None,
                    animation: InlineAnimationArgs::default(),
                };
                out.push_str(&render_inline_divider(&item_args)?);
            }
        }
    }
    Ok(out)
}

fn offset_layer_x(layer: &mut Layer, cells: u16, cell: CellSize) {
    let dx = cells as f32 * cell.width_px as f32;
    offset_node_x(&mut layer.root, dx);
}

fn offset_node_x(node: &mut Node, dx: f32) {
    match node {
        Node::Rect { rect, .. } | Node::Image { rect, .. } => {
            rect.origin.0 += dx;
        }
        Node::Group { children, .. } | Node::Composite { children, .. } => {
            for child in children {
                offset_node_x(child, dx);
            }
        }
        Node::Gradient { rect, .. }
        | Node::Glow { rect, .. }
        | Node::Scanlines { rect, .. }
        | Node::Shader { rect, .. }
        | Node::Clip { rect, .. } => {
            rect.origin.0 += dx;
        }
        Node::Mask { mask, child } => {
            offset_node_x(mask, dx);
            offset_node_x(child, dx);
        }
    }
}

fn run_inline_divider(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &InlineDividerArgs,
    mode: EmitMode,
) -> Result<()> {
    if !args.format.uses_kitty_graphics() {
        print!("{}", render_inline_divider(args)?);
        return Ok(());
    }
    let color = inline_divider_color(args)?;
    let scene = inline_divider_scene(
        args.width,
        color,
        args.style,
        args.animation.scene_animation(),
    );
    if mode.scene_json {
        println!("{}", serialize_scene_json(&scene)?);
        return Ok(());
    }
    let placement = runtime.place(&scene)?;
    let wrapper = args.format.prompt_wrapper();
    let upload = wrap_prompt_nonprinting(&placement.upload, wrapper);
    let inline_placement = wrap_prompt_nonprinting(
        &inline_background_placement(&placement, runtime.transport()),
        wrapper,
    );
    let embed = inline_chip_text_embed(&inline_divider_visible_text(args), 0, color, wrapper);
    if mode.dry_run || global.json.value {
        let mut payload =
            placement_json_payload(global, &placement, None, mode.dry_run, mode.json_bytes);
        payload["inline_component"] = serde_json::json!("divider");
        payload["inline_format"] = serde_json::json!(args.format.label());
        payload["upload_bytes"] = serde_json::json!(upload.len());
        payload["placement_bytes"] = serde_json::json!(inline_placement.len());
        payload["embed_bytes"] = serde_json::json!(embed.len());
        add_inline_animation_json(&mut payload, args.animation);
        if mode.json_bytes || mode.dry_run {
            payload["upload"] = serde_json::json!(upload);
            payload["placement"] = serde_json::json!(inline_placement);
            payload["embed"] = serde_json::json!(embed);
        }
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let any_filter = mode.upload_only || mode.placement_only || mode.embed_only;
    if !any_filter || mode.upload_only {
        handle.write_all(upload.as_bytes())?;
    }
    if !any_filter || mode.placement_only {
        handle.write_all(inline_placement.as_bytes())?;
    }
    if !any_filter || mode.embed_only {
        handle.write_all(embed.as_bytes())?;
    }
    Ok(())
}

fn inline_divider_color(args: &InlineDividerArgs) -> Result<Rgba> {
    let colors = InlineChipColors::resolve(args.theme.into(), args.style.into());
    Ok(match &args.color {
        Some(color) => parse_nord_inline_color(color)?,
        None => colors.border,
    })
}

fn inline_divider_scene(
    cols: u16,
    color: Rgba,
    style: InlineStyleArg,
    animation: Option<Animation>,
) -> Scene {
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols.max(1), 1);
    let rect = footprint.to_pixels(cell);
    let rule_height = 2.0_f32.min(rect.height.max(1.0));
    let mut layers = vec![Layer::anon(Node::Rect {
        rect: kittui_core::geom::PxRect::new(
            0.0,
            ((rect.height - rule_height) / 2.0).max(0.0),
            rect.width,
            rule_height,
        ),
        fill: Paint::Solid { color },
        stroke: None,
        corners: Corners::uniform(rule_height / 2.0),
    })];
    if animation.is_some() {
        layers.extend(inline_divider_effect_layers(rect, color, style));
    }
    Scene {
        footprint,
        cell_size: cell,
        layers,
        animation,
    }
}

fn inline_divider_visible_text(args: &InlineDividerArgs) -> String {
    let glyph = if args.glyph.is_empty() {
        "─"
    } else {
        &args.glyph
    };
    glyph
        .chars()
        .cycle()
        .take(args.width.max(1) as usize)
        .collect()
}

fn render_inline_divider(args: &InlineDividerArgs) -> Result<String> {
    let text = inline_divider_visible_text(args);
    let palette = Palette::for_tone(args.tone.into());
    Ok(match args.format {
        InlineFormatArg::Kitty
        | InlineFormatArg::PromptZsh
        | InlineFormatArg::PromptBash
        | InlineFormatArg::Plain => text,
        InlineFormatArg::Ansi => format!(
            "\x1b[1;38;2;{};{};{}m{text}\x1b[0m",
            palette.rail.0, palette.rail.1, palette.rail.2,
        ),
        InlineFormatArg::Tmux => format!(
            "#[bold,fg={}]{}#[default]",
            rgba_hex(palette.rail),
            tmux_escape(&text),
        ),
    })
}

fn rgba_hex(color: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", color.0, color.1, color.2)
}

fn tmux_escape(text: &str) -> String {
    text.replace('#', "##")
}

fn run_box(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &ResolvedBoxConfig,
    animation_args: InlineAnimationArgs,
    mode: EmitMode,
) -> Result<()> {
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
    let animation = primitive_animation(args.animate.value.as_deref(), animation_args)?;
    if animation.is_some() {
        layers.push(Layer::new(
            "primitive-box-animation",
            Node::Glow {
                rect,
                center_x_frac: 0.5,
                center_y_frac: 0.5,
                radius_frac: 1.5,
                color: fg,
                intensity: 0.6,
            },
        ));
    }
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
    animation_args: InlineAnimationArgs,
    mode: EmitMode,
) -> Result<()> {
    let cols = resolve_size(&args.width.value, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height.value, global.terminal_rows.value)?;
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, rows);
    let direction = DirectionArg::parse(&args.direction.value)?;
    let right = Rgba::parse(&args.right.value)?;
    let mut scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![background_linear(
            footprint,
            cell,
            direction.into(),
            Rgba::parse(&args.left.value)?,
            right,
        )],
        animation: animation_args.scene_animation(),
    };
    if scene.animation.is_some() {
        scene.layers.push(Layer::new(
            "primitive-gradient-animation",
            Node::Glow {
                rect: footprint.to_pixels(cell),
                center_x_frac: 0.75,
                center_y_frac: 0.5,
                radius_frac: 2.2,
                color: right,
                intensity: 0.45,
            },
        ));
    }
    emit_with_mode(global, runtime, &scene, Some(args.source_json()), mode)
}

fn run_glow(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &ResolvedGlowConfig,
    animation_args: InlineAnimationArgs,
    mode: EmitMode,
) -> Result<()> {
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
            Layer::new(
                "primitive-glow-animation",
                Node::Glow {
                    rect,
                    center_x_frac: 0.5,
                    center_y_frac: 0.5,
                    radius_frac: 0.5,
                    color: Rgba::parse(&args.color.value)?,
                    intensity: args.intensity.value,
                },
            ),
        ],
        animation: animation_args.scene_animation(),
    };
    emit_with_mode(global, runtime, &scene, Some(args.source_json()), mode)
}

fn run_panel(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &PanelArgs,
    mode: EmitMode,
) -> Result<()> {
    let cols = resolve_size(&args.width, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height, global.terminal_rows.value)?;
    let chrome = panel_chrome(args.tone.into(), &PanelOptions { animated: false });
    let area = ratatui::layout::Rect::new(0, 0, cols, rows);
    let mut scene = chrome
        .to_scene(area)
        .ok_or_else(|| anyhow!("panel chrome produced no scene for {cols}x{rows}"))?;
    let palette = Palette::for_tone(args.tone.into());
    add_affordance_animation(
        &mut scene,
        args.animation
            .scene_animation_when(args.animate || args.animation.animated),
        palette.glow,
        "affordance-panel-animation",
    );
    emit_with_mode(global, runtime, &scene, None, mode)
}

fn chrome_to_scene(chrome: ratakittui::Chrome, cols: u16, rows: u16, label: &str) -> Result<Scene> {
    chrome
        .to_scene(ratatui::layout::Rect::new(0, 0, cols, rows))
        .ok_or_else(|| anyhow!("{label} chrome produced no scene for {cols}x{rows}"))
}

fn run_chip(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &ChipArgs,
    mode: EmitMode,
) -> Result<()> {
    let cols = resolve_size(&args.width, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height, global.terminal_rows.value)?;
    let border = Rgba::parse(&args.border)?;
    let mut scene = chrome_to_scene(
        chip_chrome(Rgba::parse(&args.bg)?, border),
        cols,
        rows,
        "chip",
    )?;
    add_affordance_animation(
        &mut scene,
        args.animation.scene_animation(),
        border,
        "affordance-chip-animation",
    );
    emit_with_mode(global, runtime, &scene, None, mode)
}

fn run_divider(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &DividerArgs,
    mode: EmitMode,
) -> Result<()> {
    let cols = resolve_size(&args.width, global.terminal_cols.value)?;
    let left = Rgba::parse(&args.left)?;
    let right = Rgba::parse(&args.right)?;
    let mut scene = chrome_to_scene(divider_chrome(left, right), cols, 1, "divider")?;
    add_affordance_animation(
        &mut scene,
        args.animation.scene_animation(),
        right,
        "affordance-divider-animation",
    );
    emit_with_mode(global, runtime, &scene, None, mode)
}

fn run_wm_chrome(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &WmChromeArgs,
    mode: EmitMode,
) -> Result<()> {
    let cols = resolve_size(&args.width, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height, global.terminal_rows.value)?;
    let mut scene = wm_chrome_scene(cols, rows, args.focused, !args.floating, &args.title);
    add_affordance_animation(
        &mut scene,
        args.animation.scene_animation(),
        Rgba(0x88, 0xc0, 0xd0, 0xcc),
        "wm-chrome-animation",
    );
    emit_with_mode(global, runtime, &scene, None, mode)
}

fn run_wm_session(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &WmSessionArgs,
    mode: EmitMode,
) -> Result<()> {
    let cols = resolve_size(&args.width, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height, global.terminal_rows.value)?;
    let manifest = read_wm_session_manifest(&args.path)?;
    let mut scenes = wm_session_scenes(&manifest, cols, rows)?;
    for scene in &mut scenes {
        add_affordance_animation(
            scene,
            args.animation.scene_animation(),
            Rgba(0x88, 0xc0, 0xd0, 0xcc),
            "wm-session-animation",
        );
    }
    emit_scene_batch_with_mode(global, runtime, &scenes, mode)
}

fn wm_chrome_scene(cols: u16, rows: u16, focused: bool, tiled: bool, title: &str) -> Scene {
    wm_chrome_scene_at(0, 0, cols, rows, focused, tiled, title)
}

fn wm_chrome_scene_at(
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
    focused: bool,
    tiled: bool,
    title: &str,
) -> Scene {
    let cell = CellSize::default();
    let footprint = CellRect::new(x, y, cols, rows);
    let rect = footprint.to_pixels(cell);
    let layers = kittui_wm::chrome::WindowChromeTheme::default().layers(
        rect,
        &kittui_wm::chrome::WindowChromeState::new(focused, tiled, title),
    );
    Scene {
        footprint,
        cell_size: cell,
        layers,
        animation: None,
    }
}

#[derive(Debug, serde::Deserialize)]
struct WmSessionManifest {
    #[serde(default)]
    layout: Option<String>,
    #[serde(default)]
    panes: Vec<WmSessionPane>,
}

#[derive(Debug, serde::Deserialize)]
struct WmSessionPane {
    #[serde(default)]
    window: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default = "default_wm_session_weight")]
    weight: u16,
    #[serde(default)]
    focused: bool,
}

fn default_wm_session_weight() -> u16 {
    1
}

fn read_wm_session_manifest(path: &PathBuf) -> Result<WmSessionManifest> {
    let mut input = String::new();
    if path.as_os_str() == "-" {
        std::io::stdin().read_to_string(&mut input)?;
    } else {
        input = std::fs::read_to_string(path)?;
    }
    Ok(serde_json::from_str(&input)?)
}

fn wm_session_scenes(manifest: &WmSessionManifest, cols: u16, rows: u16) -> Result<Vec<Scene>> {
    if manifest.panes.is_empty() {
        return Err(anyhow!("wm-session manifest contains no panes"));
    }
    let layout = manifest
        .layout
        .as_deref()
        .unwrap_or("columns")
        .to_ascii_lowercase();
    if !matches!(layout.as_str(), "columns" | "rows" | "-") {
        return Err(anyhow!("wm-session layout must be columns or rows"));
    }
    let weights = manifest
        .panes
        .iter()
        .map(|pane| pane.weight.max(1))
        .collect::<Vec<_>>();
    let segments = if layout == "rows" {
        weighted_segments(rows, &weights)
    } else {
        weighted_segments(cols, &weights)
    };
    Ok(manifest
        .panes
        .iter()
        .enumerate()
        .map(|(idx, pane)| {
            let (offset, span) = segments[idx];
            let title = pane
                .title
                .as_deref()
                .or(pane.window.as_deref())
                .or(pane.command.as_deref())
                .unwrap_or("pane");
            if layout == "rows" {
                wm_chrome_scene_at(0, offset, cols, span, pane.focused, true, title)
            } else {
                wm_chrome_scene_at(offset, 0, span, rows, pane.focused, true, title)
            }
        })
        .collect())
}

fn weighted_segments(total: u16, weights: &[u16]) -> Vec<(u16, u16)> {
    if weights.is_empty() {
        return Vec::new();
    }
    let total_u32 = u32::from(total.max(1));
    let sum = weights
        .iter()
        .map(|w| u32::from((*w).max(1)))
        .sum::<u32>()
        .max(1);
    let mut used = 0u16;
    weights
        .iter()
        .enumerate()
        .map(|(idx, weight)| {
            let remaining = total.saturating_sub(used);
            let span = if idx + 1 == weights.len() {
                remaining.max(1)
            } else {
                ((total_u32 * u32::from((*weight).max(1))) / sum)
                    .max(1)
                    .min(u32::from(
                        remaining.saturating_sub((weights.len() - idx - 1) as u16),
                    )) as u16
            };
            let segment = (used, span);
            used = used.saturating_add(span);
            segment
        })
        .collect()
}

fn run_title_bar(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &TitleBarArgs,
    mode: EmitMode,
) -> Result<()> {
    let cols = resolve_size(&args.width, global.terminal_cols.value)?;
    let rows = resolve_size(&args.height, global.terminal_rows.value)?;
    let left = Rgba::parse(&args.left)?;
    let right = Rgba::parse(&args.right)?;
    let mut scene = chrome_to_scene(title_chrome(left, right), cols, rows, "title-bar")?;
    add_affordance_animation(
        &mut scene,
        args.animation.scene_animation(),
        right,
        "affordance-title-bar-animation",
    );
    emit_with_mode(global, runtime, &scene, None, mode)
}

fn run_compose(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &ComposeArgs,
    mode: EmitMode,
) -> Result<()> {
    match read_compose_input(&args.path)? {
        ComposeInput::Single(scene) => {
            let footprint = compose_placement_footprint(&scene, args.x, args.y);
            emit_scene_at_with_mode(global, runtime, &scene, footprint, None, mode)
        }
        ComposeInput::Batch(scenes) => {
            emit_scene_batch_at_origin_with_mode(global, runtime, &scenes, args.x, args.y, mode)
        }
    }
}

#[derive(Clone, Debug)]
enum ComposeInput {
    Single(Scene),
    Batch(Vec<Scene>),
}

fn compose_placement_footprint(scene: &Scene, x: Option<u16>, y: Option<u16>) -> CellRect {
    CellRect::new(
        x.unwrap_or(scene.footprint.x),
        y.unwrap_or(scene.footprint.y),
        scene.footprint.cols,
        scene.footprint.rows,
    )
}

fn read_compose_input(path: &PathBuf) -> Result<ComposeInput> {
    let bytes = if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        bytes
    } else {
        std::fs::read(path)?
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value.is_array() {
        Ok(ComposeInput::Batch(serde_json::from_value(value)?))
    } else {
        Ok(ComposeInput::Single(serde_json::from_value(value)?))
    }
}

fn run_render(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &RenderArgs,
    mode: EmitMode,
) -> Result<()> {
    match read_compose_input(&args.path)? {
        ComposeInput::Single(scene) => run_render_single(global, runtime, args, mode, scene),
        ComposeInput::Batch(scenes) => run_render_batch(global, runtime, args, mode, scenes),
    }
}

fn run_render_single(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &RenderArgs,
    mode: EmitMode,
    scene: Scene,
) -> Result<()> {
    if args.out_dir.is_some() {
        return run_render_single_animation(global, args, mode, &scene);
    }
    let png = runtime.render_png(&scene)?;
    let mut payload = serde_json::json!({
        "bytes": png.len(),
        "footprint": scene.footprint,
        "output": args.out.as_ref().map(|p| p.display().to_string()),
    });
    if mode.dry_run {
        payload["dry_run"] = serde_json::json!(true);
    }
    if mode.json_bytes {
        payload["png_base64"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(&png));
    }
    if global.json.value || mode.dry_run {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        if mode.dry_run {
            if let Some(path) = &args.manifest {
                write_json_manifest(path, &payload)?;
            }
            return Ok(());
        }
    }
    if let Some(path) = &args.out {
        std::fs::write(path, &png)?;
    } else if !global.json.value {
        std::io::stdout().lock().write_all(&png)?;
    }
    if let Some(path) = &args.manifest {
        write_json_manifest(path, &payload)?;
    }
    Ok(())
}

fn run_render_single_animation(
    global: &GlobalConfig,
    args: &RenderArgs,
    mode: EmitMode,
    scene: &Scene,
) -> Result<()> {
    if args.out.is_some() {
        return Err(anyhow!(
            "render --out and --out-dir cannot be used together"
        ));
    }
    if scene.animation.is_none() {
        return Err(anyhow!(
            "render --out-dir with a single Scene requires Scene.animation"
        ));
    }
    let Some(out_dir) = args.out_dir.as_ref() else {
        unreachable!("checked by caller")
    };
    let animation = kittui_render_cpu::render_animation(scene)?;
    let mut entries = Vec::with_capacity(animation.frames.len());
    for (idx, png) in animation.frames.iter().enumerate() {
        let path = out_dir.join(format!("frame-{idx:05}.png"));
        let mut entry = serde_json::json!({
            "index": idx,
            "bytes": png.len(),
            "delay_ms": animation.frame_delays_ms[idx],
            "output": path.display().to_string(),
        });
        if mode.json_bytes {
            entry["png_base64"] =
                serde_json::json!(base64::engine::general_purpose::STANDARD.encode(png));
        }
        entries.push(entry);
    }
    let mut payload = serde_json::json!({
        "frames": entries.len(),
        "width_px": animation.width_px,
        "height_px": animation.height_px,
        "loops": animation.loops,
        "output_dir": out_dir.display().to_string(),
        "files": entries,
    });
    if mode.dry_run {
        payload["dry_run"] = serde_json::json!(true);
    }
    if global.json.value || mode.dry_run {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        if mode.dry_run {
            if let Some(path) = &args.manifest {
                write_json_manifest(path, &payload)?;
            }
            return Ok(());
        }
    }
    std::fs::create_dir_all(out_dir)?;
    for (idx, png) in animation.frames.iter().enumerate() {
        std::fs::write(out_dir.join(format!("frame-{idx:05}.png")), png)?;
    }
    if let Some(path) = &args.manifest {
        write_json_manifest(path, &payload)?;
    }
    Ok(())
}

fn run_render_batch(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &RenderArgs,
    mode: EmitMode,
    scenes: Vec<Scene>,
) -> Result<()> {
    if args.out.is_some() {
        return Err(anyhow!("render --out is only supported for a single Scene"));
    }
    let Some(out_dir) = args.out_dir.as_ref() else {
        return Err(anyhow!("render scene arrays require --out-dir DIR"));
    };
    let pngs = runtime.render_many_png(&scenes)?;
    let mut entries = Vec::with_capacity(scenes.len());
    let mut rendered = Vec::with_capacity(scenes.len());
    for (idx, (scene, png)) in scenes.iter().zip(pngs).enumerate() {
        let path = out_dir.join(format!("scene-{idx:05}.png"));
        let mut entry = serde_json::json!({
            "index": idx,
            "bytes": png.len(),
            "footprint": scene.footprint,
            "output": path.display().to_string(),
        });
        if mode.json_bytes {
            entry["png_base64"] =
                serde_json::json!(base64::engine::general_purpose::STANDARD.encode(&png));
        }
        entries.push(entry);
        rendered.push((path, png));
    }
    let mut payload = serde_json::json!({
        "count": entries.len(),
        "output_dir": out_dir.display().to_string(),
        "files": entries,
    });
    if mode.dry_run {
        payload["dry_run"] = serde_json::json!(true);
    }
    if global.json.value || mode.dry_run {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        if mode.dry_run {
            if let Some(path) = &args.manifest {
                write_json_manifest(path, &payload)?;
            }
            return Ok(());
        }
    }
    std::fs::create_dir_all(out_dir)?;
    for (path, png) in rendered {
        std::fs::write(path, png)?;
    }
    if let Some(path) = &args.manifest {
        write_json_manifest(path, &payload)?;
    }
    Ok(())
}

fn write_json_manifest(path: &PathBuf, payload: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(payload)?),
    )?;
    Ok(())
}

fn run_image(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &ImageArgs,
    mode: EmitMode,
) -> Result<()> {
    use kittui_core::node::Fit;
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
    let src = read_image_ref(&args.src)?;
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, args.width, args.height);
    let rect = footprint.to_pixels(cell);
    let mut scene = Scene {
        footprint,
        cell_size: cell,
        layers: vec![Layer::anon(Node::Image {
            rect,
            src,
            fit,
            tint,
        })],
        animation: args.animation.scene_animation(),
    };
    if scene.animation.is_some() {
        scene.layers.push(Layer::new(
            "image-animation",
            Node::Glow {
                rect,
                center_x_frac: 0.5,
                center_y_frac: 0.5,
                radius_frac: 1.6,
                color: Rgba(255, 255, 255, 96),
                intensity: 0.42,
            },
        ));
    }
    emit_with_mode(global, runtime, &scene, None, mode)
}

fn read_image_ref(path: &PathBuf) -> Result<kittui_core::node::ImageRef> {
    if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        Ok(kittui_core::node::ImageRef::Bytes { bytes })
    } else {
        Ok(kittui_core::node::ImageRef::Path {
            path: path.to_string_lossy().into_owned(),
        })
    }
}

fn run_place(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &PlaceArgs,
    mode: EmitMode,
) -> Result<()> {
    let image_id = parse_image_id(&args.id)?;
    let footprint = CellRect::new(args.x, args.y, args.cols, args.rows);
    let transport = runtime.transport();
    let placement = kittui::Placement {
        image_id,
        upload: String::new(),
        placement: format!(
            "{}{}",
            kittui_kitty::cursor_move(footprint.x, footprint.y, transport),
            kittui_kitty::placement_command(image_id, footprint, transport)
        ),
        embed: kittui_kitty::placeholder_text(image_id, footprint),
        footprint,
    };
    emit_placement_with_mode(global, &placement, None, mode)
}

fn run_delete(
    global: &GlobalConfig,
    runtime: &Runtime,
    args: &DeleteArgs,
    mode: EmitMode,
) -> Result<()> {
    let image_id = parse_image_id(&args.id)?;
    let delete = match args.placement_id {
        Some(placement_id) => {
            kittui_kitty::delete_placement(image_id, placement_id, runtime.transport())
        }
        None => kittui_kitty::delete(image_id, runtime.transport()),
    };
    emit_delete_with_mode(global, image_id, args.placement_id, &delete, mode)
}

fn parse_image_id(value: &str) -> Result<u32> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).map_err(|e| anyhow!("invalid --id {value:?}: {e}"))
    } else {
        value
            .parse::<u32>()
            .map_err(|e| anyhow!("invalid --id {value:?}: {e}"))
    }
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
    let mut terminal = TerminalInfo::detect();
    terminal.columns = Some(global.terminal_cols.value);
    terminal.rows = Some(global.terminal_rows.value);
    let probe = probe_payload(global, &terminal, args.force);
    println!("{}", serde_json::to_string_pretty(&probe)?);
    Ok(())
}

fn probe_payload(
    global: &GlobalConfig,
    terminal: &TerminalInfo,
    force_invalidated: bool,
) -> serde_json::Value {
    serde_json::json!({
        "supports_kitty": terminal.supports_kitty,
        "supports_unicode_placeholders": terminal.supports_unicode_placeholders,
        "transport": terminal.transport,
        "columns": terminal.columns,
        "rows": terminal.rows,
        "cell_size": terminal.cell_size,
        "terminal": terminal,
        "renderer": global.renderer.value.to_string(),
        "version": env!("CARGO_PKG_VERSION"),
        "config_sources": { "global": global.source_json() },
        "force_invalidated": force_invalidated,
    })
}

fn run_proof(global: &GlobalConfig, args: &ProofArgs) -> Result<()> {
    use kittui::scene::{background_solid, rounded_rect};
    use kittui_core::terminal::Transport;
    use kittui_kitty::{
        delete, delete_placement, placeholder_text, placeholder_text_ex, placement_command,
        placement_command_ex, upload_animation, upload_still, upload_still_ex, PlacementOptions,
        Quiet, SubcellOffset, UploadMedium,
    };

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
        relative: None,
    };
    add(
        "placement with id=7, X=4, Y=2, z=1",
        format!(
            "{}{}{}",
            upload_still(0x77007700, &still_png, Transport::Direct),
            placement_command_ex(0x77007700, footprint, &p_opts, Transport::Direct),
            placeholder_text_ex(0x77007700, Some(7), footprint),
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
        if args.emit {
            // Clear screen, park cursor at top-left, print the label, then
            // emit the upload + placement + placeholder grid so each section
            // renders standalone instead of overlapping the previous one.
            handle.write_all(b"\x1b[2J\x1b[H")?;
            writeln!(handle, "\x1b[1m== {label} ==\x1b[0m")?;
            handle.write_all(body.as_bytes())?;
            writeln!(handle)?;
            handle.flush()?;
            std::thread::sleep(std::time::Duration::from_millis(
                args.dwell_ms.unwrap_or(1500),
            ));
        } else {
            writeln!(handle, "\x1b[1m== {label} ==\x1b[0m")?;
            let prefix: String = body
                .as_bytes()
                .iter()
                .take(48)
                .map(|b| format!("{:02x}", b))
                .collect();
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
    scene_json: bool,
    json_bytes: bool,
    dry_run: bool,
}

fn serialize_scene_json(scene: &Scene) -> Result<String> {
    Ok(serde_json::to_string_pretty(scene)?)
}

fn placement_json_payload(
    global: &GlobalConfig,
    placement: &kittui::Placement,
    command_sources: Option<serde_json::Value>,
    dry_run: bool,
    include_bytes: bool,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
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
    if dry_run {
        payload["dry_run"] = serde_json::json!(true);
    }
    if include_bytes {
        payload["upload"] = serde_json::json!(placement.upload);
        payload["placement"] = serde_json::json!(placement.placement);
        payload["embed"] = serde_json::json!(placement.embed);
    } else if !dry_run {
        payload["embed"] = serde_json::json!(placement.embed);
    }
    payload
}

fn emit_delete_with_mode(
    global: &GlobalConfig,
    image_id: u32,
    placement_id: Option<u32>,
    delete: &str,
    mode: EmitMode,
) -> Result<()> {
    if global.json.value || mode.dry_run {
        let mut payload = serde_json::json!({
            "image_id": format!("0x{:08x}", image_id),
            "placement_id": placement_id,
            "delete_bytes": delete.len(),
            "config_sources": { "global": global.source_json() },
        });
        if mode.dry_run {
            payload["dry_run"] = serde_json::json!(true);
        }
        if mode.json_bytes {
            payload["delete"] = serde_json::json!(delete);
        }
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    std::io::stdout().lock().write_all(delete.as_bytes())?;
    Ok(())
}

fn emit_placement_with_mode(
    global: &GlobalConfig,
    placement: &kittui::Placement,
    command_sources: Option<serde_json::Value>,
    mode: EmitMode,
) -> Result<()> {
    if mode.dry_run {
        let payload =
            placement_json_payload(global, placement, command_sources, true, mode.json_bytes);
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    if global.json.value {
        let payload =
            placement_json_payload(global, placement, command_sources, false, mode.json_bytes);
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

fn batch_json_payload(
    global: &GlobalConfig,
    batch: &kittui::BatchPlacement,
    dry_run: bool,
    include_bytes: bool,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "count": batch.image_ids.len(),
        "image_ids": batch.image_ids.iter().map(|id| format!("0x{id:08x}")).collect::<Vec<_>>(),
        "footprints": batch.footprints,
        "upload_bytes": batch.upload.len(),
        "placement_bytes": batch.placement.len(),
        "embed_bytes": batch.embed.len(),
        "config_sources": { "global": global.source_json(), "command": serde_json::Value::Null },
    });
    if dry_run {
        payload["dry_run"] = serde_json::json!(true);
    }
    if include_bytes {
        payload["upload"] = serde_json::json!(batch.upload);
        payload["placement"] = serde_json::json!(batch.placement);
        payload["embed"] = serde_json::json!(batch.embed);
    } else if !dry_run {
        payload["embed"] = serde_json::json!(batch.embed);
    }
    payload
}

fn emit_batch_with_mode(
    global: &GlobalConfig,
    batch: &kittui::BatchPlacement,
    mode: EmitMode,
) -> Result<()> {
    if mode.dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&batch_json_payload(
                global,
                batch,
                true,
                mode.json_bytes
            ))?
        );
        return Ok(());
    }
    if global.json.value {
        println!(
            "{}",
            serde_json::to_string_pretty(&batch_json_payload(
                global,
                batch,
                false,
                mode.json_bytes
            ))?
        );
        return Ok(());
    }
    let mut handle = std::io::stdout().lock();
    let any_filter = mode.upload_only || mode.placement_only || mode.embed_only;
    if !any_filter || mode.upload_only {
        handle.write_all(batch.upload.as_bytes())?;
    }
    if !any_filter || mode.placement_only {
        handle.write_all(batch.placement.as_bytes())?;
    }
    if !any_filter || mode.embed_only {
        handle.write_all(batch.embed.as_bytes())?;
    }
    Ok(())
}

fn emit_scene_batch_with_mode(
    global: &GlobalConfig,
    runtime: &Runtime,
    scenes: &[Scene],
    mode: EmitMode,
) -> Result<()> {
    if mode.scene_json {
        println!("{}", serde_json::to_string_pretty(scenes)?);
        return Ok(());
    }
    let batch = runtime.place_batch(scenes)?;
    emit_batch_with_mode(global, &batch, mode)
}

fn emit_scene_batch_at_origin_with_mode(
    global: &GlobalConfig,
    runtime: &Runtime,
    scenes: &[Scene],
    x: Option<u16>,
    y: Option<u16>,
    mode: EmitMode,
) -> Result<()> {
    if x.is_none() && y.is_none() {
        return emit_scene_batch_with_mode(global, runtime, scenes, mode);
    }
    if mode.scene_json {
        println!("{}", serde_json::to_string_pretty(scenes)?);
        return Ok(());
    }
    let min_x = scenes
        .iter()
        .map(|scene| scene.footprint.x)
        .min()
        .unwrap_or(0);
    let min_y = scenes
        .iter()
        .map(|scene| scene.footprint.y)
        .min()
        .unwrap_or(0);
    let batch = runtime.place_batch_at_origin(scenes, x.unwrap_or(min_x), y.unwrap_or(min_y))?;
    emit_batch_with_mode(global, &batch, mode)
}

fn emit_scene_at_with_mode(
    global: &GlobalConfig,
    runtime: &Runtime,
    scene: &Scene,
    footprint: CellRect,
    command_sources: Option<serde_json::Value>,
    mode: EmitMode,
) -> Result<()> {
    if mode.scene_json {
        println!("{}", serialize_scene_json(scene)?);
        return Ok(());
    }
    let placement = runtime.place_at(scene, footprint)?;
    emit_placement_with_mode(global, &placement, command_sources, mode)
}

fn emit_with_mode(
    global: &GlobalConfig,
    runtime: &Runtime,
    scene: &Scene,
    command_sources: Option<serde_json::Value>,
    mode: EmitMode,
) -> Result<()> {
    emit_scene_at_with_mode(
        global,
        runtime,
        scene,
        scene.footprint,
        command_sources,
        mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_scene() -> Scene {
        let cell = CellSize::default();
        let footprint = CellRect::new(0, 0, 2, 1);
        Scene {
            footprint,
            cell_size: cell,
            layers: vec![background_solid(
                footprint,
                cell,
                Rgba::rgba(0x10, 0x20, 0x30, 0xff),
            )],
            animation: None,
        }
    }

    #[test]
    fn update_args_map_to_shared_update_options() {
        let args = UpdateArgs {
            status: false,
            check: true,
            repository: Some("owner/repo".to_string()),
            install_dir: Some(PathBuf::from("/tmp/kittui-bin")),
        };
        let options = update_options(&args, true);
        assert_eq!(options.action, UpdateAction::Check);
        assert!(options.json);
        assert_eq!(options.repository.as_deref(), Some("owner/repo"));
        assert_eq!(
            options.install_dir.as_deref(),
            Some(std::path::Path::new("/tmp/kittui-bin"))
        );
    }

    #[test]
    fn inline_examples_cover_prompt_status_and_fallback_modes() {
        let text = inline_examples_text();
        assert!(text.contains("kittui inline examples"), "{text}");
        assert!(text.contains("--format prompt-zsh"), "{text}");
        assert!(text.contains("--format prompt-bash"), "{text}");
        assert!(text.contains("--format tmux"), "{text}");
        assert!(text.contains("--format plain"), "{text}");
        assert!(text.contains("--format ansi"), "{text}");
        assert!(text.contains("--style neon"), "{text}");
        assert!(text.contains("--animated"), "{text}");
        assert!(text.contains("60fps"), "{text}");
        assert!(
            text.contains("prompt modes wrap only nonprinting bytes"),
            "{text}"
        );
    }

    #[test]
    fn inline_animation_defaults_to_three_second_looping_period() {
        let args = InlineAnimationArgs {
            animated: true,
            ..InlineAnimationArgs::default()
        };
        let animation = args.scene_animation().unwrap();
        assert_eq!(animation.frames, 180);
        assert_eq!(animation.cycle_ms, 3000);
        assert_eq!(animation.loops, 0);
        assert!(animation.curve.closes_loop());

        let colors = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Glass);
        let chip = inline_chip_scene(
            8,
            colors,
            InlineTextComponent::Chip,
            InlineStyleArg::Glass,
            Some(animation.clone()),
        );
        assert_eq!(chip.animation, Some(animation.clone()));
        let divider = inline_divider_scene(
            8,
            colors.border,
            InlineStyleArg::Glass,
            Some(animation.clone()),
        );
        assert_eq!(divider.animation, Some(animation.clone()));
        let row_args = InlineRowArgs {
            items: vec!["chip:main".to_string(), "divider:4".to_string()],
            format: InlineFormatArg::Kitty,
            tone: ToneArg::Assistant,
            theme: InlineThemeArg::Nord,
            style: InlineStyleArg::Glass,
            padding: 1,
            gap: 0,
            animation: args,
        };
        let items = parse_inline_row_items(&row_args.items).unwrap();
        let row = inline_row_scene(
            &items,
            &row_args,
            colors,
            row_args.animation.scene_animation(),
        )
        .unwrap();
        assert_eq!(row.animation, Some(animation));
    }

    #[test]
    fn inline_animation_flags_clamp_to_safe_kitty_frame_contract() {
        let args = InlineAnimationArgs {
            animated: true,
            fps: 0,
            frames: 1,
        };
        let animation = args.scene_animation().unwrap();
        assert_eq!(animation.frames, 2);
        assert_eq!(animation.cycle_ms, 2000);
        assert!(animation.curve.closes_loop());

        let mut payload = serde_json::json!({});
        add_inline_animation_json(&mut payload, args);
        assert_eq!(payload["inline_animated"], true);
        assert_eq!(payload["inline_animation"]["fps"], 1);
        assert_eq!(payload["inline_animation"]["frames"], 2);
        assert_eq!(payload["inline_animation"]["cycle_ms"], 2000);
    }

    #[test]
    fn inline_animated_styles_add_phase_reactive_effect_layers() {
        let animation = InlineAnimationArgs {
            animated: true,
            ..InlineAnimationArgs::default()
        }
        .scene_animation();
        let cases = [
            (InlineStyleArg::Glass, "inline-effect-glass-glare"),
            (InlineStyleArg::Neon, "inline-effect-neon-pulse"),
            (InlineStyleArg::Metal, "inline-effect-metal-reflection"),
            (InlineStyleArg::Chrome, "inline-effect-chrome-sheen"),
        ];
        for (style, label) in cases {
            let colors = InlineChipColors::resolve(InlineTheme::Nord, style.into());
            let scene = inline_chip_scene(
                8,
                colors,
                InlineTextComponent::Chip,
                style,
                animation.clone(),
            );
            assert!(
                scene
                    .layers
                    .iter()
                    .any(|layer| layer.label.as_deref() == Some(label)),
                "missing {label}: {:?}",
                scene
                    .layers
                    .iter()
                    .filter_map(|layer| layer.label.as_deref())
                    .collect::<Vec<_>>()
            );
            let divider = inline_divider_scene(8, colors.border, style, animation.clone());
            assert!(divider
                .layers
                .iter()
                .any(|layer| layer.label.as_deref() == Some(label)));
        }
    }

    #[test]
    fn inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats() {
        let mut args = InlineChipArgs {
            text: "main#1".to_string(),
            format: InlineFormatArg::Plain,
            tone: ToneArg::Assistant,
            theme: InlineThemeArg::Nord,
            style: InlineStyleArg::Glass,
            bg_color: None,
            border_color: None,
            fg_color: None,
            padding: 1,
            animation: InlineAnimationArgs::default(),
        };
        assert_eq!(render_inline_chip(&args), "[ main#1 ]");

        args.format = InlineFormatArg::Ansi;
        let ansi = render_inline_chip(&args);
        assert!(ansi.starts_with("\x1b[1;38;2;"), "{ansi:?}");
        assert!(ansi.contains(" main#1 "), "{ansi:?}");
        assert!(ansi.ends_with("\x1b[0m"), "{ansi:?}");

        args.format = InlineFormatArg::Tmux;
        let tmux = render_inline_chip(&args);
        assert!(tmux.starts_with("#[bold,fg=#"), "{tmux}");
        assert!(tmux.contains(" main##1 "), "{tmux}");
        assert!(tmux.ends_with("#[default]"), "{tmux}");

        args.format = InlineFormatArg::Kitty;
        let colors = inline_chip_colors(&args).unwrap();
        assert_eq!(colors.fill.3, 175);
        assert_eq!(inline_chip_cols(&args), 8);
        let scene = inline_chip_scene(
            inline_chip_cols(&args),
            colors,
            InlineTextComponent::Chip,
            args.style,
            None,
        );
        assert_eq!(scene.footprint.cols, 8);
        assert_eq!(scene.footprint.rows, 1);
        let embed =
            inline_chip_text_embed(&args.text, args.padding, colors.fg, PromptWrapper::None);
        assert!(!embed.contains(kittui_kitty::PLACEHOLDER_CHAR), "{embed:?}");
        assert!(embed.contains(" main#1 \x1b[39m"), "{embed:?}");
        assert_eq!(
            parse_nord_inline_color("8").unwrap(),
            Rgba::parse("#88c0d0").unwrap()
        );
        assert_eq!(
            parse_nord_inline_color("purple").unwrap(),
            Rgba::parse("#b48ead").unwrap()
        );
        let placement = kittui::Placement {
            image_id: 0x00112233,
            upload: String::new(),
            placement: String::new(),
            embed: String::new(),
            footprint: CellRect::new(0, 0, 8, 1),
        };
        let placement =
            inline_background_placement(&placement, kittui_core::terminal::Transport::Direct);
        assert!(placement.contains("a=p"), "{placement:?}");
        assert!(placement.contains("c=8,r=1"), "{placement:?}");
        assert!(placement.contains("z=-1"), "{placement:?}");
        assert!(placement.contains("\x1b[8D"), "{placement:?}");
        assert!(!placement.contains("U=1"), "{placement:?}");
        assert!(!placement.contains("[1;1H"), "{placement:?}");
    }

    #[test]
    fn inline_badge_segment_and_divider_have_one_line_outputs() {
        let mut args = InlineChipArgs {
            text: "ok".to_string(),
            format: InlineFormatArg::Plain,
            tone: ToneArg::Assistant,
            theme: InlineThemeArg::Nord,
            style: InlineStyleArg::Glass,
            bg_color: None,
            border_color: None,
            fg_color: None,
            padding: 1,
            animation: InlineAnimationArgs::default(),
        };
        assert_eq!(
            render_inline_text_component(&args, InlineTextComponent::Badge),
            "< ok >"
        );
        assert_eq!(
            render_inline_text_component(&args, InlineTextComponent::Segment),
            " ok "
        );
        args.format = InlineFormatArg::Tmux;
        let badge = render_inline_text_component(&args, InlineTextComponent::Badge);
        assert!(badge.starts_with("#[bold,fg=#"), "{badge}");
        assert!(badge.contains(" ok "), "{badge}");

        let colors = inline_chip_colors(&args).unwrap();
        let segment_scene =
            inline_chip_scene(4, colors, InlineTextComponent::Segment, args.style, None);
        assert_eq!(segment_scene.footprint.rows, 1);
        assert_eq!(segment_scene.footprint.cols, 4);

        let divider = InlineDividerArgs {
            width: 5,
            glyph: "=".to_string(),
            format: InlineFormatArg::Plain,
            tone: ToneArg::Assistant,
            theme: InlineThemeArg::Nord,
            style: InlineStyleArg::Glass,
            color: None,
            animation: InlineAnimationArgs::default(),
        };
        assert_eq!(render_inline_divider(&divider).unwrap(), "=====");
        let scene = inline_divider_scene(
            5,
            inline_divider_color(&divider).unwrap(),
            divider.style,
            None,
        );
        assert_eq!(scene.footprint.cols, 5);
        assert_eq!(scene.footprint.rows, 1);
    }

    #[test]
    fn inline_row_composes_multiple_items_in_order() {
        let raw_items = vec![
            "chip:main".to_string(),
            "badge:ok".to_string(),
            "divider:3:=".to_string(),
            "segment:dev".to_string(),
        ];
        let items = parse_inline_row_items(&raw_items).unwrap();
        assert_eq!(items.len(), 4);
        let args = InlineRowArgs {
            items: raw_items,
            format: InlineFormatArg::Plain,
            tone: ToneArg::Assistant,
            theme: InlineThemeArg::Nord,
            style: InlineStyleArg::Glass,
            padding: 1,
            gap: 1,
            animation: InlineAnimationArgs::default(),
        };
        assert_eq!(
            render_inline_row_fallback(&items, &args).unwrap(),
            "[ main ] < ok > ===  dev "
        );
        assert_eq!(inline_row_cols(&items, args.padding, args.gap as u16), 21);
        let colors = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Glass);
        let scene = inline_row_scene(&items, &args, colors, None).unwrap();
        assert_eq!(scene.footprint.cols, 21);
        assert_eq!(scene.footprint.rows, 1);
        assert!(!scene.layers.is_empty());
    }

    #[test]
    fn inline_chip_prompt_formats_wrap_only_nonprinting_bytes() {
        let args = InlineChipArgs {
            text: "main#1".to_string(),
            format: InlineFormatArg::PromptZsh,
            tone: ToneArg::Assistant,
            theme: InlineThemeArg::Nord,
            style: InlineStyleArg::Glass,
            bg_color: None,
            border_color: None,
            fg_color: None,
            padding: 1,
            animation: InlineAnimationArgs::default(),
        };
        assert!(args.format.uses_kitty_graphics());
        assert_eq!(args.format.label(), "prompt-zsh");
        let colors = inline_chip_colors(&args).unwrap();
        let zsh = inline_chip_text_embed(&args.text, args.padding, colors.fg, PromptWrapper::Zsh);
        assert!(zsh.starts_with("%{\x1b[38;2;"), "{zsh:?}");
        assert!(zsh.contains("%} main#1 %{"), "{zsh:?}");
        assert!(zsh.ends_with("\x1b[39m%}"), "{zsh:?}");
        assert!(!zsh.contains("%{ main#1 %}"), "{zsh:?}");

        let bash = inline_chip_text_embed(&args.text, args.padding, colors.fg, PromptWrapper::Bash);
        assert!(bash.starts_with("\\[\x1b[38;2;"), "{bash:?}");
        assert!(bash.contains("\\] main#1 \\["), "{bash:?}");
        assert!(bash.ends_with("\x1b[39m\\]"), "{bash:?}");
        assert!(!bash.contains("\\[ main#1 \\]"), "{bash:?}");

        let placement = kittui::Placement {
            image_id: 0x00112233,
            upload: String::new(),
            placement: String::new(),
            embed: String::new(),
            footprint: CellRect::new(0, 0, 8, 1),
        };
        let raw = inline_background_placement(&placement, kittui_core::terminal::Transport::Direct);
        let wrapped = wrap_prompt_nonprinting(&raw, PromptWrapper::Zsh);
        assert!(wrapped.starts_with("%{\x1b_G"), "{wrapped:?}");
        assert!(wrapped.ends_with("\x1b[8D%}"), "{wrapped:?}");
        let upload = wrap_prompt_nonprinting("\x1b_Ga=t,f=100;payload\x1b\\", PromptWrapper::Bash);
        assert_eq!(upload, "\\[\x1b_Ga=t,f=100;payload\x1b\\\\]");
    }

    #[test]
    fn compose_placement_footprint_overrides_position_only() {
        let scene = tiny_scene();
        assert_eq!(
            compose_placement_footprint(&scene, Some(7), Some(9)),
            CellRect::new(7, 9, scene.footprint.cols, scene.footprint.rows)
        );
        assert_eq!(
            compose_placement_footprint(&scene, Some(7), None),
            CellRect::new(
                7,
                scene.footprint.y,
                scene.footprint.cols,
                scene.footprint.rows
            )
        );
    }

    #[test]
    fn scene_json_round_trips_as_compose_input() {
        let scene = tiny_scene();
        let json = serialize_scene_json(&scene).unwrap();
        assert!(json.contains("footprint"), "{json}");
        let parsed: Scene = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.footprint, scene.footprint);
        assert_eq!(parsed.cell_size, scene.cell_size);
        assert_eq!(parsed.layers.len(), scene.layers.len());
    }

    #[test]
    fn batch_json_payload_reports_counts_and_channels() {
        let batch = kittui::BatchPlacement {
            upload: "upload".to_string(),
            placement: "place".to_string(),
            embed: "embed".to_string(),
            image_ids: vec![1, 0x1234],
            footprints: vec![CellRect::new(0, 0, 1, 1), CellRect::new(2, 3, 4, 5)],
        };
        let payload = batch_json_payload(&test_global(), &batch, true, true);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["count"], 2);
        assert_eq!(payload["image_ids"][1], "0x00001234");
        assert_eq!(payload["upload"], "upload");
        assert_eq!(payload["placement"], "place");
        assert_eq!(payload["embed"], "embed");
    }

    #[test]
    fn compose_scene_reader_accepts_files() {
        let path = std::env::temp_dir().join(format!(
            "kittui-compose-scene-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let scene = tiny_scene();
        std::fs::write(&path, serialize_scene_json(&scene).unwrap()).unwrap();
        let parsed = read_compose_input(&path).unwrap();
        match parsed {
            ComposeInput::Single(parsed) => assert_eq!(parsed.footprint, scene.footprint),
            ComposeInput::Batch(_) => panic!("expected single scene"),
        }
        let _ = std::fs::remove_file(path);
    }

    fn test_global() -> GlobalConfig {
        config::ConfigLayers::from_parts(
            config::FileConfig::default(),
            config::EnvConfig::default(),
        )
        .resolve_global(GlobalFlagValues {
            cache_dir: None,
            renderer: None,
            terminal_cols: Some(132),
            terminal_rows: Some(43),
            json: true,
        })
    }

    #[test]
    fn inline_affordance_chrome_builds_scenes() {
        let chip = chrome_to_scene(
            chip_chrome(
                Rgba::parse("#001122").unwrap(),
                Rgba::parse("#00d8ff").unwrap(),
            ),
            8,
            1,
            "chip",
        )
        .unwrap();
        assert_eq!(chip.footprint.cols, 8);
        assert!(chip
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("border")));
        let divider = chrome_to_scene(
            divider_chrome(
                Rgba::parse("#001122").unwrap(),
                Rgba::parse("#00d8ff").unwrap(),
            ),
            12,
            1,
            "divider",
        )
        .unwrap();
        assert_eq!(divider.footprint.rows, 1);
        assert!(divider
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("background")));
    }

    #[test]
    fn wm_session_scenes_follow_manifest_layout_weights_and_focus() {
        let manifest: WmSessionManifest = serde_json::from_value(serde_json::json!({
            "layout": "columns",
            "panes": [
                {"window": "native-1", "title": "shell", "command": "bash", "weight": 1, "focused": false},
                {"window": "native-2", "title": "logs", "command": "tail -f app.log", "weight": 3, "focused": true}
            ]
        }))
        .unwrap();
        let scenes = wm_session_scenes(&manifest, 80, 24).unwrap();
        assert_eq!(scenes.len(), 2);
        assert_eq!(scenes[0].footprint, CellRect::new(0, 0, 20, 24));
        assert_eq!(scenes[1].footprint, CellRect::new(20, 0, 60, 24));
        let labels = scenes[1]
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"wm-chrome:tiled:logs"), "{labels:?}");
        assert!(scenes[1].layers.iter().any(|layer| matches!(
            &layer.root,
            Node::Rect { stroke: Some(stroke), .. } if stroke.width_px == 2.0
        )));
    }

    #[test]
    fn wm_session_scenes_support_rows() {
        let manifest: WmSessionManifest = serde_json::from_value(serde_json::json!({
            "layout": "rows",
            "panes": [
                {"title": "top", "command": "top", "weight": 1},
                {"title": "bottom", "command": "bottom", "weight": 1}
            ]
        }))
        .unwrap();
        let scenes = wm_session_scenes(&manifest, 80, 24).unwrap();
        assert_eq!(scenes[0].footprint, CellRect::new(0, 0, 80, 12));
        assert_eq!(scenes[1].footprint, CellRect::new(0, 12, 80, 12));
    }

    #[test]
    fn wm_chrome_scene_uses_kittwm_theme_labels() {
        let scene = wm_chrome_scene(20, 3, true, false, "logs");
        assert_eq!(scene.footprint.cols, 20);
        assert_eq!(scene.footprint.rows, 3);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"wm-chrome:floating:logs"), "{labels:?}");
        assert!(scene.layers.iter().any(|layer| matches!(
            &layer.root,
            Node::Rect { stroke: Some(stroke), .. } if stroke.width_px == 2.0
        )));
    }

    #[test]
    fn wm_chrome_and_session_can_add_animation_layers() {
        let mut chrome = wm_chrome_scene(20, 3, true, false, "logs");
        let animation = InlineAnimationArgs {
            animated: true,
            ..InlineAnimationArgs::default()
        }
        .scene_animation();
        add_affordance_animation(
            &mut chrome,
            animation.clone(),
            Rgba(0x88, 0xc0, 0xd0, 0xcc),
            "wm-chrome-animation",
        );
        assert_eq!(chrome.animation, animation);
        assert!(chrome
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("wm-chrome-animation")));

        let manifest: WmSessionManifest = serde_json::from_value(serde_json::json!({
            "layout": "columns",
            "panes": [{"title": "shell"}]
        }))
        .unwrap();
        let mut scenes = wm_session_scenes(&manifest, 20, 3).unwrap();
        add_affordance_animation(
            &mut scenes[0],
            animation.clone(),
            Rgba(0x88, 0xc0, 0xd0, 0xcc),
            "wm-session-animation",
        );
        assert_eq!(scenes[0].animation, animation);
        assert!(scenes[0]
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("wm-session-animation")));
    }

    #[test]
    fn panel_scene_uses_affordance_chrome_and_animation_flag() {
        let chrome = panel_chrome(Tone::Assistant, &PanelOptions { animated: true });
        let scene = chrome
            .to_scene(ratatui::layout::Rect::new(0, 0, 20, 4))
            .unwrap();
        assert_eq!(scene.footprint.cols, 20);
        assert_eq!(scene.footprint.rows, 4);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("background")));
        assert!(scene.animation.is_some());
    }

    #[test]
    fn parse_image_id_accepts_decimal_and_hex() {
        assert_eq!(parse_image_id("4660").unwrap(), 0x1234);
        assert_eq!(parse_image_id("0x1234").unwrap(), 0x1234);
        assert_eq!(parse_image_id("0XABCD").unwrap(), 0xabcd);
        assert!(parse_image_id("not-an-id").is_err());
    }

    #[test]
    fn placement_json_bytes_are_opt_in() {
        let placement = kittui::Placement {
            image_id: 0x1234,
            upload: "upload-bytes".to_string(),
            placement: "place-bytes".to_string(),
            embed: "embed-bytes".to_string(),
            footprint: CellRect::new(0, 0, 2, 1),
        };
        let compact = placement_json_payload(&test_global(), &placement, None, false, false);
        assert_eq!(compact["upload_bytes"], 12);
        assert_eq!(compact["placement_bytes"], 11);
        assert!(compact.get("upload").is_none());
        assert!(compact.get("placement").is_none());
        assert_eq!(compact["embed"], "embed-bytes");

        let verbose = placement_json_payload(&test_global(), &placement, None, false, true);
        assert_eq!(verbose["upload"], "upload-bytes");
        assert_eq!(verbose["placement"], "place-bytes");
        assert_eq!(verbose["embed"], "embed-bytes");
    }

    #[test]
    fn dry_run_json_bytes_include_channels_when_requested() {
        let placement = kittui::Placement {
            image_id: 0x1234,
            upload: "upload-bytes".to_string(),
            placement: "place-bytes".to_string(),
            embed: "embed-bytes".to_string(),
            footprint: CellRect::new(0, 0, 2, 1),
        };
        let payload = placement_json_payload(&test_global(), &placement, None, true, true);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["upload"], "upload-bytes");
        assert_eq!(payload["placement"], "place-bytes");
        assert_eq!(payload["embed"], "embed-bytes");
    }

    #[test]
    fn probe_payload_reports_detected_terminal_descriptor() {
        let terminal = TerminalInfo::override_with(
            Some(132),
            Some(43),
            CellSize::new(9, 18),
            false,
            false,
            kittui_core::terminal::Transport::TmuxPassthrough,
        );
        let payload = probe_payload(&test_global(), &terminal, true);
        assert_eq!(payload["supports_kitty"], false);
        assert_eq!(payload["supports_unicode_placeholders"], false);
        assert_eq!(payload["transport"], "tmux_passthrough");
        assert_eq!(payload["terminal"]["cell_size"]["width_px"], 9);
        assert_eq!(payload["columns"], 132);
        assert_eq!(payload["rows"], 43);
        assert_eq!(payload["force_invalidated"], true);
    }
}
