use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
    kittwm_proof_command: Option<String>,
    pty_timelapse_command: Option<String>,
    pty_sampled_command: Option<String>,
    pty_input: Option<String>,
    pty_input_delay_ms: u64,
    sample_ms: u64,
    max_ms: u64,
    scroll: ScrollMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollMode {
    Current,
    Top,
    Bottom,
}

struct FrameRecord {
    index: usize,
    path: PathBuf,
    cursor_x: u16,
    cursor_y: u16,
    kitty_placements: usize,
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    if args.timelapse_demo {
        return render_timelapse_demo(&args);
    }
    if let Some(command) = &args.pty_timelapse_command {
        return render_pty_timelapse_command(&args, command);
    }
    if let Some(command) = &args.pty_sampled_command {
        return render_pty_sampled_command(&args, command);
    }

    let (input, inner_exit_status) = input_bytes(&args)?;

    let mut terminal = GhosttyVtTerminal::new(args.cols, args.rows, 1_000)?;
    terminal.write(&input);
    apply_scroll(&mut terminal, args.scroll);
    let snapshot = terminal.render_snapshot()?;
    let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
    std::fs::write(&args.out, png)?;
    println!(
        "kittui-ghostty wrote {} ({}x{} cells, cursor={}, {}, kitty_placements={}, inner_exit_status={})",
        args.out.display(),
        snapshot.cols,
        snapshot.rows,
        snapshot.cursor_x,
        snapshot.cursor_y,
        snapshot.kitty_placements.len(),
        inner_exit_status
            .map(|code| code.to_string())
            .unwrap_or_else(|| "n/a".to_string())
    );
    Ok(())
}

fn render_timelapse_demo(args: &Args) -> anyhow::Result<()> {
    let chunks = timelapse_demo_steps().iter().copied().collect::<Vec<_>>();
    render_timelapse_chunks(args, chunks)
}

fn render_pty_timelapse_command(args: &Args, command: &str) -> anyhow::Result<()> {
    let pty_input = args
        .pty_input
        .as_deref()
        .map(decode_pty_input)
        .transpose()?;
    let (bytes, _inner_exit_status) = pty_command_bytes_with_input(
        command,
        args.cols,
        args.rows,
        pty_input.as_deref(),
        args.pty_input_delay_ms,
    )?;
    let chunks = line_chunks(&bytes, args.chunk_lines);
    render_timelapse_chunks(args, chunks)
}

fn render_pty_sampled_command(args: &Args, command: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.out_dir)?;
    let pty_input = args
        .pty_input
        .as_deref()
        .map(decode_pty_input)
        .transpose()?;
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: args.rows,
        cols: args.cols,
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
    cmd.env("COLUMNS", args.cols.to_string());
    cmd.env("LINES", args.rows.to_string());
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = if pty_input.is_some() {
        Some(pair.master.take_writer()?)
    } else {
        None
    };
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let (bytes_tx, bytes_rx) = mpsc::channel::<Vec<u8>>();
    let reader_handle = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if bytes_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut terminal = GhosttyVtTerminal::new(args.cols, args.rows, 1_000)?;
    let mut frames = Vec::new();
    let started = Instant::now();
    let sample = Duration::from_millis(args.sample_ms.max(1));
    let max_duration = Duration::from_millis(args.max_ms.max(args.sample_ms.max(1)));
    let mut input_sent = false;
    let mut status: Option<u32> = None;
    let mut idx = 0usize;
    loop {
        match bytes_rx.recv_timeout(sample) {
            Ok(bytes) => terminal.write(&bytes),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
        while let Ok(bytes) = bytes_rx.try_recv() {
            terminal.write(&bytes);
        }
        if !input_sent && started.elapsed() >= Duration::from_millis(args.pty_input_delay_ms) {
            if let (Some(input), Some(writer)) = (pty_input.as_deref(), writer.as_mut()) {
                writer.write_all(input)?;
                writer.flush()?;
            }
            input_sent = true;
        }
        if status.is_none() {
            if let Some(exit) = child.try_wait()? {
                status = Some(exit.exit_code());
            }
        }
        let snapshot = terminal.render_snapshot()?;
        let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
        let path = args.out_dir.join(frame_png_name(idx));
        std::fs::write(&path, png)?;
        frames.push(FrameRecord {
            index: idx,
            path,
            cursor_x: snapshot.cursor_x,
            cursor_y: snapshot.cursor_y,
            kitty_placements: snapshot.kitty_placements.len(),
        });
        idx += 1;
        if status.is_some() {
            break;
        }
        if started.elapsed() >= max_duration {
            let _ = child.kill();
            status = child.wait().ok().map(|status| status.exit_code());
            while let Ok(bytes) = bytes_rx.try_recv() {
                terminal.write(&bytes);
            }
            break;
        }
    }
    drop(bytes_rx);
    let _ = reader_handle.join();
    write_manifest(&args.out_dir, &frames)?;
    if let Some(path) = &args.montage {
        write_montage(path, &frames)?;
    }
    println!(
        "kittui-ghostty wrote {} sampled frames to {} (status={:?})",
        frames.len(),
        args.out_dir.display(),
        status
    );
    Ok(())
}

fn frame_png_name(idx: usize) -> String {
    let mut name = String::with_capacity("frame-.png".len() + 3.max(decimal_len_usize(idx)));
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

fn decimal_len_usize(mut value: usize) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

fn render_timelapse_chunks(args: &Args, chunks: Vec<&[u8]>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.out_dir)?;
    let mut terminal = GhosttyVtTerminal::new(args.cols, args.rows, 1_000)?;
    let mut frames = Vec::new();
    for (idx, bytes) in chunks.iter().enumerate() {
        terminal.write(bytes);
        let snapshot = terminal.render_snapshot()?;
        let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
        let path = args.out_dir.join(frame_png_name(idx));
        std::fs::write(&path, png)?;
        frames.push(FrameRecord {
            index: idx,
            path,
            cursor_x: snapshot.cursor_x,
            cursor_y: snapshot.cursor_y,
            kitty_placements: snapshot.kitty_placements.len(),
        });
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
    let mut kittwm_proof_command = None;
    let mut pty_timelapse_command = None;
    let mut pty_sampled_command = None;
    let mut pty_input = None;
    let mut pty_input_delay_ms = 100u64;
    let mut sample_ms = 250u64;
    let mut max_ms = 10_000u64;
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
            "--kittwm-proof-command" => {
                kittwm_proof_command = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--kittwm-proof-command COMMAND"))?,
                );
            }
            "--pty-timelapse-command" => {
                pty_timelapse_command = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--pty-timelapse-command COMMAND"))?,
                );
            }
            "--pty-sampled-command" => {
                pty_sampled_command = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--pty-sampled-command COMMAND"))?,
                );
            }
            "--pty-input" => {
                pty_input = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--pty-input TEXT"))?,
                );
            }
            "--pty-input-delay-ms" => {
                pty_input_delay_ms = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--pty-input-delay-ms N"))?
                    .parse()?;
            }
            "--sample-ms" => {
                sample_ms = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--sample-ms N"))?
                    .parse()?;
            }
            "--max-ms" => {
                max_ms = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--max-ms N"))?
                    .parse()?;
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
        kittwm_proof_command,
        pty_timelapse_command,
        pty_sampled_command,
        pty_input,
        pty_input_delay_ms,
        sample_ms,
        max_ms,
        scroll,
    })
}

fn input_bytes(args: &Args) -> anyhow::Result<(Vec<u8>, Option<u32>)> {
    let command_modes = [
        args.command.is_some(),
        args.pty_command.is_some(),
        args.kittwm_proof_command.is_some(),
        args.pty_sampled_command.is_some(),
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count();
    if command_modes > 1 {
        anyhow::bail!(
            "--command, --pty-command, --kittwm-proof-command, and --pty-sampled-command are mutually exclusive"
        );
    }
    let pty_input = args
        .pty_input
        .as_deref()
        .map(decode_pty_input)
        .transpose()?;
    if let Some(command) = &args.kittwm_proof_command {
        return pty_command_bytes_with_env_and_input(
            command,
            args.cols,
            args.rows,
            kittwm_proof_env(),
            pty_input.as_deref(),
            args.pty_input_delay_ms,
            true,
        );
    }
    if let Some(command) = &args.pty_command {
        return pty_command_bytes_with_input(
            command,
            args.cols,
            args.rows,
            pty_input.as_deref(),
            args.pty_input_delay_ms,
        );
    }
    if let Some(command) = &args.command {
        return command_bytes(command).map(|bytes| (bytes, None));
    }

    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    if args.demo || input.is_empty() {
        input = demo_bytes();
    }
    Ok((input, None))
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

fn pty_command_bytes_with_input(
    command: &str,
    cols: u16,
    rows: u16,
    pty_input: Option<&[u8]>,
    pty_input_delay_ms: u64,
) -> anyhow::Result<(Vec<u8>, Option<u32>)> {
    pty_command_bytes_with_env_and_input(
        command,
        cols,
        rows,
        [],
        pty_input,
        pty_input_delay_ms,
        false,
    )
}

fn pty_command_bytes_with_env_and_input<const N: usize>(
    command: &str,
    cols: u16,
    rows: u16,
    extra_env: [(&'static str, &'static str); N],
    pty_input: Option<&[u8]>,
    pty_input_delay_ms: u64,
    prefer_pre_input_snapshot: bool,
) -> anyhow::Result<(Vec<u8>, Option<u32>)> {
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
    for (key, value) in extra_env {
        cmd.env(key, value);
    }

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = if pty_input.is_some() {
        Some(pair.master.take_writer()?)
    } else {
        None
    };
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let shared_output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let reader_output = Arc::clone(&shared_output);
    let handle = thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut output) = reader_output.lock() {
                        output.extend_from_slice(&chunk[..n]);
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut pre_input_output = None;
    if let (Some(input), Some(writer)) = (pty_input, writer.as_mut()) {
        if pty_input_delay_ms > 0 {
            thread::sleep(std::time::Duration::from_millis(pty_input_delay_ms));
        }
        if prefer_pre_input_snapshot {
            pre_input_output = Some(
                shared_output
                    .lock()
                    .map(|output| output.clone())
                    .unwrap_or_default(),
            );
        }
        use std::io::Write as _;
        writer.write_all(input)?;
        writer.flush()?;
    }
    let status = child.wait()?;
    let exit_code = status.exit_code();
    drop(child);
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("PTY reader thread panicked"))?;
    let output = shared_output
        .lock()
        .map(|output| output.clone())
        .unwrap_or_default();
    let output = choose_pty_snapshot_output(pre_input_output, output);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"$ ");
    bytes.extend_from_slice(command.as_bytes());
    bytes.extend_from_slice(b"\r\n");
    bytes.extend_from_slice(&output);
    if !status.success() {
        bytes.extend_from_slice(format!("\r\n[exit {}]\r\n", status.exit_code()).as_bytes());
    }
    Ok((bytes, Some(exit_code)))
}

fn choose_pty_snapshot_output(pre_input_output: Option<Vec<u8>>, output: Vec<u8>) -> Vec<u8> {
    pre_input_output
        .filter(|snapshot| !snapshot.is_empty())
        .unwrap_or(output)
}

fn decode_pty_input(input: &str) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.extend_from_slice(ch.to_string().as_bytes());
            continue;
        }
        match chars.next() {
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('e') => out.push(0x1b),
            Some('x') => {
                let hi = chars
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--pty-input has incomplete \\xHH escape"))?;
                let lo = chars
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--pty-input has incomplete \\xHH escape"))?;
                let hex = format!("{hi}{lo}");
                out.push(u8::from_str_radix(&hex, 16)?);
            }
            Some('\\') => out.push(b'\\'),
            Some(other) => anyhow::bail!("unsupported --pty-input escape \\{other}"),
            None => anyhow::bail!("--pty-input ends with a trailing backslash"),
        }
    }
    Ok(out)
}

fn kittwm_proof_env() -> [(&'static str, &'static str); 7] {
    [
        ("KITTWM_NATIVE_RENDERER", "terminal"),
        ("KITTWM_NATIVE_CHROME_RENDERER", "terminal"),
        ("KITTWM_DISABLE_NATIVE_GRAPHICS", "1"),
        ("KITTWM_STARTUP_TERMINAL", "0"),
        ("KITTWM_BROWSER_FRAME", "0"),
        ("TERM_PROGRAM", "kittui-ghostty-proof"),
        ("NO_COLOR", "1"),
    ]
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
           kittui-ghostty --kittwm-proof-command COMMAND [--pty-input TEXT] [--out PATH] [--cols N] [--rows N] [--scroll top|bottom|current]\n\
           kittui-ghostty --pty-timelapse-command COMMAND [--pty-input TEXT] [--out-dir DIR] [--montage PATH] [--cols N] [--rows N] [--chunk-lines N]\n\
           kittui-ghostty --pty-sampled-command COMMAND [--pty-input TEXT] [--sample-ms N] [--max-ms N] [--out-dir DIR] [--montage PATH] [--cols N] [--rows N]\n\
           kittui-ghostty --timelapse-demo [--out-dir DIR] [--montage PATH] [--cols N] [--rows N]\n\n\
         Reads VT bytes from stdin. If stdin is empty or --demo is passed, renders demo content.\n\
         --command/-c runs COMMAND through sh -c and renders stdout/stderr.\n\
         --pty-command runs COMMAND in a PTY sized by --cols/--rows and renders captured VT bytes.\n\
         --kittwm-proof-command runs COMMAND in a PTY with kittwm-friendly terminal-renderer env for real screenshot proof artifacts.\n\
         --pty-input sends escaped interactive input after spawn (\\r, \\n, \\t, \\e, \\x1d); tune with --pty-input-delay-ms.\n\
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

fn write_montage(path: &Path, frames: &[FrameRecord]) -> anyhow::Result<()> {
    let selected = montage_frame_indices(frames.len());
    let mut entries = Vec::new();
    for idx in selected {
        let frame = &frames[idx];
        let bytes = std::fs::read(&frame.path)?;
        let image = image::load_from_memory(&bytes)?.to_rgba8();
        let label = format!(
            "frame-{:03}.png cursor={},{} kitty_placements={}",
            frame.index, frame.cursor_x, frame.cursor_y, frame.kitty_placements
        );
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

fn write_manifest(out_dir: &Path, frames: &[FrameRecord]) -> anyhow::Result<()> {
    let files = frames
        .iter()
        .map(|frame| {
            format!(
                "{{\"index\":{},\"path\":{:?},\"cursor_x\":{},\"cursor_y\":{},\"kitty_placements\":{}}}",
                frame.index,
                frame.path.display().to_string(),
                frame.cursor_x,
                frame.cursor_y,
                frame.kitty_placements
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kittwm_proof_env_forces_terminal_rendering() {
        let env = kittwm_proof_env();
        assert!(env.contains(&("KITTWM_NATIVE_RENDERER", "terminal")));
        assert!(env.contains(&("KITTWM_NATIVE_CHROME_RENDERER", "terminal")));
        assert!(env.contains(&("KITTWM_STARTUP_TERMINAL", "0")));
        assert!(env.contains(&("TERM_PROGRAM", "kittui-ghostty-proof")));
    }

    #[test]
    fn frame_png_name_builds_directly() {
        let first = frame_png_name(0);
        assert_eq!(first, "frame-000.png");
        assert_eq!(first.capacity(), first.len());
        let later = frame_png_name(42);
        assert_eq!(later, "frame-042.png");
        assert_eq!(later.capacity(), later.len());
        let wide = frame_png_name(1234);
        assert_eq!(wide, "frame-1234.png");
        assert_eq!(wide.capacity(), wide.len());
        assert_eq!(decimal_len_usize(0), 1);
        assert_eq!(decimal_len_usize(9), 1);
        assert_eq!(decimal_len_usize(10), 2);
    }

    #[test]
    fn frame_manifest_includes_kitty_placement_count() {
        let dir = std::env::temp_dir().join(format!(
            "kittui-ghostty-manifest-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let frame_path = dir.join("frame-000.png");
        let frame = FrameRecord {
            index: 0,
            path: frame_path,
            cursor_x: 2,
            cursor_y: 3,
            kitty_placements: 4,
        };
        write_manifest(&dir, &[frame]).unwrap();
        let manifest = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
        assert!(manifest.contains("\"kitty_placements\":4"), "{manifest}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn choose_pty_snapshot_output_prefers_nonempty_pre_input_snapshot() {
        assert_eq!(
            choose_pty_snapshot_output(
                Some(b"live alt-screen".to_vec()),
                b"restored shell".to_vec()
            ),
            b"live alt-screen".to_vec()
        );
        assert_eq!(
            choose_pty_snapshot_output(Some(Vec::new()), b"restored shell".to_vec()),
            b"restored shell".to_vec()
        );
        assert_eq!(
            choose_pty_snapshot_output(None, b"full output".to_vec()),
            b"full output".to_vec()
        );
    }

    #[test]
    fn decode_pty_input_supports_interactive_escapes() {
        assert_eq!(decode_pty_input(r"hello\r\n").unwrap(), b"hello\r\n");
        assert_eq!(
            decode_pty_input(r"tab\tquit\x1d").unwrap(),
            b"tab\tquit\x1d"
        );
        assert_eq!(decode_pty_input(r"esc\e").unwrap(), b"esc\x1b");
        assert_eq!(decode_pty_input(r"slash\\").unwrap(), b"slash\\");
    }

    #[test]
    fn decode_pty_input_rejects_malformed_escapes() {
        assert!(decode_pty_input(r"bad\q").is_err());
        assert!(decode_pty_input(r"bad\x1").is_err());
        assert!(decode_pty_input(r"bad\").is_err());
    }
}
