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

use anyhow::{anyhow, Result};

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
    let queue = crate::daemon::NativeSpawnQueue::bind(crate::daemon::default_socket_path())?;
    let cmd = std::env::var("KITTWM_TERMINAL_CMD")
        .or_else(|_| std::env::var("SHELL").map(|s| format!("{s} -l")))
        .unwrap_or_else(|_| "/bin/sh -l".to_string());
    let mut panes = vec![spawn_native_pane(
        1,
        &cmd,
        &sock,
        cols,
        rows.saturating_sub(1).max(1),
    )?];
    let mut focused = 0usize;
    let mut layout_axis = NativePaneLayoutAxis::Columns;
    resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
    let initial_layouts = native_layouts_for_panes(cols, rows, &panes, layout_axis);
    queue.update_panes(native_pane_statuses(&panes, focused, &initial_layouts));
    queue.update_layout(layout_axis.label());

    let fps = std::env::var("KITTUI_WM_FPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30u32)
        .clamp(1, 120);
    let frame_target = Duration::from_micros(1_000_000 / fps as u64);
    let mut stdin = io::stdin();
    let mut frame = 0u64;
    let mut prefix = false;
    let mut clear = true;
    loop {
        let frame_start = Instant::now();
        let mut chunk = [0u8; 1024];
        while poll_stdin(Duration::ZERO) {
            let n = stdin.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            for &byte in &chunk[..n] {
                if byte == 0x1d {
                    dbg.log("native terminal loop: Ctrl-] exit");
                    return Ok(());
                }
                if prefix {
                    prefix = false;
                    match byte {
                        b'%' | b'|' | b'v' | b'V' => {
                            layout_axis = NativePaneLayoutAxis::Columns;
                            if panes.len() < 8 {
                                let id = next_native_pane_id(&panes);
                                panes.push(spawn_native_pane(id, &cmd, &sock, 1, 1)?);
                                let new_focus = panes.len() - 1;
                                native_set_focus(&mut panes, &mut focused, new_focus)?;
                                resize_native_panes_for_layout(
                                    &mut panes,
                                    cols,
                                    rows,
                                    layout_axis,
                                )?;
                                clear = true;
                                dbg.log(&format!(
                                    "native terminal split {:?}: panes={}",
                                    layout_axis,
                                    panes.len()
                                ));
                            }
                        }
                        b'-' | b'\"' | b'h' | b'H' => {
                            layout_axis = NativePaneLayoutAxis::Rows;
                            if panes.len() < 8 {
                                let id = next_native_pane_id(&panes);
                                panes.push(spawn_native_pane(id, &cmd, &sock, 1, 1)?);
                                let new_focus = panes.len() - 1;
                                native_set_focus(&mut panes, &mut focused, new_focus)?;
                                resize_native_panes_for_layout(
                                    &mut panes,
                                    cols,
                                    rows,
                                    layout_axis,
                                )?;
                                clear = true;
                                dbg.log(&format!(
                                    "native terminal split {:?}: panes={}",
                                    layout_axis,
                                    panes.len()
                                ));
                            }
                        }
                        b'\t' | b'n' | b'N' => {
                            let new_focus = next_native_focus(focused, panes.len());
                            native_set_focus(&mut panes, &mut focused, new_focus)?;
                            clear = true;
                            dbg.log(&format!("native terminal focus: {}", panes[focused].window));
                        }
                        b'x' | b'X' => {
                            if panes.len() > 1 {
                                native_send_focus_event(&mut panes[focused], false)?;
                                panes[focused].app.terminate()?;
                                panes.remove(focused);
                                focused = focus_after_remove(focused, focused, panes.len() + 1);
                                native_send_focus_event(&mut panes[focused], true)?;
                                resize_native_panes_for_layout(
                                    &mut panes,
                                    cols,
                                    rows,
                                    layout_axis,
                                )?;
                                clear = true;
                                dbg.log(&format!("native terminal close: panes={}", panes.len()));
                            }
                        }
                        b'+' | b'=' => {
                            panes[focused].weight = native_adjust_weight(panes[focused].weight, 1);
                            resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                            clear = true;
                            dbg.log(&format!(
                                "native terminal resize grow: {} weight={}",
                                panes[focused].window, panes[focused].weight
                            ));
                        }
                        b'_' | b'<' => {
                            panes[focused].weight = native_adjust_weight(panes[focused].weight, -1);
                            resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                            clear = true;
                            dbg.log(&format!(
                                "native terminal resize shrink: {} weight={}",
                                panes[focused].window, panes[focused].weight
                            ));
                        }
                        b'b' | b'B' => {
                            balance_native_pane_weights(&mut panes);
                            resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                            clear = true;
                            dbg.log("native terminal balance pane weights");
                        }
                        b'[' | b',' => {
                            let to = native_move_target_index(focused, panes.len(), "left");
                            if to != focused {
                                let pane = panes.remove(focused);
                                panes.insert(to, pane);
                                focused = to;
                                resize_native_panes_for_layout(
                                    &mut panes,
                                    cols,
                                    rows,
                                    layout_axis,
                                )?;
                                clear = true;
                            }
                            dbg.log(&format!("native terminal move previous -> {focused}"));
                        }
                        b']' | b'.' => {
                            let to = native_move_target_index(focused, panes.len(), "right");
                            if to != focused {
                                let pane = panes.remove(focused);
                                panes.insert(to, pane);
                                focused = to;
                                resize_native_panes_for_layout(
                                    &mut panes,
                                    cols,
                                    rows,
                                    layout_axis,
                                )?;
                                clear = true;
                            }
                            dbg.log(&format!("native terminal move next -> {focused}"));
                        }
                        0x01 => panes[focused].app.send_bytes(&[0x01])?,
                        other => panes[focused].app.send_bytes(&[other])?,
                    }
                    continue;
                }
                if byte == 0x01 {
                    prefix = true;
                    continue;
                }
                panes[focused].app.send_bytes(&[byte])?;
            }
        }

        focused = reap_exited_native_panes(&mut panes, focused, &dbg)?;
        for command in queue.drain() {
            match command {
                crate::daemon::NativePaneCommand::SpawnPty(spawn_cmd) => {
                    let id = next_native_pane_id(&panes);
                    panes.push(spawn_native_pane(id, &spawn_cmd, &sock, 1, 1)?);
                    let new_focus = panes.len() - 1;
                    native_set_focus(&mut panes, &mut focused, new_focus)?;
                    resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                    clear = true;
                    dbg.log(&format!("native terminal socket spawn: {spawn_cmd}"));
                }
                crate::daemon::NativePaneCommand::Focus(window) => {
                    if let Some(idx) = native_pane_index(&panes, &window) {
                        native_set_focus(&mut panes, &mut focused, idx)?;
                        clear = true;
                        dbg.log(&format!("native terminal socket focus: {window}"));
                    }
                }
                crate::daemon::NativePaneCommand::FocusNext => {
                    let new_focus = next_native_focus(focused, panes.len());
                    native_set_focus(&mut panes, &mut focused, new_focus)?;
                    clear = true;
                    dbg.log(&format!(
                        "native terminal socket focus next: {}",
                        panes[focused].window
                    ));
                }
                crate::daemon::NativePaneCommand::FocusPrev => {
                    let new_focus = prev_native_focus(focused, panes.len());
                    native_set_focus(&mut panes, &mut focused, new_focus)?;
                    clear = true;
                    dbg.log(&format!(
                        "native terminal socket focus prev: {}",
                        panes[focused].window
                    ));
                }
                crate::daemon::NativePaneCommand::Close(window) => {
                    if panes.len() > 1 {
                        let target = if window == "focused" {
                            Some(focused)
                        } else {
                            native_pane_index(&panes, &window)
                        };
                        if let Some(idx) = target {
                            let old_focused = focused;
                            let closing_focused = idx == old_focused;
                            if closing_focused {
                                native_send_focus_event(&mut panes[idx], false)?;
                            }
                            panes[idx].app.terminate()?;
                            panes.remove(idx);
                            focused = focus_after_remove(old_focused, idx, panes.len() + 1);
                            if closing_focused {
                                native_send_focus_event(&mut panes[focused], true)?;
                            }
                            resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                            clear = true;
                            dbg.log(&format!("native terminal socket close: {window}"));
                        }
                    }
                }
                crate::daemon::NativePaneCommand::Layout(axis) => {
                    if let Some(axis) = NativePaneLayoutAxis::parse(&axis) {
                        layout_axis = axis;
                        resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                        clear = true;
                        dbg.log(&format!(
                            "native terminal socket layout: {}",
                            layout_axis.label()
                        ));
                    }
                }
                crate::daemon::NativePaneCommand::Move { window, direction } => {
                    let target = if window == "focused" {
                        Some(focused)
                    } else {
                        native_pane_index(&panes, &window)
                    };
                    if let Some(from) = target {
                        let to = native_move_target_index(from, panes.len(), &direction);
                        if to != from {
                            let pane = panes.remove(from);
                            panes.insert(to, pane);
                        }
                        focused = to;
                        resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                        clear = true;
                        dbg.log(&format!(
                            "native terminal socket move: {window} {direction} -> {to}"
                        ));
                    }
                }
                crate::daemon::NativePaneCommand::Resize { window, delta } => {
                    let target = if window == "focused" {
                        Some(focused)
                    } else {
                        native_pane_index(&panes, &window)
                    };
                    if let Some(idx) = target {
                        panes[idx].weight = native_adjust_weight(panes[idx].weight, delta);
                        resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                        clear = true;
                        dbg.log(&format!(
                            "native terminal socket resize: {window} delta={delta} weight={}",
                            panes[idx].weight
                        ));
                    }
                }
                crate::daemon::NativePaneCommand::Balance => {
                    balance_native_pane_weights(&mut panes);
                    resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
                    clear = true;
                    dbg.log("native terminal socket balance pane weights");
                }
                crate::daemon::NativePaneCommand::Rename { window, title } => {
                    if let Some(idx) = native_pane_index(&panes, &window) {
                        panes[idx].display_title = Some(title.clone());
                        clear = true;
                        dbg.log(&format!(
                            "native terminal socket rename: {window} -> {title}"
                        ));
                    }
                }
                crate::daemon::NativePaneCommand::RestoreSession(restore) => {
                    let restore_result: Result<(NativePaneLayoutAxis, Vec<NativePane>, usize)> =
                        (|| {
                            let new_axis = restore
                                .layout
                                .as_deref()
                                .and_then(NativePaneLayoutAxis::parse)
                                .unwrap_or(layout_axis);
                            let mut restored = Vec::with_capacity(restore.panes.len());
                            for (idx, restore_pane) in restore.panes.iter().enumerate() {
                                let id = (idx + 1).min(u32::MAX as usize) as u32;
                                let mut pane =
                                    match spawn_native_pane(id, &restore_pane.command, &sock, 1, 1)
                                    {
                                        Ok(pane) => pane,
                                        Err(err) => {
                                            terminate_native_panes(&mut restored);
                                            return Err(err);
                                        }
                                    };
                                pane.weight = restore_pane.weight.max(1);
                                pane.display_title = restore_pane.title.clone();
                                restored.push(pane);
                            }
                            if restored.is_empty() {
                                return Err(anyhow!("restore session contains no panes"));
                            }
                            if let Err(err) =
                                resize_native_panes_for_layout(&mut restored, cols, rows, new_axis)
                            {
                                terminate_native_panes(&mut restored);
                                return Err(err);
                            }
                            let new_focus =
                                native_restore_focus_index(restored.len(), restore.focus_index);
                            Ok((new_axis, restored, new_focus))
                        })();
                    match restore_result {
                        Ok((new_axis, mut restored, new_focus)) => {
                            terminate_native_panes(&mut panes);
                            std::mem::swap(&mut panes, &mut restored);
                            layout_axis = new_axis;
                            focused = new_focus;
                            clear = true;
                            dbg.log(&format!(
                                "native terminal socket restore session: panes={} focus={focused}",
                                panes.len()
                            ));
                        }
                        Err(err) => {
                            dbg.log(&format!(
                                "native terminal socket restore session failed: {err}"
                            ));
                        }
                    }
                }
                crate::daemon::NativePaneCommand::SendText {
                    window,
                    mut text,
                    newline,
                } => {
                    let target = if window == "focused" {
                        Some(focused)
                    } else {
                        native_pane_index(&panes, &window)
                    };
                    if let Some(idx) = target {
                        if newline {
                            text.push('\n');
                        }
                        panes[idx].app.send_bytes(text.as_bytes())?;
                        dbg.log(&format!(
                            "native terminal socket send text: {window} bytes={}",
                            text.len()
                        ));
                    }
                }
                crate::daemon::NativePaneCommand::SendBytes {
                    window,
                    bytes,
                    label,
                } => {
                    let target = if window == "focused" {
                        Some(focused)
                    } else {
                        native_pane_index(&panes, &window)
                    };
                    if let Some(idx) = target {
                        panes[idx].app.send_bytes(&bytes)?;
                        dbg.log(&format!(
                            "native terminal socket send key: {window} key={label} bytes={}",
                            bytes.len()
                        ));
                    }
                }
                crate::daemon::NativePaneCommand::PasteBytes { window, bytes } => {
                    let target = if window == "focused" {
                        Some(focused)
                    } else {
                        native_pane_index(&panes, &window)
                    };
                    if let Some(idx) = target {
                        let bracketed = panes[idx].app.bracketed_paste_enabled();
                        let payload = native_paste_payload(&bytes, bracketed);
                        panes[idx].app.send_bytes(&payload)?;
                        dbg.log(&format!(
                            "native terminal socket paste bytes: {window} bytes={} bracketed={}",
                            bytes.len(),
                            bracketed
                        ));
                    }
                }
            }
        }
        queue.update_layout(layout_axis.label());
        let (new_cols, new_rows) = native_terminal_size();
        if (new_cols, new_rows) != (cols, rows) {
            cols = new_cols;
            rows = new_rows;
            resize_native_panes_for_layout(&mut panes, cols, rows, layout_axis)?;
            clear = true;
            dbg.log(&format!(
                "native terminal resized to {cols}x{rows} panes={}",
                panes.len()
            ));
        }

        let layouts = native_layouts_for_panes(cols, rows, &panes, layout_axis);
        queue.update_panes(native_pane_statuses(&panes, focused, &layouts));
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        if clear {
            handle.write_all(b"\x1b[2J")?;
            clear = false;
        }
        for (idx, pane) in panes.iter_mut().enumerate() {
            let layout = layouts[idx];
            write_native_pane_title(&mut handle, pane, layout, idx == focused)?;
            match pane.app.capture()? {
                NativeFrame::Rgba {
                    width,
                    height,
                    rgba,
                } => {
                    let footprint =
                        CellRect::new(layout.app_x, layout.app_y, layout.app_cols, layout.app_rows);
                    let p = runtime.place_raw_frame(pane.image_id, &rgba, width, height, footprint);
                    handle.write_all(p.upload.as_bytes())?;
                    handle.write_all(p.placement.as_bytes())?;
                    handle.write_all(p.embed.as_bytes())?;
                }
                NativeFrame::Png { .. } => {}
            }
        }
        write!(
            handle,
            "\x1b[{};1H\x1b[Kkittwm native terminal — panes={} focused={} weight={} — C-a % cols — C-a - rows — C-a +/- resize — C-a [] move — C-a b balance — C-a Tab focus — C-a x close — KITTWM_SOCKET={} — Ctrl-] exits — frame {} (log: {})",
            rows + 2,
            panes.len(),
            panes[focused].window,
            panes[focused].weight,
            sock,
            frame,
            dbg.path_display()
        )?;
        handle.flush()?;
        frame += 1;
        if let Some(slack) = frame_target.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(slack);
        }
    }
}

struct NativePane {
    window: String,
    image_id: u32,
    command: String,
    pid: Option<u32>,
    display_title: Option<String>,
    weight: u16,
    app: PtyTerminalApp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativePaneLayoutAxis {
    Columns,
    Rows,
}

impl NativePaneLayoutAxis {
    fn label(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Rows => "rows",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "columns" => Some(Self::Columns),
            "rows" => Some(Self::Rows),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativePaneLayout {
    x: u16,
    y: u16,
    cols: u16,
    app_x: u16,
    app_y: u16,
    app_cols: u16,
    app_rows: u16,
}

fn native_pane_index(panes: &[NativePane], window: &str) -> Option<usize> {
    panes.iter().position(|pane| pane.window == window)
}

fn next_native_pane_id(panes: &[NativePane]) -> u32 {
    panes
        .iter()
        .filter_map(|pane| pane.window.strip_prefix("native-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn spawn_native_pane(id: u32, cmd: &str, sock: &str, cols: u16, rows: u16) -> Result<NativePane> {
    let window = format!("native-{id}");
    let envs = vec![
        ("KITTWM_SOCKET".to_string(), sock.to_string()),
        ("KITTWM_SOCK".to_string(), sock.to_string()),
        ("KITTUI_WM_DISPLAY".to_string(), sock.to_string()),
        ("KITTWM_DISPLAY".to_string(), sock.to_string()),
        ("KITTWM_WINDOW".to_string(), window.clone()),
    ];
    let app = PtyTerminalApp::spawn_with_env(cmd, cols.max(1), rows.max(1), envs)?;
    let pid = app.process_id();
    Ok(NativePane {
        window,
        image_id: 0x6b77_0000 | id,
        command: cmd.to_string(),
        pid,
        display_title: None,
        weight: 1,
        app,
    })
}

#[cfg(test)]
fn native_pane_layouts(
    cols: u16,
    rows: u16,
    count: usize,
    axis: NativePaneLayoutAxis,
) -> Vec<NativePaneLayout> {
    native_pane_layouts_weighted(cols, rows, &vec![1; count], axis)
}

fn native_layouts_for_panes(
    cols: u16,
    rows: u16,
    panes: &[NativePane],
    axis: NativePaneLayoutAxis,
) -> Vec<NativePaneLayout> {
    native_pane_layouts_weighted(
        cols,
        rows,
        &panes.iter().map(|pane| pane.weight).collect::<Vec<_>>(),
        axis,
    )
}

fn native_pane_layouts_weighted(
    cols: u16,
    rows: u16,
    weights: &[u16],
    axis: NativePaneLayoutAxis,
) -> Vec<NativePaneLayout> {
    let count = weights.len().max(1).min(u16::MAX as usize);
    let weights = if weights.is_empty() {
        vec![1]
    } else {
        weights.to_vec()
    };
    let total_weight = weights
        .iter()
        .take(count)
        .map(|w| (*w).max(1) as u32)
        .sum::<u32>()
        .max(1);
    let title_rows = 1;
    match axis {
        NativePaneLayoutAxis::Columns => {
            let pane_rows = rows.max(title_rows + 1);
            let mut x = 0u16;
            let mut layouts = Vec::with_capacity(count);
            for idx in 0..count {
                let remaining = cols.saturating_sub(x).max(1);
                let pane_cols = if idx + 1 == count {
                    remaining
                } else {
                    let share = ((cols as u32 * weights[idx].max(1) as u32) / total_weight) as u16;
                    share.max(1).min(remaining)
                };
                layouts.push(NativePaneLayout {
                    x,
                    y: 0,
                    cols: pane_cols,
                    app_x: x,
                    app_y: title_rows,
                    app_cols: pane_cols,
                    app_rows: pane_rows.saturating_sub(title_rows).max(1),
                });
                x = x.saturating_add(pane_cols);
            }
            layouts
        }
        NativePaneLayoutAxis::Rows => {
            let mut y = 0u16;
            let mut layouts = Vec::with_capacity(count);
            for idx in 0..count {
                let remaining = rows.saturating_sub(y).max(2);
                let pane_rows = if idx + 1 == count {
                    remaining
                } else {
                    let share = ((rows as u32 * weights[idx].max(1) as u32) / total_weight) as u16;
                    share.max(2).min(remaining)
                };
                layouts.push(NativePaneLayout {
                    x: 0,
                    y,
                    cols,
                    app_x: 0,
                    app_y: y.saturating_add(title_rows),
                    app_cols: cols,
                    app_rows: pane_rows.saturating_sub(title_rows).max(1),
                });
                y = y.saturating_add(pane_rows);
            }
            layouts
        }
    }
}

fn resize_native_panes(panes: &mut [NativePane], layouts: Vec<NativePaneLayout>) -> Result<()> {
    for (pane, layout) in panes.iter_mut().zip(layouts) {
        pane.app.resize(layout.app_cols, layout.app_rows)?;
    }
    Ok(())
}

fn resize_native_panes_for_layout(
    panes: &mut [NativePane],
    cols: u16,
    rows: u16,
    axis: NativePaneLayoutAxis,
) -> Result<()> {
    let layouts = native_layouts_for_panes(cols, rows, panes, axis);
    resize_native_panes(panes, layouts)
}

fn native_adjust_weight(weight: u16, delta: i16) -> u16 {
    if delta.is_negative() {
        weight.saturating_sub(delta.unsigned_abs()).max(1)
    } else {
        weight.saturating_add(delta as u16).max(1)
    }
}

fn balance_native_pane_weights(panes: &mut [NativePane]) {
    for pane in panes {
        pane.weight = 1;
    }
}

fn terminate_native_panes(panes: &mut [NativePane]) {
    for pane in panes {
        let _ = pane.app.terminate();
    }
}

fn native_restore_focus_index(count: usize, focus_index: Option<usize>) -> usize {
    focus_index.unwrap_or(0).min(count.saturating_sub(1))
}

fn native_move_target_index(from: usize, len: usize, direction: &str) -> usize {
    if len == 0 {
        return 0;
    }
    match direction {
        "left" | "up" => from.saturating_sub(1),
        "right" | "down" => (from + 1).min(len - 1),
        "first" => 0,
        "last" => len - 1,
        _ => from.min(len - 1),
    }
}

fn native_pane_display_title(pane: &NativePane) -> String {
    pane.display_title
        .clone()
        .unwrap_or_else(|| pane.app.title())
}

fn native_paste_payload(bytes: &[u8], bracketed_paste: bool) -> Vec<u8> {
    if !bracketed_paste {
        return bytes.to_vec();
    }
    let mut wrapped = Vec::with_capacity(bytes.len() + 12);
    wrapped.extend_from_slice(b"\x1b[200~");
    wrapped.extend_from_slice(bytes);
    wrapped.extend_from_slice(b"\x1b[201~");
    wrapped
}

fn native_focus_event_payload(focus_reporting: bool, focused: bool) -> Option<&'static [u8]> {
    if !focus_reporting {
        return None;
    }
    Some(if focused { b"\x1b[I" } else { b"\x1b[O" })
}

fn native_send_focus_event(pane: &mut NativePane, focused: bool) -> Result<()> {
    if let Some(payload) = native_focus_event_payload(pane.app.focus_reporting_enabled(), focused) {
        pane.app.send_bytes(payload)?;
    }
    Ok(())
}

fn native_set_focus(
    panes: &mut [NativePane],
    focused: &mut usize,
    new_focus: usize,
) -> Result<bool> {
    if panes.is_empty() {
        *focused = 0;
        return Ok(false);
    }
    let new_focus = new_focus.min(panes.len().saturating_sub(1));
    if *focused == new_focus {
        return Ok(false);
    }
    if *focused < panes.len() {
        native_send_focus_event(&mut panes[*focused], false)?;
    }
    native_send_focus_event(&mut panes[new_focus], true)?;
    *focused = new_focus;
    Ok(true)
}

fn native_pane_statuses(
    panes: &[NativePane],
    focused: usize,
    layouts: &[NativePaneLayout],
) -> Vec<crate::daemon::NativePaneStatus> {
    panes
        .iter()
        .enumerate()
        .map(|(idx, pane)| {
            let layout = layouts.get(idx).copied();
            let (cursor_col, cursor_row) = pane.app.cursor_position();
            let mouse = pane.app.mouse_reporting_modes();
            crate::daemon::NativePaneStatus {
                window: pane.window.clone(),
                title: native_pane_display_title(pane),
                focused: idx == focused,
                weight: pane.weight,
                pid: pane.pid,
                command: Some(pane.command.clone()),
                x: layout.map(|l| l.x),
                y: layout.map(|l| l.y),
                cols: layout.map(|l| l.cols),
                rows: layout.map(|l| l.app_rows.saturating_add(1)),
                app_x: layout.map(|l| l.app_x),
                app_y: layout.map(|l| l.app_y),
                app_cols: layout.map(|l| l.app_cols),
                cursor_col: Some(cursor_col),
                cursor_row: Some(cursor_row),
                cursor_visible: Some(pane.app.cursor_visible()),
                bracketed_paste: Some(pane.app.bracketed_paste_enabled()),
                mouse_reporting: Some(mouse.basic),
                mouse_button_motion: Some(mouse.button_motion),
                mouse_all_motion: Some(mouse.all_motion),
                mouse_sgr: Some(mouse.sgr),
                text_snapshot: Some(pane.app.text_snapshot()),
                scrollback_snapshot: Some(pane.app.scrollback_snapshot()),
                app_rows: layout.map(|l| l.app_rows),
            }
        })
        .collect()
}

fn next_native_focus(current: usize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        (current + 1) % count
    }
}

fn prev_native_focus(current: usize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        current.checked_sub(1).unwrap_or(count - 1)
    }
}

fn focus_after_remove(current: usize, removed: usize, len_before: usize) -> usize {
    let len_after = len_before.saturating_sub(1);
    if len_after == 0 {
        return 0;
    }
    if removed < current {
        current.saturating_sub(1).min(len_after - 1)
    } else if removed == current {
        current.min(len_after - 1)
    } else {
        current.min(len_after - 1)
    }
}

fn reap_exited_native_panes(
    panes: &mut Vec<NativePane>,
    mut focused: usize,
    dbg: &Debugger,
) -> Result<usize> {
    if panes.len() <= 1 {
        return Ok(focused.min(panes.len().saturating_sub(1)));
    }
    let mut idx = 0;
    while idx < panes.len() && panes.len() > 1 {
        if panes[idx].app.exited()?.is_some() {
            let len_before = panes.len();
            let removed_focused = idx == focused;
            let window = panes[idx].window.clone();
            panes.remove(idx);
            focused = focus_after_remove(focused, idx, len_before);
            if removed_focused && !panes.is_empty() {
                native_send_focus_event(&mut panes[focused], true)?;
            }
            dbg.log(&format!("native terminal reaped exited pane {window}"));
        } else {
            idx += 1;
        }
    }
    Ok(focused.min(panes.len().saturating_sub(1)))
}

fn write_native_pane_title<W: Write>(
    out: &mut W,
    pane: &NativePane,
    layout: NativePaneLayout,
    focused: bool,
) -> Result<()> {
    let marker = if focused { "*" } else { " " };
    let title = format!(
        "{marker} {} {}",
        pane.window,
        native_pane_display_title(pane)
    );
    let mut clipped = title.chars().take(layout.cols as usize).collect::<String>();
    while clipped.chars().count() < layout.cols as usize {
        clipped.push(' ');
    }
    let style = if focused { "\x1b[7m" } else { "\x1b[2m" };
    write!(
        out,
        "\x1b[{};{}H{}{}\x1b[0m",
        layout.y + 1,
        layout.x + 1,
        style,
        clipped
    )?;
    Ok(())
}

#[cfg(test)]
mod native_pane_tests {
    use super::*;

    #[test]
    fn native_paste_payload_wraps_when_bracketed() {
        assert_eq!(native_paste_payload(b"a\nb", false), b"a\nb".to_vec());
        assert_eq!(
            native_paste_payload(b"a\nb", true),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
    }

    #[test]
    fn native_focus_event_payloads_require_reporting() {
        assert_eq!(native_focus_event_payload(false, true), None);
        assert_eq!(native_focus_event_payload(false, false), None);
        assert_eq!(
            native_focus_event_payload(true, true),
            Some(b"\x1b[I".as_slice())
        );
        assert_eq!(
            native_focus_event_payload(true, false),
            Some(b"\x1b[O".as_slice())
        );
    }

    #[test]
    fn native_pane_layout_axis_labels_and_parses() {
        assert_eq!(NativePaneLayoutAxis::Columns.label(), "columns");
        assert_eq!(NativePaneLayoutAxis::Rows.label(), "rows");
        assert_eq!(
            NativePaneLayoutAxis::parse("columns"),
            Some(NativePaneLayoutAxis::Columns)
        );
        assert_eq!(
            NativePaneLayoutAxis::parse("rows"),
            Some(NativePaneLayoutAxis::Rows)
        );
        assert_eq!(NativePaneLayoutAxis::parse("diagonal"), None);
    }

    #[test]
    fn native_pane_layouts_split_columns_and_reserve_title_rows() {
        let layouts = native_pane_layouts(81, 24, 2, NativePaneLayoutAxis::Columns);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].x, 0);
        assert_eq!(layouts[0].cols, 40);
        assert_eq!(layouts[0].app_y, 1);
        assert_eq!(layouts[0].app_rows, 23);
        assert_eq!(layouts[1].x, 40);
        assert_eq!(layouts[1].cols, 41);
        assert_eq!(layouts[1].app_cols, 41);
    }

    #[test]
    fn native_pane_layouts_split_rows_and_reserve_each_title_row() {
        let layouts = native_pane_layouts(80, 25, 2, NativePaneLayoutAxis::Rows);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].x, 0);
        assert_eq!(layouts[0].y, 0);
        assert_eq!(layouts[0].cols, 80);
        assert_eq!(layouts[0].app_y, 1);
        assert_eq!(layouts[0].app_rows, 11);
        assert_eq!(layouts[1].y, 12);
        assert_eq!(layouts[1].app_y, 13);
        assert_eq!(layouts[1].app_rows, 12);
    }

    #[test]
    fn native_pane_layouts_honor_weights() {
        let columns = native_pane_layouts_weighted(90, 24, &[1, 2], NativePaneLayoutAxis::Columns);
        assert_eq!(columns[0].cols, 30);
        assert_eq!(columns[1].cols, 60);
        assert_eq!(columns[1].x, 30);
        let rows = native_pane_layouts_weighted(80, 30, &[1, 2], NativePaneLayoutAxis::Rows);
        assert_eq!(rows[0].app_rows, 9);
        assert_eq!(rows[1].app_rows, 19);
        assert_eq!(rows[1].y, 10);
    }

    #[test]
    fn native_adjust_weight_clamps_to_one() {
        assert_eq!(native_adjust_weight(1, -1), 1);
        assert_eq!(native_adjust_weight(2, -1), 1);
        assert_eq!(native_adjust_weight(2, 3), 5);
    }

    #[test]
    fn balance_native_pane_weights_resets_all_weights() {
        let mut panes = vec![
            NativePane {
                window: "native-1".to_string(),
                image_id: 1,
                command: "cmd1".to_string(),
                pid: Some(101),
                display_title: None,
                weight: 4,
                app: dummy_native_pane_app(),
            },
            NativePane {
                window: "native-2".to_string(),
                image_id: 2,
                command: "cmd2".to_string(),
                pid: Some(102),
                display_title: None,
                weight: 2,
                app: dummy_native_pane_app(),
            },
        ];
        balance_native_pane_weights(&mut panes);
        assert_eq!(
            panes.iter().map(|pane| pane.weight).collect::<Vec<_>>(),
            vec![1, 1]
        );
    }

    #[test]
    fn native_restore_focus_index_clamps_to_restored_panes() {
        assert_eq!(native_restore_focus_index(3, Some(1)), 1);
        assert_eq!(native_restore_focus_index(3, Some(99)), 2);
        assert_eq!(native_restore_focus_index(3, None), 0);
        assert_eq!(native_restore_focus_index(0, Some(1)), 0);
    }

    #[test]
    fn native_move_target_index_clamps_and_moves() {
        assert_eq!(native_move_target_index(1, 3, "left"), 0);
        assert_eq!(native_move_target_index(1, 3, "up"), 0);
        assert_eq!(native_move_target_index(1, 3, "right"), 2);
        assert_eq!(native_move_target_index(1, 3, "down"), 2);
        assert_eq!(native_move_target_index(1, 3, "first"), 0);
        assert_eq!(native_move_target_index(1, 3, "last"), 2);
        assert_eq!(native_move_target_index(0, 3, "left"), 0);
        assert_eq!(native_move_target_index(2, 3, "right"), 2);
        assert_eq!(native_move_target_index(5, 0, "last"), 0);
    }

    #[test]
    fn native_move_preserves_focus_on_moved_pane() {
        let mut order = vec!["a", "b", "c"];
        let mut focused = 1usize;
        let to = native_move_target_index(focused, order.len(), "right");
        let pane = order.remove(focused);
        order.insert(to, pane);
        focused = to;
        assert_eq!(order, vec!["a", "c", "b"]);
        assert_eq!(order[focused], "b");
    }

    #[test]
    fn native_focus_cycles_through_available_panes() {
        assert_eq!(next_native_focus(0, 1), 0);
        assert_eq!(next_native_focus(0, 3), 1);
        assert_eq!(next_native_focus(2, 3), 0);
        assert_eq!(next_native_focus(0, 0), 0);
        assert_eq!(prev_native_focus(0, 1), 0);
        assert_eq!(prev_native_focus(0, 3), 2);
        assert_eq!(prev_native_focus(2, 3), 1);
        assert_eq!(prev_native_focus(0, 0), 0);
    }

    #[test]
    fn native_focus_after_remove_stays_on_neighbor() {
        assert_eq!(focus_after_remove(1, 1, 3), 1);
        assert_eq!(focus_after_remove(2, 1, 3), 1);
        assert_eq!(focus_after_remove(0, 2, 3), 0);
        assert_eq!(focus_after_remove(0, 0, 1), 0);
    }

    #[test]
    fn native_pane_index_finds_window_tokens() {
        let panes = vec![
            NativePane {
                window: "native-1".to_string(),
                image_id: 1,
                command: "cmd1".to_string(),
                pid: Some(101),
                display_title: None,
                weight: 1,
                app: dummy_native_pane_app(),
            },
            NativePane {
                window: "native-2".to_string(),
                image_id: 2,
                command: "cmd2".to_string(),
                pid: Some(102),
                display_title: None,
                weight: 1,
                app: dummy_native_pane_app(),
            },
        ];
        assert_eq!(native_pane_index(&panes, "native-2"), Some(1));
        assert_eq!(native_pane_index(&panes, "missing"), None);
    }

    #[test]
    fn next_native_pane_id_uses_max_existing_id() {
        let panes = vec![
            NativePane {
                window: "native-1".to_string(),
                image_id: 1,
                command: "cmd1".to_string(),
                pid: Some(101),
                display_title: None,
                weight: 1,
                app: dummy_native_pane_app(),
            },
            NativePane {
                window: "native-7".to_string(),
                image_id: 7,
                command: "cmd7".to_string(),
                pid: Some(107),
                display_title: None,
                weight: 1,
                app: dummy_native_pane_app(),
            },
        ];
        assert_eq!(next_native_pane_id(&panes), 8);
    }

    #[test]
    fn native_pane_statuses_mark_focused_window() {
        let panes = vec![
            NativePane {
                window: "native-1".to_string(),
                image_id: 1,
                command: "cmd1".to_string(),
                pid: Some(101),
                display_title: None,
                weight: 1,
                app: dummy_native_pane_app(),
            },
            NativePane {
                window: "native-2".to_string(),
                image_id: 2,
                command: "editor-cmd".to_string(),
                pid: Some(202),
                display_title: Some("editor".to_string()),
                weight: 3,
                app: dummy_native_pane_app(),
            },
        ];
        let layouts = native_pane_layouts_weighted(80, 24, &[1, 3], NativePaneLayoutAxis::Columns);
        let statuses = native_pane_statuses(&panes, 1, &layouts);
        assert_eq!(statuses.len(), 2);
        assert!(!statuses[0].focused);
        assert!(statuses[1].focused);
        assert_eq!(statuses[1].window, "native-2");
        assert_eq!(statuses[1].title, "editor");
        assert_eq!(statuses[1].weight, 3);
        assert_eq!(statuses[1].pid, Some(202));
        assert_eq!(statuses[1].command.as_deref(), Some("editor-cmd"));
        assert_eq!(statuses[1].x, Some(20));
        assert_eq!(statuses[1].cols, Some(60));
        assert_eq!(statuses[1].app_y, Some(1));
        assert_eq!(statuses[1].app_rows, Some(23));
    }

    fn dummy_native_pane_app() -> PtyTerminalApp {
        PtyTerminalApp::spawn("true", 1, 1).unwrap()
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
    let mut layout = layout.clone();
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
    let mut picker_overlay = PickerOverlay::default();
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
            if picker_overlay.active {
                match picker_overlay.handle_event(&ev) {
                    OverlayEvent::Consumed => continue,
                    OverlayEvent::Close => {
                        picker_overlay.active = false;
                        last_keymap_action = Some("picker.close".to_string());
                        dbg.log("picker overlay closed");
                        continue;
                    }
                    OverlayEvent::Launch => {
                        last_keymap_action = Some(format!(
                            "picker.select {}",
                            picker_overlay.selection_label()
                        ));
                        dbg.log(&format!(
                            "picker selected {}",
                            picker_overlay.selection_label()
                        ));
                        picker_overlay.active = false;
                        continue;
                    }
                    OverlayEvent::NotHandled => {}
                }
            }
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
                                Action::PickerOpen => {
                                    picker_overlay.open();
                                    last_keymap_action = Some("picker.open".to_string());
                                    dbg.log("picker overlay opened");
                                }
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
                                    let focused = match action {
                                        Action::FocusLeft | Action::FocusUp => {
                                            compositor.focus_prev()
                                        }
                                        _ => compositor.focus_next(),
                                    };
                                    let msg = match focused {
                                        Ok(Some(id)) => format!("{msg} window={}", id.0),
                                        Ok(None) => format!("{msg} window=-"),
                                        Err(e) => format!("{msg} error={e}"),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("focus action: {msg}"));
                                }
                                Action::SwapLeft
                                | Action::SwapDown
                                | Action::SwapUp
                                | Action::SwapRight => {
                                    let msg = swap_state.apply(&action);
                                    let moved = match action {
                                        Action::SwapLeft | Action::SwapUp => {
                                            compositor.lower_focused()
                                        }
                                        _ => compositor.raise_focused(),
                                    };
                                    let msg = match moved {
                                        Ok(Some(id)) => format!("{msg} window={}", id.0),
                                        Ok(None) => format!("{msg} window=-"),
                                        Err(e) => format!("{msg} error={e}"),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("swap action: {msg}"));
                                }
                                Action::FloatToggle => {
                                    let msg = toggle_state.apply(&action);
                                    let msg = match compositor.toggle_focused_mode() {
                                        Ok(Some((id, mode))) => {
                                            let mode = match mode {
                                                kittui_wm::compositor::WindowMode::Floating => {
                                                    "floating"
                                                }
                                                kittui_wm::compositor::WindowMode::Tiled => "tiled",
                                            };
                                            format!("{msg} window={} mode={mode}", id.0)
                                        }
                                        Ok(None) => format!("{msg} window=-"),
                                        Err(e) => format!("{msg} error={e}"),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("toggle action: {msg}"));
                                }
                                Action::FullscreenToggle => {
                                    let msg = toggle_state.apply(&action);
                                    let msg = match compositor.toggle_focused_fullscreen() {
                                        Ok(Some((id, fullscreen))) => {
                                            format!("{msg} window={} fullscreen={fullscreen}", id.0)
                                        }
                                        Ok(None) => format!("{msg} window=-"),
                                        Err(e) => format!("{msg} error={e}"),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&format!("toggle action: {msg}"));
                                }
                                Action::ToggleSplit | Action::BalanceWindows => {
                                    let msg = layout_state.apply(&action);
                                    let msg = match rebuild_tiled_layout(
                                        compositor,
                                        &mut layout,
                                        &layout_state,
                                    ) {
                                        Ok(count) => format!("{msg} windows={count}"),
                                        Err(e) => format!("{msg} error={e}"),
                                    };
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
        match compositor.raw_frames(&layout) {
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
                    write_raw_frame_chrome(&mut handle, f)?;
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
                if picker_overlay.active {
                    picker_overlay.render(&mut handle)?;
                    footer_row = footer_row.max(14);
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
fn write_raw_frame_chrome<W: Write>(
    out: &mut W,
    frame: &kittui_wm::compositor::RawFrame,
) -> Result<()> {
    let marker = if frame.focused { "*" } else { " " };
    let mode = match frame.mode {
        kittui_wm::compositor::WindowMode::Floating => "float",
        kittui_wm::compositor::WindowMode::Tiled => "tile",
    };
    let fullscreen = if frame.fullscreen { " full" } else { "" };
    let label = format!("{marker} {} {mode}{fullscreen}", frame.title);
    let mut clipped = label
        .chars()
        .take(frame.footprint.cols as usize)
        .collect::<String>();
    while clipped.chars().count() < frame.footprint.cols as usize {
        clipped.push(' ');
    }
    let style = if frame.focused { "\x1b[7m" } else { "\x1b[2m" };
    write!(
        out,
        "\x1b[{};{}H{}{}\x1b[0m",
        frame.footprint.y + 1,
        frame.footprint.x + 1,
        style,
        clipped
    )?;
    Ok(())
}

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

#[derive(Debug, Clone, Eq, PartialEq, Default)]
struct PickerOverlay {
    active: bool,
    selected: usize,
    entries: Vec<String>,
}

impl PickerOverlay {
    fn open(&mut self) {
        self.active = true;
        self.selected = 0;
        self.entries = vec![
            "backend: native PTY terminal".to_string(),
            "backend: kittwm-browser".to_string(),
            "backend: fake gallery".to_string(),
            "window: current native-1".to_string(),
        ];
        #[cfg(all(target_os = "macos", feature = "quartz"))]
        {
            for w in kittui_quartz::QuartzServer::list_app_windows()
                .into_iter()
                .take(8)
            {
                self.entries
                    .push(format!("mac: {} — {}", w.owner_name, w.title));
            }
        }
    }

    fn handle_event(&mut self, ev: &InputEvent) -> OverlayEvent {
        match ev {
            InputEvent::Key { key: Key::Up, .. } => {
                self.selected = self.selected.saturating_sub(1);
                OverlayEvent::Consumed
            }
            InputEvent::Key { key: Key::Down, .. } | InputEvent::Key { key: Key::Tab, .. } => {
                let max = self.entries.len().saturating_sub(1);
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

    fn selection_label(&self) -> String {
        self.entries
            .get(self.selected.min(self.entries.len().saturating_sub(1)))
            .cloned()
            .unwrap_or_else(|| "<none>".to_string())
    }

    fn render<W: Write>(&self, handle: &mut W) -> Result<()> {
        let width = 64usize;
        write!(handle, "\x1b[2;2H┌{}┐", "─".repeat(width))?;
        write!(
            handle,
            "\x1b[3;2H│{:^width$}│",
            "kittwm picker",
            width = width
        )?;
        write!(handle, "\x1b[4;2H├{}┤", "─".repeat(width))?;
        for row in 0..8usize {
            let line = if let Some(entry) = self.entries.get(row) {
                let marker = if row == self.selected { "▶" } else { " " };
                format!("{marker} {}", entry)
            } else {
                String::new()
            };
            write!(
                handle,
                "\x1b[{};2H│{:<width$}│",
                5 + row as u16,
                truncate_cells(&line, width),
                width = width
            )?;
        }
        write!(handle, "\x1b[13;2H├{}┤", "─".repeat(width))?;
        write!(
            handle,
            "\x1b[14;2H│ {:<w$}│",
            "Enter select · Esc close · ↑/↓/Tab navigate",
            w = width - 1
        )?;
        write!(handle, "\x1b[15;2H└{}┘", "─".repeat(width))?;
        Ok(())
    }
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
    fn is_vertical(&self) -> bool {
        self.split_axis == "vertical"
    }

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

fn rebuild_tiled_layout<S: XServer>(
    compositor: &Compositor<S>,
    layout: &mut Layout,
    state: &LayoutState,
) -> std::result::Result<usize, kittui_xvfb::XError> {
    let windows = compositor.server().windows()?;
    if windows.is_empty() {
        layout.clear();
        return Ok(0);
    }
    let bounds = layout.bounds().unwrap_or_else(|| {
        windows
            .iter()
            .map(|w| w.rect)
            .reduce(px_rect_union)
            .unwrap()
    });
    layout.clear();
    let count = windows.len();
    for (idx, w) in windows.iter().enumerate() {
        layout.tile(w.id, split_slot(bounds, idx, count, state.is_vertical()));
        compositor.set_mode(w.id, kittui_wm::compositor::WindowMode::Tiled);
    }
    Ok(count)
}

fn split_slot(
    bounds: kittui_core::geom::PxRect,
    idx: usize,
    count: usize,
    vertical: bool,
) -> kittui_core::geom::PxRect {
    let count = count.max(1) as f32;
    if vertical {
        let slot = bounds.width / count;
        kittui_core::geom::PxRect::new(
            bounds.origin.0 + slot * idx as f32,
            bounds.origin.1,
            slot,
            bounds.height,
        )
    } else {
        let slot = bounds.height / count;
        kittui_core::geom::PxRect::new(
            bounds.origin.0,
            bounds.origin.1 + slot * idx as f32,
            bounds.width,
            slot,
        )
    }
}

fn px_rect_union(
    a: kittui_core::geom::PxRect,
    b: kittui_core::geom::PxRect,
) -> kittui_core::geom::PxRect {
    let min_x = a.origin.0.min(b.origin.0);
    let min_y = a.origin.1.min(b.origin.1);
    let max_x = (a.origin.0 + a.width).max(b.origin.0 + b.width);
    let max_y = (a.origin.1 + a.height).max(b.origin.1 + b.height);
    kittui_core::geom::PxRect::new(min_x, min_y, max_x - min_x, max_y - min_y)
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

    #[test]
    fn split_slot_divides_bounds_by_axis() {
        let bounds = kittui_core::geom::PxRect::new(0.0, 0.0, 90.0, 30.0);
        let a = split_slot(bounds, 1, 3, true);
        assert_eq!(a.origin.0, 30.0);
        assert_eq!(a.width, 30.0);
        assert_eq!(a.height, 30.0);
        let b = split_slot(bounds, 1, 3, false);
        assert_eq!(b.origin.1, 10.0);
        assert_eq!(b.width, 90.0);
        assert_eq!(b.height, 10.0);
    }

    #[test]
    fn rebuild_tiled_layout_assigns_current_windows() {
        let server = kittui_xvfb::FakeServer::with_windows(vec![
            (
                kittui_xvfb::XWindowId(1),
                kittui_core::geom::PxRect::new(0.0, 0.0, 90.0, 30.0),
                "a",
                [0xff, 0x00, 0x00, 0xff],
            ),
            (
                kittui_xvfb::XWindowId(2),
                kittui_core::geom::PxRect::new(0.0, 0.0, 90.0, 30.0),
                "b",
                [0x00, 0xff, 0x00, 0xff],
            ),
        ]);
        let compositor = Compositor::new(server, kittui::CellSize::new(10, 10));
        let mut layout = Layout::all_floating();
        layout.tile(
            kittui_xvfb::XWindowId(1),
            kittui_core::geom::PxRect::new(0.0, 0.0, 90.0, 30.0),
        );
        let mut state = LayoutState::default();
        assert_eq!(
            rebuild_tiled_layout(&compositor, &mut layout, &state).unwrap(),
            2
        );
        assert_eq!(
            layout.tiled_rect(kittui_xvfb::XWindowId(1)).unwrap().width,
            45.0
        );
        assert_eq!(
            layout
                .tiled_rect(kittui_xvfb::XWindowId(2))
                .unwrap()
                .origin
                .0,
            45.0
        );
        state.apply(&Action::ToggleSplit);
        assert_eq!(
            rebuild_tiled_layout(&compositor, &mut layout, &state).unwrap(),
            2
        );
        assert_eq!(
            layout.tiled_rect(kittui_xvfb::XWindowId(1)).unwrap().height,
            15.0
        );
        assert_eq!(
            layout
                .tiled_rect(kittui_xvfb::XWindowId(2))
                .unwrap()
                .origin
                .1,
            15.0
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
