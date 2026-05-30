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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use kittui::{
    CellRect, CellSize, Corners, Layer, Node, Paint, PxRect, Rgba, Runtime, Scene, Stroke,
};
#[cfg(test)]
use kittui_affordances::{button, text_input, ControlState};
use kittui_affordances::{InlineChipColors, InlineStyle, InlineTheme};
use kittui_ghostty_vt::PreviewOptions;
use kittui_input::{InputEvent, Key, MouseButton};
use kittui_wm::compositor::{Compositor, Layout};
use kittui_wm::dirty::{DirtyFrameDiff, DirtyGrid};
use kittui_wm::native::{
    BrowserArrowKey, BrowserFunctionKey, BrowserPageKey, GhosttyTerminalApp, HeadlessBrowserApp,
    MouseReportingModes, NativeFrame, NativeSurface, PtyTerminalApp, SurfaceFrame, SurfaceMetadata,
    SurfacePointerButton, SurfacePointerEvent,
};
use kittui_xvfb::XServer;
use kittwm_sdk::{ArchitectureContract, KittwmConfig, LibghosttyConfig, SurfacePlacementRole};

use crate::keymap::{Action, KeyMods, KeySpec, Keymap};
use crate::top_bar::{workspace_chip_total_cols, workspace_label, BarModel};

#[derive(Default)]
struct NativeFrameWriteBatch {
    bytes: Vec<u8>,
}

impl NativeFrameWriteBatch {
    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[cfg(test)]
    fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<bool> {
        if self.bytes.is_empty() {
            return Ok(false);
        }
        writer.write_all(&self.bytes)?;
        writer.flush()?;
        Ok(true)
    }
}

impl Write for NativeFrameWriteBatch {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativeAppPlacementDecision {
    write_upload: bool,
    write_placement: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NativeFrameWriteBytes {
    upload: usize,
    placement: usize,
    embed: usize,
}

impl NativeFrameWriteBytes {
    fn add(&mut self, placement: &kittui::Placement, include_upload: bool) {
        if include_upload {
            self.upload += placement.upload.as_bytes().len();
        }
        self.placement += placement.placement.as_bytes().len();
        self.embed += placement.embed.as_bytes().len();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativePngFrameDecision {
    upload: bool,
    placement: NativeAppPlacementDecision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeChromePlacementMemo {
    key: String,
    image_id: u32,
}

fn decide_native_app_placement_write(
    placements: &mut HashMap<u32, CellRect>,
    image_id: u32,
    footprint: CellRect,
    upload: bool,
) -> NativeAppPlacementDecision {
    let placement_changed = placements.get(&image_id).copied() != Some(footprint);
    if placement_changed {
        placements.insert(image_id, footprint);
    }
    NativeAppPlacementDecision {
        write_upload: upload,
        write_placement: placement_changed,
    }
}

fn native_png_frame_hash(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn native_raw_frame_hash(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    rgba.hash(&mut hasher);
    hasher.finish()
}

fn decide_native_raw_frame_write(
    raw_hashes: &mut HashMap<u32, u64>,
    placements: &mut HashMap<u32, CellRect>,
    image_id: u32,
    footprint: CellRect,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> NativePngFrameDecision {
    let hash = native_raw_frame_hash(width, height, rgba);
    let content_changed = raw_hashes.get(&image_id).copied() != Some(hash);
    if content_changed {
        raw_hashes.insert(image_id, hash);
    }
    let placement =
        decide_native_app_placement_write(placements, image_id, footprint, content_changed);
    NativePngFrameDecision {
        upload: content_changed || placement.write_placement,
        placement,
    }
}

fn raw_frame_write_with_chrome_change(
    mut decision: NativePngFrameDecision,
    chrome_changed: bool,
) -> NativePngFrameDecision {
    if chrome_changed {
        decision.placement.write_placement = true;
    }
    decision
}

fn should_unplace_raw_frame_before_move(had_previous: bool, footprint_changed: bool) -> bool {
    had_previous && footprint_changed
}

fn decide_native_png_frame_write(
    png_hashes: &mut HashMap<u32, u64>,
    placements: &mut HashMap<u32, CellRect>,
    image_id: u32,
    footprint: CellRect,
    bytes: &[u8],
) -> NativePngFrameDecision {
    let hash = native_png_frame_hash(bytes);
    let upload = png_hashes.get(&image_id).copied() != Some(hash);
    if upload {
        png_hashes.insert(image_id, hash);
    }
    NativePngFrameDecision {
        upload,
        placement: decide_native_app_placement_write(placements, image_id, footprint, upload),
    }
}

fn should_publish_native_frame_event(
    uploaded: bool,
    placement_changed: bool,
    changed_tiles: Option<u32>,
) -> bool {
    uploaded || placement_changed || changed_tiles.unwrap_or(0) > 0
}

fn should_write_ansi_top_bar(
    affordance_scene_chrome: bool,
    redraw_static: bool,
    current: &str,
    last: &str,
) -> bool {
    !affordance_scene_chrome && (redraw_static || current != last)
}

fn raw_compositor_app_z_index() -> i32 {
    // The raw compositor draws pane titles/footer as ANSI text, not kittui
    // scene text. Keep app images below that terminal text layer.
    native_app_z_index().min(-1)
}

fn native_live_app_z_index() -> i32 {
    // Live kittwm still paints readable chrome text through ANSI overlays while
    // kittui scenes provide graphical glass, borders, and focus affordances.
    // Keep app frames below the terminal text plane.
    native_app_z_index().min(-2)
}

fn native_live_chrome_z_index() -> i32 {
    // Place graphical chrome above app frames but below ANSI text overlays so
    // workspace, clock, title, and status text remains visible.
    native_chrome_z_index()
        .min(-1)
        .max(native_live_app_z_index().saturating_add(1))
}

fn raw_compositor_app_placement_options(image_id: u32) -> kittui_kitty::PlacementOptions {
    kittui_kitty::PlacementOptions::stable_absolute(image_id)
        .with_z_index(raw_compositor_app_z_index())
}

const RAW_COMPOSITOR_ERROR_MESSAGE_MAX_CHARS: usize = 240;
const RAW_COMPOSITOR_ERROR_LOG_PATH_MAX_CHARS: usize = 120;

fn raw_compositor_error_text(message: &str) -> String {
    truncate_cells(message, RAW_COMPOSITOR_ERROR_MESSAGE_MAX_CHARS)
}

fn raw_compositor_error_log_path(log_path: &str) -> String {
    truncate_cells(log_path, RAW_COMPOSITOR_ERROR_LOG_PATH_MAX_CHARS)
}

fn raw_compositor_error_key(message: &str, log_path: &str) -> String {
    let message = raw_compositor_error_text(message);
    let log_path = raw_compositor_error_log_path(log_path);
    let mut out = String::with_capacity(message.len() + 1 + log_path.len());
    out.push_str(&message);
    out.push('\n');
    out.push_str(&log_path);
    out
}

fn should_write_raw_compositor_error(last_key: Option<&str>, next_key: &str) -> bool {
    last_key != Some(next_key)
}

fn should_clear_raw_error_screen(last_error_key: Option<&str>) -> bool {
    last_error_key.is_some()
}

fn raw_compositor_should_render_app_graphics(text_overlay_active: bool) -> bool {
    !text_overlay_active
}

fn should_hide_raw_graphics_for_text_overlay(
    text_overlay_active: bool,
    already_hidden: bool,
) -> bool {
    text_overlay_active && !already_hidden
}

fn raw_compositor_footer_row_for_overlays(
    footer_row: u16,
    launcher_active: bool,
    picker_active: bool,
    terminal_rows: u16,
) -> Option<u16> {
    let overlay_bottom = if launcher_active {
        17
    } else if picker_active {
        15
    } else {
        0
    };
    let row = footer_row.max(overlay_bottom + 1);
    (row <= terminal_rows).then_some(row)
}

fn reset_native_app_frame_memos_for_clear(
    placements: &mut HashMap<u32, CellRect>,
    png_hashes: &mut HashMap<u32, u64>,
    dirty_frames: &mut NativeDirtyFramePolicy,
    panes: &[NativePane],
) {
    placements.clear();
    png_hashes.clear();
    for pane in panes {
        dirty_frames.forget(pane.image_id);
    }
}

const DEFAULT_NATIVE_IDLE_FPS: u32 = 4;

fn native_idle_frame_target(active_target: Duration) -> Duration {
    let idle_fps = std::env::var("KITTWM_IDLE_FPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_NATIVE_IDLE_FPS)
        .clamp(1, 60);
    Duration::from_micros(1_000_000 / idle_fps as u64).max(active_target)
}

fn native_current_frame_target(
    active_target: Duration,
    idle_target: Duration,
    consecutive_idle_frames: u16,
) -> Duration {
    if consecutive_idle_frames >= 2 {
        idle_target
    } else {
        active_target
    }
}

fn update_native_idle_counter(counter: &mut u16, emitted: bool) {
    if emitted {
        *counter = 0;
    } else {
        *counter = counter.saturating_add(1);
    }
}

fn update_native_idle_counter_for_activity(counter: &mut u16, emitted: bool, input_activity: bool) {
    update_native_idle_counter(counter, emitted || input_activity);
}

fn raw_compositor_current_frame_target(
    active_target: Duration,
    idle_target: Duration,
    consecutive_idle_frames: u16,
) -> Duration {
    native_current_frame_target(active_target, idle_target, consecutive_idle_frames)
}

fn native_pane_statuses_changed(
    last: &[crate::daemon::NativePaneStatus],
    next: &[crate::daemon::NativePaneStatus],
) -> bool {
    last != next
}

fn publish_native_pane_statuses_if_changed(
    queue: &crate::daemon::NativeSpawnQueue,
    last: &mut Vec<crate::daemon::NativePaneStatus>,
    next: Vec<crate::daemon::NativePaneStatus>,
) -> bool {
    if !native_pane_statuses_changed(last, &next) {
        return false;
    }
    queue.update_panes(next.clone());
    *last = next;
    true
}

fn should_publish_native_layout(last: &mut String, next: &str) -> bool {
    if last == next {
        return false;
    }
    last.clear();
    last.push_str(next);
    true
}

const MAX_NATIVE_TERMINAL_COLS: u16 = 512;
const MAX_NATIVE_TERMINAL_ROWS: u16 = 256;

/// Drive the kittui-wm UI loop until the operator quits.
///
/// `compositor` and `layout` are passed in so callers can wire any
/// `XServer` backend (FakeServer, Xvfb, Quartz, XQuartz, ...) without
/// this module knowing about backends.
pub fn run_native_terminal_loop(runtime: &Runtime) -> Result<()> {
    let dbg = Debugger::open();
    dbg.log("native terminal loop: enter");
    let (mut cols, mut rows) = native_terminal_size();
    let sock = crate::daemon::default_socket_path()
        .to_string_lossy()
        .to_string();
    let queue = crate::daemon::NativeSpawnQueue::bind(crate::daemon::default_socket_path())?;

    let _raw_guard = RawMode::enter()?;
    install_signal_restore();
    let kittwm_config = KittwmConfig::load_default().unwrap_or_default();
    let cmd = native_terminal_command(&kittwm_config);
    let mut last_chrome_reservation = queue.chrome_reservation();
    let mut panes = if native_startup_terminal_enabled() {
        vec![spawn_native_pane(
            1,
            &cmd,
            &sock,
            cols,
            native_tilable_rows_with_reservation(rows, &last_chrome_reservation),
        )?]
    } else {
        Vec::new()
    };
    let mut focused = 0usize;
    let mut layout_axis = NativePaneLayoutAxis::Columns;
    let mut layout_mode = NativePaneLayoutMode::Tiled;
    let mut floating_offsets = HashMap::<String, NativeFloatingPaneOffset>::new();
    let mut floating_drag = None::<NativeFloatingDrag>;
    let mut tiled_drag = None::<NativeTiledDrag>;
    resize_native_panes_for_display_mode(
        &mut panes,
        focused,
        cols,
        rows,
        layout_axis,
        layout_mode,
        &last_chrome_reservation,
    )?;
    let initial_layouts = native_layouts_for_panes_display_mode_with_offsets(
        cols,
        rows,
        &panes,
        focused,
        layout_axis,
        layout_mode,
        &last_chrome_reservation,
        &floating_offsets,
    );
    let mut last_resized_layouts = initial_layouts.clone();
    let mut last_published_pane_statuses = native_pane_statuses(
        &panes,
        focused,
        &initial_layouts,
        &floating_offsets,
        layout_mode,
        None,
    );
    let mut last_published_layout = layout_mode.label(layout_axis).to_string();
    queue.update_panes(last_published_pane_statuses.clone());
    queue.update_layout(layout_mode.label(layout_axis));

    let fps = std::env::var("KITTUI_WM_FPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30u32)
        .clamp(1, 120);
    let frame_target = Duration::from_micros(1_000_000 / fps as u64);
    let idle_frame_target = native_idle_frame_target(frame_target);
    let mut stdin = io::stdin();
    let mut prefix = false;
    let mut clear = native_initial_frame_requires_explicit_clear();
    let mut help_overlay = false;
    let mut ctrl_c_exit_guard = NativeCtrlCExitGuard::default();
    let mut quit_confirm_overlay = QuitConfirmOverlay::default();
    let mut last_quit_confirm_overlay_key = String::new();
    let mut last_title_rows = Vec::<String>::new();
    let mut last_top_bar = String::new();
    let mut last_footer = String::new();
    let mut last_terminal_render = String::new();
    let mut last_affordance_chrome_view_key = None::<u64>;
    let pure_terminal_renderer = native_should_use_pure_terminal_renderer();
    let affordance_scene_chrome = native_should_use_affordance_scene_chrome();
    let mut dirty_frames = NativeDirtyFramePolicy::from_env();
    let mut consecutive_idle_frames = 0u16;
    let mut prev_native_image_ids = HashSet::<u32>::new();
    let mut native_app_placements = HashMap::<u32, CellRect>::new();
    let mut native_png_hashes = HashMap::<u32, u64>::new();
    let mut affordance_chrome_keys = HashMap::<String, NativeChromePlacementMemo>::new();
    loop {
        let frame_start = Instant::now();
        let mut input_activity = false;
        let mut chunk = [0u8; 1024];
        while poll_stdin(Duration::ZERO) {
            let n = stdin.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            input_activity = true;
            let mut offset = 0usize;
            while offset < n {
                let remaining = &chunk[offset..n];
                if let Some((event, consumed)) = kittui_input::parse(remaining) {
                    if native_route_mouse_event(
                        &event,
                        &mut panes,
                        &mut focused,
                        cols,
                        rows,
                        layout_axis,
                        layout_mode,
                        &last_chrome_reservation,
                        &mut floating_offsets,
                        &mut floating_drag,
                        &mut tiled_drag,
                        &mut clear,
                    )? {
                        offset += consumed;
                        continue;
                    }
                    if !prefix && !panes.is_empty() {
                        if let Some(payload) = native_key_event_payload(
                            &event,
                            panes[focused].app.application_cursor_keys_enabled(),
                        ) {
                            native_send_pane_bytes_logged(
                                &mut panes[focused],
                                "keyboard-event",
                                payload,
                                &dbg,
                            );
                            offset += consumed;
                            continue;
                        }
                    }
                    for &byte in &remaining[..consumed] {
                        if process_native_terminal_byte(
                            byte,
                            &mut prefix,
                            &mut panes,
                            &mut focused,
                            &mut layout_axis,
                            &mut layout_mode,
                            &mut floating_offsets,
                            &cmd,
                            &sock,
                            cols,
                            rows,
                            &last_chrome_reservation,
                            &mut clear,
                            &mut help_overlay,
                            &mut ctrl_c_exit_guard,
                            &mut quit_confirm_overlay,
                            &dbg,
                        )? {
                            return Ok(());
                        }
                    }
                    offset += consumed;
                } else {
                    if process_native_terminal_byte(
                        remaining[0],
                        &mut prefix,
                        &mut panes,
                        &mut focused,
                        &mut layout_axis,
                        &mut layout_mode,
                        &mut floating_offsets,
                        &cmd,
                        &sock,
                        cols,
                        rows,
                        &last_chrome_reservation,
                        &mut clear,
                        &mut help_overlay,
                        &mut ctrl_c_exit_guard,
                        &mut quit_confirm_overlay,
                        &dbg,
                    )? {
                        return Ok(());
                    }
                    offset += 1;
                }
            }
        }

        if quit_confirm_overlay.expired(Instant::now()) {
            quit_confirm_overlay.close();
            last_quit_confirm_overlay_key.clear();
            clear = true;
            dbg.log("native terminal quit confirmation timed out");
        }

        focused = reap_exited_native_panes(&mut panes, focused, &dbg)?;
        for command in queue.drain() {
            match command {
                crate::daemon::NativePaneCommand::SpawnPty(spawn_cmd) => {
                    let id = next_native_pane_id(&panes);
                    match spawn_native_pane(id, &spawn_cmd, &sock, 1, 1) {
                        Ok(pane) => {
                            panes.push(pane);
                            let new_focus = panes.len() - 1;
                            native_set_focus(&mut panes, &mut focused, new_focus)?;
                            resize_native_panes_for_display_mode(
                                &mut panes,
                                focused,
                                cols,
                                rows,
                                layout_axis,
                                layout_mode,
                                &last_chrome_reservation,
                            )?;
                            clear = true;
                            dbg.log(&native_socket_spawn_log_line(&spawn_cmd));
                        }
                        Err(err) => dbg.log(&native_spawn_failure_log_line(&spawn_cmd, &err)),
                    }
                }
                crate::daemon::NativePaneCommand::Focus(window) => {
                    if let Some(idx) = native_pane_index(&panes, &window) {
                        native_set_focus(&mut panes, &mut focused, idx)?;
                        clear = true;
                        dbg.log(&native_socket_focus_log_line(&window));
                    }
                }
                crate::daemon::NativePaneCommand::FocusNext => {
                    if !panes.is_empty() {
                        let new_focus = next_native_focus(focused, panes.len());
                        native_set_focus(&mut panes, &mut focused, new_focus)?;
                        clear = true;
                        dbg.log(&native_socket_focus_next_log_line(&panes[focused].window));
                    }
                }
                crate::daemon::NativePaneCommand::FocusPrev => {
                    if !panes.is_empty() {
                        let new_focus = prev_native_focus(focused, panes.len());
                        native_set_focus(&mut panes, &mut focused, new_focus)?;
                        clear = true;
                        dbg.log(&native_socket_focus_prev_log_line(&panes[focused].window));
                    }
                }
                crate::daemon::NativePaneCommand::Close(window) => {
                    if !panes.is_empty() {
                        if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                            let old_focused = focused;
                            let closing_focused = idx == old_focused;
                            if closing_focused {
                                let _ = native_send_focus_event(&mut panes[idx], false);
                            }
                            native_terminate_pane_logged(&mut panes[idx], &dbg);
                            panes.remove(idx);
                            if panes.is_empty() {
                                focused = 0;
                            } else {
                                focused = focus_after_remove(old_focused, idx, panes.len() + 1);
                                if closing_focused {
                                    let _ = native_send_focus_event(&mut panes[focused], true);
                                }
                                resize_native_panes_for_display_mode(
                                    &mut panes,
                                    focused,
                                    cols,
                                    rows,
                                    layout_axis,
                                    layout_mode,
                                    &last_chrome_reservation,
                                )?;
                            }
                            clear = true;
                            dbg.log(&native_socket_close_log_line(&window));
                        }
                    }
                }
                crate::daemon::NativePaneCommand::Layout(axis) => {
                    if let Some(axis) = NativePaneLayoutAxis::parse(&axis) {
                        layout_axis = axis;
                        layout_mode = NativePaneLayoutMode::Tiled;
                        resize_native_panes_for_display_mode(
                            &mut panes,
                            focused,
                            cols,
                            rows,
                            layout_axis,
                            layout_mode,
                            &last_chrome_reservation,
                        )?;
                        clear = true;
                        dbg.log(&native_socket_layout_log_line(layout_axis.label()));
                    }
                }
                crate::daemon::NativePaneCommand::SplitPane {
                    window,
                    axis,
                    command,
                } => {
                    if let Some(axis) = NativePaneLayoutAxis::parse(&axis) {
                        let target = native_target_pane_index(&panes, focused, &window)
                            .unwrap_or(focused.min(panes.len().saturating_sub(1)));
                        layout_axis = axis;
                        layout_mode = NativePaneLayoutMode::Tiled;
                        let id = next_native_pane_id(&panes);
                        match spawn_native_pane(id, &command, &sock, 1, 1) {
                            Ok(pane) => {
                                let insert_at = (target + 1).min(panes.len());
                                panes.insert(insert_at, pane);
                                focused = insert_at;
                                resize_native_panes_for_display_mode(
                                    &mut panes,
                                    focused,
                                    cols,
                                    rows,
                                    layout_axis,
                                    layout_mode,
                                    &last_chrome_reservation,
                                )?;
                                clear = true;
                                dbg.log(&native_socket_split_log_line(
                                    &window,
                                    axis.label(),
                                    &command,
                                ));
                            }
                            Err(err) => dbg.log(&native_spawn_failure_log_line(&command, &err)),
                        }
                    }
                }
                crate::daemon::NativePaneCommand::Move { window, direction } => {
                    if let Some(from) = native_target_pane_index(&panes, focused, &window) {
                        let old_focused_window = panes.get(focused).map(|pane| pane.window.clone());
                        let to = native_move_target_index(from, panes.len(), &direction);
                        if to != from {
                            let pane = panes.remove(from);
                            panes.insert(to, pane);
                        }
                        if let Some(old_focused_window) = old_focused_window.as_deref() {
                            if let Some(old_focus_idx) =
                                native_window_index_after_reorder(&panes, old_focused_window)
                            {
                                focused = old_focus_idx;
                            }
                        }
                        native_set_focus(&mut panes, &mut focused, to)?;
                        resize_native_panes_for_display_mode(
                            &mut panes,
                            focused,
                            cols,
                            rows,
                            layout_axis,
                            layout_mode,
                            &last_chrome_reservation,
                        )?;
                        clear = true;
                        dbg.log(&native_socket_move_log_line(&window, &direction, to));
                    }
                }
                crate::daemon::NativePaneCommand::Nudge { window, dx, dy } => {
                    if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                        native_nudge_floating_offset(
                            &mut floating_offsets,
                            &panes[idx].window,
                            dx,
                            dy,
                        );
                        clear = true;
                        dbg.log(&native_socket_nudge_log_line(&window, dx, dy));
                    }
                }
                crate::daemon::NativePaneCommand::ResetOffset { window } => {
                    if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                        native_reset_floating_offset(&mut floating_offsets, &panes[idx].window);
                        clear = true;
                        dbg.log(&native_socket_reset_offset_log_line(&window));
                    }
                }
                crate::daemon::NativePaneCommand::ResetAllOffsets => {
                    let reset_count = native_reset_all_floating_offsets(&mut floating_offsets);
                    if reset_count > 0 {
                        clear = true;
                    }
                    dbg.log(&native_socket_reset_all_offsets_log_line(reset_count));
                }
                crate::daemon::NativePaneCommand::Resize { window, delta } => {
                    if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                        panes[idx].weight = native_adjust_weight(panes[idx].weight, delta);
                        resize_native_panes_for_display_mode(
                            &mut panes,
                            focused,
                            cols,
                            rows,
                            layout_axis,
                            layout_mode,
                            &last_chrome_reservation,
                        )?;
                        clear = true;
                        dbg.log(&native_socket_resize_log_line(
                            &window,
                            delta,
                            panes[idx].weight,
                        ));
                    }
                }
                crate::daemon::NativePaneCommand::Balance => {
                    balance_native_pane_weights(&mut panes);
                    resize_native_panes_for_display_mode(
                        &mut panes,
                        focused,
                        cols,
                        rows,
                        layout_axis,
                        layout_mode,
                        &last_chrome_reservation,
                    )?;
                    clear = true;
                    dbg.log("native terminal socket balance pane weights");
                }
                crate::daemon::NativePaneCommand::Rename { window, title } => {
                    if let Some(idx) = native_pane_index(&panes, &window) {
                        panes[idx].display_title = Some(title.clone());
                        clear = true;
                        dbg.log(&native_socket_rename_log_line(&window, &title));
                    }
                }
                crate::daemon::NativePaneCommand::RestoreSession(restore) => {
                    let restore_result: Result<(
                        NativePaneLayoutAxis,
                        Vec<NativePane>,
                        usize,
                        HashMap<String, NativeFloatingPaneOffset>,
                    )> = (|| {
                        let new_axis = restore
                            .layout
                            .as_deref()
                            .and_then(NativePaneLayoutAxis::parse)
                            .unwrap_or(layout_axis);
                        let mut restored = Vec::with_capacity(restore.panes.len());
                        for (idx, restore_pane) in restore.panes.iter().enumerate() {
                            let id = (idx + 1).min(u32::MAX as usize) as u32;
                            let mut pane =
                                match spawn_native_pane(id, &restore_pane.command, &sock, 1, 1) {
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
                        if let Err(err) = resize_native_panes_for_layout_with_reservation(
                            &mut restored,
                            cols,
                            rows,
                            new_axis,
                            &last_chrome_reservation,
                        ) {
                            terminate_native_panes(&mut restored);
                            return Err(err);
                        }
                        let new_focus =
                            native_restore_focus_target(restored.len(), restore.focus_index)
                                .expect("restored panes checked non-empty");
                        let new_offsets = restored
                            .iter()
                            .zip(restore.panes.iter())
                            .filter_map(|(pane, restore_pane)| {
                                let offset = NativeFloatingPaneOffset {
                                    dx: restore_pane.floating_dx.unwrap_or_default(),
                                    dy: restore_pane.floating_dy.unwrap_or_default(),
                                };
                                (offset != NativeFloatingPaneOffset::default())
                                    .then(|| (pane.window.clone(), offset))
                            })
                            .collect::<HashMap<_, _>>();
                        Ok((new_axis, restored, new_focus, new_offsets))
                    })();
                    match restore_result {
                        Ok((new_axis, mut restored, new_focus, new_offsets)) => {
                            terminate_native_panes(&mut panes);
                            std::mem::swap(&mut panes, &mut restored);
                            layout_axis = new_axis;
                            floating_offsets = new_offsets;
                            floating_drag = None;
                            focused = new_focus;
                            if should_focus_restored_pane(panes.len(), focused) {
                                let _ = native_send_focus_event(&mut panes[focused], true);
                            }
                            clear = true;
                            dbg.log(&native_socket_restore_session_log_line(
                                panes.len(),
                                focused,
                            ));
                        }
                        Err(err) => {
                            dbg.log(&native_socket_restore_session_failure_log_line(&err));
                        }
                    }
                }
                crate::daemon::NativePaneCommand::SendText {
                    window,
                    mut text,
                    newline,
                } => {
                    if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                        if newline {
                            text.push('\n');
                        }
                        match panes[idx].app.send_bytes(text.as_bytes()) {
                            Ok(()) => {
                                dbg.log(&native_socket_send_text_log_line(&window, text.len()))
                            }
                            Err(err) => dbg.log(&native_socket_input_failure_log_line(
                                &window,
                                "send-text",
                                text.len(),
                                &err,
                            )),
                        };
                    }
                }
                crate::daemon::NativePaneCommand::SendBytes {
                    window,
                    bytes,
                    label,
                } => {
                    if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                        match panes[idx].app.send_browser_key_label(&label) {
                            Ok(true) => {
                                dbg.log(&native_socket_send_browser_key_log_line(&window, &label))
                            }
                            Ok(false) => match panes[idx].app.send_bytes(&bytes) {
                                Ok(()) => dbg.log(&native_socket_send_key_log_line(
                                    &window,
                                    &label,
                                    bytes.len(),
                                )),
                                Err(err) => dbg.log(&native_socket_input_failure_log_line(
                                    &window,
                                    "send-key",
                                    bytes.len(),
                                    &err,
                                )),
                            },
                            Err(err) => dbg.log(&native_socket_input_failure_log_line(
                                &window,
                                "send-browser-key",
                                bytes.len(),
                                &err,
                            )),
                        };
                    }
                }
                crate::daemon::NativePaneCommand::PasteBytes { window, bytes } => {
                    if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                        let bracketed = panes[idx].app.bracketed_paste_enabled();
                        let payload = native_paste_payload(&bytes, bracketed);
                        match panes[idx].app.send_bytes(&payload) {
                            Ok(()) => dbg.log(&native_socket_paste_bytes_log_line(
                                &window,
                                bytes.len(),
                                bracketed,
                            )),
                            Err(err) => dbg.log(&native_socket_input_failure_log_line(
                                &window,
                                "paste-bytes",
                                bytes.len(),
                                &err,
                            )),
                        };
                    }
                }
                crate::daemon::NativePaneCommand::SendMouse {
                    window,
                    event,
                    col,
                    row,
                } => {
                    if let Some(idx) = native_target_pane_index(&panes, focused, &window) {
                        if panes[idx].app.supports_direct_pointer() {
                            let events = native_surface_pointer_events(&event, col, row);
                            let mut sent = 0usize;
                            for pointer_event in events.iter().copied() {
                                match NativeSurface::send_surface_pointer(
                                    &mut panes[idx].app,
                                    pointer_event,
                                ) {
                                    Ok(()) => sent += 1,
                                    Err(err) => {
                                        dbg.log(&native_socket_input_failure_log_line(
                                            &window,
                                            "send-mouse-direct",
                                            events.len(),
                                            &err,
                                        ));
                                        break;
                                    }
                                }
                            }
                            dbg.log(&native_socket_send_mouse_direct_log_line(
                                &window,
                                &event,
                                col,
                                row,
                                sent,
                                events.len(),
                            ));
                        } else {
                            let modes = panes[idx].app.mouse_reporting_modes();
                            if let Some(payload) =
                                native_mouse_event_payload(&event, col, row, modes)
                            {
                                match panes[idx].app.send_bytes(&payload) {
                                    Ok(()) => dbg.log(&native_socket_send_mouse_log_line(
                                        &window,
                                        &event,
                                        col,
                                        row,
                                        payload.len(),
                                    )),
                                    Err(err) => dbg.log(&native_socket_input_failure_log_line(
                                        &window,
                                        "send-mouse",
                                        payload.len(),
                                        &err,
                                    )),
                                };
                            } else {
                                dbg.log(&native_socket_send_mouse_ignored_log_line(
                                    &window, &event, modes,
                                ));
                            }
                        }
                    }
                }
            }
        }
        let layout_label = layout_mode.label(layout_axis);
        if should_publish_native_layout(&mut last_published_layout, layout_label) {
            queue.update_layout(layout_label);
        }
        let (new_cols, new_rows) = native_terminal_size();
        if (new_cols, new_rows) != (cols, rows) {
            cols = new_cols;
            rows = new_rows;
            let layouts = native_layouts_for_panes_display_mode_with_offsets(
                cols,
                rows,
                &panes,
                focused,
                layout_axis,
                layout_mode,
                &last_chrome_reservation,
                &floating_offsets,
            );
            let resize_failures =
                resize_native_panes_logged(&mut panes, layouts.clone(), Some(&dbg))?;
            last_resized_layouts = layouts.clone();
            publish_native_pane_statuses_if_changed(
                &queue,
                &mut last_published_pane_statuses,
                native_pane_statuses(
                    &panes,
                    focused,
                    &layouts,
                    &floating_offsets,
                    layout_mode,
                    native_active_drag_label(floating_drag.as_ref(), tiled_drag.as_ref()),
                ),
            );
            clear = true;
            dbg.log(&native_terminal_resized_log_line(
                cols,
                rows,
                panes.len(),
                resize_failures,
            ));
        }

        let chrome_reservation = queue.chrome_reservation();
        if chrome_reservation != last_chrome_reservation {
            resize_native_panes_for_display_mode(
                &mut panes,
                focused,
                cols,
                rows,
                layout_axis,
                layout_mode,
                &chrome_reservation,
            )?;
            clear = true;
            dbg.log(&native_chrome_reservation_changed_log_line(
                &chrome_reservation,
            ));
            last_chrome_reservation = chrome_reservation.clone();
        }
        let layouts = native_layouts_for_panes_display_mode_with_offsets(
            cols,
            rows,
            &panes,
            focused,
            layout_axis,
            layout_mode,
            &last_chrome_reservation,
            &floating_offsets,
        );
        if layouts != last_resized_layouts {
            let resize_failures =
                resize_native_panes_logged(&mut panes, layouts.clone(), Some(&dbg))?;
            last_resized_layouts = layouts.clone();
            clear = true;
            if should_log_resize_failures(resize_failures) {
                dbg.log(&native_layout_resize_completed_log_line(resize_failures));
            }
        }
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        let mut frame_out = NativeFrameWriteBatch::default();
        let current_native_image_ids = native_image_id_set(&panes);
        for old_id in retired_native_image_ids(&prev_native_image_ids, &current_native_image_ids) {
            native_app_placements.remove(&old_id);
            native_png_hashes.remove(&old_id);
            dirty_frames.forget(old_id);
            frame_out.write_all(runtime.unplace(old_id).as_bytes())?;
        }
        prev_native_image_ids = current_native_image_ids;
        for pane in &panes {
            let surface_events = pane.app.take_surface_events();
            if !surface_events.is_empty() {
                queue.publish_surface_events(pane.window.clone(), surface_events);
            }
            let sequences = pane.app.take_host_sequences();
            if !sequences.is_empty() {
                frame_out.write_all(&sequences)?;
                dbg.log(&native_forwarded_host_sequence_log_line(
                    &pane.window,
                    sequences.len(),
                ));
            }
        }
        let redraw_static = clear;
        if pure_terminal_renderer {
            for pane in &mut panes {
                pane.app.refresh_text_snapshot()?;
            }
            let active_drag_label =
                native_active_drag_label(floating_drag.as_ref(), tiled_drag.as_ref());
            let pre_capture_shell_view = native_shell_view(
                cols,
                rows,
                &panes,
                focused,
                &layouts,
                &floating_offsets,
                &sock,
                dbg.path_display(),
                layout_mode.label(layout_axis),
                active_drag_label,
                help_overlay,
                true,
            );
            let rendered = render_native_shell_view_terminal(&pre_capture_shell_view, cols, rows);
            let has_pending_output = !frame_out.is_empty();
            publish_native_pane_statuses_if_changed(
                &queue,
                &mut last_published_pane_statuses,
                native_pane_statuses(
                    &panes,
                    focused,
                    &layouts,
                    &floating_offsets,
                    layout_mode,
                    native_active_drag_label(floating_drag.as_ref(), tiled_drag.as_ref()),
                ),
            );
            if should_write_pure_terminal_frame(
                &last_terminal_render,
                &rendered,
                redraw_static,
                has_pending_output,
            ) {
                if redraw_static {
                    frame_out.write_all(b"\x1b[2J")?;
                }
                if redraw_static || last_terminal_render != rendered {
                    frame_out.write_all(rendered.as_bytes())?;
                    last_terminal_render = rendered;
                }
                let emitted = frame_out.write_to(&mut handle)?;
                update_native_idle_counter_for_activity(
                    &mut consecutive_idle_frames,
                    emitted,
                    input_activity,
                );
            } else {
                update_native_idle_counter_for_activity(
                    &mut consecutive_idle_frames,
                    false,
                    input_activity,
                );
            }
            clear = false;
            let sleep_target = native_current_frame_target(
                frame_target,
                idle_frame_target,
                consecutive_idle_frames,
            );
            sleep_remaining_frame_budget(frame_start, sleep_target);
            continue;
        }
        if clear {
            frame_out.write_all(b"\x1b[2J")?;
            for memo in affordance_chrome_keys.values() {
                frame_out.write_all(runtime.unplace(memo.image_id).as_bytes())?;
            }
            reset_native_app_frame_memos_for_clear(
                &mut native_app_placements,
                &mut native_png_hashes,
                &mut dirty_frames,
                &panes,
            );
            last_title_rows.clear();
            last_top_bar.clear();
            last_footer.clear();
            affordance_chrome_keys.clear();
            last_affordance_chrome_view_key = None;
            clear = false;
        }
        for (idx, pane) in panes.iter_mut().enumerate() {
            let layout = layouts[idx];
            let frame_start = Instant::now();
            let surface_frame = match NativeSurface::capture_surface(&mut pane.app) {
                Ok(surface_frame) => {
                    native_capture_failure_recovered(&mut pane.capture_failed);
                    surface_frame
                }
                Err(err) => {
                    pane.dirty_frame = None;
                    if should_log_native_capture_failure(&mut pane.capture_failed) {
                        dbg.log(&native_capture_failure_log_line(&pane.window, layout, &err));
                    }
                    continue;
                }
            };
            match surface_frame.frame {
                NativeFrame::Rgba {
                    width,
                    height,
                    rgba,
                } => {
                    if layout.app_cols == 0 || layout.app_rows == 0 {
                        pane.dirty_frame = None;
                        continue;
                    }
                    let footprint = native_app_frame_footprint(layout);
                    let (rgba, width, height) = fit_rgba_frame_to_cells(
                        rgba,
                        width,
                        height,
                        layout.app_cols,
                        layout.app_rows,
                    );
                    let decision = dirty_frames.decide(pane.image_id, width, height, &rgba);
                    pane.dirty_frame = Some(decision.metrics.clone());
                    let placement_write = decide_native_app_placement_write(
                        &mut native_app_placements,
                        pane.image_id,
                        footprint,
                        decision.upload,
                    );
                    let placement_options =
                        kittui_kitty::PlacementOptions::stable_absolute(pane.image_id)
                            .with_z_index(native_live_app_z_index());
                    let mut write_bytes = NativeFrameWriteBytes::default();
                    if placement_write.write_upload {
                        let p = runtime.place_raw_frame_with_options(
                            pane.image_id,
                            &rgba,
                            width,
                            height,
                            footprint,
                            &placement_options,
                        );
                        frame_out.write_all(p.upload.as_bytes())?;
                        if placement_write.write_placement {
                            frame_out.write_all(p.placement.as_bytes())?;
                            frame_out.write_all(p.embed.as_bytes())?;
                            write_bytes.add(&p, true);
                        } else {
                            write_bytes.upload += p.upload.as_bytes().len();
                        }
                    } else if placement_write.write_placement {
                        let p = runtime.place_uploaded_image_with_options(
                            pane.image_id,
                            footprint,
                            &placement_options,
                        );
                        frame_out.write_all(p.placement.as_bytes())?;
                        frame_out.write_all(p.embed.as_bytes())?;
                        write_bytes.add(&p, false);
                    }
                    if should_publish_native_frame_event(
                        decision.upload,
                        placement_write.write_placement,
                        Some(decision.metrics.changed_tiles),
                    ) {
                        queue.publish_frame_presented(
                            pane.window.clone(),
                            crate::daemon::NativeFramePresented {
                                renderer: "kitty".to_string(),
                                format: "rgba".to_string(),
                                pixel_width: width,
                                pixel_height: height,
                                app_x: Some(layout.app_x),
                                app_y: Some(layout.app_y),
                                app_cols: Some(layout.app_cols),
                                app_rows: Some(layout.app_rows),
                                uploaded: decision.upload,
                                skipped_upload: decision.metrics.skipped_upload,
                                changed_tiles: Some(decision.metrics.changed_tiles),
                                total_tiles: Some(decision.metrics.total_tiles),
                                upload_bytes: Some(write_bytes.upload),
                                placement_bytes: Some(write_bytes.placement),
                                embed_bytes: Some(write_bytes.embed),
                                elapsed_us: Some(
                                    frame_start.elapsed().as_micros().min(u128::from(u64::MAX))
                                        as u64,
                                ),
                            },
                        );
                    }
                }
                NativeFrame::Png {
                    width,
                    height,
                    bytes,
                } => {
                    if layout.app_cols == 0 || layout.app_rows == 0 {
                        pane.dirty_frame = None;
                        continue;
                    }
                    let footprint = native_app_frame_footprint(layout);
                    let decision = decide_native_png_frame_write(
                        &mut native_png_hashes,
                        &mut native_app_placements,
                        pane.image_id,
                        footprint,
                        &bytes,
                    );
                    let placement_options =
                        kittui_kitty::PlacementOptions::stable_absolute(pane.image_id)
                            .with_z_index(native_live_app_z_index());
                    let mut write_bytes = NativeFrameWriteBytes::default();
                    if decision.upload {
                        let p = runtime.place_png_frame_with_options(
                            pane.image_id,
                            &bytes,
                            footprint,
                            &placement_options,
                        );
                        frame_out.write_all(p.upload.as_bytes())?;
                        if decision.placement.write_placement {
                            frame_out.write_all(p.placement.as_bytes())?;
                            frame_out.write_all(p.embed.as_bytes())?;
                            write_bytes.add(&p, true);
                        } else {
                            write_bytes.upload += p.upload.as_bytes().len();
                        }
                    } else if decision.placement.write_placement {
                        let p = runtime.place_uploaded_image_with_options(
                            pane.image_id,
                            footprint,
                            &placement_options,
                        );
                        frame_out.write_all(p.placement.as_bytes())?;
                        frame_out.write_all(p.embed.as_bytes())?;
                        write_bytes.add(&p, false);
                    }
                    pane.dirty_frame = Some(NativeDirtyFrameMetrics {
                        changed_tiles: 0,
                        total_tiles: 0,
                        changed_fraction: 1.0,
                        skipped_upload: !decision.upload,
                    });
                    if should_publish_native_frame_event(
                        decision.upload,
                        decision.placement.write_placement,
                        None,
                    ) {
                        queue.publish_frame_presented(
                            pane.window.clone(),
                            crate::daemon::NativeFramePresented {
                                renderer: "kitty".to_string(),
                                format: "png".to_string(),
                                pixel_width: width,
                                pixel_height: height,
                                app_x: Some(layout.app_x),
                                app_y: Some(layout.app_y),
                                app_cols: Some(layout.app_cols),
                                app_rows: Some(layout.app_rows),
                                uploaded: decision.upload,
                                skipped_upload: !decision.upload,
                                changed_tiles: None,
                                total_tiles: None,
                                upload_bytes: Some(write_bytes.upload),
                                placement_bytes: Some(write_bytes.placement),
                                embed_bytes: Some(write_bytes.embed),
                                elapsed_us: Some(
                                    frame_start.elapsed().as_micros().min(u128::from(u64::MAX))
                                        as u64,
                                ),
                            },
                        );
                    }
                }
            }
        }
        publish_native_pane_statuses_if_changed(
            &queue,
            &mut last_published_pane_statuses,
            native_pane_statuses(
                &panes,
                focused,
                &layouts,
                &floating_offsets,
                layout_mode,
                native_active_drag_label(floating_drag.as_ref(), tiled_drag.as_ref()),
            ),
        );
        let active_drag_label =
            native_active_drag_label(floating_drag.as_ref(), tiled_drag.as_ref());
        let shell_view = native_shell_view(
            cols,
            rows,
            &panes,
            focused,
            &layouts,
            &floating_offsets,
            &sock,
            dbg.path_display(),
            layout_mode.label(layout_axis),
            active_drag_label,
            help_overlay,
            false,
        );
        if last_title_rows.len() != shell_view.panes.len() {
            last_title_rows.resize(shell_view.panes.len(), String::new());
        }
        if should_write_ansi_top_bar(
            affordance_scene_chrome,
            redraw_static,
            &shell_view.top_bar.text,
            &last_top_bar,
        ) {
            write!(
                frame_out,
                "\x1b[{};1H\x1b[7m{}\x1b[0m",
                shell_view.top_bar.row + 1,
                clip_and_pad(&shell_view.top_bar.text, cols as usize)
            )?;
            last_top_bar = shell_view.top_bar.text.clone();
        }
        if shell_view.help_overlay {
            write_native_help_overlay(&mut frame_out, cols, rows)?;
        }
        for (idx, chrome) in shell_view.panes.iter().enumerate() {
            if !affordance_scene_chrome
                && (redraw_static || last_title_rows.get(idx) != Some(&chrome.cache_key))
            {
                write_native_pane_chrome(&mut frame_out, chrome, cols, rows)?;
                last_title_rows[idx] = chrome.cache_key.clone();
            }
        }
        if affordance_scene_chrome {
            let chrome_view_key = native_shell_view_affordance_chrome_key(&shell_view, cols, rows);
            if redraw_static || last_affordance_chrome_view_key != Some(chrome_view_key) {
                write_native_shell_affordance_chrome(
                    &mut frame_out,
                    runtime,
                    &shell_view,
                    cols,
                    rows,
                    &mut affordance_chrome_keys,
                )?;
                write_native_graphical_pane_text_overlays(&mut frame_out, &shell_view, cols, rows)?;
                last_affordance_chrome_view_key = Some(chrome_view_key);
            }
            if redraw_static || shell_view.top_bar.text != last_top_bar {
                write_native_graphical_top_bar_text_overlay(&mut frame_out, &shell_view, cols)?;
                last_top_bar = shell_view.top_bar.text.clone();
            }
        }
        if !affordance_scene_chrome && !shell_view.footer.text.is_empty() {
            let visible_footer = native_footer_visible_text(&shell_view.footer.text, cols);
            if redraw_static || visible_footer != last_footer {
                write!(
                    frame_out,
                    "\x1b[0m\x1b[{};1H\x1b[K{}",
                    terminal_visible_row(shell_view.footer.row, rows) + 1,
                    visible_footer
                )?;
                last_footer = visible_footer;
            }
        }
        if quit_confirm_overlay.active {
            let overlay_key = quit_confirm_overlay_key(&quit_confirm_overlay);
            if redraw_static || overlay_key != last_quit_confirm_overlay_key {
                quit_confirm_overlay.render(&mut frame_out, rows)?;
                last_quit_confirm_overlay_key = overlay_key;
            }
        } else {
            last_quit_confirm_overlay_key.clear();
        }
        let emitted = frame_out.write_to(&mut handle)?;
        update_native_idle_counter_for_activity(
            &mut consecutive_idle_frames,
            emitted,
            input_activity,
        );
        let sleep_target =
            native_current_frame_target(frame_target, idle_frame_target, consecutive_idle_frames);
        sleep_remaining_frame_budget(frame_start, sleep_target);
    }
}

struct NativePane {
    window: String,
    image_id: u32,
    command: String,
    pid: Option<u32>,
    display_title: Option<String>,
    weight: u16,
    app: NativeTerminalApp,
    dirty_frame: Option<NativeDirtyFrameMetrics>,
    capture_failed: bool,
}

#[allow(clippy::large_enum_variant)]
enum NativeTerminalApp {
    Pty(PtyTerminalApp),
    Ghostty(GhosttyTerminalApp),
    Browser(HeadlessBrowserApp),
}

impl NativeTerminalApp {
    fn title(&self) -> String {
        match self {
            Self::Pty(app) => NativeSurface::metadata(app).title,
            Self::Ghostty(app) => NativeSurface::metadata(app).title,
            Self::Browser(app) => NativeSurface::metadata(app).title,
        }
    }

    fn text_snapshot(&self) -> String {
        match self {
            Self::Pty(app) => app.text_snapshot(),
            Self::Ghostty(app) => app.text_snapshot(),
            Self::Browser(app) => NativeSurface::metadata(app).title,
        }
    }

    fn scrollback_snapshot(&self) -> String {
        match self {
            Self::Pty(app) => app.scrollback_snapshot(),
            Self::Ghostty(_) | Self::Browser(_) => String::new(),
        }
    }

    fn refresh_text_snapshot(&mut self) -> Result<bool> {
        match self {
            Self::Pty(_) | Self::Browser(_) => Ok(false),
            Self::Ghostty(app) => app.refresh_text_snapshot(),
        }
    }

    fn take_host_sequences(&self) -> Vec<u8> {
        match self {
            Self::Pty(app) => app.take_host_sequences(),
            Self::Ghostty(_) | Self::Browser(_) => Vec::new(),
        }
    }

    fn take_surface_events(&self) -> Vec<kittui_wm::native::SurfaceEvent> {
        match self {
            Self::Pty(app) => app.take_surface_events(),
            Self::Ghostty(_) | Self::Browser(_) => Vec::new(),
        }
    }

    fn cursor_position(&self) -> (u16, u16) {
        match self {
            Self::Pty(app) => app.cursor_position(),
            Self::Ghostty(_) | Self::Browser(_) => (0, 0),
        }
    }

    fn cursor_visible(&self) -> bool {
        match self {
            Self::Pty(app) => app.cursor_visible(),
            Self::Ghostty(_) => true,
            Self::Browser(_) => false,
        }
    }

    fn focus_reporting_enabled(&self) -> bool {
        match self {
            Self::Pty(app) => app.focus_reporting_enabled(),
            Self::Ghostty(_) | Self::Browser(_) => false,
        }
    }

    fn bracketed_paste_enabled(&self) -> bool {
        match self {
            Self::Pty(app) => app.bracketed_paste_enabled(),
            Self::Ghostty(app) => app.bracketed_paste_enabled(),
            Self::Browser(_) => false,
        }
    }

    fn application_cursor_keys_enabled(&self) -> bool {
        match self {
            Self::Pty(app) => app.application_cursor_keys_enabled(),
            Self::Ghostty(app) => app.application_cursor_keys_enabled(),
            Self::Browser(_) => false,
        }
    }

    fn mouse_reporting_modes(&self) -> MouseReportingModes {
        match self {
            Self::Pty(app) => app.mouse_reporting_modes(),
            Self::Ghostty(app) => app.mouse_reporting_modes(),
            Self::Browser(_) => MouseReportingModes::default(),
        }
    }

    fn supports_direct_pointer(&self) -> bool {
        matches!(self, Self::Browser(_))
    }

    fn process_id(&self) -> Option<u32> {
        match self {
            Self::Pty(app) => app.process_id(),
            Self::Ghostty(app) => app.process_id(),
            Self::Browser(app) => app.process_id(),
        }
    }

    fn exited(&mut self) -> Result<Option<u32>> {
        match self {
            Self::Pty(app) => app.exited(),
            Self::Ghostty(app) => app.exited(),
            Self::Browser(app) => app.exited(),
        }
    }

    fn terminate(&mut self) -> Result<()> {
        match self {
            Self::Pty(app) => app.terminate(),
            Self::Ghostty(app) => app.terminate(),
            Self::Browser(app) => app.terminate(),
        }
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Pty(app) => app.send_bytes(bytes),
            Self::Ghostty(app) => app.send_bytes(bytes),
            Self::Browser(app) => NativeSurface::send_surface_bytes(app, bytes),
        }
    }

    fn send_browser_key_label(&mut self, label: &str) -> Result<bool> {
        let Self::Browser(app) = self else {
            return Ok(false);
        };
        let Some(key) = browser_key_label(label) else {
            return Ok(false);
        };
        match key {
            BrowserSocketKey::Backspace => app.send_backspace()?,
            BrowserSocketKey::Tab => app.send_tab()?,
            BrowserSocketKey::ShiftTab => app.send_shift_tab()?,
            BrowserSocketKey::Enter => app.send_enter()?,
            BrowserSocketKey::Escape => app.send_escape()?,
            BrowserSocketKey::Insert => app.send_insert()?,
            BrowserSocketKey::ShiftInsert => app.send_shift_insert()?,
            BrowserSocketKey::AltInsert => app.send_alt_insert()?,
            BrowserSocketKey::CtrlInsert => app.send_ctrl_insert()?,
            BrowserSocketKey::Delete => app.send_delete()?,
            BrowserSocketKey::ShiftDelete => app.send_shift_delete()?,
            BrowserSocketKey::AltDelete => app.send_alt_delete()?,
            BrowserSocketKey::CtrlDelete => app.send_ctrl_delete()?,
            BrowserSocketKey::Home => app.send_home()?,
            BrowserSocketKey::ShiftHome => app.send_shift_home()?,
            BrowserSocketKey::AltHome => app.send_alt_home()?,
            BrowserSocketKey::CtrlHome => app.send_ctrl_home()?,
            BrowserSocketKey::End => app.send_end()?,
            BrowserSocketKey::ShiftEnd => app.send_shift_end()?,
            BrowserSocketKey::AltEnd => app.send_alt_end()?,
            BrowserSocketKey::CtrlEnd => app.send_ctrl_end()?,
            BrowserSocketKey::Arrow(direction) => app.send_arrow_key(direction)?,
            BrowserSocketKey::ShiftArrow(direction) => app.send_shift_arrow_key(direction)?,
            BrowserSocketKey::AltArrow(direction) => app.send_alt_arrow_key(direction)?,
            BrowserSocketKey::CtrlArrow(direction) => app.send_ctrl_arrow_key(direction)?,
            BrowserSocketKey::Page(direction) => app.send_page_key(direction)?,
            BrowserSocketKey::ShiftPage(direction) => app.send_shift_page_key(direction)?,
            BrowserSocketKey::AltPage(direction) => app.send_alt_page_key(direction)?,
            BrowserSocketKey::CtrlPage(direction) => app.send_ctrl_page_key(direction)?,
            BrowserSocketKey::CtrlLetter(letter) => app.send_ctrl_letter(letter)?,
            BrowserSocketKey::Function(key) => app.send_function_key(key)?,
        }
        Ok(true)
    }
}

impl NativeSurface for NativeTerminalApp {
    fn metadata(&self) -> SurfaceMetadata {
        match self {
            Self::Pty(app) => app.metadata(),
            Self::Ghostty(app) => app.metadata(),
            Self::Browser(app) => app.metadata(),
        }
    }

    fn resize_surface(&mut self, cols: u16, rows: u16) -> Result<()> {
        match self {
            Self::Pty(app) => app.resize_surface(cols, rows),
            Self::Ghostty(app) => app.resize_surface(cols, rows),
            Self::Browser(app) => app.resize_surface(cols, rows),
        }
    }

    fn send_surface_text(&mut self, text: &str) -> Result<()> {
        match self {
            Self::Pty(app) => app.send_surface_text(text),
            Self::Ghostty(app) => app.send_surface_text(text),
            Self::Browser(app) => app.send_surface_text(text),
        }
    }

    fn send_surface_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Pty(app) => app.send_surface_bytes(bytes),
            Self::Ghostty(app) => app.send_surface_bytes(bytes),
            Self::Browser(app) => app.send_surface_bytes(bytes),
        }
    }

    fn send_surface_focus(&mut self, focused: bool) -> Result<()> {
        match self {
            Self::Pty(app) => app.send_surface_focus(focused),
            Self::Ghostty(app) => app.send_surface_focus(focused),
            Self::Browser(app) => app.send_surface_focus(focused),
        }
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        match self {
            Self::Pty(app) => app.capture_surface(),
            Self::Ghostty(app) => app.capture_surface(),
            Self::Browser(app) => app.capture_surface(),
        }
    }

    fn send_surface_pointer(&mut self, event: SurfacePointerEvent) -> Result<()> {
        match self {
            Self::Pty(app) => app.send_surface_pointer(event),
            Self::Ghostty(app) => app.send_surface_pointer(event),
            Self::Browser(app) => app.send_surface_pointer(event),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrowserSocketKey {
    Backspace,
    Tab,
    ShiftTab,
    Enter,
    Escape,
    Insert,
    ShiftInsert,
    AltInsert,
    CtrlInsert,
    Delete,
    ShiftDelete,
    AltDelete,
    CtrlDelete,
    Home,
    ShiftHome,
    AltHome,
    CtrlHome,
    End,
    ShiftEnd,
    AltEnd,
    CtrlEnd,
    Arrow(BrowserArrowKey),
    ShiftArrow(BrowserArrowKey),
    AltArrow(BrowserArrowKey),
    CtrlArrow(BrowserArrowKey),
    Page(BrowserPageKey),
    ShiftPage(BrowserPageKey),
    AltPage(BrowserPageKey),
    CtrlPage(BrowserPageKey),
    CtrlLetter(char),
    Function(BrowserFunctionKey),
}

fn browser_key_label(label: &str) -> Option<BrowserSocketKey> {
    let normalized = label.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "backspace" | "bs" => Some(BrowserSocketKey::Backspace),
        "tab" => Some(BrowserSocketKey::Tab),
        "shift-tab" | "backtab" => Some(BrowserSocketKey::ShiftTab),
        "enter" | "return" => Some(BrowserSocketKey::Enter),
        "escape" | "esc" => Some(BrowserSocketKey::Escape),
        "insert" | "ins" => Some(BrowserSocketKey::Insert),
        "shift-insert" | "shift-ins" => Some(BrowserSocketKey::ShiftInsert),
        "alt-insert" | "alt-ins" => Some(BrowserSocketKey::AltInsert),
        "ctrl-insert" | "ctrl-ins" => Some(BrowserSocketKey::CtrlInsert),
        "delete" | "del" => Some(BrowserSocketKey::Delete),
        "shift-delete" | "shift-del" => Some(BrowserSocketKey::ShiftDelete),
        "alt-delete" | "alt-del" => Some(BrowserSocketKey::AltDelete),
        "ctrl-delete" | "ctrl-del" => Some(BrowserSocketKey::CtrlDelete),
        "home" => Some(BrowserSocketKey::Home),
        "shift-home" => Some(BrowserSocketKey::ShiftHome),
        "alt-home" => Some(BrowserSocketKey::AltHome),
        "ctrl-home" => Some(BrowserSocketKey::CtrlHome),
        "end" => Some(BrowserSocketKey::End),
        "shift-end" => Some(BrowserSocketKey::ShiftEnd),
        "alt-end" => Some(BrowserSocketKey::AltEnd),
        "ctrl-end" => Some(BrowserSocketKey::CtrlEnd),
        "left" | "arrow-left" => Some(BrowserSocketKey::Arrow(BrowserArrowKey::Left)),
        "right" | "arrow-right" => Some(BrowserSocketKey::Arrow(BrowserArrowKey::Right)),
        "up" | "arrow-up" => Some(BrowserSocketKey::Arrow(BrowserArrowKey::Up)),
        "down" | "arrow-down" => Some(BrowserSocketKey::Arrow(BrowserArrowKey::Down)),
        "shift-left" | "shift-arrow-left" => {
            Some(BrowserSocketKey::ShiftArrow(BrowserArrowKey::Left))
        }
        "shift-right" | "shift-arrow-right" => {
            Some(BrowserSocketKey::ShiftArrow(BrowserArrowKey::Right))
        }
        "shift-up" | "shift-arrow-up" => Some(BrowserSocketKey::ShiftArrow(BrowserArrowKey::Up)),
        "shift-down" | "shift-arrow-down" => {
            Some(BrowserSocketKey::ShiftArrow(BrowserArrowKey::Down))
        }
        "alt-left" | "alt-arrow-left" => Some(BrowserSocketKey::AltArrow(BrowserArrowKey::Left)),
        "alt-right" | "alt-arrow-right" => Some(BrowserSocketKey::AltArrow(BrowserArrowKey::Right)),
        "alt-up" | "alt-arrow-up" => Some(BrowserSocketKey::AltArrow(BrowserArrowKey::Up)),
        "alt-down" | "alt-arrow-down" => Some(BrowserSocketKey::AltArrow(BrowserArrowKey::Down)),
        "ctrl-left" | "ctrl-arrow-left" => Some(BrowserSocketKey::CtrlArrow(BrowserArrowKey::Left)),
        "ctrl-right" | "ctrl-arrow-right" => {
            Some(BrowserSocketKey::CtrlArrow(BrowserArrowKey::Right))
        }
        "ctrl-up" | "ctrl-arrow-up" => Some(BrowserSocketKey::CtrlArrow(BrowserArrowKey::Up)),
        "ctrl-down" | "ctrl-arrow-down" => Some(BrowserSocketKey::CtrlArrow(BrowserArrowKey::Down)),
        "pageup" | "page-up" => Some(BrowserSocketKey::Page(BrowserPageKey::Up)),
        "pagedown" | "page-down" => Some(BrowserSocketKey::Page(BrowserPageKey::Down)),
        "shift-pageup" | "shift-page-up" => Some(BrowserSocketKey::ShiftPage(BrowserPageKey::Up)),
        "shift-pagedown" | "shift-page-down" => {
            Some(BrowserSocketKey::ShiftPage(BrowserPageKey::Down))
        }
        "alt-pageup" | "alt-page-up" => Some(BrowserSocketKey::AltPage(BrowserPageKey::Up)),
        "alt-pagedown" | "alt-page-down" => Some(BrowserSocketKey::AltPage(BrowserPageKey::Down)),
        "ctrl-pageup" | "ctrl-page-up" => Some(BrowserSocketKey::CtrlPage(BrowserPageKey::Up)),
        "ctrl-pagedown" | "ctrl-page-down" => {
            Some(BrowserSocketKey::CtrlPage(BrowserPageKey::Down))
        }
        "f5" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F5)),
        "f6" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F6)),
        "f7" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F7)),
        "f8" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F8)),
        "f9" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F9)),
        "f10" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F10)),
        "f11" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F11)),
        "f12" => Some(BrowserSocketKey::Function(BrowserFunctionKey::F12)),
        _ => browser_ctrl_letter_label(&normalized),
    }
}

fn browser_ctrl_letter_label(normalized: &str) -> Option<BrowserSocketKey> {
    let letter = normalized.strip_prefix("ctrl-")?;
    let mut chars = letter.chars();
    let ch = chars.next()?;
    if chars.next().is_none() && ch.is_ascii_lowercase() {
        Some(BrowserSocketKey::CtrlLetter(ch))
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
struct NativeDirtyFrameMetrics {
    changed_tiles: u32,
    total_tiles: u32,
    changed_fraction: f32,
    skipped_upload: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativePaneLayoutAxis {
    Columns,
    Rows,
    Grid,
}

impl NativePaneLayoutAxis {
    fn label(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Rows => "rows",
            Self::Grid => "grid",
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::Columns => Self::Rows,
            Self::Rows => Self::Columns,
            Self::Grid => Self::Columns,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "columns" => Some(Self::Columns),
            "rows" => Some(Self::Rows),
            "grid" => Some(Self::Grid),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativePaneLayoutMode {
    Tiled,
    Floating,
    Fullscreen,
}

impl NativePaneLayoutMode {
    fn label(self, axis: NativePaneLayoutAxis) -> &'static str {
        match self {
            Self::Tiled => axis.label(),
            Self::Floating => "floating",
            Self::Fullscreen => "fullscreen",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct NativeFloatingPaneOffset {
    dx: i16,
    dy: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeFloatingDrag {
    window: String,
    last_col: u16,
    last_row: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeTiledDrag {
    window: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeActiveDragLabel<'a> {
    kind: &'static str,
    window: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativePaneLayout {
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
    app_x: u16,
    app_y: u16,
    app_cols: u16,
    app_rows: u16,
}

impl NativePaneLayout {
    fn hidden() -> Self {
        Self {
            x: 0,
            y: 0,
            cols: 0,
            rows: 0,
            app_x: 0,
            app_y: 0,
            app_cols: 0,
            app_rows: 0,
        }
    }

    fn from_outer(x: u16, y: u16, cols: u16, rows: u16) -> Self {
        let title_rows = NATIVE_PANE_TITLE_ROWS.min(rows);
        Self {
            x,
            y,
            cols,
            rows,
            app_x: x
                .saturating_add(NATIVE_PANE_BORDER_COLS)
                .min(x.saturating_add(cols.saturating_sub(1))),
            app_y: y.saturating_add(title_rows),
            app_cols: cols.saturating_sub(NATIVE_PANE_BORDER_COLS * 2),
            app_rows: rows
                .saturating_sub(NATIVE_PANE_TITLE_ROWS)
                .saturating_sub(NATIVE_PANE_BOTTOM_BORDER_ROWS),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeShellView {
    top_bar: NativeTopBarChrome,
    panes: Vec<NativePaneChrome>,
    footer: NativeFooterChrome,
    help_overlay: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeTopBarChrome {
    row: u16,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativePaneChrome {
    x: u16,
    y: u16,
    focused: bool,
    text: String,
    cache_key: String,
    status: String,
    app_x: u16,
    app_y: u16,
    app_cols: u16,
    app_rows: u16,
    cols: u16,
    rows: u16,
    text_snapshot: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeFooterChrome {
    row: u16,
    text: String,
}

fn native_shell_view(
    cols: u16,
    rows: u16,
    panes: &[NativePane],
    focused: usize,
    layouts: &[NativePaneLayout],
    floating_offsets: &HashMap<String, NativeFloatingPaneOffset>,
    sock: &str,
    log_path: &str,
    layout_label: &str,
    active_drag_label: Option<NativeActiveDragLabel<'_>>,
    help_overlay: bool,
    include_text_snapshots: bool,
) -> NativeShellView {
    let focused_footer_label = panes.get(focused).map(native_status_pane_focus_label);
    let mut focused_status_label = None;
    let pane_chrome = panes
        .iter()
        .enumerate()
        .filter_map(|(idx, pane)| {
            let layout = layouts.get(idx).copied()?;
            let is_focused = idx == focused;
            let floating_moved = floating_offsets
                .get(&pane.window)
                .is_some_and(|offset| *offset != NativeFloatingPaneOffset::default());
            let active_drag = active_drag_label.is_some_and(|drag| drag.window == pane.window);
            let text = native_pane_title_text(
                pane,
                layout,
                is_focused,
                active_drag,
                native_title_drag_affordance_enabled(layout_label),
                native_title_reorder_affordance_enabled(layout_label),
                native_title_fullscreen_affordance_enabled(layout_label),
                idx + 1 == panes.len(),
                floating_moved,
            );
            let cache_key = native_pane_title_key_from_text(&text, layout, is_focused);
            let status = native_pane_status_chip_text(pane);
            if is_focused {
                focused_status_label = Some(status.clone());
            }
            Some(NativePaneChrome {
                x: layout.x,
                y: layout.y,
                focused: is_focused,
                text,
                cache_key,
                status,
                app_x: layout.app_x,
                app_y: layout.app_y,
                app_cols: layout.app_cols,
                app_rows: layout.app_rows,
                cols: layout.cols,
                rows: layout.rows,
                text_snapshot: if include_text_snapshots {
                    pane.app.text_snapshot()
                } else {
                    String::new()
                },
            })
        })
        .collect();
    NativeShellView {
        top_bar: NativeTopBarChrome {
            row: 0,
            text: native_top_bar_text(1, panes.len(), sock, cols),
        },
        panes: pane_chrome,
        footer: NativeFooterChrome {
            row: native_footer_row(rows),
            text: native_status_line_text(
                panes.len(),
                layout_label,
                focused_footer_label.as_deref(),
                focused_status_label.as_deref(),
                active_drag_label,
                log_path,
            ),
        },
        help_overlay,
    }
}

fn native_top_bar_text(_workspace_id: u16, panes: usize, _sock: &str, cols: u16) -> String {
    // Text top-bar rendering only displays workspace chips and clock. Avoid
    // carrying the socket path into the BarModel on every native redraw; the
    // graphical scene path still derives focus metadata from the view when it
    // needs diagnostic labels.
    BarModel::live(workspace_label(), panes, "-", std::time::SystemTime::now())
        .render_i3bar(cols as usize)
}

fn native_image_id_set(panes: &[NativePane]) -> HashSet<u32> {
    panes.iter().map(|pane| pane.image_id).collect()
}

fn retired_native_image_ids(previous: &HashSet<u32>, current: &HashSet<u32>) -> Vec<u32> {
    let mut retired = previous.difference(current).copied().collect::<Vec<_>>();
    retired.sort_unstable();
    retired
}

const NATIVE_PANE_STATUS_COMMAND_MAX_CHARS: usize = 64;

fn native_pane_status_chip_text(pane: &NativePane) -> String {
    let command = bounded_ellipsis(&pane.command, NATIVE_PANE_STATUS_COMMAND_MAX_CHARS);
    let mut out = String::with_capacity(command.len().saturating_add(48));
    out.push_str(&command);
    if pane.weight != 1 {
        out.push_str(" · wt:");
        out.push_str(&pane.weight.to_string());
    }
    out.push_str(" · pid:");
    if let Some(pid) = pane.pid {
        out.push_str(&pid.to_string());
    } else {
        out.push('-');
    }
    out.push_str(" · frame:");
    if let Some(metrics) = pane.dirty_frame.as_ref() {
        if metrics.is_clean() {
            out.push_str("clean");
        } else {
            out.push_str(&metrics.changed_tiles_label());
        }
    } else {
        out.push_str("new");
    }
    out
}

fn bounded_ellipsis(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut chars = text.chars();
    let mut out = String::with_capacity(max_chars.min(text.len()));
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return out;
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
}

fn native_footer_row(rows: u16) -> u16 {
    rows.saturating_sub(1)
}

fn terminal_visible_row(row: u16, rows: u16) -> u16 {
    row.min(rows.saturating_sub(1))
}

fn terminal_visible_row_opt(row: u16, rows: u16) -> Option<u16> {
    (rows > 0 && row < rows).then_some(row)
}

fn terminal_visible_width(x: u16, desired: u16, cols: u16) -> Option<usize> {
    (x < cols)
        .then_some(cols.saturating_sub(x).min(desired) as usize)
        .filter(|width| *width > 0)
}

const NATIVE_STATUS_LOG_PATH_MAX_CHARS: usize = 96;
const NATIVE_STATUS_FOCUS_MAX_CHARS: usize = 48;
const NATIVE_STATUS_STATE_MAX_CHARS: usize = 96;
const NATIVE_STATUS_LINE_PREFIX: &str = " mode:";
const NATIVE_STATUS_LINE_PANES_PREFIX: &str = " · panes:";
const NATIVE_STATUS_LINE_FOCUS_PREFIX: &str = " · focus:";
const NATIVE_STATUS_LINE_STATE_PREFIX: &str = " · state:";
const NATIVE_STATUS_LINE_DRAG_PREFIX: &str = " · drag:";
const NATIVE_STATUS_LINE_HINTS: &str =
    " · C-a ? help · C-a n/p focus · C-a t float · C-a f full · C-a wasd nudge · C-a {} stack · C-a r/R reset · C-a e split · C-a x close · Ctrl-] exit";
const NATIVE_STATUS_LINE_LOG_PREFIX: &str = " · log: ";

fn native_status_line_text(
    panes: usize,
    layout_label: &str,
    focused_window: Option<&str>,
    focused_status: Option<&str>,
    active_drag_label: Option<NativeActiveDragLabel<'_>>,
    log_path: &str,
) -> String {
    if panes == 0 {
        String::new()
    } else {
        let log_path = bounded_ellipsis(log_path, NATIVE_STATUS_LOG_PATH_MAX_CHARS);
        let mode = native_status_layout_label(layout_label);
        let pane_count = panes.to_string();
        let focused_window = native_status_focused_window_label(focused_window);
        let focused_status =
            focused_status.map(|status| bounded_ellipsis(status, NATIVE_STATUS_STATE_MAX_CHARS));
        let focused_status_len = focused_status
            .as_ref()
            .map(|status| NATIVE_STATUS_LINE_STATE_PREFIX.len() + status.len())
            .unwrap_or(0);
        let active_drag_len = active_drag_label
            .map(|label| NATIVE_STATUS_LINE_DRAG_PREFIX.len() + label.bounded_len())
            .unwrap_or(0);
        let mut out = String::with_capacity(
            NATIVE_STATUS_LINE_PREFIX.len()
                + mode.len()
                + NATIVE_STATUS_LINE_PANES_PREFIX.len()
                + pane_count.len()
                + NATIVE_STATUS_LINE_FOCUS_PREFIX.len()
                + focused_window.len()
                + focused_status_len
                + active_drag_len
                + NATIVE_STATUS_LINE_HINTS.len()
                + NATIVE_STATUS_LINE_LOG_PREFIX.len()
                + log_path.len(),
        );
        out.push_str(NATIVE_STATUS_LINE_PREFIX);
        out.push_str(&mode);
        out.push_str(NATIVE_STATUS_LINE_PANES_PREFIX);
        out.push_str(&pane_count);
        out.push_str(NATIVE_STATUS_LINE_FOCUS_PREFIX);
        out.push_str(&focused_window);
        if let Some(focused_status) = focused_status.as_deref() {
            out.push_str(NATIVE_STATUS_LINE_STATE_PREFIX);
            out.push_str(focused_status);
        }
        if let Some(active_drag_label) = active_drag_label {
            out.push_str(NATIVE_STATUS_LINE_DRAG_PREFIX);
            active_drag_label.push_bounded(&mut out);
        }
        out.push_str(NATIVE_STATUS_LINE_HINTS);
        out.push_str(NATIVE_STATUS_LINE_LOG_PREFIX);
        out.push_str(&log_path);
        out
    }
}

fn native_status_layout_label(layout_label: &str) -> String {
    let trimmed = layout_label.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        bounded_ellipsis(trimmed, 32)
    }
}

fn native_status_focused_window_label(focused_window: Option<&str>) -> String {
    let trimmed = focused_window
        .map(str::trim)
        .filter(|window| !window.is_empty());
    trimmed
        .map(|window| bounded_ellipsis(window, NATIVE_STATUS_FOCUS_MAX_CHARS))
        .unwrap_or_else(|| "-".to_string())
}

impl NativeActiveDragLabel<'_> {
    fn bounded_len(self) -> usize {
        self.kind
            .chars()
            .chain(":".chars())
            .chain(self.window.chars())
            .take(NATIVE_STATUS_FOCUS_MAX_CHARS)
            .count()
    }

    fn push_bounded(self, out: &mut String) {
        let mut chars = self
            .kind
            .chars()
            .chain(":".chars())
            .chain(self.window.chars());
        for _ in 0..NATIVE_STATUS_FOCUS_MAX_CHARS {
            let Some(ch) = chars.next() else {
                return;
            };
            out.push(ch);
        }
        if chars.next().is_some() {
            out.pop();
            out.push('…');
        }
    }
}

fn native_active_drag_label<'a>(
    floating_drag: Option<&'a NativeFloatingDrag>,
    tiled_drag: Option<&'a NativeTiledDrag>,
) -> Option<NativeActiveDragLabel<'a>> {
    floating_drag
        .map(|drag| NativeActiveDragLabel {
            kind: "move",
            window: drag.window.as_str(),
        })
        .or_else(|| {
            tiled_drag.map(|drag| NativeActiveDragLabel {
                kind: "reorder",
                window: drag.window.as_str(),
            })
        })
}

fn native_status_pane_focus_label(pane: &NativePane) -> String {
    let title = native_pane_display_title(pane);
    let title = title.trim();
    if title.is_empty() || title == pane.window {
        native_status_focused_window_label(Some(&pane.window))
    } else {
        let label = native_status_pane_focus_combined_label(&pane.window, title);
        native_status_focused_window_label(Some(&label))
    }
}

fn native_status_pane_focus_combined_label(window: &str, title: &str) -> String {
    let mut label = String::with_capacity(window.len() + 2 + title.len());
    label.push_str(window);
    label.push('(');
    label.push_str(title);
    label.push(')');
    label
}

fn native_footer_visible_text(text: &str, cols: u16) -> String {
    clip_and_pad(text, cols as usize)
}

fn native_help_overlay_lines() -> &'static [&'static str] {
    crate::shortcuts::NATIVE_SHORTCUTS
}

fn native_startup_terminal_enabled() -> bool {
    matches!(
        std::env::var("KITTWM_STARTUP_TERMINAL")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn native_should_use_pure_terminal_renderer() -> bool {
    match std::env::var("KITTWM_NATIVE_RENDERER") {
        Ok(value) => matches!(value.as_str(), "terminal" | "text" | "ansi" | "dec"),
        Err(_) => std::env::var_os("TMUX").is_some(),
    }
}

fn native_should_use_affordance_scene_chrome() -> bool {
    match std::env::var("KITTWM_NATIVE_CHROME_RENDERER") {
        Ok(value) => !matches!(
            value.to_ascii_lowercase().as_str(),
            "terminal" | "text" | "ansi" | "ascii" | "off" | "0" | "false"
        ),
        Err(_) => true,
    }
}

fn native_dirty_frames_skip_unchanged() -> bool {
    !matches!(
        std::env::var("KITTWM_DIRTY_FRAMES")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "always-upload" | "always_upload" | "off" | "0" | "false"
    )
}

struct NativeDirtyFramePolicy {
    skip_unchanged: bool,
    grids: BTreeMap<u32, DirtyGrid>,
}

impl NativeDirtyFramePolicy {
    fn from_env() -> Self {
        Self {
            skip_unchanged: native_dirty_frames_skip_unchanged(),
            grids: BTreeMap::new(),
        }
    }

    fn decide(
        &mut self,
        image_id: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> NativeDirtyFrameDecision {
        let Some(diff) = self.diff(image_id, width, height, rgba) else {
            return NativeDirtyFrameDecision::upload_without_metrics();
        };
        let upload = !self.skip_unchanged || !diff.is_clean();
        NativeDirtyFrameDecision {
            upload,
            metrics: NativeDirtyFrameMetrics::from_diff(&diff, !upload),
        }
    }

    fn forget(&mut self, image_id: u32) {
        self.grids.remove(&image_id);
    }

    fn diff(
        &mut self,
        image_id: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Option<DirtyFrameDiff> {
        let grid = self
            .grids
            .entry(image_id)
            .or_insert_with(|| DirtyGrid::new(64, 64));
        grid.diff_rgba(width, height, rgba)
    }
}

struct NativeDirtyFrameDecision {
    upload: bool,
    metrics: NativeDirtyFrameMetrics,
}

impl NativeDirtyFrameDecision {
    fn upload_without_metrics() -> Self {
        Self {
            upload: true,
            metrics: NativeDirtyFrameMetrics {
                changed_tiles: 0,
                total_tiles: 0,
                changed_fraction: 0.0,
                skipped_upload: false,
            },
        }
    }
}

impl NativeDirtyFrameMetrics {
    fn from_diff(diff: &DirtyFrameDiff, skipped_upload: bool) -> Self {
        Self {
            changed_tiles: diff.changed_count(),
            total_tiles: diff.tiles,
            changed_fraction: diff.changed_fraction(),
            skipped_upload,
        }
    }

    fn is_clean(&self) -> bool {
        self.skipped_upload || self.changed_tiles == 0
    }

    fn changed_tiles_label(&self) -> String {
        if self.total_tiles == 0 {
            return self.changed_tiles.to_string();
        }
        let mut out = String::with_capacity(21);
        use std::fmt::Write as _;
        let _ = write!(out, "{}/{}", self.changed_tiles, self.total_tiles);
        out
    }
}

#[cfg(test)]
const NATIVE_TOP_BAR_ROWS: u16 = 1;
const NATIVE_PANE_TITLE_ROWS: u16 = 1;
const NATIVE_PANE_BORDER_COLS: u16 = 1;
const NATIVE_PANE_BOTTOM_BORDER_ROWS: u16 = 1;
pub const NATIVE_BASE_CELL_WIDTH_PX: u32 = 8;
pub const NATIVE_BASE_CELL_HEIGHT_PX: u32 = 16;
pub const NATIVE_HIDPI_SCALE: u32 = 2;
pub const NATIVE_MAX_CELL_WIDTH_PX: u32 = 64;
pub const NATIVE_MAX_CELL_HEIGHT_PX: u32 = 128;
pub const NATIVE_MAX_GAP_CELLS: u16 = 20;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct NativeDisplayTuning {
    pub hidpi_enabled: bool,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    pub tile_gap_px: u32,
    pub header_gap_px: u32,
    pub footer_gap_px: u32,
    pub tile_gap_cols: u16,
    pub tile_gap_rows: u16,
    pub header_gap_rows: u16,
    pub footer_gap_rows: u16,
}
fn native_z_index(role: SurfacePlacementRole) -> i32 {
    ArchitectureContract::current()
        .z_index_for_role(role)
        .expect("current kittwm architecture contract defines all placement roles")
}

fn native_app_z_index() -> i32 {
    native_z_index(SurfacePlacementRole::AppSurface)
}

fn native_chrome_z_index() -> i32 {
    native_z_index(SurfacePlacementRole::Decoration)
}
const NATIVE_FRAME_BG_RGBA: [u8; 4] = [0x08, 0x0d, 0x14, 0xff];

fn native_cell_size() -> CellSize {
    CellSize::new(
        native_cell_width_px() as u16,
        native_cell_height_px() as u16,
    )
}

pub fn native_display_tuning() -> NativeDisplayTuning {
    let cell_width_px = native_cell_width_px();
    let cell_height_px = native_cell_height_px();
    let tile_gap_px = native_pixel_gap_px_from_env("KITTWM_TILE_GAP_PX");
    let header_gap_px = native_pixel_gap_px_from_env("KITTWM_HEADER_GAP_PX");
    let footer_gap_px = native_pixel_gap_px_from_env("KITTWM_FOOTER_GAP_PX");
    NativeDisplayTuning {
        hidpi_enabled: native_hidpi_enabled(),
        cell_width_px,
        cell_height_px,
        tile_gap_px,
        header_gap_px,
        footer_gap_px,
        tile_gap_cols: native_pixel_gap_cells_from_px(tile_gap_px, cell_width_px),
        tile_gap_rows: native_pixel_gap_cells_from_px(tile_gap_px, cell_height_px),
        header_gap_rows: native_pixel_gap_cells_from_px(header_gap_px, cell_height_px),
        footer_gap_rows: native_pixel_gap_cells_from_px(footer_gap_px, cell_height_px),
    }
}

fn native_cell_width_px() -> u32 {
    native_cell_px_from_env_or_host(
        "KITTWM_NATIVE_CELL_WIDTH_PX",
        HostTerminalAxis::Columns,
        NATIVE_BASE_CELL_WIDTH_PX,
        NATIVE_MAX_CELL_WIDTH_PX,
    )
}

fn native_cell_height_px() -> u32 {
    native_cell_px_from_env_or_host(
        "KITTWM_NATIVE_CELL_HEIGHT_PX",
        HostTerminalAxis::Rows,
        NATIVE_BASE_CELL_HEIGHT_PX,
        NATIVE_MAX_CELL_HEIGHT_PX,
    )
}

fn native_cell_px_from_env_or_host(key: &str, axis: HostTerminalAxis, base: u32, max: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| parse_positive_u32(&value))
        .or_else(|| inferred_native_cell_px(axis))
        .unwrap_or_else(|| {
            if native_hidpi_enabled() {
                base.saturating_mul(NATIVE_HIDPI_SCALE)
            } else {
                base
            }
        })
        .clamp(1, max)
}

fn inferred_native_cell_px(axis: HostTerminalAxis) -> Option<u32> {
    let snapshot = host_terminal_snapshot()?;
    let (pixels, cells) = match axis {
        HostTerminalAxis::Columns => (snapshot.width_px?, snapshot.cols),
        HostTerminalAxis::Rows => (snapshot.height_px?, snapshot.rows),
    };
    infer_cell_px_from_terminal_pixels(pixels, cells)
}

fn infer_cell_px_from_terminal_pixels(pixels: u32, cells: u16) -> Option<u32> {
    (pixels > 0 && cells > 0).then_some((pixels / u32::from(cells)).max(1))
}

fn native_hidpi_enabled() -> bool {
    !matches!(
        std::env::var("KITTWM_HIDPI")
            .unwrap_or_else(|_| "1".to_string())
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

fn native_pixel_gap_px_from_env(px_key: &str) -> u32 {
    std::env::var(px_key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0)
}

fn native_pixel_gap_cells(px_key: &str, cell_px: u32) -> u16 {
    native_pixel_gap_cells_from_px(native_pixel_gap_px_from_env(px_key), cell_px)
}

fn native_pixel_gap_cells_from_px(px: u32, cell_px: u32) -> u16 {
    if px == 0 {
        return 0;
    }
    px.div_ceil(cell_px.max(1))
        .min(u32::from(NATIVE_MAX_GAP_CELLS)) as u16
}

fn native_chrome_reservation_with_pixel_gaps(
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> crate::daemon::NativeChromeReservationConfig {
    let mut reservation = reservation.clone();
    let cell_width = native_cell_width_px();
    let cell_height = native_cell_height_px();
    let tile_gap_cols = native_pixel_gap_cells("KITTWM_TILE_GAP_PX", cell_width);
    let tile_gap_rows = native_pixel_gap_cells("KITTWM_TILE_GAP_PX", cell_height);
    let header_gap_rows = native_pixel_gap_cells("KITTWM_HEADER_GAP_PX", cell_height);
    let footer_gap_rows = native_pixel_gap_cells("KITTWM_FOOTER_GAP_PX", cell_height);
    reservation.gap_cols = reservation
        .gap_cols
        .saturating_add(tile_gap_cols)
        .min(NATIVE_MAX_GAP_CELLS);
    reservation.gap_rows = reservation
        .gap_rows
        .saturating_add(tile_gap_rows)
        .min(NATIVE_MAX_GAP_CELLS);
    reservation.top_bar_rows = reservation
        .top_bar_rows
        .saturating_add(header_gap_rows)
        .min(20);
    reservation.bottom_bar_rows = reservation
        .bottom_bar_rows
        .saturating_add(footer_gap_rows)
        .min(20);
    reservation
}

fn fit_rgba_frame_to_cells(
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    cols: u16,
    rows: u16,
) -> (Vec<u8>, u32, u32) {
    let cell_width = native_cell_width_px();
    let cell_height = native_cell_height_px();
    let target_width = u32::from(cols).saturating_mul(cell_width).max(1);
    let target_height = u32::from(rows).saturating_mul(cell_height).max(1);
    let expected_len = target_width as usize * target_height as usize * 4;
    if width == target_width && height == target_height && rgba.len() == expected_len {
        return (rgba, width, height);
    }
    let mut fitted = vec![0u8; expected_len];
    for px in fitted.chunks_exact_mut(4) {
        px.copy_from_slice(&NATIVE_FRAME_BG_RGBA);
    }
    let copy_width = width.min(target_width) as usize;
    let copy_height = height.min(target_height) as usize;
    let src_stride = width as usize * 4;
    let dst_stride = target_width as usize * 4;
    let copy_bytes = copy_width * 4;
    for row in 0..copy_height {
        let src_start = row * src_stride;
        let src_end = src_start.saturating_add(copy_bytes).min(rgba.len());
        let dst_start = row * dst_stride;
        let dst_end = dst_start + (src_end.saturating_sub(src_start));
        if src_start >= rgba.len() || dst_end > fitted.len() {
            break;
        }
        fitted[dst_start..dst_end].copy_from_slice(&rgba[src_start..src_end]);
    }
    (fitted, target_width, target_height)
}

fn native_pane_index(panes: &[NativePane], window: &str) -> Option<usize> {
    panes.iter().position(|pane| pane.window == window)
}

fn native_target_pane_index(panes: &[NativePane], focused: usize, window: &str) -> Option<usize> {
    if window == "focused" {
        (!panes.is_empty()).then_some(focused.min(panes.len().saturating_sub(1)))
    } else {
        native_pane_index(panes, window)
    }
}

fn next_native_pane_id(panes: &[NativePane]) -> u32 {
    panes
        .iter()
        .filter_map(|pane| pane.window.strip_prefix("native-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

const NATIVE_CTRL_C_EXIT_THRESHOLD: u8 = 3;
const NATIVE_CTRL_C_EXIT_WINDOW: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default)]
struct NativeCtrlCExitGuard {
    count: u8,
    last: Option<Instant>,
}

impl NativeCtrlCExitGuard {
    fn observe(&mut self, now: Instant) -> bool {
        if self
            .last
            .and_then(|last| now.checked_duration_since(last))
            .is_some_and(|elapsed| elapsed <= NATIVE_CTRL_C_EXIT_WINDOW)
        {
            self.count = self.count.saturating_add(1);
        } else {
            self.count = 1;
        }
        self.last = Some(now);
        self.count >= NATIVE_CTRL_C_EXIT_THRESHOLD
    }

    fn reset(&mut self) {
        self.count = 0;
        self.last = None;
    }
}

fn native_ctrl_c_action(guard: &mut NativeCtrlCExitGuard, now: Instant) -> NativeCtrlCAction {
    if guard.observe(now) {
        NativeCtrlCAction::Confirm
    } else {
        NativeCtrlCAction::Forward
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeCtrlCAction {
    Forward,
    Confirm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeQuitConfirmByteAction {
    Consumed,
    Cancel,
    Confirm,
}

fn native_quit_confirm_byte_action(
    overlay: &mut QuitConfirmOverlay,
    byte: u8,
    now: Instant,
) -> NativeQuitConfirmByteAction {
    if overlay.expired(now) {
        overlay.close();
        return NativeQuitConfirmByteAction::Cancel;
    }
    match byte {
        b'y' | b'Y' => NativeQuitConfirmByteAction::Confirm,
        b'n' | b'N' | b'q' | b'Q' | 0x1b => {
            overlay.close();
            NativeQuitConfirmByteAction::Cancel
        }
        0x03 => NativeQuitConfirmByteAction::Consumed,
        _ => NativeQuitConfirmByteAction::Consumed,
    }
}

fn native_terminal_command(config: &KittwmConfig) -> String {
    std::env::var("KITTWM_TERMINAL_CMD")
        .or_else(|_| std::env::var("KITTWM_TERMINAL_BINARY"))
        .or_else(|_| {
            config
                .terminal
                .command
                .clone()
                .ok_or(std::env::VarError::NotPresent)
        })
        .or_else(|_| std::env::var("SHELL").map(native_login_shell_command))
        .unwrap_or_else(|_| "/bin/sh -l".to_string())
}

fn native_login_shell_command(shell: String) -> String {
    let mut out = String::with_capacity(shell.len() + " -l".len());
    out.push_str(&shell);
    out.push_str(" -l");
    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeTerminalBackend {
    Pty,
    Ghostty,
}

fn native_terminal_backend(config: &KittwmConfig) -> NativeTerminalBackend {
    let configured = std::env::var("KITTWM_TERMINAL_BACKEND")
        .or_else(|_| std::env::var("KITTWM_TERMINAL_APP"))
        .unwrap_or_else(|_| config.terminal.backend.clone())
        .to_ascii_lowercase();
    if matches!(
        configured.as_str(),
        "ghostty" | "libghostty" | "ghostty-vt" | "kittui-ghostty"
    ) {
        NativeTerminalBackend::Ghostty
    } else {
        NativeTerminalBackend::Pty
    }
}

fn nord_or_hex_rgba(value: &str, alpha: u8) -> [u8; 4] {
    let lower = value.to_ascii_lowercase();
    let hex = match lower.as_str() {
        "nord0" => "#2e3440",
        "nord1" => "#3b4252",
        "nord2" => "#434c5e",
        "nord3" => "#4c566a",
        "nord4" => "#d8dee9",
        "nord5" => "#e5e9f0",
        "nord6" => "#eceff4",
        "nord7" => "#8fbcbb",
        "nord8" => "#88c0d0",
        "nord9" => "#81a1c1",
        "nord10" => "#5e81ac",
        "nord11" => "#bf616a",
        "nord12" => "#d08770",
        "nord13" => "#ebcb8b",
        "nord14" => "#a3be8c",
        "nord15" => "#b48ead",
        other => other,
    };
    let parsed = Rgba::parse(hex).unwrap_or(Rgba(0x2e, 0x34, 0x40, alpha));
    [parsed.0, parsed.1, parsed.2, alpha]
}

fn libghostty_preview_options(config: &LibghosttyConfig) -> PreviewOptions {
    let mut options = PreviewOptions::default();
    let bg_alpha = (config.background_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    options.background = nord_or_hex_rgba(&config.background, bg_alpha);
    options.foreground = nord_or_hex_rgba(&config.foreground, 255);
    options.cursor = nord_or_hex_rgba(&config.cursor, 255);
    options
}

fn kittwm_browser_target_from_command(cmd: &str) -> Option<String> {
    let rest = cmd.trim().strip_prefix("kittwm-browser")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let target = first_shell_word(rest.trim())?;
    (!target.trim().is_empty()).then_some(target)
}

fn first_shell_word(input: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;
    let mut consumed = false;
    while let Some(ch) = chars.next() {
        consumed = true;
        match quote {
            Some(q) if ch == q => quote = None,
            Some('\'') => out.push(ch),
            Some('"') if ch == '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            Some(_) => out.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => break,
            None if ch == '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            None => out.push(ch),
        }
    }
    (consumed && quote.is_none()).then_some(out)
}

fn native_pane_window_id(id: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("native-".len() + 10);
    out.push_str("native-");
    let _ = write!(out, "{id}");
    out
}

fn spawn_native_pane(id: u32, cmd: &str, sock: &str, cols: u16, rows: u16) -> Result<NativePane> {
    let config = KittwmConfig::load_default().unwrap_or_default();
    let window = native_pane_window_id(id);
    let mut envs = vec![
        ("KITTWM_SOCKET".to_string(), sock.to_string()),
        ("KITTWM_SOCK".to_string(), sock.to_string()),
        ("KITTUI_WM_DISPLAY".to_string(), sock.to_string()),
        ("KITTWM_DISPLAY".to_string(), sock.to_string()),
        ("KITTWM_WINDOW".to_string(), window.clone()),
    ];
    if config.libghostty.enable_ghostty_features {
        envs.extend([
            ("TERM".to_string(), "xterm-ghostty".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
        ]);
    }
    if config.libghostty.kitty_graphics {
        envs.extend([
            ("TERM_PROGRAM".to_string(), "ghostty".to_string()),
            ("KITTY_WINDOW_ID".to_string(), window.clone()),
            ("KITTWM_INNER_KITTY_GRAPHICS".to_string(), "1".to_string()),
        ]);
    }
    let app =
        if let Some(target) = kittwm_browser_target_from_command(cmd) {
            NativeTerminalApp::Browser(HeadlessBrowserApp::launch(
                &target,
                u32::from(cols.max(1)) * 8,
                u32::from(rows.max(1)) * 16,
            )?)
        } else {
            match native_terminal_backend(&config) {
                NativeTerminalBackend::Pty => NativeTerminalApp::Pty(
                    PtyTerminalApp::spawn_with_env(cmd, cols.max(1), rows.max(1), envs)?,
                ),
                NativeTerminalBackend::Ghostty => {
                    NativeTerminalApp::Ghostty(GhosttyTerminalApp::spawn_with_env_and_preview(
                        cmd,
                        cols.max(1),
                        rows.max(1),
                        envs,
                        libghostty_preview_options(&config.libghostty),
                    )?)
                }
            }
        };
    let pid = app.process_id();
    Ok(NativePane {
        window,
        image_id: 0x6b77_0000 | id,
        command: cmd.to_string(),
        pid,
        display_title: None,
        weight: 1,
        app,
        dirty_frame: None,
        capture_failed: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_native_terminal_byte(
    byte: u8,
    prefix: &mut bool,
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    layout_axis: &mut NativePaneLayoutAxis,
    layout_mode: &mut NativePaneLayoutMode,
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    cmd: &str,
    sock: &str,
    cols: u16,
    rows: u16,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    clear: &mut bool,
    help_overlay: &mut bool,
    ctrl_c_exit_guard: &mut NativeCtrlCExitGuard,
    quit_confirm_overlay: &mut QuitConfirmOverlay,
    dbg: &Debugger,
) -> Result<bool> {
    if quit_confirm_overlay.active {
        match native_quit_confirm_byte_action(quit_confirm_overlay, byte, Instant::now()) {
            NativeQuitConfirmByteAction::Confirm => {
                dbg.log("native terminal quit confirmation accepted");
                return Ok(true);
            }
            NativeQuitConfirmByteAction::Cancel => {
                ctrl_c_exit_guard.reset();
                *clear = true;
                dbg.log("native terminal quit confirmation cancelled");
                return Ok(false);
            }
            NativeQuitConfirmByteAction::Consumed => return Ok(false),
        }
    }
    if byte == 0x1d {
        dbg.log("native terminal loop: Ctrl-] exit");
        return Ok(true);
    }
    if *prefix {
        *prefix = false;
        match byte {
            b'?' => {
                *help_overlay = !*help_overlay;
                *clear = true;
                dbg.log(&native_help_overlay_log_line(*help_overlay));
            }
            b'\r' | b'\n' => {
                native_launch_terminal_pane(
                    panes,
                    focused,
                    *layout_axis,
                    cmd,
                    sock,
                    cols,
                    rows,
                    reservation,
                    clear,
                    dbg,
                )?;
            }
            b't' | b'T' => {
                *layout_mode = if matches!(*layout_mode, NativePaneLayoutMode::Floating) {
                    NativePaneLayoutMode::Tiled
                } else {
                    NativePaneLayoutMode::Floating
                };
                resize_native_panes_for_display_mode(
                    panes,
                    *focused,
                    cols,
                    rows,
                    *layout_axis,
                    *layout_mode,
                    reservation,
                )?;
                *clear = true;
                dbg.log(&native_layout_mode_log_line(
                    layout_mode.label(*layout_axis),
                ));
            }
            b'f' | b'F' => {
                *layout_mode = if matches!(*layout_mode, NativePaneLayoutMode::Fullscreen) {
                    NativePaneLayoutMode::Tiled
                } else {
                    NativePaneLayoutMode::Fullscreen
                };
                resize_native_panes_for_display_mode(
                    panes,
                    *focused,
                    cols,
                    rows,
                    *layout_axis,
                    *layout_mode,
                    reservation,
                )?;
                *clear = true;
                dbg.log(&native_layout_mode_log_line(
                    layout_mode.label(*layout_axis),
                ));
            }
            b'e' | b'E' => {
                *layout_mode = NativePaneLayoutMode::Tiled;
                *layout_axis = layout_axis.toggled();
                resize_native_panes_for_display_mode(
                    panes,
                    *focused,
                    cols,
                    rows,
                    *layout_axis,
                    *layout_mode,
                    reservation,
                )?;
                *clear = true;
                dbg.log(&native_split_toggle_log_line(layout_axis.label()));
            }
            b'%' | b'|' | b'v' | b'V' => {
                *layout_mode = NativePaneLayoutMode::Tiled;
                *layout_axis = NativePaneLayoutAxis::Columns;
                if panes.is_empty() {
                    native_launch_terminal_pane(
                        panes,
                        focused,
                        *layout_axis,
                        cmd,
                        sock,
                        cols,
                        rows,
                        reservation,
                        clear,
                        dbg,
                    )?;
                } else {
                    native_split_focused(
                        panes,
                        focused,
                        *layout_axis,
                        cmd,
                        sock,
                        cols,
                        rows,
                        reservation,
                        clear,
                        dbg,
                    )?;
                }
            }
            b'-' | b'\"' | b'h' | b'H' => {
                *layout_mode = NativePaneLayoutMode::Tiled;
                *layout_axis = NativePaneLayoutAxis::Rows;
                if panes.is_empty() {
                    native_launch_terminal_pane(
                        panes,
                        focused,
                        *layout_axis,
                        cmd,
                        sock,
                        cols,
                        rows,
                        reservation,
                        clear,
                        dbg,
                    )?;
                } else {
                    native_split_focused(
                        panes,
                        focused,
                        *layout_axis,
                        cmd,
                        sock,
                        cols,
                        rows,
                        reservation,
                        clear,
                        dbg,
                    )?;
                }
            }
            b'\t' | b'n' | b'N' => {
                if !panes.is_empty() {
                    let new_focus = next_native_focus(*focused, panes.len());
                    native_set_focus(panes, focused, new_focus)?;
                    *clear = true;
                    dbg.log(&native_keyboard_focus_log_line(&panes[*focused].window));
                }
            }
            b'p' | b'P' => {
                if !panes.is_empty() {
                    let new_focus = prev_native_focus(*focused, panes.len());
                    native_set_focus(panes, focused, new_focus)?;
                    *clear = true;
                    dbg.log(&native_keyboard_focus_prev_log_line(
                        &panes[*focused].window,
                    ));
                }
            }
            b'x' | b'X' => {
                if !panes.is_empty() {
                    let _ = native_send_focus_event(&mut panes[*focused], false);
                    native_terminate_pane_logged(&mut panes[*focused], dbg);
                    panes.remove(*focused);
                    if panes.is_empty() {
                        *focused = 0;
                    } else {
                        *focused = focus_after_remove(*focused, *focused, panes.len() + 1);
                        let _ = native_send_focus_event(&mut panes[*focused], true);
                        resize_native_panes_for_display_mode(
                            panes,
                            *focused,
                            cols,
                            rows,
                            *layout_axis,
                            *layout_mode,
                            reservation,
                        )?;
                    }
                    *clear = true;
                    dbg.log(&native_close_panes_log_line(panes.len()));
                }
            }
            b'+' | b'=' => {
                if !panes.is_empty() {
                    panes[*focused].weight = native_adjust_weight(panes[*focused].weight, 1);
                    resize_native_panes_for_display_mode(
                        panes,
                        *focused,
                        cols,
                        rows,
                        *layout_axis,
                        *layout_mode,
                        reservation,
                    )?;
                    *clear = true;
                    dbg.log(&native_resize_weight_log_line(
                        "grow",
                        &panes[*focused].window,
                        panes[*focused].weight,
                    ));
                }
            }
            b'_' | b'<' => {
                if !panes.is_empty() {
                    panes[*focused].weight = native_adjust_weight(panes[*focused].weight, -1);
                    resize_native_panes_for_display_mode(
                        panes,
                        *focused,
                        cols,
                        rows,
                        *layout_axis,
                        *layout_mode,
                        reservation,
                    )?;
                    *clear = true;
                    dbg.log(&native_resize_weight_log_line(
                        "shrink",
                        &panes[*focused].window,
                        panes[*focused].weight,
                    ));
                }
            }
            b'b' | b'B' => {
                balance_native_pane_weights(panes);
                resize_native_panes_for_display_mode(
                    panes,
                    *focused,
                    cols,
                    rows,
                    *layout_axis,
                    *layout_mode,
                    reservation,
                )?;
                *clear = true;
                dbg.log("native terminal balance pane weights");
            }
            b'[' | b',' => {
                if !panes.is_empty() {
                    native_move_focused(
                        panes,
                        focused,
                        *layout_axis,
                        cols,
                        rows,
                        reservation,
                        "left",
                        clear,
                        dbg,
                    )?
                }
            }
            b']' | b'.' => {
                if !panes.is_empty() {
                    native_move_focused(
                        panes,
                        focused,
                        *layout_axis,
                        cols,
                        rows,
                        reservation,
                        "right",
                        clear,
                        dbg,
                    )?
                }
            }
            b'{' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                if !panes.is_empty() {
                    native_move_focused(
                        panes,
                        focused,
                        *layout_axis,
                        cols,
                        rows,
                        reservation,
                        "first",
                        clear,
                        dbg,
                    )?
                }
            }
            b'}' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                if !panes.is_empty() {
                    native_move_focused(
                        panes,
                        focused,
                        *layout_axis,
                        cols,
                        rows,
                        reservation,
                        "last",
                        clear,
                        dbg,
                    )?
                }
            }
            b'w' | b'W' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                native_nudge_focused_pane(floating_offsets, panes, *focused, 0, -1, clear, dbg);
            }
            b'a' | b'A' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                native_nudge_focused_pane(floating_offsets, panes, *focused, -1, 0, clear, dbg);
            }
            b's' | b'S' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                native_nudge_focused_pane(floating_offsets, panes, *focused, 0, 1, clear, dbg);
            }
            b'd' | b'D' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                native_nudge_focused_pane(floating_offsets, panes, *focused, 1, 0, clear, dbg);
            }
            b'r' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                native_reset_focused_pane(floating_offsets, panes, *focused, clear, dbg);
            }
            b'R' if matches!(*layout_mode, NativePaneLayoutMode::Floating) => {
                native_reset_all_floating_offsets_from_keyboard(floating_offsets, clear, dbg);
            }
            0x01 if !panes.is_empty() => {
                native_send_pane_bytes_logged(&mut panes[*focused], "keyboard-byte", &[0x01], dbg);
            }
            other if !panes.is_empty() => {
                native_send_pane_bytes_logged(&mut panes[*focused], "keyboard-byte", &[other], dbg);
            }
            _ => {}
        }
        return Ok(false);
    }
    if byte == 0x01 {
        ctrl_c_exit_guard.reset();
        *prefix = true;
        return Ok(false);
    }
    if byte == 0x03 {
        match native_ctrl_c_action(ctrl_c_exit_guard, Instant::now()) {
            NativeCtrlCAction::Forward => {
                if !panes.is_empty() {
                    native_send_pane_bytes_logged(
                        &mut panes[*focused],
                        "keyboard-ctrl-c",
                        &[byte],
                        dbg,
                    );
                }
                return Ok(false);
            }
            NativeCtrlCAction::Confirm => {
                dbg.log("native terminal loop: triple Ctrl-C opened quit confirmation");
                ctrl_c_exit_guard.reset();
                quit_confirm_overlay.open(Instant::now());
                *clear = true;
                return Ok(false);
            }
        }
    }
    ctrl_c_exit_guard.reset();
    if !panes.is_empty() {
        native_send_pane_bytes_logged(&mut panes[*focused], "keyboard-byte", &[byte], dbg);
    }
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
fn native_launch_terminal_pane(
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    axis: NativePaneLayoutAxis,
    cmd: &str,
    sock: &str,
    cols: u16,
    rows: u16,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    clear: &mut bool,
    dbg: &Debugger,
) -> Result<()> {
    let id = next_native_pane_id(panes);
    let pane = match spawn_native_pane(id, cmd, sock, 1, 1) {
        Ok(pane) => pane,
        Err(err) => {
            dbg.log(&native_spawn_failure_log_line(cmd, &err));
            return Ok(());
        }
    };
    panes.push(pane);
    let new_focus = panes.len() - 1;
    native_set_focus(panes, focused, new_focus)?;
    resize_native_panes_for_layout_with_reservation(panes, cols, rows, axis, reservation)?;
    *clear = true;
    dbg.log(&native_launch_log_line(
        &panes[*focused].window,
        panes.len(),
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn native_split_focused(
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    axis: NativePaneLayoutAxis,
    cmd: &str,
    sock: &str,
    cols: u16,
    rows: u16,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    clear: &mut bool,
    dbg: &Debugger,
) -> Result<()> {
    if panes.len() < 8 {
        let id = next_native_pane_id(panes);
        let pane = match spawn_native_pane(id, cmd, sock, 1, 1) {
            Ok(pane) => pane,
            Err(err) => {
                dbg.log(&native_spawn_failure_log_line(cmd, &err));
                return Ok(());
            }
        };
        panes.push(pane);
        let new_focus = panes.len() - 1;
        native_set_focus(panes, focused, new_focus)?;
        resize_native_panes_for_layout_with_reservation(panes, cols, rows, axis, reservation)?;
        *clear = true;
        dbg.log(&native_split_log_line(axis, panes.len()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn native_move_focused(
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    axis: NativePaneLayoutAxis,
    cols: u16,
    rows: u16,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    direction: &str,
    clear: &mut bool,
    dbg: &Debugger,
) -> Result<()> {
    let from = *focused;
    let to = native_move_target_index(from, panes.len(), direction);
    if to != from {
        let old_focused_window = panes.get(from).map(|pane| pane.window.clone());
        let pane = panes.remove(from);
        panes.insert(to, pane);
        if let Some(old_focused_window) = old_focused_window.as_deref() {
            if let Some(old_focus_idx) =
                native_window_index_after_reorder(panes, old_focused_window)
            {
                *focused = old_focus_idx;
            }
        }
        native_set_focus(panes, focused, to)?;
        resize_native_panes_for_layout_with_reservation(panes, cols, rows, axis, reservation)?;
        *clear = true;
    }
    dbg.log(&native_move_log_line(direction, *focused));
    Ok(())
}

fn native_move_log_line(direction: &str, focused: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("native terminal move  -> ".len() + direction.len() + 20);
    out.push_str("native terminal move ");
    out.push_str(direction);
    out.push_str(" -> ");
    let _ = write!(out, "{focused}");
    out
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

fn native_layouts_for_panes_with_reservation(
    cols: u16,
    rows: u16,
    panes: &[NativePane],
    axis: NativePaneLayoutAxis,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> Vec<NativePaneLayout> {
    native_layouts_for_weights_with_reservation(
        cols,
        rows,
        &panes.iter().map(|pane| pane.weight).collect::<Vec<_>>(),
        axis,
        reservation,
    )
}

fn native_layouts_for_panes_display_mode(
    cols: u16,
    rows: u16,
    panes: &[NativePane],
    focused: usize,
    axis: NativePaneLayoutAxis,
    mode: NativePaneLayoutMode,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> Vec<NativePaneLayout> {
    match mode {
        NativePaneLayoutMode::Tiled => {
            native_layouts_for_panes_with_reservation(cols, rows, panes, axis, reservation)
        }
        NativePaneLayoutMode::Fullscreen => {
            native_fullscreen_layouts_for_panes(cols, rows, panes.len(), focused, reservation)
        }
        NativePaneLayoutMode::Floating => {
            native_floating_layouts_for_panes(cols, rows, panes.len(), reservation)
        }
    }
}

fn native_layouts_for_panes_display_mode_with_offsets(
    cols: u16,
    rows: u16,
    panes: &[NativePane],
    focused: usize,
    axis: NativePaneLayoutAxis,
    mode: NativePaneLayoutMode,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    floating_offsets: &HashMap<String, NativeFloatingPaneOffset>,
) -> Vec<NativePaneLayout> {
    let mut layouts =
        native_layouts_for_panes_display_mode(cols, rows, panes, focused, axis, mode, reservation);
    if matches!(mode, NativePaneLayoutMode::Floating) {
        apply_native_floating_offsets(
            &mut layouts,
            panes,
            cols,
            rows,
            reservation,
            floating_offsets,
        );
    }
    layouts
}

fn apply_native_floating_offsets(
    layouts: &mut [NativePaneLayout],
    panes: &[NativePane],
    cols: u16,
    rows: u16,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    floating_offsets: &HashMap<String, NativeFloatingPaneOffset>,
) {
    for (layout, pane) in layouts.iter_mut().zip(panes.iter()) {
        if let Some(offset) = floating_offsets.get(&pane.window) {
            *layout = native_offset_floating_layout(*layout, cols, rows, reservation, *offset);
        }
    }
}

fn native_offset_floating_layout(
    mut layout: NativePaneLayout,
    cols: u16,
    rows: u16,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    offset: NativeFloatingPaneOffset,
) -> NativePaneLayout {
    if layout.cols == 0 || layout.rows == 0 {
        return layout;
    }
    let reservation = native_chrome_reservation_with_pixel_gaps(reservation);
    let min_x = i32::from(reservation.left_cols.min(cols.saturating_sub(1)));
    let min_y = i32::from(reservation.top_bar_rows.min(rows.saturating_sub(1)));
    let max_x = i32::from(
        cols.saturating_sub(layout.cols)
            .max(reservation.left_cols.min(cols.saturating_sub(1))),
    );
    let max_y = i32::from(
        rows.saturating_sub(layout.rows)
            .max(reservation.top_bar_rows.min(rows.saturating_sub(1))),
    );
    let next_x = (i32::from(layout.x) + i32::from(offset.dx)).clamp(min_x, max_x) as u16;
    let next_y = (i32::from(layout.y) + i32::from(offset.dy)).clamp(min_y, max_y) as u16;
    let dx = next_x.saturating_sub(layout.x);
    let neg_dx = layout.x.saturating_sub(next_x);
    let dy = next_y.saturating_sub(layout.y);
    let neg_dy = layout.y.saturating_sub(next_y);
    layout.x = next_x;
    layout.y = next_y;
    layout.app_x = layout.app_x.saturating_add(dx).saturating_sub(neg_dx);
    layout.app_y = layout.app_y.saturating_add(dy).saturating_sub(neg_dy);
    layout
}

fn native_fullscreen_layouts_for_panes(
    cols: u16,
    rows: u16,
    count: usize,
    focused: usize,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> Vec<NativePaneLayout> {
    if count == 0 {
        return Vec::new();
    }
    let focused = focused.min(count.saturating_sub(1));
    let mut layouts = vec![NativePaneLayout::hidden(); count];
    let full = native_layouts_for_weights_with_reservation(
        cols,
        rows,
        &[1],
        NativePaneLayoutAxis::Columns,
        reservation,
    )
    .into_iter()
    .next()
    .unwrap_or_else(NativePaneLayout::hidden);
    layouts[focused] = full;
    layouts
}

fn native_floating_layouts_for_panes(
    cols: u16,
    rows: u16,
    count: usize,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> Vec<NativePaneLayout> {
    if count == 0 {
        return Vec::new();
    }
    let reservation = native_chrome_reservation_with_pixel_gaps(reservation);
    let left = reservation.left_cols.min(cols.saturating_sub(1));
    let right = reservation
        .right_cols
        .min(cols.saturating_sub(left).saturating_sub(1));
    let content_cols = cols.saturating_sub(left).saturating_sub(right).max(1);
    let content_rows = rows
        .saturating_sub(reservation.top_bar_rows)
        .saturating_sub(reservation.bottom_bar_rows)
        .max(1);
    let pane_cols = content_cols.saturating_mul(4).saturating_div(5).max(1);
    let pane_rows = content_rows
        .saturating_mul(4)
        .saturating_div(5)
        .max(3)
        .min(content_rows);
    let max_dx = content_cols.saturating_sub(pane_cols);
    let max_dy = content_rows.saturating_sub(pane_rows);
    (0..count)
        .map(|idx| {
            let idx = idx.min(u16::MAX as usize) as u16;
            let x = left.saturating_add(idx.saturating_mul(2).min(max_dx));
            let y = reservation.top_bar_rows.saturating_add(idx.min(max_dy));
            NativePaneLayout::from_outer(x, y, pane_cols, pane_rows)
        })
        .collect()
}

fn native_layouts_for_weights_with_reservation(
    cols: u16,
    rows: u16,
    weights: &[u16],
    axis: NativePaneLayoutAxis,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> Vec<NativePaneLayout> {
    if weights.is_empty() {
        return Vec::new();
    }
    let reservation = native_chrome_reservation_with_pixel_gaps(reservation);
    let count = weights.len().min(u16::MAX as usize);
    let left = reservation.left_cols.min(cols.saturating_sub(1));
    let right = reservation
        .right_cols
        .min(cols.saturating_sub(left).saturating_sub(1));
    let content_cols = cols.saturating_sub(left).saturating_sub(right).max(1);
    let tilable_rows = rows
        .saturating_sub(reservation.top_bar_rows)
        .saturating_sub(reservation.bottom_bar_rows)
        .max(1);
    let gap_cols = if matches!(axis, NativePaneLayoutAxis::Columns) {
        reservation.gap_cols
    } else {
        0
    };
    let gap_rows = if matches!(axis, NativePaneLayoutAxis::Rows) {
        reservation.gap_rows
    } else {
        0
    };
    let total_gap_cols = gap_cols.saturating_mul(count.saturating_sub(1) as u16);
    let total_gap_rows = gap_rows.saturating_mul(count.saturating_sub(1) as u16);
    let weighted_cols = content_cols.saturating_sub(total_gap_cols).max(1);
    let weighted_rows = tilable_rows.saturating_sub(total_gap_rows).max(1);
    native_pane_layouts_weighted(weighted_cols, weighted_rows, weights, axis)
        .into_iter()
        .enumerate()
        .map(|(idx, mut layout)| {
            let idx = idx.min(u16::MAX as usize) as u16;
            layout.x = layout
                .x
                .saturating_add(left)
                .saturating_add(idx.saturating_mul(gap_cols));
            layout.app_x = layout
                .app_x
                .saturating_add(left)
                .saturating_add(idx.saturating_mul(gap_cols));
            layout.y = layout
                .y
                .saturating_add(reservation.top_bar_rows)
                .saturating_add(idx.saturating_mul(gap_rows));
            layout.app_y = layout
                .app_y
                .saturating_add(reservation.top_bar_rows)
                .saturating_add(idx.saturating_mul(gap_rows));
            layout
        })
        .collect()
}

#[cfg(test)]
fn reserve_native_top_bar(layouts: Vec<NativePaneLayout>) -> Vec<NativePaneLayout> {
    let reservation = crate::daemon::NativeChromeReservationConfig::default();
    layouts
        .into_iter()
        .map(|mut layout| {
            layout.y = layout.y.saturating_add(reservation.top_bar_rows);
            layout.app_y = layout.app_y.saturating_add(reservation.top_bar_rows);
            layout
        })
        .collect()
}

#[cfg(test)]
fn native_tilable_rows(rows: u16) -> u16 {
    native_tilable_rows_with_reservation(
        rows,
        &crate::daemon::NativeChromeReservationConfig::default(),
    )
}

fn native_tilable_rows_with_reservation(
    rows: u16,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> u16 {
    let reservation = native_chrome_reservation_with_pixel_gaps(reservation);
    rows.saturating_sub(reservation.top_bar_rows)
        .saturating_sub(reservation.bottom_bar_rows)
        .max(1)
}

fn native_weighted_spans(total: u16, weights: &[u16], min_span: u16) -> Vec<u16> {
    let count = weights.len().max(1).min(u16::MAX as usize);
    let weights = if weights.is_empty() {
        NATIVE_DEFAULT_LAYOUT_WEIGHTS.as_slice()
    } else {
        weights
    };
    let effective_min = if u32::from(total) >= u32::from(min_span.max(1)) * count as u32 {
        min_span.max(1)
    } else if total as usize >= count {
        1
    } else {
        0
    };
    let mut spans = Vec::with_capacity(count);
    let mut remaining = total;
    let mut remaining_weight = weights
        .iter()
        .take(count)
        .map(|w| u32::from((*w).max(1)))
        .sum::<u32>()
        .max(1);
    for idx in 0..count {
        let weight = u32::from(weights[idx].max(1));
        let span = if idx + 1 == count {
            remaining
        } else {
            let panes_left = (count - idx - 1) as u16;
            let reserve_for_rest = panes_left.saturating_mul(effective_min);
            let max_span = remaining.saturating_sub(reserve_for_rest);
            let weighted = ((u32::from(remaining) * weight) / remaining_weight) as u16;
            weighted.max(effective_min).min(max_span)
        };
        spans.push(span);
        remaining = remaining.saturating_sub(span);
        remaining_weight = remaining_weight.saturating_sub(weight).max(1);
    }
    spans
}

const NATIVE_DEFAULT_LAYOUT_WEIGHTS: [u16; 1] = [1];

fn native_pane_layouts_weighted(
    cols: u16,
    rows: u16,
    weights: &[u16],
    axis: NativePaneLayoutAxis,
) -> Vec<NativePaneLayout> {
    let count = weights.len().max(1).min(u16::MAX as usize);
    let weights = if weights.is_empty() {
        NATIVE_DEFAULT_LAYOUT_WEIGHTS.as_slice()
    } else {
        weights
    };
    match axis {
        NativePaneLayoutAxis::Columns => {
            let pane_rows = rows;
            let title_rows = NATIVE_PANE_TITLE_ROWS.min(pane_rows);
            let spans = native_weighted_spans(cols, &weights, 1);
            let mut x = 0u16;
            let mut layouts = Vec::with_capacity(count);
            for pane_cols in spans {
                layouts.push(NativePaneLayout {
                    x,
                    y: 0,
                    cols: pane_cols,
                    rows: pane_rows,
                    app_x: x
                        .saturating_add(NATIVE_PANE_BORDER_COLS)
                        .min(x.saturating_add(pane_cols.saturating_sub(1))),
                    app_y: title_rows,
                    app_cols: pane_cols.saturating_sub(NATIVE_PANE_BORDER_COLS * 2),
                    app_rows: pane_rows
                        .saturating_sub(NATIVE_PANE_TITLE_ROWS)
                        .saturating_sub(NATIVE_PANE_BOTTOM_BORDER_ROWS),
                });
                x = x.saturating_add(pane_cols);
            }
            layouts
        }
        NativePaneLayoutAxis::Rows => {
            let min_rows = NATIVE_PANE_TITLE_ROWS + NATIVE_PANE_BOTTOM_BORDER_ROWS + 1;
            let spans = native_weighted_spans(rows, &weights, min_rows);
            let mut y = 0u16;
            let mut layouts = Vec::with_capacity(count);
            for pane_rows in spans {
                let title_rows = NATIVE_PANE_TITLE_ROWS.min(pane_rows);
                layouts.push(NativePaneLayout {
                    x: 0,
                    y,
                    cols,
                    rows: pane_rows,
                    app_x: NATIVE_PANE_BORDER_COLS.min(cols.saturating_sub(1)),
                    app_y: y.saturating_add(title_rows),
                    app_cols: cols.saturating_sub(NATIVE_PANE_BORDER_COLS * 2),
                    app_rows: pane_rows
                        .saturating_sub(NATIVE_PANE_TITLE_ROWS)
                        .saturating_sub(NATIVE_PANE_BOTTOM_BORDER_ROWS),
                });
                y = y.saturating_add(pane_rows);
            }
            layouts
        }
        NativePaneLayoutAxis::Grid => {
            let grid_cols = ((count as f64).sqrt().ceil() as usize).max(1);
            let grid_rows = count.div_ceil(grid_cols).max(1);
            let col_weights = vec![1; grid_cols];
            let row_weights = vec![1; grid_rows];
            let col_spans = native_weighted_spans(cols, &col_weights, 1);
            let row_spans = native_weighted_spans(
                rows,
                &row_weights,
                NATIVE_PANE_TITLE_ROWS + NATIVE_PANE_BOTTOM_BORDER_ROWS + 1,
            );
            let mut layouts = Vec::with_capacity(count);
            let mut y = 0u16;
            for (row_idx, pane_rows) in row_spans.into_iter().enumerate() {
                let mut x = 0u16;
                for pane_cols in col_spans.iter().copied() {
                    if layouts.len() >= count {
                        break;
                    }
                    let title_rows = NATIVE_PANE_TITLE_ROWS.min(pane_rows);
                    layouts.push(NativePaneLayout {
                        x,
                        y,
                        cols: pane_cols,
                        rows: pane_rows,
                        app_x: x
                            .saturating_add(NATIVE_PANE_BORDER_COLS)
                            .min(x.saturating_add(pane_cols.saturating_sub(1))),
                        app_y: y.saturating_add(title_rows),
                        app_cols: pane_cols.saturating_sub(NATIVE_PANE_BORDER_COLS * 2),
                        app_rows: pane_rows
                            .saturating_sub(NATIVE_PANE_TITLE_ROWS)
                            .saturating_sub(NATIVE_PANE_BOTTOM_BORDER_ROWS),
                    });
                    x = x.saturating_add(pane_cols);
                }
                if row_idx + 1 >= grid_rows {
                    break;
                }
                y = y.saturating_add(pane_rows);
            }
            layouts
        }
    }
}

fn native_app_frame_footprint(layout: NativePaneLayout) -> CellRect {
    CellRect::new(layout.app_x, layout.app_y, layout.app_cols, layout.app_rows)
}

fn native_status_outer_rows(layout: NativePaneLayout) -> u16 {
    layout.rows
}

fn should_log_native_capture_failure(capture_failed: &mut bool) -> bool {
    let should_log = !*capture_failed;
    *capture_failed = true;
    should_log
}

fn native_capture_failure_recovered(capture_failed: &mut bool) {
    *capture_failed = false;
}

fn native_capture_failure_log_line(
    window: &str,
    layout: NativePaneLayout,
    err: &dyn std::fmt::Display,
) -> String {
    use std::fmt::Write as _;

    let err = bounded_ellipsis(&err.to_string(), 160);
    let mut out = String::with_capacity(
        "native pane capture failed: window= app=x layout=x+, err=".len()
            + window.len()
            + err.len()
            + 32,
    );
    out.push_str("native pane capture failed: window=");
    out.push_str(window);
    out.push_str(" app=");
    let _ = write!(
        out,
        "{}x{} layout={}x{}+{},{} err=",
        layout.app_cols, layout.app_rows, layout.cols, layout.rows, layout.x, layout.y
    );
    out.push_str(&err);
    out
}

fn native_send_pane_bytes_logged(
    pane: &mut NativePane,
    operation: &str,
    bytes: &[u8],
    dbg: &Debugger,
) -> bool {
    match pane.app.send_bytes(bytes) {
        Ok(()) => true,
        Err(err) => {
            dbg.log(&native_socket_input_failure_log_line(
                &pane.window,
                operation,
                bytes.len(),
                &err,
            ));
            false
        }
    }
}

fn native_socket_input_failure_log_line(
    window: &str,
    operation: &str,
    bytes: usize,
    err: &dyn std::fmt::Display,
) -> String {
    use std::fmt::Write as _;

    let err = bounded_ellipsis(&err.to_string(), 160);
    let mut out = String::with_capacity(
        "native pane socket input failed: window= operation= bytes= err=".len()
            + window.len()
            + operation.len()
            + err.len()
            + 20,
    );
    out.push_str("native pane socket input failed: window=");
    out.push_str(window);
    out.push_str(" operation=");
    out.push_str(operation);
    out.push_str(" bytes=");
    let _ = write!(out, "{bytes}");
    out.push_str(" err=");
    out.push_str(&err);
    out
}

fn native_resize_failure_log_line(
    window: &str,
    layout: NativePaneLayout,
    err: &dyn std::fmt::Display,
) -> String {
    use std::fmt::Write as _;

    let err = err.to_string();
    let mut out = String::with_capacity(
        "native pane resize failed: window= app=x layout=x+, err=".len()
            + window.len()
            + err.len()
            + 32,
    );
    out.push_str("native pane resize failed: window=");
    out.push_str(window);
    out.push_str(" app=");
    let _ = write!(
        out,
        "{}x{} layout={}x{}+{},{} err=",
        layout.app_cols.max(1),
        layout.app_rows.max(1),
        layout.cols,
        layout.rows,
        layout.x,
        layout.y
    );
    out.push_str(&err);
    out
}

fn resize_native_panes_logged(
    panes: &mut [NativePane],
    layouts: Vec<NativePaneLayout>,
    dbg: Option<&Debugger>,
) -> Result<usize> {
    let mut failures = 0usize;
    for (pane, layout) in panes.iter_mut().zip(layouts) {
        if let Err(err) = NativeSurface::resize_surface(
            &mut pane.app,
            layout.app_cols.max(1),
            layout.app_rows.max(1),
        ) {
            failures = failures.saturating_add(1);
            if let Some(dbg) = dbg {
                dbg.log(&native_resize_failure_log_line(&pane.window, layout, &err));
            }
        }
    }
    Ok(failures)
}

fn resize_native_panes(panes: &mut [NativePane], layouts: Vec<NativePaneLayout>) -> Result<()> {
    resize_native_panes_logged(panes, layouts, None).map(|_| ())
}

fn should_log_resize_failures(failures: usize) -> bool {
    failures > 0
}

fn resize_native_panes_for_display_mode(
    panes: &mut [NativePane],
    focused: usize,
    cols: u16,
    rows: u16,
    axis: NativePaneLayoutAxis,
    mode: NativePaneLayoutMode,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> Result<()> {
    let layouts =
        native_layouts_for_panes_display_mode(cols, rows, panes, focused, axis, mode, reservation);
    resize_native_panes_logged(panes, layouts, None).map(|_| ())
}

fn resize_native_panes_for_layout_with_reservation(
    panes: &mut [NativePane],
    cols: u16,
    rows: u16,
    axis: NativePaneLayoutAxis,
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> Result<()> {
    let layouts = native_layouts_for_panes_with_reservation(cols, rows, panes, axis, reservation);
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

fn native_spawn_failure_log_line(command: &str, err: &dyn std::fmt::Display) -> String {
    let command = bounded_ellipsis(command, 120);
    let err = bounded_ellipsis(&err.to_string(), 160);
    let mut out = String::with_capacity(
        "native pane spawn failed: command= err=".len() + command.len() + err.len(),
    );
    out.push_str("native pane spawn failed: command=");
    out.push_str(&command);
    out.push_str(" err=");
    out.push_str(&err);
    out
}

fn native_terminate_failure_log_line(window: &str, err: &dyn std::fmt::Display) -> String {
    let err = bounded_ellipsis(&err.to_string(), 160);
    let mut out = String::with_capacity(
        "native pane terminate failed: window= err=".len() + window.len() + err.len(),
    );
    out.push_str("native pane terminate failed: window=");
    out.push_str(window);
    out.push_str(" err=");
    out.push_str(&err);
    out
}

fn native_terminate_pane_logged(pane: &mut NativePane, dbg: &Debugger) -> bool {
    match pane.app.terminate() {
        Ok(()) => true,
        Err(err) => {
            dbg.log(&native_terminate_failure_log_line(&pane.window, &err));
            false
        }
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

fn native_restore_focus_target(count: usize, focus_index: Option<usize>) -> Option<usize> {
    (count > 0).then(|| native_restore_focus_index(count, focus_index))
}

fn should_focus_restored_pane(count: usize, focused: usize) -> bool {
    count > 0 && focused < count
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

fn native_window_index_after_reorder(panes: &[NativePane], window: &str) -> Option<usize> {
    panes.iter().position(|pane| pane.window == window)
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

fn native_route_mouse_event(
    event: &InputEvent,
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    cols: u16,
    rows: u16,
    axis: NativePaneLayoutAxis,
    mode: NativePaneLayoutMode,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    floating_drag: &mut Option<NativeFloatingDrag>,
    tiled_drag: &mut Option<NativeTiledDrag>,
    clear: &mut bool,
) -> Result<bool> {
    let Some((event_name, col, row, should_focus)) = native_mouse_event_name_and_position(&event)
    else {
        return Ok(false);
    };
    if native_update_floating_drag(event_name, col, row, floating_offsets, floating_drag) {
        *clear = true;
        return Ok(true);
    }
    let layouts = native_layouts_for_panes_display_mode_with_offsets(
        cols,
        rows,
        panes,
        *focused,
        axis,
        mode,
        reservation,
        floating_offsets,
    );
    let topmost_first = matches!(mode, NativePaneLayoutMode::Floating);
    if native_update_tiled_drag(
        event_name,
        col,
        row,
        panes,
        focused,
        cols,
        rows,
        axis,
        reservation,
        &layouts,
        tiled_drag,
        clear,
    )? {
        return Ok(true);
    }
    let Some((idx, local_col, local_row)) =
        native_pane_at_host_cell_ordered(&layouts, col, row, topmost_first)
    else {
        if should_focus {
            if let Some(idx) =
                native_pane_chrome_at_host_cell_ordered(&layouts, col, row, topmost_first)
            {
                let title_hit =
                    native_pane_title_at_host_cell_ordered(&layouts, col, row, topmost_first)
                        == Some(idx);
                let target = native_focus_or_raise_for_mouse(panes, focused, idx, mode)?;
                if event_name == "press-left" && title_hit {
                    if matches!(mode, NativePaneLayoutMode::Floating) {
                        if let Some(pane) = panes.get(target) {
                            *floating_drag = Some(NativeFloatingDrag {
                                window: pane.window.clone(),
                                last_col: col,
                                last_row: row,
                            });
                        }
                    } else if matches!(mode, NativePaneLayoutMode::Tiled) {
                        if let Some(pane) = panes.get(target) {
                            *tiled_drag = Some(NativeTiledDrag {
                                window: pane.window.clone(),
                            });
                        }
                    }
                }
                *clear = true;
            }
        }
        return Ok(true);
    };
    let mut target_idx = idx;
    if should_focus {
        target_idx = native_focus_or_raise_for_mouse(panes, focused, idx, mode)?;
        *clear = true;
    }
    if panes[target_idx].app.supports_direct_pointer() {
        for event in native_surface_pointer_events(event_name, local_col, local_row) {
            NativeSurface::send_surface_pointer(&mut panes[target_idx].app, event)?;
        }
        return Ok(true);
    }
    let modes = panes[target_idx].app.mouse_reporting_modes();
    if let Some(payload) = native_mouse_event_payload(event_name, local_col, local_row, modes) {
        panes[target_idx].app.send_bytes(&payload)?;
    }
    Ok(true)
}

fn native_nudge_floating_offset(
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    window: &str,
    dx: i16,
    dy: i16,
) {
    let offset = floating_offsets.entry(window.to_string()).or_default();
    offset.dx = native_saturating_i16_add_i32(offset.dx, i32::from(dx));
    offset.dy = native_saturating_i16_add_i32(offset.dy, i32::from(dy));
}

fn native_reset_floating_offset(
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    window: &str,
) {
    floating_offsets.remove(window);
}

fn native_reset_all_floating_offsets(
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
) -> usize {
    let reset_count = floating_offsets.len();
    floating_offsets.clear();
    reset_count
}

fn native_nudge_focused_pane(
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    panes: &[NativePane],
    focused: usize,
    dx: i16,
    dy: i16,
    clear: &mut bool,
    dbg: &Debugger,
) {
    let Some(pane) = panes.get(focused) else {
        return;
    };
    native_nudge_floating_offset(floating_offsets, &pane.window, dx, dy);
    *clear = true;
    dbg.log(&native_keyboard_nudge_log_line(&pane.window, dx, dy));
}

fn native_reset_focused_pane(
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    panes: &[NativePane],
    focused: usize,
    clear: &mut bool,
    dbg: &Debugger,
) {
    let Some(pane) = panes.get(focused) else {
        return;
    };
    native_reset_floating_offset(floating_offsets, &pane.window);
    *clear = true;
    dbg.log(&native_keyboard_reset_offset_log_line(&pane.window));
}

fn native_reset_all_floating_offsets_from_keyboard(
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    clear: &mut bool,
    dbg: &Debugger,
) {
    let reset_count = native_reset_all_floating_offsets(floating_offsets);
    if reset_count > 0 {
        *clear = true;
    }
    dbg.log(&native_keyboard_reset_all_offsets_log_line(reset_count));
}

fn native_update_floating_drag(
    event_name: &str,
    col: u16,
    row: u16,
    floating_offsets: &mut HashMap<String, NativeFloatingPaneOffset>,
    floating_drag: &mut Option<NativeFloatingDrag>,
) -> bool {
    match event_name {
        "move-left" => {
            let Some(drag) = floating_drag.as_mut() else {
                return false;
            };
            let delta_col = col as i32 - drag.last_col as i32;
            let delta_row = row as i32 - drag.last_row as i32;
            drag.last_col = col;
            drag.last_row = row;
            if delta_col == 0 && delta_row == 0 {
                return true;
            }
            let offset = floating_offsets.entry(drag.window.clone()).or_default();
            offset.dx = native_saturating_i16_add_i32(offset.dx, delta_col);
            offset.dy = native_saturating_i16_add_i32(offset.dy, delta_row);
            true
        }
        "release-left" | "release" => floating_drag.take().is_some(),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn native_update_tiled_drag(
    event_name: &str,
    col: u16,
    row: u16,
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    cols: u16,
    rows: u16,
    axis: NativePaneLayoutAxis,
    reservation: &crate::daemon::NativeChromeReservationConfig,
    layouts: &[NativePaneLayout],
    tiled_drag: &mut Option<NativeTiledDrag>,
    clear: &mut bool,
) -> Result<bool> {
    match event_name {
        "release-left" | "release" => {
            let Some(drag) = tiled_drag.take() else {
                return Ok(false);
            };
            let Some(from) = native_window_index_after_reorder(panes, &drag.window) else {
                return Ok(true);
            };
            let target = native_pane_at_host_cell_ordered(layouts, col, row, false)
                .map(|(idx, _, _)| idx)
                .or_else(|| native_pane_chrome_at_host_cell_ordered(layouts, col, row, false));
            let Some(to) = target else {
                return Ok(true);
            };
            if to != from {
                let pane = panes.remove(from);
                let to = to.min(panes.len());
                panes.insert(to, pane);
                if let Some(new_focus) = native_window_index_after_reorder(panes, &drag.window) {
                    native_set_focus(panes, focused, new_focus)?;
                }
                resize_native_panes_for_layout_with_reservation(
                    panes,
                    cols,
                    rows,
                    axis,
                    reservation,
                )?;
                *clear = true;
            }
            Ok(true)
        }
        _ => Ok(tiled_drag.is_some()),
    }
}

fn native_saturating_i16_add_i32(value: i16, delta: i32) -> i16 {
    (i32::from(value) + delta).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn native_mouse_event_name_and_position(
    event: &InputEvent,
) -> Option<(&'static str, u16, u16, bool)> {
    match event {
        InputEvent::MousePress {
            button, col, row, ..
        } => match button {
            MouseButton::Left => Some(("press-left", *col, *row, true)),
            MouseButton::Middle => Some(("press-middle", *col, *row, true)),
            MouseButton::Right => Some(("press-right", *col, *row, true)),
            MouseButton::ScrollUp => Some(("scroll-up", *col, *row, false)),
            MouseButton::ScrollDown => Some(("scroll-down", *col, *row, false)),
            _ => None,
        },
        InputEvent::MouseRelease {
            button, col, row, ..
        } => match button {
            MouseButton::Left => Some(("release-left", *col, *row, false)),
            MouseButton::Middle => Some(("release-middle", *col, *row, false)),
            MouseButton::Right => Some(("release-right", *col, *row, false)),
            _ => Some(("release", *col, *row, false)),
        },
        InputEvent::MouseMove {
            button, col, row, ..
        } => match button {
            MouseButton::Left => Some(("move-left", *col, *row, false)),
            MouseButton::Middle => Some(("move-middle", *col, *row, false)),
            MouseButton::Right => Some(("move-right", *col, *row, false)),
            MouseButton::None => Some(("move", *col, *row, false)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
fn native_pane_chrome_at_host_cell(
    layouts: &[NativePaneLayout],
    host_col: u16,
    host_row: u16,
) -> Option<usize> {
    native_pane_chrome_at_host_cell_ordered(layouts, host_col, host_row, false)
}

fn native_pane_chrome_at_host_cell_ordered(
    layouts: &[NativePaneLayout],
    host_col: u16,
    host_row: u16,
    topmost_first: bool,
) -> Option<usize> {
    let col0 = host_col.checked_sub(1)?;
    let row0 = host_row.checked_sub(1)?;
    let hit = |layout: &NativePaneLayout| {
        let within_cols = col0 >= layout.x && col0 < layout.x.saturating_add(layout.cols);
        let within_rows = row0 >= layout.y && row0 < layout.y.saturating_add(layout.rows);
        within_cols && within_rows
    };
    if topmost_first {
        (0..layouts.len()).rev().find(|idx| hit(&layouts[*idx]))
    } else {
        layouts.iter().position(hit)
    }
}

fn native_pane_title_at_host_cell_ordered(
    layouts: &[NativePaneLayout],
    host_col: u16,
    host_row: u16,
    topmost_first: bool,
) -> Option<usize> {
    let col0 = host_col.checked_sub(1)?;
    let row0 = host_row.checked_sub(1)?;
    let hit = |layout: &NativePaneLayout| {
        let within_cols = col0 >= layout.x && col0 < layout.x.saturating_add(layout.cols);
        within_cols && row0 == layout.y
    };
    if topmost_first {
        (0..layouts.len()).rev().find(|idx| hit(&layouts[*idx]))
    } else {
        layouts.iter().position(hit)
    }
}

#[cfg(test)]
fn native_pane_at_host_cell(
    layouts: &[NativePaneLayout],
    host_col: u16,
    host_row: u16,
) -> Option<(usize, u16, u16)> {
    native_pane_at_host_cell_ordered(layouts, host_col, host_row, false)
}

fn native_pane_at_host_cell_ordered(
    layouts: &[NativePaneLayout],
    host_col: u16,
    host_row: u16,
    topmost_first: bool,
) -> Option<(usize, u16, u16)> {
    let col0 = host_col.checked_sub(1)?;
    let row0 = host_row.checked_sub(1)?;
    let hit = |idx: usize| {
        let layout = layouts.get(idx)?;
        let within_cols =
            col0 >= layout.app_x && col0 < layout.app_x.saturating_add(layout.app_cols);
        let within_rows =
            row0 >= layout.app_y && row0 < layout.app_y.saturating_add(layout.app_rows);
        within_cols.then_some(())?;
        within_rows.then_some(())?;
        Some((idx, col0 - layout.app_x + 1, row0 - layout.app_y + 1))
    };
    if topmost_first {
        (0..layouts.len()).rev().find_map(hit)
    } else {
        (0..layouts.len()).find_map(hit)
    }
}

fn native_surface_pointer_events(event: &str, col: u16, row: u16) -> Vec<SurfacePointerEvent> {
    let x_px = i32::from(col.saturating_sub(1)) * 8;
    let y_px = i32::from(row.saturating_sub(1)) * 16;
    let mut events = vec![SurfacePointerEvent::Move { x_px, y_px }];
    match event {
        "press-left" => events.push(SurfacePointerEvent::Press {
            button: SurfacePointerButton::Left,
        }),
        "press-middle" => events.push(SurfacePointerEvent::Press {
            button: SurfacePointerButton::Middle,
        }),
        "press-right" => events.push(SurfacePointerEvent::Press {
            button: SurfacePointerButton::Right,
        }),
        "release-left" => events.push(SurfacePointerEvent::Release {
            button: SurfacePointerButton::Left,
        }),
        "release-middle" => events.push(SurfacePointerEvent::Release {
            button: SurfacePointerButton::Middle,
        }),
        "release-right" => events.push(SurfacePointerEvent::Release {
            button: SurfacePointerButton::Right,
        }),
        "release" => events.push(SurfacePointerEvent::Release {
            button: SurfacePointerButton::Left,
        }),
        "scroll-up" => events.push(SurfacePointerEvent::Press {
            button: SurfacePointerButton::ScrollUp,
        }),
        "scroll-down" => events.push(SurfacePointerEvent::Press {
            button: SurfacePointerButton::ScrollDown,
        }),
        "move" | "move-left" | "move-middle" | "move-right" => {}
        _ => events.clear(),
    }
    events
}

fn native_mouse_event_payload(
    event: &str,
    col: u16,
    row: u16,
    modes: MouseReportingModes,
) -> Option<Vec<u8>> {
    if col == 0 || row == 0 {
        return None;
    }
    let click_capable = modes.basic || modes.button_motion || modes.all_motion;
    let (bits, suffix) = match event {
        "press-left" if click_capable => (0, 'M'),
        "press-middle" if click_capable => (1, 'M'),
        "press-right" if click_capable => (2, 'M'),
        "release-left" if click_capable && modes.sgr => (0, 'm'),
        "release-middle" if click_capable && modes.sgr => (1, 'm'),
        "release-right" if click_capable && modes.sgr => (2, 'm'),
        "release" | "release-left" | "release-middle" | "release-right" if click_capable => {
            (3, 'm')
        }
        "move" if modes.all_motion => (35, 'M'),
        "move-left" if modes.button_motion || modes.all_motion => (32, 'M'),
        "move-middle" if modes.button_motion || modes.all_motion => (33, 'M'),
        "move-right" if modes.button_motion || modes.all_motion => (34, 'M'),
        "scroll-up" if click_capable => (64, 'M'),
        "scroll-down" if click_capable => (65, 'M'),
        _ => return None,
    };
    if modes.sgr {
        return Some(native_sgr_mouse_payload(bits, col, row, suffix));
    }
    native_legacy_mouse_payload(bits, col, row)
}

fn push_u16_decimal(out: &mut Vec<u8>, mut value: u16) {
    if value == 0 {
        out.push(b'0');
        return;
    }
    let mut digits = [0u8; 5];
    let mut len = 0usize;
    while value > 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    for digit in digits[..len].iter().rev() {
        out.push(*digit);
    }
}

fn native_sgr_mouse_payload(bits: u16, col: u16, row: u16, suffix: char) -> Vec<u8> {
    let mut out = Vec::with_capacity("\x1b[<;;".len() + 5 + 5 + 5 + 1);
    out.extend_from_slice(b"\x1b[<");
    push_u16_decimal(&mut out, bits);
    out.push(b';');
    push_u16_decimal(&mut out, col);
    out.push(b';');
    push_u16_decimal(&mut out, row);
    out.push(suffix as u8);
    out
}

fn native_legacy_mouse_payload(bits: u16, col: u16, row: u16) -> Option<Vec<u8>> {
    if bits > 223 || col == 0 || col > 223 || row == 0 || row > 223 {
        return None;
    }
    let mut out = Vec::with_capacity(6);
    out.extend_from_slice(b"\x1b[M");
    out.push((bits + 32) as u8);
    out.push((col + 32) as u8);
    out.push((row + 32) as u8);
    Some(out)
}

fn native_key_event_payload(
    event: &InputEvent,
    application_cursor_keys: bool,
) -> Option<&'static [u8]> {
    let InputEvent::Key { key, mods } = event else {
        return None;
    };
    if mods.shift || mods.alt || mods.ctrl {
        return None;
    }
    match (key, application_cursor_keys) {
        (Key::Up, true) => Some(b"\x1bOA"),
        (Key::Down, true) => Some(b"\x1bOB"),
        (Key::Right, true) => Some(b"\x1bOC"),
        (Key::Left, true) => Some(b"\x1bOD"),
        (Key::Up, false) => Some(b"\x1b[A"),
        (Key::Down, false) => Some(b"\x1b[B"),
        (Key::Right, false) => Some(b"\x1b[C"),
        (Key::Left, false) => Some(b"\x1b[D"),
        _ => None,
    }
}

fn native_focus_event_payload(focus_reporting: bool, focused: bool) -> Option<&'static [u8]> {
    if !focus_reporting {
        return None;
    }
    Some(if focused { b"\x1b[I" } else { b"\x1b[O" })
}

fn native_send_focus_event(pane: &mut NativePane, focused: bool) -> bool {
    let Some(payload) = native_focus_event_payload(pane.app.focus_reporting_enabled(), focused)
    else {
        return true;
    };
    pane.app.send_bytes(payload).is_ok()
}

fn native_focus_or_raise_for_mouse(
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    target: usize,
    mode: NativePaneLayoutMode,
) -> Result<usize> {
    if matches!(mode, NativePaneLayoutMode::Floating) {
        return native_raise_and_focus_pane(panes, focused, target);
    }
    native_set_focus(panes, focused, target)?;
    Ok(target.min(panes.len().saturating_sub(1)))
}

fn native_raise_and_focus_pane(
    panes: &mut Vec<NativePane>,
    focused: &mut usize,
    target: usize,
) -> Result<usize> {
    if panes.is_empty() {
        *focused = 0;
        return Ok(0);
    }
    let target = target.min(panes.len().saturating_sub(1));
    if target == panes.len().saturating_sub(1) {
        native_set_focus(panes, focused, target)?;
        return Ok(target);
    }
    let old_focus = (*focused).min(panes.len().saturating_sub(1));
    if old_focus != target {
        let _ = native_send_focus_event(&mut panes[old_focus], false);
    }
    let pane = panes.remove(target);
    panes.push(pane);
    let new_focus = panes.len().saturating_sub(1);
    if old_focus != target {
        let _ = native_send_focus_event(&mut panes[new_focus], true);
    }
    *focused = new_focus;
    Ok(new_focus)
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
        let _ = native_send_focus_event(&mut panes[*focused], false);
    }
    let _ = native_send_focus_event(&mut panes[new_focus], true);
    *focused = new_focus;
    Ok(true)
}

fn native_pane_statuses(
    panes: &[NativePane],
    focused: usize,
    layouts: &[NativePaneLayout],
    floating_offsets: &HashMap<String, NativeFloatingPaneOffset>,
    mode: NativePaneLayoutMode,
    active_drag_label: Option<NativeActiveDragLabel<'_>>,
) -> Vec<crate::daemon::NativePaneStatus> {
    panes
        .iter()
        .enumerate()
        .map(|(idx, pane)| {
            let layout = layouts.get(idx).copied();
            let offset = floating_offsets
                .get(&pane.window)
                .copied()
                .unwrap_or_default();
            let (title_drag_col, title_drag_row) =
                native_status_title_drag_cell(layout, mode).unwrap_or((None, None));
            let title_drag_active =
                active_drag_label.is_some_and(|drag| drag.window == pane.window);
            let (cursor_col, cursor_row) = pane.app.cursor_position();
            let mouse = pane.app.mouse_reporting_modes();
            let title_draggable = native_status_title_draggable(mode);
            let title_drag_kind = native_status_title_drag_kind(mode).map(str::to_string);
            crate::daemon::NativePaneStatus {
                window: pane.window.clone(),
                title: native_pane_display_title(pane),
                focused: idx == focused,
                weight: pane.weight,
                stack_index: Some(idx.min(u16::MAX as usize) as u16),
                stack_top: Some(idx + 1 == panes.len()),
                floating_dx: Some(offset.dx),
                floating_dy: Some(offset.dy),
                floating_moved: Some(offset != NativeFloatingPaneOffset::default()),
                title_draggable: Some(title_draggable),
                title_drag_kind,
                title_drag_col,
                title_drag_row,
                title_drag_active: Some(title_drag_active),
                pid: pane.pid,
                command: Some(pane.command.clone()),
                x: layout.map(|l| l.x),
                y: layout.map(|l| l.y),
                cols: layout.map(|l| l.cols),
                rows: layout.map(native_status_outer_rows),
                app_x: layout.map(|l| l.app_x),
                app_y: layout.map(|l| l.app_y),
                app_cols: layout.map(|l| l.app_cols),
                cursor_col: Some(cursor_col),
                cursor_row: Some(cursor_row),
                cursor_visible: Some(pane.app.cursor_visible()),
                bracketed_paste: Some(pane.app.bracketed_paste_enabled()),
                application_cursor_keys: Some(pane.app.application_cursor_keys_enabled()),
                mouse_reporting: Some(mouse.basic),
                mouse_button_motion: Some(mouse.button_motion),
                mouse_all_motion: Some(mouse.all_motion),
                mouse_sgr: Some(mouse.sgr),
                dirty_frame: pane.dirty_frame.as_ref().map(|metrics| {
                    crate::daemon::NativeDirtyFrameStatus {
                        changed_tiles: metrics.changed_tiles,
                        total_tiles: metrics.total_tiles,
                        changed_fraction: metrics.changed_fraction,
                        skipped_upload: metrics.skipped_upload,
                    }
                }),
                text_snapshot: Some(pane.app.text_snapshot()),
                scrollback_snapshot: Some(pane.app.scrollback_snapshot()),
                app_rows: layout.map(|l| l.app_rows),
            }
        })
        .collect()
}

fn native_status_title_draggable(mode: NativePaneLayoutMode) -> bool {
    matches!(
        mode,
        NativePaneLayoutMode::Tiled | NativePaneLayoutMode::Floating
    )
}

fn native_status_title_drag_kind(mode: NativePaneLayoutMode) -> Option<&'static str> {
    match mode {
        NativePaneLayoutMode::Tiled => Some("reorder"),
        NativePaneLayoutMode::Floating => Some("reposition"),
        NativePaneLayoutMode::Fullscreen => None,
    }
}

fn native_status_title_drag_cell(
    layout: Option<NativePaneLayout>,
    mode: NativePaneLayoutMode,
) -> Option<(Option<u16>, Option<u16>)> {
    if !native_status_title_draggable(mode) {
        return Some((None, None));
    }
    let layout = layout?;
    if layout.cols == 0 || layout.rows == 0 {
        return Some((None, None));
    }
    let local_col = if matches!(mode, NativePaneLayoutMode::Floating) {
        layout.cols.saturating_sub(1).min(3)
    } else {
        layout.cols.saturating_sub(1).min(1)
    };
    Some((
        Some(layout.x.saturating_add(local_col).saturating_add(1)),
        Some(layout.y.saturating_add(1)),
    ))
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

fn native_socket_spawn_log_line(command: &str) -> String {
    let mut out = String::with_capacity("native terminal socket spawn: ".len() + command.len());
    out.push_str("native terminal socket spawn: ");
    out.push_str(command);
    out
}

fn native_socket_focus_log_line(window: &str) -> String {
    let mut out = String::with_capacity("native terminal socket focus: ".len() + window.len());
    out.push_str("native terminal socket focus: ");
    out.push_str(window);
    out
}

fn native_socket_focus_next_log_line(window: &str) -> String {
    let mut out = String::with_capacity("native terminal socket focus next: ".len() + window.len());
    out.push_str("native terminal socket focus next: ");
    out.push_str(window);
    out
}

fn native_socket_focus_prev_log_line(window: &str) -> String {
    let mut out = String::with_capacity("native terminal socket focus prev: ".len() + window.len());
    out.push_str("native terminal socket focus prev: ");
    out.push_str(window);
    out
}

fn native_keyboard_focus_prev_log_line(window: &str) -> String {
    let mut out =
        String::with_capacity("native terminal keyboard focus prev: ".len() + window.len());
    out.push_str("native terminal keyboard focus prev: ");
    out.push_str(window);
    out
}

fn native_socket_close_log_line(window: &str) -> String {
    let mut out = String::with_capacity("native terminal socket close: ".len() + window.len());
    out.push_str("native terminal socket close: ");
    out.push_str(window);
    out
}

fn native_socket_layout_log_line(axis: &str) -> String {
    let mut out = String::with_capacity("native terminal socket layout: ".len() + axis.len());
    out.push_str("native terminal socket layout: ");
    out.push_str(axis);
    out
}

fn native_socket_rename_log_line(window: &str, title: &str) -> String {
    let mut out = String::with_capacity(
        "native terminal socket rename:  -> ".len() + window.len() + title.len(),
    );
    out.push_str("native terminal socket rename: ");
    out.push_str(window);
    out.push_str(" -> ");
    out.push_str(title);
    out
}

fn native_socket_split_log_line(window: &str, axis: &str, command: &str) -> String {
    let mut out = String::with_capacity(
        "native terminal socket split: target= axis= command=".len()
            + window.len()
            + axis.len()
            + command.len(),
    );
    out.push_str("native terminal socket split: target=");
    out.push_str(window);
    out.push_str(" axis=");
    out.push_str(axis);
    out.push_str(" command=");
    out.push_str(command);
    out
}

fn native_socket_move_log_line(window: &str, direction: &str, to: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket move:   -> ".len() + window.len() + direction.len() + 20,
    );
    out.push_str("native terminal socket move: ");
    out.push_str(window);
    out.push(' ');
    out.push_str(direction);
    out.push_str(" -> ");
    let _ = write!(out, "{to}");
    out
}

fn native_socket_nudge_log_line(window: &str, dx: i16, dy: i16) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("native terminal socket nudge:  dx= dy=".len() + window.len() + 16);
    out.push_str("native terminal socket nudge: ");
    out.push_str(window);
    out.push_str(" dx=");
    let _ = write!(out, "{dx}");
    out.push_str(" dy=");
    let _ = write!(out, "{dy}");
    out
}

fn native_keyboard_nudge_log_line(window: &str, dx: i16, dy: i16) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("native terminal keyboard nudge:  dx= dy=".len() + window.len() + 16);
    out.push_str("native terminal keyboard nudge: ");
    out.push_str(window);
    out.push_str(" dx=");
    let _ = write!(out, "{dx}");
    out.push_str(" dy=");
    let _ = write!(out, "{dy}");
    out
}

fn native_socket_reset_offset_log_line(window: &str) -> String {
    let mut out =
        String::with_capacity("native terminal socket reset offset: ".len() + window.len());
    out.push_str("native terminal socket reset offset: ");
    out.push_str(window);
    out
}

fn native_socket_reset_all_offsets_log_line(reset_count: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("native terminal socket reset all offsets: ".len() + 20);
    out.push_str("native terminal socket reset all offsets: ");
    let _ = write!(out, "{reset_count}");
    out
}

fn native_keyboard_reset_offset_log_line(window: &str) -> String {
    let mut out =
        String::with_capacity("native terminal keyboard reset offset: ".len() + window.len());
    out.push_str("native terminal keyboard reset offset: ");
    out.push_str(window);
    out
}

fn native_keyboard_reset_all_offsets_log_line(reset_count: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("native terminal keyboard reset all offsets: ".len() + 20);
    out.push_str("native terminal keyboard reset all offsets: ");
    let _ = write!(out, "{reset_count}");
    out
}

fn native_socket_resize_log_line(window: &str, delta: i16, weight: u16) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket resize:  delta= weight=".len() + window.len() + 12,
    );
    out.push_str("native terminal socket resize: ");
    out.push_str(window);
    out.push_str(" delta=");
    let _ = write!(out, "{delta}");
    out.push_str(" weight=");
    let _ = write!(out, "{weight}");
    out
}

fn native_socket_restore_session_log_line(panes: usize, focused: usize) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("native terminal socket restore session: panes= focus=".len() + 40);
    out.push_str("native terminal socket restore session: panes=");
    let _ = write!(out, "{panes}");
    out.push_str(" focus=");
    let _ = write!(out, "{focused}");
    out
}

fn native_socket_restore_session_failure_log_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("native terminal socket restore session failed: ".len() + 80);
    out.push_str("native terminal socket restore session failed: ");
    let _ = write!(out, "{err}");
    out
}

fn native_socket_send_text_log_line(window: &str, bytes: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket send text:  bytes=".len() + window.len() + 20,
    );
    out.push_str("native terminal socket send text: ");
    out.push_str(window);
    out.push_str(" bytes=");
    let _ = write!(out, "{bytes}");
    out
}

fn native_socket_send_key_log_line(window: &str, key: &str, bytes: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket send key:  key= bytes=".len() + window.len() + key.len() + 20,
    );
    out.push_str("native terminal socket send key: ");
    out.push_str(window);
    out.push_str(" key=");
    out.push_str(key);
    out.push_str(" bytes=");
    let _ = write!(out, "{bytes}");
    out
}

fn native_socket_send_browser_key_log_line(window: &str, key: &str) -> String {
    let mut out = String::with_capacity(
        "native terminal socket send browser key:  key=".len() + window.len() + key.len(),
    );
    out.push_str("native terminal socket send browser key: ");
    out.push_str(window);
    out.push_str(" key=");
    out.push_str(key);
    out
}

fn native_socket_paste_bytes_log_line(window: &str, bytes: usize, bracketed: bool) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket paste bytes:  bytes= bracketed=".len() + window.len() + 25,
    );
    out.push_str("native terminal socket paste bytes: ");
    out.push_str(window);
    out.push_str(" bytes=");
    let _ = write!(out, "{bytes}");
    out.push_str(" bracketed=");
    out.push_str(if bracketed { "true" } else { "false" });
    out
}

fn native_socket_send_mouse_log_line(
    window: &str,
    event: &str,
    col: u16,
    row: u16,
    bytes: usize,
) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket send mouse:  event= col= row= bytes=".len()
            + window.len()
            + event.len()
            + 40,
    );
    out.push_str("native terminal socket send mouse: ");
    out.push_str(window);
    out.push_str(" event=");
    out.push_str(event);
    out.push_str(" col=");
    let _ = write!(out, "{col}");
    out.push_str(" row=");
    let _ = write!(out, "{row}");
    out.push_str(" bytes=");
    let _ = write!(out, "{bytes}");
    out
}

fn native_socket_send_mouse_direct_log_line(
    window: &str,
    event: &str,
    col: u16,
    row: u16,
    sent: usize,
    total: usize,
) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket send mouse direct:  event= col= row= events=/".len()
            + window.len()
            + event.len()
            + 60,
    );
    out.push_str("native terminal socket send mouse direct: ");
    out.push_str(window);
    out.push_str(" event=");
    out.push_str(event);
    out.push_str(" col=");
    let _ = write!(out, "{col}");
    out.push_str(" row=");
    let _ = write!(out, "{row}");
    out.push_str(" events=");
    let _ = write!(out, "{sent}");
    out.push('/');
    let _ = write!(out, "{total}");
    out
}

fn native_socket_send_mouse_ignored_log_line(
    window: &str,
    event: &str,
    modes: MouseReportingModes,
) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal socket send mouse ignored:  event= modes=".len()
            + window.len()
            + event.len()
            + 96,
    );
    out.push_str("native terminal socket send mouse ignored: ");
    out.push_str(window);
    out.push_str(" event=");
    out.push_str(event);
    out.push_str(" modes=");
    let _ = write!(out, "{modes:?}");
    out
}

fn native_terminal_resized_log_line(
    cols: u16,
    rows: u16,
    panes: usize,
    resize_failures: usize,
) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("native terminal resized to x panes= resize_failures=".len() + 60);
    out.push_str("native terminal resized to ");
    let _ = write!(out, "{cols}");
    out.push('x');
    let _ = write!(out, "{rows}");
    out.push_str(" panes=");
    let _ = write!(out, "{panes}");
    out.push_str(" resize_failures=");
    let _ = write!(out, "{resize_failures}");
    out
}

fn native_chrome_reservation_changed_log_line(
    reservation: &crate::daemon::NativeChromeReservationConfig,
) -> String {
    use std::fmt::Write as _;

    let owner = reservation.owner.as_deref().unwrap_or("-");
    let mut out = String::with_capacity(
        "native chrome reservation changed: top= bottom= left= right= gap=x owner=".len()
            + owner.len()
            + 60,
    );
    out.push_str("native chrome reservation changed: top=");
    let _ = write!(out, "{}", reservation.top_bar_rows);
    out.push_str(" bottom=");
    let _ = write!(out, "{}", reservation.bottom_bar_rows);
    out.push_str(" left=");
    let _ = write!(out, "{}", reservation.left_cols);
    out.push_str(" right=");
    let _ = write!(out, "{}", reservation.right_cols);
    out.push_str(" gap=");
    let _ = write!(out, "{}", reservation.gap_cols);
    out.push('x');
    let _ = write!(out, "{}", reservation.gap_rows);
    out.push_str(" owner=");
    out.push_str(owner);
    out
}

fn native_layout_resize_completed_log_line(resize_failures: usize) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("native layout resize completed with resize_failures=".len() + 20);
    out.push_str("native layout resize completed with resize_failures=");
    let _ = write!(out, "{resize_failures}");
    out
}

fn native_forwarded_host_sequence_log_line(window: &str, bytes: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal forwarded host sequence: window= bytes=".len() + window.len() + 20,
    );
    out.push_str("native terminal forwarded host sequence: window=");
    out.push_str(window);
    out.push_str(" bytes=");
    let _ = write!(out, "{bytes}");
    out
}

fn native_keyboard_focus_log_line(window: &str) -> String {
    let mut out = String::with_capacity("native terminal focus: ".len() + window.len());
    out.push_str("native terminal focus: ");
    out.push_str(window);
    out
}

fn native_layout_mode_log_line(label: &str) -> String {
    let mut out = String::with_capacity("native terminal layout mode: ".len() + label.len());
    out.push_str("native terminal layout mode: ");
    out.push_str(label);
    out
}

fn native_split_toggle_log_line(axis_label: &str) -> String {
    let mut out = String::with_capacity("native terminal split toggle: ".len() + axis_label.len());
    out.push_str("native terminal split toggle: ");
    out.push_str(axis_label);
    out
}

fn native_resize_weight_log_line(direction: &str, window: &str, weight: u16) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "native terminal resize :  weight=".len() + direction.len() + window.len() + 5,
    );
    out.push_str("native terminal resize ");
    out.push_str(direction);
    out.push_str(": ");
    out.push_str(window);
    out.push_str(" weight=");
    let _ = write!(out, "{weight}");
    out
}

fn native_launch_log_line(window: &str, panes: usize) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("native terminal launch:  panes=".len() + window.len() + 20);
    out.push_str("native terminal launch: ");
    out.push_str(window);
    out.push_str(" panes=");
    let _ = write!(out, "{panes}");
    out
}

fn native_split_log_line(axis: NativePaneLayoutAxis, panes: usize) -> String {
    use std::fmt::Write as _;

    let axis = match axis {
        NativePaneLayoutAxis::Columns => "Columns",
        NativePaneLayoutAxis::Rows => "Rows",
        NativePaneLayoutAxis::Grid => "Grid",
    };
    let mut out = String::with_capacity("native terminal split : panes=".len() + axis.len() + 20);
    out.push_str("native terminal split ");
    out.push_str(axis);
    out.push_str(": panes=");
    let _ = write!(out, "{panes}");
    out
}

fn native_help_overlay_log_line(visible: bool) -> String {
    let mut out = String::with_capacity("native terminal help overlay: ".len() + 5);
    out.push_str("native terminal help overlay: ");
    out.push_str(if visible { "true" } else { "false" });
    out
}

fn native_close_panes_log_line(panes: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("native terminal close: panes=".len() + 20);
    out.push_str("native terminal close: panes=");
    let _ = write!(out, "{panes}");
    out
}

fn native_reaped_pane_log_line(window: &str) -> String {
    let mut out = String::with_capacity("native terminal reaped exited pane ".len() + window.len());
    out.push_str("native terminal reaped exited pane ");
    out.push_str(window);
    out
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
                let _ = native_send_focus_event(&mut panes[focused], true);
            }
            dbg.log(&native_reaped_pane_log_line(&window));
        } else {
            idx += 1;
        }
    }
    Ok(focused.min(panes.len().saturating_sub(1)))
}

const NATIVE_FOCUSED_PANE_TITLE_MARKER: &str = "▶";
const NATIVE_UNFOCUSED_PANE_TITLE_MARKER: &str = " ";
const NATIVE_ACTIVE_PANE_TITLE_DRAG_MARKER: &str = "◆";
const NATIVE_FLOATING_PANE_DRAG_MARKER: &str = "≡";
const NATIVE_FLOATING_PANE_TOP_MARKER: &str = "▲";
const NATIVE_FLOATING_PANE_NOT_TOP_MARKER: &str = " ";
const NATIVE_FLOATING_PANE_MOVED_MARKER: &str = "●";
const NATIVE_FLOATING_PANE_NOT_MOVED_MARKER: &str = " ";
const NATIVE_TILED_PANE_REORDER_MARKER: &str = "⇄";
const NATIVE_RESIZED_TILED_PANE_MARKER: &str = "↔";
const NATIVE_FULLSCREEN_PANE_MARKER: &str = "▣";

fn native_title_drag_affordance_enabled(layout_label: &str) -> bool {
    layout_label.trim().eq_ignore_ascii_case("floating")
}

fn native_title_reorder_affordance_enabled(layout_label: &str) -> bool {
    let label = layout_label.trim();
    label.eq_ignore_ascii_case("columns")
        || label.eq_ignore_ascii_case("rows")
        || label.eq_ignore_ascii_case("grid")
        || label.eq_ignore_ascii_case("tiled")
}

fn native_title_fullscreen_affordance_enabled(layout_label: &str) -> bool {
    layout_label.trim().eq_ignore_ascii_case("fullscreen")
}

fn native_pane_title_text(
    pane: &NativePane,
    layout: NativePaneLayout,
    focused: bool,
    active_drag: bool,
    drag_affordance: bool,
    reorder_affordance: bool,
    fullscreen_affordance: bool,
    stack_top: bool,
    floating_moved: bool,
) -> String {
    let width = layout.cols as usize;
    let mut out = String::with_capacity(width);
    let mut count = 0usize;
    native_pane_title_push(
        &mut out,
        &mut count,
        width,
        if focused {
            NATIVE_FOCUSED_PANE_TITLE_MARKER
        } else {
            NATIVE_UNFOCUSED_PANE_TITLE_MARKER
        },
    );
    if active_drag {
        native_pane_title_push(
            &mut out,
            &mut count,
            width,
            NATIVE_ACTIVE_PANE_TITLE_DRAG_MARKER,
        );
    }
    if reorder_affordance {
        native_pane_title_push(
            &mut out,
            &mut count,
            width,
            NATIVE_TILED_PANE_REORDER_MARKER,
        );
    }
    if fullscreen_affordance {
        native_pane_title_push(&mut out, &mut count, width, NATIVE_FULLSCREEN_PANE_MARKER);
    }
    if reorder_affordance && pane.weight > 1 {
        native_pane_title_push(
            &mut out,
            &mut count,
            width,
            NATIVE_RESIZED_TILED_PANE_MARKER,
        );
    }
    if drag_affordance {
        native_pane_title_push(
            &mut out,
            &mut count,
            width,
            NATIVE_FLOATING_PANE_DRAG_MARKER,
        );
        native_pane_title_push(
            &mut out,
            &mut count,
            width,
            if stack_top {
                NATIVE_FLOATING_PANE_TOP_MARKER
            } else {
                NATIVE_FLOATING_PANE_NOT_TOP_MARKER
            },
        );
        native_pane_title_push(
            &mut out,
            &mut count,
            width,
            if floating_moved {
                NATIVE_FLOATING_PANE_MOVED_MARKER
            } else {
                NATIVE_FLOATING_PANE_NOT_MOVED_MARKER
            },
        );
    }
    native_pane_title_push(&mut out, &mut count, width, " ");
    native_pane_title_push(&mut out, &mut count, width, &pane.window);
    native_pane_title_push(&mut out, &mut count, width, " ");
    if count < width {
        if let Some(title) = pane.display_title.as_deref() {
            native_pane_title_push(&mut out, &mut count, width, title);
        } else {
            native_pane_title_push(&mut out, &mut count, width, &pane.app.title());
        }
    }
    if count < width {
        out.extend(std::iter::repeat(' ').take(width - count));
    }
    out
}

fn native_pane_title_push(out: &mut String, count: &mut usize, width: usize, text: &str) {
    if *count >= width {
        return;
    }
    for ch in text.chars().take(width - *count) {
        out.push(ch);
        *count += 1;
    }
}

fn ansi_fg_bg(fg: Rgba, bg: Rgba) -> String {
    let mut out = String::with_capacity(38);
    let _ = write!(
        out,
        "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m",
        fg.0, fg.1, fg.2, bg.0, bg.1, bg.2
    );
    out
}

fn native_initial_frame_requires_explicit_clear() -> bool {
    !raw_mode_enter_sequence_clears_screen()
}

fn should_write_pure_terminal_frame(
    last_rendered: &str,
    rendered: &str,
    redraw_static: bool,
    has_pending_output: bool,
) -> bool {
    redraw_static || has_pending_output || last_rendered != rendered
}

fn native_terminal_chrome_styles(colors: InlineChipColors) -> (String, String, String) {
    (
        ansi_fg_bg(colors.fg, colors.fill),
        ansi_fg_bg(colors.fg, colors.border),
        ansi_fg_bg(colors.fg, rgba_with_alpha(colors.fill, 180)),
    )
}

fn render_native_shell_view_terminal(view: &NativeShellView, cols: u16, rows: u16) -> String {
    let colors = native_glass_chrome_colors();
    let (top_bar_style, focused_title_style, unfocused_title_style) =
        native_terminal_chrome_styles(colors);
    let frame_cells = (cols as usize).saturating_mul(rows as usize);
    let ansi_overhead = view.panes.len().saturating_add(3).saturating_mul(64);
    let mut out = String::with_capacity(frame_cells.saturating_add(ansi_overhead));
    out.push_str("\x1b[H");
    if let Some(top_bar_row) = terminal_visible_row_opt(view.top_bar.row, rows) {
        let text = clip_and_pad(&view.top_bar.text, cols as usize);
        let _ = write!(
            out,
            "\x1b[{};1H{}{}\x1b[0m",
            top_bar_row + 1,
            top_bar_style,
            text
        );
    }
    // Empty workspaces intentionally render only the top bar by default.
    for pane in &view.panes {
        let title_style = if pane.focused {
            focused_title_style.as_str()
        } else {
            unfocused_title_style.as_str()
        };
        if let (Some(title_row), Some(title_width)) = (
            terminal_visible_row_opt(pane.y, rows),
            terminal_visible_width(pane.x, pane.cols, cols),
        ) {
            let text = clip_and_pad(&pane.text, title_width);
            let _ = write!(
                out,
                "\x1b[{};{}H{}{}\x1b[0m",
                title_row + 1,
                pane.x + 1,
                title_style,
                text
            );
        }
        if pane.app_cols > 0 && pane.app_rows > 0 {
            for (line_idx, line) in pane
                .text_snapshot
                .lines()
                .take(pane.app_rows as usize)
                .enumerate()
            {
                let Some(line_row) = terminal_visible_row_opt(pane.app_y + line_idx as u16, rows)
                else {
                    continue;
                };
                let Some(line_width) = terminal_visible_width(pane.app_x, pane.app_cols, cols)
                else {
                    continue;
                };
                let clipped = clip_and_pad(line, line_width);
                let _ = write!(out, "\x1b[{};{}H{}", line_row + 1, pane.app_x + 1, clipped);
            }
        }
    }
    if view.help_overlay {
        if let Some(help_width) = native_help_overlay_ansi_width(cols) {
            for (idx, line) in native_help_overlay_lines().iter().enumerate() {
                let row = 2 + idx as u16;
                if row >= rows {
                    break;
                }
                let text = clip_and_pad(line, help_width);
                let _ = write!(out, "\x1b[{};3H\x1b[7m {} \x1b[0m", row + 1, text);
            }
        }
    }
    if !view.footer.text.is_empty() {
        let footer = clip_and_pad(&view.footer.text, cols as usize);
        let _ = write!(
            out,
            "\x1b[0m\x1b[{};1H\x1b[K{}",
            terminal_visible_row(view.footer.row, rows) + 1,
            footer
        );
    }
    out
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct NativeShellCompositionEntry {
    id: String,
    kind: String,
    z: u16,
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct NativeShellChromeScene {
    id: String,
    x: u16,
    y: u16,
    scene: Scene,
}

fn native_top_bar_scene(view: &NativeShellView, cols: u16, cell_size: CellSize) -> Scene {
    let mut scene =
        native_top_bar_model_from_view(view).scene_with_prefix(cols, "kittwm-live-top-bar");
    scene.footprint.y = view.top_bar.row;
    scene.cell_size = cell_size;
    scene
}

fn native_top_bar_model_from_view(view: &NativeShellView) -> BarModel {
    let text = view.top_bar.text.trim();
    let time = native_top_bar_time_from_text(text);
    BarModel {
        workspace: workspace_label(),
        panes: view.panes.len() as u64,
        state: if view.panes.is_empty() {
            "empty"
        } else {
            "active"
        }
        .to_string(),
        focus: "-".to_string(),
        time,
        connected: true,
    }
}

fn native_top_bar_time_from_text(text: &str) -> String {
    let mut parts = text.split_whitespace();
    let Some(last) = parts.next_back() else {
        return "00:00 UTC".to_string();
    };
    if last == "UTC" {
        if let Some(clock) = parts.next_back().filter(|part| is_hh_mm_clock(part)) {
            return native_top_bar_time_label(clock, "UTC");
        }
    } else if is_hh_mm_clock(last) {
        return native_top_bar_time_label(last, "UTC");
    }
    "00:00 UTC".to_string()
}

fn native_top_bar_time_label(clock: &str, zone: &str) -> String {
    let mut out = String::with_capacity(clock.len() + 1 + zone.len());
    out.push_str(clock);
    out.push(' ');
    out.push_str(zone);
    out
}

fn is_hh_mm_clock(value: &str) -> bool {
    let Some((hour, minute)) = value.split_once(':') else {
        return false;
    };
    hour.len() == 2
        && minute.len() == 2
        && hour.chars().all(|ch| ch.is_ascii_digit())
        && minute.chars().all(|ch| ch.is_ascii_digit())
}

pub fn native_showcase_composition_json(
    cols: u16,
    rows: u16,
    help_overlay: bool,
) -> Result<String> {
    let scenes = native_showcase_scenes(cols, rows, help_overlay);
    let mut entries = Vec::new();
    entries.push(NativeShellCompositionEntry {
        id: "background".to_string(),
        kind: "background".to_string(),
        z: 0,
        x: 0,
        y: 0,
        cols: cols.max(40),
        rows: rows.max(12),
    });
    for scene in &scenes {
        let kind = if scene.id.contains("title")
            || scene.id.contains("border")
            || scene.id == "footer"
            || scene.id == "top-bar"
        {
            "chrome"
        } else {
            "overlay"
        };
        if scene.id.ends_with("-border") {
            entries.push(NativeShellCompositionEntry {
                id: scene.id.replace("-border", "-app-frame"),
                kind: "app-frame".to_string(),
                z: 10,
                x: scene.x.saturating_add(NATIVE_PANE_BORDER_COLS),
                y: scene.y.saturating_add(NATIVE_PANE_TITLE_ROWS),
                cols: scene
                    .scene
                    .footprint
                    .cols
                    .saturating_sub(NATIVE_PANE_BORDER_COLS * 2)
                    .max(1),
                rows: scene
                    .scene
                    .footprint
                    .rows
                    .saturating_sub(NATIVE_PANE_TITLE_ROWS)
                    .saturating_sub(NATIVE_PANE_BOTTOM_BORDER_ROWS)
                    .max(1),
            });
        }
        entries.push(NativeShellCompositionEntry {
            id: scene.id.clone(),
            kind: kind.to_string(),
            z: if kind == "overlay" { 30 } else { 20 },
            x: scene.x,
            y: scene.y,
            cols: scene.scene.footprint.cols,
            rows: scene.scene.footprint.rows,
        });
    }
    entries.sort_by_key(|entry| entry.z);
    serde_json::to_string_pretty(&serde_json::json!({
        "kind": "kittwm-shell-composition",
        "entries": entries,
    }))
    .map_err(Into::into)
}

pub fn native_showcase_scene_json(cols: u16, rows: u16, help_overlay: bool) -> Result<String> {
    let scenes = native_showcase_scenes(cols, rows, help_overlay);
    serde_json::to_string_pretty(&scenes).map_err(Into::into)
}

pub fn native_tui_smoke_matrix_json() -> Result<String> {
    let cases = [
        (
            "shell-prompts",
            "basic prompt text and newline handling",
            "covered",
        ),
        (
            "cursor-addressing",
            "CUP/HVP cursor movement used by editors",
            "covered",
        ),
        (
            "alternate-screen",
            "htop/vim style DEC alt screen enter/leave",
            "covered",
        ),
        ("colors", "SGR foreground/background attributes", "covered"),
        ("box-drawing", "line/table glyph rendering", "covered"),
        (
            "mouse-sgr",
            "SGR mouse press/move/release routing",
            "covered",
        ),
        ("bracketed-paste", "DECSET 2004 paste wrapping", "covered"),
        (
            "ctrl-c",
            "Ctrl-C forwarding and triple-exit guard",
            "covered",
        ),
        (
            "real-fonts",
            "Fira Code/real glyph rasterization",
            "covered",
        ),
    ];
    let cases = cases
        .into_iter()
        .map(|(id, description, status)| {
            serde_json::json!({
                "id": id,
                "description": description,
                "status": status,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "kind": "kittwm-tui-smoke-matrix",
        "cases": cases,
    }))
    .map_err(Into::into)
}

pub fn native_showcase_metrics_json(cols: u16, rows: u16, help_overlay: bool) -> Result<String> {
    let cols = cols.max(40);
    let rows = rows.max(12);
    let scenes = native_showcase_scenes(cols, rows, help_overlay);
    let scene_count = scenes.len();
    let layer_count = scenes
        .iter()
        .map(|scene| scene.scene.layers.len())
        .sum::<usize>();
    let total_pixels = scenes
        .iter()
        .map(|scene| scene.scene.pixel_width() as u64 * scene.scene.pixel_height() as u64)
        .sum::<u64>();
    serde_json::to_string_pretty(&serde_json::json!({
        "kind": "kittwm-showcase-metrics",
        "cols": cols,
        "rows": rows,
        "help_overlay": help_overlay,
        "scene_count": scene_count,
        "layer_count": layer_count,
        "total_pixels": total_pixels,
        "cell_width_px": native_cell_width_px(),
        "cell_height_px": native_cell_height_px(),
    }))
    .map_err(Into::into)
}

fn native_showcase_scenes(cols: u16, rows: u16, help_overlay: bool) -> Vec<NativeShellChromeScene> {
    let cols = cols.max(40);
    let rows = rows.max(12);
    let app_rows = rows.saturating_sub(4).max(3);
    let split_cols = cols / 2;
    let view = NativeShellView {
        top_bar: NativeTopBarChrome {
            row: 0,
            text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
        },
        panes: vec![
            NativePaneChrome {
                x: 0,
                y: 1,
                focused: true,
                text: "* native-1 shell".to_string(),
                cache_key: "showcase-1".to_string(),
                status: "shell · pid:101 · frame:clean".to_string(),
                app_x: 0,
                app_y: 2,
                app_cols: split_cols,
                app_rows,
                cols: split_cols,
                rows: app_rows.saturating_add(1),
                text_snapshot: String::new(),
            },
            NativePaneChrome {
                x: split_cols,
                y: 1,
                focused: false,
                text: "  native-2 logs".to_string(),
                cache_key: "showcase-2".to_string(),
                status: "logs · pid:102 · frame:4".to_string(),
                app_x: split_cols,
                app_y: 2,
                app_cols: cols.saturating_sub(split_cols),
                app_rows,
                cols: cols.saturating_sub(split_cols),
                rows: app_rows.saturating_add(1),
                text_snapshot: String::new(),
            },
        ],
        footer: NativeFooterChrome {
            row: rows.saturating_sub(1),
            text: " C-a ? help · C-a g launcher · C-a Enter term · C-a t float · C-a f full · C-a e split · C-a x close · Ctrl-] exit · log: showcase"
                .to_string(),
        },
        help_overlay,
    };
    render_native_shell_view_affordance_scenes(&view, native_cell_size(), cols, rows)
}

fn native_pane_border_scene_id(idx: usize) -> String {
    let mut id = String::with_capacity("pane--border".len() + 20);
    id.push_str("pane-");
    let _ = write!(id, "{idx}");
    id.push_str("-border");
    id
}

fn render_native_shell_view_affordance_scenes(
    view: &NativeShellView,
    cell_size: CellSize,
    cols: u16,
    _rows: u16,
) -> Vec<NativeShellChromeScene> {
    let mut scenes = Vec::new();
    scenes.push(NativeShellChromeScene {
        id: "top-bar".to_string(),
        x: 0,
        y: view.top_bar.row,
        scene: native_top_bar_scene(view, cols, cell_size),
    });
    // Empty workspaces intentionally render only the top bar by default.
    for (idx, pane) in view.panes.iter().enumerate() {
        scenes.push(NativeShellChromeScene {
            id: native_pane_border_scene_id(idx),
            x: pane.x,
            y: pane.y,
            scene: native_pane_border_scene(idx, pane, cell_size),
        });
    }
    if !view.footer.text.is_empty() {
        scenes.push(NativeShellChromeScene {
            id: "footer".to_string(),
            x: 0,
            y: view.footer.row,
            scene: native_footer_status_scene(cell_size, cols, &view.footer.text),
        });
        if native_should_show_footer_toast(&view.footer.text) {
            if let Some((x, y, scene)) =
                native_toast_scene(cell_size, cols, view.footer.row, &view.footer.text)
            {
                scenes.push(NativeShellChromeScene {
                    id: "toast".to_string(),
                    x,
                    y,
                    scene,
                });
            }
        }
    }
    scenes
}

static NATIVE_GLASS_CHROME_COLORS: OnceLock<InlineChipColors> = OnceLock::new();

fn native_glass_chrome_colors() -> InlineChipColors {
    NATIVE_GLASS_CHROME_COLORS
        .get_or_init(resolve_native_glass_chrome_colors)
        .clone()
}

fn resolve_native_glass_chrome_colors() -> InlineChipColors {
    let mut colors = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Glass);
    if let Ok(config) = KittwmConfig::load_default() {
        apply_kittwm_config_to_chrome_colors(&mut colors, &config);
    }
    colors
}

fn apply_kittwm_config_to_chrome_colors(colors: &mut InlineChipColors, config: &KittwmConfig) {
    colors.fill = config_color_rgba(
        &config.background.color,
        config.background.opacity,
        colors.fill,
    );
    colors.border = config_color_rgba(&config.colorscheme.fg, 1.0, colors.border);
    colors.highlight = config_color_rgba(
        config
            .colorscheme
            .ansi_color(4)
            .unwrap_or(&config.colorscheme.fg),
        0.42,
        colors.highlight,
    );
    colors.fg = config_color_rgba(&config.colorscheme.fg, 1.0, colors.fg);
}

fn config_color_rgba(value: &str, opacity: f32, fallback: Rgba) -> Rgba {
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgba = nord_or_hex_rgba(value, alpha);
    if rgba == [0x2e, 0x34, 0x40, alpha]
        && !matches!(value.to_ascii_lowercase().as_str(), "nord0" | "#2e3440")
    {
        return fallback;
    }
    Rgba(rgba[0], rgba[1], rgba[2], rgba[3])
}

#[cfg(test)]
fn native_pane_title_focus_marker_label(idx: usize) -> String {
    use std::fmt::Write as _;

    let mut label = String::with_capacity("pane--title-focus-marker".len() + 20);
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-title-focus-marker");
    label
}

#[cfg(test)]
fn native_pane_title_status_scene(
    idx: usize,
    pane: &NativePaneChrome,
    cell_size: CellSize,
) -> Scene {
    let colors = native_glass_chrome_colors();
    let cols = pane.cols.max(1);
    let rect = CellRect::new(0, 0, cols, 1).to_pixels(cell_size);
    let cell_w = cell_size.width_px.max(1) as f32;
    let chip_h = (cell_size.height_px.max(1) as f32 - 4.0).max(6.0);
    let mut layers = vec![Layer::new(
        native_pane_title_strip_label(idx, &pane.text),
        Node::Rect {
            rect,
            fill: Paint::Solid {
                color: if pane.focused {
                    colors.fill
                } else {
                    rgba_with_alpha(colors.fill, 115)
                },
            },
            stroke: Some(Stroke::inside(
                1.0,
                Paint::Solid {
                    color: if pane.focused {
                        colors.border
                    } else {
                        rgba_with_alpha(colors.border, 145)
                    },
                },
            )),
            corners: Corners::uniform(5.0),
        },
    )];
    let focus_width = if pane.focused { 4.0 } else { 2.0 };
    layers.push(Layer::new(
        native_pane_title_focus_marker_label(idx),
        Node::Rect {
            rect: PxRect::new(0.0, 0.0, focus_width, rect.height),
            fill: Paint::Solid {
                color: if pane.focused {
                    colors.border
                } else {
                    rgba_with_alpha(colors.border, 90)
                },
            },
            stroke: None,
            corners: Corners::uniform(4.0),
        },
    ));
    layers.push(Layer::new(
        native_pane_status_chip_label(idx, &pane.status),
        Node::Rect {
            rect: native_pane_status_chip_rect(cols, rect.width, cell_w, chip_h),
            fill: Paint::Solid {
                color: rgba_with_alpha(colors.border, if pane.focused { 70 } else { 42 }),
            },
            stroke: Some(Stroke::inside(
                1.0,
                Paint::Solid {
                    color: rgba_with_alpha(colors.border, if pane.focused { 220 } else { 120 }),
                },
            )),
            corners: Corners::uniform(5.0),
        },
    ));
    Scene {
        footprint: CellRect::new(0, 0, cols, 1),
        cell_size,
        layers,
        animation: None,
    }
}

const NATIVE_TOAST_TRIGGER_KEYWORDS: &[&str] = &["error", "failed", "denied", "launcher.error"];

fn native_should_show_footer_toast(message: &str) -> bool {
    ascii_contains_any_ignore_case(message, NATIVE_TOAST_TRIGGER_KEYWORDS)
}

fn ascii_contains_any_ignore_case(haystack: &str, needles: &[&str]) -> bool {
    if needles.iter().any(|needle| needle.is_empty()) {
        return true;
    }
    let haystack = haystack.as_bytes();
    for start in 0..haystack.len() {
        if needles.iter().any(|needle| {
            let needle = needle.as_bytes();
            haystack
                .get(start..start.saturating_add(needle.len()))
                .is_some_and(|window| window.eq_ignore_ascii_case(needle))
        }) {
            return true;
        }
    }
    false
}

fn native_toast_scene(
    cell_size: CellSize,
    cols: u16,
    footer_row: u16,
    message: &str,
) -> Option<(u16, u16, Scene)> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }
    let colors = native_toast_colors(trimmed);
    let msg_cols = native_toast_message_cols(trimmed);
    let toast_cols = native_toast_cols(msg_cols, cols);
    let toast_rows = 3u16;
    let x = cols.saturating_sub(toast_cols).saturating_div(2);
    let y = footer_row
        .saturating_sub(toast_rows.saturating_add(1))
        .max(1);
    let rect = CellRect::new(0, 0, toast_cols, toast_rows).to_pixels(cell_size);
    let rail =
        kittui_core::geom::PxRect::new(0.0, 0.0, cell_size.width_px.max(1) as f32, rect.height);
    let layers = vec![
        Layer::new(
            native_toast_label(
                "toast-backdrop:",
                clip_and_pad(trimmed, toast_cols as usize).trim(),
            ),
            Node::Rect {
                rect,
                fill: Paint::Solid { color: colors.fill },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: colors.border,
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        ),
        Layer::new(
            "toast-accent-rail",
            Node::Rect {
                rect: rail,
                fill: Paint::Solid {
                    color: colors.border,
                },
                stroke: None,
                corners: Corners::uniform(8.0),
            },
        ),
        Layer::new(
            "toast-highlight",
            Node::Rect {
                rect: kittui_core::geom::PxRect::new(
                    0.0,
                    0.0,
                    rect.width,
                    (rect.height / 2.0).max(1.0),
                ),
                fill: Paint::Solid {
                    color: colors.highlight,
                },
                stroke: None,
                corners: Corners::uniform(8.0),
            },
        ),
        Layer::new(
            native_toast_label(
                "toast-text:",
                clip_and_pad(trimmed, toast_cols.saturating_sub(4) as usize).trim(),
            ),
            Node::Group {
                opacity: 1.0,
                children: Vec::new(),
            },
        ),
    ];
    Some((
        x,
        y,
        Scene {
            footprint: CellRect::new(0, 0, toast_cols, toast_rows),
            cell_size,
            layers,
            animation: None,
        },
    ))
}

fn native_toast_label(prefix: &str, visible: &str) -> String {
    let mut label = String::with_capacity(prefix.len() + visible.len());
    label.push_str(prefix);
    label.push_str(visible);
    label
}

fn native_toast_message_cols(message: &str) -> u16 {
    message.chars().take(u16::MAX as usize).count() as u16
}

fn native_toast_cols(message_cols: u16, terminal_cols: u16) -> u16 {
    let available = terminal_cols.max(1);
    message_cols.saturating_add(4).clamp(
        20.min(available),
        terminal_cols.saturating_sub(4).max(20).min(available),
    )
}

fn native_toast_colors(message: &str) -> InlineChipColors {
    let mut colors = native_glass_chrome_colors();
    if native_should_show_footer_toast(message) {
        colors.highlight = colors.border;
        colors.border = config_color_rgba("#bf616a", 1.0, colors.border);
    }
    colors
}

const NATIVE_FOOTER_STATUS_LABEL_MAX_CHARS: usize = 96;

fn native_footer_status_backdrop_label(status_label: &str) -> String {
    let mut label = String::with_capacity("status-bar-backdrop:".len() + status_label.len());
    label.push_str("status-bar-backdrop:");
    label.push_str(status_label);
    label
}

fn native_footer_status_chip_label(label: &str) -> String {
    let mut out = String::with_capacity("status-chip-".len() + label.len());
    out.push_str("status-chip-");
    out.push_str(label);
    out
}

fn native_footer_status_scene(cell_size: CellSize, cols: u16, status_text: &str) -> Scene {
    let colors = native_glass_chrome_colors();
    let cols = cols.max(1);
    let rect = CellRect::new(0, 0, cols, 1).to_pixels(cell_size);
    let cell_w = cell_size.width_px.max(1) as f32;
    let chip_h = (cell_size.height_px.max(1) as f32 - 4.0).max(6.0);
    let chip_specs = [
        ("help", 1.0, 10.0),
        ("focus", 12.5, 9.0),
        ("state", 23.0, 9.0),
        ("drag", 33.5, 8.0),
        ("terminal", 43.0, 14.0),
        ("close", 58.5, 9.0),
    ];
    let status_label = bounded_ellipsis(status_text, NATIVE_FOOTER_STATUS_LABEL_MAX_CHARS);
    let mut layers = vec![Layer::new(
        native_footer_status_backdrop_label(&status_label),
        Node::Rect {
            rect,
            fill: Paint::Solid {
                color: rgba_with_alpha(colors.fill, 145),
            },
            stroke: Some(Stroke::inside(
                1.0,
                Paint::Solid {
                    color: rgba_with_alpha(colors.border, 180),
                },
            )),
            corners: Corners::uniform(5.0),
        },
    )];
    for (label, x_cells, width_cells) in chip_specs {
        let x = x_cells * cell_w;
        if x >= rect.width {
            continue;
        }
        let width = (width_cells * cell_w).min((rect.width - x - 4.0).max(1.0));
        if width <= 1.0 || x + width > rect.width {
            continue;
        }
        layers.push(Layer::new(
            native_footer_status_chip_label(label),
            Node::Rect {
                rect: PxRect::new(x, 2.0, width, chip_h),
                fill: Paint::Solid {
                    color: rgba_with_alpha(colors.border, 62),
                },
                stroke: Some(Stroke::inside(
                    1.0,
                    Paint::Solid {
                        color: colors.border,
                    },
                )),
                corners: Corners::uniform(5.0),
            },
        ));
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, 1),
        cell_size,
        layers,
        animation: None,
    }
}

fn native_empty_workspace_action_chip_label(idx: usize) -> String {
    let mut label = String::with_capacity("empty-workspace-action-chip-".len() + 20);
    label.push_str("empty-workspace-action-chip-");
    let _ = write!(label, "{idx}");
    label
}

#[allow(dead_code)]
fn native_empty_workspace_scene(
    cell_size: CellSize,
    cols: u16,
    footer_row: u16,
) -> (u16, u16, Scene) {
    let colors = native_glass_chrome_colors();
    let panel_cols = cols.saturating_sub(8).clamp(24, 72).min(cols.max(1));
    let y = 2.min(footer_row.saturating_sub(1));
    let available_rows = footer_row.saturating_sub(y).max(1);
    let panel_rows = available_rows.min(10).max(1);
    let x = cols.saturating_sub(panel_cols).saturating_div(2);
    let rect = CellRect::new(0, 0, panel_cols, panel_rows).to_pixels(cell_size);
    let cell_h = cell_size.height_px.max(1) as f32;
    let accent_x = if rect.width > 20.0 { 10.0 } else { 0.0 };
    let accent_w = (rect.width - accent_x * 2.0).max(1.0);
    let accent_y = (cell_h * 2.7).min((rect.height - 1.0).max(0.0));
    let accent_h = 2.0_f32.min((rect.height - accent_y).max(1.0));
    let chip_y =
        ((panel_rows.saturating_sub(3).max(1) as f32) * cell_h).min((rect.height - 1.0).max(0.0));
    let chip_gap = if rect.width > 36.0 { 8.0 } else { 1.0 };
    let chip_x0 = if rect.width > 20.0 { 10.0 } else { 0.0 };
    let chip_available_w = (rect.width - chip_x0 * 2.0 - chip_gap * 2.0).max(1.0);
    let chip_w = (chip_available_w / 3.0).max(1.0);
    let chip_h = (cell_h - 4.0).max(6.0).min((rect.height - chip_y).max(1.0));
    let mut layers = vec![
        Layer::new(
            "empty-workspace-backdrop",
            Node::Rect {
                rect,
                fill: Paint::Solid { color: colors.fill },
                stroke: Some(Stroke::inside(
                    2.0,
                    Paint::Solid {
                        color: colors.border,
                    },
                )),
                corners: Corners::uniform(10.0),
            },
        ),
        Layer::new(
            "empty-workspace-hero-band",
            Node::Rect {
                rect: PxRect::new(0.0, 0.0, rect.width, (cell_h * 2.2).min(rect.height)),
                fill: Paint::Solid {
                    color: colors.highlight,
                },
                stroke: None,
                corners: Corners::uniform(10.0),
            },
        ),
        Layer::new(
            "empty-workspace-accent-rail",
            Node::Rect {
                rect: PxRect::new(accent_x, accent_y, accent_w, accent_h),
                fill: Paint::Solid {
                    color: colors.border,
                },
                stroke: None,
                corners: Corners::uniform(2.0),
            },
        ),
    ];
    for idx in 0..3 {
        layers.push(Layer::new(
            native_empty_workspace_action_chip_label(idx),
            Node::Rect {
                rect: PxRect::new(
                    (chip_x0 + idx as f32 * (chip_w + chip_gap))
                        .min((rect.width - chip_w).max(0.0)),
                    chip_y,
                    chip_w,
                    chip_h,
                ),
                fill: Paint::Solid {
                    color: rgba_with_alpha(colors.border, 72),
                },
                stroke: Some(Stroke::inside(
                    1.0,
                    Paint::Solid {
                        color: colors.border,
                    },
                )),
                corners: Corners::uniform(6.0),
            },
        ));
    }
    (
        x,
        y,
        Scene {
            footprint: CellRect::new(0, 0, panel_cols, panel_rows),
            cell_size,
            layers,
            animation: None,
        },
    )
}

fn rgba_with_alpha(color: Rgba, alpha: u8) -> Rgba {
    Rgba::rgba(color.0, color.1, color.2, alpha)
}

#[cfg(test)]
fn native_help_overlay_key_chip_label(idx: usize) -> String {
    use std::fmt::Write as _;

    let mut label = String::with_capacity("help-overlay-key-chip-".len() + 20);
    label.push_str("help-overlay-key-chip-");
    let _ = write!(label, "{idx}");
    label
}

#[cfg(test)]
fn native_help_overlay_row_label(idx: usize) -> String {
    use std::fmt::Write as _;

    let mut label = String::with_capacity("help-overlay-row-".len() + 20);
    label.push_str("help-overlay-row-");
    let _ = write!(label, "{idx}");
    label
}

#[cfg(test)]
fn native_help_overlay_scene(
    cell_size: CellSize,
    cols: u16,
    rows: u16,
    lines: &[&str],
) -> (u16, u16, Scene) {
    if lines.is_empty() {
        return (
            0,
            0,
            Scene {
                footprint: CellRect::new(0, 0, 1, 1),
                cell_size,
                layers: Vec::new(),
                animation: None,
            },
        );
    }
    let colors = native_glass_chrome_colors();
    let max_line = native_help_overlay_max_line_cols(lines);
    let available_cols = cols.max(1);
    let panel_cols = max_line
        .saturating_add(4)
        .min(cols.saturating_sub(4).max(20))
        .min(available_cols);
    let y = 2.min(rows.saturating_sub(1));
    let available_rows = rows.saturating_sub(y).max(1);
    let panel_rows = native_help_overlay_panel_rows(lines.len(), available_rows);
    let x = cols.saturating_sub(panel_cols).saturating_div(2);
    let rect = CellRect::new(0, 0, panel_cols, panel_rows).to_pixels(cell_size);
    let row_h = cell_size.height_px.max(1) as f32;
    let chip_h = (row_h - 5.0).max(6.0);
    let chip_x = if rect.width > 20.0 { 10.0 } else { 0.0 };
    let chip_w =
        (cell_size.width_px.max(1) as f32 * 15.0).min((rect.width - chip_x * 2.0).max(1.0));
    let row_line_x = if rect.width > 16.0 { 8.0 } else { 0.0 };
    let row_line_w = (rect.width - row_line_x * 2.0).max(1.0);
    let mut layers = vec![
        Layer::new(
            "help-overlay-backdrop",
            Node::Rect {
                rect,
                fill: Paint::Solid { color: colors.fill },
                stroke: Some(Stroke::inside(
                    2.0,
                    Paint::Solid {
                        color: colors.border,
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        ),
        Layer::new(
            "help-overlay-heading-band",
            Node::Rect {
                rect: PxRect::new(0.0, 0.0, rect.width, (row_h * 1.4).min(rect.height)),
                fill: Paint::Solid {
                    color: colors.highlight,
                },
                stroke: None,
                corners: Corners::uniform(8.0),
            },
        ),
    ];
    for (idx, line) in lines.iter().enumerate().skip(1) {
        let row_y = row_h * (idx as f32 + 1.0);
        if row_y >= rect.height {
            break;
        }
        let keyish = line.starts_with("C-a") || line.starts_with("Ctrl-");
        let chip_y = row_y + 2.0;
        if keyish && chip_y < rect.height {
            let bounded_chip_h = chip_h.min((rect.height - chip_y).max(1.0));
            layers.push(Layer::new(
                native_help_overlay_key_chip_label(idx),
                Node::Rect {
                    rect: PxRect::new(chip_x, chip_y, chip_w, bounded_chip_h),
                    fill: Paint::Solid {
                        color: rgba_with_alpha(colors.border, 80),
                    },
                    stroke: Some(Stroke::inside(
                        1.0,
                        Paint::Solid {
                            color: colors.border,
                        },
                    )),
                    corners: Corners::uniform(5.0),
                },
            ));
        }
        let row_line_y = row_y + row_h - 2.0;
        if row_line_y < rect.height {
            layers.push(Layer::new(
                native_help_overlay_row_label(idx),
                Node::Rect {
                    rect: PxRect::new(row_line_x, row_line_y, row_line_w, 1.0),
                    fill: Paint::Solid {
                        color: colors.highlight,
                    },
                    stroke: None,
                    corners: Corners::default(),
                },
            ));
        }
    }
    native_help_overlay_control_layers(cell_size, panel_cols, panel_rows, &mut layers);
    (
        x,
        y,
        Scene {
            footprint: CellRect::new(0, 0, panel_cols, panel_rows),
            cell_size,
            layers,
            animation: None,
        },
    )
}

#[cfg(test)]
fn native_help_overlay_max_line_cols(lines: &[&str]) -> u16 {
    lines
        .iter()
        .map(|line| line.chars().count().min(u16::MAX as usize) as u16)
        .max()
        .unwrap_or(20)
}

#[cfg(test)]
fn native_help_overlay_panel_rows(line_count: usize, available_rows: u16) -> u16 {
    let rows = line_count.saturating_add(2).min(u16::MAX as usize) as u16;
    rows.max(4).min(available_rows)
}

#[cfg(test)]
fn native_help_overlay_control_layers(
    cell_size: CellSize,
    panel_cols: u16,
    panel_rows: u16,
    layers: &mut Vec<Layer>,
) {
    if panel_rows < 4 || panel_cols < 24 {
        return;
    }
    let row = panel_rows.saturating_sub(2);
    let mut close = button("help.close", "Close", 9)
        .state(ControlState::default().focused(true).selected(true))
        .to_scene(cell_size);
    native_prefix_and_offset_control_layers(
        &mut close,
        "help-overlay-control-button:toggle-help",
        2,
        row,
        cell_size,
    );
    layers.extend(close.layers);

    let mut filter = text_input(
        "help.filter",
        "Filter",
        "shortcuts",
        panel_cols.saturating_sub(14),
    )
    .state(ControlState::default().focused(false))
    .to_scene(cell_size);
    native_prefix_and_offset_control_layers(
        &mut filter,
        "help-overlay-control-text-input:filter-placeholder",
        12,
        row,
        cell_size,
    );
    layers.extend(filter.layers);

    layers.push(Layer::new(
        "help-overlay-control-action:toggle-help:C-a ?",
        Node::Group {
            opacity: 1.0,
            children: Vec::new(),
        },
    ));
}

#[cfg(test)]
fn native_prefixed_control_layer_label(prefix: &str, idx: usize, suffix: &str) -> String {
    let mut label = String::with_capacity(prefix.len() + 1 + 20 + 1 + suffix.len());
    label.push_str(prefix);
    label.push(':');
    let _ = write!(label, "{idx}");
    label.push(':');
    label.push_str(suffix);
    label
}

#[cfg(test)]
fn native_prefix_and_offset_control_layers(
    scene: &mut Scene,
    prefix: &str,
    x_cells: u16,
    y_cells: u16,
    cell_size: CellSize,
) {
    let dx = x_cells as f32 * cell_size.width_px as f32;
    let dy = y_cells as f32 * cell_size.height_px as f32;
    for (idx, layer) in scene.layers.iter_mut().enumerate() {
        let suffix = layer.label.as_deref().unwrap_or("layer");
        layer.label = Some(native_prefixed_control_layer_label(prefix, idx, suffix));
        native_offset_node(&mut layer.root, dx, dy);
    }
}

#[cfg(test)]
fn native_offset_node(node: &mut Node, dx: f32, dy: f32) {
    match node {
        Node::Rect { rect, .. }
        | Node::Gradient { rect, .. }
        | Node::Glow { rect, .. }
        | Node::Scanlines { rect, .. }
        | Node::Image { rect, .. }
        | Node::Shader { rect, .. }
        | Node::Clip { rect, .. } => {
            rect.origin.0 += dx;
            rect.origin.1 += dy;
        }
        Node::Group { children, .. } | Node::Composite { children, .. } => {
            for child in children {
                native_offset_node(child, dx, dy);
            }
        }
        Node::Mask { mask, child } => {
            native_offset_node(mask, dx, dy);
            native_offset_node(child, dx, dy);
        }
    }
}

fn native_pane_focus_glow_label(idx: usize) -> String {
    let mut label = String::with_capacity("pane--focus-glow".len() + 20);
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-focus-glow");
    label
}

fn native_pane_title_gutter_label(idx: usize) -> String {
    let mut label = String::with_capacity("pane--title-gutter".len() + 20);
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-title-gutter");
    label
}

fn native_pane_focus_accent_rail_label(idx: usize) -> String {
    let mut label = String::with_capacity("pane--focus-accent-rail".len() + 20);
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-focus-accent-rail");
    label
}

fn native_pane_kittui_border_label(idx: usize) -> String {
    let mut label = String::with_capacity("pane--kittui-border".len() + 20);
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-kittui-border");
    label
}

fn native_pane_focus_ring_label(idx: usize) -> String {
    let mut label = String::with_capacity("pane--focus-ring".len() + 20);
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-focus-ring");
    label
}

fn native_pane_title_strip_label(idx: usize, text: &str) -> String {
    let mut label = String::with_capacity("pane--title-strip:".len() + 20 + text.len());
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-title-strip:");
    label.push_str(text);
    label
}

fn native_pane_status_chip_label(idx: usize, status: &str) -> String {
    let mut label = String::with_capacity("pane--status-chip:".len() + 20 + status.len());
    label.push_str("pane-");
    let _ = write!(label, "{idx}");
    label.push_str("-status-chip:");
    label.push_str(status);
    label
}

fn native_pane_status_chip_rect(cols: u16, rect_width: f32, cell_w: f32, chip_h: f32) -> PxRect {
    let min_w = cell_w.max(1.0).min(rect_width.max(1.0));
    let right_pad = 4.0_f32.min((rect_width - min_w).max(0.0));
    let preferred_x = (cols.saturating_sub(12).max(4) as f32) * cell_w.max(1.0);
    let max_x = (rect_width - min_w).max(0.0);
    let x = preferred_x.min(max_x).max(0.0);
    let w = (rect_width - x - right_pad)
        .max(min_w)
        .min((rect_width - x).max(1.0));
    PxRect::new(x, 2.0, w, chip_h)
}

fn native_pane_border_scene(idx: usize, pane: &NativePaneChrome, cell_size: CellSize) -> Scene {
    let colors = native_glass_chrome_colors();
    let cols = pane.cols.max(1);
    let rows = pane.rows.max(1);
    let rect = CellRect::new(0, 0, cols, rows).to_pixels(cell_size);
    let border = if pane.focused {
        colors.border
    } else {
        rgba_with_alpha(colors.border, 150)
    };
    let title_fill = if pane.focused {
        colors.fill
    } else {
        rgba_with_alpha(colors.fill, 110)
    };
    let title_rect = PxRect::new(0.0, 0.0, rect.width, cell_size.height_px.max(1) as f32);
    let cell_w = cell_size.width_px.max(1) as f32;
    let chip_h = (cell_size.height_px.max(1) as f32 - 4.0).max(6.0);
    let mut layers = Vec::new();
    if pane.focused {
        layers.push(Layer::new(
            native_pane_focus_glow_label(idx),
            Node::Rect {
                rect,
                fill: Paint::Solid {
                    color: rgba_with_alpha(colors.border, 26),
                },
                stroke: None,
                corners: Corners::uniform(7.0),
            },
        ));
    }
    layers.push(Layer::new(
        native_pane_title_strip_label(idx, &pane.text),
        Node::Rect {
            rect: title_rect,
            fill: Paint::Solid { color: title_fill },
            stroke: Some(Stroke::inside(
                1.0,
                Paint::Solid {
                    color: if pane.focused {
                        colors.border
                    } else {
                        rgba_with_alpha(colors.border, 145)
                    },
                },
            )),
            corners: Corners::default(),
        },
    ));
    layers.push(Layer::new(
        native_pane_title_gutter_label(idx),
        Node::Rect {
            rect: title_rect,
            fill: Paint::Solid { color: title_fill },
            stroke: None,
            corners: Corners::default(),
        },
    ));
    if pane.focused {
        layers.push(Layer::new(
            native_pane_focus_accent_rail_label(idx),
            Node::Rect {
                rect: PxRect::new(0.0, 0.0, 4.0, rect.height),
                fill: Paint::Solid {
                    color: colors.border,
                },
                stroke: None,
                corners: Corners::uniform(4.0),
            },
        ));
    }
    layers.push(Layer::new(
        native_pane_kittui_border_label(idx),
        Node::Rect {
            rect,
            fill: Paint::Solid {
                color: Rgba::rgba(0, 0, 0, 0),
            },
            stroke: Some(Stroke::inside(2.0, Paint::Solid { color: border })),
            corners: Corners::uniform(5.0),
        },
    ));
    layers.push(Layer::new(
        native_pane_status_chip_label(idx, &pane.status),
        Node::Rect {
            rect: native_pane_status_chip_rect(cols, rect.width, cell_w, chip_h),
            fill: Paint::Solid {
                color: rgba_with_alpha(colors.border, if pane.focused { 70 } else { 42 }),
            },
            stroke: Some(Stroke::inside(
                1.0,
                Paint::Solid {
                    color: rgba_with_alpha(colors.border, if pane.focused { 220 } else { 120 }),
                },
            )),
            corners: Corners::uniform(5.0),
        },
    ));
    if pane.focused {
        layers.push(Layer::new(
            native_pane_focus_ring_label(idx),
            Node::Rect {
                rect,
                fill: Paint::Solid {
                    color: Rgba::rgba(0, 0, 0, 0),
                },
                stroke: Some(Stroke::inside(
                    4.0,
                    Paint::Solid {
                        color: colors.border,
                    },
                )),
                corners: Corners::uniform(7.0),
            },
        ));
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size,
        layers,
        animation: None,
    }
}

fn write_native_graphical_pane_text_overlays<W: Write>(
    out: &mut W,
    view: &NativeShellView,
    cols: u16,
    rows: u16,
) -> Result<()> {
    let colors = native_glass_chrome_colors();
    let focused_style = ansi_fg_bg(colors.fg, colors.border);
    let unfocused_style = ansi_fg_bg(colors.fg, rgba_with_alpha(colors.fill, 180));
    let status_style = ansi_fg_bg(colors.fg, rgba_with_alpha(colors.border, 180));
    for pane in &view.panes {
        let Some(row) = terminal_visible_row_opt(pane.y, rows) else {
            continue;
        };
        let Some(width) = terminal_visible_width(pane.x, pane.cols, cols) else {
            continue;
        };
        let status_cols = native_graphical_pane_status_overlay_cols(&pane.status, width);
        let title_cols = width.saturating_sub(status_cols);
        if title_cols > 0 {
            let title = clip_and_pad(&pane.text, title_cols);
            let style = if pane.focused {
                focused_style.as_str()
            } else {
                unfocused_style.as_str()
            };
            write!(
                out,
                "\x1b[{};{}H{}{}\x1b[0m",
                row + 1,
                pane.x + 1,
                style,
                title
            )?;
        }
        if status_cols > 0 {
            let status_x = pane.x.saturating_add(title_cols as u16);
            let status = clip_and_pad(&pane.status, status_cols);
            write!(
                out,
                "\x1b[{};{}H{}{}\x1b[0m",
                row + 1,
                status_x + 1,
                status_style,
                status
            )?;
        }
    }
    Ok(())
}

fn native_graphical_pane_status_overlay_cols(status: &str, title_row_width: usize) -> usize {
    if title_row_width < 16 || status.is_empty() {
        return 0;
    }
    let desired = status.chars().count().saturating_add(2);
    desired.min(title_row_width / 2).max(8)
}

fn write_native_graphical_top_bar_text_overlay<W: Write>(
    out: &mut W,
    view: &NativeShellView,
    cols: u16,
) -> Result<()> {
    let row = view.top_bar.row + 1;
    let model = native_top_bar_model_from_view(view);
    let clock = model
        .time
        .strip_suffix(" UTC")
        .unwrap_or("00:00")
        .to_string();
    let palette = native_top_bar_overlay_palette(native_glass_chrome_colors());
    write!(out, "{}", native_graphical_top_bar_overlay_clear(row))?;
    let mut workspace_cols = 0u16;
    for workspace in native_graphical_top_bar_overlay_labels(&model, cols) {
        let active = model.workspace.trim() == workspace;
        let (fg, bg) = if active {
            (palette.active_fg, palette.active_bg)
        } else {
            (palette.inactive_fg, palette.inactive_bg)
        };
        let Some(label) = native_graphical_top_bar_fit_label(&workspace, cols, workspace_cols)
        else {
            break;
        };
        let label_cols = native_top_bar_overlay_text_cols(&label, 1);
        write!(
            out,
            "\x1b[1m{}{}{}\x1b[0m ",
            ansi_fg(fg),
            ansi_bg(bg),
            label
        )?;
        workspace_cols = workspace_cols.saturating_add(label_cols);
    }
    let clock_text = native_graphical_top_bar_clock_text(&clock);
    if let Some(clock_col) = native_graphical_top_bar_clock_col(
        cols,
        workspace_cols,
        native_top_bar_overlay_text_cols(&clock_text, 0),
    ) {
        write!(
            out,
            "\x1b[{};{}H\x1b[1m{}{}{}\x1b[0m",
            row,
            clock_col,
            ansi_fg(palette.clock_fg),
            ansi_bg(palette.clock_bg),
            clock_text
        )?;
    }
    Ok(())
}

fn native_top_bar_overlay_text_cols(text: &str, padding_cols: u16) -> u16 {
    let count = text.chars().take(u16::MAX as usize).count() as u16;
    count.saturating_add(padding_cols)
}

fn native_graphical_top_bar_overlay_clear(row: u16) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("\x1b[0m\x1b[;1H\x1b[K".len() + 5);
    out.push_str("\x1b[0m\x1b[");
    let _ = write!(out, "{row}");
    out.push_str(";1H\x1b[K");
    out
}

fn native_graphical_top_bar_overlay_labels(model: &BarModel, cols: u16) -> Vec<String> {
    let labels = model.workspace_chip_labels();
    let total_cols = workspace_chip_total_cols(&labels);
    if total_cols > cols {
        return model.workspace_chip_labels_active_first();
    }
    labels
}

fn native_graphical_top_bar_fit_label(label: &str, cols: u16, used_cols: u16) -> Option<String> {
    let remaining = cols.saturating_sub(used_cols);
    if remaining == 0 {
        return None;
    }
    let max_chip_cols = remaining.saturating_sub(1) as usize;
    if max_chip_cols < 3 {
        return None;
    }
    let label_width = max_chip_cols.saturating_sub(2);
    if label_width == 0 {
        return None;
    }
    let mut out = String::with_capacity(label_width.saturating_add(2));
    out.push(' ');
    if native_top_bar_label_fits_cells(label, label_width) {
        out.push_str(label);
    } else {
        out.extend(label.chars().take(label_width));
    }
    out.push(' ');
    Some(out)
}

fn native_top_bar_label_fits_cells(label: &str, max_label_cols: usize) -> bool {
    label.chars().take(max_label_cols.saturating_add(1)).count() <= max_label_cols
}

#[cfg(test)]
fn native_graphical_top_bar_label_fits(cols: u16, used_cols: u16, label_cols: u16) -> bool {
    label_cols > 0 && used_cols.saturating_add(label_cols) <= cols
}

fn native_graphical_top_bar_clock_text(clock: &str) -> String {
    let mut out = String::with_capacity(clock.len().saturating_add(2));
    out.push(' ');
    out.push_str(clock);
    out.push(' ');
    out
}

fn native_graphical_top_bar_clock_col(
    cols: u16,
    workspace_cols: u16,
    clock_cols: u16,
) -> Option<u16> {
    let min_gap = 1;
    (workspace_cols
        .saturating_add(min_gap)
        .saturating_add(clock_cols)
        <= cols)
        .then(|| cols.saturating_sub(clock_cols).saturating_add(1).max(1))
}

#[cfg(test)]
fn native_graphical_top_bar_text_palette(colors: &InlineChipColors, active: bool) -> (Rgba, Rgba) {
    if active {
        (opaque_rgb(colors.fill), opaque_rgb(colors.highlight))
    } else {
        (opaque_rgb(colors.fg), rgba_with_alpha(colors.fill, 235))
    }
}

#[cfg(test)]
fn native_graphical_top_bar_clock_palette(colors: &InlineChipColors) -> (Rgba, Rgba) {
    (opaque_rgb(colors.fg), rgba_with_alpha(colors.fill, 240))
}

#[cfg(test)]
fn opaque_rgb(color: Rgba) -> Rgba {
    Rgba(color.0, color.1, color.2, 255)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativeTopBarOverlayPalette {
    active_fg: Rgba,
    active_bg: Rgba,
    inactive_fg: Rgba,
    inactive_bg: Rgba,
    clock_fg: Rgba,
    clock_bg: Rgba,
}

fn native_top_bar_overlay_palette(colors: InlineChipColors) -> NativeTopBarOverlayPalette {
    let active_bg = rgba_with_alpha(colors.highlight, colors.highlight.3.max(235));
    let inactive_bg = rgba_with_alpha(colors.fill, colors.fill.3.max(210));
    let clock_bg = rgba_with_alpha(colors.fill, colors.fill.3.max(240));
    NativeTopBarOverlayPalette {
        active_fg: high_contrast_text_for(active_bg),
        active_bg,
        inactive_fg: high_contrast_text_for(inactive_bg),
        inactive_bg,
        clock_fg: high_contrast_text_for(clock_bg),
        clock_bg,
    }
}

fn high_contrast_text_for(bg: Rgba) -> Rgba {
    let luminance = (u32::from(bg.0) * 299 + u32::from(bg.1) * 587 + u32::from(bg.2) * 114) / 1000;
    if luminance > 150 {
        Rgba(0x2e, 0x34, 0x40, 255)
    } else {
        Rgba(0xec, 0xef, 0xf4, 255)
    }
}

fn ansi_fg(color: Rgba) -> String {
    let mut out = String::with_capacity(20);
    let _ = write!(out, "\x1b[38;2;{};{};{}m", color.0, color.1, color.2);
    out
}

fn ansi_bg(color: Rgba) -> String {
    let mut out = String::with_capacity(20);
    let _ = write!(out, "\x1b[48;2;{};{};{}m", color.0, color.1, color.2);
    out
}

fn native_shell_view_affordance_chrome_key(view: &NativeShellView, cols: u16, rows: u16) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cols.hash(&mut hasher);
    rows.hash(&mut hasher);
    view.top_bar.row.hash(&mut hasher);
    view.top_bar.text.hash(&mut hasher);
    view.footer.row.hash(&mut hasher);
    view.footer.text.hash(&mut hasher);
    view.help_overlay.hash(&mut hasher);
    view.panes.len().hash(&mut hasher);
    for pane in &view.panes {
        pane.x.hash(&mut hasher);
        pane.y.hash(&mut hasher);
        pane.cols.hash(&mut hasher);
        pane.rows.hash(&mut hasher);
        pane.focused.hash(&mut hasher);
        pane.text.hash(&mut hasher);
        pane.cache_key.hash(&mut hasher);
        pane.status.hash(&mut hasher);
        pane.app_x.hash(&mut hasher);
        pane.app_y.hash(&mut hasher);
        pane.app_cols.hash(&mut hasher);
        pane.app_rows.hash(&mut hasher);
    }
    hasher.finish()
}

fn write_native_shell_affordance_chrome<W: Write>(
    out: &mut W,
    runtime: &Runtime,
    view: &NativeShellView,
    cols: u16,
    rows: u16,
    last_keys: &mut HashMap<String, NativeChromePlacementMemo>,
) -> Result<()> {
    let scenes = render_native_shell_view_affordance_scenes(view, native_cell_size(), cols, rows);
    let current_ids = scenes
        .iter()
        .map(|chrome| chrome.id.clone())
        .collect::<HashSet<_>>();
    let retired = last_keys
        .keys()
        .filter(|id| !current_ids.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    for id in retired {
        if let Some(memo) = last_keys.remove(&id) {
            out.write_all(runtime.unplace(memo.image_id).as_bytes())?;
        }
    }

    for chrome in scenes {
        let key = native_shell_chrome_scene_key(&chrome);
        if last_keys.get(&chrome.id).map(|memo| memo.key.as_str()) == Some(key.as_str()) {
            continue;
        }
        let placement = CellRect::new(
            chrome.x,
            chrome.y,
            chrome.scene.footprint.cols,
            chrome.scene.footprint.rows,
        );
        let image_id = chrome.scene.id().kitty_image_id();
        let placement_options = kittui_kitty::PlacementOptions::stable_absolute(image_id)
            .with_z_index(native_live_chrome_z_index());
        let p = runtime.place_at_with_options(&chrome.scene, placement, &placement_options)?;
        out.write_all(p.upload.as_bytes())?;
        out.write_all(p.placement.as_bytes())?;
        out.write_all(p.embed.as_bytes())?;
        last_keys.insert(chrome.id, NativeChromePlacementMemo { key, image_id });
    }
    Ok(())
}

fn native_shell_chrome_scene_key(chrome: &NativeShellChromeScene) -> String {
    let scene_id = chrome.scene.id().0;
    let label_hash = native_shell_chrome_scene_label_hash(&chrome.scene);
    let mut key =
        String::with_capacity(chrome.id.len() + scene_id.len() + "@,:x:::".len() + 20 * 5);
    key.push_str(&chrome.id);
    key.push('@');
    let _ = write!(key, "{}", chrome.x);
    key.push(',');
    let _ = write!(key, "{}", chrome.y);
    key.push(':');
    let _ = write!(key, "{}", chrome.scene.footprint.cols);
    key.push('x');
    let _ = write!(key, "{}", chrome.scene.footprint.rows);
    key.push(':');
    let _ = write!(key, "{}", chrome.scene.cell_size.width_px);
    key.push(':');
    key.push_str(&scene_id);
    key.push(':');
    let _ = write!(key, "{label_hash}");
    key
}

fn native_shell_chrome_scene_label_hash(scene: &Scene) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for layer in &scene.layers {
        layer.label.hash(&mut hasher);
    }
    hasher.finish()
}

fn clip_and_pad(text: &str, width: usize) -> String {
    let mut count = 0usize;
    let mut clipped = String::with_capacity(width);
    for ch in text.chars().take(width) {
        clipped.push(ch);
        count += 1;
    }
    if count < width {
        clipped.extend(std::iter::repeat(' ').take(width - count));
    }
    clipped
}

fn native_pane_title_key_from_text(text: &str, layout: NativePaneLayout, focused: bool) -> String {
    let mut out = String::with_capacity(text.len().saturating_add(32));
    let _ = write!(
        out,
        "{},{},{}x{}:{}:",
        layout.x, layout.y, layout.cols, layout.rows, focused
    );
    out.push_str(text);
    out
}

fn native_help_overlay_ansi_width(cols: u16) -> Option<usize> {
    (cols >= 5).then_some(cols.saturating_sub(4) as usize)
}

fn write_native_help_overlay<W: Write>(out: &mut W, cols: u16, rows: u16) -> Result<()> {
    let Some(help_width) = native_help_overlay_ansi_width(cols) else {
        return Ok(());
    };
    for (idx, line) in native_help_overlay_lines().iter().enumerate() {
        let row = 2 + idx as u16;
        if row >= rows {
            break;
        }
        write!(
            out,
            "\x1b[{};3H\x1b[7m {} \x1b[0m",
            row + 1,
            clip_and_pad(line, help_width)
        )?;
    }
    Ok(())
}

fn write_native_pane_chrome<W: Write>(
    out: &mut W,
    chrome: &NativePaneChrome,
    cols: u16,
    rows: u16,
) -> Result<()> {
    let Some(row) = terminal_visible_row_opt(chrome.y, rows) else {
        return Ok(());
    };
    let Some(width) = terminal_visible_width(chrome.x, chrome.cols, cols) else {
        return Ok(());
    };
    let style = if chrome.focused { "\x1b[7m" } else { "\x1b[2m" };
    write!(
        out,
        "\x1b[{};{}H{}{}\x1b[0m",
        row + 1,
        chrome.x + 1,
        style,
        clip_and_pad(&chrome.text, width)
    )?;
    Ok(())
}

#[cfg(test)]
mod native_pane_tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[derive(Default)]
    struct CountingWriter {
        writes: usize,
        flushes: usize,
        bytes: Vec<u8>,
    }

    impl Write for CountingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn native_frame_write_batch_coalesces_frame_segments_into_one_flush() {
        let mut batch = NativeFrameWriteBatch::default();
        assert!(batch.is_empty());
        batch.write_all(b"host-seq").unwrap();
        batch.write_all(b"upload").unwrap();
        batch.write_all(b"place").unwrap();
        batch.write_all(b"chrome").unwrap();
        assert_eq!(batch.as_bytes(), b"host-sequploadplacechrome");

        let mut writer = CountingWriter::default();
        assert!(batch.write_to(&mut writer).unwrap());
        assert_eq!(writer.writes, 1);
        assert_eq!(writer.flushes, 1);
        assert_eq!(writer.bytes, b"host-sequploadplacechrome");

        let mut empty_writer = CountingWriter::default();
        assert!(!NativeFrameWriteBatch::default()
            .write_to(&mut empty_writer)
            .unwrap());
        assert_eq!(empty_writer.writes, 0);
        assert_eq!(empty_writer.flushes, 0);
    }

    #[test]
    fn native_chrome_colors_apply_kittwm_config_without_disk_reload() {
        let mut colors = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Glass);
        let mut config = KittwmConfig::default();
        config.background.color = "#112233".to_string();
        config.background.opacity = 0.5;
        config.colorscheme.fg = "#abcdef".to_string();
        config.colorscheme.colors[4] = "#445566".to_string();
        apply_kittwm_config_to_chrome_colors(&mut colors, &config);
        assert_eq!(colors.fill, Rgba(0x11, 0x22, 0x33, 128));
        assert_eq!(colors.border, Rgba(0xab, 0xcd, 0xef, 255));
        assert_eq!(colors.fg, Rgba(0xab, 0xcd, 0xef, 255));
        assert_eq!(colors.highlight, Rgba(0x44, 0x55, 0x66, 107));
    }

    #[test]
    fn native_layout_publish_decision_skips_unchanged_label() {
        let mut last = "columns".to_string();
        assert!(!should_publish_native_layout(&mut last, "columns"));
        assert_eq!(last, "columns");
        assert!(should_publish_native_layout(&mut last, "rows"));
        assert_eq!(last, "rows");
        assert!(!should_publish_native_layout(&mut last, "rows"));
    }

    #[test]
    fn raw_compositor_idle_pacing_uses_native_idle_policy() {
        let active = Duration::from_millis(16);
        let idle = Duration::from_millis(100);
        assert_eq!(raw_compositor_current_frame_target(active, idle, 0), active);
        assert_eq!(raw_compositor_current_frame_target(active, idle, 1), active);
        assert_eq!(raw_compositor_current_frame_target(active, idle, 2), idle);
    }

    #[test]
    fn native_pane_statuses_changed_detects_stable_snapshots() {
        let status = crate::daemon::NativePaneStatus {
            window: "native-1".to_string(),
            title: "shell".to_string(),
            focused: true,
            weight: 1,
            stack_index: Some(0),
            stack_top: Some(true),
            floating_dx: Some(0),
            floating_dy: Some(0),
            floating_moved: None,
            title_draggable: Some(false),
            title_drag_kind: None,
            title_drag_col: None,
            title_drag_row: None,
            title_drag_active: None,
            pid: Some(42),
            command: Some("sh".to_string()),
            x: Some(0),
            y: Some(1),
            cols: Some(80),
            rows: Some(23),
            app_x: Some(1),
            app_y: Some(2),
            app_cols: Some(78),
            cursor_col: Some(0),
            cursor_row: Some(0),
            cursor_visible: Some(true),
            bracketed_paste: Some(false),
            application_cursor_keys: Some(false),
            mouse_reporting: Some(false),
            mouse_button_motion: Some(false),
            mouse_all_motion: Some(false),
            mouse_sgr: Some(false),
            dirty_frame: None,
            text_snapshot: None,
            scrollback_snapshot: None,
            app_rows: Some(21),
        };
        assert!(!native_pane_statuses_changed(
            std::slice::from_ref(&status),
            std::slice::from_ref(&status),
        ));
        let mut changed = status.clone();
        changed.focused = false;
        assert!(native_pane_statuses_changed(&[status], &[changed]));
    }

    #[test]
    fn ansi_top_bar_is_disabled_when_graphical_chrome_is_active() {
        assert!(!should_write_ansi_top_bar(true, true, "new", "old"));
        assert!(!should_write_ansi_top_bar(true, false, "new", "old"));
        assert!(should_write_ansi_top_bar(false, true, "same", "same"));
        assert!(should_write_ansi_top_bar(false, false, "new", "old"));
        assert!(!should_write_ansi_top_bar(false, false, "same", "same"));
    }

    #[test]
    fn native_top_bar_overlay_palette_uses_configured_colors_with_contrast() {
        let colors = InlineChipColors {
            fill: Rgba(0x11, 0x22, 0x33, 210),
            fg: Rgba(0xdd, 0xee, 0xff, 255),
            border: Rgba(0xaa, 0xbb, 0xcc, 255),
            highlight: Rgba(0xee, 0xdd, 0xaa, 235),
        };
        let palette = native_top_bar_overlay_palette(colors);
        assert_eq!(palette.active_bg, Rgba(0xee, 0xdd, 0xaa, 235));
        assert_eq!(palette.inactive_bg, Rgba(0x11, 0x22, 0x33, 210));
        assert_eq!(palette.clock_bg, Rgba(0x11, 0x22, 0x33, 240));
        assert_eq!(palette.active_fg, Rgba(0x2e, 0x34, 0x40, 255));
        assert_eq!(palette.inactive_fg, Rgba(0xec, 0xef, 0xf4, 255));
        assert_eq!(palette.clock_fg, Rgba(0xec, 0xef, 0xf4, 255));
    }

    #[test]
    fn raw_compositor_footer_launch_note_builds_directly() {
        let note = raw_compositor_footer_launch_note(Some(12345));
        assert_eq!(note, " — last launch pid=12345");
        assert!(note.capacity() >= note.len());
        assert!(raw_compositor_footer_launch_note(None).is_empty());
    }

    #[test]
    fn raw_compositor_footer_keymap_note_builds_directly() {
        let note = raw_compositor_footer_keymap_note(false, Some("launch"));
        assert_eq!(note, " — action=launch");
        assert!(note.capacity() >= note.len());
        assert_eq!(
            raw_compositor_footer_keymap_note(true, Some("ignored")),
            " — keymap prefix"
        );
        assert!(raw_compositor_footer_keymap_note(false, None).is_empty());
    }

    #[test]
    fn raw_footer_test_long_action_builds_directly() {
        let action = raw_footer_test_long_action(4);
        assert_eq!(action, " — action=xxxx");
        assert!(action.capacity() >= action.len());
    }

    #[test]
    fn raw_compositor_footer_key_reuses_precomputed_state_labels() {
        let key = raw_compositor_footer_key(
            24,
            "dev",
            "split",
            "columns",
            "cfg",
            "focus",
            "swap",
            "normal",
            2,
            " — last launch pid=12345",
            " — action=launch",
            "q to quit",
        );
        assert_eq!(
            key,
            "row=24;ws=dev;panes=split;layout=columns;cfg=cfg;focus=focus;swap=swap;mode=normal;windows=2;launch= — last launch pid=12345;keymap= — action=launch;quit=q to quit"
        );

        let long_action = raw_footer_test_long_action(10_000);
        let bounded = raw_compositor_footer_key(
            24,
            "dev",
            "split",
            "columns",
            "cfg",
            "focus",
            "swap",
            "normal",
            2,
            "",
            &long_action,
            "q to quit",
        );
        assert!(bounded.contains('…'), "{bounded}");
        assert!(!bounded.contains(&"x".repeat(128)), "{bounded}");
    }

    fn raw_footer_test_long_action(len: usize) -> String {
        let mut action = String::with_capacity(" — action=".len() + len);
        action.push_str(" — action=");
        action.extend(std::iter::repeat_n('x', len));
        action
    }

    #[test]
    fn raw_compositor_footer_fps_label_builds_directly() {
        let fps = raw_compositor_footer_fps_label(59.6);
        assert_eq!(fps, "60");
        assert!(fps.capacity() >= fps.len());
    }

    #[test]
    fn raw_compositor_footer_text_clips_to_terminal_width() {
        let text = raw_compositor_footer_text(
            123,
            "dev",
            "split",
            "columns",
            "cfg",
            "focus",
            "swap",
            "normal",
            2,
            60.0,
            120.0,
            120,
            " — last launch pid=12345",
            " — action=very-long-action-name",
            "Ctrl-] exit",
            "/tmp/a/very/long/kittui-wm.log",
            24,
        );
        assert_eq!(text.chars().count(), 24, "{text:?}");
        assert!(text.capacity() >= 24);
        assert!(text.starts_with("kittui-wm frame 123"), "{text:?}");
        let tiny = raw_compositor_footer_text(
            1, "w", "s", "l", "c", "f", "sw", "m", 0, 0.0, 0.0, 60, "", "", "q", "log", 1,
        );
        assert_eq!(tiny.chars().count(), 1, "{tiny:?}");

        let huge = raw_compositor_footer_text(
            1,
            &"workspace-".repeat(10_000),
            "s",
            "l",
            "c",
            "f",
            "sw",
            "m",
            0,
            0.0,
            0.0,
            60,
            "",
            "",
            "q",
            &"log-path-".repeat(10_000),
            32,
        );
        assert_eq!(huge.chars().count(), 32, "{huge:?}");
        assert!(huge.ends_with('…'), "{huge:?}");
        assert!(!huge.contains(&"workspace-".repeat(4)), "{huge:?}");
    }

    #[test]
    fn run_loop_entry_log_line_builds_directly() {
        let line = run_loop_entry_log_line(60, false, true);
        assert_eq!(
            line,
            "run_loop: enter fps=60 launch_on_f12=false launcher_overlay=true"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn ctrl_c_debounce_log_line_builds_directly() {
        let line = ctrl_c_debounce_log_line(3);
        assert_eq!(line, "ctrl-c press #3 within debounce window");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn keymap_prefix_entered_log_line_builds_directly() {
        let line = keymap_prefix_entered_log_line(&"C-a");
        assert_eq!(line, "keymap prefix entered: C-a");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn keymap_action_log_line_builds_directly() {
        let line = keymap_action_log_line("C-a Enter", "launch");
        assert_eq!(line, "keymap action: C-a Enter -> launch");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn keymap_unbound_prefix_log_line_builds_directly() {
        let line = keymap_unbound_prefix_log_line(&"C-x");
        assert_eq!(line, "keymap unbound prefix chord: C-x");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn keymap_unbound_status_line_builds_directly() {
        let line = keymap_unbound_status_line(&"C-x");
        assert_eq!(line, "unbound C-x");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn char_event_log_line_builds_directly() {
        let line = char_event_log_line('x');
        assert_eq!(line, "char event: 'x'");
        assert!(line.capacity() >= line.len());
        let escaped = char_event_log_line('\n');
        assert_eq!(escaped, "char event: '\\n'");
    }

    #[test]
    fn key_event_log_line_builds_directly() {
        let line = key_event_log_line(&Key::Enter);
        assert_eq!(line, "key event: Enter");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn raw_frame_count_log_line_builds_directly() {
        let line = raw_frame_count_log_line(30, 2);
        assert_eq!(line, "frame 30: 2 raw frames");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn frame_budget_blown_log_line_builds_directly() {
        let line = frame_budget_blown_log_line(30, 42, 16);
        assert_eq!(line, "frame 30 budget blown: 42 ms (target 16 ms)");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn stdin_read_log_line_builds_directly() {
        let line = stdin_read_log_line(4, &[0, 1, 0xab, 0xff]);
        assert_eq!(line, "stdin read 4 bytes: [00, 01, ab, ff]");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn compose_error_log_line_builds_directly() {
        let line = compose_error_log_line("capture denied");
        assert_eq!(line, "compose err: capture denied");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn keymap_reload_failed_log_line_builds_directly() {
        let err = anyhow!("missing keymap");
        let line = keymap_reload_failed_log_line(&err);
        assert_eq!(
            line,
            "keymap reload failed, keeping previous keymap: missing keymap"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn config_action_log_line_builds_directly() {
        let line = config_action_log_line("config.reload.ok#1");
        assert_eq!(line, "config action: config.reload.ok#1");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn split_action_log_line_builds_directly() {
        let line = split_action_log_line("split.vertical -> spawned#1");
        assert_eq!(line, "split action: split.vertical -> spawned#1");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn workspace_action_log_line_builds_directly() {
        let line = workspace_action_log_line("workspace.next -> 2/3");
        assert_eq!(line, "workspace action: workspace.next -> 2/3");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn action_result_status_lines_build_directly() {
        let window = action_window_status_line("focus.right -> right#1", 7);
        assert_eq!(window, "focus.right -> right#1 window=7");
        assert!(window.capacity() >= window.len());
        let missing = action_no_window_status_line("focus.right -> right#1");
        assert_eq!(missing, "focus.right -> right#1 window=-");
        assert_eq!(missing.capacity(), missing.len());
        let err = anyhow!("no focus");
        let error = action_error_status_line("focus.right -> right#1", &err);
        assert_eq!(error, "focus.right -> right#1 error=no focus");
        assert!(error.capacity() >= error.len());
    }

    #[test]
    fn action_window_mode_status_line_builds_directly() {
        let line = action_window_mode_status_line("toggle.float -> floating#1", 7, "floating");
        assert_eq!(line, "toggle.float -> floating#1 window=7 mode=floating");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn action_window_fullscreen_status_line_builds_directly() {
        let line = action_window_fullscreen_status_line("toggle.fullscreen -> on#1", 7, true);
        assert_eq!(line, "toggle.fullscreen -> on#1 window=7 fullscreen=true");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn action_windows_count_status_line_builds_directly() {
        let line = action_windows_count_status_line("layout.balance -> axis=x balanced#1", 3);
        assert_eq!(line, "layout.balance -> axis=x balanced#1 windows=3");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn focus_action_log_line_builds_directly() {
        let line = focus_action_log_line("focus.right -> right#1 window=native-2");
        assert_eq!(line, "focus action: focus.right -> right#1 window=native-2");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn swap_action_log_line_builds_directly() {
        let line = swap_action_log_line("swap.left -> left#1 window=native-1");
        assert_eq!(line, "swap action: swap.left -> left#1 window=native-1");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn toggle_action_log_line_builds_directly() {
        let line = toggle_action_log_line("toggle.float -> floating window=native-1");
        assert_eq!(
            line,
            "toggle action: toggle.float -> floating window=native-1"
        );
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn layout_action_log_line_builds_directly() {
        let line = layout_action_log_line("layout.balance -> axis=x balanced#1");
        assert_eq!(line, "layout action: layout.balance -> axis=x balanced#1");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn picker_select_status_line_builds_directly() {
        let line = picker_select_status_line("window native-1");
        assert_eq!(line, "picker.select window native-1");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn picker_selected_log_line_builds_directly() {
        let line = picker_selected_log_line("window native-1");
        assert_eq!(line, "picker selected window native-1");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn launcher_overlay_launch_failed_log_line_builds_directly() {
        let err = anyhow!("spawn denied");
        let line = launcher_overlay_launch_failed_log_line(&err);
        assert_eq!(line, "launcher overlay launch failed: spawn denied");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn launcher_overlay_opened_log_line_builds_directly() {
        let line = launcher_overlay_opened_log_line("term");
        assert_eq!(line, "launcher overlay opened query=\"term\"");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn split_launcher_overlay_opened_log_line_builds_directly() {
        let line = split_launcher_overlay_opened_log_line("term");
        assert_eq!(line, "split opened launcher overlay query=\"term\"");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn launcher_action_status_line_builds_directly() {
        let selection = LauncherSelection {
            kind: LauncherKind::Shell,
            command: "echo hi".to_string(),
        };
        let line = launcher_action_status_line(&selection);
        assert_eq!(line, "launcher.launch shell:echo hi");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn launcher_error_status_line_builds_directly() {
        let err = anyhow!("spawn denied");
        let line = launcher_error_status_line(&err);
        assert_eq!(line, "launcher.error spawn denied");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn launcher_overlay_selected_log_line_builds_directly() {
        let line = launcher_overlay_selected_log_line(LauncherKind::Path, "htop", 99);
        assert_eq!(
            line,
            "launcher overlay selected Path \"htop\" spawned pid=99"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn keymap_launcher_selected_log_line_builds_directly() {
        let line = keymap_launcher_selected_log_line(LauncherKind::Shell, "echo hi", 42);
        assert_eq!(
            line,
            "keymap launcher selected Shell \"echo hi\" spawned pid=42"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn keymap_launcher_failed_log_line_builds_directly() {
        let err = anyhow!("spawn denied");
        let line = keymap_launcher_failed_log_line(&err);
        assert_eq!(line, "keymap launcher failed: spawn denied");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn split_launcher_selected_log_line_builds_directly() {
        let line = split_launcher_selected_log_line(LauncherKind::Path, "top", 77);
        assert_eq!(line, "split launcher selected Path \"top\" spawned pid=77");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn split_launcher_failed_log_line_builds_directly() {
        let err = anyhow!("spawn denied");
        let line = split_launcher_failed_log_line(&err);
        assert_eq!(line, "split launcher failed: spawn denied");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn launcher_f12_overlay_opened_log_line_builds_directly() {
        let line = launcher_f12_overlay_opened_log_line("term");
        assert_eq!(line, "launcher F12 opened overlay query=\"term\"");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn launcher_f12_selected_log_line_builds_directly() {
        let line = launcher_f12_selected_log_line(LauncherKind::MacOsApp, "Safari", 88);
        assert_eq!(
            line,
            "launcher F12 selected MacOsApp \"Safari\" spawned pid=88"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn launcher_f12_failed_log_line_builds_directly() {
        let err = anyhow!("spawn denied");
        let line = launcher_f12_failed_log_line(&err);
        assert_eq!(line, "launcher F12 failed: spawn denied");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn keymap_action_unimplemented_log_line_builds_directly() {
        let line = keymap_action_unimplemented_log_line(&"picker.open");
        assert_eq!(line, "keymap action not implemented yet: picker.open");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn loaded_keymap_log_line_builds_directly() {
        let line = loaded_keymap_log_line("/tmp/kittwm.toml");
        assert_eq!(line, "loaded keymap from /tmp/kittwm.toml");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn runtime_keymap_load_failed_log_line_builds_directly() {
        let err = anyhow!("missing keymap");
        let line = runtime_keymap_load_failed_log_line(&err);
        assert_eq!(
            line,
            "failed to load runtime keymap: missing keymap; using defaults"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn raw_compositor_footer_refresh_defaults_to_state_changes_only() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_FOOTER_REFRESH_FRAMES");
        assert_eq!(raw_compositor_footer_refresh_interval(), 0);
        assert!(should_write_compositor_footer("old", "new", 30, 0));
        assert!(!should_write_compositor_footer("same", "same", 30, 0));
        std::env::set_var("KITTWM_FOOTER_REFRESH_FRAMES", "120");
        assert_eq!(raw_compositor_footer_refresh_interval(), 120);
        assert!(should_write_compositor_footer("same", "same", 120, 120));
        std::env::remove_var("KITTWM_FOOTER_REFRESH_FRAMES");
    }

    #[test]
    fn raw_compositor_app_placement_is_stable_absolute_below_text() {
        let opts = raw_compositor_app_placement_options(42);
        assert_eq!(opts.placement_id, Some(42));
        assert!(!opts.unicode_placeholder);
        assert_eq!(opts.z_index, raw_compositor_app_z_index());
        assert!(
            opts.z_index < 0,
            "raw app images must stay below ANSI chrome text"
        );
    }

    #[test]
    fn raw_compositor_error_repaint_skips_unchanged_panel() {
        let key = raw_compositor_error_key("capture denied", "/tmp/kittui-wm.log");
        assert!(should_write_raw_compositor_error(None, &key));
        assert!(!should_write_raw_compositor_error(Some(&key), &key));
        let changed = raw_compositor_error_key("backend died", "/tmp/kittui-wm.log");
        assert!(should_write_raw_compositor_error(Some(&key), &changed));
        assert!(should_clear_raw_error_screen(Some(&key)));
        assert!(!should_clear_raw_error_screen(None));
    }

    #[test]
    fn raw_compositor_error_test_expected_key_builds_directly() {
        let key = raw_compositor_error_test_expected_key("error", "/tmp/log");
        assert_eq!(key, "error\n/tmp/log");
        assert!(key.capacity() >= key.len());
    }

    #[test]
    fn raw_compositor_error_text_and_key_are_bounded() {
        let huge_message = "capture backend failed: ".to_string() + &"x".repeat(10_000);
        let huge_log = "/tmp/".to_string() + &"kittui-wm/".repeat(10_000);
        let text = raw_compositor_error_text(&huge_message);
        let log = raw_compositor_error_log_path(&huge_log);
        assert_eq!(text.chars().count(), RAW_COMPOSITOR_ERROR_MESSAGE_MAX_CHARS);
        assert!(text.ends_with('…'), "{text}");
        assert_eq!(log.chars().count(), RAW_COMPOSITOR_ERROR_LOG_PATH_MAX_CHARS);
        assert!(log.ends_with('…'), "{log}");
        let key = raw_compositor_error_key(&huge_message, &huge_log);
        assert!(key.len() < 512, "{}", key.len());
        assert!(!key.contains(&"x".repeat(512)), "{}", key.len());
        assert_eq!(key, raw_compositor_error_test_expected_key(&text, &log));
        assert_eq!(key.capacity(), key.len());
    }

    fn raw_compositor_error_test_expected_key(text: &str, log: &str) -> String {
        let mut key = String::with_capacity(text.len() + 1 + log.len());
        key.push_str(text);
        key.push('\n');
        key.push_str(log);
        key
    }

    #[test]
    fn native_top_bar_time_from_text_preserves_rendered_hh_mm_clock() {
        assert_eq!(
            native_top_bar_time_from_text("|[1]| 2 | 3 |                  12:34"),
            "12:34 UTC"
        );
        assert_eq!(
            native_top_bar_time_from_text("kittui-bar ws:1 active 12:34 UTC"),
            "12:34 UTC"
        );
        assert_eq!(native_top_bar_time_from_text("no clock here"), "00:00 UTC");
        assert_eq!(
            native_top_bar_time_from_text("broken 12:34 PST"),
            "00:00 UTC"
        );
        assert_eq!(native_top_bar_time_from_text("UTC"), "00:00 UTC");
    }

    #[test]
    fn graphical_top_bar_overlay_ansi_helpers_build_directly() {
        let fg = ansi_fg(Rgba(1, 2, 3, 255));
        assert_eq!(fg, "\x1b[38;2;1;2;3m");
        assert!(fg.capacity() >= 20);
        let bg = ansi_bg(Rgba(4, 5, 6, 255));
        assert_eq!(bg, "\x1b[48;2;4;5;6m");
        assert!(bg.capacity() >= 20);
    }

    #[test]
    fn graphical_top_bar_overlay_clears_row_before_rewrite() {
        assert_eq!(
            native_graphical_top_bar_overlay_clear(1),
            "\x1b[0m\x1b[1;1H\x1b[K"
        );
        let row_three = native_graphical_top_bar_overlay_clear(3);
        assert_eq!(row_three, "\x1b[0m\x1b[3;1H\x1b[K");
        assert!(row_three.capacity() >= row_three.len());
    }

    #[test]
    fn ansi_help_overlay_width_requires_left_margin() {
        assert_eq!(native_help_overlay_ansi_width(0), None);
        assert_eq!(native_help_overlay_ansi_width(4), None);
        assert_eq!(native_help_overlay_ansi_width(5), Some(1));
        assert_eq!(native_help_overlay_ansi_width(80), Some(76));
    }

    #[test]
    fn clip_and_pad_tracks_width_without_recounting_padding() {
        let padded = clip_and_pad("abc", 6);
        assert_eq!(padded, "abc   ");
        assert!(padded.capacity() >= 6);
        assert_eq!(clip_and_pad("abcdef", 3), "abc");
        assert_eq!(clip_and_pad("éx", 4), "éx  ");
        assert_eq!(clip_and_pad("anything", 0), "");
    }

    #[test]
    fn native_footer_test_huge_log_path_builds_directly() {
        let path = native_footer_test_huge_log_path(4);
        assert_eq!(path, "/tmp/xxxx");
        assert!(path.capacity() >= path.len());
    }

    #[test]
    fn native_status_pane_focus_combined_label_builds_directly() {
        let label = native_status_pane_focus_combined_label("native-1", "editor");
        assert_eq!(label, "native-1(editor)");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_footer_visible_text_clips_huge_log_paths_to_terminal_width() {
        assert_eq!(
            native_status_line_text(0, "columns", None, None, None, "/tmp/kittwm.log"),
            ""
        );
        let footer = native_status_line_text(
            1,
            "floating",
            Some("native-1"),
            Some("sh · pid:101 · frame:clean"),
            None,
            "/tmp/kittwm.log",
        );
        assert_eq!(
            footer,
            " mode:floating · panes:1 · focus:native-1 · state:sh · pid:101 · frame:clean · C-a ? help · C-a n/p focus · C-a t float · C-a f full · C-a wasd nudge · C-a {} stack · C-a r/R reset · C-a e split · C-a x close · Ctrl-] exit · log: /tmp/kittwm.log"
        );
        assert_eq!(footer.capacity(), footer.len());
        let drag_footer = native_status_line_text(
            1,
            "floating",
            Some("native-1"),
            Some("sh · pid:101 · frame:clean"),
            Some(NativeActiveDragLabel {
                kind: "move",
                window: "native-1",
            }),
            "/tmp/kittwm.log",
        );
        assert!(
            drag_footer.contains(" · focus:native-1 · state:sh · pid:101 · frame:clean · drag:move:native-1 · C-a ? help"),
            "{drag_footer}"
        );

        let huge_footer = native_status_line_text(
            1,
            "floating",
            Some("native-1"),
            Some("sh · pid:101 · frame:clean"),
            None,
            &native_footer_test_huge_log_path(10_000),
        );
        assert!(huge_footer.contains('…'), "{huge_footer:?}");
        assert!(
            !huge_footer.contains(&"x".repeat(128)),
            "{}",
            huge_footer.len()
        );
        let visible = native_footer_visible_text(&huge_footer, 24);
        assert_eq!(visible.chars().count(), 24);
        assert!(visible.starts_with(" mode:floating"), "{visible:?}");
        assert!(!visible.contains(&"x".repeat(128)), "{}", visible.len());
        let narrow = native_footer_visible_text(&footer, 16);
        assert_eq!(narrow, " mode:floating ·");
        let count_visible = native_footer_visible_text(&footer, 25);
        assert!(count_visible.contains("panes:1"), "{count_visible:?}");
        assert_eq!(native_footer_visible_text("short", 8), "short   ");

        let huge_state = native_status_line_text(
            1,
            "floating",
            Some("native-1"),
            Some(&"state-".repeat(10_000)),
            None,
            "/tmp/kittwm.log",
        );
        let state_segment = huge_state
            .split(NATIVE_STATUS_LINE_STATE_PREFIX)
            .nth(1)
            .unwrap()
            .split(NATIVE_STATUS_LINE_HINTS)
            .next()
            .unwrap();
        assert_eq!(state_segment.chars().count(), NATIVE_STATUS_STATE_MAX_CHARS);
        assert!(state_segment.ends_with('…'), "{state_segment}");
        assert!(
            !huge_state.contains(&"state-".repeat(128)),
            "{huge_state:?}"
        );

        let mut pane = dummy_native_pane("native-1", "sh", 1);
        pane.display_title = Some("editor".to_string());
        assert_eq!(native_status_pane_focus_label(&pane), "native-1(editor)");
        pane.display_title = Some("title-".repeat(20));
        let bounded = native_status_pane_focus_label(&pane);
        assert_eq!(bounded.chars().count(), NATIVE_STATUS_FOCUS_MAX_CHARS);
        assert!(bounded.ends_with('…'), "{bounded}");
        assert!(!bounded.contains(&"title-".repeat(8)), "{bounded}");
    }

    fn native_footer_test_huge_log_path(x_count: usize) -> String {
        let mut path = String::with_capacity("/tmp/".len() + x_count);
        path.push_str("/tmp/");
        path.extend(std::iter::repeat_n('x', x_count));
        path
    }

    #[test]
    fn text_overlay_inner_width_respects_narrow_terminal_columns() {
        assert_eq!(overlay_inner_width_for_cols(64, None), 64);
        assert_eq!(overlay_inner_width_for_cols(64, Some(20)), 17);
        assert_eq!(overlay_inner_width_for_cols(64, Some(8)), 5);
        assert_eq!(overlay_inner_width_for_cols(64, Some(2)), 1);
        assert_eq!(overlay_inner_width_for_cols(58, Some(120)), 58);
        assert_eq!(overlay_inner_width_for_sources(64, None, Some(20)), 17);
        assert_eq!(overlay_inner_width_for_sources(64, Some(30), Some(20)), 27);
        assert_eq!(overlay_inner_width_for_sources(64, Some(0), Some(20)), 17);
    }

    #[test]
    fn kittwm_browser_command_targets_native_browser_surface() {
        assert_eq!(
            kittwm_browser_target_from_command("kittwm-browser https://example.com"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            kittwm_browser_target_from_command("kittwm-browser 'https://example.com/a b'"),
            Some("https://example.com/a b".to_string())
        );
        assert_eq!(
            kittwm_browser_target_from_command("kittwm-browser 'https://example.com/it'\\''s'"),
            Some("https://example.com/it's".to_string())
        );
        assert_eq!(kittwm_browser_target_from_command("kittwm-browser"), None);
        assert_eq!(
            kittwm_browser_target_from_command("kittwm-browserish https://example.com"),
            None
        );
    }

    #[test]
    fn truncate_cells_uses_bounded_prefix_for_huge_fields() {
        let huge = "workspace-".repeat(10_000);
        let clipped = truncate_cells(&huge, 12);
        assert_eq!(clipped, "workspace-w…");
        assert_eq!(clipped.chars().count(), 12);
        assert!(clipped.capacity() >= 12);
        let short = truncate_cells("short", 12);
        assert_eq!(short, "short");
        assert!(short.capacity() >= "short".len());
        assert_eq!(truncate_cells("anything", 1), "…");
        assert_eq!(truncate_cells("anything", 0), "");
    }

    #[test]
    fn graphical_top_bar_overlay_text_cols_saturate_pathological_labels() {
        let long = "x".repeat(u16::MAX as usize + 32);
        assert_eq!(native_top_bar_overlay_text_cols(&long, 0), u16::MAX);
        assert_eq!(native_top_bar_overlay_text_cols(&long, 1), u16::MAX);
        assert_eq!(native_top_bar_overlay_text_cols("dev", 1), 4);
    }

    #[test]
    fn graphical_top_bar_overlay_labels_saturate_long_workspace_width() {
        let long = "x".repeat(u16::MAX as usize);
        let model = BarModel::new(long.clone(), 0, "-", true, std::time::UNIX_EPOCH);
        let labels = native_graphical_top_bar_overlay_labels(&model, 8);
        assert_eq!(labels.first(), Some(&long));
    }

    #[test]
    fn graphical_top_bar_overlay_labels_prioritize_active_when_constrained() {
        let model = BarModel::new("dev", 0, "-", true, std::time::UNIX_EPOCH);
        assert_eq!(
            native_graphical_top_bar_overlay_labels(&model, 80),
            vec!["dev"]
        );
        assert_eq!(
            native_graphical_top_bar_overlay_labels(&model, 8),
            vec!["dev"]
        );
        let numeric = BarModel::new("2", 0, "-", true, std::time::UNIX_EPOCH);
        assert_eq!(
            native_graphical_top_bar_overlay_labels(&numeric, 8),
            vec!["2"]
        );
    }

    #[test]
    fn graphical_top_bar_label_fit_prevents_wrapping() {
        assert!(native_graphical_top_bar_label_fits(12, 0, 4));
        assert!(native_graphical_top_bar_label_fits(12, 8, 4));
        assert!(!native_graphical_top_bar_label_fits(12, 9, 4));
        assert!(!native_graphical_top_bar_label_fits(12, 0, 0));
    }

    #[test]
    fn graphical_top_bar_fit_label_clips_long_custom_workspace() {
        assert_eq!(
            native_graphical_top_bar_fit_label("abcdef", 6, 0),
            Some(" abc ".to_string())
        );
        assert_eq!(native_graphical_top_bar_fit_label("abcdef", 2, 0), None);
        let fitted = native_graphical_top_bar_fit_label("dev", 12, 4).unwrap();
        assert_eq!(fitted, " dev ");
        assert!(fitted.capacity() >= 5);
        let long = "x".repeat(u16::MAX as usize);
        assert!(!native_top_bar_label_fits_cells(&long, 8));
        assert_eq!(
            native_graphical_top_bar_fit_label(&long, 12, 0),
            Some(" xxxxxxxxx ".to_string())
        );
    }

    #[test]
    fn graphical_top_bar_overlay_width_accounts_for_custom_workspace() {
        let model = BarModel::new("dev", 0, "-", true, std::time::UNIX_EPOCH);
        let labels = model.workspace_chip_labels();
        let workspace_cols = workspace_chip_total_cols(&labels);
        assert_eq!(labels, vec!["dev"]);
        assert_eq!(workspace_cols, 6);
        assert_eq!(
            native_graphical_top_bar_clock_col(26, workspace_cols, 7),
            Some(20)
        );
        assert_eq!(
            native_graphical_top_bar_clock_col(13, workspace_cols, 7),
            None
        );
    }

    #[test]
    fn graphical_top_bar_clock_col_avoids_workspace_overlap() {
        let clock = native_graphical_top_bar_clock_text("12:34");
        assert_eq!(clock, " 12:34 ");
        assert_eq!(clock.capacity(), clock.len());
        assert_eq!(native_graphical_top_bar_clock_col(24, 12, 7), Some(18));
        assert_eq!(native_graphical_top_bar_clock_col(20, 12, 7), Some(14));
        assert_eq!(native_graphical_top_bar_clock_col(19, 12, 7), None);
        assert_eq!(native_graphical_top_bar_clock_col(8, 12, 7), None);
    }

    #[test]
    fn graphical_top_bar_text_overlay_palette_uses_chrome_colors() {
        let colors = InlineChipColors {
            fill: Rgba(1, 2, 3, 120),
            border: Rgba(4, 5, 6, 200),
            highlight: Rgba(7, 8, 9, 80),
            fg: Rgba(10, 11, 12, 255),
        };
        assert_eq!(
            native_graphical_top_bar_text_palette(&colors, true),
            (Rgba(1, 2, 3, 255), Rgba(7, 8, 9, 255))
        );
        assert_eq!(
            native_graphical_top_bar_text_palette(&colors, false),
            (Rgba(10, 11, 12, 255), Rgba(1, 2, 3, 235))
        );
        assert_eq!(
            native_graphical_top_bar_clock_palette(&colors),
            (Rgba(10, 11, 12, 255), Rgba(1, 2, 3, 240))
        );
    }

    #[test]
    fn text_overlays_temporarily_hide_raw_app_graphics() {
        assert!(raw_compositor_should_render_app_graphics(false));
        assert!(!raw_compositor_should_render_app_graphics(true));
        assert!(should_hide_raw_graphics_for_text_overlay(true, false));
        assert!(!should_hide_raw_graphics_for_text_overlay(true, true));
        assert!(!should_hide_raw_graphics_for_text_overlay(false, false));
        assert!(!raw_compositor_should_render_app_graphics(true || false));
        assert!(!raw_compositor_should_render_app_graphics(false || true));
        assert_eq!(
            raw_compositor_footer_row_for_overlays(2, true, false, 24),
            Some(18)
        );
        assert_eq!(
            raw_compositor_footer_row_for_overlays(2, false, true, 24),
            Some(16)
        );
        assert_eq!(
            raw_compositor_footer_row_for_overlays(22, true, false, 24),
            Some(22)
        );
        assert_eq!(
            raw_compositor_footer_row_for_overlays(2, true, false, 17),
            None
        );
    }

    #[test]
    fn native_idle_frame_pacing_uses_active_then_idle_target() {
        let active = Duration::from_millis(33);
        let idle = Duration::from_millis(100);
        assert_eq!(native_current_frame_target(active, idle, 0), active);
        assert_eq!(native_current_frame_target(active, idle, 1), active);
        assert_eq!(native_current_frame_target(active, idle, 2), idle);

        let mut counter = 0;
        update_native_idle_counter(&mut counter, false);
        update_native_idle_counter(&mut counter, false);
        assert_eq!(counter, 2);
        update_native_idle_counter(&mut counter, true);
        assert_eq!(counter, 0);
        update_native_idle_counter(&mut counter, false);
        update_native_idle_counter(&mut counter, false);
        assert_eq!(counter, 2);
        update_native_idle_counter_for_activity(&mut counter, false, true);
        assert_eq!(counter, 0);
    }

    #[test]
    fn native_idle_frame_target_defaults_to_calm_four_fps() {
        std::env::remove_var("KITTWM_IDLE_FPS");
        assert_eq!(DEFAULT_NATIVE_IDLE_FPS, 4);
        assert_eq!(
            native_idle_frame_target(Duration::from_millis(16)),
            Duration::from_millis(250)
        );
        std::env::set_var("KITTWM_IDLE_FPS", "20");
        assert_eq!(
            native_idle_frame_target(Duration::from_millis(16)),
            Duration::from_millis(50)
        );
        std::env::remove_var("KITTWM_IDLE_FPS");
    }

    #[test]
    fn native_app_placement_write_decision_skips_redundant_placements() {
        let mut placements = HashMap::new();
        let first =
            decide_native_app_placement_write(&mut placements, 7, CellRect::new(2, 3, 10, 4), true);
        assert_eq!(
            first,
            NativeAppPlacementDecision {
                write_upload: true,
                write_placement: true,
            }
        );

        let pixel_only =
            decide_native_app_placement_write(&mut placements, 7, CellRect::new(2, 3, 10, 4), true);
        assert_eq!(
            pixel_only,
            NativeAppPlacementDecision {
                write_upload: true,
                write_placement: false,
            }
        );

        let unchanged_clean = decide_native_app_placement_write(
            &mut placements,
            7,
            CellRect::new(2, 3, 10, 4),
            false,
        );
        assert_eq!(
            unchanged_clean,
            NativeAppPlacementDecision {
                write_upload: false,
                write_placement: false,
            }
        );

        let moved_clean = decide_native_app_placement_write(
            &mut placements,
            7,
            CellRect::new(4, 3, 10, 4),
            false,
        );
        assert_eq!(
            moved_clean,
            NativeAppPlacementDecision {
                write_upload: false,
                write_placement: true,
            }
        );
    }

    #[test]
    fn raw_frame_chrome_change_forces_move_only_replacement() {
        let clean = NativePngFrameDecision {
            upload: false,
            placement: NativeAppPlacementDecision {
                write_upload: false,
                write_placement: false,
            },
        };
        assert_eq!(raw_frame_write_with_chrome_change(clean, false), clean);
        assert_eq!(
            raw_frame_write_with_chrome_change(clean, true),
            NativePngFrameDecision {
                upload: false,
                placement: NativeAppPlacementDecision {
                    write_upload: false,
                    write_placement: true,
                },
            }
        );
    }

    #[test]
    fn native_png_frame_write_decision_skips_unchanged_uploads() {
        let mut placements = HashMap::new();
        let mut hashes = HashMap::new();
        let fp = CellRect::new(1, 2, 10, 4);
        let first = decide_native_png_frame_write(&mut hashes, &mut placements, 9, fp, b"png-a");
        assert_eq!(
            first,
            NativePngFrameDecision {
                upload: true,
                placement: NativeAppPlacementDecision {
                    write_upload: true,
                    write_placement: true,
                },
            }
        );

        let unchanged =
            decide_native_png_frame_write(&mut hashes, &mut placements, 9, fp, b"png-a");
        assert_eq!(
            unchanged,
            NativePngFrameDecision {
                upload: false,
                placement: NativeAppPlacementDecision {
                    write_upload: false,
                    write_placement: false,
                },
            }
        );

        let moved = decide_native_png_frame_write(
            &mut hashes,
            &mut placements,
            9,
            CellRect::new(2, 2, 10, 4),
            b"png-a",
        );
        assert_eq!(
            moved,
            NativePngFrameDecision {
                upload: false,
                placement: NativeAppPlacementDecision {
                    write_upload: false,
                    write_placement: true,
                },
            }
        );

        let changed = decide_native_png_frame_write(
            &mut hashes,
            &mut placements,
            9,
            CellRect::new(2, 2, 10, 4),
            b"png-b",
        );
        assert!(changed.upload);
        assert!(!changed.placement.write_placement);
    }

    #[test]
    fn native_raw_frame_write_decision_skips_unchanged_uploads() {
        let mut placements = HashMap::new();
        let mut hashes = HashMap::new();
        let fp = CellRect::new(1, 2, 10, 4);
        let rgba = vec![0xaa; 2 * 2 * 4];

        let first =
            decide_native_raw_frame_write(&mut hashes, &mut placements, 12, fp, 2, 2, &rgba);
        assert!(first.upload);
        assert!(first.placement.write_placement);

        let unchanged =
            decide_native_raw_frame_write(&mut hashes, &mut placements, 12, fp, 2, 2, &rgba);
        assert!(!unchanged.upload);
        assert!(!unchanged.placement.write_placement);

        let moved = decide_native_raw_frame_write(
            &mut hashes,
            &mut placements,
            12,
            CellRect::new(3, 2, 10, 4),
            2,
            2,
            &rgba,
        );
        assert!(moved.upload);
        assert!(moved.placement.write_placement);
        assert!(should_unplace_raw_frame_before_move(
            true,
            moved.placement.write_placement
        ));

        let mut changed_rgba = rgba.clone();
        changed_rgba[0] ^= 0xff;
        let changed = decide_native_raw_frame_write(
            &mut hashes,
            &mut placements,
            12,
            CellRect::new(3, 2, 10, 4),
            2,
            2,
            &changed_rgba,
        );
        assert!(changed.upload);
        assert!(!changed.placement.write_placement);
        assert!(!should_unplace_raw_frame_before_move(
            false,
            first.placement.write_placement
        ));
    }

    #[test]
    fn native_frame_write_bytes_counts_actual_sequences() {
        let placement = kittui::Placement {
            image_id: 42,
            upload: "\x1b_Gupload\x1b\\".to_string(),
            placement: "\x1b_Gplace\x1b\\".to_string(),
            embed: "▓▓".to_string(),
            footprint: CellRect::new(0, 0, 2, 1),
        };
        let mut bytes = NativeFrameWriteBytes::default();
        bytes.add(&placement, true);
        assert_eq!(bytes.upload, placement.upload.as_bytes().len());
        assert_eq!(bytes.placement, placement.placement.as_bytes().len());
        assert_eq!(bytes.embed, placement.embed.as_bytes().len());
        assert!(bytes.embed > placement.embed.chars().count());

        let mut move_only = NativeFrameWriteBytes::default();
        move_only.add(&placement, false);
        assert_eq!(move_only.upload, 0);
        assert_eq!(move_only.placement, placement.placement.as_bytes().len());
        assert_eq!(move_only.embed, placement.embed.as_bytes().len());
    }

    #[test]
    fn native_frame_event_publish_decision_suppresses_clean_static_frames() {
        assert!(should_publish_native_frame_event(true, false, None));
        assert!(should_publish_native_frame_event(false, true, None));
        assert!(should_publish_native_frame_event(true, true, Some(0)));
        assert!(should_publish_native_frame_event(false, false, Some(1)));
        assert!(!should_publish_native_frame_event(false, false, Some(0)));
        assert!(!should_publish_native_frame_event(false, false, None));
    }

    #[test]
    fn native_renderer_defaults_to_terminal_inside_tmux_unless_overridden() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_NATIVE_RENDERER");
        std::env::remove_var("TMUX");
        assert!(!native_should_use_pure_terminal_renderer());
        std::env::set_var("TMUX", "/tmp/tmux-1/default,1,0");
        assert!(native_should_use_pure_terminal_renderer());
        std::env::set_var("KITTWM_NATIVE_RENDERER", "kitty");
        assert!(!native_should_use_pure_terminal_renderer());
        std::env::set_var("KITTWM_NATIVE_RENDERER", "terminal");
        assert!(native_should_use_pure_terminal_renderer());
        std::env::remove_var("TMUX");
        std::env::remove_var("KITTWM_NATIVE_RENDERER");
    }

    #[test]
    fn native_terminal_command_honors_config_env_precedence() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_TERMINAL_CMD");
        std::env::remove_var("KITTWM_TERMINAL_BINARY");
        std::env::set_var("SHELL", "/bin/test-shell");
        let mut config = KittwmConfig::default();
        config.terminal.command = Some("config-shell".to_string());
        assert_eq!(native_terminal_command(&config), "config-shell");
        config.terminal.command = None;
        let login_shell = native_login_shell_command("/bin/test-shell".to_string());
        assert_eq!(login_shell, "/bin/test-shell -l");
        assert_eq!(login_shell.capacity(), login_shell.len());
        assert_eq!(native_terminal_command(&config), login_shell);
        std::env::set_var("KITTWM_TERMINAL_BINARY", "kittui-ghostty --app");
        assert_eq!(native_terminal_command(&config), "kittui-ghostty --app");
        std::env::set_var("KITTWM_TERMINAL_CMD", "htop");
        assert_eq!(native_terminal_command(&config), "htop");
        std::env::remove_var("KITTWM_TERMINAL_CMD");
        std::env::remove_var("KITTWM_TERMINAL_BINARY");
        std::env::remove_var("SHELL");
    }

    #[test]
    fn native_terminal_backend_selects_libghostty_from_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_TERMINAL_BACKEND");
        std::env::remove_var("KITTWM_TERMINAL_APP");
        let mut config = KittwmConfig::default();
        config.terminal.backend = "pty".to_string();
        assert_eq!(native_terminal_backend(&config), NativeTerminalBackend::Pty);
        config.terminal.backend = "ghostty".to_string();
        assert_eq!(
            native_terminal_backend(&config),
            NativeTerminalBackend::Ghostty
        );
        std::env::set_var("KITTWM_TERMINAL_BACKEND", "libghostty");
        assert_eq!(
            native_terminal_backend(&config),
            NativeTerminalBackend::Ghostty
        );
        std::env::remove_var("KITTWM_TERMINAL_BACKEND");
        std::env::set_var("KITTWM_TERMINAL_APP", "kittui-ghostty");
        assert_eq!(
            native_terminal_backend(&config),
            NativeTerminalBackend::Ghostty
        );
        std::env::remove_var("KITTWM_TERMINAL_APP");
    }

    #[test]
    fn native_startup_terminal_is_opt_in() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_STARTUP_TERMINAL");
        assert!(!native_startup_terminal_enabled());
        std::env::set_var("KITTWM_STARTUP_TERMINAL", "1");
        assert!(native_startup_terminal_enabled());
        std::env::set_var("KITTWM_STARTUP_TERMINAL", "true");
        assert!(native_startup_terminal_enabled());
        std::env::set_var("KITTWM_STARTUP_TERMINAL", "0");
        assert!(!native_startup_terminal_enabled());
        std::env::remove_var("KITTWM_STARTUP_TERMINAL");
    }

    #[test]
    fn native_theme_test_temp_dir_name_builds_directly() {
        let name = native_theme_test_temp_dir_name(1234);
        assert_eq!(name, "kittwm-theme-test-1234");
        assert!(name.capacity() >= name.len());
    }

    fn native_theme_test_temp_dir_name(pid: u32) -> String {
        let mut name = String::with_capacity("kittwm-theme-test-".len() + 20);
        name.push_str("kittwm-theme-test-");
        let _ = write!(name, "{pid}");
        name
    }

    #[test]
    fn native_chrome_colors_follow_kittwm_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(native_theme_test_temp_dir_name(std::process::id()));
        let cfg_dir = root.join("kittwm");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.yaml"),
            "background:\n  color: '#112233'\n  opacity: 0.5\ncolorscheme:\n  fg: '#abcdef'\n",
        )
        .unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &root);
        let colors = native_glass_chrome_colors();
        assert_eq!(colors.fill, Rgba(0x11, 0x22, 0x33, 128));
        assert_eq!(colors.border, Rgba(0xab, 0xcd, 0xef, 255));
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_chrome_renderer_selector_defaults_to_kittui_graphics() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_NATIVE_CHROME_RENDERER");
        assert!(native_should_use_affordance_scene_chrome());
        std::env::set_var("KITTWM_NATIVE_CHROME_RENDERER", "affordance-scene");
        assert!(native_should_use_affordance_scene_chrome());
        std::env::set_var("KITTWM_NATIVE_CHROME_RENDERER", "kittui");
        assert!(native_should_use_affordance_scene_chrome());
        std::env::set_var("KITTWM_NATIVE_CHROME_RENDERER", "ansi");
        assert!(!native_should_use_affordance_scene_chrome());
        std::env::set_var("KITTWM_NATIVE_CHROME_RENDERER", "off");
        assert!(!native_should_use_affordance_scene_chrome());
        std::env::remove_var("KITTWM_NATIVE_CHROME_RENDERER");
    }

    #[test]
    fn native_dirty_frame_policy_skips_identical_frames_by_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_DIRTY_FRAMES");
        let rgba = vec![0u8; 4 * 4 * 4];
        let mut enabled = NativeDirtyFramePolicy::from_env();
        let first = enabled.decide(1, 4, 4, &rgba);
        assert!(first.upload);
        assert_eq!(first.metrics.changed_tiles, 1);
        let second = enabled.decide(1, 4, 4, &rgba);
        assert!(!second.upload);
        assert!(second.metrics.skipped_upload);
        assert_eq!(second.metrics.changed_tiles, 0);
        let mut changed = rgba.clone();
        changed[0] = 1;
        let third = enabled.decide(1, 4, 4, &changed);
        assert!(third.upload);
        assert_eq!(third.metrics.changed_tiles, 1);
        std::env::remove_var("KITTWM_DIRTY_FRAMES");
    }

    #[test]
    fn native_clear_resets_app_frame_memos() {
        let pane = dummy_native_pane("native-1", "sh", 1);
        let image_id = pane.image_id;
        let mut placements = HashMap::from([(image_id, CellRect::new(1, 2, 3, 4))]);
        let mut png_hashes = HashMap::from([(image_id, 99u64)]);
        let rgba = vec![0u8; 4 * 4 * 4];
        let mut dirty_frames = NativeDirtyFramePolicy::from_env();
        assert!(dirty_frames.decide(image_id, 4, 4, &rgba).upload);
        assert!(!dirty_frames.decide(image_id, 4, 4, &rgba).upload);

        reset_native_app_frame_memos_for_clear(
            &mut placements,
            &mut png_hashes,
            &mut dirty_frames,
            &[pane],
        );

        assert!(placements.is_empty());
        assert!(png_hashes.is_empty());
        assert!(dirty_frames.decide(image_id, 4, 4, &rgba).upload);
    }

    #[test]
    fn native_dirty_frame_policy_forget_forces_reused_id_upload() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_DIRTY_FRAMES");
        let rgba = vec![0u8; 4 * 4 * 4];
        let mut policy = NativeDirtyFramePolicy::from_env();
        assert!(policy.decide(9, 4, 4, &rgba).upload);
        assert!(!policy.decide(9, 4, 4, &rgba).upload);
        policy.forget(9);
        assert!(policy.decide(9, 4, 4, &rgba).upload);
    }

    #[test]
    fn native_dirty_frame_policy_can_force_every_upload() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_DIRTY_FRAMES", "always-upload");
        let rgba = vec![0u8; 4 * 4 * 4];
        let mut disabled = NativeDirtyFramePolicy::from_env();
        assert!(disabled.decide(1, 4, 4, &rgba).upload);
        assert!(disabled.decide(1, 4, 4, &rgba).upload);
        std::env::remove_var("KITTWM_DIRTY_FRAMES");
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "Nix Darwin sandbox lacks a stable PTY shell for dummy panes"
    )]
    fn native_pane_statuses_include_dirty_frame_metrics() {
        let panes = vec![NativePane {
            window: "native-1".to_string(),
            image_id: 1,
            command: "cmd1".to_string(),
            pid: Some(101),
            display_title: None,
            weight: 1,
            app: dummy_native_pane_app(),
            dirty_frame: Some(NativeDirtyFrameMetrics {
                changed_tiles: 2,
                total_tiles: 4,
                changed_fraction: 0.5,
                skipped_upload: true,
            }),
            capture_failed: false,
        }];
        let layouts = vec![NativePaneLayout {
            x: 0,
            y: 0,
            cols: 10,
            rows: 6,
            app_x: 0,
            app_y: 1,
            app_cols: 10,
            app_rows: 4,
        }];
        let statuses = native_pane_statuses(
            &panes,
            0,
            &layouts,
            &HashMap::new(),
            NativePaneLayoutMode::Tiled,
            None,
        );
        let dirty = statuses[0].dirty_frame.as_ref().unwrap();
        assert_eq!(dirty.changed_tiles, 2);
        assert_eq!(dirty.total_tiles, 4);
        assert_eq!(dirty.changed_fraction, 0.5);
        assert!(dirty.skipped_upload);
    }

    #[test]
    fn fit_rgba_frame_to_cells_crops_and_pads_without_scaling() {
        let red = [0xff, 0x00, 0x00, 0xff];
        let green = [0x00, 0xff, 0x00, 0xff];
        let oversized = [red, green].concat();
        let (cropped, width, height) = fit_rgba_frame_to_cells(oversized, 2, 1, 1, 1);
        assert_eq!((width, height), (8, 16));
        assert_eq!(&cropped[..4], &red);
        assert_eq!(&cropped[4..8], &green);

        let (padded, width, height) =
            fit_rgba_frame_to_cells(vec![0xaa, 0xbb, 0xcc, 0xff], 1, 1, 1, 1);
        assert_eq!((width, height), (8, 16));
        assert_eq!(&padded[..4], &[0xaa, 0xbb, 0xcc, 0xff]);
        assert_eq!(&padded[4..8], &NATIVE_FRAME_BG_RGBA);
    }

    #[test]
    fn native_key_event_payload_honors_application_cursor_mode() {
        let event = InputEvent::Key {
            key: Key::Up,
            mods: kittui_input::Modifiers::default(),
        };
        assert_eq!(
            native_key_event_payload(&event, false),
            Some(&b"\x1b[A"[..])
        );
        assert_eq!(native_key_event_payload(&event, true), Some(&b"\x1bOA"[..]));
    }

    #[test]
    fn native_paste_payload_wraps_when_bracketed() {
        assert_eq!(native_paste_payload(b"a\nb", false), b"a\nb".to_vec());
        assert_eq!(
            native_paste_payload(b"a\nb", true),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
    }

    #[test]
    fn native_paste_payload_preserves_exact_bytes_and_wraps_only_when_enabled() {
        let bytes = b"\0\x1b[31mraw\xff\n";
        assert_eq!(native_paste_payload(bytes, false), bytes.to_vec());
        let wrapped = native_paste_payload(bytes, true);
        assert!(wrapped.starts_with(b"\x1b[200~"));
        assert!(wrapped.ends_with(b"\x1b[201~"));
        assert_eq!(&wrapped[6..wrapped.len() - 6], bytes);
    }

    #[test]
    fn native_pane_at_host_cell_translates_to_local_coordinates() {
        let layouts = vec![
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 20,
                rows: 11,
                app_x: 0,
                app_y: 1,
                app_cols: 20,
                app_rows: 9,
            },
            NativePaneLayout {
                x: 20,
                y: 0,
                cols: 20,
                rows: 11,
                app_x: 20,
                app_y: 1,
                app_cols: 20,
                app_rows: 9,
            },
        ];
        assert_eq!(native_pane_at_host_cell(&layouts, 1, 1), None);
        assert_eq!(native_pane_at_host_cell(&layouts, 1, 2), Some((0, 1, 1)));
        assert_eq!(native_pane_at_host_cell(&layouts, 21, 2), Some((1, 1, 1)));
        assert_eq!(native_pane_at_host_cell(&layouts, 40, 10), Some((1, 20, 9)));
        assert_eq!(native_pane_at_host_cell(&layouts, 41, 10), None);
    }

    fn dummy_native_pane(window: &str, command: &str, weight: u16) -> NativePane {
        NativePane {
            window: window.to_string(),
            image_id: 1,
            command: command.to_string(),
            pid: None,
            display_title: None,
            weight,
            app: dummy_native_pane_app(),
            dirty_frame: None,
            capture_failed: false,
        }
    }

    #[test]
    fn native_route_mouse_focuses_chrome_and_app_without_top_bar_leakage() {
        let mut panes = vec![
            dummy_native_pane("native-1", "left", 1),
            dummy_native_pane("native-2", "right", 1),
        ];
        let mut focused = 0usize;
        let mut clear = false;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();

        // Top-bar row is reserved chrome outside any pane; clicking it should
        // be consumed by the WM but must not change pane focus.
        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: 1,
                row: 1,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Tiled,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 0);
        assert!(!clear);

        // Pane title chrome should focus the pane and force a redraw so focus
        // visuals cannot lag behind input routing.
        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: 42,
                row: 2,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Tiled,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 1);
        assert!(clear);

        // App-area clicks keep focus aligned with the pane that receives input.
        clear = false;
        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: 2,
                row: 3,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Tiled,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 0);
        assert!(clear);
    }

    #[test]
    fn native_route_mouse_drags_tiled_title_to_reorder_panes() {
        let mut panes = vec![
            dummy_native_pane("native-1", "left", 1),
            dummy_native_pane("native-2", "right", 1),
        ];
        let mut focused = 0usize;
        let mut clear = false;
        let mut tiled_drag = None;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();

        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: 2,
                row: 2,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Tiled,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut tiled_drag,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 0);
        assert_eq!(
            tiled_drag,
            Some(NativeTiledDrag {
                window: "native-1".to_string()
            })
        );

        assert!(native_route_mouse_event(
            &InputEvent::MouseRelease {
                col: 42,
                row: 2,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Tiled,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut tiled_drag,
            &mut clear,
        )
        .unwrap());
        assert_eq!(
            panes
                .iter()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-2", "native-1"]
        );
        assert_eq!(focused, 1);
        assert!(tiled_drag.is_none());
        assert!(clear);
    }

    #[test]
    fn native_route_mouse_does_not_drag_tiled_bottom_chrome() {
        let mut panes = vec![
            dummy_native_pane("native-1", "left", 1),
            dummy_native_pane("native-2", "right", 1),
        ];
        let mut focused = 0usize;
        let mut clear = false;
        let mut tiled_drag = None;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let layouts = native_layouts_for_panes_display_mode_with_offsets(
            80,
            24,
            &panes,
            focused,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Tiled,
            &reservation,
            &HashMap::new(),
        );
        let bottom_row = layouts[0]
            .y
            .saturating_add(layouts[0].rows.saturating_sub(1))
            .saturating_add(1);

        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: layouts[0].x + 2,
                row: bottom_row,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Tiled,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut tiled_drag,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 0);
        assert!(tiled_drag.is_none());
        assert!(clear);
    }

    #[test]
    fn native_route_mouse_uses_fullscreen_layout_for_visible_pane() {
        let mut panes = vec![
            dummy_native_pane("native-1", "left", 1),
            dummy_native_pane("native-2", "right", 1),
        ];
        let mut focused = 1usize;
        let mut clear = false;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();

        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: 2,
                row: 3,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Fullscreen,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 1);
    }

    #[test]
    fn native_route_mouse_uses_topmost_floating_pane_for_overlap() {
        let mut panes = vec![
            dummy_native_pane("native-1", "bottom", 1),
            dummy_native_pane("native-2", "top", 1),
        ];
        let mut focused = 0usize;
        let mut clear = false;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let layouts = native_layouts_for_panes_display_mode(
            80,
            24,
            &panes,
            focused,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
        );
        let top = layouts[1];
        let host_col = top.app_x + 1;
        let host_row = top.app_y + 1;
        assert_eq!(
            native_pane_at_host_cell_ordered(&layouts, host_col, host_row, false).map(|hit| hit.0),
            Some(0)
        );
        assert_eq!(
            native_pane_at_host_cell_ordered(&layouts, host_col, host_row, true).map(|hit| hit.0),
            Some(1)
        );

        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: host_col,
                row: host_row,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 1);
        assert!(clear);
    }

    #[test]
    fn native_route_mouse_raises_background_floating_pane_on_focus() {
        let mut panes = vec![
            dummy_native_pane("native-1", "bottom", 1),
            dummy_native_pane("native-2", "top", 1),
        ];
        let mut focused = 1usize;
        let mut clear = false;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let layouts = native_layouts_for_panes_display_mode(
            80,
            24,
            &panes,
            focused,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
        );
        let (host_col, host_row) = (1..=80)
            .flat_map(|col| (1..=24).map(move |row| (col, row)))
            .find(|(col, row)| {
                native_pane_at_host_cell_ordered(&layouts, *col, *row, false).map(|hit| hit.0)
                    == Some(0)
                    && native_pane_at_host_cell_ordered(&layouts, *col, *row, true).map(|hit| hit.0)
                        == Some(0)
            })
            .expect("non-overlapped app cell for background floating pane");

        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: host_col,
                row: host_row,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &mut HashMap::new(),
            &mut None,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert_eq!(focused, 1);
        assert_eq!(panes[1].command, "bottom");
        assert!(clear);
    }

    #[test]
    fn native_route_mouse_drags_floating_pane_from_title_chrome() {
        let mut panes = vec![dummy_native_pane("native-1", "float", 1)];
        let mut focused = 0usize;
        let mut clear = false;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let mut offsets = HashMap::new();
        let mut drag = None;
        let before = native_layouts_for_panes_display_mode_with_offsets(
            80,
            24,
            &panes,
            focused,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &offsets,
        );
        let title_col = before[0].x + 1;
        let title_row = before[0].y + 1;

        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: title_col,
                row: title_row,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &mut offsets,
            &mut drag,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert!(drag.is_some());

        assert!(native_route_mouse_event(
            &InputEvent::MouseMove {
                col: title_col + 5,
                row: title_row + 2,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &mut offsets,
            &mut drag,
            &mut None,
            &mut clear,
        )
        .unwrap());
        let after = native_layouts_for_panes_display_mode_with_offsets(
            80,
            24,
            &panes,
            focused,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &offsets,
        );
        assert_eq!(after[0].x, before[0].x + 5);
        assert_eq!(after[0].y, before[0].y + 2);

        assert!(native_route_mouse_event(
            &InputEvent::MouseRelease {
                col: title_col + 5,
                row: title_row + 2,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &mut offsets,
            &mut drag,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert!(drag.is_none());
    }

    #[test]
    fn native_route_mouse_does_not_drag_floating_bottom_chrome() {
        let mut panes = vec![dummy_native_pane("native-1", "float", 1)];
        let mut focused = 0usize;
        let mut clear = false;
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let mut offsets = HashMap::new();
        let mut drag = None;
        let layouts = native_layouts_for_panes_display_mode_with_offsets(
            80,
            24,
            &panes,
            focused,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &offsets,
        );
        let bottom_row = layouts[0]
            .y
            .saturating_add(layouts[0].rows.saturating_sub(1))
            .saturating_add(1);

        assert!(native_route_mouse_event(
            &InputEvent::MousePress {
                col: layouts[0].x + 1,
                row: bottom_row,
                button: MouseButton::Left,
                mods: Default::default(),
            },
            &mut panes,
            &mut focused,
            80,
            24,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
            &mut offsets,
            &mut drag,
            &mut None,
            &mut clear,
        )
        .unwrap());
        assert!(drag.is_none());
        assert!(offsets.is_empty());
        assert!(clear);
    }

    #[test]
    fn native_nudge_floating_offset_updates_and_saturates() {
        let mut offsets = HashMap::new();
        native_nudge_floating_offset(&mut offsets, "native-1", 5, -2);
        assert_eq!(
            offsets["native-1"],
            NativeFloatingPaneOffset { dx: 5, dy: -2 }
        );
        native_nudge_floating_offset(&mut offsets, "native-1", i16::MAX, i16::MIN);
        assert_eq!(
            offsets["native-1"],
            NativeFloatingPaneOffset {
                dx: i16::MAX,
                dy: i16::MIN,
            }
        );
    }

    #[test]
    fn native_reset_floating_offset_removes_stored_offset() {
        let mut offsets = HashMap::new();
        native_nudge_floating_offset(&mut offsets, "native-1", 5, -2);
        native_reset_floating_offset(&mut offsets, "native-1");
        assert!(!offsets.contains_key("native-1"));
        assert_eq!(
            native_socket_reset_offset_log_line("focused"),
            "native terminal socket reset offset: focused"
        );
    }

    #[test]
    fn native_reset_all_floating_offsets_clears_every_stored_offset() {
        let mut offsets = HashMap::new();
        native_nudge_floating_offset(&mut offsets, "native-1", 5, -2);
        native_nudge_floating_offset(&mut offsets, "native-2", -1, 3);
        assert_eq!(native_reset_all_floating_offsets(&mut offsets), 2);
        assert!(offsets.is_empty());
        assert_eq!(
            native_socket_reset_all_offsets_log_line(2),
            "native terminal socket reset all offsets: 2"
        );
    }

    #[test]
    fn native_keyboard_nudge_updates_focused_floating_offset() {
        let panes = vec![
            dummy_native_pane("native-1", "left", 1),
            dummy_native_pane("native-2", "right", 1),
        ];
        let mut offsets = HashMap::new();
        let mut clear = false;
        let dbg = Debugger::open();
        native_nudge_focused_pane(&mut offsets, &panes, 1, -1, 1, &mut clear, &dbg);
        assert!(clear);
        assert_eq!(
            offsets["native-2"],
            NativeFloatingPaneOffset { dx: -1, dy: 1 }
        );
        assert!(!offsets.contains_key("native-1"));
        assert_eq!(
            native_keyboard_nudge_log_line("native-2", -1, 1),
            "native terminal keyboard nudge: native-2 dx=-1 dy=1"
        );
    }

    #[test]
    fn native_keyboard_reset_removes_focused_floating_offset() {
        let panes = vec![dummy_native_pane("native-1", "float", 1)];
        let mut offsets = HashMap::from([(
            "native-1".to_string(),
            NativeFloatingPaneOffset { dx: 4, dy: -3 },
        )]);
        let mut clear = false;
        let dbg = Debugger::open();
        native_reset_focused_pane(&mut offsets, &panes, 0, &mut clear, &dbg);
        assert!(clear);
        assert!(!offsets.contains_key("native-1"));
        assert_eq!(
            native_keyboard_reset_offset_log_line("native-1"),
            "native terminal keyboard reset offset: native-1"
        );
    }

    #[test]
    fn native_keyboard_reset_all_removes_every_floating_offset() {
        let mut offsets = HashMap::from([
            (
                "native-1".to_string(),
                NativeFloatingPaneOffset { dx: 4, dy: -3 },
            ),
            (
                "native-2".to_string(),
                NativeFloatingPaneOffset { dx: -1, dy: 2 },
            ),
        ]);
        let mut clear = false;
        let dbg = Debugger::open();
        native_reset_all_floating_offsets_from_keyboard(&mut offsets, &mut clear, &dbg);
        assert!(clear);
        assert!(offsets.is_empty());
        assert_eq!(
            native_keyboard_reset_all_offsets_log_line(2),
            "native terminal keyboard reset all offsets: 2"
        );
    }

    #[test]
    fn native_prefix_wasd_nudges_in_floating_mode() {
        let mut panes = vec![dummy_native_pane("native-1", "float", 1)];
        let mut focused = 0usize;
        let mut layout_axis = NativePaneLayoutAxis::Columns;
        let mut layout_mode = NativePaneLayoutMode::Floating;
        let mut offsets = HashMap::new();
        let mut prefix = false;
        let mut clear = false;
        let mut help_overlay = false;
        let mut ctrl_c_guard = NativeCtrlCExitGuard::default();
        let mut quit_overlay = QuitConfirmOverlay::default();
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let dbg = Debugger::open();

        assert!(!process_native_terminal_byte(
            0x01,
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(prefix);

        assert!(!process_native_terminal_byte(
            b'd',
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(!prefix);
        assert!(clear);
        assert_eq!(
            offsets["native-1"],
            NativeFloatingPaneOffset { dx: 1, dy: 0 }
        );

        assert!(!process_native_terminal_byte(
            0x01,
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(!process_native_terminal_byte(
            b'r',
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(!prefix);
        assert!(!offsets.contains_key("native-1"));
    }

    #[test]
    fn native_prefix_shift_r_resets_all_floating_offsets() {
        let mut panes = vec![
            dummy_native_pane("native-1", "left", 1),
            dummy_native_pane("native-2", "right", 1),
        ];
        let mut focused = 0usize;
        let mut layout_axis = NativePaneLayoutAxis::Columns;
        let mut layout_mode = NativePaneLayoutMode::Floating;
        let mut offsets = HashMap::from([
            (
                "native-1".to_string(),
                NativeFloatingPaneOffset { dx: 1, dy: 0 },
            ),
            (
                "native-2".to_string(),
                NativeFloatingPaneOffset { dx: -2, dy: 3 },
            ),
        ]);
        let mut prefix = false;
        let mut clear = false;
        let mut help_overlay = false;
        let mut ctrl_c_guard = NativeCtrlCExitGuard::default();
        let mut quit_overlay = QuitConfirmOverlay::default();
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let dbg = Debugger::open();

        assert!(!process_native_terminal_byte(
            0x01,
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(prefix);

        assert!(!process_native_terminal_byte(
            b'R',
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(!prefix);
        assert!(clear);
        assert!(offsets.is_empty());
    }

    #[test]
    fn native_prefix_p_focuses_previous_pane() {
        let mut panes = vec![
            dummy_native_pane("native-1", "one", 1),
            dummy_native_pane("native-2", "two", 1),
            dummy_native_pane("native-3", "three", 1),
        ];
        let mut focused = 0usize;
        let mut layout_axis = NativePaneLayoutAxis::Columns;
        let mut layout_mode = NativePaneLayoutMode::Tiled;
        let mut offsets = HashMap::new();
        let mut prefix = false;
        let mut clear = false;
        let mut help_overlay = false;
        let mut ctrl_c_guard = NativeCtrlCExitGuard::default();
        let mut quit_overlay = QuitConfirmOverlay::default();
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let dbg = Debugger::open();

        assert!(!process_native_terminal_byte(
            0x01,
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(!process_native_terminal_byte(
            b'p',
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert_eq!(focused, 2);
        assert!(clear);
        assert_eq!(
            native_keyboard_focus_prev_log_line("native-3"),
            "native terminal keyboard focus prev: native-3"
        );
    }

    #[test]
    fn native_prefix_braces_raise_and_lower_floating_pane() {
        let mut panes = vec![
            dummy_native_pane("native-1", "bottom", 1),
            dummy_native_pane("native-2", "top", 1),
        ];
        let mut focused = 0usize;
        let mut layout_axis = NativePaneLayoutAxis::Columns;
        let mut layout_mode = NativePaneLayoutMode::Floating;
        let mut offsets = HashMap::new();
        let mut prefix = false;
        let mut clear = false;
        let mut help_overlay = false;
        let mut ctrl_c_guard = NativeCtrlCExitGuard::default();
        let mut quit_overlay = QuitConfirmOverlay::default();
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        let dbg = Debugger::open();

        assert!(!process_native_terminal_byte(
            0x01,
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(!process_native_terminal_byte(
            b'}',
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert_eq!(focused, 1);
        assert_eq!(panes[1].window, "native-1");

        assert!(!process_native_terminal_byte(
            0x01,
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert!(!process_native_terminal_byte(
            b'{',
            &mut prefix,
            &mut panes,
            &mut focused,
            &mut layout_axis,
            &mut layout_mode,
            &mut offsets,
            "sh",
            "/tmp/kittwm.sock",
            80,
            24,
            &reservation,
            &mut clear,
            &mut help_overlay,
            &mut ctrl_c_guard,
            &mut quit_overlay,
            &dbg,
        )
        .unwrap());
        assert_eq!(focused, 0);
        assert_eq!(panes[0].window, "native-1");
    }

    #[test]
    fn native_mouse_hit_testing_separates_top_bar_chrome_and_app_area() {
        let layouts = reserve_native_top_bar(native_pane_layouts_weighted(
            80,
            native_tilable_rows(24),
            &[1, 1],
            NativePaneLayoutAxis::Columns,
        ));
        assert_eq!(native_pane_at_host_cell(&layouts, 1, 1), None);
        assert_eq!(native_pane_chrome_at_host_cell(&layouts, 1, 1), None);

        assert_eq!(native_pane_at_host_cell(&layouts, 1, 2), None);
        assert_eq!(native_pane_chrome_at_host_cell(&layouts, 1, 2), Some(0));

        assert_eq!(native_pane_at_host_cell(&layouts, 1, 3), None);
        assert_eq!(native_pane_chrome_at_host_cell(&layouts, 1, 3), Some(0));

        assert_eq!(native_pane_at_host_cell(&layouts, 2, 3), Some((0, 1, 1)));
        assert_eq!(native_pane_at_host_cell(&layouts, 42, 3), Some((1, 1, 1)));
        assert_eq!(native_pane_chrome_at_host_cell(&layouts, 42, 3), Some(1));
    }

    #[test]
    fn native_mouse_event_payload_requires_compatible_modes() {
        let modes = MouseReportingModes {
            basic: true,
            button_motion: false,
            all_motion: true,
            sgr: true,
        };
        let press_left = native_mouse_event_payload("press-left", 7, 9, modes).unwrap();
        assert_eq!(press_left, b"\x1b[<0;7;9M".to_vec());
        assert!(press_left.capacity() >= press_left.len());
        assert_eq!(
            native_sgr_mouse_payload(223, 65535, 65535, 'M'),
            b"\x1b[<223;65535;65535M".to_vec()
        );
        assert_eq!(
            native_mouse_event_payload("release-left", 7, 9, modes).unwrap(),
            b"\x1b[<0;7;9m".to_vec()
        );
        assert_eq!(
            native_mouse_event_payload("release-middle", 7, 9, modes).unwrap(),
            b"\x1b[<1;7;9m".to_vec()
        );
        assert_eq!(
            native_mouse_event_payload("release-right", 7, 9, modes).unwrap(),
            b"\x1b[<2;7;9m".to_vec()
        );
        assert_eq!(
            native_mouse_event_payload("scroll-down", 7, 9, modes).unwrap(),
            b"\x1b[<65;7;9M".to_vec()
        );
        assert_eq!(
            native_mouse_event_payload(
                "move-left",
                7,
                9,
                MouseReportingModes {
                    button_motion: true,
                    all_motion: false,
                    ..modes
                },
            )
            .unwrap(),
            b"\x1b[<32;7;9M".to_vec()
        );
        assert!(native_mouse_event_payload(
            "move",
            7,
            9,
            MouseReportingModes {
                all_motion: false,
                ..modes
            }
        )
        .is_none());
        let legacy_press = native_mouse_event_payload(
            "press-left",
            7,
            9,
            MouseReportingModes {
                sgr: false,
                ..modes
            },
        )
        .unwrap();
        assert_eq!(legacy_press, vec![b'\x1b', b'[', b'M', 32, 39, 41]);
        assert_eq!(legacy_press.capacity(), 6);
        assert_eq!(
            native_mouse_event_payload(
                "release-left",
                7,
                9,
                MouseReportingModes {
                    sgr: false,
                    ..modes
                }
            )
            .unwrap(),
            vec![b'\x1b', b'[', b'M', 35, 39, 41]
        );
        assert!(native_mouse_event_payload(
            "press-left",
            224,
            9,
            MouseReportingModes {
                sgr: false,
                ..modes
            }
        )
        .is_none());
        assert_eq!(
            native_mouse_event_payload(
                "press-left",
                7,
                9,
                MouseReportingModes {
                    basic: false,
                    button_motion: true,
                    all_motion: false,
                    sgr: true,
                },
            )
            .unwrap(),
            b"\x1b[<0;7;9M".to_vec()
        );
        assert_eq!(
            native_mouse_event_payload(
                "scroll-up",
                7,
                9,
                MouseReportingModes {
                    basic: false,
                    button_motion: false,
                    all_motion: true,
                    sgr: true,
                },
            )
            .unwrap(),
            b"\x1b[<64;7;9M".to_vec()
        );
    }

    #[test]
    fn browser_key_labels_cover_socket_send_key_names() {
        assert_eq!(
            browser_key_label("arrow-left"),
            Some(BrowserSocketKey::Arrow(BrowserArrowKey::Left))
        );
        assert_eq!(
            browser_key_label("page-down"),
            Some(BrowserSocketKey::Page(BrowserPageKey::Down))
        );
        assert_eq!(
            browser_key_label("shift-left"),
            Some(BrowserSocketKey::ShiftArrow(BrowserArrowKey::Left))
        );
        assert_eq!(
            browser_key_label("shift-arrow-up"),
            Some(BrowserSocketKey::ShiftArrow(BrowserArrowKey::Up))
        );
        assert_eq!(
            browser_key_label("alt-left"),
            Some(BrowserSocketKey::AltArrow(BrowserArrowKey::Left))
        );
        assert_eq!(
            browser_key_label("alt-arrow-up"),
            Some(BrowserSocketKey::AltArrow(BrowserArrowKey::Up))
        );
        assert_eq!(
            browser_key_label("ctrl-left"),
            Some(BrowserSocketKey::CtrlArrow(BrowserArrowKey::Left))
        );
        assert_eq!(
            browser_key_label("ctrl-arrow-up"),
            Some(BrowserSocketKey::CtrlArrow(BrowserArrowKey::Up))
        );
        assert_eq!(
            browser_key_label("shift-insert"),
            Some(BrowserSocketKey::ShiftInsert)
        );
        assert_eq!(
            browser_key_label("ctrl-delete"),
            Some(BrowserSocketKey::CtrlDelete)
        );
        assert_eq!(
            browser_key_label("ctrl-home"),
            Some(BrowserSocketKey::CtrlHome)
        );
        assert_eq!(
            browser_key_label("shift-end"),
            Some(BrowserSocketKey::ShiftEnd)
        );
        assert_eq!(
            browser_key_label("shift-page-down"),
            Some(BrowserSocketKey::ShiftPage(BrowserPageKey::Down))
        );
        assert_eq!(
            browser_key_label("ctrl-page-up"),
            Some(BrowserSocketKey::CtrlPage(BrowserPageKey::Up))
        );
        assert_eq!(browser_key_label("return"), Some(BrowserSocketKey::Enter));
        assert_eq!(
            browser_key_label("shift-tab"),
            Some(BrowserSocketKey::ShiftTab)
        );
        assert_eq!(
            browser_key_label("backtab"),
            Some(BrowserSocketKey::ShiftTab)
        );
        assert_eq!(
            browser_key_label("f12"),
            Some(BrowserSocketKey::Function(BrowserFunctionKey::F12))
        );
        assert_eq!(
            browser_key_label("ctrl-a"),
            Some(BrowserSocketKey::CtrlLetter('a'))
        );
        assert_eq!(
            browser_key_label("ctrl-c"),
            Some(BrowserSocketKey::CtrlLetter('c'))
        );
        assert_eq!(
            browser_key_label("ctrl-z"),
            Some(BrowserSocketKey::CtrlLetter('z'))
        );
        assert_eq!(browser_key_label("ctrl-1"), None);
    }

    #[test]
    fn native_surface_pointer_events_map_cells_to_pixels_and_buttons() {
        assert_eq!(
            native_surface_pointer_events("press-left", 3, 4),
            vec![
                SurfacePointerEvent::Move { x_px: 16, y_px: 48 },
                SurfacePointerEvent::Press {
                    button: SurfacePointerButton::Left,
                },
            ]
        );
        assert_eq!(
            native_surface_pointer_events("release-right", 1, 1),
            vec![
                SurfacePointerEvent::Move { x_px: 0, y_px: 0 },
                SurfacePointerEvent::Release {
                    button: SurfacePointerButton::Right,
                },
            ]
        );
        assert_eq!(
            native_surface_pointer_events("scroll-down", 2, 2),
            vec![
                SurfacePointerEvent::Move { x_px: 8, y_px: 16 },
                SurfacePointerEvent::Press {
                    button: SurfacePointerButton::ScrollDown,
                },
            ]
        );
        assert_eq!(
            native_surface_pointer_events("move", 1, 1),
            vec![SurfacePointerEvent::Move { x_px: 0, y_px: 0 }]
        );
        assert!(native_surface_pointer_events("unknown", 1, 1).is_empty());
    }

    #[test]
    fn native_mouse_event_mapping_preserves_drag_buttons() {
        assert_eq!(
            native_mouse_event_name_and_position(&InputEvent::MouseRelease {
                button: MouseButton::Right,
                col: 5,
                row: 6,
                mods: Default::default(),
            }),
            Some(("release-right", 5, 6, false))
        );
        assert_eq!(
            native_mouse_event_name_and_position(&InputEvent::MousePress {
                button: MouseButton::ScrollDown,
                col: 5,
                row: 6,
                mods: Default::default(),
            }),
            Some(("scroll-down", 5, 6, false))
        );
        assert_eq!(
            native_mouse_event_name_and_position(&InputEvent::MousePress {
                button: MouseButton::Left,
                col: 5,
                row: 6,
                mods: Default::default(),
            }),
            Some(("press-left", 5, 6, true))
        );
        assert_eq!(
            native_mouse_event_name_and_position(&InputEvent::MouseMove {
                button: MouseButton::Left,
                col: 5,
                row: 6,
                mods: Default::default(),
            }),
            Some(("move-left", 5, 6, false))
        );
        assert_eq!(
            native_mouse_event_name_and_position(&InputEvent::MouseMove {
                button: MouseButton::None,
                col: 5,
                row: 6,
                mods: Default::default(),
            }),
            Some(("move", 5, 6, false))
        );
    }

    #[test]
    fn raw_mode_sequences_restore_alt_cursor_mouse_and_focus_modes() {
        let enter = std::str::from_utf8(raw_mode_enter_sequence()).unwrap();
        let restore = std::str::from_utf8(raw_mode_restore_sequence()).unwrap();
        for enabled in [
            "?1049h", "?25l", "[2J", "[H", "?1000h", "?1002h", "?1003h", "?1004h", "?1006h",
        ] {
            assert!(enter.contains(enabled), "missing {enabled}: {enter:?}");
        }
        assert!(enter.find("[2J").unwrap() < enter.find("?1000h").unwrap());
        for disabled in [
            "?1006l", "?1004l", "?1003l", "?1002l", "?1000l", "?25h", "?1049l",
        ] {
            assert!(
                restore.contains(disabled),
                "missing {disabled}: {restore:?}"
            );
        }
        assert!(restore.find("?25h").unwrap() < restore.find("?1049l").unwrap());
        assert!(restore.find("?1006l").unwrap() < restore.find("?1000l").unwrap());
    }

    #[test]
    fn native_initial_frame_skips_explicit_clear_when_raw_enter_already_clears() {
        assert!(raw_mode_enter_sequence_clears_screen());
        assert!(!native_initial_frame_requires_explicit_clear());
    }

    #[cfg(unix)]
    #[test]
    fn raw_mode_iflag_preserves_raw_enter_flow_control_and_high_bit_bytes() {
        use libc::{BRKINT, ICRNL, IGNCR, INLCR, ISTRIP, IXOFF, IXON, PARMRK};
        let flags = raw_mode_iflag(ICRNL | IGNCR | INLCR | IXON | IXOFF | BRKINT | PARMRK | ISTRIP);
        assert_eq!(flags & ICRNL, 0);
        assert_eq!(flags & IGNCR, 0);
        assert_eq!(flags & INLCR, 0);
        assert_eq!(flags & IXON, 0);
        assert_eq!(flags & IXOFF, 0);
        assert_eq!(flags & BRKINT, 0);
        assert_eq!(flags & PARMRK, 0);
        assert_eq!(flags & ISTRIP, 0);
    }

    #[cfg(unix)]
    #[test]
    fn raw_mode_oflag_disables_output_post_processing() {
        use libc::{OCRNL, OPOST};
        let flags = raw_mode_oflag(OPOST | OCRNL);
        assert_eq!(flags & OPOST, 0);
        assert_ne!(flags & OCRNL, 0);
    }

    #[cfg(unix)]
    #[test]
    fn raw_mode_cflag_forces_eight_bit_characters() {
        use libc::{CLOCAL, CS7, CS8, CSIZE};
        let flags = raw_mode_cflag(CS7 | CLOCAL);
        assert_eq!(flags & CSIZE, CS8);
        assert_ne!(flags & CLOCAL, 0);
    }

    #[cfg(unix)]
    #[test]
    fn raw_mode_lflag_disables_signal_and_extended_line_processing() {
        use libc::{ECHO, ICANON, IEXTEN, ISIG};
        let flags = raw_mode_lflag(ICANON | ECHO | ISIG | IEXTEN);
        assert_eq!(flags & ICANON, 0);
        assert_eq!(flags & ECHO, 0);
        assert_eq!(flags & ISIG, 0);
        assert_eq!(flags & IEXTEN, 0);
    }

    #[test]
    fn clock_timestamp_line_builds_directly() {
        assert_eq!(clock_timestamp_line(7, 3), "7.003");
        assert_eq!(clock_timestamp_line(123, 456), "123.456");
        assert!(clock_timestamp_line(123, 456).capacity() >= "123.456".len());
    }

    #[test]
    fn native_ctrl_c_action_forwards_until_confirmation_threshold() {
        let start = Instant::now();
        let mut guard = NativeCtrlCExitGuard::default();
        assert_eq!(
            native_ctrl_c_action(&mut guard, start),
            NativeCtrlCAction::Forward
        );
        assert_eq!(
            native_ctrl_c_action(&mut guard, start + Duration::from_millis(500)),
            NativeCtrlCAction::Forward
        );
        assert_eq!(
            native_ctrl_c_action(&mut guard, start + Duration::from_millis(900)),
            NativeCtrlCAction::Confirm
        );
    }

    #[test]
    fn single_ctrl_c_never_confirms_outer_quit() {
        let start = Instant::now();
        let mut guard = NativeCtrlCExitGuard::default();

        assert_eq!(
            native_ctrl_c_action(&mut guard, start),
            NativeCtrlCAction::Forward
        );
        assert_eq!(
            native_ctrl_c_action(&mut guard, start + Duration::from_millis(250)),
            NativeCtrlCAction::Forward
        );
    }

    #[test]
    fn native_quit_confirm_byte_action_requires_explicit_yes() {
        let start = Instant::now();
        let mut overlay = QuitConfirmOverlay::default();
        overlay.open(start);
        assert_eq!(
            native_quit_confirm_byte_action(&mut overlay, 0x03, start),
            NativeQuitConfirmByteAction::Consumed
        );
        assert!(overlay.active);
        assert_eq!(
            native_quit_confirm_byte_action(&mut overlay, b'n', start),
            NativeQuitConfirmByteAction::Cancel
        );
        assert!(!overlay.active);
        overlay.open(start);
        assert_eq!(
            native_quit_confirm_byte_action(&mut overlay, b'y', start),
            NativeQuitConfirmByteAction::Confirm
        );
        overlay.open(start);
        assert_eq!(
            native_quit_confirm_byte_action(
                &mut overlay,
                b'x',
                start + QUIT_CONFIRM_TIMEOUT + Duration::from_millis(1),
            ),
            NativeQuitConfirmByteAction::Cancel
        );
    }

    #[test]
    fn native_ctrl_c_exit_guard_requires_three_presses_in_window() {
        let start = Instant::now();
        let mut guard = NativeCtrlCExitGuard::default();
        assert!(!guard.observe(start));
        assert!(!guard.observe(start + Duration::from_millis(500)));
        assert!(guard.observe(start + Duration::from_millis(900)));

        let mut guard = NativeCtrlCExitGuard::default();
        assert!(!guard.observe(start));
        assert!(!guard.observe(start + NATIVE_CTRL_C_EXIT_WINDOW + Duration::from_millis(1)));
        assert!(!guard.observe(start + NATIVE_CTRL_C_EXIT_WINDOW + Duration::from_millis(100)));
        assert!(guard.observe(start + NATIVE_CTRL_C_EXIT_WINDOW + Duration::from_millis(200)));

        guard.reset();
        assert!(!guard.observe(start + Duration::from_secs(10)));
    }

    #[test]
    fn native_focus_event_payloads_require_reporting() {
        let mut pane = dummy_native_pane("native-1", "sh", 1);
        assert!(native_send_focus_event(&mut pane, true));
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
    fn ansi_pane_chrome_write_guards_visible_bounds_and_pads() {
        let chrome = NativePaneChrome {
            x: 3,
            y: 1,
            focused: true,
            text: "sh".to_string(),
            cache_key: "key".to_string(),
            status: "status".to_string(),
            app_x: 3,
            app_y: 2,
            app_cols: 4,
            app_rows: 1,
            cols: 5,
            rows: 2,
            text_snapshot: String::new(),
        };
        let mut out = Vec::new();
        write_native_pane_chrome(&mut out, &chrome, 6, 4).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, "\x1b[2;4H\x1b[7msh \x1b[0m");

        let mut offscreen = Vec::new();
        let mut hidden = chrome.clone();
        hidden.y = 4;
        write_native_pane_chrome(&mut offscreen, &hidden, 6, 4).unwrap();
        assert!(offscreen.is_empty());
    }

    #[test]
    fn native_terminal_chrome_styles_are_precomputed_once_per_frame() {
        let colors = InlineChipColors {
            fg: Rgba::rgba(1, 2, 3, 255),
            fill: Rgba::rgba(4, 5, 6, 255),
            border: Rgba::rgba(7, 8, 9, 255),
            highlight: Rgba::rgba(0, 0, 0, 0),
        };
        let (top, focused, unfocused) = native_terminal_chrome_styles(colors);
        assert!(top.capacity() >= 38);
        assert_eq!(top, "\x1b[38;2;1;2;3m\x1b[48;2;4;5;6m");
        assert_eq!(focused, "\x1b[38;2;1;2;3m\x1b[48;2;7;8;9m");
        assert_eq!(unfocused, "\x1b[38;2;1;2;3m\x1b[48;2;4;5;6m");
    }

    #[test]
    fn native_shell_terminal_renderer_draws_chrome_and_snapshots() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: vec![NativePaneChrome {
                x: 0,
                y: 1,
                focused: true,
                text: "* native-1 shell".to_string(),
                cache_key: "key".to_string(),
                status: "shell · pid:101 · frame:new".to_string(),
                app_x: 0,
                app_y: 2,
                app_cols: 8,
                app_rows: 2,
                cols: 8,
                rows: 3,
                text_snapshot: "hello\nworld\nignored\n".to_string(),
            }],
            footer: NativeFooterChrome {
                row: 4,
                text: "footer".to_string(),
            },
            help_overlay: false,
        };
        let rendered = render_native_shell_view_terminal(&view, 12, 5);
        assert!(rendered.capacity() >= 12 * 5);
        assert!(rendered.contains("| 1 | 2 | 3"), "{rendered:?}");
        assert!(rendered.contains("\x1b[2;1H"), "{rendered:?}");
        assert!(rendered.contains("* native\x1b[0m"), "{rendered:?}");
        assert!(
            !rendered.contains("* native-1 shell\x1b[0m"),
            "{rendered:?}"
        );
        assert!(rendered.contains("\x1b[3;1Hhello   "), "{rendered:?}");
        assert!(rendered.contains("\x1b[4;1Hworld   "), "{rendered:?}");
        assert!(!rendered.contains("ignored"), "{rendered:?}");
        assert!(rendered.contains("footer"), "{rendered:?}");
    }

    #[test]
    fn native_shell_terminal_renderer_pads_pane_titles_to_visible_width() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "top".to_string(),
            },
            panes: vec![NativePaneChrome {
                x: 0,
                y: 1,
                focused: true,
                text: "sh".to_string(),
                cache_key: "key".to_string(),
                status: "status".to_string(),
                app_x: 0,
                app_y: 2,
                app_cols: 8,
                app_rows: 1,
                cols: 8,
                rows: 2,
                text_snapshot: String::new(),
            }],
            footer: NativeFooterChrome {
                row: 0,
                text: String::new(),
            },
            help_overlay: false,
        };
        let rendered = render_native_shell_view_terminal(&view, 10, 4);
        assert!(rendered.contains("\x1b[2;1H"), "{rendered:?}");
        assert!(rendered.contains("sh      \x1b[0m"), "{rendered:?}");
    }

    #[test]
    fn native_shell_terminal_renderer_skips_offscreen_rows_and_clips_width() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 99,
                text: "top".to_string(),
            },
            panes: vec![NativePaneChrome {
                x: 3,
                y: 99,
                focused: true,
                text: "title".to_string(),
                cache_key: "key".to_string(),
                status: "status".to_string(),
                app_x: 3,
                app_y: 1,
                app_cols: 10,
                app_rows: 2,
                cols: 10,
                rows: 3,
                text_snapshot: "abcdef\nghijkl".to_string(),
            }],
            footer: NativeFooterChrome {
                row: 0,
                text: String::new(),
            },
            help_overlay: false,
        };
        let rendered = render_native_shell_view_terminal(&view, 5, 2);
        assert!(!rendered.contains("top"), "{rendered:?}");
        assert!(!rendered.contains("title"), "{rendered:?}");
        assert!(rendered.contains("\x1b[2;4Hab"), "{rendered:?}");
        assert!(!rendered.contains("cdef"), "{rendered:?}");
        assert!(!rendered.contains("ghijkl"), "{rendered:?}");
        assert!(!rendered.contains("\x1b[100;"), "{rendered:?}");
    }

    #[test]
    fn native_shell_terminal_renderer_clamps_footer_to_visible_row() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "top".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 99,
                text: "footer".to_string(),
            },
            help_overlay: false,
        };
        let rendered = render_native_shell_view_terminal(&view, 20, 5);
        assert!(
            rendered.contains("\x1b[0m\x1b[5;1H\x1b[Kfooter"),
            "{rendered:?}"
        );
        assert!(!rendered.contains("\x1b[100;1H"), "{rendered:?}");
    }

    #[test]
    fn native_shell_terminal_renderer_draws_empty_workspace_top_bar_and_help() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 4,
                text: String::new(),
            },
            help_overlay: true,
        };
        let rendered = render_native_shell_view_terminal(&view, 40, 8);
        assert!(rendered.contains("| 1 | 2 | 3 |"), "{rendered:?}");
        assert!(!rendered.contains("kittui-bar"), "{rendered:?}");
        assert!(rendered.contains("kittwm shortcuts"), "{rendered:?}");
        assert!(!rendered.contains("footer"), "{rendered:?}");
    }

    #[test]
    fn native_shell_terminal_renderer_keeps_empty_workspace_minimal_without_help_overlay() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 4,
                text: String::new(),
            },
            help_overlay: false,
        };
        let rendered = render_native_shell_view_terminal(&view, 96, 8);
        assert!(!rendered.contains("Empty kittwm workspace"), "{rendered:?}");
        assert!(
            !rendered.contains("C-a Enter / C-a t opens a terminal"),
            "{rendered:?}"
        );
        assert!(!rendered.contains("kittwm quickstart"), "{rendered:?}");
    }

    #[test]
    fn native_shell_affordance_renderer_keeps_empty_workspace_minimal_by_default() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 8,
                text: String::new(),
            },
            help_overlay: false,
        };
        let scenes =
            render_native_shell_view_affordance_scenes(&view, CellSize::new(8, 16), 80, 24);
        assert_eq!(scenes.len(), 1, "{scenes:?}");
        assert_eq!(scenes[0].id, "top-bar");
    }

    #[test]
    fn native_status_title_drag_cell_reports_tiled_and_floating_modes() {
        let layout = NativePaneLayout {
            x: 10,
            y: 4,
            cols: 20,
            rows: 6,
            app_x: 10,
            app_y: 5,
            app_cols: 20,
            app_rows: 5,
        };
        assert!(native_status_title_draggable(NativePaneLayoutMode::Tiled));
        assert!(native_status_title_draggable(
            NativePaneLayoutMode::Floating
        ));
        assert!(!native_status_title_draggable(
            NativePaneLayoutMode::Fullscreen
        ));
        assert_eq!(
            native_status_title_drag_kind(NativePaneLayoutMode::Tiled),
            Some("reorder")
        );
        assert_eq!(
            native_status_title_drag_kind(NativePaneLayoutMode::Floating),
            Some("reposition")
        );
        assert_eq!(
            native_status_title_drag_kind(NativePaneLayoutMode::Fullscreen),
            None
        );
        assert_eq!(
            native_status_title_drag_cell(Some(layout), NativePaneLayoutMode::Tiled),
            Some((Some(12), Some(5)))
        );
        let tiny_layout = NativePaneLayout { cols: 1, ..layout };
        assert_eq!(
            native_status_title_drag_cell(Some(tiny_layout), NativePaneLayoutMode::Tiled),
            Some((Some(11), Some(5)))
        );
        assert_eq!(
            native_status_title_drag_cell(Some(layout), NativePaneLayoutMode::Floating),
            Some((Some(14), Some(5)))
        );
        assert_eq!(
            native_status_title_drag_cell(Some(layout), NativePaneLayoutMode::Fullscreen),
            Some((None, None))
        );
    }

    #[test]
    fn native_pane_status_chip_command_text_is_bounded() {
        let long = "cmd-".repeat(10_000);
        let bounded = bounded_ellipsis(&long, NATIVE_PANE_STATUS_COMMAND_MAX_CHARS);
        assert_eq!(
            bounded.chars().count(),
            NATIVE_PANE_STATUS_COMMAND_MAX_CHARS
        );
        assert!(bounded.ends_with('…'));
        assert!(bounded.capacity() >= NATIVE_PANE_STATUS_COMMAND_MAX_CHARS);
        let short = bounded_ellipsis("shell", NATIVE_PANE_STATUS_COMMAND_MAX_CHARS);
        assert_eq!(short, "shell");
        assert!(short.capacity() >= "shell".len());

        let mut pane = dummy_native_pane("native-1", &long, 1);
        pane.pid = Some(1234);
        pane.dirty_frame = Some(NativeDirtyFrameMetrics {
            changed_tiles: 7,
            total_tiles: 9,
            changed_fraction: 7.0 / 9.0,
            skipped_upload: false,
        });
        let chip = native_pane_status_chip_text(&pane);
        assert!(chip.starts_with(&bounded), "{chip}");
        assert!(chip.ends_with(" · pid:1234 · frame:7/9"), "{chip}");
        assert!(!chip.contains(&"cmd-".repeat(128)), "{chip}");
        assert!(chip.capacity() >= bounded.len() + 32);

        pane.weight = 3;
        let resized_chip = native_pane_status_chip_text(&pane);
        assert!(
            resized_chip.contains(" · wt:3 · pid:1234"),
            "{resized_chip}"
        );

        pane.dirty_frame = Some(NativeDirtyFrameMetrics {
            changed_tiles: 0,
            total_tiles: 9,
            changed_fraction: 0.0,
            skipped_upload: false,
        });
        let clean_chip = native_pane_status_chip_text(&pane);
        assert!(clean_chip.ends_with(" · frame:clean"), "{clean_chip}");
        assert!(pane.dirty_frame.as_ref().unwrap().is_clean());

        let metrics = NativeDirtyFrameMetrics {
            changed_tiles: 5,
            total_tiles: 8,
            changed_fraction: 5.0 / 8.0,
            skipped_upload: false,
        };
        assert_eq!(metrics.changed_tiles_label(), "5/8");
    }

    #[test]
    fn native_pane_title_text_builds_only_visible_prefix() {
        let mut pane = dummy_native_pane("native-1", "sh", 1);
        pane.display_title = Some("title-".repeat(10_000));
        let text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            false,
            false,
            false,
            false,
            true,
            false,
        );
        assert_eq!(text, "▶ native-1 title");
        assert_eq!(text.chars().count(), 16);
        assert!(text.capacity() >= 16);
        assert!(!text.contains(&"title-".repeat(2)), "{text}");

        let reorder_text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            false,
            false,
            true,
            false,
            true,
            false,
        );
        assert_eq!(reorder_text, "▶⇄ native-1 titl");
        assert_eq!(reorder_text.chars().count(), 16);

        pane.weight = 3;
        let resized_text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            false,
            false,
            true,
            false,
            true,
            false,
        );
        assert_eq!(resized_text, "▶⇄↔ native-1 tit");
        assert_eq!(resized_text.chars().count(), 16);
        assert_eq!(
            native_pane_title_text(
                &pane,
                NativePaneLayout {
                    x: 0,
                    y: 0,
                    cols: 0,
                    rows: 0,
                    app_x: 0,
                    app_y: 0,
                    app_cols: 0,
                    app_rows: 0,
                },
                true,
                false,
                false,
                false,
                false,
                true,
                false,
            ),
            ""
        );

        let fullscreen_resized_text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            false,
            false,
            false,
            true,
            true,
            false,
        );
        assert_eq!(fullscreen_resized_text, "▶▣ native-1 titl");
        assert!(!fullscreen_resized_text.contains('↔'));

        pane.weight = 1;
        let fullscreen_text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            false,
            false,
            false,
            true,
            true,
            false,
        );
        assert_eq!(fullscreen_text, "▶▣ native-1 titl");
        assert_eq!(fullscreen_text.chars().count(), 16);

        let active_reorder_text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            true,
            false,
            true,
            false,
            true,
            false,
        );
        assert_eq!(active_reorder_text, "▶◆⇄ native-1 tit");

        let floating_text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            false,
            true,
            false,
            false,
            true,
            true,
        );
        assert_eq!(floating_text, "▶≡▲● native-1 ti");
        assert_eq!(floating_text.chars().count(), 16);
        let lower_floating_text = native_pane_title_text(
            &pane,
            NativePaneLayout {
                x: 0,
                y: 0,
                cols: 16,
                rows: 4,
                app_x: 0,
                app_y: 1,
                app_cols: 16,
                app_rows: 3,
            },
            true,
            false,
            true,
            false,
            false,
            false,
            false,
        );
        assert_eq!(lower_floating_text, "▶≡   native-1 ti");
        assert!(native_title_drag_affordance_enabled("floating"));
        assert!(!native_title_drag_affordance_enabled("columns"));
        assert!(native_title_reorder_affordance_enabled("columns"));
        assert!(native_title_reorder_affordance_enabled("Rows"));
        assert!(native_title_reorder_affordance_enabled(" grid "));
        assert!(native_title_reorder_affordance_enabled("TILED"));
        assert!(!native_title_reorder_affordance_enabled("floating"));
        assert!(native_title_fullscreen_affordance_enabled(" fullscreen "));
        assert!(!native_title_fullscreen_affordance_enabled("rows"));
    }

    #[test]
    fn native_pane_title_status_chip_stays_inside_tiny_width() {
        let pane = NativePaneChrome {
            x: 0,
            y: 1,
            focused: true,
            text: "* tiny".to_string(),
            cache_key: "tiny".to_string(),
            status: "status".to_string(),
            app_x: 0,
            app_y: 2,
            app_cols: 1,
            app_rows: 3,
            cols: 1,
            rows: 4,
            text_snapshot: String::new(),
        };
        let scene = native_pane_title_status_scene(0, &pane, CellSize::new(8, 16));
        assert_eq!(scene.footprint.cols, 1);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        let chip = scene
            .layers
            .iter()
            .find(|layer| {
                layer
                    .label
                    .as_deref()
                    .unwrap_or_default()
                    .starts_with("pane-0-status-chip:")
            })
            .expect("status chip layer");
        let Node::Rect { rect, .. } = &chip.root else {
            panic!("expected rect");
        };
        assert!(rect.origin.0 >= 0.0, "{rect:?}");
        assert!(rect.width >= 1.0, "{rect:?}");
        assert!(
            rect.origin.0 + rect.width <= width + 0.01,
            "{rect:?} > {width}"
        );
    }

    #[test]
    fn native_pane_focus_glow_label_builds_directly() {
        let label = native_pane_focus_glow_label(7);
        assert_eq!(label, "pane-7-focus-glow");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_title_gutter_label_builds_directly() {
        let label = native_pane_title_gutter_label(7);
        assert_eq!(label, "pane-7-title-gutter");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_focus_accent_rail_label_builds_directly() {
        let label = native_pane_focus_accent_rail_label(7);
        assert_eq!(label, "pane-7-focus-accent-rail");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_kittui_border_label_builds_directly() {
        let label = native_pane_kittui_border_label(7);
        assert_eq!(label, "pane-7-kittui-border");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_focus_ring_label_builds_directly() {
        let label = native_pane_focus_ring_label(7);
        assert_eq!(label, "pane-7-focus-ring");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_title_focus_marker_label_builds_directly() {
        let label = native_pane_title_focus_marker_label(7);
        assert_eq!(label, "pane-7-title-focus-marker");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_title_strip_label_builds_directly() {
        let label = native_pane_title_strip_label(7, "* native-7 editor");
        assert_eq!(label, "pane-7-title-strip:* native-7 editor");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_status_chip_label_builds_directly() {
        let label = native_pane_status_chip_label(7, "shell · pid:101");
        assert_eq!(label, "pane-7-status-chip:shell · pid:101");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_pane_border_scene_id_builds_directly() {
        let id = native_pane_border_scene_id(7);
        assert_eq!(id, "pane-7-border");
        assert!(id.capacity() >= id.len());
    }

    #[test]
    fn native_shell_affordance_renderer_builds_kittui_scenes() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: vec![
                NativePaneChrome {
                    x: 0,
                    y: 1,
                    focused: true,
                    text: "* native-1 shell".to_string(),
                    cache_key: "key1".to_string(),
                    status: "shell · pid:101 · frame:new".to_string(),
                    app_x: 0,
                    app_y: 2,
                    app_cols: 8,
                    app_rows: 2,
                    cols: 8,
                    rows: 3,
                    text_snapshot: "hello".to_string(),
                },
                NativePaneChrome {
                    x: 8,
                    y: 1,
                    focused: false,
                    text: "  native-2 logs".to_string(),
                    cache_key: "key2".to_string(),
                    status: "logs · pid:102 · frame:clean".to_string(),
                    app_x: 8,
                    app_y: 2,
                    app_cols: 10,
                    app_rows: 2,
                    cols: 10,
                    rows: 3,
                    text_snapshot: "logs".to_string(),
                },
            ],
            footer: NativeFooterChrome {
                row: 4,
                text: "footer".to_string(),
            },
            help_overlay: false,
        };
        let scenes =
            render_native_shell_view_affordance_scenes(&view, CellSize::new(8, 16), 18, 24);
        assert_eq!(scenes.len(), 4);
        assert_eq!(scenes[0].id, "top-bar");
        assert_eq!((scenes[0].x, scenes[0].y), (0, 0));
        assert!(scenes[0]
            .scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("kittwm-live-top-bar:active:1")));
        assert_eq!(scenes[1].id, "pane-0-border");
        assert_eq!((scenes[2].x, scenes[2].y), (8, 1));
        assert_eq!(scenes[3].id, "footer");
        assert!(scenes[3].scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .starts_with("status-bar-backdrop:footer")));
        assert!(scenes[3]
            .scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("status-chip-help")));
        assert!(scenes.iter().all(|scene| scene.id != "toast"));
        assert!(scenes[1].scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .starts_with("pane-0-title-strip:* native-1 shell")));
        assert!(scenes[1].scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .starts_with("pane-0-status-chip:shell · pid:101")));
        let focused_border_labels = scenes[1]
            .scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            focused_border_labels.contains(&"pane-0-kittui-border"),
            "{focused_border_labels:?}"
        );
        assert!(
            focused_border_labels.contains(&"pane-0-focus-glow"),
            "{focused_border_labels:?}"
        );
        assert!(
            focused_border_labels.contains(&"pane-0-focus-accent-rail"),
            "{focused_border_labels:?}"
        );
        assert!(
            focused_border_labels.contains(&"pane-0-focus-ring"),
            "{focused_border_labels:?}"
        );
        let unfocused_border_labels = scenes[2]
            .scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            !unfocused_border_labels
                .iter()
                .any(|label| label.contains("focus-")),
            "{unfocused_border_labels:?}"
        );
        let colors = native_glass_chrome_colors();
        let title_gutter = scenes[1]
            .scene
            .layers
            .iter()
            .find(|layer| layer.label.as_deref() == Some("pane-0-title-gutter"))
            .expect("focused border scene has title gutter");
        match &title_gutter.root {
            Node::Rect {
                fill: Paint::Solid { color },
                ..
            } => {
                assert_eq!(*color, colors.fill);
                assert!(
                    color.3 < 255,
                    "expected translucent title gutter: {color:?}"
                );
            }
            node => panic!("expected pane title gutter rect, got {node:?}"),
        }
        assert!(scenes.iter().all(|chrome| !chrome.scene.layers.is_empty()));
    }

    #[test]
    fn native_shell_chrome_scene_key_tracks_placement_and_scene_identity() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 4,
                text: String::new(),
            },
            help_overlay: false,
        };
        let mut scenes =
            render_native_shell_view_affordance_scenes(&view, CellSize::new(8, 16), 80, 24);
        let baseline = native_shell_chrome_scene_key(&scenes[0]);
        assert_eq!(baseline, native_shell_chrome_scene_key(&scenes[0]));

        scenes[0].x = 1;
        assert_ne!(baseline, native_shell_chrome_scene_key(&scenes[0]));

        let mut changed_scene = scenes[0].clone();
        changed_scene.x = 0;
        changed_scene.scene.layers[0].label = Some("changed-top-bar-state".to_string());
        assert_ne!(baseline, native_shell_chrome_scene_key(&changed_scene));

        let mut huge_label_scene = scenes[0].clone();
        huge_label_scene.scene.layers[0].label = Some("label-".repeat(10_000));
        let key = native_shell_chrome_scene_key(&huge_label_scene);
        assert!(key.len() < 128, "{key}");
        assert!(!key.contains(&"label-".repeat(8)), "{key}");
    }

    #[test]
    fn native_shell_view_affordance_chrome_key_skips_unchanged_view() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: vec![NativePaneChrome {
                x: 0,
                y: 1,
                focused: true,
                text: "* native-1 shell".to_string(),
                cache_key: "title-key".to_string(),
                status: "shell · pid:101 · frame:clean".to_string(),
                app_x: 1,
                app_y: 2,
                app_cols: 18,
                app_rows: 5,
                cols: 20,
                rows: 7,
                text_snapshot: "ignored by chrome key".to_string(),
            }],
            footer: NativeFooterChrome {
                row: 4,
                text: "footer".to_string(),
            },
            help_overlay: false,
        };
        let baseline = native_shell_view_affordance_chrome_key(&view, 80, 24);
        assert_eq!(
            baseline,
            native_shell_view_affordance_chrome_key(&view, 80, 24)
        );

        let mut changed_text_snapshot = view.clone();
        changed_text_snapshot.panes[0].text_snapshot = "app output changed".to_string();
        assert_eq!(
            baseline,
            native_shell_view_affordance_chrome_key(&changed_text_snapshot, 80, 24)
        );

        let mut changed_status = view.clone();
        changed_status.panes[0].status = "shell · pid:101 · frame:4".to_string();
        assert_ne!(
            baseline,
            native_shell_view_affordance_chrome_key(&changed_status, 80, 24)
        );
        assert_ne!(
            baseline,
            native_shell_view_affordance_chrome_key(&view, 81, 24)
        );
    }

    #[test]
    fn native_shell_chrome_scene_key_tracks_visual_paint_changes() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 4,
                text: String::new(),
            },
            help_overlay: false,
        };
        let scenes =
            render_native_shell_view_affordance_scenes(&view, CellSize::new(8, 16), 80, 24);
        let mut changed_scene = scenes[0].clone();
        let baseline = native_shell_chrome_scene_key(&changed_scene);
        match &mut changed_scene.scene.layers[0].root {
            Node::Rect {
                fill: Paint::Solid { color },
                ..
            } => *color = Rgba(1, 2, 3, 255),
            Node::Gradient { stops, .. } => stops[0].color = Rgba(1, 2, 3, 255),
            node => panic!("expected top-bar visual paint node, got {node:?}"),
        }
        assert_ne!(baseline, native_shell_chrome_scene_key(&changed_scene));
    }

    #[test]
    fn native_alpha_chrome_layers_are_translucent_and_ordered() {
        let pane = NativePaneChrome {
            x: 0,
            y: 1,
            focused: true,
            text: "* native-1 shell".to_string(),
            cache_key: "key".to_string(),
            status: "shell · pid:101 · frame:clean".to_string(),
            app_x: 1,
            app_y: 2,
            app_cols: 18,
            app_rows: 5,
            cols: 20,
            rows: 7,
            text_snapshot: String::new(),
        };
        let border = native_pane_border_scene(0, &pane, native_cell_size());
        let labels = border
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(labels.first(), Some(&"pane-0-focus-glow"));
        assert_eq!(labels.last(), Some(&"pane-0-focus-ring"));
        for label in ["pane-0-focus-glow", "pane-0-title-gutter"] {
            let layer = border
                .layers
                .iter()
                .find(|layer| layer.label.as_deref() == Some(label))
                .unwrap();
            match &layer.root {
                Node::Rect {
                    fill: Paint::Solid { color },
                    ..
                } => assert!(color.3 < 255, "{label} should be translucent: {color:?}"),
                node => panic!("expected rect for {label}, got {node:?}"),
            }
        }

        let (_x, _y, overlay) = native_help_overlay_scene(
            native_cell_size(),
            80,
            24,
            &["kittwm shortcuts", "C-a ? help"],
        );
        match &overlay.layers[0].root {
            Node::Rect {
                fill: Paint::Solid { color },
                ..
            } => assert!(
                color.3 < 255,
                "overlay backdrop should be translucent: {color:?}"
            ),
            node => panic!("expected overlay backdrop rect, got {node:?}"),
        }
    }

    #[test]
    fn graphical_command_palette_scene_maps_daily_driver_actions() {
        let scene = command_palette_scene("split", 1, native_cell_size());
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("command-palette-backdrop:kittwm command palette")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("command-palette-row-0:split-columns")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("command-palette-row-1:split-rows")),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"command-palette-footer-hints"),
            "{labels:?}"
        );
    }

    #[test]
    fn graphical_command_palette_title_builds_directly() {
        let title = graphical_command_palette_title("split");
        assert_eq!(title, "kittwm command palette query=split");
        assert!(title.capacity() >= title.len());
    }

    #[test]
    fn graphical_launcher_overlay_candidate_row_builds_directly() {
        let candidate = LauncherSelection {
            kind: LauncherKind::Shell,
            command: "kittwm-terminal".to_string(),
        };
        let row = graphical_launcher_overlay_candidate_row(1, &candidate);
        assert_eq!(row, "2. [shell] kittwm-terminal");
        assert!(row.capacity() >= row.len());
    }

    #[test]
    fn graphical_launcher_overlay_title_builds_directly() {
        let title = graphical_launcher_overlay_title("term");
        assert_eq!(title, "kittwm launcher query=term");
        assert!(title.capacity() >= title.len());
    }

    #[test]
    fn graphical_overlay_panel_heading_label_builds_directly() {
        let label = graphical_overlay_panel_heading_label("launcher-overlay");
        assert_eq!(label, "launcher-overlay-heading");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn graphical_overlay_panel_backdrop_label_builds_directly() {
        let label = graphical_overlay_panel_backdrop_label("launcher-overlay", "kittwm launcher");
        assert_eq!(label, "launcher-overlay-backdrop:kittwm launcher");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn graphical_overlay_panel_footer_label_builds_directly() {
        let label = graphical_overlay_panel_footer_label("launcher-overlay");
        assert_eq!(label, "launcher-overlay-footer-hints");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn graphical_overlay_panel_row_label_builds_directly() {
        let label =
            graphical_overlay_panel_row_label("launcher-overlay", 1, "2. [shell] kittwm-terminal");
        assert_eq!(label, "launcher-overlay-row-1:2. [shell] kittwm-terminal");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn picker_mac_window_entry_builds_directly() {
        let entry = picker_mac_window_entry("Safari", "Docs");
        assert_eq!(entry, "mac: Safari — Docs");
        assert!(entry.capacity() >= entry.len());
    }

    #[test]
    fn graphical_launcher_and_picker_overlay_scenes_expose_selection_rows() {
        let launcher = LauncherOverlay {
            active: true,
            query: "term".to_string(),
            selected: 1,
        };
        let candidates = vec![
            LauncherSelection {
                kind: LauncherKind::Path,
                command: "bash".to_string(),
            },
            LauncherSelection {
                kind: LauncherKind::Shell,
                command: "kittwm-terminal".to_string(),
            },
        ];
        let scene =
            launcher_overlay_scene_for_candidates(&launcher, &candidates, native_cell_size());
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("launcher-overlay-backdrop:kittwm launcher")),
            "{labels:?}"
        );
        assert!(labels.iter().any(|label| label
            .starts_with("launcher-overlay-row-1:2. [shell] kittwm-terminal")), "{labels:?}");
        assert!(
            labels.contains(&"launcher-overlay-footer-hints"),
            "{labels:?}"
        );

        let mut picker = PickerOverlay::default();
        picker.open();
        picker.selected = 1;
        let scene = picker_overlay_scene(&picker, native_cell_size());
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.contains(&"picker-overlay-backdrop:kittwm picker"),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("picker-overlay-row-1:backend: kittwm-browser")),
            "{labels:?}"
        );
    }

    #[test]
    fn native_live_top_bar_defaults_to_kittui_bar_scene_metadata() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_NATIVE_CHROME_RENDERER");
        assert!(native_should_use_affordance_scene_chrome());
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: vec![
                NativePaneChrome {
                    x: 0,
                    y: 1,
                    focused: true,
                    text: "* native-1 shell".to_string(),
                    cache_key: "key1".to_string(),
                    status: "shell · pid:101 · frame:new".to_string(),
                    app_x: 0,
                    app_y: 2,
                    app_cols: 10,
                    app_rows: 4,
                    cols: 10,
                    rows: 5,
                    text_snapshot: String::new(),
                },
                NativePaneChrome {
                    x: 10,
                    y: 1,
                    focused: false,
                    text: "  native-2 logs".to_string(),
                    cache_key: "key2".to_string(),
                    status: "logs · pid:102 · frame:clean".to_string(),
                    app_x: 10,
                    app_y: 2,
                    app_cols: 10,
                    app_rows: 4,
                    cols: 10,
                    rows: 5,
                    text_snapshot: String::new(),
                },
            ],
            footer: NativeFooterChrome {
                row: 7,
                text: String::new(),
            },
            help_overlay: false,
        };
        let scene = native_top_bar_scene(&view, 40, CellSize::new(8, 16));
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.contains(&"kittwm-live-top-bar:active:1"),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("kittwm-live-top-bar-text:|[1]|")),
            "{labels:?}"
        );
        assert_eq!(scene.footprint.rows, 1);
        assert_eq!(scene.footprint.cols, 40);
    }

    #[test]
    fn native_pane_window_chrome_scenes_align_with_app_bounds() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: vec![NativePaneChrome {
                x: 4,
                y: 3,
                focused: true,
                text: "* native-7 editor".to_string(),
                cache_key: "key".to_string(),
                status: "editor · pid:707 · frame:clean".to_string(),
                app_x: 4,
                app_y: 4,
                app_cols: 20,
                app_rows: 6,
                cols: 20,
                rows: 7,
                text_snapshot: String::new(),
            }],
            footer: NativeFooterChrome {
                row: 12,
                text: String::new(),
            },
            help_overlay: false,
        };
        let scenes =
            render_native_shell_view_affordance_scenes(&view, CellSize::new(8, 16), 80, 24);
        let border = scenes
            .iter()
            .find(|scene| scene.id == "pane-0-border")
            .unwrap();
        assert_eq!((border.x, border.y), (4, 3));
        assert_eq!(border.scene.footprint.cols, view.panes[0].cols);
        assert_eq!(border.scene.footprint.rows, view.panes[0].rows);
        assert!(border.scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .starts_with("pane-0-title-strip:* native-7 editor")));
        assert_eq!(view.panes[0].app_y, view.panes[0].y + 1);
        assert_eq!(view.panes[0].app_cols, view.panes[0].cols);
    }

    #[test]
    fn native_graphical_pane_overlay_writes_title_and_status_text() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: vec![NativePaneChrome {
                x: 4,
                y: 3,
                focused: true,
                text: "* native-7 editor".to_string(),
                cache_key: "key".to_string(),
                status: "editor · pid:707 · frame:clean".to_string(),
                app_x: 4,
                app_y: 4,
                app_cols: 20,
                app_rows: 6,
                cols: 40,
                rows: 7,
                text_snapshot: String::new(),
            }],
            footer: NativeFooterChrome {
                row: 12,
                text: String::new(),
            },
            help_overlay: false,
        };
        let mut out = Vec::new();
        write_native_graphical_pane_text_overlays(&mut out, &view, 80, 24).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("\x1b[4;5H"), "{text:?}");
        assert!(text.contains("* native-7 editor"), "{text:?}");
        assert!(text.contains("editor · pid:707"), "{text:?}");
    }

    #[test]
    fn native_graphical_pane_status_overlay_width_avoids_tiny_titles() {
        assert_eq!(native_graphical_pane_status_overlay_cols("status", 8), 0);
        assert_eq!(native_graphical_pane_status_overlay_cols("status", 16), 8);
        assert_eq!(
            native_graphical_pane_status_overlay_cols("long-status-label", 40),
            19
        );
    }

    #[test]
    fn native_graphical_top_bar_uses_ansi_shortcut_overlay_text() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 8,
                text: String::new(),
            },
            help_overlay: true,
        };
        let scenes =
            render_native_shell_view_affordance_scenes(&view, CellSize::new(8, 16), 80, 24);
        let top_bar = scenes.iter().find(|scene| scene.id == "top-bar").unwrap();
        assert!(top_bar.scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .starts_with("kittwm-live-top-bar:")));
        assert!(scenes.iter().all(|scene| scene.id != "help-overlay"));

        let fallback = render_native_shell_view_terminal(&view, 80, 12);
        assert!(fallback.contains("| 1 | 2 | 3 |"), "{fallback:?}");
        assert!(fallback.contains("kittwm shortcuts"), "{fallback:?}");
    }

    #[test]
    fn graphical_help_overlay_path_emits_ansi_help_text() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 8,
                text: String::new(),
            },
            help_overlay: true,
        };
        let runtime = Runtime::builder()
            .terminal(kittui::TerminalInfo::override_with(
                Some(80),
                Some(24),
                CellSize::new(8, 16),
                true,
                true,
                kittui_core::terminal::Transport::Direct,
            ))
            .build()
            .unwrap();
        let mut out = Vec::new();
        let mut last = HashMap::new();
        write_native_shell_affordance_chrome(&mut out, &runtime, &view, 80, 24, &mut last).unwrap();
        write_native_help_overlay(&mut out, 80, 24).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("_G"), "{text:?}");
        assert!(text.contains("z=-1"), "{text:?}");
        assert!(text.contains("kittwm shortcuts"), "{text:?}");
        assert!(!last.contains_key("help-overlay"));
    }

    #[test]
    fn native_tui_smoke_matrix_json_lists_common_tui_capabilities() {
        let matrix: serde_json::Value =
            serde_json::from_str(&native_tui_smoke_matrix_json().unwrap()).unwrap();
        assert_eq!(matrix["kind"], "kittwm-tui-smoke-matrix");
        let cases = matrix["cases"].as_array().unwrap();
        for id in [
            "shell-prompts",
            "cursor-addressing",
            "alternate-screen",
            "colors",
            "box-drawing",
            "mouse-sgr",
            "bracketed-paste",
            "ctrl-c",
            "real-fonts",
        ] {
            assert!(
                cases.iter().any(|case| case["id"] == id),
                "missing {id}: {cases:?}"
            );
        }
        assert!(cases
            .iter()
            .any(|case| case["id"] == "real-fonts" && case["status"] == "covered"));
    }

    #[test]
    fn native_showcase_composition_json_orders_app_frames_below_chrome_and_overlays() {
        let value: serde_json::Value =
            serde_json::from_str(&native_showcase_composition_json(96, 24, true).unwrap()).unwrap();
        assert_eq!(value["kind"], "kittwm-shell-composition");
        let entries = value["entries"].as_array().unwrap();
        let app = entries
            .iter()
            .find(|entry| entry["id"] == "pane-0-app-frame")
            .unwrap();
        let chrome = entries
            .iter()
            .find(|entry| entry["id"] == "pane-0-border")
            .unwrap();
        let overlay = entries
            .iter()
            .find(|entry| entry["id"] == "help-overlay")
            .unwrap();
        assert_eq!(app["kind"], "app-frame");
        assert_eq!(chrome["kind"], "chrome");
        assert_eq!(overlay["kind"], "overlay");
        assert!(app["z"].as_u64().unwrap() < chrome["z"].as_u64().unwrap());
        assert!(chrome["z"].as_u64().unwrap() < overlay["z"].as_u64().unwrap());
        assert!(app["x"].as_u64().unwrap() > chrome["x"].as_u64().unwrap());
        assert!(app["cols"].as_u64().unwrap() < chrome["cols"].as_u64().unwrap());
    }

    #[test]
    fn native_showcase_metrics_json_reports_scene_layer_and_pixel_budget() {
        let metrics: serde_json::Value =
            serde_json::from_str(&native_showcase_metrics_json(96, 24, true).unwrap()).unwrap();
        assert_eq!(metrics["kind"], "kittwm-showcase-metrics");
        assert_eq!(metrics["cols"], 96);
        assert_eq!(metrics["rows"], 24);
        assert_eq!(metrics["help_overlay"], true);
        assert_eq!(metrics["scene_count"], 4);
        assert!(metrics["layer_count"].as_u64().unwrap() >= 20, "{metrics}");
        assert!(metrics["total_pixels"].as_u64().unwrap() > 0, "{metrics}");
        assert_eq!(metrics["cell_width_px"], native_cell_width_px());
        assert_eq!(metrics["cell_height_px"], native_cell_height_px());
    }

    #[test]
    fn native_toast_test_stress_strings_build_directly() {
        let prefixed = native_toast_test_prefixed_repeat("launcher.error ", 'x', 4);
        assert_eq!(prefixed, "launcher.error xxxx");
        assert!(prefixed.capacity() >= prefixed.len());

        let suffixed = native_toast_test_repeat_suffixed('x', 4, "FAILED");
        assert_eq!(suffixed, "xxxxFAILED");
        assert!(suffixed.capacity() >= suffixed.len());
    }

    #[test]
    fn native_toast_label_builds_directly() {
        let backdrop = native_toast_label("toast-backdrop:", "launcher.error boom");
        assert_eq!(backdrop, "toast-backdrop:launcher.error boom");
        assert!(backdrop.capacity() >= backdrop.len());

        let text = native_toast_label("toast-text:", "launcher.error boom");
        assert_eq!(text, "toast-text:launcher.error boom");
        assert!(text.capacity() >= text.len());
    }

    #[test]
    fn native_toast_cols_fit_terminal_width() {
        assert_eq!(native_toast_cols(5, 0), 1);
        assert_eq!(native_toast_cols(5, 1), 1);
        assert_eq!(native_toast_cols(5, 8), 8);
        assert_eq!(native_toast_cols(5, 19), 19);
        assert_eq!(native_toast_cols(5, 80), 20);
        assert_eq!(native_toast_cols(u16::MAX, 80), 76);
    }

    #[test]
    fn native_toast_scene_fits_tiny_terminal_width() {
        for cols in [0, 1, 8, 19] {
            let (_x, _y, scene) =
                native_toast_scene(CellSize::new(8, 16), cols, 24, "launcher.error boom").unwrap();
            assert!(
                scene.footprint.cols <= cols.max(1),
                "cols={cols} scene={:?}",
                scene.footprint
            );
        }
    }

    #[test]
    fn native_toast_scene_saturates_long_message_width() {
        let long =
            native_toast_test_prefixed_repeat("launcher.error ", 'x', u16::MAX as usize + 4096);
        assert_eq!(native_toast_message_cols(&long), u16::MAX);
        let (_x, _y, scene) = native_toast_scene(CellSize::new(8, 16), 80, 24, &long).unwrap();
        assert_eq!(scene.footprint.cols, 76);
        assert_eq!(scene.footprint.rows, 3);
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or("")
            .starts_with("toast-text:")));
    }

    #[test]
    fn native_footer_toast_is_only_for_transient_errors() {
        assert!(!native_should_show_footer_toast(
            "C-a ? help · C-a g launcher · C-a Enter term"
        ));
        assert!(native_should_show_footer_toast(
            "launcher.error no candidate"
        ));
        assert!(native_should_show_footer_toast("capture denied"));
        assert!(native_should_show_footer_toast("backend failed"));
        assert!(native_should_show_footer_toast("LAUNCHER.ERROR boom"));
        assert!(ascii_contains_any_ignore_case(
            &native_toast_test_repeat_suffixed('x', 4096, "FAILED"),
            NATIVE_TOAST_TRIGGER_KEYWORDS
        ));
        assert!(!ascii_contains_any_ignore_case(
            &native_toast_test_repeat_suffixed('x', 4096, "healthy"),
            NATIVE_TOAST_TRIGGER_KEYWORDS
        ));
    }

    #[test]
    fn native_showcase_scene_json_exports_reviewable_shell_artifact() {
        let json = native_showcase_scene_json(96, 24, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let scenes = value.as_array().unwrap();
        let ids = scenes
            .iter()
            .filter_map(|scene| scene["id"].as_str())
            .collect::<Vec<_>>();
        for id in ["top-bar", "pane-0-border", "pane-1-border", "footer"] {
            assert!(ids.contains(&id), "missing {id}: {ids:?}");
        }
        assert!(scenes
            .iter()
            .all(|scene| scene["scene"]["layers"].is_array()));
    }

    #[test]
    fn native_showcase_scene_signature_matches_visual_golden() {
        let json = native_showcase_scene_json(96, 24, true).unwrap();
        let actual = native_showcase_scene_signature(&json);
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/kittwm_showcase_scene_signature.json"
        ))
        .unwrap();
        assert_eq!(actual, expected);
    }

    fn native_showcase_scene_signature(json: &str) -> serde_json::Value {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        serde_json::Value::Array(
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| {
                    serde_json::json!({
                        "id": entry["id"],
                        "x": entry["x"],
                        "y": entry["y"],
                        "layers": entry["scene"]["layers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|layer| layer["label"].clone())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>(),
        )
    }

    fn native_toast_test_prefixed_repeat(prefix: &str, ch: char, count: usize) -> String {
        let mut out = String::with_capacity(prefix.len() + count * ch.len_utf8());
        out.push_str(prefix);
        out.extend(std::iter::repeat_n(ch, count));
        out
    }

    fn native_toast_test_repeat_suffixed(ch: char, count: usize, suffix: &str) -> String {
        let mut out = String::with_capacity(count * ch.len_utf8() + suffix.len());
        out.extend(std::iter::repeat_n(ch, count));
        out.push_str(suffix);
        out
    }

    #[test]
    fn native_toast_colors_follow_configured_chrome_fill() {
        let base = native_glass_chrome_colors();
        let normal = native_toast_colors("hello");
        assert_eq!(normal.fill, base.fill);
        assert_eq!(normal.fg, base.fg);
        assert_eq!(normal.border, base.border);
        let error = native_toast_colors("launcher.error failed");
        assert_eq!(error.fill, base.fill);
        assert_eq!(error.fg, base.fg);
        assert_ne!(error.border, base.border);
    }

    #[test]
    fn native_footer_status_backdrop_label_builds_directly() {
        let label = native_footer_status_backdrop_label("footer");
        assert_eq!(label, "status-bar-backdrop:footer");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_footer_status_chip_label_builds_directly() {
        let label = native_footer_status_chip_label("help");
        assert_eq!(label, "status-chip-help");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_footer_status_scene_labels_focus_and_state_chips() {
        let scene = native_footer_status_scene(
            CellSize::new(8, 16),
            80,
            " mode:floating · panes:1 · focus:native-1 · state:sh · pid:101 · frame:clean",
        );
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"status-chip-help"), "{labels:?}");
        assert!(labels.contains(&"status-chip-focus"), "{labels:?}");
        assert!(labels.contains(&"status-chip-state"), "{labels:?}");
        assert!(labels.contains(&"status-chip-drag"), "{labels:?}");
    }

    #[test]
    fn native_footer_status_scene_fits_tiny_terminals() {
        for cols in [0, 1, 8, 19] {
            let scene = native_footer_status_scene(CellSize::new(8, 16), cols, "status");
            assert!(
                scene.footprint.cols <= cols.max(1),
                "cols={cols} scene={:?}",
                scene.footprint
            );
            let scene_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
            for layer in &scene.layers {
                let Some(rect) = (match &layer.root {
                    Node::Rect { rect, .. } | Node::Gradient { rect, .. } => Some(rect),
                    _ => None,
                }) else {
                    continue;
                };
                assert!(
                    rect.origin.0 >= 0.0 && rect.origin.0 + rect.width <= scene_width,
                    "layer {:?} escapes width {scene_width}: {:?}",
                    layer.label,
                    rect
                );
            }
        }
    }

    #[test]
    fn native_footer_status_scene_label_is_bounded() {
        let long_status = native_footer_test_long_status(10_000);
        let scene = native_footer_status_scene(CellSize::new(8, 16), 80, &long_status);
        let label = scene.layers[0].label.as_deref().unwrap_or_default();
        assert!(label.starts_with("status-bar-backdrop:log: "), "{label}");
        assert!(label.ends_with('…'), "{label}");
        assert!(
            label.chars().count()
                <= "status-bar-backdrop:".chars().count() + NATIVE_FOOTER_STATUS_LABEL_MAX_CHARS
        );
        assert!(!label.contains(&"x".repeat(256)), "{label}");
    }

    #[test]
    fn native_footer_test_long_status_builds_directly() {
        let status = native_footer_test_long_status(4);
        assert_eq!(status, "log: xxxx");
        assert!(status.capacity() >= status.len());
    }

    fn native_footer_test_long_status(x_count: usize) -> String {
        let mut status = String::with_capacity("log: ".len() + x_count);
        status.push_str("log: ");
        status.extend(std::iter::repeat_n('x', x_count));
        status
    }

    #[test]
    fn native_empty_workspace_action_chip_label_builds_directly() {
        let label = native_empty_workspace_action_chip_label(7);
        assert_eq!(label, "empty-workspace-action-chip-7");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_empty_workspace_scene_fits_tiny_terminals() {
        for (cols, footer_row) in [(0, 0), (1, 1), (8, 3), (20, 4)] {
            let (_x, y, scene) =
                native_empty_workspace_scene(CellSize::new(8, 16), cols, footer_row);
            assert!(
                scene.footprint.cols <= cols.max(1),
                "cols={cols} scene={:?}",
                scene.footprint
            );
            assert!(y < footer_row.max(1), "footer_row={footer_row} y={y}");
            assert!(
                y.saturating_add(scene.footprint.rows) <= footer_row.max(1),
                "footer_row={footer_row} y={y} scene={:?}",
                scene.footprint
            );
            let scene_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
            let scene_height = scene.footprint.rows as f32 * scene.cell_size.height_px as f32;
            for layer in &scene.layers {
                let Some(rect) = (match &layer.root {
                    Node::Rect { rect, .. } | Node::Gradient { rect, .. } => Some(rect),
                    _ => None,
                }) else {
                    continue;
                };
                assert!(
                    rect.origin.0 >= 0.0 && rect.origin.0 + rect.width <= scene_width,
                    "layer {:?} escapes width {scene_width}: {:?}",
                    layer.label,
                    rect
                );
                assert!(
                    rect.origin.1 >= 0.0 && rect.origin.1 + rect.height <= scene_height,
                    "layer {:?} escapes height {scene_height}: {:?}",
                    layer.label,
                    rect
                );
            }
        }
    }

    #[test]
    fn native_empty_workspace_builds_graphical_landing_surface() {
        let (_x, y, scene) = native_empty_workspace_scene(CellSize::new(8, 16), 96, 20);
        assert_eq!(y, 2);
        assert!(scene.footprint.cols <= 72, "{:?}", scene.footprint);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"empty-workspace-backdrop"), "{labels:?}");
        assert!(labels.contains(&"empty-workspace-hero-band"), "{labels:?}");
        assert!(
            labels.contains(&"empty-workspace-accent-rail"),
            "{labels:?}"
        );
        assert_eq!(
            labels
                .iter()
                .filter(|label| label.starts_with("empty-workspace-action-chip-"))
                .count(),
            3,
            "{labels:?}"
        );
        match &scene.layers[0].root {
            Node::Rect {
                fill: Paint::Solid { color },
                ..
            } => {
                let colors = native_glass_chrome_colors();
                assert_eq!(*color, colors.fill);
                assert!(color.3 < 255, "expected translucent empty panel: {color:?}");
            }
            node => panic!("expected empty workspace backdrop rect, got {node:?}"),
        }
    }

    #[test]
    fn native_help_overlay_key_chip_label_builds_directly() {
        let label = native_help_overlay_key_chip_label(3);
        assert_eq!(label, "help-overlay-key-chip-3");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_help_overlay_row_label_builds_directly() {
        let label = native_help_overlay_row_label(3);
        assert_eq!(label, "help-overlay-row-3");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_help_overlay_scene_skips_blank_content() {
        let (x, y, scene) = native_help_overlay_scene(CellSize::new(8, 16), 80, 24, &[]);
        assert_eq!((x, y), (0, 0));
        assert_eq!(scene.footprint, CellRect::new(0, 0, 1, 1));
        assert!(scene.layers.is_empty());
    }

    #[test]
    fn native_help_overlay_dimensions_saturate_long_inputs() {
        let long = "x".repeat(u16::MAX as usize);
        let long_ref: &str = &long;
        assert_eq!(native_help_overlay_max_line_cols(&[long_ref]), u16::MAX);
        assert_eq!(native_help_overlay_panel_rows(usize::MAX, 24), 24);
        assert_eq!(native_help_overlay_panel_rows(0, 24), 4);

        let (_x, _y, scene) = native_help_overlay_scene(CellSize::new(8, 16), 80, 24, &[long_ref]);
        assert!(scene.footprint.cols <= 80);
        assert!(scene.footprint.rows <= 24);
    }

    #[test]
    fn native_help_overlay_scene_height_fits_short_terminals() {
        for rows in [0, 1, 2, 3] {
            let (x, y, scene) = native_help_overlay_scene(
                CellSize::new(8, 16),
                80,
                rows,
                &[
                    "kittwm shortcuts",
                    "C-a ? help",
                    "C-a x close",
                    "C-a g launcher",
                ],
            );
            assert!(x < 80, "x={x}");
            assert!(y < rows.max(1), "rows={rows} y={y}");
            assert!(
                y.saturating_add(scene.footprint.rows) <= rows.max(1),
                "rows={rows} y={y} scene={:?}",
                scene.footprint
            );
        }
    }

    #[test]
    fn native_help_overlay_internal_layers_fit_short_height() {
        let (_x, _y, scene) = native_help_overlay_scene(
            CellSize::new(8, 16),
            80,
            3,
            &[
                "kittwm shortcuts",
                "C-a ? help",
                "C-a x close",
                "C-a g launcher",
            ],
        );
        let scene_height = scene.footprint.rows as f32 * scene.cell_size.height_px as f32;
        for layer in &scene.layers {
            let rect = match &layer.root {
                Node::Rect { rect, .. } | Node::Gradient { rect, .. } => Some(rect),
                _ => None,
            };
            if let Some(rect) = rect {
                assert!(
                    rect.origin.1 >= 0.0 && rect.origin.1 + rect.height <= scene_height,
                    "layer {:?} escapes scene height {scene_height}: {:?}",
                    layer.label,
                    rect
                );
            }
        }
    }

    #[test]
    fn native_help_overlay_internal_layers_fit_tiny_width() {
        let (_x, _y, scene) = native_help_overlay_scene(
            CellSize::new(8, 16),
            1,
            24,
            &["kittwm shortcuts", "C-a ? help"],
        );
        let scene_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            let rect = match &layer.root {
                Node::Rect { rect, .. } | Node::Gradient { rect, .. } => Some(rect),
                _ => None,
            };
            if let Some(rect) = rect {
                assert!(
                    rect.origin.0 >= 0.0 && rect.origin.0 + rect.width <= scene_width,
                    "layer {:?} escapes scene width {scene_width}: {:?}",
                    layer.label,
                    rect
                );
            }
        }
    }

    #[test]
    fn native_help_overlay_scene_width_fits_tiny_terminals() {
        for cols in [0, 1, 8, 19] {
            let (x, _y, scene) = native_help_overlay_scene(
                CellSize::new(8, 16),
                cols,
                24,
                &["kittwm shortcuts", "C-a ? help"],
            );
            assert!(
                scene.footprint.cols <= cols.max(1),
                "cols={cols} scene={:?}",
                scene.footprint
            );
            assert_eq!(x, 0);
        }
    }

    #[test]
    fn native_prefixed_control_layer_label_builds_directly() {
        let label = native_prefixed_control_layer_label("help-overlay-control", 7, "button");
        assert_eq!(label, "help-overlay-control:7:button");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_help_overlay_builds_graphical_panel_and_key_chips() {
        let (_x, y, scene) = native_help_overlay_scene(
            CellSize::new(8, 16),
            80,
            24,
            &[
                "kittwm shortcuts",
                "C-a ?              toggle this help",
                "C-a x              close pane",
                "outside: kittwm info · kittwm cheat",
            ],
        );
        assert_eq!(y, 2);
        assert!(scene.footprint.cols <= 76, "{:?}", scene.footprint);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"help-overlay-backdrop"), "{labels:?}");
        assert!(labels.contains(&"help-overlay-heading-band"), "{labels:?}");
        match &scene.layers[0].root {
            Node::Rect {
                fill: Paint::Solid { color },
                stroke: Some(stroke),
                ..
            } => {
                let colors = native_glass_chrome_colors();
                assert_eq!(*color, colors.fill);
                assert!(color.3 < 255, "expected translucent backdrop: {color:?}");
                match &stroke.paint {
                    Paint::Solid { color } => assert_eq!(*color, colors.border),
                    paint => panic!("expected solid border paint, got {paint:?}"),
                }
            }
            node => panic!("expected help overlay backdrop rect, got {node:?}"),
        }
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("help-overlay-key-chip-")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("help-overlay-row-")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("help-overlay-control-button:toggle-help")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(
                |label| label.starts_with("help-overlay-control-text-input:filter-placeholder")
            ),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"help-overlay-control-action:toggle-help:C-a ?"),
            "{labels:?}"
        );
    }

    #[test]
    fn native_top_bar_scene_marks_empty_workspace() {
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 4,
                text: String::new(),
            },
            help_overlay: false,
        };
        let scene = native_top_bar_scene(&view, 20, CellSize::new(8, 16));
        assert_eq!(scene.footprint.rows, 1);
        assert_eq!(scene.footprint.cols, 20);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("kittwm-live-top-bar:empty:1")));
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("|[1]|")));
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("workspace-chip:1:active")));
    }

    #[test]
    fn native_top_bar_scene_uses_workspace_label_env_for_active_chip() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WORKSPACE", "2");
        let view = NativeShellView {
            top_bar: NativeTopBarChrome {
                row: 0,
                text: "| 1 | 2 | 3 |                  12:00 ".to_string(),
            },
            panes: Vec::new(),
            footer: NativeFooterChrome {
                row: 4,
                text: String::new(),
            },
            help_overlay: false,
        };
        let scene = native_top_bar_scene(&view, 20, CellSize::new(8, 16));
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("workspace-chip:2:active")));
        std::env::remove_var("KITTWM_WORKSPACE");
    }

    #[test]
    fn native_top_bar_uses_workspace_label_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WORKSPACE", "dev");
        let text = native_top_bar_text(1, 0, "/tmp/kittwm.sock", 40);
        assert!(text.contains("|[dev]|"), "{text}");
        assert!(!text.contains("| 1 | 2 | 3 |"), "{text}");
        assert!(text.ends_with("00:00 ") || text.contains(":"), "{text}");
        let active = native_top_bar_text(1, 1, &"/tmp/kittwm.sock".repeat(10_000), 40);
        assert!(!active.contains("kittwm.sock"), "{active}");
        assert_eq!(active.chars().count(), 40, "{active}");
        std::env::remove_var("KITTWM_WORKSPACE");
    }

    #[test]
    fn native_pane_title_key_builds_directly() {
        let layout = NativePaneLayout {
            x: 0,
            y: 0,
            cols: 12,
            rows: 7,
            app_x: 0,
            app_y: 1,
            app_cols: 12,
            app_rows: 5,
        };
        let key = native_pane_title_key_from_text("* native-1 sh", layout, true);
        assert_eq!(key, "0,0,12x7:true:* native-1 sh");
        assert!(key.capacity() >= "* native-1 sh".len() + 32);
    }

    #[test]
    fn native_pane_window_id_builds_directly() {
        let id = native_pane_window_id(42);
        assert_eq!(id, "native-42");
        assert!(id.capacity() >= "native-".len() + 10);
    }

    #[test]
    fn native_shell_view_builds_presentation_agnostic_chrome() {
        let view = native_shell_view(
            80,
            10,
            &[],
            0,
            &[],
            &HashMap::new(),
            "/tmp/kittwm.sock",
            "/tmp/kittwm.log",
            "columns",
            None,
            false,
            false,
        );
        assert_eq!(view.top_bar.row, 0);
        assert!(view.top_bar.text.contains("[1]"), "{}", view.top_bar.text);
        assert!(!view.top_bar.text.contains("kittui-bar"));
        assert!(view.footer.text.is_empty());
    }

    #[test]
    fn native_terminal_size_clamps_overrides_to_host_cells() {
        assert_eq!(clamp_native_terminal_size(200, 80, (100, 40)), (100, 40));
        assert_eq!(clamp_native_terminal_size(0, 0, (100, 40)), (1, 1));
        assert_eq!(clamp_native_terminal_size(80, 24, (0, 0)), (1, 1));
        assert_eq!(clamp_native_terminal_size(80, 24, (100, 40)), (80, 24));
        assert_eq!(
            clamp_native_terminal_size(u16::MAX, u16::MAX, (u16::MAX, u16::MAX)),
            (MAX_NATIVE_TERMINAL_COLS, MAX_NATIVE_TERMINAL_ROWS)
        );
        assert_eq!(
            clamp_native_terminal_size(1_000, 500, (2_000, 1_000)),
            (MAX_NATIVE_TERMINAL_COLS, MAX_NATIVE_TERMINAL_ROWS)
        );
    }

    #[test]
    fn host_terminal_cells_fall_back_to_shell_columns_and_lines() {
        assert_eq!(
            host_terminal_cells_from_sources(None, None, Some("132"), Some("43")),
            Some((132, 43))
        );
        assert_eq!(
            host_terminal_cells_from_sources(Some(100), Some(40), Some("132"), Some("43")),
            Some((100, 40))
        );
        assert_eq!(
            host_terminal_cells_from_sources(Some(0), Some(0), Some("132"), Some("43")),
            Some((132, 43))
        );
        assert_eq!(
            host_terminal_cells_from_sources(None, None, Some("132"), Some("0")),
            None
        );
    }

    #[test]
    fn host_terminal_pixels_use_env_or_remote_ioctl_sources() {
        assert_eq!(
            host_terminal_pixels_from_sources(None, None, Some("2112"), Some("1032"), false),
            Some((2112, 1032))
        );
        assert_eq!(
            host_terminal_pixels_from_sources(Some(1600), Some(800), None, None, true),
            Some((1600, 800))
        );
        assert_eq!(
            host_terminal_pixels_from_sources(Some(1600), Some(800), None, None, false),
            None
        );
        assert_eq!(infer_cell_px_from_terminal_pixels(2112, 132), Some(16));
        assert_eq!(infer_cell_px_from_terminal_pixels(1032, 43), Some(24));
    }

    #[test]
    fn native_status_outer_rows_reports_layout_not_app_height() {
        let layout = NativePaneLayout {
            x: 0,
            y: 3,
            cols: 40,
            rows: 12,
            app_x: 1,
            app_y: 4,
            app_cols: 38,
            app_rows: 10,
        };
        assert_eq!(native_status_outer_rows(layout), 12);
        assert_ne!(
            native_status_outer_rows(layout),
            layout.app_rows.saturating_add(1)
        );
    }

    #[test]
    fn native_capture_failure_logging_only_fires_on_first_failure_until_recovery() {
        let mut failed = false;
        assert!(should_log_native_capture_failure(&mut failed));
        assert!(failed);
        assert!(!should_log_native_capture_failure(&mut failed));
        native_capture_failure_recovered(&mut failed);
        assert!(!failed);
        assert!(should_log_native_capture_failure(&mut failed));
    }

    #[test]
    fn native_spawn_failure_log_includes_command_and_bounded_error() {
        let line = native_spawn_failure_log_line(
            &"very-long-command ".repeat(20),
            &"spawn failed ".repeat(20),
        );
        assert!(line.contains("command=very-long-command"), "{line}");
        assert!(line.contains("err=spawn failed"), "{line}");
        assert!(line.capacity() >= line.len());
        assert!(line.chars().count() < 320, "{line}");
    }

    #[test]
    fn native_terminate_failure_log_includes_window_and_bounded_error() {
        let line = native_terminate_failure_log_line("native-2", &"terminate failed ".repeat(20));
        assert!(line.contains("window=native-2"), "{line}");
        assert!(line.contains("err=terminate failed"), "{line}");
        assert!(line.capacity() >= line.len());
        assert!(line.chars().count() < 220, "{line}");
    }

    #[test]
    fn native_socket_input_failure_log_includes_window_operation_bytes_and_bounded_error() {
        let line = native_socket_input_failure_log_line(
            "native-browser",
            "paste-bytes",
            42,
            &"non-utf8 browser input ".repeat(20),
        );
        assert!(line.contains("window=native-browser"), "{line}");
        assert!(line.contains("operation=paste-bytes"), "{line}");
        let mouse_line =
            native_socket_input_failure_log_line("native-1", "send-mouse-direct", 2, &"boom");
        assert!(
            mouse_line.contains("operation=send-mouse-direct"),
            "{mouse_line}"
        );
        let keyboard_line =
            native_socket_input_failure_log_line("native-1", "keyboard-byte", 1, &"closed");
        assert!(
            keyboard_line.contains("operation=keyboard-byte"),
            "{keyboard_line}"
        );
        assert!(line.contains("bytes=42"), "{line}");
        assert!(line.contains("err=non-utf8 browser input"), "{line}");
        assert!(line.capacity() >= line.len());
        assert!(line.chars().count() < 260, "{line}");
    }

    #[test]
    fn native_capture_failure_log_includes_window_canvas_and_bounded_error() {
        let layout = NativePaneLayout {
            x: 2,
            y: 3,
            cols: 40,
            rows: 12,
            app_x: 3,
            app_y: 4,
            app_cols: 38,
            app_rows: 10,
        };
        let line = native_capture_failure_log_line("native-2", layout, &"boom".repeat(100));
        assert!(line.contains("window=native-2"), "{line}");
        assert!(line.contains("app=38x10"), "{line}");
        assert!(line.contains("layout=40x12+2,3"), "{line}");
        assert!(line.contains("err=boom"), "{line}");
        assert!(line.capacity() >= line.len());
        assert!(!line.contains(&"boom".repeat(80)), "{line}");
    }

    #[test]
    fn native_resize_failure_log_includes_window_and_canvas() {
        let layout = NativePaneLayout {
            x: 2,
            y: 3,
            cols: 40,
            rows: 12,
            app_x: 3,
            app_y: 4,
            app_cols: 38,
            app_rows: 10,
        };
        let line = native_resize_failure_log_line("native-2", layout, &"boom");
        assert!(line.contains("window=native-2"), "{line}");
        assert!(line.contains("app=38x10"), "{line}");
        assert!(line.contains("layout=40x12+2,3"), "{line}");
        assert!(line.contains("err=boom"), "{line}");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_resize_failure_summary_logs_only_on_failures() {
        assert!(!should_log_resize_failures(0));
        assert!(should_log_resize_failures(1));
    }

    #[test]
    fn terminal_visible_row_clamps_to_last_row() {
        assert_eq!(terminal_visible_row(0, 0), 0);
        assert_eq!(terminal_visible_row(5, 0), 0);
        assert_eq!(terminal_visible_row(0, 1), 0);
        assert_eq!(terminal_visible_row(5, 3), 2);
    }

    #[test]
    fn terminal_visible_row_and_width_skip_offscreen_writes() {
        assert_eq!(terminal_visible_row_opt(0, 0), None);
        assert_eq!(terminal_visible_row_opt(2, 2), None);
        assert_eq!(terminal_visible_row_opt(1, 2), Some(1));
        assert_eq!(terminal_visible_width(0, 10, 5), Some(5));
        assert_eq!(terminal_visible_width(3, 10, 5), Some(2));
        assert_eq!(terminal_visible_width(5, 10, 5), None);
        assert_eq!(terminal_visible_width(0, 0, 5), None);
    }

    #[test]
    fn native_footer_row_stays_on_screen() {
        assert_eq!(native_footer_row(0), 0);
        assert_eq!(native_footer_row(1), 0);
        assert_eq!(native_footer_row(24), 23);
    }

    #[test]
    fn native_shell_view_footer_stays_on_last_visible_row() {
        let mut pane = dummy_native_pane("native-1", "sh", 1);
        pane.pid = Some(42);
        let layout = NativePaneLayout {
            x: 0,
            y: 1,
            cols: 10,
            rows: 5,
            app_x: 0,
            app_y: 2,
            app_cols: 10,
            app_rows: 3,
        };
        let view = native_shell_view(
            80,
            10,
            &[pane],
            0,
            &[layout],
            &HashMap::new(),
            "/tmp/kittwm.sock",
            "/tmp/kittwm.log",
            "columns",
            None,
            false,
            false,
        );
        assert_eq!(view.footer.row, 9);
        assert!(view.footer.text.contains("C-a ? help"));
        let dragging = native_shell_view(
            80,
            10,
            &[dummy_native_pane("native-1", "sh", 1)],
            0,
            &[layout],
            &HashMap::new(),
            "/tmp/kittwm.sock",
            "/tmp/kittwm.log",
            "columns",
            Some(NativeActiveDragLabel {
                kind: "reorder",
                window: "native-1",
            }),
            false,
            false,
        );
        assert!(
            dragging.footer.text.contains(" · drag:reorder:native-1 · "),
            "{}",
            dragging.footer.text
        );
    }

    #[test]
    fn native_shell_view_skips_text_snapshots_when_not_requested() {
        let pane = dummy_native_pane("native-1", "sh", 1);
        let layout = NativePaneLayout {
            x: 0,
            y: 1,
            cols: 10,
            rows: 5,
            app_x: 0,
            app_y: 2,
            app_cols: 10,
            app_rows: 3,
        };
        let view = native_shell_view(
            80,
            10,
            &[pane],
            0,
            &[layout],
            &HashMap::new(),
            "/tmp/kittwm.sock",
            "/tmp/kittwm.log",
            "columns",
            None,
            false,
            false,
        );
        assert_eq!(view.panes[0].text_snapshot, "");
    }

    #[test]
    fn native_shell_view_marks_moved_floating_pane_title() {
        let pane = dummy_native_pane("native-1", "sh", 1);
        let layout = NativePaneLayout {
            x: 0,
            y: 1,
            cols: 16,
            rows: 5,
            app_x: 0,
            app_y: 2,
            app_cols: 16,
            app_rows: 3,
        };
        let offsets = HashMap::from([(
            "native-1".to_string(),
            NativeFloatingPaneOffset { dx: 2, dy: -1 },
        )]);
        let view = native_shell_view(
            80,
            10,
            &[pane],
            0,
            &[layout],
            &offsets,
            "/tmp/kittwm.sock",
            "/tmp/kittwm.log",
            "floating",
            None,
            false,
            false,
        );
        assert!(
            view.panes[0].text.starts_with("▶≡▲● native-1"),
            "{}",
            view.panes[0].text
        );
    }

    #[test]
    fn native_layouts_reserve_top_bar_chrome_band() {
        let layouts = reserve_native_top_bar(native_pane_layouts_weighted(
            80,
            native_tilable_rows(24),
            &[1],
            NativePaneLayoutAxis::Columns,
        ));
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].y, NATIVE_TOP_BAR_ROWS);
        assert_eq!(layouts[0].app_y, NATIVE_TOP_BAR_ROWS + 1);
        assert_eq!(layouts[0].app_x, NATIVE_PANE_BORDER_COLS);
        assert_eq!(
            layouts[0].app_rows,
            native_tilable_rows(24)
                .saturating_sub(NATIVE_PANE_TITLE_ROWS)
                .saturating_sub(NATIVE_PANE_BOTTOM_BORDER_ROWS)
        );
        assert_eq!(native_tilable_rows(1), 1);
    }

    #[test]
    fn native_layouts_apply_chrome_reservation_bands_and_gaps() {
        let reservation = crate::daemon::NativeChromeReservationConfig {
            top_bar_rows: 2,
            bottom_bar_rows: 1,
            left_cols: 3,
            right_cols: 5,
            gap_cols: 2,
            gap_rows: 1,
            owner: Some("bar".to_string()),
        };
        let weights = [1, 1];
        let columns = native_layouts_for_weights_with_reservation(
            80,
            24,
            &weights,
            NativePaneLayoutAxis::Columns,
            &reservation,
        );
        assert_eq!(columns[0].x, 3);
        assert_eq!(columns[0].y, 2);
        assert_eq!(columns[1].x, 40);
        assert_eq!(columns[0].cols, 35);
        assert_eq!(columns[1].cols, 35);
        assert_eq!(columns[0].app_y, 3);
        assert_eq!(columns[0].app_rows, 19);
        assert!(columns[0].app_x + columns[0].app_cols < columns[1].app_x);

        let rows = native_layouts_for_weights_with_reservation(
            80,
            24,
            &weights,
            NativePaneLayoutAxis::Rows,
            &reservation,
        );
        assert_eq!(rows[0].x, 3);
        assert_eq!(rows[0].cols, 72);
        assert_eq!(rows[0].y, 2);
        assert_eq!(rows[1].y, 13);
        assert!(rows[0].app_y + rows[0].app_rows < rows[1].app_y);
    }

    #[test]
    fn raw_overlay_clear_range_is_bounded_by_terminal_height() {
        assert_eq!(raw_overlay_clear_end_row(24), Some(17));
        assert_eq!(raw_overlay_clear_end_row(10), Some(10));
        assert_eq!(raw_overlay_clear_end_row(1), None);
    }

    #[test]
    fn raw_overlay_clear_decision_handles_picker_and_launcher_close() {
        assert!(should_clear_raw_overlay_area(
            true, false, false, false, false, false
        ));
        assert!(should_clear_raw_overlay_area(
            false, false, true, false, false, false
        ));
        assert!(should_clear_raw_overlay_area(
            false, false, false, false, true, false
        ));
        assert!(!should_clear_raw_overlay_area(
            true, true, true, true, true, true
        ));
        assert!(!should_clear_raw_overlay_area(
            false, true, false, true, false, true
        ));
    }

    #[test]
    fn text_overlay_renderers_bound_rows_to_terminal_height() {
        let mut out = Vec::new();
        let mut confirm = QuitConfirmOverlay::default();
        confirm.open(Instant::now());
        confirm.render(&mut out, 4).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b[4;2H"), "{text:?}");
        assert!(!text.contains("\x1b[5;2H"), "{text:?}");

        let mut out = Vec::new();
        let mut picker = PickerOverlay::default();
        picker.open();
        picker.render(&mut out, 6).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b[6;2H"), "{text:?}");
        assert!(!text.contains("\x1b[7;2H"), "{text:?}");

        let mut out = Vec::new();
        let mut launcher = LauncherOverlay::default();
        launcher.active = true;
        launcher.render(&mut out, 8).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b[8;2H"), "{text:?}");
        assert!(!text.contains("\x1b[9;2H"), "{text:?}");
    }

    #[test]
    fn overlay_keys_change_when_visual_state_changes() {
        let mut launcher = LauncherOverlay::default();
        launcher.active = true;
        launcher.query = "term".to_string();
        let base = launcher_overlay_key(&launcher);
        launcher.selected = 1;
        assert_ne!(base, launcher_overlay_key(&launcher));
        launcher.query = "query-".repeat(10_000);
        let bounded_launcher = launcher_overlay_key(&launcher);
        assert!(bounded_launcher.len() < 160, "{bounded_launcher}");
        assert!(bounded_launcher.contains('…'), "{bounded_launcher}");
        assert!(
            !bounded_launcher.contains(&"query-".repeat(32)),
            "{bounded_launcher}"
        );

        let mut picker = PickerOverlay::default();
        picker.active = true;
        picker.entries = vec!["one".to_string()];
        let picker_base = picker_overlay_key(&picker);
        picker.entries.push("two".to_string());
        assert_ne!(picker_base, picker_overlay_key(&picker));
        let long_entry = "window-title-".repeat(10_000);
        picker.entries[0] = long_entry;
        let bounded = picker_overlay_key(&picker);
        assert!(bounded.len() < 64, "{bounded}");
        assert!(!bounded.contains("window-title"), "{bounded}");
    }

    #[test]
    fn compositor_frame_flush_decision_requires_output() {
        assert!(should_flush_compositor_frame(true));
        assert!(!should_flush_compositor_frame(false));
    }

    fn huge_raw_frame_title(suffix: char) -> String {
        let prefix = "title-".repeat(10_000);
        let mut title = String::with_capacity(prefix.len() + suffix.len_utf8());
        title.push_str(&prefix);
        title.push(suffix);
        title
    }

    #[test]
    fn huge_raw_frame_title_builds_directly() {
        let title = huge_raw_frame_title('A');
        assert!(title.starts_with("title-title-"));
        assert!(title.ends_with('A'));
        assert_eq!(title.capacity(), title.len());
    }

    #[test]
    fn raw_frame_chrome_key_changes_when_visual_chrome_changes() {
        let footprint = CellRect::new(1, 2, 10, 4);
        let base = raw_frame_chrome_key(
            "app",
            false,
            kittui_wm::compositor::WindowMode::Tiled,
            false,
            footprint,
        );
        assert_eq!(
            base,
            raw_frame_chrome_key(
                "app",
                false,
                kittui_wm::compositor::WindowMode::Tiled,
                false,
                footprint,
            )
        );
        assert_ne!(
            base,
            raw_frame_chrome_key(
                "app",
                true,
                kittui_wm::compositor::WindowMode::Tiled,
                false,
                footprint,
            )
        );
        assert_ne!(
            base,
            raw_frame_chrome_key(
                "app",
                false,
                kittui_wm::compositor::WindowMode::Floating,
                false,
                footprint,
            )
        );
        assert_ne!(
            base,
            raw_frame_chrome_key(
                "app",
                false,
                kittui_wm::compositor::WindowMode::Tiled,
                false,
                CellRect::new(2, 2, 10, 4),
            )
        );
        let huge_a = huge_raw_frame_title('A');
        let huge_b = huge_raw_frame_title('B');
        let key_a = raw_frame_chrome_key(
            &huge_a,
            true,
            kittui_wm::compositor::WindowMode::Tiled,
            false,
            CellRect::new(1, 2, 12, 4),
        );
        let key_b = raw_frame_chrome_key(
            &huge_b,
            true,
            kittui_wm::compositor::WindowMode::Tiled,
            false,
            CellRect::new(1, 2, 12, 4),
        );
        assert_eq!(key_a, key_b);
        assert!(key_a.len() < 80, "{key_a}");
        assert!(!key_a.contains(&"title-".repeat(16)), "{key_a}");
    }

    #[test]
    fn raw_frame_chrome_text_bounds_huge_titles() {
        let text = raw_frame_chrome_text(
            &"title-".repeat(10_000),
            true,
            kittui_wm::compositor::WindowMode::Tiled,
            false,
            12,
        );
        assert_eq!(text, "* title-titl");
        assert_eq!(text.chars().count(), 12);
        assert!(text.capacity() >= 12);
        let short = raw_frame_chrome_text(
            "app",
            false,
            kittui_wm::compositor::WindowMode::Floating,
            true,
            20,
        );
        assert_eq!(short, "  app float full    ");
        assert!(short.capacity() >= 20);
    }

    #[test]
    fn compositor_footer_write_decision_throttles_volatile_repaints() {
        assert!(should_write_compositor_footer("", "state", 1, 30));
        assert!(should_write_compositor_footer("old", "state", 1, 30));
        assert!(!should_write_compositor_footer("state", "state", 29, 30));
        assert!(should_write_compositor_footer("state", "state", 30, 30));
        assert!(!should_write_compositor_footer("state", "state", 30, 0));
    }

    #[test]
    fn frame_sleep_stops_when_input_is_ready() {
        assert!(frame_sleep_should_stop_for_input(true));
        assert!(!frame_sleep_should_stop_for_input(false));
    }

    #[test]
    fn frame_sleep_chunk_caps_long_slack_for_responsiveness() {
        assert_eq!(
            frame_sleep_chunk(Duration::from_millis(100)),
            Duration::from_millis(16)
        );
        assert_eq!(
            frame_sleep_chunk(Duration::from_millis(3)),
            Duration::from_millis(3)
        );
        assert_eq!(
            frame_sleep_chunks_for_budget(Duration::from_millis(35)),
            vec![
                Duration::from_millis(16),
                Duration::from_millis(16),
                Duration::from_millis(3),
            ]
        );
    }

    #[test]
    fn pure_terminal_frame_write_decision_skips_unchanged_frames() {
        assert!(should_write_pure_terminal_frame(
            "", "frame-a", false, false
        ));
        assert!(!should_write_pure_terminal_frame(
            "frame-a", "frame-a", false, false
        ));
        assert!(should_write_pure_terminal_frame(
            "frame-a", "frame-a", false, true
        ));
        assert!(should_write_pure_terminal_frame(
            "frame-a", "frame-a", true, false
        ));
        assert!(should_write_pure_terminal_frame(
            "frame-a", "frame-b", false, false
        ));
    }

    #[test]
    fn native_z_indices_follow_architecture_contract_roles() {
        let contract = ArchitectureContract::current();
        assert_eq!(
            native_app_z_index(),
            contract.app_surface_z_index().unwrap()
        );
        assert_eq!(
            native_chrome_z_index(),
            contract.decoration_z_index().unwrap()
        );
        assert!(native_chrome_z_index() > native_app_z_index());
        assert!(native_live_app_z_index() < native_live_chrome_z_index());
        assert!(native_live_chrome_z_index() < 0);
    }

    #[test]
    fn native_frame_lifecycle_retires_only_missing_image_ids() {
        let previous = HashSet::from([0x6b77_0001, 0x6b77_0002, 0x6b77_0004]);
        let current = HashSet::from([0x6b77_0002, 0x6b77_0003]);
        assert_eq!(
            retired_native_image_ids(&previous, &current),
            vec![0x6b77_0001, 0x6b77_0004]
        );
    }

    #[test]
    fn native_layouts_cover_empty_and_small_counts_without_overlap() {
        let empty: Vec<NativePane> = Vec::new();
        let reservation = crate::daemon::NativeChromeReservationConfig::default();
        assert!(native_layouts_for_panes_with_reservation(
            80,
            24,
            &empty,
            NativePaneLayoutAxis::Columns,
            &reservation,
        )
        .is_empty());

        for axis in [NativePaneLayoutAxis::Columns, NativePaneLayoutAxis::Rows] {
            for (cols, rows) in [(80, 24), (13, 7), (2, 2), (1, 1)] {
                for weights in [vec![1], vec![1, 1], vec![1, 2, 3], vec![4, 1, 1, 2]] {
                    let layouts = native_pane_layouts_weighted(cols, rows, &weights, axis);
                    assert_eq!(layouts.len(), weights.len(), "{axis:?} {cols}x{rows}");
                    assert_native_layout_invariants(&layouts, cols, rows);
                }
            }
        }
    }

    #[test]
    fn native_display_modes_produce_float_and_fullscreen_layouts() {
        let panes = vec![
            dummy_native_pane("native-1", "left", 1),
            dummy_native_pane("native-2", "right", 1),
        ];
        let reservation = crate::daemon::NativeChromeReservationConfig::default();

        let fullscreen = native_layouts_for_panes_display_mode(
            80,
            24,
            &panes,
            1,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Fullscreen,
            &reservation,
        );
        assert_eq!(fullscreen.len(), 2);
        assert_eq!(fullscreen[0].app_cols, 0);
        assert_eq!(fullscreen[0].app_rows, 0);
        assert_eq!(fullscreen[1].x, 0);
        assert_eq!(fullscreen[1].y, reservation.top_bar_rows);
        assert_eq!(fullscreen[1].cols, 80);
        assert_eq!(fullscreen[1].rows, native_tilable_rows(24));

        let floating = native_layouts_for_panes_display_mode(
            80,
            24,
            &panes,
            0,
            NativePaneLayoutAxis::Columns,
            NativePaneLayoutMode::Floating,
            &reservation,
        );
        assert_eq!(floating.len(), 2);
        assert!(floating[0].cols < 80, "{floating:?}");
        assert!(floating[0].rows < native_tilable_rows(24), "{floating:?}");
        assert!(
            floating[1].x > floating[0].x || floating[1].y > floating[0].y,
            "{floating:?}"
        );
        for layout in &floating {
            assert!(layout.x.saturating_add(layout.cols) <= 80, "{floating:?}");
            assert!(layout.y.saturating_add(layout.rows) <= 24, "{floating:?}");
        }
    }

    #[test]
    fn native_layouts_keep_reserved_chrome_and_gaps_out_of_pane_bounds() {
        let reservation = crate::daemon::NativeChromeReservationConfig {
            top_bar_rows: 2,
            bottom_bar_rows: 2,
            left_cols: 4,
            right_cols: 3,
            gap_cols: 1,
            gap_rows: 2,
            owner: Some("bar".to_string()),
        };
        let weights = [1, 2, 3];

        for axis in [NativePaneLayoutAxis::Columns, NativePaneLayoutAxis::Rows] {
            let layouts =
                native_layouts_for_weights_with_reservation(80, 24, &weights, axis, &reservation);
            assert_native_layout_invariants(&layouts, 80, 24);
            for layout in &layouts {
                assert!(layout.x >= reservation.left_cols, "{axis:?}: {layouts:?}");
                assert!(
                    layout.y >= reservation.top_bar_rows,
                    "{axis:?}: {layouts:?}"
                );
                assert!(
                    layout.x.saturating_add(layout.cols)
                        <= 80u16.saturating_sub(reservation.right_cols),
                    "{axis:?}: {layouts:?}"
                );
                assert!(
                    layout.y.saturating_add(layout.rows)
                        <= 24u16.saturating_sub(reservation.bottom_bar_rows),
                    "{axis:?}: {layouts:?}"
                );
            }
        }
    }

    fn assert_native_layout_invariants(layouts: &[NativePaneLayout], cols: u16, rows: u16) {
        for layout in layouts {
            assert!(layout.x.saturating_add(layout.cols) <= cols, "{layouts:?}");
            assert!(layout.y.saturating_add(layout.rows) <= rows, "{layouts:?}");
            assert!(layout.app_x >= layout.x, "{layouts:?}");
            assert!(layout.app_y >= layout.y, "{layouts:?}");
            assert!(
                layout.app_x.saturating_add(layout.app_cols)
                    <= layout.x.saturating_add(layout.cols),
                "app cols escape outer bounds: {layouts:?}"
            );
            assert!(
                layout.app_y.saturating_add(layout.app_rows)
                    <= layout.y.saturating_add(layout.rows),
                "app rows escape outer bounds: {layouts:?}"
            );
        }
        for (idx, a) in layouts.iter().enumerate() {
            for b in layouts.iter().skip(idx + 1) {
                assert!(
                    !rects_overlap((a.x, a.y, a.cols, a.rows), (b.x, b.y, b.cols, b.rows)),
                    "outer bounds overlap: {layouts:?}"
                );
                assert!(
                    !rects_overlap(
                        (a.app_x, a.app_y, a.app_cols, a.app_rows),
                        (b.app_x, b.app_y, b.app_cols, b.app_rows),
                    ),
                    "app bounds overlap: {layouts:?}"
                );
            }
        }
    }

    fn rects_overlap(a: (u16, u16, u16, u16), b: (u16, u16, u16, u16)) -> bool {
        let (ax, ay, aw, ah) = a;
        let (bx, by, bw, bh) = b;
        aw > 0
            && ah > 0
            && bw > 0
            && bh > 0
            && ax < bx.saturating_add(bw)
            && bx < ax.saturating_add(aw)
            && ay < by.saturating_add(bh)
            && by < ay.saturating_add(ah)
    }

    #[test]
    fn native_graphics_cell_size_defines_pixel_density_contract() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITTWM_HIDPI");
        std::env::remove_var("KITTWM_NATIVE_CELL_WIDTH_PX");
        std::env::remove_var("KITTWM_NATIVE_CELL_HEIGHT_PX");
        let cell_size = native_cell_size();
        assert_eq!(cell_size.width_px as u32, native_cell_width_px());
        assert_eq!(cell_size.height_px as u32, native_cell_height_px());
        assert_eq!(
            native_cell_width_px(),
            NATIVE_BASE_CELL_WIDTH_PX * NATIVE_HIDPI_SCALE
        );
        assert_eq!(
            native_cell_height_px(),
            NATIVE_BASE_CELL_HEIGHT_PX * NATIVE_HIDPI_SCALE
        );

        let scene = native_empty_workspace_scene(cell_size, 80, 20).2;
        assert_eq!(
            scene.pixel_width(),
            scene.footprint.cols as u32 * native_cell_width_px()
        );
        assert_eq!(
            scene.pixel_height(),
            scene.footprint.rows as u32 * native_cell_height_px()
        );

        let source = vec![0xff; 4];
        let (fitted, width, height) = fit_rgba_frame_to_cells(source, 1, 1, 7, 3);
        assert_eq!(width, 7 * native_cell_width_px());
        assert_eq!(height, 3 * native_cell_height_px());
        assert_eq!(fitted.len(), (width * height * 4) as usize);
        std::env::remove_var("KITTWM_HIDPI");
        std::env::remove_var("KITTWM_NATIVE_CELL_WIDTH_PX");
        std::env::remove_var("KITTWM_NATIVE_CELL_HEIGHT_PX");
    }

    #[test]
    fn native_hidpi_and_pixel_gap_env_are_configurable() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_HIDPI", "0");
        std::env::remove_var("KITTWM_NATIVE_CELL_WIDTH_PX");
        std::env::remove_var("KITTWM_NATIVE_CELL_HEIGHT_PX");
        assert_eq!(native_cell_width_px(), NATIVE_BASE_CELL_WIDTH_PX);
        assert_eq!(native_cell_height_px(), NATIVE_BASE_CELL_HEIGHT_PX);
        std::env::set_var("KITTWM_HIDPI", "1");
        assert_eq!(
            native_cell_width_px(),
            NATIVE_BASE_CELL_WIDTH_PX * NATIVE_HIDPI_SCALE
        );
        assert_eq!(
            native_cell_height_px(),
            NATIVE_BASE_CELL_HEIGHT_PX * NATIVE_HIDPI_SCALE
        );
        std::env::set_var("KITTWM_NATIVE_CELL_WIDTH_PX", "24");
        std::env::set_var("KITTWM_NATIVE_CELL_HEIGHT_PX", "48");
        assert_eq!(native_cell_size(), CellSize::new(24, 48));
        std::env::set_var("KITTWM_TILE_GAP_PX", "25");
        std::env::set_var("KITTWM_HEADER_GAP_PX", "49");
        std::env::set_var("KITTWM_FOOTER_GAP_PX", "1");
        let reservation = native_chrome_reservation_with_pixel_gaps(
            &crate::daemon::NativeChromeReservationConfig::default(),
        );
        assert_eq!(reservation.gap_cols, 2);
        assert_eq!(reservation.gap_rows, 1);
        assert_eq!(reservation.top_bar_rows, 3);
        assert_eq!(reservation.bottom_bar_rows, 1);
        for key in [
            "KITTWM_HIDPI",
            "KITTWM_NATIVE_CELL_WIDTH_PX",
            "KITTWM_NATIVE_CELL_HEIGHT_PX",
            "KITTWM_TILE_GAP_PX",
            "KITTWM_HEADER_GAP_PX",
            "KITTWM_FOOTER_GAP_PX",
        ] {
            std::env::remove_var(key);
        }
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
        assert_eq!(layouts[0].app_x, 1);
        assert_eq!(layouts[0].app_y, 1);
        assert_eq!(layouts[0].app_cols, 38);
        assert_eq!(layouts[0].app_rows, 22);
        assert_eq!(layouts[1].x, 40);
        assert_eq!(layouts[1].cols, 41);
        assert_eq!(layouts[1].app_x, 41);
        assert_eq!(layouts[1].app_cols, 39);
        assert!(layouts[0].app_x + layouts[0].app_cols <= layouts[1].app_x);
    }

    #[test]
    fn native_pane_layouts_split_rows_and_reserve_each_title_row() {
        let layouts = native_pane_layouts(80, 25, 2, NativePaneLayoutAxis::Rows);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].x, 0);
        assert_eq!(layouts[0].y, 0);
        assert_eq!(layouts[0].cols, 80);
        assert_eq!(layouts[0].app_x, 1);
        assert_eq!(layouts[0].app_y, 1);
        assert_eq!(layouts[0].app_cols, 78);
        assert_eq!(layouts[0].app_rows, 10);
        assert_eq!(layouts[1].y, 12);
        assert_eq!(layouts[1].app_y, 13);
        assert_eq!(layouts[1].app_rows, 11);
        assert!(layouts[0].app_y + layouts[0].app_rows <= layouts[1].y);
    }

    #[test]
    fn native_pane_layouts_keep_three_weighted_panes_disjoint() {
        for axis in [NativePaneLayoutAxis::Columns, NativePaneLayoutAxis::Rows] {
            let layouts = reserve_native_top_bar(native_pane_layouts_weighted(
                101,
                native_tilable_rows(31),
                &[1, 2, 3],
                axis,
            ));
            assert_eq!(layouts.len(), 3);
            let total_outer: u16 = match axis {
                NativePaneLayoutAxis::Columns => layouts.iter().map(|layout| layout.cols).sum(),
                NativePaneLayoutAxis::Rows => layouts
                    .iter()
                    .map(|layout| {
                        layout
                            .app_rows
                            .saturating_add(NATIVE_PANE_TITLE_ROWS)
                            .saturating_add(NATIVE_PANE_BOTTOM_BORDER_ROWS)
                    })
                    .sum(),
                NativePaneLayoutAxis::Grid => unreachable!("grid is covered by a dedicated test"),
            };
            let expected_total = match axis {
                NativePaneLayoutAxis::Columns => 101,
                NativePaneLayoutAxis::Rows => native_tilable_rows(31),
                NativePaneLayoutAxis::Grid => unreachable!("grid is covered by a dedicated test"),
            };
            assert_eq!(total_outer, expected_total, "{axis:?}: {layouts:?}");
            for pair in layouts.windows(2) {
                let a = pair[0];
                let b = pair[1];
                match axis {
                    NativePaneLayoutAxis::Columns => {
                        assert_eq!(a.x.saturating_add(a.cols), b.x, "{layouts:?}");
                        assert!(
                            a.app_x.saturating_add(a.app_cols) <= b.app_x,
                            "app bounds overlap: {layouts:?}"
                        );
                    }
                    NativePaneLayoutAxis::Rows => {
                        let a_rows = a
                            .app_rows
                            .saturating_add(NATIVE_PANE_TITLE_ROWS)
                            .saturating_add(NATIVE_PANE_BOTTOM_BORDER_ROWS);
                        assert_eq!(a.y.saturating_add(a_rows), b.y, "{layouts:?}");
                        assert!(
                            a.app_y.saturating_add(a.app_rows) <= b.y,
                            "app bounds overlap chrome: {layouts:?}"
                        );
                    }
                    NativePaneLayoutAxis::Grid => {
                        unreachable!("grid is covered by a dedicated test")
                    }
                }
            }
        }
    }

    #[test]
    fn native_app_frame_footprint_matches_split_app_bounds() {
        let layouts = reserve_native_top_bar(native_pane_layouts_weighted(
            100,
            native_tilable_rows(30),
            &[1, 1],
            NativePaneLayoutAxis::Columns,
        ));
        let left = layouts[0];
        let right = layouts[1];
        let left_frame = native_app_frame_footprint(left);
        let right_frame = native_app_frame_footprint(right);
        assert_eq!(left_frame.x, left.app_x);
        assert_eq!(left_frame.y, left.app_y);
        assert_eq!(left_frame.cols, left.app_cols);
        assert_eq!(left_frame.rows, left.app_rows);
        assert!(left_frame.x > left.x);
        assert!(left_frame.y > left.y);
        assert!(left_frame.cols < left.cols);
        assert!(left_frame.x + left_frame.cols <= right_frame.x);
        assert!(right_frame.cols < right.cols);
    }

    #[test]
    fn native_pane_layouts_honor_weights() {
        let columns = native_pane_layouts_weighted(90, 24, &[1, 2], NativePaneLayoutAxis::Columns);
        assert_eq!(columns[0].cols, 30);
        assert_eq!(columns[1].cols, 60);
        assert_eq!(columns[1].x, 30);
        let rows = native_pane_layouts_weighted(80, 30, &[1, 2], NativePaneLayoutAxis::Rows);
        assert_eq!(rows[0].app_rows, 8);
        assert_eq!(rows[1].app_rows, 18);
        assert_eq!(rows[1].y, 10);
    }

    #[test]
    fn native_pane_layouts_grid_builds_two_by_two_for_four_panes() {
        let layouts =
            native_pane_layouts_weighted(100, 40, &[1, 1, 1, 1], NativePaneLayoutAxis::Grid);
        assert_eq!(layouts.len(), 4);
        assert_eq!(layouts[0].x, 0);
        assert_eq!(layouts[0].y, 0);
        assert!(layouts[1].x > layouts[0].x);
        assert_eq!(layouts[1].y, layouts[0].y);
        assert_eq!(layouts[2].x, layouts[0].x);
        assert!(layouts[2].y > layouts[0].y);
        assert_eq!(layouts[3].x, layouts[1].x);
        assert_eq!(layouts[3].y, layouts[2].y);
    }

    #[test]
    fn native_adjust_weight_clamps_to_one() {
        assert_eq!(native_adjust_weight(1, -1), 1);
        assert_eq!(native_adjust_weight(2, -1), 1);
        assert_eq!(native_adjust_weight(2, 3), 5);
    }

    #[test]
    fn native_dummy_pane_helper_resolves_true_from_path() {
        let resolved =
            resolve_test_program("true").expect("test environment should provide true on PATH");
        assert_ne!(resolved, "true");
        assert!(std::path::Path::new(&resolved).is_file(), "{resolved}");
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "Nix Darwin sandbox lacks a stable PTY shell for dummy panes"
    )]
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
                dirty_frame: None,
                capture_failed: false,
            },
            NativePane {
                window: "native-2".to_string(),
                image_id: 2,
                command: "cmd2".to_string(),
                pid: Some(102),
                display_title: None,
                weight: 2,
                app: dummy_native_pane_app(),
                dirty_frame: None,
                capture_failed: false,
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
        assert!(should_focus_restored_pane(3, 2));
        assert!(!should_focus_restored_pane(0, 0));
        assert!(!should_focus_restored_pane(3, 3));
    }

    #[test]
    fn native_restore_focus_target_requires_restored_panes() {
        assert_eq!(native_restore_focus_target(3, Some(2)), Some(2));
        assert_eq!(native_restore_focus_target(3, Some(99)), Some(2));
        assert_eq!(native_restore_focus_target(3, None), Some(0));
        assert_eq!(native_restore_focus_target(0, Some(0)), None);
    }

    #[test]
    fn native_move_log_line_builds_directly() {
        let line = native_move_log_line("right", 2);
        assert_eq!(line, "native terminal move right -> 2");
        assert!(line.capacity() >= "native terminal move  -> ".len() + "right".len() + 20);
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
        let mut order = vec![
            dummy_native_pane("a", "a", 1),
            dummy_native_pane("b", "b", 1),
            dummy_native_pane("c", "c", 1),
        ];
        let mut focused = 1usize;
        let old_focused = order[focused].window.clone();
        let to = native_move_target_index(focused, order.len(), "right");
        let pane = order.remove(focused);
        order.insert(to, pane);
        focused = native_window_index_after_reorder(&order, &old_focused).unwrap();
        let names = order
            .iter()
            .map(|pane| pane.window.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "c", "b"]);
        assert_eq!(order[focused].window, "b");
        assert_eq!(native_window_index_after_reorder(&order, "b"), Some(2));
        assert_eq!(native_window_index_after_reorder(&order, "missing"), None);
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
    fn native_socket_spawn_log_line_builds_directly() {
        let line = native_socket_spawn_log_line("zsh -l");
        assert_eq!(line, "native terminal socket spawn: zsh -l");
        assert_eq!(
            line.capacity(),
            "native terminal socket spawn: ".len() + "zsh -l".len()
        );
    }

    #[test]
    fn native_socket_focus_log_line_builds_directly() {
        let line = native_socket_focus_log_line("native-2");
        assert_eq!(line, "native terminal socket focus: native-2");
        assert_eq!(
            line.capacity(),
            "native terminal socket focus: ".len() + "native-2".len()
        );
    }

    #[test]
    fn native_socket_focus_next_log_line_builds_directly() {
        let line = native_socket_focus_next_log_line("native-3");
        assert_eq!(line, "native terminal socket focus next: native-3");
        assert_eq!(
            line.capacity(),
            "native terminal socket focus next: ".len() + "native-3".len()
        );
    }

    #[test]
    fn native_socket_focus_prev_log_line_builds_directly() {
        let line = native_socket_focus_prev_log_line("native-1");
        assert_eq!(line, "native terminal socket focus prev: native-1");
        assert_eq!(
            line.capacity(),
            "native terminal socket focus prev: ".len() + "native-1".len()
        );
    }

    #[test]
    fn native_socket_close_log_line_builds_directly() {
        let line = native_socket_close_log_line("native-4");
        assert_eq!(line, "native terminal socket close: native-4");
        assert_eq!(
            line.capacity(),
            "native terminal socket close: ".len() + "native-4".len()
        );
    }

    #[test]
    fn native_socket_layout_log_line_builds_directly() {
        let line = native_socket_layout_log_line("columns");
        assert_eq!(line, "native terminal socket layout: columns");
        assert_eq!(
            line.capacity(),
            "native terminal socket layout: ".len() + "columns".len()
        );
    }

    #[test]
    fn native_socket_rename_log_line_builds_directly() {
        let line = native_socket_rename_log_line("native-1", "editor");
        assert_eq!(line, "native terminal socket rename: native-1 -> editor");
        assert_eq!(
            line.capacity(),
            "native terminal socket rename:  -> ".len() + "native-1".len() + "editor".len()
        );
    }

    #[test]
    fn native_socket_split_log_line_builds_directly() {
        let line = native_socket_split_log_line("native-1", "rows", "zsh -l");
        assert_eq!(
            line,
            "native terminal socket split: target=native-1 axis=rows command=zsh -l"
        );
        assert_eq!(
            line.capacity(),
            "native terminal socket split: target= axis= command=".len()
                + "native-1".len()
                + "rows".len()
                + "zsh -l".len()
        );
    }

    #[test]
    fn native_socket_move_log_line_builds_directly() {
        let line = native_socket_move_log_line("native-2", "right", 3);
        assert_eq!(line, "native terminal socket move: native-2 right -> 3");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_resize_log_line_builds_directly() {
        let line = native_socket_resize_log_line("native-2", -3, 7);
        assert_eq!(
            line,
            "native terminal socket resize: native-2 delta=-3 weight=7"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_restore_session_log_line_builds_directly() {
        let line = native_socket_restore_session_log_line(3, 1);
        assert_eq!(
            line,
            "native terminal socket restore session: panes=3 focus=1"
        );
        assert!(
            line.capacity() >= "native terminal socket restore session: panes= focus=".len() + 40
        );
    }

    #[test]
    fn native_socket_restore_session_failure_log_line_builds_directly() {
        let err = anyhow!("restore session contains no panes");
        let line = native_socket_restore_session_failure_log_line(&err);
        assert_eq!(
            line,
            "native terminal socket restore session failed: restore session contains no panes"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_send_text_log_line_builds_directly() {
        let line = native_socket_send_text_log_line("native-1", 42);
        assert_eq!(line, "native terminal socket send text: native-1 bytes=42");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_send_key_log_line_builds_directly() {
        let line = native_socket_send_key_log_line("native-1", "ctrl-c", 1);
        assert_eq!(
            line,
            "native terminal socket send key: native-1 key=ctrl-c bytes=1"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_send_browser_key_log_line_builds_directly() {
        let line = native_socket_send_browser_key_log_line("native-browser", "back");
        assert_eq!(
            line,
            "native terminal socket send browser key: native-browser key=back"
        );
        assert_eq!(
            line.capacity(),
            "native terminal socket send browser key:  key=".len()
                + "native-browser".len()
                + "back".len()
        );
    }

    #[test]
    fn native_socket_paste_bytes_log_line_builds_directly() {
        let line = native_socket_paste_bytes_log_line("native-1", 42, true);
        assert_eq!(
            line,
            "native terminal socket paste bytes: native-1 bytes=42 bracketed=true"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_send_mouse_log_line_builds_directly() {
        let line = native_socket_send_mouse_log_line("native-1", "press-left", 7, 9, 6);
        assert_eq!(
            line,
            "native terminal socket send mouse: native-1 event=press-left col=7 row=9 bytes=6"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_send_mouse_direct_log_line_builds_directly() {
        let line = native_socket_send_mouse_direct_log_line("native-1", "move-left", 7, 9, 2, 3);
        assert_eq!(
            line,
            "native terminal socket send mouse direct: native-1 event=move-left col=7 row=9 events=2/3"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_socket_send_mouse_ignored_log_line_builds_directly() {
        let modes = MouseReportingModes::default();
        let line = native_socket_send_mouse_ignored_log_line("native-1", "move-left", modes);
        assert_eq!(
            line,
            "native terminal socket send mouse ignored: native-1 event=move-left modes=MouseReportingModes { basic: false, button_motion: false, all_motion: false, sgr: false }"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_terminal_resized_log_line_builds_directly() {
        let line = native_terminal_resized_log_line(120, 40, 3, 1);
        assert_eq!(
            line,
            "native terminal resized to 120x40 panes=3 resize_failures=1"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_chrome_reservation_changed_log_line_builds_directly() {
        let reservation = crate::daemon::NativeChromeReservationConfig {
            top_bar_rows: 1,
            bottom_bar_rows: 2,
            left_cols: 3,
            right_cols: 4,
            gap_cols: 5,
            gap_rows: 6,
            owner: Some("kittwm-bar".to_string()),
        };
        let line = native_chrome_reservation_changed_log_line(&reservation);
        assert_eq!(
            line,
            "native chrome reservation changed: top=1 bottom=2 left=3 right=4 gap=5x6 owner=kittwm-bar"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_layout_resize_completed_log_line_builds_directly() {
        let line = native_layout_resize_completed_log_line(2);
        assert_eq!(
            line,
            "native layout resize completed with resize_failures=2"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_forwarded_host_sequence_log_line_builds_directly() {
        let line = native_forwarded_host_sequence_log_line("native-2", 128);
        assert_eq!(
            line,
            "native terminal forwarded host sequence: window=native-2 bytes=128"
        );
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_keyboard_focus_log_line_builds_directly() {
        let line = native_keyboard_focus_log_line("native-3");
        assert_eq!(line, "native terminal focus: native-3");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn native_layout_mode_log_line_builds_directly() {
        let line = native_layout_mode_log_line("float");
        assert_eq!(line, "native terminal layout mode: float");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn native_split_toggle_log_line_builds_directly() {
        let line = native_split_toggle_log_line("rows");
        assert_eq!(line, "native terminal split toggle: rows");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn native_resize_weight_log_line_builds_directly() {
        let grow = native_resize_weight_log_line("grow", "native-1", 4);
        assert_eq!(grow, "native terminal resize grow: native-1 weight=4");
        assert!(grow.capacity() >= grow.len());
        let shrink = native_resize_weight_log_line("shrink", "native-2", 1);
        assert_eq!(shrink, "native terminal resize shrink: native-2 weight=1");
        assert!(shrink.capacity() >= shrink.len());
    }

    #[test]
    fn native_launch_log_line_builds_directly() {
        let line = native_launch_log_line("native-1", 2);
        assert_eq!(line, "native terminal launch: native-1 panes=2");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_split_log_line_builds_directly() {
        let line = native_split_log_line(NativePaneLayoutAxis::Rows, 3);
        assert_eq!(line, "native terminal split Rows: panes=3");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn native_help_overlay_log_line_builds_directly() {
        let visible = native_help_overlay_log_line(true);
        assert_eq!(visible, "native terminal help overlay: true");
        assert_eq!(
            visible.capacity(),
            "native terminal help overlay: ".len() + 5
        );
        let hidden = native_help_overlay_log_line(false);
        assert_eq!(hidden, "native terminal help overlay: false");
        assert_eq!(
            hidden.capacity(),
            "native terminal help overlay: ".len() + 5
        );
    }

    #[test]
    fn native_close_panes_log_line_builds_directly() {
        let line = native_close_panes_log_line(12);
        assert_eq!(line, "native terminal close: panes=12");
        assert!(line.capacity() >= "native terminal close: panes=".len() + 20);
    }

    #[test]
    fn native_reaped_pane_log_line_builds_directly() {
        let line = native_reaped_pane_log_line("native-42");
        assert_eq!(line, "native terminal reaped exited pane native-42");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn native_focus_after_remove_stays_on_neighbor() {
        assert_eq!(focus_after_remove(1, 1, 3), 1);
        assert_eq!(focus_after_remove(2, 1, 3), 1);
        assert_eq!(focus_after_remove(0, 2, 3), 0);
        assert_eq!(focus_after_remove(0, 0, 1), 0);
    }

    #[test]
    fn native_focus_sequence_survives_rapid_close_churn() {
        let mut windows = vec!["native-1", "native-2", "native-3", "native-4"];
        let mut focused = 2usize;
        let mut len_before = windows.len();
        let removed = 1usize;
        windows.remove(removed);
        focused = focus_after_remove(focused, removed, len_before);
        assert_eq!(focused, 1);
        assert_eq!(windows[focused], "native-3");

        len_before = windows.len();
        let removed = focused;
        windows.remove(removed);
        focused = focus_after_remove(focused, removed, len_before);
        assert_eq!(focused, 1);
        assert_eq!(windows[focused], "native-4");

        len_before = windows.len();
        let removed = 1usize;
        windows.remove(removed);
        focused = focus_after_remove(focused, removed, len_before);
        assert_eq!(focused, 0);
        assert_eq!(windows[focused], "native-1");

        len_before = windows.len();
        windows.remove(0);
        focused = focus_after_remove(focused, 0, len_before);
        assert_eq!(focused, 0);
        assert!(windows.is_empty());
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "Nix Darwin sandbox lacks a stable PTY shell for dummy panes"
    )]
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
                dirty_frame: None,
                capture_failed: false,
            },
            NativePane {
                window: "native-2".to_string(),
                image_id: 2,
                command: "cmd2".to_string(),
                pid: Some(102),
                display_title: None,
                weight: 1,
                app: dummy_native_pane_app(),
                dirty_frame: None,
                capture_failed: false,
            },
        ];
        assert_eq!(native_pane_index(&panes, "native-2"), Some(1));
        assert_eq!(native_pane_index(&panes, "missing"), None);
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "Nix Darwin sandbox lacks a stable PTY shell for dummy panes"
    )]
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
                dirty_frame: None,
                capture_failed: false,
            },
            NativePane {
                window: "native-7".to_string(),
                image_id: 7,
                command: "cmd7".to_string(),
                pid: Some(107),
                display_title: None,
                weight: 1,
                app: dummy_native_pane_app(),
                dirty_frame: None,
                capture_failed: false,
            },
        ];
        assert_eq!(next_native_pane_id(&panes), 8);
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "Nix Darwin sandbox lacks a stable PTY shell for dummy panes"
    )]
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
                dirty_frame: None,
                capture_failed: false,
            },
            NativePane {
                window: "native-2".to_string(),
                image_id: 2,
                command: "editor-cmd".to_string(),
                pid: Some(202),
                display_title: Some("editor".to_string()),
                weight: 3,
                app: dummy_native_pane_app(),
                dirty_frame: None,
                capture_failed: false,
            },
        ];
        let layouts = native_pane_layouts_weighted(80, 24, &[1, 3], NativePaneLayoutAxis::Columns);
        let tiled_statuses = native_pane_statuses(
            &panes,
            1,
            &layouts,
            &HashMap::new(),
            NativePaneLayoutMode::Tiled,
            None,
        );
        assert_eq!(tiled_statuses[1].title_draggable, Some(true));
        assert_eq!(
            tiled_statuses[1].title_drag_kind.as_deref(),
            Some("reorder")
        );
        assert_eq!(tiled_statuses[1].title_drag_col, Some(layouts[1].x + 2));
        assert_eq!(tiled_statuses[1].title_drag_row, Some(layouts[1].y + 1));

        let statuses = native_pane_statuses(
            &panes,
            1,
            &layouts,
            &HashMap::new(),
            NativePaneLayoutMode::Floating,
            Some(NativeActiveDragLabel {
                kind: "move",
                window: "native-2",
            }),
        );
        assert_eq!(statuses.len(), 2);
        assert!(!statuses[0].focused);
        assert!(statuses[1].focused);
        assert_eq!(statuses[1].window, "native-2");
        assert_eq!(statuses[1].title, "editor");
        assert_eq!(statuses[1].weight, 3);
        assert_eq!(statuses[1].stack_index, Some(1));
        assert_eq!(statuses[1].stack_top, Some(true));
        assert_eq!(statuses[1].title_draggable, Some(true));
        assert_eq!(statuses[0].title_draggable, Some(true));
        assert_eq!(statuses[1].title_drag_kind.as_deref(), Some("reposition"));
        assert_eq!(statuses[0].title_drag_active, Some(false));
        assert_eq!(statuses[1].title_drag_active, Some(true));
        assert_eq!(statuses[1].title_drag_col, Some(layouts[1].x + 4));
        assert_eq!(statuses[1].title_drag_row, Some(layouts[1].y + 1));
        assert_eq!(statuses[1].pid, Some(202));
        assert_eq!(statuses[1].command.as_deref(), Some("editor-cmd"));
        let layout = layouts[1];
        assert_eq!(statuses[1].x, Some(layout.x));
        assert_eq!(statuses[1].y, Some(layout.y));
        assert_eq!(statuses[1].cols, Some(layout.cols));
        assert_eq!(statuses[1].rows, Some(layout.rows));
        assert_eq!(statuses[1].app_x, Some(layout.app_x));
        assert_eq!(statuses[1].app_y, Some(layout.app_y));
        assert_eq!(statuses[1].app_cols, Some(layout.app_cols));
        assert_eq!(statuses[1].app_rows, Some(layout.app_rows));
        assert!(statuses[1].app_x.unwrap() > statuses[1].x.unwrap());
        assert!(statuses[1].app_cols.unwrap() < statuses[1].cols.unwrap());
    }

    fn dummy_native_pane_app() -> NativeTerminalApp {
        let program = resolve_test_program("true").unwrap_or_else(|| "true".to_string());
        NativeTerminalApp::Pty(PtyTerminalApp::spawn_program(&program, &[], 1, 1).unwrap())
    }

    fn resolve_test_program(name: &str) -> Option<String> {
        let candidate = std::path::Path::new(name);
        if candidate.components().count() > 1 && candidate.exists() {
            return Some(name.to_string());
        }
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|candidate| candidate.is_file())
                .map(|candidate| candidate.to_string_lossy().into_owned())
        })
    }
}

fn native_terminal_size() -> (u16, u16) {
    let host = host_terminal_cells().unwrap_or((80, 24));
    let requested_cols = std::env::var("KITTWM_NATIVE_COLS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(host.0);
    let requested_rows = std::env::var("KITTWM_NATIVE_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| host.1.saturating_sub(2).max(1));
    clamp_native_terminal_size(requested_cols, requested_rows, host)
}

fn clamp_native_terminal_size(
    requested_cols: u16,
    requested_rows: u16,
    host: (u16, u16),
) -> (u16, u16) {
    let host_cols = host.0.max(1).min(MAX_NATIVE_TERMINAL_COLS);
    let host_rows = host.1.max(1).min(MAX_NATIVE_TERMINAL_ROWS);
    (
        requested_cols.max(1).min(host_cols),
        requested_rows.max(1).min(host_rows),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostTerminalSnapshot {
    cols: u16,
    rows: u16,
    width_px: Option<u32>,
    height_px: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostTerminalAxis {
    Columns,
    Rows,
}

fn host_terminal_cells() -> Option<(u16, u16)> {
    host_terminal_snapshot().map(|snapshot| (snapshot.cols, snapshot.rows))
}

fn host_terminal_snapshot() -> Option<HostTerminalSnapshot> {
    let ws = ioctl_terminal_winsize();
    let cells = host_terminal_cells_from_sources(
        ws.as_ref().map(|ws| ws.ws_col),
        ws.as_ref().map(|ws| ws.ws_row),
        std::env::var("COLUMNS").ok().as_deref(),
        std::env::var("LINES").ok().as_deref(),
    )?;
    let pixels = host_terminal_pixels_from_sources(
        ws.as_ref().map(|ws| u32::from(ws.ws_xpixel)),
        ws.as_ref().map(|ws| u32::from(ws.ws_ypixel)),
        std::env::var("KITTWM_HOST_TERMINAL_WIDTH_PX")
            .or_else(|_| std::env::var("KITTWM_HOST_WIDTH_PX"))
            .ok()
            .as_deref(),
        std::env::var("KITTWM_HOST_TERMINAL_HEIGHT_PX")
            .or_else(|_| std::env::var("KITTWM_HOST_HEIGHT_PX"))
            .ok()
            .as_deref(),
        remote_ssh_session(),
    );
    Some(HostTerminalSnapshot {
        cols: cells.0,
        rows: cells.1,
        width_px: pixels.map(|(width, _)| width),
        height_px: pixels.map(|(_, height)| height),
    })
}

fn ioctl_terminal_winsize() -> Option<libc::winsize> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    (rc == 0).then_some(ws)
}

fn host_terminal_cells_from_sources(
    ioctl_cols: Option<u16>,
    ioctl_rows: Option<u16>,
    env_cols: Option<&str>,
    env_rows: Option<&str>,
) -> Option<(u16, u16)> {
    match (
        ioctl_cols.filter(|cols| *cols > 0),
        ioctl_rows.filter(|rows| *rows > 0),
    ) {
        (Some(cols), Some(rows)) => Some((cols, rows)),
        _ => match (
            env_cols.and_then(parse_positive_u16),
            env_rows.and_then(parse_positive_u16),
        ) {
            (Some(cols), Some(rows)) => Some((cols, rows)),
            _ => None,
        },
    }
}

fn host_terminal_pixels_from_sources(
    ioctl_width_px: Option<u32>,
    ioctl_height_px: Option<u32>,
    env_width_px: Option<&str>,
    env_height_px: Option<&str>,
    remote_ssh: bool,
) -> Option<(u32, u32)> {
    match (
        env_width_px.and_then(parse_positive_u32),
        env_height_px.and_then(parse_positive_u32),
    ) {
        (Some(width), Some(height)) => Some((width, height)),
        _ if remote_ssh => match (
            ioctl_width_px.filter(|width| *width > 0),
            ioctl_height_px.filter(|height| *height > 0),
        ) {
            (Some(width), Some(height)) => Some((width, height)),
            _ => None,
        },
        _ => None,
    }
}

fn parse_positive_u16(value: &str) -> Option<u16> {
    value.parse::<u16>().ok().filter(|value| *value > 0)
}

fn parse_positive_u32(value: &str) -> Option<u32> {
    value.parse::<u32>().ok().filter(|value| *value > 0)
}

fn remote_ssh_session() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
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

fn run_loop_entry_log_line(fps: u32, launch_on_f12: bool, launcher_overlay: bool) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("run_loop: enter fps= launch_on_f12= launcher_overlay=".len() + 32);
    out.push_str("run_loop: enter fps=");
    let _ = write!(out, "{fps}");
    out.push_str(" launch_on_f12=");
    out.push_str(if launch_on_f12 { "true" } else { "false" });
    out.push_str(" launcher_overlay=");
    out.push_str(if launcher_overlay { "true" } else { "false" });
    out
}

fn ctrl_c_debounce_log_line(count: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("ctrl-c press # within debounce window".len() + 3);
    out.push_str("ctrl-c press #");
    let _ = write!(out, "{count}");
    out.push_str(" within debounce window");
    out
}

fn keymap_prefix_entered_log_line(spec: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("keymap prefix entered: ".len() + 16);
    out.push_str("keymap prefix entered: ");
    let _ = write!(out, "{spec}");
    out
}

fn keymap_action_log_line(chord: &str, action: &str) -> String {
    let mut out = String::with_capacity("keymap action:  -> ".len() + chord.len() + action.len());
    out.push_str("keymap action: ");
    out.push_str(chord);
    out.push_str(" -> ");
    out.push_str(action);
    out
}

fn keymap_unbound_prefix_log_line(spec: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("keymap unbound prefix chord: ".len() + 16);
    out.push_str("keymap unbound prefix chord: ");
    let _ = write!(out, "{spec}");
    out
}

fn keymap_unbound_status_line(spec: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("unbound ".len() + 16);
    out.push_str("unbound ");
    let _ = write!(out, "{spec}");
    out
}

fn char_event_log_line(ch: char) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("char event: ".len() + 8);
    out.push_str("char event: ");
    let _ = write!(out, "{ch:?}");
    out
}

fn key_event_log_line(key: &Key) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("key event: ".len() + 24);
    out.push_str("key event: ");
    let _ = write!(out, "{key:?}");
    out
}

fn raw_frame_count_log_line(frame: u64, frames: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("frame :  raw frames".len() + 40);
    out.push_str("frame ");
    let _ = write!(out, "{frame}");
    out.push_str(": ");
    let _ = write!(out, "{frames}");
    out.push_str(" raw frames");
    out
}

fn frame_budget_blown_log_line(frame: u64, elapsed_ms: u128, target_ms: u128) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("frame  budget blown:  ms (target  ms)".len() + 64);
    out.push_str("frame ");
    let _ = write!(out, "{frame}");
    out.push_str(" budget blown: ");
    let _ = write!(out, "{elapsed_ms}");
    out.push_str(" ms (target ");
    let _ = write!(out, "{target_ms}");
    out.push_str(" ms)");
    out
}

fn stdin_read_log_line(count: usize, preview: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("stdin read  bytes: []".len() + preview.len() * 4 + 24);
    out.push_str("stdin read ");
    let _ = write!(out, "{count}");
    out.push_str(" bytes: ");
    let _ = write!(out, "{preview:02x?}");
    out
}

fn compose_error_log_line(message: &str) -> String {
    let mut out = String::with_capacity("compose err: ".len() + message.len());
    out.push_str("compose err: ");
    out.push_str(message);
    out
}

fn keymap_reload_failed_log_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("keymap reload failed, keeping previous keymap: ".len() + 80);
    out.push_str("keymap reload failed, keeping previous keymap: ");
    let _ = write!(out, "{err}");
    out
}

fn config_action_log_line(message: &str) -> String {
    let mut out = String::with_capacity("config action: ".len() + message.len());
    out.push_str("config action: ");
    out.push_str(message);
    out
}

fn split_action_log_line(message: &str) -> String {
    let mut out = String::with_capacity("split action: ".len() + message.len());
    out.push_str("split action: ");
    out.push_str(message);
    out
}

fn workspace_action_log_line(message: &str) -> String {
    let mut out = String::with_capacity("workspace action: ".len() + message.len());
    out.push_str("workspace action: ");
    out.push_str(message);
    out
}

fn action_window_status_line(message: &str, window_id: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(message.len() + " window=".len() + 10);
    out.push_str(message);
    out.push_str(" window=");
    let _ = write!(out, "{window_id}");
    out
}

fn action_window_mode_status_line(message: &str, window_id: u32, mode: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(message.len() + " window= mode=".len() + mode.len() + 10);
    out.push_str(message);
    out.push_str(" window=");
    let _ = write!(out, "{window_id}");
    out.push_str(" mode=");
    out.push_str(mode);
    out
}

fn action_window_fullscreen_status_line(message: &str, window_id: u32, fullscreen: bool) -> String {
    use std::fmt::Write as _;

    let fullscreen = if fullscreen { "true" } else { "false" };
    let mut out =
        String::with_capacity(message.len() + " window= fullscreen=".len() + fullscreen.len() + 10);
    out.push_str(message);
    out.push_str(" window=");
    let _ = write!(out, "{window_id}");
    out.push_str(" fullscreen=");
    out.push_str(fullscreen);
    out
}

fn action_windows_count_status_line(message: &str, count: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(message.len() + " windows=".len() + 20);
    out.push_str(message);
    out.push_str(" windows=");
    let _ = write!(out, "{count}");
    out
}

fn action_no_window_status_line(message: &str) -> String {
    let mut out = String::with_capacity(message.len() + " window=-".len());
    out.push_str(message);
    out.push_str(" window=-");
    out
}

fn action_error_status_line(message: &str, err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(message.len() + " error=".len() + 80);
    out.push_str(message);
    out.push_str(" error=");
    let _ = write!(out, "{err}");
    out
}

fn focus_action_log_line(message: &str) -> String {
    let mut out = String::with_capacity("focus action: ".len() + message.len());
    out.push_str("focus action: ");
    out.push_str(message);
    out
}

fn swap_action_log_line(message: &str) -> String {
    let mut out = String::with_capacity("swap action: ".len() + message.len());
    out.push_str("swap action: ");
    out.push_str(message);
    out
}

fn toggle_action_log_line(message: &str) -> String {
    let mut out = String::with_capacity("toggle action: ".len() + message.len());
    out.push_str("toggle action: ");
    out.push_str(message);
    out
}

fn layout_action_log_line(message: &str) -> String {
    let mut out = String::with_capacity("layout action: ".len() + message.len());
    out.push_str("layout action: ");
    out.push_str(message);
    out
}

fn picker_select_status_line(label: &str) -> String {
    let mut out = String::with_capacity("picker.select ".len() + label.len());
    out.push_str("picker.select ");
    out.push_str(label);
    out
}

fn picker_selected_log_line(label: &str) -> String {
    let mut out = String::with_capacity("picker selected ".len() + label.len());
    out.push_str("picker selected ");
    out.push_str(label);
    out
}

fn launcher_overlay_launch_failed_log_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("launcher overlay launch failed: ".len() + 80);
    out.push_str("launcher overlay launch failed: ");
    let _ = write!(out, "{err}");
    out
}

fn launcher_overlay_opened_log_line(query: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("launcher overlay opened query=".len() + query.len() + 2);
    out.push_str("launcher overlay opened query=");
    let _ = write!(out, "{query:?}");
    out
}

fn split_launcher_overlay_opened_log_line(query: &str) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("split opened launcher overlay query=".len() + query.len() + 2);
    out.push_str("split opened launcher overlay query=");
    let _ = write!(out, "{query:?}");
    out
}

fn launcher_action_status_line(selection: &LauncherSelection) -> String {
    let kind = selection.kind_name();
    let mut out =
        String::with_capacity("launcher.launch :".len() + kind.len() + selection.command.len());
    out.push_str("launcher.launch ");
    out.push_str(kind);
    out.push(':');
    out.push_str(&selection.command);
    out
}

fn launcher_error_status_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("launcher.error ".len() + 80);
    out.push_str("launcher.error ");
    let _ = write!(out, "{err}");
    out
}

fn launcher_overlay_selected_log_line(kind: LauncherKind, command: &str, pid: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "launcher overlay selected  \"\" spawned pid=".len() + command.len() + 32,
    );
    out.push_str("launcher overlay selected ");
    let _ = write!(out, "{kind:?}");
    out.push(' ');
    let _ = write!(out, "{command:?}");
    out.push_str(" spawned pid=");
    let _ = write!(out, "{pid}");
    out
}

fn keymap_launcher_selected_log_line(kind: LauncherKind, command: &str, pid: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "keymap launcher selected  \"\" spawned pid=".len() + command.len() + 32,
    );
    out.push_str("keymap launcher selected ");
    let _ = write!(out, "{kind:?}");
    out.push(' ');
    let _ = write!(out, "{command:?}");
    out.push_str(" spawned pid=");
    let _ = write!(out, "{pid}");
    out
}

fn keymap_launcher_failed_log_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("keymap launcher failed: ".len() + 80);
    out.push_str("keymap launcher failed: ");
    let _ = write!(out, "{err}");
    out
}

fn split_launcher_selected_log_line(kind: LauncherKind, command: &str, pid: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "split launcher selected  \"\" spawned pid=".len() + command.len() + 32,
    );
    out.push_str("split launcher selected ");
    let _ = write!(out, "{kind:?}");
    out.push(' ');
    let _ = write!(out, "{command:?}");
    out.push_str(" spawned pid=");
    let _ = write!(out, "{pid}");
    out
}

fn split_launcher_failed_log_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("split launcher failed: ".len() + 80);
    out.push_str("split launcher failed: ");
    let _ = write!(out, "{err}");
    out
}

fn launcher_f12_overlay_opened_log_line(query: &str) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("launcher F12 opened overlay query=".len() + query.len() + 2);
    out.push_str("launcher F12 opened overlay query=");
    let _ = write!(out, "{query:?}");
    out
}

fn launcher_f12_selected_log_line(kind: LauncherKind, command: &str, pid: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(
        "launcher F12 selected  \"\" spawned pid=".len() + command.len() + 32,
    );
    out.push_str("launcher F12 selected ");
    let _ = write!(out, "{kind:?}");
    out.push(' ');
    let _ = write!(out, "{command:?}");
    out.push_str(" spawned pid=");
    let _ = write!(out, "{pid}");
    out
}

fn launcher_f12_failed_log_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("launcher F12 failed: ".len() + 80);
    out.push_str("launcher F12 failed: ");
    let _ = write!(out, "{err}");
    out
}

fn keymap_action_unimplemented_log_line(action: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("keymap action not implemented yet: ".len() + 32);
    out.push_str("keymap action not implemented yet: ");
    let _ = write!(out, "{action}");
    out
}

fn loaded_keymap_log_line(path: &str) -> String {
    let mut out = String::with_capacity("loaded keymap from ".len() + path.len());
    out.push_str("loaded keymap from ");
    out.push_str(path);
    out
}

fn runtime_keymap_load_failed_log_line(err: &dyn std::fmt::Display) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("failed to load runtime keymap: ; using defaults".len() + 80);
    out.push_str("failed to load runtime keymap: ");
    let _ = write!(out, "{err}");
    out.push_str("; using defaults");
    out
}

pub fn run_loop_with<S: XServer>(
    runtime: &Runtime,
    compositor: &Compositor<S>,
    layout: &Layout,
    opts: RunOptions,
) -> Result<()> {
    let mut layout = layout.clone();
    let dbg = Debugger::open();
    dbg.log(&run_loop_entry_log_line(
        opts.fps,
        opts.launch_on_f12,
        opts.launcher_overlay,
    ));
    let _raw_guard = RawMode::enter()?;
    dbg.log("raw mode + alt screen entered");
    install_signal_restore();

    let fps = opts.fps.clamp(1, 240);
    let frame_target = Duration::from_micros(1_000_000 / fps as u64);
    let idle_frame_target = native_idle_frame_target(frame_target);
    let mut consecutive_idle_frames = 0u16;
    // Live fps tracking: instantaneous over the last 30 frames + peak.
    let mut fps_window_start = std::time::Instant::now();
    let mut fps_window_frames = 0u32;
    let mut live_fps: f32 = 0.0;
    let mut peak_fps: f32 = 0.0;
    let footer_refresh_interval = raw_compositor_footer_refresh_interval();
    let mut frame = 0u64;
    let mut input_buf = Vec::<u8>::with_capacity(256);
    let mut stdin = io::stdin();
    let mut last_launch_pid: Option<u32> = None;
    let mut keymap = load_runtime_keymap(&dbg);
    let mut prefix_active = false;
    let mut last_keymap_action: Option<String> = None;
    let mut workspaces = WorkspaceState::default();
    publish_workspace_label_for_status(&workspaces.active_label());
    let mut focus_state = FocusState::default();
    let mut swap_state = SwapState::default();
    let mut toggle_state = ToggleState::default();
    let mut layout_state = LayoutState::default();
    let mut split_state = SplitState::default();
    let mut config_state = ConfigState::default();
    let mut launcher_overlay = LauncherOverlay::default();
    let mut picker_overlay = PickerOverlay::default();
    let mut quit_confirm_overlay = QuitConfirmOverlay::default();
    let mut launcher_overlay_was_active = false;
    let mut picker_overlay_was_active = false;
    let mut quit_confirm_overlay_was_active = false;
    let mut text_overlay_hid_raw_graphics = false;
    let mut last_footer_key = String::new();
    let mut last_footer_row: Option<u16> = None;
    let mut last_launcher_overlay_key = String::new();
    let mut last_picker_overlay_key = String::new();
    let mut last_quit_confirm_overlay_key = String::new();
    let mut last_error_key: Option<String> = None;
    // Triple-Ctrl-C quit guard: single Ctrl-C is forwarded to the focused
    // window like any other key; three within 1s opens an explicit
    // confirmation dialog instead of exiting immediately.
    let mut ctrl_c_guard = CtrlCGuard::new();
    // Per-window placement and content memo. We only re-upload raw RGBA
    // payloads when pixels/dimensions change, and only re-emit placement when
    // the footprint moves. Kitty keeps the same image id live between frames.
    let mut last_placed: std::collections::HashMap<u32, CellRect> =
        std::collections::HashMap::new();
    let mut last_raw_hashes: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    let mut last_raw_chrome_keys: std::collections::HashMap<u32, String> =
        std::collections::HashMap::new();
    // Set of window image-ids seen on the previous frame so we can delete
    // ones that disappear without redrawing the whole screen.
    let mut prev_window_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();

    loop {
        let frame_start = Instant::now();
        let mut input_activity = false;

        // Drain any pending stdin BEFORE the expensive compose, so q/Esc
        // takes effect even when a single frame is slow.
        let mut chunk = [0u8; 512];
        while poll_stdin(Duration::ZERO) {
            let n = stdin.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            input_activity = true;
            input_buf.extend_from_slice(&chunk[..n]);
        }
        let mut quit = false;
        while let Some((ev, consumed)) = kittui_input::parse(&input_buf) {
            input_buf.drain(..consumed);
            let now = Instant::now();
            if quit_confirm_overlay.expired(now) {
                quit_confirm_overlay.close();
                last_keymap_action = Some("quit.confirm.timeout".to_string());
                dbg.log("quit confirmation timed out");
            }
            if quit_confirm_overlay.active {
                match quit_confirm_overlay.handle_event(&ev, now) {
                    QuitConfirmEvent::Consumed => continue,
                    QuitConfirmEvent::Cancel => {
                        quit_confirm_overlay.close();
                        ctrl_c_guard.clear();
                        last_keymap_action = Some("quit.cancel".to_string());
                        dbg.log("quit confirmation cancelled");
                        continue;
                    }
                    QuitConfirmEvent::Confirm => {
                        dbg.log("quit confirmation accepted");
                        last_keymap_action = Some("quit.confirm".to_string());
                        quit = true;
                        break;
                    }
                    QuitConfirmEvent::NotHandled => {}
                }
            }
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
                        last_keymap_action =
                            Some(picker_select_status_line(&picker_overlay.selection_label()));
                        dbg.log(&picker_selected_log_line(&picker_overlay.selection_label()));
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
                                    last_keymap_action = Some(launcher_action_status_line(&sel));
                                    dbg.log(&launcher_overlay_selected_log_line(
                                        sel.kind,
                                        &sel.command,
                                        pid,
                                    ));
                                }
                                Err(e) => {
                                    last_keymap_action = Some(launcher_error_status_line(&e));
                                    dbg.log(&launcher_overlay_launch_failed_log_line(&e));
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
                    dbg.log(&keymap_prefix_entered_log_line(&spec));
                    continue;
                }
                if prefix_active {
                    prefix_active = false;
                    if let Some(prefix) = keymap.prefix.as_ref() {
                        let chord = vec![prefix.clone(), spec.clone()];
                        if let Some(action) = keymap.action_for_chord(&chord).cloned() {
                            let action_name = action.to_string();
                            last_keymap_action = Some(action_name.clone());
                            dbg.log(&keymap_action_log_line(
                                &crate::keymap::chord_string(&chord),
                                &action_name,
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
                                        dbg.log(&launcher_overlay_opened_log_line(
                                            &launcher_overlay.query,
                                        ));
                                    } else {
                                        let selection = launcher_selection();
                                        match spawn_launcher_command() {
                                            Ok(pid) => {
                                                last_launch_pid = Some(pid);
                                                dbg.log(&keymap_launcher_selected_log_line(
                                                    selection.kind,
                                                    &selection.command,
                                                    pid,
                                                ));
                                            }
                                            Err(e) => {
                                                last_keymap_action =
                                                    Some(launcher_error_status_line(&e));
                                                dbg.log(&keymap_launcher_failed_log_line(&e));
                                            }
                                        }
                                    }
                                }
                                Action::SplitVerticalLauncher | Action::SplitHorizontalLauncher => {
                                    let msg = split_state.apply(&action);
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&split_action_log_line(&msg));
                                    if opts.launcher_overlay {
                                        launcher_overlay.open_from_env();
                                        dbg.log(&split_launcher_overlay_opened_log_line(
                                            &launcher_overlay.query,
                                        ));
                                    } else {
                                        let selection = launcher_selection();
                                        match spawn_launcher_command() {
                                            Ok(pid) => {
                                                last_launch_pid = Some(pid);
                                                dbg.log(&split_launcher_selected_log_line(
                                                    selection.kind,
                                                    &selection.command,
                                                    pid,
                                                ));
                                            }
                                            Err(e) => {
                                                last_keymap_action =
                                                    Some(launcher_error_status_line(&e));
                                                dbg.log(&split_launcher_failed_log_line(&e));
                                            }
                                        }
                                    }
                                }
                                Action::WorkspaceNew
                                | Action::WorkspaceNext
                                | Action::WorkspacePrev
                                | Action::WorkspaceSwitch(_) => {
                                    let msg = workspaces.apply(&action);
                                    publish_workspace_label_for_status(&workspaces.active_label());
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&workspace_action_log_line(&msg));
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
                                        Ok(Some(id)) => action_window_status_line(&msg, id.0),
                                        Ok(None) => action_no_window_status_line(&msg),
                                        Err(e) => action_error_status_line(&msg, &e),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&focus_action_log_line(&msg));
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
                                        Ok(Some(id)) => action_window_status_line(&msg, id.0),
                                        Ok(None) => action_no_window_status_line(&msg),
                                        Err(e) => action_error_status_line(&msg, &e),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&swap_action_log_line(&msg));
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
                                            action_window_mode_status_line(&msg, id.0, mode)
                                        }
                                        Ok(None) => action_no_window_status_line(&msg),
                                        Err(e) => action_error_status_line(&msg, &e),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&toggle_action_log_line(&msg));
                                }
                                Action::FullscreenToggle => {
                                    let msg = toggle_state.apply(&action);
                                    let msg = match compositor.toggle_focused_fullscreen() {
                                        Ok(Some((id, fullscreen))) => {
                                            action_window_fullscreen_status_line(
                                                &msg, id.0, fullscreen,
                                            )
                                        }
                                        Ok(None) => action_no_window_status_line(&msg),
                                        Err(e) => action_error_status_line(&msg, &e),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&toggle_action_log_line(&msg));
                                }
                                Action::ToggleSplit | Action::BalanceWindows => {
                                    let msg = layout_state.apply(&action);
                                    let msg = match rebuild_tiled_layout(
                                        compositor,
                                        &mut layout,
                                        &layout_state,
                                    ) {
                                        Ok(count) => action_windows_count_status_line(&msg, count),
                                        Err(e) => action_error_status_line(&msg, &e),
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&layout_action_log_line(&msg));
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
                                            dbg.log(&keymap_reload_failed_log_line(&e));
                                            msg
                                        }
                                    };
                                    last_keymap_action = Some(msg.clone());
                                    dbg.log(&config_action_log_line(&msg));
                                }
                                Action::Quit => {
                                    quit = true;
                                    break;
                                }
                                other => {
                                    dbg.log(&keymap_action_unimplemented_log_line(&other));
                                }
                            }
                            continue;
                        }
                    }
                    last_keymap_action = Some(keymap_unbound_status_line(&spec));
                    dbg.log(&keymap_unbound_prefix_log_line(&spec));
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
                dbg.log(&ctrl_c_debounce_log_line(count));
                if count >= CTRL_C_TRIGGER {
                    dbg.log("ctrl-c triple-press opened quit confirmation");
                    ctrl_c_guard.clear();
                    quit_confirm_overlay.open(Instant::now());
                    last_keymap_action = Some("quit.confirm.open".to_string());
                    continue;
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
                    dbg.log(&char_event_log_line(*ch));
                    let _ = compositor.route_key(&ev);
                }
                InputEvent::Key {
                    key: Key::F(12), ..
                } if opts.launch_on_f12 => {
                    if opts.launcher_overlay {
                        launcher_overlay.open_from_env();
                        last_keymap_action = Some("launcher.open".to_string());
                        dbg.log(&launcher_f12_overlay_opened_log_line(
                            &launcher_overlay.query,
                        ));
                    } else {
                        let selection = launcher_selection();
                        match spawn_launcher_command() {
                            Ok(pid) => {
                                last_launch_pid = Some(pid);
                                dbg.log(&launcher_f12_selected_log_line(
                                    selection.kind,
                                    &selection.command,
                                    pid,
                                ));
                            }
                            Err(e) => {
                                last_keymap_action = Some(launcher_error_status_line(&e));
                                dbg.log(&launcher_f12_failed_log_line(&e));
                            }
                        }
                    }
                }
                InputEvent::Key { key, .. } => {
                    dbg.log(&key_event_log_line(&key));
                    let _ = compositor.route_key(&ev);
                }
                _ => {}
            }
            if consumed == 0 {
                break;
            }
        }
        if !quit && quit_confirm_overlay.expired(Instant::now()) {
            quit_confirm_overlay.close();
            last_keymap_action = Some("quit.confirm.timeout".to_string());
            dbg.log("quit confirmation timed out");
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
                let recovering_from_error =
                    should_clear_raw_error_screen(last_error_key.as_deref());
                if recovering_from_error {
                    last_error_key = None;
                }
                let last_window_count = frames.len();
                if frame % 30 == 0 {
                    dbg.log(&raw_frame_count_log_line(frame, frames.len()));
                }
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                let mut frame_out = NativeFrameWriteBatch::default();
                let mut wrote_frame_output = false;
                if recovering_from_error {
                    write!(frame_out, "\x1b[H\x1b[2J")?;
                    last_placed.clear();
                    last_raw_hashes.clear();
                    last_raw_chrome_keys.clear();
                    last_launcher_overlay_key.clear();
                    last_picker_overlay_key.clear();
                    last_quit_confirm_overlay_key.clear();
                    last_footer_key.clear();
                    last_footer_row = None;
                    wrote_frame_output = true;
                }
                // If a text overlay just closed, erase its rows and force
                // image/chrome placeholders to be re-emitted underneath.
                // Without this, boxed overlay glyphs remain burned into the
                // terminal cells even though the overlay state is inactive.
                if should_clear_raw_overlay_area(
                    launcher_overlay_was_active,
                    launcher_overlay.active,
                    picker_overlay_was_active,
                    picker_overlay.active,
                    quit_confirm_overlay_was_active,
                    quit_confirm_overlay.active,
                ) {
                    clear_launcher_overlay_area(&mut frame_out)?;
                    wrote_frame_output = true;
                    last_placed.clear();
                    last_raw_hashes.clear();
                    last_raw_chrome_keys.clear();
                    last_launcher_overlay_key.clear();
                    last_quit_confirm_overlay_key.clear();
                    last_footer_key.clear();
                    text_overlay_hid_raw_graphics = false;
                }
                // Track which windows are present this frame so we can
                // delete the ones that have disappeared.
                let mut current_ids: std::collections::HashSet<u32> =
                    std::collections::HashSet::with_capacity(frames.len());
                let mut footer_row = 2u16;
                let text_overlay_active =
                    launcher_overlay.active || picker_overlay.active || quit_confirm_overlay.active;
                let render_app_graphics =
                    raw_compositor_should_render_app_graphics(text_overlay_active);
                for f in &frames {
                    current_ids.insert(f.image_id);
                    if !render_app_graphics {
                        footer_row = footer_row.max(f.footprint.y + f.footprint.rows + 2);
                        continue;
                    }
                    let had_previous_placement = last_placed.contains_key(&f.image_id);
                    let footprint_changed = last_placed.get(&f.image_id) != Some(&f.footprint);
                    let chrome_key = raw_frame_chrome_key(
                        &f.title,
                        f.focused,
                        f.mode,
                        f.fullscreen,
                        f.footprint,
                    );
                    let chrome_changed = last_raw_chrome_keys.get(&f.image_id) != Some(&chrome_key);
                    let decision = raw_frame_write_with_chrome_change(
                        decide_native_raw_frame_write(
                            &mut last_raw_hashes,
                            &mut last_placed,
                            f.image_id,
                            f.footprint,
                            f.width,
                            f.height,
                            &f.rgba,
                        ),
                        chrome_changed,
                    );
                    let placement_options = raw_compositor_app_placement_options(f.image_id);
                    if should_unplace_raw_frame_before_move(
                        had_previous_placement,
                        footprint_changed,
                    ) {
                        frame_out.write_all(runtime.unplace(f.image_id).as_bytes())?;
                        wrote_frame_output = true;
                    }
                    if decision.upload {
                        let p = runtime.place_raw_frame_with_options(
                            f.image_id,
                            &f.rgba,
                            f.width,
                            f.height,
                            f.footprint,
                            &placement_options,
                        );
                        frame_out.write_all(p.upload.as_bytes())?;
                        wrote_frame_output = true;
                        if decision.placement.write_placement {
                            frame_out.write_all(p.placement.as_bytes())?;
                            frame_out.write_all(p.embed.as_bytes())?;
                            wrote_frame_output = true;
                        }
                    } else if decision.placement.write_placement {
                        let p = runtime.place_uploaded_image_with_options(
                            f.image_id,
                            f.footprint,
                            &placement_options,
                        );
                        frame_out.write_all(p.placement.as_bytes())?;
                        frame_out.write_all(p.embed.as_bytes())?;
                        wrote_frame_output = true;
                    }
                    if chrome_changed {
                        write_raw_frame_chrome(&mut frame_out, f)?;
                        wrote_frame_output = true;
                        last_raw_chrome_keys.insert(f.image_id, chrome_key);
                    }
                    footer_row = footer_row.max(f.footprint.y + f.footprint.rows + 2);
                }
                if should_hide_raw_graphics_for_text_overlay(
                    text_overlay_active,
                    text_overlay_hid_raw_graphics,
                ) {
                    for image_id in &current_ids {
                        frame_out.write_all(runtime.unplace(*image_id).as_bytes())?;
                        wrote_frame_output = true;
                    }
                    last_placed.clear();
                    last_raw_hashes.clear();
                    last_raw_chrome_keys.clear();
                    text_overlay_hid_raw_graphics = true;
                }
                // Delete any window that disappeared since last frame.
                for old_id in prev_window_ids.difference(&current_ids) {
                    frame_out.write_all(runtime.unplace(*old_id).as_bytes())?;
                    wrote_frame_output = true;
                    last_placed.remove(old_id);
                    last_raw_hashes.remove(old_id);
                    last_raw_chrome_keys.remove(old_id);
                }
                prev_window_ids = current_ids;
                if launcher_overlay.active {
                    let overlay_key = launcher_overlay_key(&launcher_overlay);
                    if last_launcher_overlay_key != overlay_key {
                        let (_, terminal_rows) = host_terminal_cells().unwrap_or((80, 24));
                        launcher_overlay.render(&mut frame_out, terminal_rows)?;
                        wrote_frame_output = true;
                        last_launcher_overlay_key = overlay_key;
                    }
                } else {
                    last_launcher_overlay_key.clear();
                }
                if picker_overlay.active {
                    let overlay_key = picker_overlay_key(&picker_overlay);
                    if last_picker_overlay_key != overlay_key {
                        let (_, terminal_rows) = host_terminal_cells().unwrap_or((80, 24));
                        picker_overlay.render(&mut frame_out, terminal_rows)?;
                        wrote_frame_output = true;
                        last_picker_overlay_key = overlay_key;
                    }
                } else {
                    last_picker_overlay_key.clear();
                }
                if quit_confirm_overlay.active {
                    let overlay_key = quit_confirm_overlay_key(&quit_confirm_overlay);
                    if last_quit_confirm_overlay_key != overlay_key {
                        let (_, terminal_rows) = host_terminal_cells().unwrap_or((80, 24));
                        quit_confirm_overlay.render(&mut frame_out, terminal_rows)?;
                        wrote_frame_output = true;
                        last_quit_confirm_overlay_key = overlay_key;
                    }
                } else {
                    last_quit_confirm_overlay_key.clear();
                }
                let (terminal_cols, terminal_rows) = host_terminal_cells().unwrap_or((80, 24));
                let safe_footer_row = raw_compositor_footer_row_for_overlays(
                    footer_row,
                    launcher_overlay.active,
                    picker_overlay.active,
                    terminal_rows,
                );
                let launch_note = raw_compositor_footer_launch_note(last_launch_pid);
                let keymap_note =
                    raw_compositor_footer_keymap_note(prefix_active, last_keymap_action.as_deref());
                if let Some(footer_row) = safe_footer_row {
                    let workspace_label = workspaces.label();
                    let split_label = split_state.label();
                    let layout_label = layout_state.label();
                    let config_label = config_state.label();
                    let focus_label = focus_state.label();
                    let swap_label = swap_state.label();
                    let mode_label = toggle_state.label();
                    let quit_hint = ctrl_c_guard.quit_hint(last_window_count > 0);
                    let footer_key = raw_compositor_footer_key(
                        footer_row,
                        &workspace_label,
                        &split_label,
                        &layout_label,
                        &config_label,
                        &focus_label,
                        &swap_label,
                        &mode_label,
                        last_window_count,
                        &launch_note,
                        &keymap_note,
                        quit_hint,
                    );
                    if should_write_compositor_footer(
                        &last_footer_key,
                        &footer_key,
                        frame,
                        footer_refresh_interval,
                    ) {
                        if let Some(old_row) = last_footer_row {
                            if old_row != footer_row {
                                write!(frame_out, "\x1b[0m\x1b[{};1H\x1b[K", old_row)?;
                            }
                        }
                        let footer_text = raw_compositor_footer_text(
                            frame,
                            &workspace_label,
                            &split_label,
                            &layout_label,
                            &config_label,
                            &focus_label,
                            &swap_label,
                            &mode_label,
                            last_window_count,
                            live_fps,
                            peak_fps,
                            fps,
                            &launch_note,
                            &keymap_note,
                            quit_hint,
                            &dbg.path_display(),
                            terminal_cols,
                        );
                        write!(
                            frame_out,
                            "\x1b[0m\x1b[{};1H\x1b[K{}",
                            footer_row, footer_text
                        )?;
                        wrote_frame_output = true;
                        last_footer_key = footer_key;
                        last_footer_row = Some(footer_row);
                    }
                } else {
                    if let Some(old_row) = last_footer_row.take() {
                        if old_row <= terminal_rows {
                            write!(frame_out, "\x1b[0m\x1b[{};1H\x1b[K", old_row)?;
                            wrote_frame_output = true;
                        }
                    }
                    last_footer_key.clear();
                }
                launcher_overlay_was_active = launcher_overlay.active;
                picker_overlay_was_active = picker_overlay.active;
                quit_confirm_overlay_was_active = quit_confirm_overlay.active;
                let emitted = if should_flush_compositor_frame(wrote_frame_output) {
                    frame_out.write_to(&mut handle)?
                } else {
                    false
                };
                update_native_idle_counter_for_activity(
                    &mut consecutive_idle_frames,
                    emitted,
                    input_activity,
                );
            }
            Err(e) => {
                let msg = e.to_string();
                let error_key = raw_compositor_error_key(&msg, &dbg.path_display());
                if should_write_raw_compositor_error(last_error_key.as_deref(), &error_key) {
                    dbg.log(&compose_error_log_line(&msg));
                    let stdout = io::stdout();
                    let mut handle = stdout.lock();
                    let error_text = raw_compositor_error_text(&msg);
                    let log_path = raw_compositor_error_log_path(dbg.path_display());
                    write!(
                        handle,
                        "\x1b[H\x1b[J\x1b[1mkittui-wm error\x1b[0m\n\n  {}\n\n  q/Esc to quit. On macOS, grant Screen Recording + Accessibility.\n  (log: {})\n",
                        error_text,
                        log_path
                    )?;
                    handle.flush()?;
                    update_native_idle_counter_for_activity(
                        &mut consecutive_idle_frames,
                        true,
                        input_activity,
                    );
                    last_error_key = Some(error_key);
                } else {
                    update_native_idle_counter_for_activity(
                        &mut consecutive_idle_frames,
                        false,
                        input_activity,
                    );
                }
                launcher_overlay_was_active = launcher_overlay.active;
                picker_overlay_was_active = picker_overlay.active;
                quit_confirm_overlay_was_active = quit_confirm_overlay.active;
            }
        }

        let current_frame_target = raw_compositor_current_frame_target(
            frame_target,
            idle_frame_target,
            consecutive_idle_frames,
        );
        let elapsed = frame_start.elapsed();
        let remaining = current_frame_target
            .checked_sub(elapsed)
            .unwrap_or_default();
        if remaining > Duration::ZERO {
            let mut chunk = [0u8; 512];
            // Brief stdin poll with a 1ms cap so even on a fd that returns
            // ready immediately we don't spin. Skip entirely when the
            // frame budget is already small.
            let poll_budget = remaining.min(Duration::from_millis(1));
            if poll_budget >= Duration::from_micros(500) && poll_stdin(poll_budget) {
                let n = stdin.read(&mut chunk).unwrap_or(0);
                if n > 0 {
                    update_native_idle_counter_for_activity(
                        &mut consecutive_idle_frames,
                        false,
                        true,
                    );
                    dbg.log(&stdin_read_log_line(n, &chunk[..n.min(32)]));
                    input_buf.extend_from_slice(&chunk[..n]);
                }
            }
            // Sleep out the rest of the frame budget so --fps actually caps,
            // but never longer than ~16ms at a stretch (preserves Ctrl-C
            // responsiveness on a backgrounded stdin).
            loop {
                let used = frame_start.elapsed();
                let Some(slack) = current_frame_target.checked_sub(used) else {
                    break;
                };
                if slack < Duration::from_micros(500) {
                    break;
                }
                std::thread::sleep(frame_sleep_chunk(slack));
            }
        } else {
            dbg.log(&frame_budget_blown_log_line(
                frame,
                elapsed.as_millis(),
                current_frame_target.as_millis(),
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

fn frame_sleep_chunk(slack: Duration) -> Duration {
    slack.min(Duration::from_millis(16))
}

const RAW_COMPOSITOR_FOOTER_KEY_NOTE_MAX_CHARS: usize = 96;

fn raw_compositor_footer_launch_note(last_launch_pid: Option<u32>) -> String {
    let Some(pid) = last_launch_pid else {
        return String::new();
    };
    let mut note = String::with_capacity(" — last launch pid=".len() + 10);
    note.push_str(" — last launch pid=");
    let _ = write!(note, "{pid}");
    note
}

fn raw_compositor_footer_keymap_note(
    prefix_active: bool,
    last_keymap_action: Option<&str>,
) -> String {
    if prefix_active {
        return " — keymap prefix".to_string();
    }
    let Some(action) = last_keymap_action else {
        return String::new();
    };
    let mut note = String::with_capacity(" — action=".len() + action.len());
    note.push_str(" — action=");
    note.push_str(action);
    note
}

#[allow(clippy::too_many_arguments)]
fn raw_compositor_footer_key(
    footer_row: u16,
    workspace: &str,
    split: &str,
    layout: &str,
    config: &str,
    focus: &str,
    swap: &str,
    mode: &str,
    window_count: usize,
    launch_note: &str,
    keymap_note: &str,
    quit_hint: &str,
) -> String {
    use std::fmt::Write as _;

    let launch_note = truncate_cells(launch_note, RAW_COMPOSITOR_FOOTER_KEY_NOTE_MAX_CHARS);
    let keymap_note = truncate_cells(keymap_note, RAW_COMPOSITOR_FOOTER_KEY_NOTE_MAX_CHARS);
    let mut out = String::with_capacity(
        "row=;ws=;panes=;layout=;cfg=;focus=;swap=;mode=;windows=;launch=;keymap=;quit=".len()
            + workspace.len()
            + split.len()
            + layout.len()
            + config.len()
            + focus.len()
            + swap.len()
            + mode.len()
            + launch_note.len()
            + keymap_note.len()
            + quit_hint.len()
            + 32,
    );
    out.push_str("row=");
    let _ = write!(out, "{footer_row}");
    out.push_str(";ws=");
    out.push_str(workspace);
    out.push_str(";panes=");
    out.push_str(split);
    out.push_str(";layout=");
    out.push_str(layout);
    out.push_str(";cfg=");
    out.push_str(config);
    out.push_str(";focus=");
    out.push_str(focus);
    out.push_str(";swap=");
    out.push_str(swap);
    out.push_str(";mode=");
    out.push_str(mode);
    out.push_str(";windows=");
    let _ = write!(out, "{window_count}");
    out.push_str(";launch=");
    out.push_str(&launch_note);
    out.push_str(";keymap=");
    out.push_str(&keymap_note);
    out.push_str(";quit=");
    out.push_str(quit_hint);
    out
}

fn raw_compositor_footer_fps_label(fps: f32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(8);
    let _ = write!(out, "{fps:.0}");
    out
}

fn raw_compositor_footer_text(
    frame: u64,
    workspace: &str,
    split: &str,
    layout: &str,
    config: &str,
    focus: &str,
    swap: &str,
    mode: &str,
    window_count: usize,
    live_fps: f32,
    peak_fps: f32,
    cap_fps: u32,
    launch_note: &str,
    keymap_note: &str,
    quit_hint: &str,
    log_path: &str,
    terminal_cols: u16,
) -> String {
    let max = terminal_cols.max(1) as usize;
    let mut out = String::with_capacity(max);
    let mut used = 0usize;
    macro_rules! push_footer {
        ($segment:expr) => {
            if !push_truncated_cells(&mut out, &mut used, max, $segment) {
                return out;
            }
        };
    }
    push_footer!("kittui-wm frame ");
    push_footer!(&frame.to_string());
    push_footer!(" — ws ");
    push_footer!(workspace);
    push_footer!(" — panes ");
    push_footer!(split);
    push_footer!(" — layout ");
    push_footer!(layout);
    push_footer!(" — cfg ");
    push_footer!(config);
    push_footer!(" — focus ");
    push_footer!(focus);
    push_footer!(" — swap ");
    push_footer!(swap);
    push_footer!(" — mode ");
    push_footer!(mode);
    push_footer!(" — ");
    push_footer!(&window_count.to_string());
    push_footer!(" windows — ");
    push_footer!(&raw_compositor_footer_fps_label(live_fps));
    push_footer!(" fps (peak ");
    push_footer!(&raw_compositor_footer_fps_label(peak_fps));
    push_footer!(", cap ");
    push_footer!(&cap_fps.to_string());
    push_footer!(")");
    push_footer!(launch_note);
    push_footer!(keymap_note);
    push_footer!(" — ");
    push_footer!(quit_hint);
    push_footer!(" (log: ");
    push_footer!(log_path);
    push_footer!(")");
    out
}

fn push_truncated_cells(out: &mut String, used: &mut usize, max: usize, segment: &str) -> bool {
    if segment.is_empty() {
        return true;
    }
    let mut chars = segment.chars();
    while *used < max {
        let Some(ch) = chars.next() else {
            return true;
        };
        out.push(ch);
        *used += 1;
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
        return false;
    }
    true
}

fn raw_compositor_footer_refresh_interval() -> u64 {
    std::env::var("KITTWM_FOOTER_REFRESH_FRAMES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn should_write_compositor_footer(
    last_key: &str,
    next_key: &str,
    frame: u64,
    refresh_interval: u64,
) -> bool {
    last_key != next_key || (refresh_interval > 0 && frame % refresh_interval == 0)
}

fn should_flush_compositor_frame(wrote_output: bool) -> bool {
    wrote_output
}

fn should_clear_raw_overlay_area(
    launcher_was_active: bool,
    launcher_active: bool,
    picker_was_active: bool,
    picker_active: bool,
    quit_confirm_was_active: bool,
    quit_confirm_active: bool,
) -> bool {
    (launcher_was_active && !launcher_active)
        || (picker_was_active && !picker_active)
        || (quit_confirm_was_active && !quit_confirm_active)
}

fn frame_sleep_chunks_for_budget(mut remaining: Duration) -> Vec<Duration> {
    let mut chunks = Vec::new();
    while remaining >= Duration::from_micros(500) {
        let chunk = frame_sleep_chunk(remaining);
        chunks.push(chunk);
        remaining = remaining.saturating_sub(chunk);
    }
    chunks
}

fn sleep_remaining_frame_budget(frame_start: Instant, frame_target: Duration) {
    let remaining = frame_target
        .checked_sub(frame_start.elapsed())
        .unwrap_or_default();
    sleep_frame_budget_or_input(remaining);
}

fn sleep_frame_budget_or_input(remaining: Duration) {
    for chunk in frame_sleep_chunks_for_budget(remaining) {
        if frame_sleep_should_stop_for_input(poll_stdin(chunk)) {
            break;
        }
    }
}

fn frame_sleep_should_stop_for_input(input_ready: bool) -> bool {
    input_ready
}

fn raw_frame_chrome_key(
    title: &str,
    focused: bool,
    mode: kittui_wm::compositor::WindowMode,
    fullscreen: bool,
    footprint: CellRect,
) -> String {
    let visible = raw_frame_chrome_text(title, focused, mode, fullscreen, footprint.cols);
    let mut key =
        String::with_capacity(visible.len() + "visible=;x=;y=;cols=;rows=".len() + 20 * 4);
    key.push_str("visible=");
    key.push_str(&visible);
    key.push_str(";x=");
    let _ = write!(key, "{}", footprint.x);
    key.push_str(";y=");
    let _ = write!(key, "{}", footprint.y);
    key.push_str(";cols=");
    let _ = write!(key, "{}", footprint.cols);
    key.push_str(";rows=");
    let _ = write!(key, "{}", footprint.rows);
    key
}

/// Append-only log for the kittui-wm session. Stderr is invisible inside
/// the alt screen, so we mirror everything to a file at $KITTUI_WM_LOG
/// (default `/tmp/kittui-wm.log`).
fn raw_frame_chrome_text(
    title: &str,
    focused: bool,
    mode: kittui_wm::compositor::WindowMode,
    fullscreen: bool,
    cols: u16,
) -> String {
    let width = cols as usize;
    let marker = if focused { "*" } else { " " };
    let mode = match mode {
        kittui_wm::compositor::WindowMode::Floating => "float",
        kittui_wm::compositor::WindowMode::Tiled => "tile",
    };
    let fullscreen = if fullscreen { " full" } else { "" };
    let mut out = String::with_capacity(width);
    let mut used = 0usize;
    for segment in [marker, " ", title, " ", mode, fullscreen] {
        for ch in segment.chars() {
            if used >= width {
                return out;
            }
            out.push(ch);
            used += 1;
        }
    }
    while used < width {
        out.push(' ');
        used += 1;
    }
    out
}

fn write_raw_frame_chrome<W: Write>(
    out: &mut W,
    frame: &kittui_wm::compositor::RawFrame,
) -> Result<()> {
    let clipped = raw_frame_chrome_text(
        &frame.title,
        frame.focused,
        frame.mode,
        frame.fullscreen,
        frame.footprint.cols,
    );
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
    clock_timestamp_line(now.as_secs(), now.subsec_millis())
}

fn clock_timestamp_line(secs: u64, millis: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(24);
    let _ = write!(out, "{secs}");
    out.push('.');
    let _ = write!(out, "{millis:03}");
    out
}

// --- raw mode + SGR mouse reporting guard ----------------------------------

struct RawMode;

fn raw_mode_enter_sequence() -> &'static [u8] {
    // Alt screen + hide cursor + clear/home, then SGR mouse + motion + focus reporting.
    b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h"
}

fn raw_mode_enter_sequence_clears_screen() -> bool {
    true
}

fn raw_mode_restore_sequence() -> &'static [u8] {
    // Disable in reverse-ish dependency order, restore cursor, then leave alt screen.
    b"\x1b[?1006l\x1b[?1004l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l"
}

#[cfg(unix)]
fn raw_mode_iflag(iflag: libc::tcflag_t) -> libc::tcflag_t {
    use libc::{BRKINT, ICRNL, IGNCR, INLCR, ISTRIP, IXOFF, IXON, PARMRK};
    // Preserve raw CR/LF bytes. If ICRNL/INLCR/IGNCR remain enabled, the
    // kernel can rewrite Enter to newline, rewrite newline to carriage return,
    // or drop carriage return before kittwm routes bytes to the native app.
    // Disable software flow control so Ctrl-S/Ctrl-Q pass through to native
    // apps. Disable byte-mangling flags so break/parity handling and high-bit
    // stripping do not corrupt Alt/UTF-8/control-sequence passthrough.
    iflag & !(ICRNL | INLCR | IGNCR | IXON | IXOFF | BRKINT | PARMRK | ISTRIP)
}

#[cfg(unix)]
fn raw_mode_oflag(oflag: libc::tcflag_t) -> libc::tcflag_t {
    use libc::OPOST;
    // Keep kittwm's rendered output byte-exact while it writes alt-screen
    // control sequences, cursor-addressed chrome, and kitty graphics payloads.
    oflag & !OPOST
}

#[cfg(unix)]
fn raw_mode_cflag(cflag: libc::tcflag_t) -> libc::tcflag_t {
    use libc::{CS8, CSIZE};
    // Force 8-bit characters so host tty character-size settings cannot strip
    // high-bit UTF-8/Alt bytes before kittwm routes them to native apps.
    (cflag & !CSIZE) | CS8
}

#[cfg(unix)]
fn raw_mode_lflag(lflag: libc::tcflag_t) -> libc::tcflag_t {
    use libc::{ECHO, ICANON, IEXTEN, ISIG};
    // Disable ISIG so Ctrl-C is delivered as byte 0x03 and can be handled by
    // kittwm's triple-press guard instead of the kernel sending SIGINT.
    // Disable IEXTEN so implementation-defined controls (for example Ctrl-V /
    // LNEXT) pass through to native terminal apps instead of the host line
    // discipline consuming them.
    lflag & !(ICANON | ECHO | ISIG | IEXTEN)
}

impl RawMode {
    fn enter() -> Result<Self> {
        let mut out = io::stdout();
        out.write_all(raw_mode_enter_sequence())?;
        out.flush()?;
        #[cfg(unix)]
        unsafe {
            use libc::*;
            let mut term: termios = std::mem::zeroed();
            tcgetattr(STDIN_FILENO, &mut term);
            let mut raw = term;
            raw.c_iflag = raw_mode_iflag(term.c_iflag);
            raw.c_oflag = raw_mode_oflag(term.c_oflag);
            raw.c_cflag = raw_mode_cflag(term.c_cflag);
            raw.c_lflag = raw_mode_lflag(term.c_lflag);
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
    let _ = out.write_all(raw_mode_restore_sequence());
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
    for cmd in path_commands(5000) {
        if launcher_match_score(&cmd, query).is_some() {
            return Some(LauncherSelection {
                kind: LauncherKind::Path,
                command: cmd,
            });
        }
    }
    #[cfg(target_os = "macos")]
    for app in macos_apps(5000) {
        if launcher_match_score(&app, query).is_some() {
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum QuitConfirmEvent {
    Consumed,
    Cancel,
    Confirm,
    NotHandled,
}

const QUIT_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Eq, PartialEq, Default)]
struct QuitConfirmOverlay {
    active: bool,
    opened_at: Option<Instant>,
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

const LAUNCHER_OVERLAY_KEY_QUERY_MAX_CHARS: usize = 96;

fn launcher_overlay_key(overlay: &LauncherOverlay) -> String {
    let query = bounded_ellipsis(&overlay.query, LAUNCHER_OVERLAY_KEY_QUERY_MAX_CHARS);
    let mut key = String::with_capacity("active=;query=;selected=".len() + 5 + query.len() + 20);
    key.push_str("active=");
    key.push_str(if overlay.active { "true" } else { "false" });
    key.push_str(";query=");
    key.push_str(&query);
    key.push_str(";selected=");
    let _ = write!(key, "{}", overlay.selected);
    key
}

fn launcher_candidate_row_text(
    row: usize,
    selected: usize,
    candidate: &LauncherSelection,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let marker = if row == selected { "▶" } else { " " };
    let prefix = launcher_candidate_row_prefix(marker, row + 1, candidate.kind_name());
    let mut out = String::with_capacity(width);
    let mut used = 0usize;
    if !push_truncated_cells(&mut out, &mut used, width, &prefix) {
        return out;
    }
    let _ = push_truncated_cells(&mut out, &mut used, width, &candidate.command);
    out
}

fn launcher_candidate_row_prefix(marker: &str, row: usize, kind: &str) -> String {
    let mut prefix = String::with_capacity(marker.len() + " 00. [     ] ".len() + kind.len());
    prefix.push_str(marker);
    prefix.push(' ');
    let _ = write!(prefix, "{row:>2}");
    prefix.push_str(". [");
    let _ = write!(prefix, "{kind:<5}");
    prefix.push_str("] ");
    prefix
}

fn picker_overlay_key(overlay: &PickerOverlay) -> String {
    let mut key = String::with_capacity("active=;selected=;entry_count=".len() + 5 + 20 + 20);
    key.push_str("active=");
    key.push_str(if overlay.active { "true" } else { "false" });
    key.push_str(";selected=");
    let _ = write!(key, "{}", overlay.selected);
    key.push_str(";entry_count=");
    let _ = write!(key, "{}", overlay.entries.len());
    key
}

fn picker_entry_row_text(row: usize, selected: usize, entry: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let marker = if row == selected { "▶ " } else { "  " };
    let mut out = String::with_capacity(width);
    let mut used = 0usize;
    if !push_truncated_cells(&mut out, &mut used, width, marker) {
        return out;
    }
    let _ = push_truncated_cells(&mut out, &mut used, width, entry);
    out
}

fn quit_confirm_overlay_key(overlay: &QuitConfirmOverlay) -> String {
    let mut key = String::with_capacity("active=;opened=".len() + 5 + 5);
    key.push_str("active=");
    key.push_str(if overlay.active { "true" } else { "false" });
    key.push_str(";opened=");
    key.push_str(if overlay.opened_at.is_some() {
        "true"
    } else {
        "false"
    });
    key
}

fn overlay_row_visible(row: u16, terminal_rows: u16) -> bool {
    row >= 1 && row <= terminal_rows
}

impl QuitConfirmOverlay {
    fn open(&mut self, now: Instant) {
        self.active = true;
        self.opened_at = Some(now);
    }

    fn close(&mut self) {
        self.active = false;
        self.opened_at = None;
    }

    fn expired(&self, now: Instant) -> bool {
        self.active
            && self
                .opened_at
                .and_then(|opened| now.checked_duration_since(opened))
                .is_some_and(|elapsed| elapsed > QUIT_CONFIRM_TIMEOUT)
    }

    fn handle_event(&mut self, ev: &InputEvent, now: Instant) -> QuitConfirmEvent {
        if self.expired(now) {
            return QuitConfirmEvent::Cancel;
        }
        match ev {
            InputEvent::Char { ch: 'y' | 'Y', .. } => QuitConfirmEvent::Confirm,
            InputEvent::Char { ch: 'n' | 'N', .. }
            | InputEvent::Char { ch: 'q' | 'Q', .. }
            | InputEvent::Key {
                key: Key::Escape, ..
            } => QuitConfirmEvent::Cancel,
            InputEvent::Char { ch: 'c', mods } if mods.ctrl && !mods.alt => {
                QuitConfirmEvent::Consumed
            }
            _ => QuitConfirmEvent::NotHandled,
        }
    }

    fn render<W: Write>(&self, handle: &mut W, terminal_rows: u16) -> Result<()> {
        let width = overlay_inner_width(64);
        if overlay_row_visible(2, terminal_rows) {
            write!(handle, "\x1b[2;2H┌{}┐", "─".repeat(width))?;
        }
        let title = truncate_cells("confirm quit kittwm", width);
        if overlay_row_visible(3, terminal_rows) {
            write!(handle, "\x1b[3;2H│{:^width$}│", title, width = width)?;
        }
        if overlay_row_visible(4, terminal_rows) {
            write!(handle, "\x1b[4;2H├{}┤", "─".repeat(width))?;
        }
        let prompt = truncate_cells("Triple Ctrl-C received. Quit the window manager?", width);
        if overlay_row_visible(5, terminal_rows) {
            write!(handle, "\x1b[5;2H│{:<width$}│", prompt, width = width)?;
        }
        let hint = truncate_cells("Press y to quit, n/Esc to cancel. Times out in 5s.", width);
        if overlay_row_visible(6, terminal_rows) {
            write!(handle, "\x1b[6;2H│{:<width$}│", hint, width = width)?;
        }
        if overlay_row_visible(7, terminal_rows) {
            write!(handle, "\x1b[7;2H└{}┘", "─".repeat(width))?;
        }
        Ok(())
    }
}

fn picker_mac_window_entry(owner_name: &str, title: &str) -> String {
    let mut entry = String::with_capacity("mac:  — ".len() + owner_name.len() + title.len());
    entry.push_str("mac: ");
    entry.push_str(owner_name);
    entry.push_str(" — ");
    entry.push_str(title);
    entry
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
                    .push(picker_mac_window_entry(&w.owner_name, &w.title));
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

    fn render<W: Write>(&self, handle: &mut W, terminal_rows: u16) -> Result<()> {
        let width = overlay_inner_width(64);
        if overlay_row_visible(2, terminal_rows) {
            write!(handle, "\x1b[2;2H┌{}┐", "─".repeat(width))?;
        }
        let title = truncate_cells("kittwm picker", width);
        if overlay_row_visible(3, terminal_rows) {
            write!(handle, "\x1b[3;2H│{:^width$}│", title, width = width)?;
        }
        if overlay_row_visible(4, terminal_rows) {
            write!(handle, "\x1b[4;2H├{}┤", "─".repeat(width))?;
        }
        for row in 0..8usize {
            let terminal_row = 5 + row as u16;
            if !overlay_row_visible(terminal_row, terminal_rows) {
                break;
            }
            let line = self
                .entries
                .get(row)
                .map(|entry| picker_entry_row_text(row, self.selected, entry, width))
                .unwrap_or_default();
            write!(
                handle,
                "\x1b[{};2H│{:<width$}│",
                terminal_row,
                line,
                width = width
            )?;
        }
        if overlay_row_visible(13, terminal_rows) {
            write!(handle, "\x1b[13;2H├{}┤", "─".repeat(width))?;
        }
        let hint = truncate_cells("Enter select · Esc close · ↑/↓/Tab navigate", width);
        if overlay_row_visible(14, terminal_rows) {
            write!(handle, "\x1b[14;2H│{:<width$}│", hint, width = width)?;
        }
        if overlay_row_visible(15, terminal_rows) {
            write!(handle, "\x1b[15;2H└{}┘", "─".repeat(width))?;
        }
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

    fn render<W: Write>(&self, handle: &mut W, terminal_rows: u16) -> Result<()> {
        let candidates = self.candidates();
        let width = overlay_inner_width(58);
        if overlay_row_visible(2, terminal_rows) {
            write!(handle, "\x1b[2;2H┌{}┐", "─".repeat(width))?;
        }
        let title = truncate_cells("kittwm launcher", width);
        if overlay_row_visible(3, terminal_rows) {
            write!(handle, "\x1b[3;2H│{:^width$}│", title, width = width)?;
        }
        if overlay_row_visible(4, terminal_rows) {
            write!(handle, "\x1b[4;2H├{}┤", "─".repeat(width))?;
        }
        let query_line = launcher_overlay_query_line(&self.query, width);
        if overlay_row_visible(5, terminal_rows) {
            write!(handle, "\x1b[5;2H│{:<width$}│", query_line, width = width)?;
        }
        if overlay_row_visible(6, terminal_rows) {
            write!(handle, "\x1b[6;2H├{}┤", "─".repeat(width))?;
        }
        for row in 0..8usize {
            let terminal_row = 7 + row as u16;
            if !overlay_row_visible(terminal_row, terminal_rows) {
                break;
            }
            let line = candidates
                .get(row)
                .map(|candidate| launcher_candidate_row_text(row, self.selected, candidate, width))
                .unwrap_or_default();
            write!(
                handle,
                "\x1b[{};2H│{:<width$}│",
                terminal_row,
                line,
                width = width
            )?;
        }
        if overlay_row_visible(15, terminal_rows) {
            write!(handle, "\x1b[15;2H├{}┤", "─".repeat(width))?;
        }
        let hint = truncate_cells("Enter launch · Esc close · type filter · ↑/↓ select", width);
        if overlay_row_visible(16, terminal_rows) {
            write!(handle, "\x1b[16;2H│{:<width$}│", hint, width = width)?;
        }
        if overlay_row_visible(17, terminal_rows) {
            write!(handle, "\x1b[17;2H└{}┘", "─".repeat(width))?;
        }
        Ok(())
    }
}

fn launcher_overlay_query_line(query: &str, width: usize) -> String {
    if width > 8 {
        let visible = truncate_cells(query, width - 8);
        let mut line = String::with_capacity(" query: ".len() + visible.len());
        line.push_str(" query: ");
        line.push_str(&visible);
        line
    } else {
        truncate_cells(query, width)
    }
}

#[cfg(test)]
fn graphical_overlay_panel_backdrop_label(id: &str, title: &str) -> String {
    let mut label = String::with_capacity(id.len() + "-backdrop:".len() + title.len());
    label.push_str(id);
    label.push_str("-backdrop:");
    label.push_str(title);
    label
}

#[cfg(test)]
fn graphical_overlay_panel_heading_label(id: &str) -> String {
    let mut label = String::with_capacity(id.len() + "-heading".len());
    label.push_str(id);
    label.push_str("-heading");
    label
}

#[cfg(test)]
fn graphical_overlay_panel_row_label(id: &str, idx: usize, row: &str) -> String {
    let mut label = String::with_capacity(id.len() + "-row-:".len() + 20 + row.len());
    label.push_str(id);
    label.push_str("-row-");
    let _ = write!(label, "{idx}");
    label.push(':');
    label.push_str(row);
    label
}

#[cfg(test)]
fn graphical_overlay_panel_footer_label(id: &str) -> String {
    let mut label = String::with_capacity(id.len() + "-footer-hints".len());
    label.push_str(id);
    label.push_str("-footer-hints");
    label
}

#[cfg(test)]
fn graphical_overlay_panel_scene(
    id: &str,
    title: &str,
    rows: &[String],
    selected: usize,
    cell_size: CellSize,
) -> Scene {
    let colors = native_glass_chrome_colors();
    let width_cells = 64u16;
    let height_cells = rows.len().min(8) as u16 + 5;
    let rect = CellRect::new(0, 0, width_cells, height_cells).to_pixels(cell_size);
    let row_h = cell_size.height_px.max(1) as f32;
    let mut layers = vec![
        Layer::new(
            graphical_overlay_panel_backdrop_label(id, title),
            Node::Rect {
                rect,
                fill: Paint::Solid { color: colors.fill },
                stroke: Some(Stroke::inside(
                    2.0,
                    Paint::Solid {
                        color: colors.border,
                    },
                )),
                corners: Corners::uniform(9.0),
            },
        ),
        Layer::new(
            graphical_overlay_panel_heading_label(id),
            Node::Rect {
                rect: PxRect::new(0.0, 0.0, rect.width, row_h * 1.6),
                fill: Paint::Solid {
                    color: colors.highlight,
                },
                stroke: None,
                corners: Corners::uniform(9.0),
            },
        ),
    ];
    for (idx, row) in rows.iter().take(8).enumerate() {
        let y = row_h * (idx as f32 + 2.0);
        let selected_row = idx == selected.min(rows.len().saturating_sub(1));
        layers.push(Layer::new(
            graphical_overlay_panel_row_label(id, idx, row),
            Node::Rect {
                rect: PxRect::new(
                    8.0,
                    y + 2.0,
                    (rect.width - 16.0).max(1.0),
                    (row_h - 4.0).max(6.0),
                ),
                fill: Paint::Solid {
                    color: rgba_with_alpha(colors.border, if selected_row { 96 } else { 34 }),
                },
                stroke: Some(Stroke::inside(
                    if selected_row { 2.0 } else { 1.0 },
                    Paint::Solid {
                        color: rgba_with_alpha(colors.border, if selected_row { 255 } else { 130 }),
                    },
                )),
                corners: Corners::uniform(5.0),
            },
        ));
    }
    layers.push(Layer::new(
        graphical_overlay_panel_footer_label(id),
        Node::Rect {
            rect: PxRect::new(
                8.0,
                rect.height - row_h * 1.4,
                (rect.width - 16.0).max(1.0),
                1.0,
            ),
            fill: Paint::Solid {
                color: colors.highlight,
            },
            stroke: None,
            corners: Corners::default(),
        },
    ));
    Scene {
        footprint: CellRect::new(0, 0, width_cells, height_cells),
        cell_size,
        layers,
        animation: None,
    }
}

#[cfg(test)]
fn graphical_launcher_overlay_title(query: &str) -> String {
    let mut title = String::with_capacity("kittwm launcher query=".len() + query.len());
    title.push_str("kittwm launcher query=");
    title.push_str(query);
    title
}

#[cfg(test)]
fn graphical_launcher_overlay_candidate_row(idx: usize, candidate: &LauncherSelection) -> String {
    let kind = candidate.kind_name();
    let mut row = String::with_capacity(20 + ". [] ".len() + kind.len() + candidate.command.len());
    let _ = write!(row, "{}", idx + 1);
    row.push_str(". [");
    row.push_str(kind);
    row.push_str("] ");
    row.push_str(&candidate.command);
    row
}

#[cfg(test)]
fn launcher_overlay_scene_for_candidates(
    overlay: &LauncherOverlay,
    candidates: &[LauncherSelection],
    cell_size: CellSize,
) -> Scene {
    let rows = candidates
        .iter()
        .enumerate()
        .map(|(idx, candidate)| graphical_launcher_overlay_candidate_row(idx, candidate))
        .collect::<Vec<_>>();
    let title = graphical_launcher_overlay_title(&overlay.query);
    graphical_overlay_panel_scene(
        "launcher-overlay",
        &title,
        &rows,
        overlay.selected,
        cell_size,
    )
}

#[cfg(test)]
fn graphical_command_palette_title(query: &str) -> String {
    let mut title = String::with_capacity("kittwm command palette query=".len() + query.len());
    title.push_str("kittwm command palette query=");
    title.push_str(query);
    title
}

#[cfg(test)]
fn command_palette_scene(query: &str, selected: usize, cell_size: CellSize) -> Scene {
    let actions = [
        "terminal: spawn a new shell",
        "split-columns: split focused pane vertically",
        "split-rows: split focused pane horizontally",
        "focus-next: move focus to next pane",
        "layout-columns: arrange panes as columns",
        "help: open shortcut overlay",
        "examples: show daily-driver examples",
        "apps: open app launcher",
    ];
    let query_lower = query.to_ascii_lowercase();
    let rows = actions
        .iter()
        .filter(|action| query_lower.is_empty() || action.contains(query_lower.as_str()))
        .map(|action| (*action).to_string())
        .collect::<Vec<_>>();
    let title = graphical_command_palette_title(query);
    graphical_overlay_panel_scene("command-palette", &title, &rows, selected, cell_size)
}

#[cfg(test)]
fn picker_overlay_scene(overlay: &PickerOverlay, cell_size: CellSize) -> Scene {
    graphical_overlay_panel_scene(
        "picker-overlay",
        "kittwm picker",
        &overlay.entries,
        overlay.selected,
        cell_size,
    )
}

fn raw_overlay_clear_end_row(terminal_rows: u16) -> Option<u16> {
    (terminal_rows >= 2).then_some(17.min(terminal_rows))
}

fn clear_launcher_overlay_area<W: Write>(handle: &mut W) -> Result<()> {
    // LauncherOverlay::render currently owns rows 2..=17 and starts at
    // column 2. Clear whole rows so stale box-drawing glyphs cannot remain
    // when the overlay closes after launch/Esc, but do not write below the
    // visible terminal and accidentally scroll short displays.
    let terminal_rows = host_terminal_cells().map(|(_, rows)| rows).unwrap_or(24);
    if let Some(end_row) = raw_overlay_clear_end_row(terminal_rows) {
        for row in 2..=end_row {
            write!(handle, "\x1b[0m\x1b[{};1H\x1b[K", row)?;
        }
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
    let mut scored: Vec<(u8, String)> = items
        .into_iter()
        .filter_map(|item| launcher_match_score(&item, query).map(|score| (score, item)))
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| a_score.cmp(b_score).then_with(|| a.cmp(b)));
    scored
        .into_iter()
        .map(|(_, item)| item)
        .take(limit)
        .collect()
}

fn launcher_match_score(item: &str, query: &str) -> Option<u8> {
    if ascii_casefold_eq(item, query) {
        Some(0)
    } else if ascii_casefold_starts_with(item, query) {
        Some(1)
    } else if ascii_casefold_contains(item, query) {
        Some(2)
    } else {
        None
    }
}

fn ascii_casefold_eq(item: &str, query: &str) -> bool {
    item.len() == query.len() && ascii_casefold_starts_with(item, query)
}

fn ascii_casefold_starts_with(item: &str, query: &str) -> bool {
    let item = item.as_bytes();
    let query = query.as_bytes();
    item.len() >= query.len()
        && item
            .iter()
            .zip(query.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn ascii_casefold_contains(item: &str, query: &str) -> bool {
    let item = item.as_bytes();
    let query = query.as_bytes();
    if query.is_empty() {
        return true;
    }
    item.len() >= query.len()
        && item
            .windows(query.len())
            .any(|window| window.eq_ignore_ascii_case(query))
}

fn overlay_inner_width(preferred: usize) -> usize {
    let env_cols = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|cols| *cols > 0);
    let detected_cols = host_terminal_cells().map(|(cols, _)| cols as usize);
    overlay_inner_width_for_sources(preferred, env_cols, detected_cols)
}

fn overlay_inner_width_for_sources(
    preferred: usize,
    env_cols: Option<usize>,
    detected_cols: Option<usize>,
) -> usize {
    overlay_inner_width_for_cols(
        preferred,
        env_cols
            .filter(|cols| *cols > 0)
            .or_else(|| detected_cols.filter(|cols| *cols > 0)),
    )
}

fn overlay_inner_width_for_cols(preferred: usize, terminal_cols: Option<usize>) -> usize {
    terminal_cols
        .map(|cols| preferred.min(cols.saturating_sub(3).max(1)))
        .unwrap_or(preferred)
        .max(1)
}

fn truncate_cells(s: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let mut chars = s.chars();
    let mut out = String::with_capacity(n.min(s.len()));
    for _ in 0..n {
        let Some(ch) = chars.next() else {
            return out;
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
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
    use super::{
        CtrlCGuard, QuitConfirmEvent, QuitConfirmOverlay, CTRL_C_TRIGGER, CTRL_C_WINDOW,
        QUIT_CONFIRM_TIMEOUT,
    };
    use kittui_input::{InputEvent, Key, Modifiers};
    use std::time::{Duration, Instant};

    #[test]
    fn single_press_does_not_trigger() {
        let mut g = CtrlCGuard::new();
        let now = Instant::now();
        assert_eq!(g.record_press(now), 1);
        assert!(1 < CTRL_C_TRIGGER);
    }

    #[test]
    fn three_presses_within_window_open_confirmation_but_can_be_cleared() {
        let mut g = CtrlCGuard::new();
        let t0 = Instant::now();
        assert_eq!(g.record_press(t0), 1);
        assert_eq!(g.record_press(t0 + Duration::from_millis(200)), 2);
        assert_eq!(
            g.record_press(t0 + Duration::from_millis(400)),
            CTRL_C_TRIGGER
        );
        g.clear();
        assert_eq!(g.record_press(t0 + Duration::from_millis(500)), 1);
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
        assert_eq!(g.quit_hint(true), "q or Ctrl-C×3 then y to quit");
    }

    #[test]
    fn quit_confirmation_requires_explicit_yes_before_timeout() {
        let t0 = Instant::now();
        let mut overlay = QuitConfirmOverlay::default();
        overlay.open(t0);
        assert!(overlay.active);
        assert!(!overlay.expired(t0 + Duration::from_secs(1)));
        assert_eq!(
            overlay.handle_event(
                &InputEvent::Char {
                    ch: 'c',
                    mods: Modifiers {
                        ctrl: true,
                        ..Modifiers::default()
                    }
                },
                t0
            ),
            QuitConfirmEvent::Consumed
        );
        assert_eq!(
            overlay.handle_event(
                &InputEvent::Key {
                    key: Key::Escape,
                    mods: Modifiers::default()
                },
                t0
            ),
            QuitConfirmEvent::Cancel
        );
        assert_eq!(
            overlay.handle_event(
                &InputEvent::Char {
                    ch: 'y',
                    mods: Modifiers::default()
                },
                t0
            ),
            QuitConfirmEvent::Confirm
        );
        assert!(overlay.expired(t0 + QUIT_CONFIRM_TIMEOUT + Duration::from_millis(1)));
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
            dbg.log(&runtime_keymap_load_failed_log_line(&e));
            crate::keymap::default_keymap()
        }
    }
}

fn load_runtime_keymap_result(dbg: &Debugger) -> Result<Keymap> {
    if let Ok(path) = std::env::var("KITTUI_WM_KEYMAP") {
        let km = Keymap::load(std::path::Path::new(&path))?;
        dbg.log(&loaded_keymap_log_line(&path));
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
                Key::F(n) => function_key_spec_label(*n),
            },
        }),
        _ => None,
    }
}

fn function_key_spec_label(n: u8) -> String {
    let mut label = String::with_capacity(3);
    label.push('f');
    let _ = write!(label, "{n}");
    label
}

#[cfg(test)]
mod runtime_keymap_tests {
    use super::*;
    use kittui_input::Modifiers;

    #[test]
    fn function_key_spec_label_builds_directly() {
        let label = function_key_spec_label(12);
        assert_eq!(label, "f12");
        assert!(label.capacity() >= label.len());
    }

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
        let f12 = key_spec_for_event(&InputEvent::Key {
            key: Key::F(12),
            mods: Modifiers::default(),
        })
        .unwrap();
        assert_eq!(f12.to_string(), "f12");
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
                workspace_state_action_label("workspace.new", &self.label())
            }
            Action::WorkspaceNext => {
                self.current = (self.current + 1) % self.count;
                workspace_state_action_label("workspace.next", &self.label())
            }
            Action::WorkspacePrev => {
                self.current = (self.current + self.count - 1) % self.count;
                workspace_state_action_label("workspace.prev", &self.label())
            }
            Action::WorkspaceSwitch(target) => {
                let target = (*target).max(1);
                if target > self.count {
                    self.count = target;
                }
                self.current = target - 1;
                workspace_state_switch_action_label(target, &self.label())
            }
            other => workspace_state_ignored_action_label(other),
        }
    }

    fn label(&self) -> String {
        let current = self.current + 1;
        let mut label = String::with_capacity(40);
        let _ = write!(label, "{current}");
        label.push('/');
        let _ = write!(label, "{}", self.count);
        label
    }

    fn active_label(&self) -> String {
        (self.current + 1).to_string()
    }
}

fn workspace_state_action_label(action: &str, label: &str) -> String {
    let mut out = String::with_capacity(action.len() + " -> ".len() + label.len());
    out.push_str(action);
    out.push_str(" -> ");
    out.push_str(label);
    out
}

fn workspace_state_switch_action_label(target: usize, label: &str) -> String {
    let mut out = String::with_capacity("workspace.switch. -> ".len() + 20 + label.len());
    out.push_str("workspace.switch.");
    let _ = write!(out, "{target}");
    out.push_str(" -> ");
    out.push_str(label);
    out
}

fn workspace_state_ignored_action_label(action: &Action) -> String {
    let mut out = String::with_capacity("workspace ignored action ".len() + 32);
    out.push_str("workspace ignored action ");
    let _ = write!(out, "{action}");
    out
}

fn publish_workspace_label_for_status(label: &str) {
    std::env::set_var("KITTWM_WORKSPACE", label.trim());
}

#[cfg(test)]
mod workspace_state_tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn workspace_state_create_and_cycle() {
        let mut ws = WorkspaceState::default();
        assert_eq!(ws.label(), "1/1");
        assert_eq!(ws.active_label(), "1");
        assert_eq!(ws.apply(&Action::WorkspaceNew), "workspace.new -> 2/2");
        assert_eq!(ws.apply(&Action::WorkspaceNew), "workspace.new -> 3/3");
        assert_eq!(ws.apply(&Action::WorkspaceNext), "workspace.next -> 1/3");
        assert_eq!(ws.apply(&Action::WorkspacePrev), "workspace.prev -> 3/3");
        assert_eq!(
            ws.apply(&Action::WorkspaceSwitch(7)),
            "workspace.switch.7 -> 7/7"
        );
        assert_eq!(ws.active_label(), "7");
        let _guard = ENV_LOCK.lock().unwrap();
        publish_workspace_label_for_status(&ws.active_label());
        assert_eq!(std::env::var("KITTWM_WORKSPACE").as_deref(), Ok("7"));
        std::env::remove_var("KITTWM_WORKSPACE");
        assert_eq!(
            ws.apply(&Action::WorkspaceSwitch(2)),
            "workspace.switch.2 -> 2/7"
        );
        assert_eq!(
            ws.apply(&Action::WorkspaceSwitch(0)),
            "workspace.switch.1 -> 1/7"
        );
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
        focus_state_action_label(self.last_direction, &self.label())
    }

    fn label(&self) -> String {
        let mut label = String::with_capacity(self.last_direction.len() + 1 + 20);
        label.push_str(self.last_direction);
        label.push('#');
        let _ = write!(label, "{}", self.moves);
        label
    }
}

fn focus_state_action_label(direction: &str, label: &str) -> String {
    let mut out = String::with_capacity("focus. -> ".len() + direction.len() + label.len());
    out.push_str("focus.");
    out.push_str(direction);
    out.push_str(" -> ");
    out.push_str(label);
    out
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
        split_state_action_label(self.last_orientation, &self.label())
    }

    fn label(&self) -> String {
        let mut label = String::with_capacity(20 + 1 + self.last_orientation.len());
        let _ = write!(label, "{}", self.panes);
        label.push(':');
        label.push_str(self.last_orientation);
        label
    }
}

fn split_state_action_label(orientation: &str, label: &str) -> String {
    let mut action =
        String::with_capacity("split..launcher -> ".len() + orientation.len() + label.len());
    action.push_str("split.");
    action.push_str(orientation);
    action.push_str(".launcher -> ");
    action.push_str(label);
    action
}

#[cfg(test)]
mod split_state_tests {
    use super::*;

    #[test]
    fn split_state_action_label_builds_directly() {
        let action = split_state_action_label("vertical", "2:vertical");
        assert_eq!(action, "split.vertical.launcher -> 2:vertical");
        assert!(action.capacity() >= action.len());
    }

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
        swap_state_action_label(self.last_direction, &self.label())
    }

    fn label(&self) -> String {
        let mut label = String::with_capacity(self.last_direction.len() + 1 + 20);
        label.push_str(self.last_direction);
        label.push('#');
        let _ = write!(label, "{}", self.swaps);
        label
    }
}

fn swap_state_action_label(direction: &str, label: &str) -> String {
    let mut out = String::with_capacity("swap. -> ".len() + direction.len() + label.len());
    out.push_str("swap.");
    out.push_str(direction);
    out.push_str(" -> ");
    out.push_str(label);
    out
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

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    fn picker_entry_row_text_builds_only_visible_prefix() {
        let row = picker_entry_row_text(
            0,
            0,
            &"window-title-with-pathological-length-".repeat(10_000),
            24,
        );
        assert_eq!(row.chars().count(), 24, "{row:?}");
        assert!(row.starts_with("▶ window-title-with-pa"), "{row:?}");
        assert!(row.ends_with('…'), "{row:?}");
        assert!(row.capacity() >= 24);
        assert!(!row.contains(&"window-title-with-pathological-length-".repeat(2)));
        assert_eq!(picker_entry_row_text(0, 0, "anything", 1), "…");
        assert_eq!(picker_entry_row_text(0, 0, "anything", 0), "");
    }

    #[test]
    fn launcher_overlay_query_line_builds_directly() {
        assert_eq!(launcher_overlay_query_line("term", 20), " query: term");
        assert_eq!(launcher_overlay_query_line("abcdef", 4), "abc…");
        let long = launcher_overlay_query_line(&"x".repeat(10_000), 12);
        assert_eq!(long.chars().count(), 12);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn launcher_candidate_row_prefix_builds_directly() {
        let prefix = launcher_candidate_row_prefix("▶", 3, "path");
        assert_eq!(prefix, "▶  3. [path ] ");
        assert!(prefix.capacity() >= prefix.len());
    }

    #[test]
    fn launcher_candidate_row_text_builds_only_visible_prefix() {
        let candidate = LauncherSelection {
            kind: LauncherKind::Shell,
            command: "command-with-pathological-length-".repeat(10_000),
        };
        let row = launcher_candidate_row_text(0, 0, &candidate, 24);
        assert_eq!(row.chars().count(), 24, "{row:?}");
        assert!(row.starts_with("▶  1. [shell] command"), "{row:?}");
        assert!(row.ends_with('…'), "{row:?}");
        assert!(row.capacity() >= 24);
        assert!(!row.contains(&"command-with-pathological-length-".repeat(2)));
        assert_eq!(launcher_candidate_row_text(0, 0, &candidate, 1), "…");
        assert_eq!(launcher_candidate_row_text(0, 0, &candidate, 0), "");
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

    #[test]
    fn launcher_match_score_avoids_candidate_and_query_lowercase_allocation() {
        let huge = launcher_match_test_wrapped_needle(10_000);
        let huge_query = launcher_match_test_prefixed_missing_query(10_000);
        assert_eq!(launcher_match_score("Needle", "needle"), Some(0));
        assert_eq!(launcher_match_score("NeedleSuffix", "needle"), Some(1));
        assert_eq!(launcher_match_score(&huge, "needle"), Some(2));
        assert_eq!(launcher_match_score(&huge, "NEEDLE"), Some(2));
        assert_eq!(launcher_match_score(&huge, "missing"), None);
        assert_eq!(launcher_match_score("short", &huge_query), None);
        assert!(ascii_casefold_contains("RésuméNeedle", "needle"));
    }

    #[test]
    fn launcher_match_test_stress_strings_build_directly() {
        let huge = launcher_match_test_wrapped_needle(3);
        assert_eq!(huge, "xxxNeedleyyy");
        assert_eq!(huge.capacity(), huge.len());

        let query = launcher_match_test_prefixed_missing_query(3);
        assert_eq!(query, "qqqmissing");
        assert_eq!(query.capacity(), query.len());
    }

    fn launcher_match_test_wrapped_needle(count: usize) -> String {
        let mut out = String::with_capacity(count + "Needle".len() + count);
        out.extend(std::iter::repeat_n('x', count));
        out.push_str("Needle");
        out.extend(std::iter::repeat_n('y', count));
        out
    }

    fn launcher_match_test_prefixed_missing_query(count: usize) -> String {
        let mut out = String::with_capacity(count + "missing".len());
        out.extend(std::iter::repeat_n('q', count));
        out.push_str("missing");
        out
    }

    fn launcher_path_test_temp_dir_name(pid: u32) -> String {
        let mut name = String::with_capacity("kittwm-launcher-path-".len() + 20);
        name.push_str("kittwm-launcher-path-");
        let _ = write!(name, "{pid}");
        name
    }

    #[test]
    fn launcher_path_test_temp_dir_name_builds_directly() {
        let name = launcher_path_test_temp_dir_name(1234);
        assert_eq!(name, "kittwm-launcher-path-1234");
        assert!(name.capacity() >= name.len());
    }

    #[test]
    fn first_launcher_candidate_matches_path_case_insensitively_without_candidate_lowercase() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(launcher_path_test_temp_dir_name(std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cmd = dir.join("NeedleTool");
        std::fs::write(&cmd, b"#!/bin/sh\n").unwrap();
        let old_path = std::env::var_os("PATH");
        std::env::set_var("PATH", &dir);
        let selection = first_launcher_candidate("needle").unwrap();
        assert_eq!(selection.kind, LauncherKind::Path);
        assert!(selection.command.starts_with("Needle"));
        if let Some(old_path) = old_path {
            std::env::set_var("PATH", old_path);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_file(cmd);
        let _ = std::fs::remove_dir(dir);
    }
}

/// Triple-Ctrl-C quit guard with decay window. (bd-2776ad)
///
/// Single Ctrl-C is forwarded to the focused window; three Ctrl-C presses
/// within `CTRL_C_WINDOW` open a confirmation dialog. Presses older than
/// the window are discarded so a slow typist won't accidentally reach the
/// confirmation path.
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

    fn clear(&mut self) {
        self.presses.clear();
    }

    /// Footer hint for the operator. Switches the visible quit message
    /// to mention the Ctrl-C kill switch whenever the WM is actually
    /// hosting an app that might swallow `q` / Esc.
    fn quit_hint(&self, hosting_app: bool) -> &'static str {
        if hosting_app {
            "q or Ctrl-C×3 then y to quit"
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
                toggle_state_action_label("fullscreen.toggle", &self.label())
            }
            Action::FloatToggle => {
                self.floating = !self.floating;
                toggle_state_action_label("float.toggle", &self.label())
            }
            other => toggle_state_ignored_action_label(other),
        }
    }

    fn label(&self) -> String {
        let mut label = String::with_capacity("full=false float=false".len());
        label.push_str("full=");
        label.push_str(if self.fullscreen { "true" } else { "false" });
        label.push_str(" float=");
        label.push_str(if self.floating { "true" } else { "false" });
        label
    }
}

fn toggle_state_action_label(action: &str, label: &str) -> String {
    let mut out = String::with_capacity(action.len() + " -> ".len() + label.len());
    out.push_str(action);
    out.push_str(" -> ");
    out.push_str(label);
    out
}

fn toggle_state_ignored_action_label(action: &Action) -> String {
    let mut out = String::with_capacity("toggle ignored action ".len() + 32);
    out.push_str("toggle ignored action ");
    let _ = write!(out, "{action}");
    out
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
                layout_state_action_label("toggle.split", &self.label())
            }
            Action::BalanceWindows => {
                self.balances += 1;
                layout_state_action_label("balance.windows", &self.label())
            }
            other => layout_state_ignored_action_label(other),
        }
    }

    fn label(&self) -> String {
        let mut label = String::with_capacity("axis= balanced#".len() + self.split_axis.len() + 20);
        label.push_str("axis=");
        label.push_str(self.split_axis);
        label.push_str(" balanced#");
        let _ = write!(label, "{}", self.balances);
        label
    }
}

fn layout_state_action_label(action: &str, label: &str) -> String {
    let mut out = String::with_capacity(action.len() + " -> ".len() + label.len());
    out.push_str(action);
    out.push_str(" -> ");
    out.push_str(label);
    out
}

fn layout_state_ignored_action_label(action: &Action) -> String {
    let mut out = String::with_capacity("layout ignored action ".len() + 32);
    out.push_str("layout ignored action ");
    let _ = write!(out, "{action}");
    out
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
        config_state_action_label("reload.config", &self.label())
    }

    fn reload_err(&mut self, err: &str) -> String {
        self.reloads += 1;
        self.last_error = Some(err.to_string());
        config_state_action_label("reload.config error", &self.label())
    }

    fn label(&self) -> String {
        let mut label = String::with_capacity("reload#".len() + 20 + ":err".len());
        label.push_str("reload#");
        let _ = write!(label, "{}", self.reloads);
        if self.last_error.is_some() {
            label.push_str(":err");
        }
        label
    }
}

fn config_state_action_label(action: &str, label: &str) -> String {
    let mut out = String::with_capacity(action.len() + " -> ".len() + label.len());
    out.push_str(action);
    out.push_str(" -> ");
    out.push_str(label);
    out
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
