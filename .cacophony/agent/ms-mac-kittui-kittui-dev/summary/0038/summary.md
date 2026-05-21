# Session summary — live launcher overlay

## Goal

Move the boxed launcher preview into the live raw-mode `kitwm` session as an in-session overlay, so launcher actions can display a candidate menu instead of immediately spawning.

## Bead(s)

- `bd-190a5c` — kitwm live launcher overlay: Ctrl-A Enter opens type-to-filter boxed menu
- parent/decomposes: `bd-8d64b1` — App launcher inside kittui-wm

## Before state

- Failing tests: none; workspace tests were green at wake start.
- Context: `kitwm launcher` provided an external boxed preview with search/selection/launch, but live launch actions only spawned commands and did not display an overlay inside the WM loop.

## After state

- Failing tests: none in `cargo test --workspace --lib --bins --tests -- --test-threads=2`.
- Relevant metrics: +2 launcher-overlay unit tests covering query editing/Enter/Escape handling and case-insensitive candidate filtering.
- Context: `--launcher-overlay` makes runtime launch actions open an in-session boxed overlay. The overlay starts from `KITTUI_WM_LAUNCH_QUERY`, supports typed filtering, Backspace, Up/Down selection, Enter launch, and Escape close. It is rendered over the live terminal without changing raw-mode RAII handling.

## Diff summary

- Code/content commits: pending final squash SHA from reintegration receipt
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kitwm.rs`, `crates/kittui-cli/src/session.rs`, `.cacophony/agent/ms-mac-kittui-kittui-dev/summary/pending/screenshots/bd-190a5c-live-launcher-overlay.png`
- Tests: +2 / -0 / flipped 0
- Behavioural delta: launch actions can now display and drive a live in-session launcher menu rather than only spawning a fixed command.

## Embedded artefacts

- `screenshots/bd-190a5c-live-launcher-overlay.png` — tendril proof showing a real `kitwm --backend fake --launch-on-f12 --launcher-overlay --launcher-query echo` session with the launcher overlay opened.

## Operator-takeaway

The launcher is now inside the WM loop. The remaining work is to polish the trigger to Mod+Return/Ctrl-A Enter consistently across terminals and wire this overlay into real app selection against XQuartz-hosted apps.
