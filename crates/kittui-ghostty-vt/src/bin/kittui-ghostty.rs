use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use kittui_ghostty_vt::{render_snapshot_preview_png, GhosttyVtTerminal, PreviewOptions};

#[derive(Debug)]
struct Args {
    out: PathBuf,
    out_dir: PathBuf,
    cols: u16,
    rows: u16,
    demo: bool,
    timelapse_demo: bool,
    command: Option<String>,
    scroll: ScrollMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollMode {
    Current,
    Top,
    Bottom,
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    if args.timelapse_demo {
        return render_timelapse_demo(&args);
    }

    let input = input_bytes(&args)?;

    let mut terminal = GhosttyVtTerminal::new(args.cols, args.rows, 1_000)?;
    terminal.write(&input);
    apply_scroll(&mut terminal, args.scroll);
    let snapshot = terminal.render_snapshot()?;
    let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
    std::fs::write(&args.out, png)?;
    println!(
        "kittui-ghostty wrote {} ({}x{} cells, cursor={}, {})",
        args.out.display(),
        snapshot.cols,
        snapshot.rows,
        snapshot.cursor_x,
        snapshot.cursor_y
    );
    Ok(())
}

fn render_timelapse_demo(args: &Args) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.out_dir)?;
    let mut terminal = GhosttyVtTerminal::new(args.cols, args.rows, 1_000)?;
    let mut frames = Vec::new();
    for (idx, bytes) in timelapse_demo_steps().iter().enumerate() {
        terminal.write(*bytes);
        let snapshot = terminal.render_snapshot()?;
        let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
        let path = args.out_dir.join(format!("frame-{idx:03}.png"));
        std::fs::write(&path, png)?;
        frames.push((idx, path, snapshot.cursor_x, snapshot.cursor_y));
    }
    write_manifest(&args.out_dir, &frames)?;
    println!(
        "kittui-ghostty wrote {} timelapse frames to {}",
        frames.len(),
        args.out_dir.display()
    );
    Ok(())
}

fn parse_args() -> anyhow::Result<Args> {
    let mut out = PathBuf::from("/tmp/kittui-ghostty.png");
    let mut out_dir = PathBuf::from("/tmp/kittui-ghostty-timelapse");
    let mut cols = 64u16;
    let mut rows = 12u16;
    let mut demo = false;
    let mut timelapse_demo = false;
    let mut command = None;
    let mut scroll = ScrollMode::Current;
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" | "-o" => {
                out = iter
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("--out PATH"))?;
            }
            "--out-dir" => {
                out_dir = iter
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("--out-dir DIR"))?;
            }
            "--cols" => {
                cols = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--cols N"))?
                    .parse()?;
            }
            "--rows" => {
                rows = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--rows N"))?
                    .parse()?;
            }
            "--demo" => demo = true,
            "--timelapse-demo" => timelapse_demo = true,
            "--command" | "-c" => {
                command = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--command COMMAND"))?,
                );
            }
            "--scroll" => {
                scroll = parse_scroll(
                    &iter
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--scroll top|bottom|current"))?,
                )?;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other:?}; try --help"),
        }
    }
    Ok(Args {
        out,
        out_dir,
        cols,
        rows,
        demo,
        timelapse_demo,
        command,
        scroll,
    })
}

fn input_bytes(args: &Args) -> anyhow::Result<Vec<u8>> {
    if let Some(command) = &args.command {
        return command_bytes(command);
    }

    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    if args.demo || input.is_empty() {
        input = demo_bytes();
    }
    Ok(input)
}

fn command_bytes(command: &str) -> anyhow::Result<Vec<u8>> {
    let output = Command::new("sh").arg("-c").arg(command).output()?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"$ ");
    bytes.extend_from_slice(command.as_bytes());
    bytes.extend_from_slice(b"\n");
    bytes.extend_from_slice(&output.stdout);
    if !output.stderr.is_empty() {
        bytes.extend_from_slice(b"\n[stderr]\n");
        bytes.extend_from_slice(&output.stderr);
    }
    if !output.status.success() {
        bytes.extend_from_slice(format!("\n[exit {:?}]\n", output.status.code()).as_bytes());
    }
    Ok(bytes)
}

fn parse_scroll(value: &str) -> anyhow::Result<ScrollMode> {
    match value {
        "current" => Ok(ScrollMode::Current),
        "top" => Ok(ScrollMode::Top),
        "bottom" => Ok(ScrollMode::Bottom),
        other => anyhow::bail!("--scroll expects top|bottom|current, got {other:?}"),
    }
}

fn apply_scroll(terminal: &mut GhosttyVtTerminal, scroll: ScrollMode) {
    match scroll {
        ScrollMode::Current => {}
        ScrollMode::Top => terminal.scroll_top(),
        ScrollMode::Bottom => terminal.scroll_bottom(),
    }
}

fn print_help() {
    println!(
        "kittui-ghostty — portable headless libghostty-vt PNG preview\n\n\
         Usage:\n\
           kittui-ghostty [--out PATH] [--cols N] [--rows N] [--demo] [--scroll top|bottom|current]\n\
           kittui-ghostty --command COMMAND [--out PATH] [--cols N] [--rows N] [--scroll top|bottom|current]\n\
           kittui-ghostty --timelapse-demo [--out-dir DIR] [--cols N] [--rows N]\n\n\
         Reads VT bytes from stdin. If stdin is empty or --demo is passed, renders demo content.\n\
         --command/-c runs COMMAND through sh -c and renders stdout/stderr.\n\
         --timelapse-demo emits frame-*.png plus manifest.json into --out-dir."
    );
}

fn demo_bytes() -> Vec<u8> {
    b"kittui-ghostty CLI\n\
      \x1b[32mportable libghostty-vt\x1b[0m render-state preview\n\
      stdin -> Ghostty VT state -> kittui-owned PNG\n\
      \x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[36mcolor\x1b[0m\n"
        .to_vec()
}

fn timelapse_demo_steps() -> &'static [&'static [u8]] {
    &[
        b"kittui-ghostty CLI timelapse\n",
        b"\x1b[32mstep 1:\x1b[0m stdin and demo bytes feed Ghostty VT state\n",
        b"\x1b[33mstep 2:\x1b[0m render-state rows/cells become PNG frames\n",
        b"\x1b[1mstep 3:\x1b[0m styles: \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[36mcolor\x1b[0m\n",
        b"\x1b[35mstep 4:\x1b[0m deterministic artifacts for agents and CI\n",
    ]
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
            "{{\n  \"kind\": \"kittui-ghostty-cli-timelapse\",\n  \"frame_count\": {},\n  \"frames\": [\n  {}\n  ]\n}}\n",
            frames.len(), files
        ),
    )?;
    Ok(())
}
