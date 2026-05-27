//! `kittwm-browser` — first-class kittwm-native browser app.
//!
//! This binary is intentionally WM-context aware: when launched inside a
//! kittwm native terminal it inherits `KITTWM_SOCKET` / `KITTWM_WINDOW`, just
//! like an X app inherits `DISPLAY`. The current implementation is a direct
//! full-terminal replacement app; the socket spawn/replace protocol will make
//! the same binary ask a live kittwm host to create or replace panes.

use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
#[cfg(test)]
use kittui::Transport;
use kittui::{CellRect, CellSize, Runtime, TerminalInfo};
use kittui_kitty as kitty;
use kittui_wm::native::{
    BrowserArrowKey, BrowserPageKey, HeadlessBrowserApp, NativeApp, NativeFrame,
};
use kittui_wm::semantic::render_sdk_semantic_surface;
use kittwm_sdk::{Kittwm, SemanticSurfaceSnapshot};

const DEFAULT_URL: &str =
    "data:text/html,<html><body><h1>kittwm-browser</h1><input autofocus value='ready'></body></html>";
const BROWSER_RESERVED_STATUS_ROWS: u16 = 2;
const BROWSER_IMAGE_ID: u32 = 1;
const BROWSER_IMAGE_Z_INDEX: i32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserArgs {
    url: String,
    semantic_snapshot: bool,
    semantic_scene_json: bool,
    semantic_kitty: bool,
    capabilities: bool,
    capabilities_json: bool,
    compact_json: bool,
    help: bool,
}

impl BrowserArgs {
    fn parse_from<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut url = None;
        let mut semantic_snapshot = false;
        let mut semantic_scene_json = false;
        let mut semantic_kitty = false;
        let mut capabilities = false;
        let mut capabilities_json = false;
        let mut compact_json = true;
        let mut help = false;
        let mut iter = args.into_iter().map(Into::into);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => help = true,
                "--semantic-snapshot" | "--print-semantic" => semantic_snapshot = true,
                "--semantic-scene-json" => semantic_scene_json = true,
                "--semantic-kitty" | "--semantic-graphics" => semantic_kitty = true,
                "--capabilities" | "--native-capabilities" => capabilities = true,
                "--capabilities-json" | "--native-capabilities-json" => capabilities_json = true,
                "--pretty" | "--pretty-json" => compact_json = false,
                "--compact" | "--compact-json" => compact_json = true,
                "--" => {
                    if let Some(value) = iter.next() {
                        url = Some(value);
                    }
                    if let Some(extra) = iter.next() {
                        return Err(format!(
                            "unexpected extra argument {extra}\n\n{}",
                            help_text()
                        ));
                    }
                    break;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other}\n\n{}", help_text()));
                }
                other => {
                    if url.replace(other.to_string()).is_some() {
                        return Err(format!(
                            "unexpected extra argument {other}\n\n{}",
                            help_text()
                        ));
                    }
                }
            }
        }
        Ok(Self {
            url: url.unwrap_or_else(|| DEFAULT_URL.to_string()),
            semantic_snapshot,
            semantic_scene_json,
            semantic_kitty,
            capabilities,
            capabilities_json,
            compact_json,
            help,
        })
    }
}

fn help_text() -> String {
    "kittwm-browser — first-party kittwm-native browser app\n\n\
Usage:\n  kittwm-browser [OPTIONS] [URL]\n\n\
Options:\n  --semantic-snapshot, --print-semantic  load URL, print DOM/ARIA semantic snapshot JSON, and exit\n  --semantic-scene-json                  load URL, render semantic snapshot as kittui scene JSON, and exit\n  --semantic-kitty, --semantic-graphics  load URL, render semantic snapshot as kitty graphics, and exit\n  --capabilities, --native-capabilities  print SDK/kittui native capability summary and exit\n  --capabilities-json                    print SDK/kittui native capability JSON and exit\n  --pretty, --pretty-json                pretty-print semantic snapshot JSON\n  --compact, --compact-json              compact semantic snapshot JSON (default)\n  -h, --help                             show this help\n\n\
Default mode renders the browser surface in the terminal and publishes semantic snapshots to kittwm when KITTWM_SOCKET is set.\n"
        .to_string()
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kittwm-browser: {e}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let args = BrowserArgs::parse_from(std::env::args().skip(1)).map_err(anyhow::Error::msg)?;
    if args.help {
        print!("{}", help_text());
        return Ok(());
    }
    if args.semantic_snapshot {
        return print_semantic_snapshot(&args.url, args.compact_json);
    }
    if args.semantic_scene_json {
        return print_semantic_scene_json(&args.url);
    }
    if args.semantic_kitty {
        return print_semantic_kitty(&args.url);
    }
    if args.capabilities {
        return print_capabilities();
    }
    if args.capabilities_json {
        return print_capabilities_json();
    }
    let url = args.url;
    let mut viewport = BrowserViewport::from_terminal_cells(terminal_cells().unwrap_or((80, 24)));
    let mut browser = HeadlessBrowserApp::launch(
        &url,
        u32::from(viewport.cols) * 8,
        u32::from(viewport.content_rows) * 16,
    )?;
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;
    let _guard = TtyGuard::enter()?;
    let mut semantic_publisher = BrowserSemanticPublisher::from_env();
    let mut placed = false;
    let mut last_frame_key: Option<(usize, u64)> = None;
    let mut frame = 0u64;
    let mut last_status: Option<(u16, String)> = None;
    let show_status_frame = browser_status_frame_counter_enabled();
    let status_metadata = BrowserStatusMetadata::from_env();
    let status_url = truncate(&url, 40);
    let active_interval = browser_active_frame_interval();
    let idle_interval = browser_idle_frame_interval(active_interval);
    let static_interval = browser_static_frame_interval(idle_interval);
    let mut consecutive_idle_frames = 0u16;
    let mut input_parser = BrowserInputParser::default();
    let mut stdin = std::io::stdin();
    loop {
        let start = Instant::now();
        let mut user_activity = false;
        let mut buf = [0u8; 1024];
        while stdin_ready(Duration::ZERO) {
            let n = stdin.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            if buf[..n].contains(&0x1d) {
                return Ok(());
            }
            for action in input_parser.actions(&buf[..n]) {
                match action {
                    BrowserInputAction::Text(text) => browser.send_text(&text)?,
                    BrowserInputAction::Backspace => browser.send_backspace()?,
                    BrowserInputAction::CtrlBackspace => browser.send_ctrl_backspace()?,
                    BrowserInputAction::Tab => browser.send_tab()?,
                    BrowserInputAction::ShiftTab => browser.send_shift_tab()?,
                    BrowserInputAction::Enter => browser.send_enter()?,
                    BrowserInputAction::ShiftEnter => browser.send_shift_enter()?,
                    BrowserInputAction::CtrlEnter => browser.send_ctrl_enter()?,
                    BrowserInputAction::Escape => browser.send_escape()?,
                    BrowserInputAction::Insert => browser.send_insert()?,
                    BrowserInputAction::Delete => browser.send_delete()?,
                    BrowserInputAction::ShiftInsert => browser.send_shift_insert()?,
                    BrowserInputAction::ShiftDelete => browser.send_shift_delete()?,
                    BrowserInputAction::CtrlInsert => browser.send_ctrl_insert()?,
                    BrowserInputAction::CtrlDelete => browser.send_ctrl_delete()?,
                    BrowserInputAction::AltInsert => browser.send_alt_insert()?,
                    BrowserInputAction::AltDelete => browser.send_alt_delete()?,
                    BrowserInputAction::Home => browser.send_home()?,
                    BrowserInputAction::End => browser.send_end()?,
                    BrowserInputAction::ShiftHome => browser.send_shift_home()?,
                    BrowserInputAction::ShiftEnd => browser.send_shift_end()?,
                    BrowserInputAction::CtrlHome => browser.send_ctrl_home()?,
                    BrowserInputAction::CtrlEnd => browser.send_ctrl_end()?,
                    BrowserInputAction::AltHome => browser.send_alt_home()?,
                    BrowserInputAction::AltEnd => browser.send_alt_end()?,
                    BrowserInputAction::Arrow(direction) => browser.send_arrow_key(direction)?,
                    BrowserInputAction::ShiftArrow(direction) => {
                        browser.send_shift_arrow_key(direction)?
                    }
                    BrowserInputAction::CtrlArrow(direction) => {
                        browser.send_ctrl_arrow_key(direction)?
                    }
                    BrowserInputAction::AltArrow(direction) => {
                        browser.send_alt_arrow_key(direction)?
                    }
                    BrowserInputAction::Page(direction) => browser.send_page_key(direction)?,
                    BrowserInputAction::ShiftPage(direction) => {
                        browser.send_shift_page_key(direction)?
                    }
                    BrowserInputAction::CtrlPage(direction) => {
                        browser.send_ctrl_page_key(direction)?
                    }
                    BrowserInputAction::AltPage(direction) => {
                        browser.send_alt_page_key(direction)?
                    }
                }
                user_activity = true;
            }
        }
        if let Some(cells) = terminal_cells() {
            let new_viewport = BrowserViewport::from_terminal_cells(cells);
            if new_viewport != viewport {
                viewport = new_viewport;
                browser.resize(viewport.cols, viewport.content_rows)?;
                placed = false;
                last_frame_key = None;
                user_activity = true;
            }
        }
        let NativeFrame::Png {
            width: _,
            height: _,
            bytes,
        } = browser.capture()?
        else {
            return Err(anyhow!("browser returned non-PNG frame"));
        };
        if user_activity {
            semantic_publisher.reset_after_activity();
        }
        semantic_publisher.maybe_publish(&mut browser);
        let fp = viewport.frame_footprint();
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        let mut wrote_output = false;
        let frame_key = browser_frame_key(&bytes);
        let upload_frame = should_upload_browser_frame(last_frame_key, frame_key);
        if should_build_browser_placement(upload_frame, placed) {
            let placement = runtime.place_png_frame_with_options(
                BROWSER_IMAGE_ID,
                &bytes,
                fp,
                &browser_image_placement_options(),
            );
            if upload_frame {
                handle.write_all(placement.upload.as_bytes())?;
                last_frame_key = Some(frame_key);
                wrote_output = true;
            }
            if !placed {
                handle.write_all(placement.placement.as_bytes())?;
                placed = true;
                wrote_output = true;
            }
        }
        let status = browser_status_text_for_cols_with_precomputed_url(
            &status_url,
            frame,
            show_status_frame,
            viewport.cols,
            &status_metadata,
        );
        if should_write_browser_status(last_status.as_ref(), viewport.status_row, &status) {
            if let Some((old_row, _)) = last_status.as_ref() {
                if *old_row != viewport.status_row {
                    write!(handle, "\x1b[0m\x1b[{};1H\x1b[K", old_row)?;
                }
            }
            write!(
                handle,
                "\x1b[0m\x1b[{};1H\x1b[K{}",
                viewport.status_row, status
            )?;
            last_status = Some((viewport.status_row, status));
            wrote_output = true;
        }
        if wrote_output {
            handle.flush()?;
        }
        update_browser_idle_counter(&mut consecutive_idle_frames, wrote_output || user_activity);
        frame += 1;
        let frame_interval = browser_current_frame_interval(
            active_interval,
            idle_interval,
            static_interval,
            consecutive_idle_frames,
        );
        if let Some(slack) = frame_interval.checked_sub(start.elapsed()) {
            sleep_browser_frame_or_input(slack);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BrowserInputAction {
    Text(String),
    Backspace,
    CtrlBackspace,
    Tab,
    ShiftTab,
    Enter,
    ShiftEnter,
    CtrlEnter,
    Escape,
    Insert,
    Delete,
    ShiftInsert,
    ShiftDelete,
    CtrlInsert,
    CtrlDelete,
    AltInsert,
    AltDelete,
    Home,
    End,
    ShiftHome,
    ShiftEnd,
    CtrlHome,
    CtrlEnd,
    AltHome,
    AltEnd,
    Arrow(BrowserArrowKey),
    ShiftPage(BrowserPageKey),
    CtrlPage(BrowserPageKey),
    AltPage(BrowserPageKey),
    ShiftArrow(BrowserArrowKey),
    CtrlArrow(BrowserArrowKey),
    AltArrow(BrowserArrowKey),
    Page(BrowserPageKey),
}

fn browser_csi_sequence_complete(bytes: &[u8]) -> bool {
    bytes.iter().any(|b| (0x40..=0x7e).contains(b))
}

fn browser_csi_input_action(bytes: &[u8]) -> (Option<BrowserInputAction>, usize) {
    if bytes.is_empty() {
        return (None, 0);
    }
    let consumed = bytes
        .iter()
        .position(|b| (0x40..=0x7e).contains(b))
        .unwrap_or(bytes.len().saturating_sub(1));
    let sequence = &bytes[..=consumed.min(bytes.len().saturating_sub(1))];
    let action = match sequence {
        [b'A'] => Some(BrowserInputAction::Arrow(BrowserArrowKey::Up)),
        [b'B'] => Some(BrowserInputAction::Arrow(BrowserArrowKey::Down)),
        [b'C'] => Some(BrowserInputAction::Arrow(BrowserArrowKey::Right)),
        [b'D'] => Some(BrowserInputAction::Arrow(BrowserArrowKey::Left)),
        [b'1', b';', b'2', b'A'] => Some(BrowserInputAction::ShiftArrow(BrowserArrowKey::Up)),
        [b'1', b';', b'2', b'B'] => Some(BrowserInputAction::ShiftArrow(BrowserArrowKey::Down)),
        [b'1', b';', b'2', b'C'] => Some(BrowserInputAction::ShiftArrow(BrowserArrowKey::Right)),
        [b'1', b';', b'2', b'D'] => Some(BrowserInputAction::ShiftArrow(BrowserArrowKey::Left)),
        [b'1', b';', b'5', b'A'] => Some(BrowserInputAction::CtrlArrow(BrowserArrowKey::Up)),
        [b'1', b';', b'5', b'B'] => Some(BrowserInputAction::CtrlArrow(BrowserArrowKey::Down)),
        [b'1', b';', b'5', b'C'] => Some(BrowserInputAction::CtrlArrow(BrowserArrowKey::Right)),
        [b'1', b';', b'5', b'D'] => Some(BrowserInputAction::CtrlArrow(BrowserArrowKey::Left)),
        [b'1', b';', b'3', b'A'] => Some(BrowserInputAction::AltArrow(BrowserArrowKey::Up)),
        [b'1', b';', b'3', b'B'] => Some(BrowserInputAction::AltArrow(BrowserArrowKey::Down)),
        [b'1', b';', b'3', b'C'] => Some(BrowserInputAction::AltArrow(BrowserArrowKey::Right)),
        [b'1', b';', b'3', b'D'] => Some(BrowserInputAction::AltArrow(BrowserArrowKey::Left)),
        [b'1', b';', b'2', b'H'] => Some(BrowserInputAction::ShiftHome),
        [b'1', b';', b'2', b'F'] => Some(BrowserInputAction::ShiftEnd),
        [b'1', b';', b'5', b'H'] => Some(BrowserInputAction::CtrlHome),
        [b'1', b';', b'5', b'F'] => Some(BrowserInputAction::CtrlEnd),
        [b'1', b';', b'3', b'H'] => Some(BrowserInputAction::AltHome),
        [b'1', b';', b'3', b'F'] => Some(BrowserInputAction::AltEnd),
        [b'Z'] => Some(BrowserInputAction::ShiftTab),
        [b'8', b';', b'5', b'u'] | [b'1', b'2', b'7', b';', b'5', b'u'] => {
            Some(BrowserInputAction::CtrlBackspace)
        }
        [b'1', b'3', b';', b'2', b'u'] => Some(BrowserInputAction::ShiftEnter),
        [b'1', b'3', b';', b'5', b'u'] => Some(BrowserInputAction::CtrlEnter),
        [b'H'] | [b'1', b'~'] | [b'7', b'~'] => Some(BrowserInputAction::Home),
        [b'F'] | [b'4', b'~'] | [b'8', b'~'] => Some(BrowserInputAction::End),
        [b'2', b'~'] => Some(BrowserInputAction::Insert),
        [b'3', b'~'] => Some(BrowserInputAction::Delete),
        [b'2', b';', b'2', b'~'] => Some(BrowserInputAction::ShiftInsert),
        [b'3', b';', b'2', b'~'] => Some(BrowserInputAction::ShiftDelete),
        [b'2', b';', b'5', b'~'] => Some(BrowserInputAction::CtrlInsert),
        [b'3', b';', b'5', b'~'] => Some(BrowserInputAction::CtrlDelete),
        [b'2', b';', b'3', b'~'] => Some(BrowserInputAction::AltInsert),
        [b'3', b';', b'3', b'~'] => Some(BrowserInputAction::AltDelete),
        [b'5', b';', b'2', b'~'] => Some(BrowserInputAction::ShiftPage(BrowserPageKey::Up)),
        [b'6', b';', b'2', b'~'] => Some(BrowserInputAction::ShiftPage(BrowserPageKey::Down)),
        [b'5', b';', b'5', b'~'] => Some(BrowserInputAction::CtrlPage(BrowserPageKey::Up)),
        [b'6', b';', b'5', b'~'] => Some(BrowserInputAction::CtrlPage(BrowserPageKey::Down)),
        [b'5', b';', b'3', b'~'] => Some(BrowserInputAction::AltPage(BrowserPageKey::Up)),
        [b'6', b';', b'3', b'~'] => Some(BrowserInputAction::AltPage(BrowserPageKey::Down)),
        [b'5', b'~'] => Some(BrowserInputAction::Page(BrowserPageKey::Up)),
        [b'6', b'~'] => Some(BrowserInputAction::Page(BrowserPageKey::Down)),
        _ => None,
    };
    (action, consumed)
}

fn browser_ss3_input_action(byte: u8) -> Option<BrowserInputAction> {
    match byte {
        b'A' => Some(BrowserInputAction::Arrow(BrowserArrowKey::Up)),
        b'B' => Some(BrowserInputAction::Arrow(BrowserArrowKey::Down)),
        b'C' => Some(BrowserInputAction::Arrow(BrowserArrowKey::Right)),
        b'D' => Some(BrowserInputAction::Arrow(BrowserArrowKey::Left)),
        b'H' => Some(BrowserInputAction::Home),
        b'F' => Some(BrowserInputAction::End),
        _ => None,
    }
}

#[derive(Default)]
struct BrowserInputParser {
    pending_text: Vec<u8>,
    pending_control: Vec<u8>,
}

impl BrowserInputParser {
    fn actions(&mut self, bytes: &[u8]) -> Vec<BrowserInputAction> {
        self.actions_inner(bytes, true)
    }

    #[cfg(test)]
    fn actions_final(&mut self, bytes: &[u8]) -> Vec<BrowserInputAction> {
        self.actions_inner(bytes, false)
    }

    fn actions_inner(
        &mut self,
        bytes: &[u8],
        preserve_incomplete: bool,
    ) -> Vec<BrowserInputAction> {
        let mut actions = Vec::new();
        let mut input;
        let bytes = if self.pending_control.is_empty() {
            bytes
        } else {
            input = std::mem::take(&mut self.pending_control);
            input.extend_from_slice(bytes);
            input.as_slice()
        };
        let mut idx = 0usize;
        while idx < bytes.len() {
            match bytes[idx] {
                b'\r' | b'\n' => {
                    flush_browser_text_action(&mut actions, &mut self.pending_text, false);
                    actions.push(BrowserInputAction::Enter);
                }
                0x08 | 0x7f => {
                    flush_browser_text_action(&mut actions, &mut self.pending_text, false);
                    actions.push(BrowserInputAction::Backspace);
                }
                b'\t' => {
                    flush_browser_text_action(&mut actions, &mut self.pending_text, false);
                    actions.push(BrowserInputAction::Tab);
                }
                0x1b if idx + 1 < bytes.len() && bytes[idx + 1] == b'[' => {
                    if preserve_incomplete && !browser_csi_sequence_complete(&bytes[idx + 2..]) {
                        self.pending_control.extend_from_slice(&bytes[idx..]);
                        break;
                    }
                    let (action, consumed) = browser_csi_input_action(&bytes[idx + 2..]);
                    if let Some(action) = action {
                        flush_browser_text_action(&mut actions, &mut self.pending_text, false);
                        actions.push(action);
                    }
                    idx += consumed + 2;
                }
                0x1b if idx + 1 < bytes.len() && bytes[idx + 1] == b'O' => {
                    if idx + 2 < bytes.len() {
                        if let Some(action) = browser_ss3_input_action(bytes[idx + 2]) {
                            flush_browser_text_action(&mut actions, &mut self.pending_text, false);
                            actions.push(action);
                        }
                        idx += 2;
                    } else if preserve_incomplete {
                        self.pending_control.extend_from_slice(&bytes[idx..]);
                        break;
                    } else {
                        idx += 1;
                    }
                }
                0x1b => {
                    flush_browser_text_action(&mut actions, &mut self.pending_text, false);
                    actions.push(BrowserInputAction::Escape);
                }
                0x20..=0x7e | 0x80..=0xff => self.pending_text.push(bytes[idx]),
                _ => {}
            }
            idx += 1;
        }
        flush_browser_text_action(&mut actions, &mut self.pending_text, preserve_incomplete);
        actions
    }
}

#[cfg(test)]
fn browser_input_actions(bytes: &[u8]) -> Vec<BrowserInputAction> {
    BrowserInputParser::default().actions_final(bytes)
}

fn flush_browser_text_action(
    actions: &mut Vec<BrowserInputAction>,
    text: &mut Vec<u8>,
    preserve_incomplete: bool,
) {
    if text.is_empty() {
        return;
    }
    let (decoded, pending) = decode_browser_text_bytes(text, preserve_incomplete);
    text.clear();
    text.extend(pending);
    if !decoded.is_empty() {
        actions.push(BrowserInputAction::Text(decoded));
    }
}

fn decode_browser_text_bytes(mut bytes: &[u8], preserve_incomplete: bool) -> (String, Vec<u8>) {
    let mut out = String::new();
    let mut pending = Vec::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(valid) => {
                out.push_str(valid);
                break;
            }
            Err(err) => {
                let valid_up_to = err.valid_up_to();
                if valid_up_to > 0 {
                    out.push_str(std::str::from_utf8(&bytes[..valid_up_to]).unwrap_or(""));
                }
                let Some(error_len) = err.error_len() else {
                    if preserve_incomplete {
                        pending.extend_from_slice(&bytes[valid_up_to..]);
                    }
                    break;
                };
                bytes = &bytes[valid_up_to + error_len..];
            }
        }
    }
    (out, pending)
}

fn print_semantic_snapshot(url: &str, compact: bool) -> Result<()> {
    let mut browser = HeadlessBrowserApp::launch(url, 1024, 768)?;
    let snapshot = browser.semantic_snapshot()?;
    if compact {
        println!("{}", serde_json::to_string(&snapshot)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    }
    Ok(())
}

fn browser_capabilities_json_text() -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-browser-native-capabilities",
            "surface": "kittwm-browser",
            "surface_kind": "browser",
            "sdk_entry": "SurfaceSpec::browser",
            "sdk_backed": true,
            "kitty_graphics_native": true,
            "kittui_entries": [
                "HeadlessBrowserApp -> Runtime::place_png_frame_with_options",
                "SemanticSurfaceSnapshot -> render_sdk_semantic_surface -> Runtime::place_at_with_options"
            ],
            "semantic_outputs": [
                "--semantic-snapshot",
                "--semantic-scene-json",
                "--semantic-kitty",
                "--semantic-graphics"
            ],
            "render_outputs": [
                "default browser PNG surface",
                "semantic kittui scene JSON",
                "semantic kitty graphics"
            ]
        })
    )
}

fn browser_capabilities_text() -> String {
    let json: serde_json::Value = serde_json::from_str(&browser_capabilities_json_text())
        .expect("capabilities JSON is valid");
    let mut out = String::from("kittwm-browser native capabilities\n");
    out.push_str(&format!(
        "surface: {} ({})\n",
        json["surface"].as_str().unwrap_or("kittwm-browser"),
        json["surface_kind"].as_str().unwrap_or("browser")
    ));
    out.push_str(&format!(
        "sdk: {} backed={}\n",
        json["sdk_entry"].as_str().unwrap_or("SurfaceSpec::browser"),
        json["sdk_backed"].as_bool().unwrap_or(false)
    ));
    out.push_str(&format!(
        "kitty graphics native: {}\n",
        if json["kitty_graphics_native"].as_bool().unwrap_or(false) {
            "yes"
        } else {
            "no"
        }
    ));
    out.push_str("kittui entries:\n");
    if let Some(entries) = json["kittui_entries"].as_array() {
        for entry in entries {
            out.push_str(&format!("  - {}\n", entry.as_str().unwrap_or_default()));
        }
    }
    out.push_str("semantic outputs:\n");
    if let Some(outputs) = json["semantic_outputs"].as_array() {
        for output in outputs {
            out.push_str(&format!("  - {}\n", output.as_str().unwrap_or_default()));
        }
    }
    out
}

fn print_capabilities() -> Result<()> {
    print!("{}", browser_capabilities_text());
    Ok(())
}

fn print_capabilities_json() -> Result<()> {
    print!("{}", browser_capabilities_json_text());
    Ok(())
}

fn print_semantic_scene_json(url: &str) -> Result<()> {
    let mut browser = HeadlessBrowserApp::launch(url, 1024, 768)?;
    let snapshot = browser.semantic_snapshot()?;
    let scene = browser_semantic_scene(&snapshot);
    println!("{}", serde_json::to_string(&scene)?);
    Ok(())
}

fn print_semantic_kitty(url: &str) -> Result<()> {
    let mut browser = HeadlessBrowserApp::launch(url, 1024, 768)?;
    let snapshot = browser.semantic_snapshot()?;
    let scene = browser_semantic_scene(&snapshot);
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;
    let placement = runtime.place_at_with_options(
        &scene,
        scene.footprint,
        &browser_semantic_scene_placement_options(),
    )?;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(placement.upload.as_bytes())?;
    handle.write_all(placement.placement.as_bytes())?;
    handle.flush()?;
    Ok(())
}

fn browser_semantic_scene(snapshot: &SemanticSurfaceSnapshot) -> kittui::Scene {
    render_sdk_semantic_surface(snapshot, CellSize::default())
}

fn browser_semantic_scene_placement_options() -> kitty::PlacementOptions {
    let mut options = kitty::PlacementOptions::absolute();
    options.z_index = BROWSER_IMAGE_Z_INDEX;
    options
}

struct BrowserSemanticPublisher {
    socket: Option<PathBuf>,
    window: String,
    active_interval: Duration,
    idle_interval: Duration,
    unchanged_payloads: u16,
    last_attempt: Option<Instant>,
    last_payload: Option<String>,
}

impl BrowserSemanticPublisher {
    fn from_env() -> Self {
        let socket = std::env::var_os("KITTWM_SOCKET")
            .or_else(|| std::env::var_os("KITTWM_SOCK"))
            .map(PathBuf::from);
        let window = std::env::var("KITTWM_WINDOW").unwrap_or_else(|_| "focused".to_string());
        let active_interval = browser_semantic_active_interval();
        Self {
            socket,
            window,
            active_interval,
            idle_interval: browser_semantic_idle_interval(active_interval),
            unchanged_payloads: 0,
            last_attempt: None,
            last_payload: None,
        }
    }

    fn maybe_publish(&mut self, browser: &mut HeadlessBrowserApp) {
        if self.socket.is_none() {
            return;
        }
        let now = Instant::now();
        if !self.due(now) {
            return;
        }
        self.last_attempt = Some(now);
        let Ok(snapshot) = browser.semantic_snapshot() else {
            return;
        };
        let Ok(payload) = serde_json::to_string(&snapshot) else {
            return;
        };
        if !self.record_payload(&payload) {
            return;
        }
        if let Some(socket) = self.socket.as_ref() {
            let _ = publish_semantic_snapshot(socket, &self.window, &snapshot);
        }
    }

    fn due(&self, now: Instant) -> bool {
        self.last_attempt
            .map(|last| now.saturating_duration_since(last) >= self.current_interval())
            .unwrap_or(true)
    }

    fn current_interval(&self) -> Duration {
        browser_semantic_current_interval(
            self.active_interval,
            self.idle_interval,
            self.unchanged_payloads,
        )
    }

    fn reset_after_activity(&mut self) {
        self.unchanged_payloads = 0;
        self.last_attempt = None;
    }

    fn record_payload(&mut self, payload: &str) -> bool {
        if self.last_payload.as_deref() == Some(payload) {
            self.unchanged_payloads = self.unchanged_payloads.saturating_add(1);
            return false;
        }
        match self.last_payload.as_mut() {
            Some(last_payload) => {
                last_payload.clear();
                last_payload.push_str(payload);
            }
            None => self.last_payload = Some(payload.to_string()),
        }
        self.unchanged_payloads = 0;
        true
    }
}

fn publish_semantic_snapshot(
    socket: &PathBuf,
    window: &str,
    snapshot: &SemanticSurfaceSnapshot,
) -> Result<String> {
    Kittwm::connect_path(socket)
        .surface(window)
        .semantic_publish(snapshot)
        .map_err(|e| anyhow!(e))
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut chars = s.chars();
    let mut out = String::with_capacity(max.min(s.len()));
    for _ in 0..max {
        let Some(ch) = chars.next() else {
            return s.to_string();
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

fn browser_status_frame_counter_enabled() -> bool {
    std::env::var("KITTWM_BROWSER_STATUS_FRAMES")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

fn browser_semantic_active_interval() -> Duration {
    browser_interval_from_env("KITTWM_BROWSER_SEMANTIC_MS", 500)
}

fn browser_semantic_idle_interval(active: Duration) -> Duration {
    browser_interval_from_env("KITTWM_BROWSER_SEMANTIC_IDLE_MS", 2000).max(active)
}

fn browser_semantic_current_interval(
    active: Duration,
    idle: Duration,
    unchanged_payloads: u16,
) -> Duration {
    if unchanged_payloads >= 2 {
        idle
    } else {
        active
    }
}

fn browser_interval_from_env(name: &str, default_ms: u64) -> Duration {
    let ms = std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_ms)
        .clamp(16, 10_000);
    Duration::from_millis(ms)
}

fn browser_active_frame_interval() -> Duration {
    browser_interval_from_env("KITTWM_BROWSER_FRAME_MS", 250)
}

fn browser_idle_frame_interval(active: Duration) -> Duration {
    browser_interval_from_env("KITTWM_BROWSER_IDLE_MS", 1000).max(active)
}

fn browser_static_frame_interval(idle: Duration) -> Duration {
    browser_interval_from_env("KITTWM_BROWSER_STATIC_MS", 3000).max(idle)
}

fn browser_current_frame_interval(
    active: Duration,
    idle: Duration,
    static_idle: Duration,
    consecutive_idle_frames: u16,
) -> Duration {
    if consecutive_idle_frames >= 10 {
        static_idle
    } else if consecutive_idle_frames >= 2 {
        idle
    } else {
        active
    }
}

fn update_browser_idle_counter(counter: &mut u16, activity: bool) {
    if activity {
        *counter = 0;
    } else {
        *counter = counter.saturating_add(1);
    }
}

fn browser_sleep_poll_chunk(slack: Duration) -> Duration {
    slack.min(Duration::from_millis(50))
}

fn sleep_browser_frame_or_input(mut slack: Duration) {
    while slack > Duration::ZERO {
        let chunk = browser_sleep_poll_chunk(slack);
        if stdin_ready(chunk) {
            break;
        }
        slack = slack.saturating_sub(chunk);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BrowserStatusMetadata {
    window: String,
    socket: String,
}

impl BrowserStatusMetadata {
    fn from_env() -> Self {
        Self {
            window: truncate(
                &std::env::var("KITTWM_WINDOW").unwrap_or_else(|_| "<none>".into()),
                32,
            ),
            socket: truncate(
                &std::env::var("KITTWM_SOCKET").unwrap_or_else(|_| "<none>".into()),
                48,
            ),
        }
    }
}

#[cfg(test)]
fn browser_status_text(url: &str, frame: u64, show_frame: bool) -> String {
    browser_status_text_with_metadata(url, frame, show_frame, &BrowserStatusMetadata::from_env())
}

#[cfg(test)]
fn browser_status_text_with_metadata(
    url: &str,
    frame: u64,
    show_frame: bool,
    metadata: &BrowserStatusMetadata,
) -> String {
    browser_status_text_with_precomputed_url(&truncate(url, 40), frame, show_frame, metadata)
}

fn browser_status_text_with_precomputed_url(
    url_label: &str,
    frame: u64,
    show_frame: bool,
    metadata: &BrowserStatusMetadata,
) -> String {
    let mut status = format!(
        "kittwm-browser — {} — window={} socket={} — Ctrl-] exits",
        url_label, metadata.window, metadata.socket
    );
    if show_frame {
        status.push_str(&format!(" — frame {frame}"));
    }
    status
}

#[cfg(test)]
fn browser_status_text_for_cols(url: &str, frame: u64, show_frame: bool, cols: u16) -> String {
    clip_to_cols(&browser_status_text(url, frame, show_frame), cols as usize)
}

fn browser_status_text_for_cols_with_precomputed_url(
    url_label: &str,
    frame: u64,
    show_frame: bool,
    cols: u16,
    metadata: &BrowserStatusMetadata,
) -> String {
    clip_to_cols(
        &browser_status_text_with_precomputed_url(url_label, frame, show_frame, metadata),
        cols as usize,
    )
}

fn clip_to_cols(s: &str, cols: usize) -> String {
    if cols == 0 {
        return String::new();
    }
    let mut chars = s.chars();
    let mut out = String::with_capacity(cols.min(s.len()));
    for _ in 0..cols {
        let Some(ch) = chars.next() else {
            return out;
        };
        out.push(ch);
    }
    if chars.next().is_none() {
        return out;
    }
    out.pop();
    out.push('…');
    out
}

fn should_write_browser_status(
    last_status: Option<&(u16, String)>,
    next_row: u16,
    next_status: &str,
) -> bool {
    match last_status {
        Some((row, status)) => *row != next_row || status != next_status,
        None => true,
    }
}

fn browser_frame_key(bytes: &[u8]) -> (usize, u64) {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    (bytes.len(), hasher.finish())
}

fn should_upload_browser_frame(last_key: Option<(usize, u64)>, next_key: (usize, u64)) -> bool {
    last_key != Some(next_key)
}

fn should_build_browser_placement(upload_frame: bool, placed: bool) -> bool {
    upload_frame || !placed
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct BrowserViewport {
    cols: u16,
    raw_rows: u16,
    content_rows: u16,
    status_row: u16,
}

impl BrowserViewport {
    fn from_terminal_cells((cols, raw_rows): (u16, u16)) -> Self {
        let raw_rows = raw_rows.max(1);
        let content_rows = raw_rows.saturating_sub(BROWSER_RESERVED_STATUS_ROWS).max(1);
        Self {
            cols: cols.max(1),
            raw_rows,
            content_rows,
            status_row: raw_rows,
        }
    }

    fn frame_footprint(&self) -> CellRect {
        CellRect::new(0, 0, self.cols, self.content_rows)
    }
}

fn browser_image_placement_options() -> kitty::PlacementOptions {
    let mut options = kitty::PlacementOptions::absolute();
    options.z_index = BROWSER_IMAGE_Z_INDEX;
    options
}

#[cfg(test)]
fn browser_image_placement(image_id: u32, footprint: CellRect, transport: Transport) -> String {
    let options = browser_image_placement_options();
    format!(
        "{}{}",
        kitty::cursor_move(footprint.x, footprint.y, transport),
        kitty::placement_command_ex(image_id, footprint, &options, transport)
    )
}

fn terminal_cells() -> Option<(u16, u16)> {
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

fn stdin_ready(timeout: Duration) -> bool {
    let mut fds = unsafe { std::mem::zeroed::<libc::fd_set>() };
    unsafe { libc::FD_ZERO(&mut fds) };
    unsafe { libc::FD_SET(libc::STDIN_FILENO, &mut fds) };
    let mut tv = libc::timeval {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_usec: timeout.subsec_micros() as libc::suseconds_t,
    };
    let rc = unsafe {
        libc::select(
            libc::STDIN_FILENO + 1,
            &mut fds,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut tv,
        )
    };
    rc > 0
}

struct TtyGuard {
    orig: libc::termios,
}

impl TtyGuard {
    fn enter() -> Result<Self> {
        let mut orig = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut orig) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut raw = orig;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        print!("\x1b[?1049h\x1b[?25l");
        std::io::stdout().flush().ok();
        Ok(Self { orig })
    }
}

impl Drop for TtyGuard {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.orig) };
        print!("\x1b[?25h\x1b[?1049l");
        std::io::stdout().flush().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn parses_semantic_snapshot_flags() {
        let args =
            BrowserArgs::parse_from(["--semantic-snapshot", "--pretty", "https://example.com/app"])
                .unwrap();
        assert_eq!(args.url, "https://example.com/app");
        assert!(args.semantic_snapshot);
        assert!(!args.compact_json);
    }

    #[test]
    fn parses_print_semantic_alias_and_default_url() {
        let args = BrowserArgs::parse_from(["--print-semantic"]).unwrap();
        assert_eq!(args.url, DEFAULT_URL);
        assert!(args.semantic_snapshot);
        assert!(args.compact_json);
    }

    #[test]
    fn parses_semantic_scene_and_kitty_modes() {
        let scene =
            BrowserArgs::parse_from(["--semantic-scene-json", "https://example.com"]).unwrap();
        assert_eq!(scene.url, "https://example.com");
        assert!(scene.semantic_scene_json);
        assert!(!scene.semantic_kitty);

        let kitty = BrowserArgs::parse_from(["--semantic-graphics"]).unwrap();
        assert_eq!(kitty.url, DEFAULT_URL);
        assert!(kitty.semantic_kitty);
        assert!(!kitty.semantic_scene_json);

        let caps = BrowserArgs::parse_from(["--capabilities"]).unwrap();
        assert!(caps.capabilities);
        assert!(!caps.capabilities_json);
        let caps_json = BrowserArgs::parse_from(["--capabilities-json"]).unwrap();
        assert!(caps_json.capabilities_json);

        let help = help_text();
        assert!(help.contains("--semantic-scene-json"), "{help}");
        assert!(help.contains("--semantic-kitty"), "{help}");
        assert!(help.contains("--capabilities"), "{help}");
        assert!(help.contains("--capabilities-json"), "{help}");
    }

    #[test]
    fn browser_capabilities_text_reports_sdk_and_kittui_paths() {
        let text = browser_capabilities_text();
        assert!(
            text.contains("kittwm-browser native capabilities"),
            "{text}"
        );
        assert!(text.contains("surface: kittwm-browser (browser)"), "{text}");
        assert!(
            text.contains("sdk: SurfaceSpec::browser backed=true"),
            "{text}"
        );
        assert!(text.contains("kitty graphics native: yes"), "{text}");
        assert!(
            text.contains("Runtime::place_png_frame_with_options"),
            "{text}"
        );
        assert!(text.contains("--semantic-scene-json"), "{text}");
    }

    #[test]
    fn browser_capabilities_json_reports_sdk_and_kittui_paths() {
        let json: serde_json::Value =
            serde_json::from_str(&browser_capabilities_json_text()).unwrap();
        assert_eq!(json["kind"], "kittwm-browser-native-capabilities");
        assert_eq!(json["surface_kind"], "browser");
        assert_eq!(json["sdk_entry"], "SurfaceSpec::browser");
        assert_eq!(json["kitty_graphics_native"], true);
        assert!(json["kittui_entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry
                .as_str()
                .unwrap()
                .contains("Runtime::place_png_frame_with_options")));
        assert!(json["semantic_outputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "--semantic-scene-json"));
    }

    #[test]
    fn browser_idle_capture_pacing_uses_longer_interval_after_static_frames() {
        let active = Duration::from_millis(250);
        let idle = Duration::from_millis(1000);
        let static_idle = Duration::from_millis(3000);
        assert_eq!(
            browser_current_frame_interval(active, idle, static_idle, 0),
            active
        );
        assert_eq!(
            browser_current_frame_interval(active, idle, static_idle, 1),
            active
        );
        assert_eq!(
            browser_current_frame_interval(active, idle, static_idle, 2),
            idle
        );
        assert_eq!(
            browser_current_frame_interval(active, idle, static_idle, 10),
            static_idle
        );

        let mut idle_frames = 0u16;
        update_browser_idle_counter(&mut idle_frames, false);
        assert_eq!(idle_frames, 1);
        update_browser_idle_counter(&mut idle_frames, false);
        assert_eq!(idle_frames, 2);
        update_browser_idle_counter(&mut idle_frames, true);
        assert_eq!(idle_frames, 0);
        assert_eq!(
            browser_idle_frame_interval(Duration::from_millis(1500)),
            Duration::from_millis(1500)
        );
        assert_eq!(
            browser_static_frame_interval(Duration::from_millis(4000)),
            Duration::from_millis(4000)
        );
    }

    #[test]
    fn browser_sleep_poll_chunk_bounds_idle_waits() {
        assert_eq!(
            browser_sleep_poll_chunk(Duration::from_millis(1000)),
            Duration::from_millis(50)
        );
        assert_eq!(
            browser_sleep_poll_chunk(Duration::from_millis(17)),
            Duration::from_millis(17)
        );
    }

    #[test]
    fn browser_idle_counter_resets_on_user_activity() {
        let active = Duration::from_millis(250);
        let idle = Duration::from_millis(1000);
        let mut idle_frames = 2u16;
        let static_idle = Duration::from_millis(3000);
        assert_eq!(
            browser_current_frame_interval(active, idle, static_idle, idle_frames),
            idle
        );
        update_browser_idle_counter(&mut idle_frames, true);
        assert_eq!(idle_frames, 0);
        assert_eq!(
            browser_current_frame_interval(active, idle, static_idle, idle_frames),
            active
        );
    }

    #[test]
    fn browser_input_actions_preserve_text_backspace_tab_enter_page_and_arrow_order() {
        assert_eq!(
            browser_input_actions(
                b"ab\x7fc\x1b[8;5u\x1b[127;5u\t\x1b[Zde\x1b[D\x1b[1;2A\x1b[1;2B\x1b[1;2C\x1b[1;2D\x1b[1;5A\x1b[1;5B\x1b[1;5C\x1b[1;5D\x1b[1;3A\x1b[1;3B\x1b[1;3C\x1b[1;3D\x1b[1;2H\x1b[1;2F\x1b[1;5H\x1b[1;5F\x1b[1;3H\x1b[1;3F\x1b[2~\x1b[3~\x1b[2;2~\x1b[3;2~\x1b[2;5~\x1b[3;5~\x1b[2;3~\x1b[3;3~\x1b[H\x1b[F\x1b[5;2~\x1b[6;2~\x1b[5;5~\x1b[6;5~\x1b[5;3~\x1b[6;3~\x1b[5~\x1b[6~\x1b[13;2u\x1b[13;5u\x1b\x08\rfg\n"
            ),
            vec![
                BrowserInputAction::Text("ab".to_string()),
                BrowserInputAction::Backspace,
                BrowserInputAction::Text("c".to_string()),
                BrowserInputAction::CtrlBackspace,
                BrowserInputAction::CtrlBackspace,
                BrowserInputAction::Tab,
                BrowserInputAction::ShiftTab,
                BrowserInputAction::Text("de".to_string()),
                BrowserInputAction::Arrow(BrowserArrowKey::Left),
                BrowserInputAction::ShiftArrow(BrowserArrowKey::Up),
                BrowserInputAction::ShiftArrow(BrowserArrowKey::Down),
                BrowserInputAction::ShiftArrow(BrowserArrowKey::Right),
                BrowserInputAction::ShiftArrow(BrowserArrowKey::Left),
                BrowserInputAction::CtrlArrow(BrowserArrowKey::Up),
                BrowserInputAction::CtrlArrow(BrowserArrowKey::Down),
                BrowserInputAction::CtrlArrow(BrowserArrowKey::Right),
                BrowserInputAction::CtrlArrow(BrowserArrowKey::Left),
                BrowserInputAction::AltArrow(BrowserArrowKey::Up),
                BrowserInputAction::AltArrow(BrowserArrowKey::Down),
                BrowserInputAction::AltArrow(BrowserArrowKey::Right),
                BrowserInputAction::AltArrow(BrowserArrowKey::Left),
                BrowserInputAction::ShiftHome,
                BrowserInputAction::ShiftEnd,
                BrowserInputAction::CtrlHome,
                BrowserInputAction::CtrlEnd,
                BrowserInputAction::AltHome,
                BrowserInputAction::AltEnd,
                BrowserInputAction::Insert,
                BrowserInputAction::Delete,
                BrowserInputAction::ShiftInsert,
                BrowserInputAction::ShiftDelete,
                BrowserInputAction::CtrlInsert,
                BrowserInputAction::CtrlDelete,
                BrowserInputAction::AltInsert,
                BrowserInputAction::AltDelete,
                BrowserInputAction::Home,
                BrowserInputAction::End,
                BrowserInputAction::ShiftPage(BrowserPageKey::Up),
                BrowserInputAction::ShiftPage(BrowserPageKey::Down),
                BrowserInputAction::CtrlPage(BrowserPageKey::Up),
                BrowserInputAction::CtrlPage(BrowserPageKey::Down),
                BrowserInputAction::AltPage(BrowserPageKey::Up),
                BrowserInputAction::AltPage(BrowserPageKey::Down),
                BrowserInputAction::Page(BrowserPageKey::Up),
                BrowserInputAction::Page(BrowserPageKey::Down),
                BrowserInputAction::ShiftEnter,
                BrowserInputAction::CtrlEnter,
                BrowserInputAction::Escape,
                BrowserInputAction::Backspace,
                BrowserInputAction::Enter,
                BrowserInputAction::Text("fg".to_string()),
                BrowserInputAction::Enter,
            ]
        );
        assert_eq!(
            browser_input_actions(b"\x1b[A\x1b[B\x1b[C\x1b[D"),
            vec![
                BrowserInputAction::Arrow(BrowserArrowKey::Up),
                BrowserInputAction::Arrow(BrowserArrowKey::Down),
                BrowserInputAction::Arrow(BrowserArrowKey::Right),
                BrowserInputAction::Arrow(BrowserArrowKey::Left),
            ]
        );
        assert_eq!(
            browser_input_actions(b"\x1bOA\x1bOB\x1bOC\x1bOD\x1bOH\x1bOF"),
            vec![
                BrowserInputAction::Arrow(BrowserArrowKey::Up),
                BrowserInputAction::Arrow(BrowserArrowKey::Down),
                BrowserInputAction::Arrow(BrowserArrowKey::Right),
                BrowserInputAction::Arrow(BrowserArrowKey::Left),
                BrowserInputAction::Home,
                BrowserInputAction::End,
            ]
        );
        assert_eq!(
            browser_input_actions(b"x\x1b[1~y\x1b[4~z\x1b[7~\x1b[8~"),
            vec![
                BrowserInputAction::Text("x".to_string()),
                BrowserInputAction::Home,
                BrowserInputAction::Text("y".to_string()),
                BrowserInputAction::End,
                BrowserInputAction::Text("z".to_string()),
                BrowserInputAction::Home,
                BrowserInputAction::End,
            ]
        );
        assert_eq!(
            browser_input_actions(b"x\x1b[200~y"),
            vec![BrowserInputAction::Text("xy".to_string())]
        );
        assert_eq!(
            browser_input_actions(&[0x1b]),
            vec![BrowserInputAction::Escape]
        );
        assert_eq!(
            browser_input_actions(b"x\x1b["),
            vec![BrowserInputAction::Text("x".to_string())]
        );
        assert_eq!(
            browser_input_actions(b"x\x1b[2"),
            vec![BrowserInputAction::Text("x".to_string())]
        );
        assert_eq!(
            browser_input_actions(b"x\x1bO"),
            vec![BrowserInputAction::Text("x".to_string())]
        );
        assert_eq!(
            browser_input_actions("hé🙂\x08水".as_bytes()),
            vec![
                BrowserInputAction::Text("hé🙂".to_string()),
                BrowserInputAction::Backspace,
                BrowserInputAction::Text("水".to_string()),
            ]
        );
        let mut parser = BrowserInputParser::default();
        let smile = "🙂".as_bytes();
        assert_eq!(parser.actions(&smile[..2]), vec![]);
        assert_eq!(
            parser.actions(&smile[2..]),
            vec![BrowserInputAction::Text("🙂".to_string())]
        );
        let mut parser = BrowserInputParser::default();
        assert_eq!(
            parser.actions(b"x\x1b[1;"),
            vec![BrowserInputAction::Text("x".to_string())]
        );
        assert_eq!(
            parser.actions(b"5Cy"),
            vec![
                BrowserInputAction::CtrlArrow(BrowserArrowKey::Right),
                BrowserInputAction::Text("y".to_string()),
            ]
        );
        let mut parser = BrowserInputParser::default();
        assert_eq!(parser.actions(b"\x1bO"), vec![]);
        assert_eq!(parser.actions(b"H"), vec![BrowserInputAction::Home]);
    }

    #[test]
    fn browser_frame_upload_skips_unchanged_png_content() {
        let key = browser_frame_key(b"same png bytes");
        assert!(should_upload_browser_frame(None, key));
        assert!(!should_upload_browser_frame(Some(key), key));
        let changed = browser_frame_key(b"different png bytes");
        assert!(should_upload_browser_frame(Some(key), changed));
        assert!(should_build_browser_placement(true, true));
        assert!(should_build_browser_placement(false, false));
        assert!(!should_build_browser_placement(false, true));
    }

    #[test]
    fn browser_status_is_stable_by_default_and_frame_opt_in() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WINDOW", "win-1");
        std::env::set_var("KITTWM_SOCKET", "/tmp/kittwm.sock");
        std::env::remove_var("KITTWM_BROWSER_STATUS_FRAMES");
        assert!(!browser_status_frame_counter_enabled());
        let huge_url = format!("https://example.com/{}", "path/".repeat(10_000));
        assert_eq!(truncate(&huge_url, 12), "https://exa…");
        assert_eq!(truncate("short", 12), "short");
        assert_eq!(truncate("anything", 1), "…");
        assert_eq!(truncate("anything", 0), "");
        let metadata = BrowserStatusMetadata::from_env();
        assert_eq!(metadata.window, "win-1");
        assert_eq!(metadata.socket, "/tmp/kittwm.sock");
        let stable =
            browser_status_text_with_metadata("https://example.com/a", 42, false, &metadata);
        assert!(
            stable.starts_with("kittwm-browser — https://example.com/a"),
            "{stable}"
        );
        assert!(!stable.contains("frame"), "{stable}");
        let with_frame =
            browser_status_text_with_metadata("https://example.com/a", 42, true, &metadata);
        assert!(with_frame.ends_with("frame 42"), "{with_frame}");
        let precomputed = browser_status_text_for_cols_with_precomputed_url(
            "https://example.com/a",
            42,
            false,
            200,
            &metadata,
        );
        assert_eq!(precomputed, stable);
        let narrow = browser_status_text_for_cols("https://example.com/a", 42, false, 12);
        assert_eq!(narrow.chars().count(), 12);
        assert!(narrow.ends_with('…'), "{narrow}");
        let huge_window = "window-".repeat(10_000);
        let huge_socket = format!("/tmp/{}", "sock/".repeat(10_000));
        std::env::set_var("KITTWM_WINDOW", huge_window);
        std::env::set_var("KITTWM_SOCKET", huge_socket);
        let huge_metadata = BrowserStatusMetadata::from_env();
        assert!(huge_metadata.window.ends_with('…'), "{:?}", huge_metadata);
        assert!(huge_metadata.socket.ends_with('…'), "{:?}", huge_metadata);
        let bounded_status =
            browser_status_text_with_metadata("https://example.com/a", 42, false, &huge_metadata);
        assert!(
            bounded_status.contains("window=window-window-window-window-win…"),
            "{bounded_status}"
        );
        assert!(
            bounded_status.contains("socket=/tmp/sock/sock/sock/sock/sock/sock/sock/sock/so…"),
            "{bounded_status}"
        );
        assert!(
            !bounded_status.contains(&"window-".repeat(8)),
            "{bounded_status}"
        );
        assert!(
            !bounded_status.contains(&"sock/".repeat(16)),
            "{bounded_status}"
        );
        let huge_status = browser_status_text_for_cols("https://example.com/a", 42, false, 24);
        assert_eq!(huge_status.chars().count(), 24);
        assert!(huge_status.ends_with('…'), "{huge_status}");
        assert!(!huge_status.contains(&"window-".repeat(4)), "{huge_status}");
        std::env::set_var("KITTWM_WINDOW", "win-1");
        assert_eq!(
            browser_status_text_for_cols("https://example.com/a", 42, false, 0),
            ""
        );
        assert_eq!(
            browser_status_text_for_cols("https://example.com/a", 42, false, 1),
            "…"
        );
        assert!(should_write_browser_status(None, 24, &stable));
        assert!(!should_write_browser_status(
            Some(&(24, stable.clone())),
            24,
            &stable
        ));
        assert!(should_write_browser_status(
            Some(&(23, stable.clone())),
            24,
            &stable
        ));
        std::env::set_var("KITTWM_BROWSER_STATUS_FRAMES", "1");
        assert!(browser_status_frame_counter_enabled());
        std::env::remove_var("KITTWM_BROWSER_STATUS_FRAMES");
        std::env::remove_var("KITTWM_WINDOW");
        std::env::remove_var("KITTWM_SOCKET");
    }

    #[test]
    fn rejects_unknown_browser_option() {
        let err = BrowserArgs::parse_from(["--bogus"]).unwrap_err();
        assert!(err.contains("unknown option --bogus"));
        assert!(err.contains("--semantic-snapshot"));
    }

    #[test]
    fn browser_viewport_clamps_content_and_status_to_reported_rows() {
        let normal = BrowserViewport::from_terminal_cells((100, 30));
        assert_eq!(normal.cols, 100);
        assert_eq!(normal.content_rows, 28);
        assert_eq!(normal.status_row, 30);
        assert_eq!(normal.frame_footprint(), CellRect::new(0, 0, 100, 28));

        let tiny = BrowserViewport::from_terminal_cells((0, 1));
        assert_eq!(tiny.cols, 1);
        assert_eq!(tiny.content_rows, 1);
        assert_eq!(tiny.status_row, 1);
        assert_eq!(tiny.frame_footprint(), CellRect::new(0, 0, 1, 1));
    }

    #[test]
    fn browser_image_placement_uses_absolute_kitty_graphics_without_placeholders() {
        let placement = browser_image_placement(42, CellRect::new(0, 0, 80, 22), Transport::Direct);
        assert!(placement.contains("a=p"), "{placement:?}");
        assert!(placement.contains("c=80"), "{placement:?}");
        assert!(placement.contains("r=22"), "{placement:?}");
        assert!(!placement.contains("U=1"), "{placement:?}");
    }

    #[test]
    fn browser_semantic_scene_renders_snapshot_through_kittui_affordances() {
        let snapshot = SemanticSurfaceSnapshot::new(
            "browser-1",
            7,
            kittwm_sdk::ComponentNode::new("root", kittwm_sdk::ComponentRole::Group).children(
                vec![kittwm_sdk::ComponentNode::new(
                    "search",
                    kittwm_sdk::ComponentRole::TextInput,
                )
                .labeled("Search")
                .valued(kittwm_sdk::ComponentValue::Text("kittwm".to_string()))
                .state(kittwm_sdk::ComponentState {
                    focused: true,
                    focusable: true,
                    ..kittwm_sdk::ComponentState::default()
                })],
            ),
        )
        .focused("search");
        let scene = browser_semantic_scene(&snapshot);
        assert!(scene.footprint.cols > 0);
        assert!(scene.footprint.rows > 0);
        assert!(!scene.layers.is_empty());
        let scene_json = serde_json::to_string(&scene).unwrap();
        assert!(scene_json.contains("control"), "{scene_json}");
        let opts = browser_semantic_scene_placement_options();
        assert!(!opts.unicode_placeholder);
        assert_eq!(opts.z_index, BROWSER_IMAGE_Z_INDEX);
    }

    #[test]
    fn semantic_publisher_debounces_and_skips_unchanged_payloads() {
        let start = Instant::now();
        let mut publisher = BrowserSemanticPublisher {
            socket: Some(PathBuf::from("/tmp/unused.sock")),
            window: "native-1".to_string(),
            active_interval: Duration::from_millis(500),
            idle_interval: Duration::from_millis(2000),
            unchanged_payloads: 0,
            last_attempt: None,
            last_payload: None,
        };

        assert!(publisher.due(start));
        publisher.last_attempt = Some(start);
        assert!(!publisher.due(start + Duration::from_millis(499)));
        assert!(publisher.due(start + Duration::from_millis(500)));
        assert!(publisher.record_payload("{\"revision\":1}"));
        assert!(!publisher.record_payload("{\"revision\":1}"));
        assert_eq!(publisher.unchanged_payloads, 1);
        assert!(!publisher.record_payload("{\"revision\":1}"));
        assert_eq!(publisher.unchanged_payloads, 2);
        assert_eq!(publisher.current_interval(), Duration::from_millis(2000));
        publisher.last_attempt = Some(start + Duration::from_millis(500));
        assert!(!publisher.due(start + Duration::from_millis(2499)));
        assert!(publisher.due(start + Duration::from_millis(2500)));
        publisher.reset_after_activity();
        assert_eq!(publisher.unchanged_payloads, 0);
        assert!(publisher.due(start + Duration::from_millis(501)));
        assert!(publisher.record_payload("{\"revision\":2}"));
        assert_eq!(publisher.unchanged_payloads, 0);
        assert_eq!(publisher.current_interval(), Duration::from_millis(500));
    }

    #[test]
    fn semantic_publisher_reuses_payload_string_allocation() {
        let mut publisher = BrowserSemanticPublisher {
            socket: Some(PathBuf::from("/tmp/unused.sock")),
            window: "native-1".to_string(),
            active_interval: Duration::from_millis(500),
            idle_interval: Duration::from_millis(2000),
            unchanged_payloads: 0,
            last_attempt: None,
            last_payload: None,
        };
        let large = "x".repeat(4096);
        assert!(publisher.record_payload(&large));
        let initial_capacity = publisher.last_payload.as_ref().unwrap().capacity();
        assert!(publisher.record_payload("tiny"));
        assert_eq!(publisher.last_payload.as_deref(), Some("tiny"));
        assert!(publisher.last_payload.as_ref().unwrap().capacity() >= initial_capacity);
    }

    #[test]
    fn semantic_publisher_defaults_to_focused_without_socket() {
        let publisher = BrowserSemanticPublisher {
            socket: None,
            window: "focused".to_string(),
            active_interval: Duration::from_millis(500),
            idle_interval: Duration::from_millis(2000),
            unchanged_payloads: 0,
            last_attempt: None,
            last_payload: None,
        };
        assert!(publisher.socket.is_none());
        assert_eq!(publisher.window, "focused");
    }
}
