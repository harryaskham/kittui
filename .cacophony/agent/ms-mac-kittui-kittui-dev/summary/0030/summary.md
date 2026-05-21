# Session summary — kitwm apps JSON output

## Goal

Make the `kitwm apps` launcher-candidate source machine-readable so the future floating launcher UI and daemon attach tooling can consume the same candidate list without scraping text output.

## Bead(s)

- `bd-13d80f` — kitwm apps --json: machine-readable launcher candidate list
- parent/decomposes: `bd-8d64b1` — App launcher inside kittui-wm

## Before state

- Failing tests: none; workspace tests were green at wake start.
- Context: `kitwm apps` printed a human-readable list of default launcher resolution, PATH commands, and macOS apps, but there was no structured mode.

## After state

- Failing tests: none in `cargo test --workspace --lib --bins --tests -- --test-threads=2`.
- Relevant metrics: +1 `kitwm_smoke` test; smoke count is now 18 tests.
- Context: `kitwm apps --json --limit N` emits one JSON-ish object with `default_command`, `default_resolved`, `path_commands`, and `macos_apps`. `default_resolved` is a proper JSON string or `null`.

## Diff summary

- Code/content commits: pending final squash SHA from reintegration receipt
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kitwm.rs`, `crates/kittui-cli/tests/kitwm_smoke.rs`, `.cacophony/agent/ms-mac-kittui-kittui-dev/summary/pending/screenshots/bd-13d80f-kitwm-apps-json.png`
- Tests: +1 / -0 / flipped 0
- Behavioural delta: launcher candidates can now be consumed programmatically by a menu or daemon client.

## Embedded artefacts

- `screenshots/bd-13d80f-kitwm-apps-json.png` — tmux/tendril proof showing JSON mode and text mode side-by-side.

## Operator-takeaway

The launcher data source is now both human-readable and scriptable, reducing the remaining floating launcher work to UI/filter/selection wiring.
