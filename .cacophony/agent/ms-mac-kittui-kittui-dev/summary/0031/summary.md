# Session summary — daemon APPS launcher verbs

## Goal

Expose the launcher candidate source through the running `kitwm --serve` daemon so attach clients can retrieve app candidates without shelling out to `kitwm apps` separately.

## Bead(s)

- `bd-f92697` — kitwm daemon APPS verb: expose launcher candidates through --attach
- parent/decomposes: `bd-8d64b1` — App launcher inside kittui-wm

## Before state

- Failing tests: none; workspace tests were green at wake start.
- Context: `kitwm apps` and `kitwm apps --json` exposed launch candidates from the standalone CLI, while the daemon protocol only exposed status/windows/displays.

## After state

- Failing tests: none in `cargo test --workspace --lib --bins --tests -- --test-threads=2`.
- Relevant metrics: +1 `kitwm_smoke` test; smoke count is now 19 tests.
- Context: `kitwm --attach -c APPS` returns a text launcher candidate report from the daemon, and `kitwm --attach -c APPS_JSON` returns the same structured launcher source in JSON form.

## Diff summary

- Code/content commits: pending final squash SHA from reintegration receipt
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/tests/kitwm_smoke.rs`, `.cacophony/agent/ms-mac-kittui-kittui-dev/summary/pending/screenshots/bd-f92697-daemon-apps.png`
- Tests: +1 / -0 / flipped 0
- Behavioural delta: launcher candidates are now available from a live daemon via attach one-shot commands, not just local CLI inspection.

## Embedded artefacts

- `screenshots/bd-f92697-daemon-apps.png` — tmux/tendril proof showing `--attach -c APPS` and `--attach -c APPS_JSON` against a live `kitwm --serve` socket.

## Operator-takeaway

The daemon can now serve launcher candidate data to clients, which is a direct bridge toward making the floating launcher menu daemon-backed and scriptable.
