# Session summary — deterministic scene hashing

## Goal

Implement `bd-6928ef` so `Scene::id()` hashes a deterministic render-equivalent form instead of raw serde data, preventing semantically identical scenes from missing the cache or producing different kitty image ids.

## Bead(s)

- `bd-6928ef` — Implement deterministic Scene normalization before hashing

## Before state

- Failing tests: none in `kittui-core`; broader wgpu/CLI link validation remains blocked by `bd-b62e3e` (`ld: library not found for -liconv`).
- Relevant metrics: `Scene::id()` hashed `self` directly via serde JSON, so debug labels, empty/zero-opacity layers, zero-width strokes, stop ordering/clamping noise, and subpixel jitter changed scene ids.
- Context: `DESIGN.md` required normalization before hashing, but `crates/kittui-core/src/scene.rs` only tested clone stability and content mutation.

## After state

- Failing tests: none for the touched crate.
- Relevant metrics: `cargo test -p kittui-core` passes 18 tests total (16 unit + 2 integration); `cargo clippy -p kittui-core --all-targets` completes with the pre-existing `Rgba::rgba` self-named-constructor warning.
- Context: `Scene::normalized()` now drops no-op layers, removes debug labels from identity, snaps pixel geometry to 1/64 px, clamps/sorts stops, removes zero/transparent structural no-ops where safe, and collapses single-child normal composites.

## Diff summary

- Code/content commits: `9cc1a0b` (`bd-6928ef: normalize scenes before hashing`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-core/src/scene.rs`
- Tests: added 4 focused scene-id normalization unit tests; existing randomized serde/clone stability integration tests still pass.
- Behavioural delta: Cache keys and kitty ids are now stable across render-equivalent scene descriptions while serde JSON compatibility remains unchanged because normalization is applied at identity time, not during deserialization.

## Operator-takeaway

This closes the core cache-identity gap: harmless scene-description noise no longer fragments kittui's content-addressed cache.
