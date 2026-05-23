# Session summary — DEC Special Graphics line drawing

## Goal

Improve native kittwm curses/TUI fidelity by honoring DEC Special Graphics character-set selection for ACS box drawing.

## Bead(s)

- `bd-aff821` — kittwm: support DEC Special Graphics line drawing

## Before state

- Failing tests: none known.
- Relevant gap: native kittwm rendered DEC ACS bytes literally. Curses apps that emit `ESC ( 0` followed by bytes like `lqkx` drew `lqqk`/`x  x` instead of box borders in rendered frames and `READ_TEXT` snapshots.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_honors_dec_special_graphics -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_full_and_soft_reset_controls -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks G0 DEC Special Graphics selection, handles `ESC ( 0` and `ESC ( B`, maps common DEC line-drawing/symbol bytes to Unicode glyphs during printable output, preserves the selection across resize, and resets it during terminal resets. docs/wm now mentions DEC Special Graphics line drawing.

## Diff summary

- Code/content commit: `f40be44`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: curses/ACS borders render as box-drawing characters instead of literal ASCII fallback bytes.

## Operator-takeaway

Native kittwm now handles a major curses-era terminal feature that still appears in many TUI box/chrome renderers.
