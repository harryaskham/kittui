# Session summary — kittui JSON byte channels

## Goal

Improve kittui as a shell/external-platform renderer by giving JSON-mode callers access to the actual upload/placement/embed byte channels without parsing raw terminal output.

## Bead(s)

- `bd-94dff7` — kittui-cli: expose channelized bytes in JSON output

## Before state

- Failing tests: none known.
- Relevant gap: `kittui --json` exposed byte counts and embed text, but not the actual upload or placement strings. Scripts needing channelized writes had to use non-JSON output and parse/route escapes themselves.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui placement_json -- --nocapture` passed.
  - `cargo test -p kittui-cli --test json_channels -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui -- box -w 2 -h 1 --json --json-bytes | python3 -c '...'` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: new global `--json-bytes` opt-in includes `upload`, `placement`, and `embed` strings alongside `upload_bytes`, `placement_bytes`, and `embed_bytes`. Compact `--json` remains backward compatible and omits `upload`/`placement` strings.

## Diff summary

- Code/content commit: `40495b6`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/json_channels.rs`
- Behavioural delta: scripts can now request machine-readable, channelized terminal output.

## Operator-takeaway

This makes kittui CLI much more practical as a renderer backend for shell scripts and non-Rust hosts: JSON can now carry the exact bytes to write in each terminal channel.
