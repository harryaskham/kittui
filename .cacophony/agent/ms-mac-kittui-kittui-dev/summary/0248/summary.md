# Session summary — compose scene arrays

## Goal

Improve kittui CLI as a shell/external-platform renderer by allowing `kittui compose` to handle batches of scenes in one invocation instead of requiring one process per scene.

## Bead(s)

- `bd-f47143` — kittui-cli: support compose JSON scene arrays

## Before state

- Failing tests: none known.
- Relevant gap: `kittui compose <path>|-` accepted exactly one Scene JSON document. Hosts generating many scenes had to invoke the CLI repeatedly or write custom Rust/FFI glue.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui batch_json_payload -- --nocapture` passed.
  - `cargo test -p kittui-cli --test compose_batch -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: `compose` now accepts either a single Scene or an array of Scenes. Array input uses `Runtime::place_batch` and returns concatenated upload/placement/embed streams. JSON/dry-run output reports count, image ids, footprints, byte counts, and optional byte channels via `--json-bytes`. `--x/--y` overrides are rejected for batch input with a clear error.

## Diff summary

- Code/content commit: `f36bdb5`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/compose_batch.rs`
- Behavioural delta: shell pipelines can batch-compose scene arrays through one CLI process.

## Operator-takeaway

This removes another process-boundary bottleneck for using kittui as a shell renderer/backend: generated scene arrays can now be rendered/placed as a batch.
