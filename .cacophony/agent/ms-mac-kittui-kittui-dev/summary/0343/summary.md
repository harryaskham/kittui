# Session summary — native PTY cursor visibility

## Goal

Make native kittwm panes visibly show/hide the terminal cursor and publish cursor visibility in status metadata.

## Bead(s)

- `bd-abf2c3` — kittwm: render and publish native PTY cursor visibility

## Before state

- Failing tests: none known.
- Relevant gap: native PTY status exposed cursor coordinates, but rendered panes had no visible caret and did not honor `CSI ? 25 h/l`. Users could not see the cursor even though controllers could inspect its position.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_tracks_cursor_visibility_mode -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_renderer_draws_visible_cursor -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Native `TerminalState` now tracks cursor visibility and handles DEC private `?25h` / `?25l`. `PtyTerminalApp::cursor_visible()` exposes it. Native pane status includes optional `cursor_visible`, and text `PANES` prints `cursor_visible=on|off|-`. `render_terminal_rgba` draws a small cursor underline at the cursor cell when visible. docs/wm now mentions cursor visibility metadata/fidelity.

## Diff summary

- Code/content commit: `a1e8e23`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-cli/src/daemon.rs`, `docs/wm.md`
- Behavioural delta: users can see a native pane cursor, apps can hide/show it, and controllers can inspect visibility state.

## Operator-takeaway

Native kittwm panes now display a caret and expose `cursor_visible` via `PANES_JSON` / `STATUS_JSON`.
