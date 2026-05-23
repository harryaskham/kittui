# Session summary — native OSC pane titles

## Goal

Make native kittwm pane chrome/status follow terminal app OSC window-title updates instead of only showing the launch command or manual rename.

## Bead(s)

- `bd-0a947d` — kittwm: capture OSC titles for native pane chrome

## Before state

- Failing tests: none known.
- Relevant gap: `PtyTerminalApp::title()` returned the launch command forever unless session-level `RENAME_PANE` overrode it. Real terminal apps commonly publish window titles with OSC 0/1/2, but kittwm ignored those updates.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm pty_terminal_captures_osc_window_title -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_preserves_osc_title_across_resize -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now captures OSC 0/1/2 titles via `vte::Perform::osc_dispatch`, preserves the captured title across resize, and `PtyTerminalApp::title()` returns the captured terminal title when present. Existing native session title display still calls `native_pane_display_title`, so explicit socket `RENAME_PANE` display-title overrides continue to win at the WM chrome/status layer.

## Diff summary

- Code/content commit: `e1d13df`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`
- Behavioural delta: native pane title bars and socket status can reflect app-provided terminal window titles.

## Operator-takeaway

Native kittwm pane chrome is more terminal-realistic: shells/editors that emit OSC titles can name their panes automatically.
