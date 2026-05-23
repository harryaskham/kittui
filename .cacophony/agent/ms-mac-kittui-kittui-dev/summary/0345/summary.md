# Session summary — native PTY mouse reporting modes

## Goal

Prepare native kittwm for correct mouse-aware pane routing by tracking and publishing terminal mouse-reporting modes requested by PTY apps.

## Bead(s)

- `bd-43c3e5` — kittwm: publish native PTY mouse reporting modes

## Before state

- Failing tests: none known.
- Relevant gap: native panes did not track `CSI ? 1000/1002/1003/1006 h/l` mouse-reporting modes. kittwm could not tell whether a TUI requested mouse input, which is a prerequisite for safe host-mouse routing into native panes.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_tracks_mouse_reporting_modes -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added public `MouseReportingModes` with `basic`, `button_motion`, `all_motion`, and `sgr`. Native PTY parser now toggles those for DEC private `?1000`, `?1002`, `?1003`, and `?1006`. `PtyTerminalApp::mouse_reporting_modes()` exposes state. Native pane status now includes optional `mouse_reporting`, `mouse_button_motion`, `mouse_all_motion`, and `mouse_sgr`; text `PANES` includes a compact `mouse=...` label. docs/wm documents the new status metadata.

## Diff summary

- Code/content commit: `b4d23e4`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-cli/src/daemon.rs`, `docs/wm.md`
- Behavioural delta: controllers and future native mouse routing can observe which panes request mouse input modes.

## Operator-takeaway

Use `PANES_JSON` / `STATUS_JSON` to inspect native pane mouse mode booleans before routing or diagnosing mouse-aware TUIs.
