# Session summary — native PTY scrollback snapshots

## Goal

Close a terminal-WM automation gap by adding native PTY scrollback and exposing it through the kittwm socket/CLI control plane.

## Bead(s)

- `bd-d78bd3` — kittwm: expose native PTY scrollback snapshots

## Before state

- Failing tests: none known.
- Relevant gap: native panes only exposed the current screen via `READ_TEXT`. Output that scrolled off-screen was lost to controllers unless they polled fast enough, which is a large gap for a terminal-based WM and shell automation substrate.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_captures_scrollback_on_scroll -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_does_not_capture_alt_screen_scrollback -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_read_text_round_trip_over_socket -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Native `TerminalState` now keeps a bounded 10k-line scrollback for normal-screen scrolls and skips alternate-screen scrolls. `PtyTerminalApp::scrollback_snapshot()` exposes it internally. Native pane status carries non-serialized scrollback side-data. Socket commands `READ_SCROLLBACK` and `READ_SCROLLBACK_JSON` expose it, and `kittwm --read-scrollback WINDOW` wraps the text command. README/docs/help were updated.

## Diff summary

- Code/content commit: `6b21a0a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: controllers can retrieve prior normal-screen output after it scrolls off the active pane.

## Operator-takeaway

Use `kittwm --read-scrollback focused` or socket `READ_SCROLLBACK_JSON focused` to inspect off-screen output without scraping or polling every frame.
