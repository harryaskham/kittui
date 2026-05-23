# Session summary — application cursor-key mode in native panes

## Goal

Improve native kittwm input fidelity by honoring DECCKM application cursor-key mode for hosted TUIs.

## Bead(s)

- `bd-a7fa5f` — kittwm: honor application cursor-key mode in native panes

## Before state

- Failing tests: none known.
- Relevant gap: native panes parsed many DEC modes but ignored `CSI ? 1 h/l`. TUIs that enable application cursor-key mode expect arrow keys as SS3 `ESC O A/B/C/D`; native host key routing kept forwarding normal CSI arrows, so navigation could be misinterpreted.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_tracks_application_cursor_key_mode -- --nocapture` passed.
  - `cargo test -p kittui-cli native_pane_tests::native_key_event_payload_honors_application_cursor_mode -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks `application_cursor_keys` and handles DEC private `?1h` / `?1l`. `PtyTerminalApp` exposes the mode. Native pane status text/JSON includes `application_cursor_keys`. Parsed host arrow key events are converted to SS3 arrows when the focused pane has application cursor-key mode enabled, and remain normal CSI arrows otherwise. Modified arrows are left on the raw path.

## Diff summary

- Code/content commit: `59163d6`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `README.md`, `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: full-screen TUIs that request application cursor keys receive the expected arrow encoding from native kittwm.

## Operator-takeaway

Native kittwm now adapts arrow-key input to a pane's requested cursor-key mode and publishes that mode for automation/inspection.
