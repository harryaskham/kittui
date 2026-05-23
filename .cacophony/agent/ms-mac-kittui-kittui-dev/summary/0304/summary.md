# Session summary — native pane text snapshots

## Goal

Expose native kittwm PTY pane screen text through the socket control plane for automation, accessibility, tests, and controller feedback loops.

## Bead(s)

- `bd-573757` — kittwm: add native socket pane text snapshots

## Before state

- Failing tests: none known.
- Relevant gap: controllers could spawn, inspect, focus, resize, move, inject text, and inject keys, but could not read pane text without scraping kitty graphics output.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_read_text_round_trip_over_socket -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: The native session publishes each pane's `PtyTerminalApp::text_snapshot()` to the socket queue as non-serialized side data on `NativePaneStatus`. Added:
  - `READ_TEXT <window|focused>` returning a multi-line `TEXT ...` response terminated by `END`.
  - `READ_TEXT_JSON <window|focused>` returning `{window, text}` JSON.
  These commands validate targets, support `focused`, avoid adding full text to `PANES_JSON`, and are included in HELP/HELP_JSON plus README/docs examples. `client_request_multi` now treats `TEXT ...` as a multi-line reply header.

## Diff summary

- Code/content commit: `88a8c2d`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/session.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: socket controllers can observe native pane terminal text directly.

## Operator-takeaway

Native kittwm now has the key loop for automation: drive panes over the socket, then read their current text snapshots back over the same control plane.
