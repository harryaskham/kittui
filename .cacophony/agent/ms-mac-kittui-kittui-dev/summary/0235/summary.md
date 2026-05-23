# Session summary — kittui image stdin bytes

## Goal

Improve kittui as a shell/external-platform renderer by allowing `kittui image` to consume generated/transformed image bytes from stdin rather than requiring a temporary path.

## Bead(s)

- `bd-449135` — kittui-cli: support stdin image bytes

## Before state

- Failing tests: none known.
- Relevant gap: `kittui image --src` only accepted filesystem paths, so pipelines had to write image data to a temp file before kittui could render/place it.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --test image_stdin -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: `kittui image --src -` reads bytes from stdin and uses `ImageRef::Bytes`; normal path-based `--src PATH` continues to use `ImageRef::Path`.

## Diff summary

- Code/content commit: `0f214a0`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/image_stdin.rs`, `README.md`, `DESIGN.md`
- Behavioural delta: image rendering now works in shell pipelines with stdin image bytes.

## Operator-takeaway

This closes another CLI substrate gap: scripts can now pipe PNG/JPEG bytes directly into kittui without staging files.
