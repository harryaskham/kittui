use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use image::{imageops, Rgba, RgbaImage};
use kittui_ghostty_vt::{render_snapshot_preview_png, GhosttyVtTerminal, PreviewOptions};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

#[derive(Debug)]
struct Args {
    out: PathBuf,
    out_dir: PathBuf,
    montage: Option<PathBuf>,
    cols: u16,
    rows: u16,
    chunk_lines: usize,
    demo: bool,
    timelapse_demo: bool,
    command: Option<String>,
    pty_command: Option<String>,
    pty_timelapse_command: Option<String>,
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
    if let Some(command) = &args.pty_timelapse_command {
        return render_pty_timelapse_command(&args, command);
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
    let chunks = timelapse_demo_steps().iter().copied().collect::<Vec<_>>();
    render_timelapse_chunks(args, chunks)
}

fn render_pty_timelapse_command(args: &Args, command: &str) -> anyhow::Result<()> {
    let bytes = pty_command_bytes(command, args.cols, args.rows)?;
    let chunks = line_chunks(&bytes, args.chunk_lines);
    render_timelapse_chunks(args, chunks)
}

fn render_timelapse_chunks(args: &Args, chunks: Vec<&[u8]>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.out_dir)?;
    let mut terminal = GhosttyVtTerminal::new(args.cols, args.rows, 1_000)?;
    let mut frames = Vec::new();
    for (idx, bytes) in chunks.iter().enumerate() {
        terminal.write(bytes);
        let snapshot = terminal.render_snapshot()?;
        let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
        let path = args.out_dir.join(format!("frame-{idx:03}.png"));
        std::fs::write(&path, png)?;
        frames.push((idx, path, snapshot.cursor_x, snapshot.cursor_y));
    }
    write_manifest(&args.out_dir, &frames)?;
    if let Some(path) = &args.montage {
        write_montage(path, &frames)?;
    }
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
    let mut montage = None;
    let mut cols = 64u16;
    let mut rows = 12u16;
    let mut chunk_lines = 1usize;
    let mut demo = false;
    let mut timelapse_demo = false;
    let mut command = None;
    let mut pty_command = None;
    let mut pty_timelapse_command = None;
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
            "--montage" => {
                montage = Some(
                    iter.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| anyhow::anyhow!("--montage PATH"))?,
                );
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
            "--chunk-lines" => {
                chunk_lines = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--chunk-lines N"))?
                    .parse()?;
                if chunk_lines == 0 {
                    anyhow::bail!("--chunk-lines must be greater than zero");
                }
            }
            "--demo" => demo = true,
            "--timelapse-demo" => timelapse_demo = true,
            "--command" | "-c" => {
                command = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--command COMMAND"))?,
                );
            }
            "--pty-command" => {
                pty_command = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--pty-command COMMAND"))?,
                );
            }
            "--pty-timelapse-command" => {
                pty_timelapse_command = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--pty-timelapse-command COMMAND"))?,
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
        montage,
        cols,
        rows,
        chunk_lines,
        demo,
        timelapse_demo,
        command,
        pty_command,
        pty_timelapse_command,
        scroll,
    })
}

fn input_bytes(args: &Args) -> anyhow::Result<Vec<u8>> {
    if args.command.is_some() && args.pty_command.is_some() {
        anyhow::bail!("--command and --pty-command are mutually exclusive");
    }
    if let Some(command) = &args.pty_command {
        return pty_command_bytes(command, args.cols, args.rows);
    }
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

fn pty_command_bytes(command: &str, cols: u16, rows: u16) -> anyhow::Result<Vec<u8>> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = CommandBuilder::new("sh");
    cmd.arg("-c");
    cmd.arg(command);
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd.as_os_str());
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLUMNS", cols.to_string());
    cmd.env("LINES", rows.to_string());

    let mut reader = pair.master.try_clone_reader()?;
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let handle = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = reader.read_to_end(&mut output);
        output
    });
    let status = child.wait()?;
    drop(child);
    let output = handle
        .join()
        .map_err(|_| anyhow::anyhow!("PTY reader thread panicked"))?;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"$ ");
    bytes.extend_from_slice(command.as_bytes());
    bytes.extend_from_slice(b"\r\n");
    bytes.extend_from_slice(&output);
    if !status.success() {
        bytes.extend_from_slice(format!("\r\n[exit {}]\r\n", status.exit_code()).as_bytes());
    }
    Ok(bytes)
}

fn line_chunks(bytes: &[u8], lines_per_chunk: usize) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return vec![b""];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut lines = 0;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines += 1;
            if lines >= lines_per_chunk {
                chunks.push(&bytes[start..=idx]);
                start = idx + 1;
                lines = 0;
            }
        }
    }
    if start < bytes.len() {
        chunks.push(&bytes[start..]);
    }
    chunks
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
           kittui-ghostty --pty-command COMMAND [--out PATH] [--cols N] [--rows N] [--scroll top|bottom|current]\n\
           kittui-ghostty --pty-timelapse-command COMMAND [--out-dir DIR] [--montage PATH] [--cols N] [--rows N] [--chunk-lines N]\n\
           kittui-ghostty --timelapse-demo [--out-dir DIR] [--montage PATH] [--cols N] [--rows N]\n\n\
         Reads VT bytes from stdin. If stdin is empty or --demo is passed, renders demo content.\n\
         --command/-c runs COMMAND through sh -c and renders stdout/stderr.\n\
         --pty-command runs COMMAND in a PTY sized by --cols/--rows and renders captured VT bytes.\n\
         --pty-timelapse-command replays captured PTY bytes into frame-*.png plus manifest.json.\n\
         --chunk-lines controls PTY timelapse replay density; default is 1.\n\
         --timelapse-demo emits frame-*.png plus manifest.json into --out-dir.\n\
         --montage writes a representative vertical PNG montage for timelapse modes."
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

fn write_montage(path: &Path, frames: &[(usize, PathBuf, u16, u16)]) -> anyhow::Result<()> {
    let selected = montage_frame_indices(frames.len());
    let mut entries = Vec::new();
    for idx in selected {
        let (frame_idx, frame_path, cursor_x, cursor_y) = &frames[idx];
        let bytes = std::fs::read(frame_path)?;
        let image = image::load_from_memory(&bytes)?.to_rgba8();
        let label = format!("frame-{frame_idx:03}.png cursor={cursor_x},{cursor_y}");
        entries.push((label, image));
    }
    if entries.is_empty() {
        anyhow::bail!("cannot build montage without frames");
    }

    let pad = 14u32;
    let gap = 18u32;
    let label_height = 14u32;
    let width = entries
        .iter()
        .map(|(_, image)| image.width())
        .max()
        .unwrap_or(1)
        + pad * 2;
    let height = entries
        .iter()
        .map(|(_, image)| label_height + image.height())
        .sum::<u32>()
        + gap * (entries.len().saturating_sub(1) as u32)
        + pad * 2;
    let mut montage = RgbaImage::from_pixel(width, height, Rgba([16, 24, 32, 255]));
    let mut y = pad;
    for (label, image) in entries {
        draw_text(&mut montage, pad, y, &label, Rgba([216, 222, 233, 255]));
        y += label_height;
        imageops::overlay(&mut montage, &image, pad.into(), y.into());
        y += image.height() + gap;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    montage.save(path)?;
    Ok(())
}

fn draw_text(img: &mut RgbaImage, x: u32, y: u32, text: &str, color: Rgba<u8>) {
    use font8x8::UnicodeFonts;

    let mut cursor_x = x;
    for ch in text.chars() {
        if ch == ' ' {
            cursor_x += 8;
            continue;
        }
        let Some(glyph) = font8x8::BASIC_FONTS.get(ch) else {
            cursor_x += 8;
            continue;
        };
        for (gy, row_bits) in glyph.iter().enumerate() {
            for gx in 0..8u32 {
                if (row_bits >> gx) & 1 == 1 {
                    let px = cursor_x + gx;
                    let py = y + gy as u32;
                    if px < img.width() && py < img.height() {
                        img.put_pixel(px, py, color);
                    }
                }
            }
        }
        cursor_x += 8;
    }
}

fn montage_frame_indices(len: usize) -> Vec<usize> {
    if len <= 6 {
        return (0..len).collect();
    }
    let last = len - 1;
    vec![
        0,
        last / 5,
        (last * 2) / 5,
        (last * 3) / 5,
        (last * 4) / 5,
        last,
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
