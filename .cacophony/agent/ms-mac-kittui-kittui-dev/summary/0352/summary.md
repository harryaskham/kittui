# Session summary — DEC origin mode in native PTY

## Goal

Improve native kittwm TUI fidelity by supporting DEC origin mode together with scroll regions.

## Bead(s)

- `bd-b145a9` — kittwm: support DEC origin mode in native PTY

## Before state

- Failing tests: none known.
- Relevant gap: native kittwm supported scroll regions but not DEC origin mode (`CSI ? 6 h/l`). TUIs often enable origin mode so cursor addressing is relative to scroll margins; without it, output could land in fixed header/status rows.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_honors_origin_mode_with_scroll_region -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_origin_mode_disable_restores_absolute_addressing -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_scroll_region -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks origin mode, handles DEC private `?6h` / `?6l`, resets cursor home on toggle, and makes `CSI H/f` row addressing relative to `scroll_top..scroll_bottom` when origin mode is enabled. docs/wm now mentions scroll region/origin mode fidelity.

## Diff summary

- Code/content commit: `8891991`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: TUI cursor addressing inside scroll margins is more faithful and less likely to corrupt headers/footers.

## Operator-takeaway

Native kittwm now handles DEC origin mode, a common companion to scroll margins in full-screen terminal apps.
