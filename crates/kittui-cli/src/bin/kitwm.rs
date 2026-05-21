//! `kitwm` — the kittui window manager launcher.
//!
//! With no args, opens a kittui-wm session in the current terminal,
//! picking the best available backend (Quartz on macOS, Xvfb on Linux,
//! `FakeServer` otherwise). Survives terminal restoration on
//! SIGINT/HUP/TERM/QUIT via the shared `kittui_cli::session` module.
//!
//! Flags:
//!
//! ```text
//! kitwm              # open a session in the current terminal
//! kitwm --serve      # run only the (in-process today) backend host loop
//! kitwm --attach     # attach to an existing daemon (placeholder; bd-fb5d9d)
//! kitwm --kill       # send shutdown to the daemon (placeholder; bd-fb5d9d)
//! kitwm --status     # print whether a daemon is running (placeholder)
//! kitwm --backend X  # force a specific backend: fake | quartz | xvfb
//! ```
//!
//! Once the daemon/client split (bd-fb5d9d) lands, `--serve` becomes a
//! `fork + setsid + exec` of the daemon and `kitwm` (no args) attaches
//! to the running socket transparently.
//!
//! The end-goal acceptance criterion is that `kitwm` opens a usable
//! session with an app launcher that can spawn an X11 app (xterm via
//! XQuartz on macOS, xterm via Xvfb on Linux) and route keystrokes into
//! it. See bead bd-a9ec5b.

use std::process::ExitCode;

use anyhow::{anyhow, Result};

use kittui::{CellSize, Runtime, TerminalInfo};
use kittui_core::geom::PxRect;
use kittui_wm::compositor::{Compositor, Layout, WindowMode};
use kittui_xvfb::{FakeServer, XServer, XWindowId};

#[derive(Debug, Default)]
struct Cli {
    mode: Mode,
    backend: Option<Backend>,
    pick_window: bool,
    list_windows: bool,
    list_displays: bool,
    capture: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Session,
    Serve,
    Attach,
    Kill,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Fake,
    Quartz,
    Xvfb,
}

fn parse_args() -> Result<Cli> {
    let mut args = std::env::args().skip(1);
    let mut out = Cli::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--serve" => out.mode = Mode::Serve,
            "--attach" => out.mode = Mode::Attach,
            "--kill" => out.mode = Mode::Kill,
            "--status" => out.mode = Mode::Status,
            "--backend" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--backend requires a value"))?;
                out.backend = Some(match v.as_str() {
                    "fake" => Backend::Fake,
                    "quartz" => Backend::Quartz,
                    "xvfb" => Backend::Xvfb,
                    other => return Err(anyhow!("unknown backend {other}")),
                });
            }
            "--pick-window" => out.pick_window = true,
            "--list-windows" => out.list_windows = true,
            "--list-displays" => out.list_displays = true,
            "--capture" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--capture requires a target spec"))?;
                out.capture = Some(v);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown arg {other}")),
        }
    }
    Ok(out)
}

fn print_help() {
    println!(
        "kitwm — kittui window manager\n\n\
         Usage: kitwm [--serve | --attach | --kill | --status] [--backend fake|quartz|xvfb]\n\n\
         Default: open a kittui-wm session in the current terminal, picking the\n\
         best available backend. q or Esc to quit.\n\n\
         --serve   run the in-process backend host (today same as no args; will\n\
                   become 'fork daemon + listen on $KITTUI_WM_DISPLAY socket' in\n\
                   bd-fb5d9d).\n\
         --attach  attach to an existing daemon (placeholder until bd-fb5d9d).\n\
         --kill    send shutdown to the daemon (placeholder).\n\
         --status  print whether a daemon is running (placeholder).\n\
         --backend fake|quartz|xvfb force a specific backend.\n\
         --pick-window   (macOS+quartz) live picker over CGWindowList; pick\n\
                         one window, then run a kitwm session capturing only it.\n\
         --list-windows  (macOS+quartz) print every titled CGWindow with id,\n\
                         bounds, owner, title — useful for scripting kitwm.\n\
         --list-displays (macOS+quartz) print every connected CGDirectDisplayID\n\
                         with bounds + index.\n\
         --capture SPEC  (macOS+quartz) capture a specific source non-interactively:\n\
                         'main'                 = main display (default)\n\
                         'display:<n>'          = nth connected display\n\
                         'window:<substring>'   = first matching app window title/owner\n\
                         'all'                  = all displays as a multi-window session\n"
    );
}

fn pick_backend(forced: Option<Backend>) -> Backend {
    if let Some(b) = forced {
        return b;
    }
    // Auto-pick by host. Order matters: prefer the richest available
    // backend (Quartz on macOS with sck, Xvfb on Linux), falling back
    // to FakeServer everywhere.
    #[cfg(all(target_os = "macos", feature = "quartz"))]
    {
        return Backend::Quartz;
    }
    #[cfg(all(not(all(target_os = "macos", feature = "quartz")), feature = "xvfb"))]
    {
        return Backend::Xvfb;
    }
    #[cfg(not(any(
        all(target_os = "macos", feature = "quartz"),
        feature = "xvfb"
    )))]
    {
        Backend::Fake
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kitwm: {e}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let cli = parse_args()?;

    // Inspection flags run cooked, never enter raw mode.
    if cli.list_windows {
        return list_windows_cmd();
    }
    if cli.list_displays {
        return list_displays_cmd();
    }

    match cli.mode {
        Mode::Session | Mode::Serve => run_session(cli),
        Mode::Attach => Err(anyhow!(
            "--attach is a placeholder until the kittui-wm daemon lands (bd-fb5d9d). \
             Run `kitwm` with no args to open a session in this terminal."
        )),
        Mode::Kill => Err(anyhow!(
            "--kill is a placeholder until the kittui-wm daemon lands (bd-fb5d9d)."
        )),
        Mode::Status => {
            println!("kitwm: no daemon yet (bd-fb5d9d). Single-session in-process mode is active.");
            Ok(())
        }
    }
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn list_windows_cmd() -> Result<()> {
    use kittui_quartz::QuartzServer;
    let wins = QuartzServer::list_app_windows();
    println!("{:>8}  {:<24}  {:<48}  bounds", "id", "owner", "title");
    for w in wins {
        println!(
            "{:>8}  {:<24}  {:<48}  ({:.0},{:.0}) {:.0}x{:.0}",
            w.id,
            truncate(&w.owner_name, 24),
            truncate(&w.title, 48),
            w.bounds.origin.0,
            w.bounds.origin.1,
            w.bounds.width,
            w.bounds.height,
        );
    }
    Ok(())
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn list_windows_cmd() -> Result<()> {
    Err(anyhow!("--list-windows requires --features quartz on macOS"))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn list_displays_cmd() -> Result<()> {
    use kittui_quartz::QuartzServer;
    let displays = QuartzServer::displays();
    println!("{:>3}  {:>10}  bounds", "#", "id");
    for d in displays {
        println!(
            "{:>3}  {:>10}  ({:.0},{:.0}) {:.0}x{:.0}",
            d.index,
            d.id,
            d.bounds.origin.0,
            d.bounds.origin.1,
            d.bounds.width,
            d.bounds.height,
        );
    }
    Ok(())
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn list_displays_cmd() -> Result<()> {
    Err(anyhow!("--list-displays requires --features quartz on macOS"))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn run_session(cli: Cli) -> Result<()> {
    let cell = CellSize::default();
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;
    let backend = pick_backend(cli.backend);

    match backend {
        Backend::Fake => run_with_fake(&runtime, cell),
        #[cfg(all(target_os = "macos", feature = "quartz"))]
        Backend::Quartz => run_with_quartz(&runtime, cell, cli.pick_window, cli.capture.as_deref()),
        #[cfg(all(not(all(target_os = "macos", feature = "quartz")), feature = "xvfb"))]
        Backend::Xvfb => run_with_xvfb(&runtime, cell),
        #[cfg(not(all(target_os = "macos", feature = "quartz")))]
        Backend::Quartz => Err(anyhow!(
            "Quartz backend requires --features quartz on macOS"
        )),
        #[cfg(not(feature = "xvfb"))]
        Backend::Xvfb => Err(anyhow!("Xvfb backend requires --features xvfb on Linux")),
    }
}

fn run_with_fake(runtime: &Runtime, cell: CellSize) -> Result<()> {
    // Show a tiny gallery so `kitwm` (no args) always renders something
    // visible on any host, even without Quartz/Xvfb permissions.
    let server = FakeServer::with_windows(vec![
        (
            XWindowId(1),
            PxRect::new(8.0, 16.0, 256.0, 160.0),
            "welcome",
            [0x00, 0xd8, 0xff, 0xff],
        ),
        (
            XWindowId(2),
            PxRect::new(320.0, 16.0, 256.0, 160.0),
            "press q to quit",
            [0xb4, 0x8c, 0xff, 0xff],
        ),
    ]);
    let compositor = Compositor::new(server, cell);
    compositor.set_mode(XWindowId(1), WindowMode::Tiled);
    let mut layout = Layout::all_floating();
    layout.tile(
        XWindowId(1),
        PxRect::new(8.0, 16.0, 320.0, 192.0),
    );
    kittui_cli::session::run_loop(runtime, &compositor, &layout)
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn run_with_quartz(
    runtime: &Runtime,
    cell: CellSize,
    pick_window: bool,
    capture: Option<&str>,
) -> Result<()> {
    use kittui_quartz::{CaptureTarget, QuartzServer};

    let target = if pick_window {
        let windows = QuartzServer::list_app_windows();
        let chosen = prompt_pick(&windows)?;
        eprintln!(
            "kitwm: capturing window {} ({}: {})",
            chosen.id, chosen.owner_name, chosen.title
        );
        CaptureTarget::Window(chosen.id)
    } else if let Some(spec) = capture {
        resolve_capture_spec(spec)?
    } else {
        CaptureTarget::MainDisplay
    };

    let mut server = QuartzServer::with_target(target);
    let max_w = 80u32 * cell.width_px as u32 * 2;
    let max_h = 24u32 * cell.height_px as u32 * 2;
    server.set_max_size(Some((max_w, max_h)));

    eprintln!("kitwm: probing macOS Screen Recording permission...");
    let probe = server.windows().and_then(|w| {
        if let Some(first) = w.first() {
            server.capture(first.id).map(|_| ())
        } else {
            Err(kittui_xvfb::XError::Unavailable("no displays".into()))
        }
    });
    if let Err(e) = probe {
        return Err(anyhow!(
            "kitwm could not capture the screen: {e}\n\n  Grant Screen Recording \
             to your terminal under System Settings -> Privacy & Security -> \
             Screen Recording, then quit and relaunch the terminal."
        ));
    }
    eprintln!("kitwm: backend ready. q/Esc to quit.");
    std::thread::sleep(std::time::Duration::from_millis(600));

    let compositor = Compositor::new(server, cell);
    let mut layout = Layout::all_floating();
    if let Ok(ws) = compositor.server().windows() {
        if let Some(w) = ws.first() {
            layout.tile(
                w.id,
                PxRect::new(
                    0.0,
                    0.0,
                    80.0 * cell.width_px as f32,
                    24.0 * cell.height_px as f32,
                ),
            );
            compositor.set_mode(w.id, WindowMode::Tiled);
        }
    }
    kittui_cli::session::run_loop(runtime, &compositor, &layout)
}

#[cfg(all(not(all(target_os = "macos", feature = "quartz")), feature = "xvfb"))]
fn run_with_xvfb(runtime: &Runtime, cell: CellSize) -> Result<()> {
    let display: u32 = std::env::var("KITTUI_WM_DISPLAY")
        .ok()
        .and_then(|s| s.trim_start_matches(':').parse().ok())
        .unwrap_or(99);
    let server = kittui_xvfb::xvfb::XvfbServer::spawn(display)
        .map_err(|e| anyhow!("XvfbServer::spawn: {e}"))?;
    let compositor = Compositor::new(server, cell);
    let layout = Layout::all_floating();
    kittui_cli::session::run_loop(runtime, &compositor, &layout)
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn prompt_pick(windows: &[kittui_quartz::MacWindow]) -> Result<kittui_quartz::MacWindow> {
    use std::io::{BufRead, Write};
    if windows.is_empty() {
        return Err(anyhow!(
            "no macOS app windows visible via CGWindowList; nothing to pick"
        ));
    }
    println!("\nkitwm --pick-window\n");
    for (i, w) in windows.iter().enumerate() {
        println!(
            "  [{:>2}]  {:<24}  {:<48}  ({:.0},{:.0}) {:.0}x{:.0}",
            i,
            truncate(&w.owner_name, 24),
            truncate(&w.title, 48),
            w.bounds.origin.0,
            w.bounds.origin.1,
            w.bounds.width,
            w.bounds.height,
        );
    }
    print!("\nNumber to capture (Enter to cancel): ");
    std::io::stdout().flush().ok();
    let stdin = std::io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line).ok();
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("cancelled by operator"));
    }
    let idx: usize = trimmed
        .parse()
        .map_err(|_| anyhow!("expected a number, got {trimmed:?}"))?;
    windows
        .get(idx)
        .cloned()
        .ok_or_else(|| anyhow!("out of range; pick 0..{}", windows.len()))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn resolve_capture_spec(spec: &str) -> Result<kittui_quartz::CaptureTarget> {
    use kittui_quartz::{CaptureTarget, QuartzServer};
    if spec == "main" {
        return Ok(CaptureTarget::MainDisplay);
    }
    if spec == "all" {
        return Ok(CaptureTarget::AllDisplays);
    }
    if let Some(n) = spec.strip_prefix("display:") {
        let idx: usize = n
            .parse()
            .map_err(|_| anyhow!("display:N expects an integer, got {n:?}"))?;
        let displays = QuartzServer::displays();
        let chosen = displays
            .get(idx)
            .ok_or_else(|| anyhow!("display index {idx} out of range (0..{})", displays.len()))?;
        return Ok(CaptureTarget::Display(chosen.id));
    }
    if let Some(needle) = spec.strip_prefix("window:") {
        let needle_lc = needle.to_ascii_lowercase();
        let windows = QuartzServer::list_app_windows();
        let chosen = windows
            .iter()
            .find(|w| {
                w.title.to_ascii_lowercase().contains(&needle_lc)
                    || w.owner_name.to_ascii_lowercase().contains(&needle_lc)
            })
            .ok_or_else(|| {
                anyhow!(
                    "no Mac window matched 'window:{needle}'; run `kitwm --list-windows` to see candidates"
                )
            })?;
        eprintln!(
            "kitwm: --capture window:{} matched id={} owner={:?} title={:?}",
            needle, chosen.id, chosen.owner_name, chosen.title
        );
        return Ok(CaptureTarget::Window(chosen.id));
    }
    Err(anyhow!(
        "unknown --capture spec {spec:?}. Use: main | all | display:<n> | window:<substr>"
    ))
}
