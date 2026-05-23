# Session summary — terminal insert mode in native PTY

## Goal

Improve native kittwm editor/TUI fidelity by honoring terminal insert/replace mode.

## Bead(s)

- `bd-39cc45` — kittwm: honor terminal insert mode in native PTY

## Before state

- Failing tests: none known.
- Relevant gap: native kittwm implemented explicit insert-character CSI (`CSI n @`) but not IRM terminal insert mode (`CSI 4 h/l`). Apps that enable insert mode expect printable characters to shift existing cells right rather than overwrite them.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_honors_insert_mode -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_edit -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks `insert_mode`, handles non-private `CSI 4 h` / `CSI 4 l`, preserves the mode across resize, and resets it during terminal reset. Printable output in insert mode shifts the current row right by one cell before drawing; default replace behavior remains unchanged. docs/wm now mentions insert mode fidelity.

## Diff summary

- Code/content commit: `6e740d8`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: native pane rendering and READ_TEXT snapshots better match editors/TUIs that temporarily enable insert mode.

## Operator-takeaway

Native kittwm now supports IRM insert mode, closing another common terminal-fidelity gap for line editors and full-screen apps.
