# Session summary — native PTY alternate screen

## Goal

Close a major terminal-WM fidelity gap by supporting alternate-screen DEC private modes in native kittwm PTY panes.

## Bead(s)

- `bd-9ea3aa` — kittwm: support native PTY alternate screen

## Before state

- Failing tests: none known.
- Relevant gap: full-screen terminal apps commonly enter `CSI ? 1049 h` and leave with `CSI ? 1049 l`. Native kittwm did not maintain separate normal/alternate buffers, so TUI output could contaminate the shell snapshot and there was no restore of the normal prompt view.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_honors_alternate_screen_modes -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_resizes_saved_alternate_screen_buffer -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_edit -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_additional_cursor_csi_modes -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks an optional saved normal buffer/cursor while alternate screen is active. DEC private `?1049h` / `?1049l` and compatible `?47` / `?1047` switch between visible buffers. Resize now preserves both active and saved buffers. docs/wm notes alternate-screen fidelity.

## Diff summary

- Code/content commit: `a45f9b9`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: full-screen TUIs can draw into alternate screen snapshots and leave back to restored shell text/cursor.

## Operator-takeaway

Native kittwm panes now behave much more like a real terminal for full-screen apps; `READ_TEXT` should show the active TUI while in alternate mode and restored shell contents after exit.
