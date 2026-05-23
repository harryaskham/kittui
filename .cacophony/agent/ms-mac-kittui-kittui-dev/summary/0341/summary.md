# Session summary — native PTY scroll regions

## Goal

Improve kittwm native terminal fidelity for TUIs/pagers by supporting DEC vertical scroll margins instead of always scrolling the whole screen.

## Bead(s)

- `bd-6b49da` — kittwm: support native PTY scroll regions

## Before state

- Failing tests: none known.
- Relevant gap: native kittwm scrolled the entire screen whenever output reached the bottom row. Full-screen TUIs commonly set `CSI top;bottom r` so body regions scroll while headers/status lines remain fixed; without this, rendered panes and `READ_TEXT` snapshots could corrupt fixed chrome/status lines.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_honors_scroll_region -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_resets_scroll_region -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_captures_scrollback_on_scroll -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_alternate_screen_modes -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks `scroll_top` / `scroll_bottom`, handles `CSI top;bottom r`, resets margins on `CSI r`, clamps margins across resize, and saves/restores normal-screen margins across alternate-screen entry/exit. Newline at the region bottom scrolls only the active region; full-screen scrollback behavior is preserved.

## Diff summary

- Code/content commit: `d34fb86`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: native pane rendering/snapshots keep header/footer/status rows stable for apps using DEC scroll margins.

## Operator-takeaway

Native kittwm handles another major TUI primitive: region-scoped scrolling for body panes with fixed status/header rows.
