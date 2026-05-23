# Session summary — kittwm native PTY split panes

## Goal

Move the default no-arg `kittwm` path from a single fullscreen PTY proof toward a real terminal WM by adding multiple native PTY panes, split/focus controls, per-pane geometry, and minimal focus chrome.

## Bead(s)

- `bd-0760eb` — kittwm: add real native PTY split panes

## Before state

- Failing tests: none known.
- Relevant gap: `kittwm` with no backend flags spawned exactly one `PtyTerminalApp`, hard-coded `native-1`, rendered it fullscreen, and routed all stdin directly to that single app.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli native_pane -- --nocapture` passed.
  - `cargo test -p kittui-cli layout_state -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: no-arg `kittwm` now manages a vector of native panes. `Ctrl-A %`, `Ctrl-A |`, or `Ctrl-A v` spawns another shell pane; `Ctrl-A Tab` or `Ctrl-A n` cycles focus. Typing routes only to the focused pane. Pane geometry is recalculated on host resize, and each pane has a title/focus row.

## Diff summary

- Code/content commit: `fd11ec2`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: the native terminal session has real split panes and focus routing instead of a single fullscreen PTY.

## Operator-takeaway

This is the first substantial step toward making the default `kittwm` a terminal WM: it now has live pane state, split spawning, focused input routing, resize-aware pane layouts, and visible per-pane focus chrome.
