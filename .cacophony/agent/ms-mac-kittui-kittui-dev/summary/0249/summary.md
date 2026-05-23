# Session summary — FFI batch placement

## Goal

Expose kittui's batch placement path to non-Rust hosts so external platforms can place many scenes in one FFI call instead of crossing the boundary once per scene.

## Bead(s)

- `bd-f5954c` — kittui-ffi: add batch place_many_json API

## Before state

- Failing tests: none known.
- Relevant gap: Rust had `Runtime::place_batch` and CLI had batched `compose` arrays, but FFI callers only had single-scene `kittui_place_json`.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-ffi place_many -- --nocapture` passed.
  - `cargo test -p kittui-ffi --test abi_snapshot -- --nocapture` passed.
  - `cargo build -p kittui-ffi` passed.
  - `git diff --check` passed.
- Context: added `kittui_place_many_json(runtime, scenes_json, out)`, accepting a JSON array of scenes, using `Runtime::place_batch`, and returning concatenated upload+placement+embed bytes. Non-array input returns `BadScene` and sets `last_error`. ABI minor bumped to 5 and `kittui.h` updated.

## Diff summary

- Code/content commit: `02d4d99`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-ffi/src/lib.rs`, `crates/kittui-ffi/kittui.h`, `crates/kittui-ffi/tests/abi_snapshot.rs`
- Behavioural delta: C/FFI hosts now have a batch scene placement entrypoint.

## Operator-takeaway

This closes another platform gap: non-Rust hosts can use kittui efficiently for multi-scene frames without per-scene FFI overhead.
