# Session summary — native pane cursor position status

## Goal

Publish native PTY cursor position through kittwm status surfaces for automation and preview/controller workflows.

## Bead(s)

- `bd-7c4a62` — kittwm: publish native pane cursor position

## Before state

- Failing tests: none known.
- Relevant gap: native pane status and `READ_TEXT` exposed screen contents but not cursor position, even though the PTY parser already tracked cursor row/column. Controllers could not tell where a prompt/caret was.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_reports_cursor_position -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added `PtyTerminalApp::cursor_position() -> (u16, u16)`. Native session snapshots now include `cursor_col` and `cursor_row` in `NativePaneStatus`. `PANES_JSON`, `STATUS_JSON` pane details, and text `PANES` output publish cursor metadata when available. docs/wm now mentions cursor metadata.

## Diff summary

- Code/content commit: `fb4b32f`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-cli/src/daemon.rs`, `docs/wm.md`
- Behavioural delta: external controllers can inspect native pane cursor position alongside text and geometry.

## Operator-takeaway

Use `kittwm --panes-json` or `STATUS_JSON` to inspect `cursor_col` / `cursor_row` for prompt/caret-aware automation.
