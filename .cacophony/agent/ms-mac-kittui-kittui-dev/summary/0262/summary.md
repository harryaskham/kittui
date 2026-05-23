# Session summary — kittui batch placement origin API

## Goal

Promote batch-origin placement from a CLI-only convenience into the core Rust renderer API so external hosts and bindings can place reusable scene groups without mutating each scene footprint.

## Bead(s)

- `bd-e17a4e` — kittui: add batch placement origin API

## Before state

- Failing tests: none known.
- Relevant gap: `kittui compose` could move scene arrays as a group, but `Runtime` only exposed `place_batch(&[Scene])` at scene-embedded coordinates. Non-CLI hosts had to clone/rewrite scene footprints themselves.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui place_batch_at_origin -- --nocapture` passed.
  - `cargo test -p kittui-cli --test compose_batch -- --nocapture` passed.
  - `cargo test -p kittui-cli --test compose_at -- --nocapture` passed.
  - `git diff --check` passed.
- Context: `Runtime::place_batch_at_origin(&[Scene], origin_x, origin_y)` maps the batch minimum x/y to a caller-supplied origin while preserving relative offsets. Empty batches return an empty `BatchPlacement`. CLI compose batch origin support now uses the core API instead of mutating cloned scenes.

## Diff summary

- Code/content commit: `5de2719`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`, `crates/kittui-cli/src/main.rs`
- Behavioural delta: core kittui hosts can place a batch at a runtime group origin with one API call.

## Operator-takeaway

This closes a platform-renderer gap: reusable scene groups are now a first-class Rust API concept, not just a CLI behaviour.
