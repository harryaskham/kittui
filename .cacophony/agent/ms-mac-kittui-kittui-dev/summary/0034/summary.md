# Session summary — daemon launcher first-match verbs

## Goal

Expose the launcher selection/execution primitive through the running `kitwm --serve` daemon, so attach clients can search, select, and launch without shelling out to local `kitwm apps`.

## Bead(s)

- `bd-accc3c` — kitwm daemon APPS_FIRST/APPS_LAUNCH_FIRST verbs
- parent/decomposes: `bd-8d64b1` — App launcher inside kittui-wm

## Before state

- Failing tests: none; workspace tests were green at wake start.
- Context: the CLI had `kitwm apps --first` / `--launch-first`, and the daemon exposed `APPS` / `APPS_JSON`, but the daemon could not select or launch the first filtered candidate.

## After state

- Failing tests: none in `cargo test --workspace --lib --bins --tests -- --test-threads=2`.
- Relevant metrics: +1 `kitwm_smoke` test for daemon `APPS_FIRST` and `APPS_LAUNCH_FIRST`.
- Context: `kitwm --attach -c 'APPS_FIRST echo'` returns the first matching candidate; `kitwm --attach -c 'APPS_LAUNCH_FIRST echo'` launches it and reports pid/kind/name.

## Diff summary

- Code/content commits: pending final squash SHA from reintegration receipt
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/tests/kitwm_smoke.rs`, `.cacophony/agent/ms-mac-kittui-kittui-dev/summary/pending/screenshots/bd-accc3c-daemon-apps-first.png`
- Tests: +1 / -0 / flipped 0
- Behavioural delta: attach clients can now perform search+select+launch against a live daemon via one-shot commands.

## Embedded artefacts

- `screenshots/bd-accc3c-daemon-apps-first.png` — tendril display capture showing `APPS_FIRST echo` and `APPS_LAUNCH_FIRST echo` against a live daemon socket.

## Operator-takeaway

The daemon now exposes the complete launcher primitive chain: list, JSON list, first match, and launch first match. The remaining launcher work can focus on UI/input flow.
