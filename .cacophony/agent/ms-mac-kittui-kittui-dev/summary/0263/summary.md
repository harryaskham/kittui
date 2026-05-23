# Session summary — FFI/TypeScript batch origin placement

## Goal

Expose the new kittui batch placement origin API to external platforms through the C ABI and TypeScript bindings.

## Bead(s)

- `bd-de703a` — kittui-ffi: expose batch placement origin API

## Before state

- Failing tests: none known.
- Relevant gap: Rust and CLI supported group-origin placement for scene batches, but FFI/TypeScript hosts could only batch-place scenes at embedded coordinates.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-ffi place_many_json_at -- --nocapture` passed.
  - `cargo test -p kittui-ffi --test abi_snapshot -- --nocapture` passed.
  - `cargo build -p kittui-ffi` passed.
  - `npm test --prefix bindings/ts` passed.
  - `git diff --check` passed.
- Context: Added additive C ABI `kittui_place_many_json_at(runtime, scenes_json, x, y, out)` backed by `Runtime::place_batch_at_origin`. Bumped ABI minor to 6 and updated `kittui.h` plus snapshot tests. TypeScript now exposes `Kittui.placeManyAt(scenes, x, y)`.

## Diff summary

- Code/content commit: `cdfe1b0`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-ffi/src/lib.rs`, `crates/kittui-ffi/kittui.h`, `crates/kittui-ffi/tests/abi_snapshot.rs`, `bindings/ts/src/index.js`, `bindings/ts/src/index.d.ts`, `bindings/ts/test/koffi.test.js`
- Behavioural delta: C/TypeScript hosts can place a scene batch at a runtime group origin in one FFI round-trip.

## Operator-takeaway

Batch-origin placement is now available across Rust, CLI, C ABI, and TypeScript, improving kittui's usefulness as a renderer substrate for other platforms.
