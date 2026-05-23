# Session summary — native PTY cursor save/restore

## Goal

Improve native kittwm PTY cursor fidelity by supporting common cursor save/restore sequences used by shells and TUIs.

## Bead(s)

- `bd-5c62ce` — kittwm: support native PTY cursor save and restore

## Before state

- Failing tests: none known.
- Relevant gap: native kittwm handled many cursor movement sequences but not DECSC/DECRC (`ESC 7` / `ESC 8`) or SCO-style `CSI s` / `CSI u`. Programs using these around status/prompt updates could leave subsequent output in the wrong cell and corrupt `READ_TEXT` snapshots.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_honors_cursor_save_restore_modes -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_additional_cursor_csi_modes -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks a saved cursor position, clamps it on resize, and handles `ESC 7`, `ESC 8`, `CSI s`, and `CSI u`. docs/wm now notes cursor movement/save/restore fidelity.

## Diff summary

- Code/content commit: `663ba2c`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: native pane rendering and text snapshots are more faithful for apps that save/restore cursor positions.

## Operator-takeaway

Native kittwm handles another common terminal cursor primitive, reducing prompt/status-line corruption in pane snapshots.
