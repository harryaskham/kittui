# Session summary — native pane status over socket

## Goal

Make the live native `kittwm` session socket inspectable by reporting actual visible native panes instead of only the SPAWN_PTY queue metadata.

## Bead(s)

- `bd-3c5bfa` — kittwm: report live native panes over session socket

## Before state

- Failing tests: none known.
- Relevant gap: no-arg native `kittwm` owned a `SPAWN_PTY` socket queue, but `STATUS` reported only pending command count and `PANES` was not useful for visible native pane state.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli native_pane_statuses -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `NativeSpawnQueue` now stores a live pane snapshot (`NativePaneStatus`) updated by the native session loop. `STATUS` reports `pending`, `panes`, and `focus`; `PANES` returns per-pane window/title/focus records.

## Diff summary

- Code/content commit: `950f161`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/session.rs`, `docs/wm.md`
- Behavioural delta: `kittwm --attach -c STATUS` and `kittwm --attach -c PANES` are now useful against live native sessions.

## Operator-takeaway

The native kittwm socket is closer to a real DISPLAY-like control plane: shell scripts can inspect visible panes and focused window state, not just enqueue spawns.
