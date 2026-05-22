//! kittui-wm session loop.
//!
//! Owns the full UI thread for a kittui-wm viewer:
//!
//! - terminal raw mode + alt screen + SGR mouse reporting (RAII-restored
//!   even on SIGINT/HUP/QUIT/TERM)
//! - stdin draining + `kittui_input` parsing
//! - `MultiCompositor`-friendly compose + place pipeline (currently
//!   wired against the single-backend `Compositor` for back-compat;
//!   migrates when the Pump architecture lands)
//! - file-based debug log so the agent and the operator can introspect
//!   without escapes interfering with the live render
//!
//! Both the `kittui_wm_demo` example and the `kittwm` binary call into
//! [`run_loop`].

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use anyhow::Result;

use kittui::{CellRect, Runtime};
use kittui_input::{InputEvent, Key};
use kittui_wm::compositor::{Compositor, Layout};
use kittui_wm::native::{NativeApp, NativeFrame, PtyTerminalApp};
use kittui_xvfb::XServer;

use crate::keymap::{Action, KeyMods, KeySpec, Keymap};

/// Drive the kittui-wm UI loop until the operator quits.
///
/// `compositor` and `layout` are passed in so callers can wire any
/// `XServer` backend (FakeServer, Xvfb, Quartz, XQuartz, ...) without
/// this module knowing about backends.
pub fn run_native_terminal_loop(runtime: &Runtime) -> Result<()> {
    let dbg = Debugger::open();
    dbg.log("native terminal loop: enter");
    let _raw_guard = RawMode::enter()?;
    install_signal_restore();

    let (mut cols, mut rows) = native_terminal_size();
    let sock = crate::daemon::default_socket_path()
        .to_string_lossy()
        .to_string();
    let window = "native-1".to_string();
    let cmd = std::env::var("KITTWM_TERMINAL_CMD")
        .or_else(|_| std::env::var("SHELL").map(|s| format!("{s} -l")))
        .unwrap_or_else(|_| "/bin/sh -l".to_string());
    let mut app = PtyTerminalApp::spawn_with_env(
        &cmd,
        cols,
        rows,
        [
            ("KITTWM_SOCKET", sock.as_str()),
            ("KITTWM_SOCK", sock.as_str()),
            ("KITTUI_WM_DISPLAY", sock.as_str()),
            ("KITTWM_DISPLAY", sock.as_str()),
            ("KITTWM_WINDOW", window.as_str()),
        ],
    )?;
    let fps = std::env::var("KITTUI_WM_FPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30u32)
        .clamp(1, 120);
    let frame_target = Duration::from_micros(1_000_000 / fps as u64);
    let mut stdin = io::stdin();
    let mut frame = 0u64;
    let mut placed = false;
    loop {
        let frame_start = Instant::now();
        let mut chunk = [0u8; 1024];
        while poll_stdin(Duration::ZERO) {
            let n = stdin.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            if chunk[..n].contains(&0x1d) {
                dbg.log("native terminal loop: Ctrl-] exit");
                return Ok(());
            }
            app.send_bytes(&chunk[..n])?;
        }

        let (new_cols, new_rows) = native_terminal_size();
        if (new_cols, new_rows) != (cols, rows) {
            cols = new_cols;
            rows = new_rows;
            app.resize(cols, rows)?;
            placed = false;
            dbg.log(&format!("native terminal resized to {cols}x{rows}"));
        }

        match app.capture()? {
            NativeFrame::Rgba {
                width,
                height,
                rgba,
            } => {
                let footprint = CellRect::new(0, 0, cols, rows);
                let p = runtime.place_raw_frame(1, &rgba, width, height, footprint);
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                handle.write_all(p.upload.as_bytes())?;
                if !placed {
                    write!(handle, "\x1b[1;1H")?;
                    handle.write_all(p.placement.as_bytes())?;
                    handle.write_all(p.embed.as_bytes())?;
                    placed = true;
                }
                write!(
                    handle,
                    "\x1b[{};1H\x1b[Kkittwm native terminal — {} — KITTWM_SOCKET={} — Ctrl-] exits — frame {} (log: {})",
                    rows + 2,
                    app.title(),
                    sock,
                    frame,
                    dbg.path_display()
                )?;
                handle.flush()?;
            }
            NativeFrame::Png { .. } => {}
        }
        frame += 1;
        if let Some(slack) = frame_target.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(slack);
        }
    }
}

fn native_terminal_size() -> (u16, u16) {
    let host = host_terminal_cells().unwrap_or((80, 24));
    let cols = std::env::var("KITTWM_NATIVE_COLS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(host.0)
        .max(1);
    let rows = std::env::var("KITTWM_NATIVE_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| host.1.saturating_sub(2).max(1))
        .max(1);
    (cols, rows)
}

fn host_terminal_cells() -> Option<(u16, u16)> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        Some((ws.ws_col, ws.ws_row))
    } else {
        None
    }
}

pub fn run_loop<S: XServer>(
    runtime: &Runtime,
    compositor: &Compositor<S>,
    layout: &Layout,
) -> Result<()> {
    let opts = RunOptions {
        fps: std::env::var("KITTUI_WM_FPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
        launch_on_f12: std::env::var("KITTUI_WM_LAUNCH_ON_F12")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        launcher_overlay: std::env::var("KITTUI_WM_LAUNCHER_OVERLAY")
            .map(|v| {
                !(v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
            })
            .unwrap_or(true),
    };
    run_loop_with(runtime, compositor, layout, opts)
}

/// Tunable runtime options for the kittwm session loop.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Target frames per second. Capped at 240 to keep terminal output sane.
    pub fps: u32,
    /// If true, intercept F12 and spawn the launcher command instead of
    /// forwarding it to the focused backend window.
    pub launch_on_f12: bool,
    /// If true, launch actions open an in-session overlay first.
    pub launcher_overlay: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            fps: 60,
            launch_on_f12: false,
            launcher_overlay: true,
        }
    }
}

pub fn run_loop_with<S: XServer>(
    runtime: &Runtime,
    compositor: &Compositor<S>,
    layout: &Layout,
    opts: RunOptions,
) -> Result<()> {
    let dbg = Debugger::open();
    dbg.log(&format!(
        "run_loop: enter fps={} launch_on_f12={} launcher_overlay={}",
        opts.fps, opts.launch_on_f12, opts.launcher_overlay
    ));
    let _raw_guard = RawMode::enter()?;
    dbg.log("raw mode + alt screen entered");
    install_signal_restore();

    let fps = opts.fps.clamp(1, 240);
    let frame_target = Duration::from_micros(1_000_000 / fps as u64);
    // Live fps tracking: instantaneous over the last 30 frames + peak.
    let mut fps_window_start = std::time::Instant::now();
    let mut fps_window_frames = 0u32;
    let mut live_fps: f32 = 0.0;
    let mut peak_fps: f32 = 0.0;
    let mut frame = 0u64;
    let mut input_buf = Vec::<u8>::with_capacity(256);
    let mut stdin = io::stdin();
    let mut last_launch_pid: Option<u32> = None;
    let mut keymap = load_runtime_keymap(&dbg);
    let mut prefix_active = false;
    let mut last_keymap_action: Option<String> = None;
    let mut workspaces = WorkspaceState::default();
    let mut focus_state = FocusState::default();
    let mut swap_state = SwapState::default();
    let mut toggle_state = ToggleState::default();
    let mut layout_state = LayoutState::default();
    let mut split_state = SplitState::default();
    let mut config_state = ConfigState::default();
    let mut launcher_overlay = LauncherOverlay::default();
    let mut launcher_overlay_was_active = false;
    // Triple-Ctrl-C kill switch (bd-2776ad): single Ctrl-C is forwarded to
    // the focused window like any other key; three within 1s exits cleanly.
    let mut ctrl_c_guard = CtrlCGuard::new();
    // Per-window placement memo: (image_id, footprint) -> placement+embed.
    // We only re-emit placement+placeholder when the footprint or image_id
    // changes. Kitty atomically replaces the image at the same id on each
    // raw RGBA upload, so the picture updates without us clearing the screen.
    let mut last_placed: std::collections::HashMap<u32, (kittui::CellRect, String, String)> =
        std::collections::HashMap::new();
    // Set of window image-ids seen on the previous frame so we can delete
    // ones that disappear without redrawing the whole screen.
    let mut prev_window_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();

    loop {
        let frame_start = Instant::now();

        // Drain any pending stdin BEFORE the expensive compose, so q/Esc
        // takes effect even when a single frame is slow.
        let mut chunk = [0u8; 512];
        while poll_stdin(Duration::ZERO) {
            let n = stdin.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            input_buf.extend_from_slice(&chunk[..n]);
        }
        let mut quit = false;
        while let Some((ev, consumed)) = kittui_input::parse(&input_buf) {
            input_buf.drain(..consumed);
            if launcher_overlay.active {
                match launcher_overlay.handle_event(&ev) {
                    OverlayEvent::Consumed => continue,
                    OverlayEvent::Close => {
                        launcher_overlay.active = false;
                        last_keymap_action = Some("launcher.close".to_string());
                        dbg.log("launcher overlay closed");
                        continue;
                    }
                    OverlayEvent::Launch => {
                        let selection = launcher_overlay.selection();
                        match selection {
                            Some(sel) => match launch_selection(&sel) {
                                Ok(pid) => {
                                    last_launch_pid = Some(pid);
                                    last_keymap_action = Some(format!(
                                        "launcher.launch {}:{}",
                                        sel.kind_name(),
                                        sel.command
                                    ));
                                    dbg.log(&format!(
                                        "launcher overlay selected {:?} {:?} spawned pid={pid}",
                                        sel.kind, sel.command
                                    ));
                                }
                                Err(e) => {
                                    last_keymap_action = Some(format!("launcher.error {e}"));
                                    dbg.log(&format!("launcher overlay launch failed: {e}"));
                                }
                            },
                            None => {
                                last_keymap_action =
                                    Some("launcher.error no candidate".to_string());
                                dbg.log("launcher overlay launch requested with no candidate");
                            }
                        }
                        launcher_overlay.active = false;
                        continue;
                    }
                    OverlayEvent::NotHandled => {}
                }
            }
            if let Some(spec) = key_spec_for_event(&ev) {
                if keymap.prefix.as_ref() == Some(&spec) {
                    prefix_active = true;
                    last_keymap_action = Some("prefix".to_string());
                    dbg.log(&format!("keymap prefix entered: {spec}"));
                    continue;
                }
                if prefix_active {
                    prefix_active = false;
                    if let Some(prefix) = keymap.prefix.as_ref() {
                        let chord = vec![prefix.clone(), spec.clone()];
                        if let Some(action) = keymap.action_for_chord(&chord).cloned() {
                            let action_name = action.to_string();
                            last_keymap_action = Some(action_name.clone());
                            dbg.log(&format!(
                                "keymap action: {} -> {action_name}",
                                chord
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            ));
                            match action {
                                Action::Launch => {
                                    if opts.launcher_overlay {
                                        launcher_overlay.open_from_env();
                                        last_keymap_action = Some("launcher.open".to_string());
                                        dbg.log(&format!(
                                            "launcher overlay opened query={:?}",
                                            launcher_overlay.query
                                        ));
                                    } else {
                                        let selection = launcher_selection();
                                        match spawn_launcher_command() {
                                            Ok(pid) => {
                                                last_launch_pid = Some(pid);
                                                dbg.log(&format!("keymap launcher selected {:?} {:?} spawned pid={pid}", selection.kind, selection.command));
                                            }
                                            Err(e) => {
                                                last_keymap_action =
                                                    Some(format!("launcher.error {e}"));
                                                dbg.log(&format!("keymap launcher failed: {e}"));
                                            }
                                        }
                                    }
                                }
                                Action::SplitVerticalLauncher | Action::SplitHorizontalLauncher => {
                                    let msg = split_state.apply(&action);
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("split action: {msg}"));
                                    if opts.launcher_overlay {
                                        launcher_overlay.open_from_env();
                                        dbg.log(&format!(
                                            "split opened launcher overlay query={:?}",
                                            launcher_overlay.query
                                        ));
                                    } else {
                                        let selection = launcher_selection();
                                        match spawn_launcher_command() {
                                            Ok(pid) => {
                                                last_launch_pid = Some(pid);
                                                dbg.log(&format!("split launcher selected {:?} {:?} spawned pid={pid}", selection.kind, selection.command));
                                            }
                                            Err(e) => {
                                                last_keymap_action =
                                                    Some(format!("launcher.error {e}"));
                                                dbg.log(&format!("split launcher failed: {e}"));
                                            }
                                        }
                                    }
                                }
                                Action::WorkspaceNew
                                | Action::WorkspaceNext
                                | Action::WorkspacePrev => {
                                    let msg = workspaces.apply(&action);
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("workspace action: {msg}"));
                                }
                                Action::FocusLeft
                                | Action::FocusDown
                                | Action::FocusUp
                                | Action::FocusRight => {
                                    let msg = focus_state.apply(&action);
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("focus action: {msg}"));
                                }
                                Action::SwapLeft
                                | Action::SwapDown
                                | Action::SwapUp
                                | Action::SwapRight => {
                                    let msg = swap_state.apply(&action);
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("swap action: {msg}"));
                                }
                                Action::FullscreenToggle | Action::FloatToggle => {
                                    let msg = toggle_state.apply(&action);
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("toggle action: {msg}"));
                                }
                                Action::ToggleSplit | Action::BalanceWindows => {
                                    let msg = layout_state.apply(&action);
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("layout action: {msg}"));
                                }
                                Action::ReloadConfig => {
                                    let loaded = load_runtime_keymap_result(&dbg);
                                    let msg = match loaded {
                                        Ok(new_keymap) => {
                                            keymap = new_keymap;
                                            config_state.reload_ok()
                                        }
                                        Err(e) => {
                                            let msg = config_state.reload_err(&e.to_string());
                                            dbg.log(&format!("keymap reload failed, keeping previous keymap: {e}"));
                                            msg
                                        }
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("config action: {msg}"));
                                }
                                Action::Quit => {
                                    quit = true;
                                    break;
                                }
                                other => {
                                    dbg.log(&format!("keymap action not implemented yet: {other}"));
                                }
                            }
                            continue;
                        }
                    }
                    last_keymap_action = Some(format!("unbound {spec}"));
                    dbg.log(&format!("keymap unbound prefix chord: {spec}"));
                    continue;
                }
            }
            // Intercept Ctrl-C for the triple-press kill switch before
            // forwarding to the focused window. (bd-2776ad)
            if matches!(
                &ev,
                InputEvent::Char { ch: 'c', mods } if mods.ctrl && !mods.alt
            ) {
                let count = ctrl_c_guard.record_press(Instant::now());
                dbg.log(&format!("ctrl-c press #{count} within window"));
                if count >= CTRL_C_TRIGGER {
                    dbg.log("ctrl-c triple-press exit triggered");
                    last_keymap_action = Some("ctrl_c.triple_exit".to_string());
                    quit = true;
                    break;
                }
                // Forward single Ctrl-C to the focused window.
                let _ = compositor.route_key(&ev);
                continue;
            }
            match &ev {
                InputEvent::Char { ch: 'q', .. }
                | InputEvent::Key {
                    key: Key::Escape, ..
                } => {
                    dbg.log("quit event received during pre-compose drain");
                    quit = true;
                    break;
                }
                InputEvent::MousePress { .. }
                | InputEvent::MouseRelease { .. }
                | InputEvent::MouseMove { .. } => {
                    let _ = compositor.route_pointer(&ev);
                }
                InputEvent::Char { ch, .. } => {
                    dbg.log(&format!("char event: {:?}", ch));
                    let _ = compositor.route_key(&ev);
                }
                InputEvent::Key {
                    key: Key::F(12), ..
                } if opts.launch_on_f12 => {
                    if opts.launcher_overlay {
                        launcher_overlay.open_from_env();
                        last_keymap_action = Some("launcher.open".to_string());
                        dbg.log(&format!(
                            "launcher F12 opened overlay query={:?}",
                            launcher_overlay.query
                        ));
                    } else {
                        let selection = launcher_selection();
                        match spawn_launcher_command() {
                            Ok(pid) => {
                                last_launch_pid = Some(pid);
                                dbg.log(&format!(
                                    "launcher F12 selected {:?} {:?} spawned pid={pid}",
                                    selection.kind, selection.command
                                ));
                            }
                            Err(e) => {
                                last_keymap_action = Some(format!("launcher.error {e}"));
                                dbg.log(&format!("launcher F12 failed: {e}"));
                            }
                        }
                    }
                }
                InputEvent::Key { key, .. } => {
                    dbg.log(&format!("key event: {:?}", key));
                    let _ = compositor.route_key(&ev);
                }
                _ => {}
            }
            if consumed == 0 {
                break;
            }
        }
        if quit {
            return Ok(());
        }

        // Drive frame. Use the raw RGBA fast path so PNG encode falls
        // out of the per-frame cost. Errors are surfaced inside the chrome
        // footer instead of bailing, so a TCC failure or backend death
        // never leaks the terminal.
        match compositor.raw_frames(layout) {
            Ok(frames) => {
                let last_window_count = frames.len();
                if frame % 30 == 0 {
                    dbg.log(&format!("frame {frame}: {} raw frames", frames.len()));
                }
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                // If the launcher overlay just closed, erase its text rows
                // and force image placeholders to be re-emitted underneath.
                // Without this, the boxed menu remains burned into the
                // terminal cells even though the overlay state is inactive.
                if launcher_overlay_was_active && !launcher_overlay.active {
                    clear_launcher_overlay_area(&mut handle)?;
                    last_placed.clear();
                }
                // Track which windows are present this frame so we can
                // delete the ones that have disappeared.
                let mut current_ids: std::collections::HashSet<u32> =
                    std::collections::HashSet::with_capacity(frames.len());
                let mut footer_row = 2u16;
                for f in &frames {
                    current_ids.insert(f.image_id);
                    // Detect whether placement (footprint) changed since
                    // last frame for this id. If so, emit a kitty
                    // 'delete by id' first so the placeholder grid is
                    // cleared from its old cells, then place fresh.
                    let footprint_changed = last_placed
                        .get(&f.image_id)
                        .map(|(prev_fp, _, _)| prev_fp != &f.footprint)
                        .unwrap_or(true);
                    if footprint_changed {
                        if last_placed.contains_key(&f.image_id) {
                            handle.write_all(runtime.unplace(f.image_id).as_bytes())?;
                        }
                    }
                    let p = runtime.place_raw_frame(
                        f.image_id,
                        &f.rgba,
                        f.width,
                        f.height,
                        f.footprint,
                    );
                    // Always re-upload (kitty atomically replaces the
                    // image at the same id; no flicker because no clear).
                    handle.write_all(p.upload.as_bytes())?;
                    if footprint_changed {
                        write!(handle, "\x1b[{};{}H", f.footprint.y + 1, f.footprint.x + 1)?;
                        handle.write_all(p.placement.as_bytes())?;
                        handle.write_all(p.embed.as_bytes())?;
                        last_placed.insert(
                            f.image_id,
                            (f.footprint, p.placement.clone(), p.embed.clone()),
                        );
                    }
                    footer_row = footer_row.max(f.footprint.y + f.footprint.rows + 2);
                }
                // Delete any window that disappeared since last frame.
                for old_id in prev_window_ids.difference(&current_ids) {
                    handle.write_all(runtime.unplace(*old_id).as_bytes())?;
                    last_placed.remove(old_id);
                }
                prev_window_ids = current_ids;
                if launcher_overlay.active {
                    launcher_overlay.render(&mut handle)?;
                    footer_row = footer_row.max(12);
                }
                let launch_note = last_launch_pid
                    .map(|pid| format!(" — last launch pid={pid}"))
                    .unwrap_or_default();
                let keymap_note = if prefix_active {
                    " — keymap prefix".to_string()
                } else {
                    last_keymap_action
                        .as_ref()
                        .map(|a| format!(" — action={a}"))
                        .unwrap_or_default()
                };
                write!(
                    handle,
                    "\x1b[{};1H\x1b[Kkittui-wm frame {} — ws {} — panes {} — layout {} — cfg {} — focus {} — swap {} — mode {} — {} windows — {:.0} fps (peak {:.0}, cap {}){}{} — {} (log: {})",
                    footer_row,
                    frame,
                    workspaces.label(),
                    split_state.label(),
                    layout_state.label(),
                    config_state.label(),
                    focus_state.label(),
                    swap_state.label(),
                    toggle_state.label(),
                    last_window_count,
                    live_fps,
                    peak_fps,
                    fps,
                    launch_note,
                    keymap_note,
                    ctrl_c_guard.quit_hint(last_window_count > 0),
                    dbg.path_display()
                )?;
                launcher_overlay_was_active = launcher_overlay.active;
                handle.flush()?;
            }
            Err(e) => {
                let msg = e.to_string();
                dbg.log(&format!("compose err: {msg}"));
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                write!(
                    handle,
                    "\x1b[H\x1b[J\x1b[1mkittui-wm error\x1b[0m\n\n  {}\n\n  q/Esc to quit. On macOS, grant Screen Recording + Accessibility.\n  (log: {})\n",
                    msg,
                    dbg.path_display()
                )?;
                launcher_overlay_was_active = launcher_overlay.active;
                handle.flush()?;
            }
        }

        let elapsed = frame_start.elapsed();
        let remaining = frame_target.checked_sub(elapsed).unwrap_or_default();
        if remaining > Duration::ZERO {
            let mut chunk = [0u8; 512];
            // Brief stdin poll with a 1ms cap so even on a fd that returns
            // ready immediately we don't spin. Skip entirely when the
            // frame budget is already small.
            let poll_budget = remaining.min(Duration::from_millis(1));
            if poll_budget >= Duration::from_micros(500) && poll_stdin(poll_budget) {
                let n = stdin.read(&mut chunk).unwrap_or(0);
                if n > 0 {
                    dbg.log(&format!(
                        "stdin read {n} bytes: {:02x?}",
                        &chunk[..n.min(32)]
                    ));
                    input_buf.extend_from_slice(&chunk[..n]);
                }
            }
            // Sleep out the rest of the frame budget so --fps actually caps,
            // but never longer than ~16ms at a stretch (preserves Ctrl-C
            // responsiveness on a backgrounded stdin).
            loop {
                let used = frame_start.elapsed();
                let Some(slack) = frame_target.checked_sub(used) else {
                    break;
                };
                if slack < Duration::from_micros(500) {
                    break;
                }
                std::thread::sleep(slack);
            }
        } else {
            dbg.log(&format!(
                "frame {frame} budget blown: {} ms (target {} ms)",
                elapsed.as_millis(),
                frame_target.as_millis()
            ));
        }

        frame = frame.wrapping_add(1);
        fps_window_frames += 1;
        let elapsed_w = fps_window_start.elapsed();
        if fps_window_frames >= 30 || elapsed_w >= Duration::from_millis(500) {
            let secs = elapsed_w.as_secs_f32().max(1e-6);
            live_fps = fps_window_frames as f32 / secs;
            if live_fps > peak_fps {
                peak_fps = live_fps;
            }
            fps_window_frames = 0;
            fps_window_start = std::time::Instant::now();
        }
    }
}

/// Append-only log for the kittui-wm session. Stderr is invisible inside
/// the alt screen, so we mirror everything to a file at $KITTUI_WM_LOG
/// (default `/tmp/kittui-wm.log`).
pub struct Debugger {
    file: std::sync::Mutex<Option<std::fs::File>>,
    path: String,
}

impl Debugger {
    /// Open the log file (truncating on each session).
    pub fn open() -> Self {
        let path =
            std::env::var("KITTUI_WM_LOG").unwrap_or_else(|_| "/tmp/kittui-wm.log".to_string());
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .ok();
        if let Some(mut f) = file.as_ref() {
            use std::io::Write;
            let _ = writeln!(f, "kittui-wm log {} (pid {})", clock(), std::process::id());
        }
        Self {
            file: std::sync::Mutex::new(file),
            path,
        }
    }

    /// Append a single log line.
    pub fn log(&self, line: &str) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "[{}] {}", clock(), line);
                let _ = f.flush();
            }
        }
    }

    /// Path the log was opened at.
    pub fn path_display(&self) -> &str {
        &self.path
    }
}

fn clock() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

// --- raw mode + SGR mouse reporting guard ----------------------------------

struct RawMode;

impl RawMode {
    fn enter() -> Result<Self> {
        let mut out = io::stdout();
        // Alt screen + hide cursor, then SGR mouse + motion + focus reporting.
        out.write_all(
            b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h",
        )?;
        out.flush()?;
        #[cfg(unix)]
        unsafe {
            use libc::*;
            let mut term: termios = std::mem::zeroed();
            tcgetattr(STDIN_FILENO, &mut term);
            let mut raw = term;
            raw.c_lflag &= !(ICANON | ECHO);
            raw.c_cc[VMIN] = 0;
            raw.c_cc[VTIME] = 0;
            tcsetattr(STDIN_FILENO, TCSANOW, &raw);
            ORIG_TERM = Some(term);
            RAW_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    #[cfg(unix)]
    {
        use std::sync::atomic::Ordering;
        if !RAW_ACTIVE.swap(false, Ordering::SeqCst) {
            return;
        }
    }
    let mut out = io::stdout();
    let _ = out
        .write_all(b"\x1b[?1006l\x1b[?1004l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l");
    let _ = out.flush();
    #[cfg(unix)]
    unsafe {
        use libc::*;
        let orig = std::ptr::addr_of_mut!(ORIG_TERM);
        if let Some(t) = (*orig).take() {
            tcsetattr(STDIN_FILENO, TCSANOW, &t);
        }
    }
}

static RAW_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
static mut ORIG_TERM: Option<libc::termios> = None;

#[cfg(unix)]
fn install_signal_restore() {
    use libc::*;
    extern "C" fn handler(_sig: i32) {
        restore_terminal();
        std::process::exit(130);
    }
    unsafe {
        for sig in [SIGINT, SIGTERM, SIGHUP, SIGQUIT] {
            signal(sig, handler as *const () as sighandler_t);
        }
    }
}

#[cfg(not(unix))]
fn install_signal_restore() {}

#[cfg(unix)]
fn poll_stdin(timeout: Duration) -> bool {
    use libc::*;
    unsafe {
        let mut fds = pollfd {
            fd: STDIN_FILENO,
            events: POLLIN,
            revents: 0,
        };
        let ms: i32 = timeout.as_millis().min(i32::MAX as u128) as i32;
        let n = poll(&mut fds, 1, ms);
        n > 0 && fds.revents & POLLIN != 0
    }
}

#[cfg(not(unix))]
fn poll_stdin(timeout: Duration) -> bool {
    std::thread::sleep(timeout);
    false
}

/// Return the shell command used by the F12 launcher.
///
/// Defaults to `xterm`; set `KITTWM_LAUNCH_CMD` to override, e.g.
/// `KITTWM_LAUNCH_CMD='open -a Terminal'` or `'/bin/sleep 10'`.
pub fn launcher_command() -> String {
    std::env::var("KITTWM_LAUNCH_CMD").unwrap_or_else(|_| "xterm".to_string())
}

fn spawn_launcher_command() -> Result<u32> {
    let selection = launcher_selection();
    launch_selection(&selection)
}

fn launch_selection(selection: &LauncherSelection) -> Result<u32> {
    let child = match selection.kind {
        LauncherKind::Shell => std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&selection.command)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
        LauncherKind::Path => std::process::Command::new(&selection.command)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
        LauncherKind::MacOsApp => std::process::Command::new("open")
            .arg("-a")
            .arg(&selection.command)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
    };
    Ok(child.id())
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LauncherKind {
    Shell,
    Path,
    MacOsApp,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LauncherSelection {
    kind: LauncherKind,
    command: String,
}

impl LauncherSelection {
    fn kind_name(&self) -> &'static str {
        match self.kind {
            LauncherKind::Shell => "shell",
            LauncherKind::Path => "path",
            LauncherKind::MacOsApp => "macos",
        }
    }
}

fn launcher_selection() -> LauncherSelection {
    if let Ok(query) = std::env::var("KITTUI_WM_LAUNCH_QUERY") {
        if let Some(sel) = first_launcher_candidate(&query) {
            return sel;
        }
    }
    LauncherSelection {
        kind: LauncherKind::Shell,
        command: launcher_command(),
    }
}

fn first_launcher_candidate(query: &str) -> Option<LauncherSelection> {
    let q = query.to_ascii_lowercase();
    for cmd in path_commands(5000) {
        if cmd.to_ascii_lowercase().contains(&q) {
            return Some(LauncherSelection {
                kind: LauncherKind::Path,
                command: cmd,
            });
        }
    }
    #[cfg(target_os = "macos")]
    for app in macos_apps(5000) {
        if app.to_ascii_lowercase().contains(&q) {
            return Some(LauncherSelection {
                kind: LauncherKind::MacOsApp,
                command: app,
            });
        }
    }
    None
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum OverlayEvent {
    Consumed,
    Close,
    Launch,
    NotHandled,
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
struct LauncherOverlay {
    active: bool,
    query: String,
    selected: usize,
}

impl LauncherOverlay {
    fn open_from_env(&mut self) {
        self.active = true;
        self.query = std::env::var("KITTUI_WM_LAUNCH_QUERY").unwrap_or_default();
        self.selected = 0;
    }

    fn handle_event(&mut self, ev: &InputEvent) -> OverlayEvent {
        match ev {
            InputEvent::Char { ch, mods } if !mods.ctrl && !mods.alt => {
                self.query.push(*ch);
                self.selected = 0;
                OverlayEvent::Consumed
            }
            InputEvent::Key {
                key: Key::Backspace,
                ..
            } => {
                self.query.pop();
                self.selected = 0;
                OverlayEvent::Consumed
            }
            InputEvent::Key { key: Key::Up, .. } => {
                self.selected = self.selected.saturating_sub(1);
                OverlayEvent::Consumed
            }
            InputEvent::Key { key: Key::Down, .. } => {
                let max = self.candidates().len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
                OverlayEvent::Consumed
            }
            InputEvent::Key {
                key: Key::Enter, ..
            } => OverlayEvent::Launch,
            InputEvent::Key {
                key: Key::Escape, ..
            } => OverlayEvent::Close,
            _ => OverlayEvent::NotHandled,
        }
    }

    fn candidates(&self) -> Vec<LauncherSelection> {
        let mut out = Vec::new();
        let query = if self.query.is_empty() {
            None
        } else {
            Some(self.query.as_str())
        };
        for cmd in filter_launcher_candidates(path_commands(5000), query, 8) {
            out.push(LauncherSelection {
                kind: LauncherKind::Path,
                command: cmd,
            });
        }
        #[cfg(target_os = "macos")]
        for app in filter_launcher_candidates(macos_apps(5000), query, 8) {
            out.push(LauncherSelection {
                kind: LauncherKind::MacOsApp,
                command: app,
            });
        }
        out.truncate(8);
        out
    }

    fn selection(&self) -> Option<LauncherSelection> {
        let candidates = self.candidates();
        candidates
            .get(self.selected.min(candidates.len().saturating_sub(1)))
            .cloned()
    }

    fn render<W: Write>(&self, handle: &mut W) -> Result<()> {
        let candidates = self.candidates();
        let width = 58usize;
        write!(handle, "\x1b[2;2H┌{}┐", "─".repeat(width))?;
        write!(
            handle,
            "\x1b[3;2H│{:^width$}│",
            "kittwm launcher",
            width = width
        )?;
        write!(handle, "\x1b[4;2H├{}┤", "─".repeat(width))?;
        write!(
            handle,
            "\x1b[5;2H│ query: {:<qwidth$}│",
            truncate_cells(&self.query, width - 8),
            qwidth = width - 8
        )?;
        write!(handle, "\x1b[6;2H├{}┤", "─".repeat(width))?;
        for row in 0..8usize {
            let line = if let Some(c) = candidates.get(row) {
                let marker = if row == self.selected { "▶" } else { " " };
                format!(
                    "{marker} {:>2}. [{:<5}] {}",
                    row + 1,
                    c.kind_name(),
                    c.command
                )
            } else {
                String::new()
            };
            write!(
                handle,
                "\x1b[{};2H│{:<width$}│",
                7 + row as u16,
                truncate_cells(&line, width),
                width = width
            )?;
        }
        write!(handle, "\x1b[15;2H├{}┤", "─".repeat(width))?;
        write!(
            handle,
            "\x1b[16;2H│ {:<w$}│",
            "Enter launch · Esc close · type filter · ↑/↓ select",
            w = width - 1
        )?;
        write!(handle, "\x1b[17;2H└{}┘", "─".repeat(width))?;
        Ok(())
    }
}

fn clear_launcher_overlay_area<W: Write>(handle: &mut W) -> Result<()> {
    // LauncherOverlay::render currently owns rows 2..=17 and starts at
    // column 2. Clear whole rows so stale box-drawing glyphs cannot remain
    // when the overlay closes after launch/Esc.
    for row in 2..=17u16 {
        write!(handle, "\x1b[{};1H\x1b[K", row)?;
    }
    Ok(())
}

fn filter_launcher_candidates(
    items: Vec<String>,
    query: Option<&str>,
    limit: usize,
) -> Vec<String> {
    let Some(query) = query else {
        return items.into_iter().take(limit).collect();
    };
    let q = query.to_ascii_lowercase();
    let mut scored: Vec<(u8, String)> = items
        .into_iter()
        .filter_map(|item| launcher_match_score(&item, &q).map(|score| (score, item)))
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| a_score.cmp(b_score).then_with(|| a.cmp(b)));
    scored
        .into_iter()
        .map(|(_, item)| item)
        .take(limit)
        .collect()
}

fn launcher_match_score(item: &str, lower_query: &str) -> Option<u8> {
    let lower_item = item.to_ascii_lowercase();
    if lower_item == lower_query {
        Some(0)
    } else if lower_item.starts_with(lower_query) {
        Some(1)
    } else if lower_item.contains(lower_query) {
        Some(2)
    } else {
        None
    }
}

fn truncate_cells(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn path_commands(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let Ok(read) = std::fs::read_dir(dir) else {
                continue;
            };
            for ent in read.flatten() {
                let path = ent.path();
                if !path.is_file() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                out.insert(name.to_string());
                if out.len() >= limit {
                    break;
                }
            }
            if out.len() >= limit {
                break;
            }
        }
    }
    out.into_iter().take(limit).collect()
}

#[cfg(target_os = "macos")]
fn macos_apps(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for root in ["/Applications", "/System/Applications"] {
        let Ok(read) = std::fs::read_dir(root) else {
            continue;
        };
        for ent in read.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("app") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            out.insert(name.trim_end_matches(".app").to_string());
            if out.len() >= limit {
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.into_iter().take(limit).collect()
}

#[cfg(test)]
mod ctrl_c_guard_tests {
    use super::{CtrlCGuard, CTRL_C_TRIGGER, CTRL_C_WINDOW};
    use std::time::{Duration, Instant};

    #[test]
    fn single_press_does_not_trigger() {
        let mut g = CtrlCGuard::new();
        let now = Instant::now();
        assert_eq!(g.record_press(now), 1);
        assert!(1 < CTRL_C_TRIGGER);
    }

    #[test]
    fn three_presses_within_window_trigger() {
        let mut g = CtrlCGuard::new();
        let t0 = Instant::now();
        assert_eq!(g.record_press(t0), 1);
        assert_eq!(g.record_press(t0 + Duration::from_millis(200)), 2);
        assert_eq!(
            g.record_press(t0 + Duration::from_millis(400)),
            CTRL_C_TRIGGER
        );
    }

    #[test]
    fn presses_outside_window_decay() {
        let mut g = CtrlCGuard::new();
        let t0 = Instant::now();
        g.record_press(t0);
        g.record_press(t0 + Duration::from_millis(500));
        // Third press is past the 1s window from the first press; only
        // the 2nd + 3rd should remain.
        let count = g.record_press(t0 + CTRL_C_WINDOW + Duration::from_millis(50));
        assert_eq!(count, 2);
        assert!(count < CTRL_C_TRIGGER);
    }

    #[test]
    fn footer_hint_switches_when_hosting_app() {
        let g = CtrlCGuard::new();
        assert_eq!(g.quit_hint(false), "q to quit");
        assert_eq!(g.quit_hint(true), "q or Ctrl-C×3 to quit");
    }
}

#[cfg(test)]
mod launcher_tests {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn launcher_command_defaults_to_xterm() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_LAUNCH_CMD");
        assert_eq!(super::launcher_command(), "xterm");
    }

    #[test]
    fn launcher_command_honors_env_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_LAUNCH_CMD", "/bin/sleep 1");
        assert_eq!(super::launcher_command(), "/bin/sleep 1");
        std::env::remove_var("KITTWM_LAUNCH_CMD");
    }
}

fn load_runtime_keymap(dbg: &Debugger) -> Keymap {
    match load_runtime_keymap_result(dbg) {
        Ok(km) => km,
        Err(e) => {
            dbg.log(&format!(
                "failed to load runtime keymap: {e}; using defaults"
            ));
            crate::keymap::default_keymap()
        }
    }
}

fn load_runtime_keymap_result(dbg: &Debugger) -> Result<Keymap> {
    if let Ok(path) = std::env::var("KITTUI_WM_KEYMAP") {
        let km = Keymap::load(std::path::Path::new(&path))?;
        dbg.log(&format!("loaded keymap from {path}"));
        Ok(km)
    } else {
        dbg.log("loaded default keymap");
        Ok(crate::keymap::default_keymap())
    }
}

fn key_spec_for_event(ev: &InputEvent) -> Option<KeySpec> {
    match ev {
        InputEvent::Char { ch, mods } => Some(KeySpec {
            mods: KeyMods {
                ctrl: mods.ctrl,
                alt: mods.alt,
                shift: mods.shift,
            },
            key: match ch {
                ' ' => "space".to_string(),
                other => other.to_ascii_lowercase().to_string(),
            },
        }),
        InputEvent::Key { key, mods } => Some(KeySpec {
            mods: KeyMods {
                ctrl: mods.ctrl,
                alt: mods.alt,
                shift: mods.shift,
            },
            key: match key {
                Key::Up => "up".to_string(),
                Key::Down => "down".to_string(),
                Key::Left => "left".to_string(),
                Key::Right => "right".to_string(),
                Key::Home => "home".to_string(),
                Key::End => "end".to_string(),
                Key::PageUp => "pageup".to_string(),
                Key::PageDown => "pagedown".to_string(),
                Key::Insert => "insert".to_string(),
                Key::Delete => "delete".to_string(),
                Key::Tab => "tab".to_string(),
                Key::Backspace => "backspace".to_string(),
                Key::Enter => "enter".to_string(),
                Key::Escape => "escape".to_string(),
                Key::F(n) => format!("f{n}"),
            },
        }),
        _ => None,
    }
}

#[cfg(test)]
mod runtime_keymap_tests {
    use super::*;
    use kittui_input::Modifiers;

    #[test]
    fn event_to_keyspec_maps_ctrl_a_and_enter() {
        let ctrl_a = key_spec_for_event(&InputEvent::Char {
            ch: 'a',
            mods: Modifiers {
                ctrl: true,
                alt: false,
                shift: false,
            },
        })
        .unwrap();
        assert_eq!(ctrl_a.to_string(), "C-a");
        let enter = key_spec_for_event(&InputEvent::Key {
            key: Key::Enter,
            mods: Modifiers::default(),
        })
        .unwrap();
        assert_eq!(enter.to_string(), "enter");
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct WorkspaceState {
    current: usize,
    count: usize,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            current: 0,
            count: 1,
        }
    }
}

impl WorkspaceState {
    fn apply(&mut self, action: &Action) -> String {
        match action {
            Action::WorkspaceNew => {
                self.count += 1;
                self.current = self.count - 1;
                format!("workspace.new -> {}", self.label())
            }
            Action::WorkspaceNext => {
                self.current = (self.current + 1) % self.count;
                format!("workspace.next -> {}", self.label())
            }
            Action::WorkspacePrev => {
                self.current = (self.current + self.count - 1) % self.count;
                format!("workspace.prev -> {}", self.label())
            }
            other => format!("workspace ignored action {other}"),
        }
    }

    fn label(&self) -> String {
        format!("{}/{}", self.current + 1, self.count)
    }
}

#[cfg(test)]
mod workspace_state_tests {
    use super::*;

    #[test]
    fn workspace_state_create_and_cycle() {
        let mut ws = WorkspaceState::default();
        assert_eq!(ws.label(), "1/1");
        assert_eq!(ws.apply(&Action::WorkspaceNew), "workspace.new -> 2/2");
        assert_eq!(ws.apply(&Action::WorkspaceNew), "workspace.new -> 3/3");
        assert_eq!(ws.apply(&Action::WorkspaceNext), "workspace.next -> 1/3");
        assert_eq!(ws.apply(&Action::WorkspacePrev), "workspace.prev -> 3/3");
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct FocusState {
    last_direction: &'static str,
    moves: u64,
}

impl Default for FocusState {
    fn default() -> Self {
        Self {
            last_direction: "none",
            moves: 0,
        }
    }
}

impl FocusState {
    fn apply(&mut self, action: &Action) -> String {
        self.last_direction = match action {
            Action::FocusLeft => "left",
            Action::FocusDown => "down",
            Action::FocusUp => "up",
            Action::FocusRight => "right",
            _ => self.last_direction,
        };
        self.moves += 1;
        format!("focus.{} -> {}", self.last_direction, self.label())
    }

    fn label(&self) -> String {
        format!("{}#{}", self.last_direction, self.moves)
    }
}

#[cfg(test)]
mod focus_state_tests {
    use super::*;

    #[test]
    fn focus_state_tracks_direction_and_count() {
        let mut f = FocusState::default();
        assert_eq!(f.label(), "none#0");
        assert_eq!(f.apply(&Action::FocusLeft), "focus.left -> left#1");
        assert_eq!(f.apply(&Action::FocusDown), "focus.down -> down#2");
        assert_eq!(f.apply(&Action::FocusUp), "focus.up -> up#3");
        assert_eq!(f.apply(&Action::FocusRight), "focus.right -> right#4");
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SplitState {
    panes: usize,
    last_orientation: &'static str,
}

impl Default for SplitState {
    fn default() -> Self {
        Self {
            panes: 1,
            last_orientation: "none",
        }
    }
}

impl SplitState {
    fn apply(&mut self, action: &Action) -> String {
        self.last_orientation = match action {
            Action::SplitVerticalLauncher => "vertical",
            Action::SplitHorizontalLauncher => "horizontal",
            _ => self.last_orientation,
        };
        self.panes += 1;
        format!(
            "split.{}.launcher -> {}",
            self.last_orientation,
            self.label()
        )
    }

    fn label(&self) -> String {
        format!("{}:{}", self.panes, self.last_orientation)
    }
}

#[cfg(test)]
mod split_state_tests {
    use super::*;

    #[test]
    fn split_state_tracks_panes_and_orientation() {
        let mut s = SplitState::default();
        assert_eq!(s.label(), "1:none");
        assert_eq!(
            s.apply(&Action::SplitVerticalLauncher),
            "split.vertical.launcher -> 2:vertical"
        );
        assert_eq!(
            s.apply(&Action::SplitHorizontalLauncher),
            "split.horizontal.launcher -> 3:horizontal"
        );
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SwapState {
    last_direction: &'static str,
    swaps: u64,
}

impl Default for SwapState {
    fn default() -> Self {
        Self {
            last_direction: "none",
            swaps: 0,
        }
    }
}

impl SwapState {
    fn apply(&mut self, action: &Action) -> String {
        self.last_direction = match action {
            Action::SwapLeft => "left",
            Action::SwapDown => "down",
            Action::SwapUp => "up",
            Action::SwapRight => "right",
            _ => self.last_direction,
        };
        self.swaps += 1;
        format!("swap.{} -> {}", self.last_direction, self.label())
    }

    fn label(&self) -> String {
        format!("{}#{}", self.last_direction, self.swaps)
    }
}

#[cfg(test)]
mod swap_state_tests {
    use super::*;

    #[test]
    fn swap_state_tracks_direction_and_count() {
        let mut s = SwapState::default();
        assert_eq!(s.label(), "none#0");
        assert_eq!(s.apply(&Action::SwapLeft), "swap.left -> left#1");
        assert_eq!(s.apply(&Action::SwapDown), "swap.down -> down#2");
        assert_eq!(s.apply(&Action::SwapUp), "swap.up -> up#3");
        assert_eq!(s.apply(&Action::SwapRight), "swap.right -> right#4");
    }
}

#[cfg(test)]
mod launcher_query_tests {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn launcher_selection_uses_path_query_before_shell_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTUI_WM_LAUNCH_QUERY", "echo");
        let sel = super::launcher_selection();
        std::env::remove_var("KITTUI_WM_LAUNCH_QUERY");
        assert_eq!(sel.kind, super::LauncherKind::Path);
        assert!(sel.command.to_ascii_lowercase().contains("echo"));
    }
}

#[cfg(test)]
mod launcher_overlay_tests {
    use super::*;
    use kittui_input::Modifiers;

    #[test]
    fn overlay_edits_query_and_tracks_selection() {
        let mut overlay = LauncherOverlay::default();
        overlay.active = true;
        assert_eq!(
            overlay.handle_event(&InputEvent::Char {
                ch: 'e',
                mods: Modifiers::default()
            }),
            OverlayEvent::Consumed
        );
        assert_eq!(
            overlay.handle_event(&InputEvent::Char {
                ch: 'c',
                mods: Modifiers::default()
            }),
            OverlayEvent::Consumed
        );
        assert_eq!(overlay.query, "ec");
        assert_eq!(
            overlay.handle_event(&InputEvent::Key {
                key: Key::Backspace,
                mods: Modifiers::default()
            }),
            OverlayEvent::Consumed
        );
        assert_eq!(overlay.query, "e");
        assert_eq!(
            overlay.handle_event(&InputEvent::Key {
                key: Key::Enter,
                mods: Modifiers::default()
            }),
            OverlayEvent::Launch
        );
        assert_eq!(
            overlay.handle_event(&InputEvent::Key {
                key: Key::Escape,
                mods: Modifiers::default()
            }),
            OverlayEvent::Close
        );
    }

    #[test]
    fn filter_launcher_candidates_is_case_insensitive() {
        let items = vec![
            "Echo".to_string(),
            "cat".to_string(),
            "lessecho".to_string(),
        ];
        assert_eq!(
            filter_launcher_candidates(items, Some("ECHO"), 10),
            vec!["Echo".to_string(), "lessecho".to_string()]
        );
    }

    #[test]
    fn filter_launcher_candidates_prefers_exact_then_prefix_matches() {
        let items = vec![
            "multixterm".to_string(),
            "xterm".to_string(),
            "xtermcontrol".to_string(),
        ];
        assert_eq!(
            filter_launcher_candidates(items, Some("xterm"), 10),
            vec![
                "xterm".to_string(),
                "xtermcontrol".to_string(),
                "multixterm".to_string()
            ]
        );
    }
}

/// Triple-Ctrl-C kill switch with decay window. (bd-2776ad)
///
/// Single Ctrl-C is forwarded to the focused window; only three Ctrl-C
/// presses within `CTRL_C_WINDOW` cause the WM to exit. Presses older
/// than the window are discarded so a slow typist won't accidentally
/// quit.
const CTRL_C_TRIGGER: usize = 3;
const CTRL_C_WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug, Default, Clone)]
struct CtrlCGuard {
    presses: std::collections::VecDeque<Instant>,
}

impl CtrlCGuard {
    fn new() -> Self {
        Self::default()
    }

    /// Record a Ctrl-C press at `now`. Returns the number of Ctrl-C
    /// presses currently within the decay window (including this one).
    fn record_press(&mut self, now: Instant) -> usize {
        while let Some(front) = self.presses.front() {
            if now.duration_since(*front) > CTRL_C_WINDOW {
                self.presses.pop_front();
            } else {
                break;
            }
        }
        self.presses.push_back(now);
        self.presses.len()
    }

    /// Footer hint for the operator. Switches the visible quit message
    /// to mention the Ctrl-C kill switch whenever the WM is actually
    /// hosting an app that might swallow `q` / Esc.
    fn quit_hint(&self, hosting_app: bool) -> &'static str {
        if hosting_app {
            "q or Ctrl-C×3 to quit"
        } else {
            "q to quit"
        }
    }
}

#[derive(Debug, Default, Clone)]
struct ToggleState {
    fullscreen: bool,
    floating: bool,
}

impl ToggleState {
    fn apply(&mut self, action: &Action) -> String {
        match action {
            Action::FullscreenToggle => {
                self.fullscreen = !self.fullscreen;
                format!("fullscreen.toggle -> {}", self.label())
            }
            Action::FloatToggle => {
                self.floating = !self.floating;
                format!("float.toggle -> {}", self.label())
            }
            other => format!("toggle ignored action {other}"),
        }
    }

    fn label(&self) -> String {
        format!("full={} float={}", self.fullscreen, self.floating)
    }
}

#[cfg(test)]
mod toggle_state_tests {
    use super::*;

    #[test]
    fn toggle_state_tracks_fullscreen_and_float() {
        let mut t = ToggleState::default();
        assert_eq!(t.label(), "full=false float=false");
        assert_eq!(
            t.apply(&Action::FullscreenToggle),
            "fullscreen.toggle -> full=true float=false"
        );
        assert_eq!(
            t.apply(&Action::FloatToggle),
            "float.toggle -> full=true float=true"
        );
        assert_eq!(
            t.apply(&Action::FullscreenToggle),
            "fullscreen.toggle -> full=false float=true"
        );
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LayoutState {
    split_axis: &'static str,
    balances: u64,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            split_axis: "vertical",
            balances: 0,
        }
    }
}

impl LayoutState {
    fn apply(&mut self, action: &Action) -> String {
        match action {
            Action::ToggleSplit => {
                self.split_axis = if self.split_axis == "vertical" {
                    "horizontal"
                } else {
                    "vertical"
                };
                format!("toggle.split -> {}", self.label())
            }
            Action::BalanceWindows => {
                self.balances += 1;
                format!("balance.windows -> {}", self.label())
            }
            other => format!("layout ignored action {other}"),
        }
    }

    fn label(&self) -> String {
        format!("axis={} balanced#{}", self.split_axis, self.balances)
    }
}

#[cfg(test)]
mod layout_state_tests {
    use super::*;

    #[test]
    fn layout_state_toggles_axis_and_counts_balance() {
        let mut s = LayoutState::default();
        assert_eq!(s.label(), "axis=vertical balanced#0");
        assert_eq!(
            s.apply(&Action::ToggleSplit),
            "toggle.split -> axis=horizontal balanced#0"
        );
        assert_eq!(
            s.apply(&Action::ToggleSplit),
            "toggle.split -> axis=vertical balanced#0"
        );
        assert_eq!(
            s.apply(&Action::BalanceWindows),
            "balance.windows -> axis=vertical balanced#1"
        );
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
struct ConfigState {
    reloads: u64,
    last_error: Option<String>,
}

impl ConfigState {
    fn reload_ok(&mut self) -> String {
        self.reloads += 1;
        self.last_error = None;
        format!("reload.config -> {}", self.label())
    }

    fn reload_err(&mut self, err: &str) -> String {
        self.reloads += 1;
        self.last_error = Some(err.to_string());
        format!("reload.config error -> {}", self.label())
    }

    fn label(&self) -> String {
        match &self.last_error {
            Some(_) => format!("reload#{}:err", self.reloads),
            None => format!("reload#{}", self.reloads),
        }
    }
}

#[cfg(test)]
mod config_state_tests {
    use super::*;

    #[test]
    fn config_state_counts_reload() {
        let mut s = ConfigState::default();
        assert_eq!(s.label(), "reload#0");
        assert_eq!(s.reload_ok(), "reload.config -> reload#1");
        assert_eq!(s.reload_err("bad"), "reload.config error -> reload#2:err");
        assert_eq!(s.reload_ok(), "reload.config -> reload#3");
    }
}
