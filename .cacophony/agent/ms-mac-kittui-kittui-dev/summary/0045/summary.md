# Session summary — launcher overlay enabled by default

## Goal

Move the live boxed launcher overlay from an opt-in flag to the default `kitwm` runtime behavior, so `kitwm` (no args) gives operators a real in-session launcher menu without extra flags.

## Bead(s)

- `bd-d0b716` — kitwm default launcher overlay: Ctrl-A d opens boxed menu without extra flag
- parent: `bd-031e54` — kittui-wm v2

## Before state

- Failing tests: none; workspace tests were green at wake start.
- Context: the in-session launcher overlay existed but required `--launcher-overlay` / `KITTUI_WM_LAUNCHER_OVERLAY=1`. Default `kitwm` sessions still spawned commands immediately on Ctrl-A d / Ctrl-A Enter.

## After state

- Failing tests: none in `cargo test --workspace --lib --bins --tests -- --test-threads=2`.
- Relevant metrics: existing launcher_overlay unit tests still pass; `RunOptions::default()` now sets `launcher_overlay: true` and the env var is interpreted as opt-out (`0`/`false`/`off`).
- Context: live runtime defaults to overlay on, with `--no-launcher-overlay` and `KITTUI_WM_LAUNCHER_OVERLAY=0` opt-outs. Help text updated.

## Diff summary

- Code/content commits: pending final squash SHA from reintegration receipt
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kitwm.rs`, `crates/kittui-cli/src/session.rs`, `.cacophony/agent/ms-mac-kittui-kittui-dev/summary/pending/screenshots/bd-d0b716-default-overlay.png`
- Tests: existing launcher_overlay coverage preserved / +0 new files / flipped 0
- Behavioural delta: operator-visible default: `kitwm --backend fake` + Ctrl-A d now opens the boxed launcher overlay without any extra flag.

## Embedded artefacts

- `screenshots/bd-d0b716-default-overlay.png` — tendril proof showing `env -u KITTUI_WM_LAUNCHER_OVERLAY ./target/release/kitwm --backend fake` opening the launcher overlay on Ctrl-A d.

## Operator-takeaway

`kitwm` defaults are now closer to the end-goal: a usable WM with a launcher menu, with the override available for shell-style immediate-spawn behavior.
