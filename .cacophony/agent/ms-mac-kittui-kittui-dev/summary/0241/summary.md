# Session summary — FFI place_json_at

## Goal

Expose scene-local render with host-controlled terminal placement to FFI users, matching Rust `Runtime::place_at` and CLI `compose --x/--y` capabilities.

## Bead(s)

- `bd-d7ff2a` — kittui-ffi: add place_json_at placement override

## Before state

- Failing tests: none known.
- Relevant gap: non-Rust hosts could only call `kittui_place_json`, which places a scene at its embedded footprint. Moving output required mutating scene JSON and changing scene/cache identity.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-ffi place_json_at -- --nocapture` passed.
  - `cargo test -p kittui-ffi --test abi_snapshot -- --nocapture` passed.
  - `cargo build -p kittui-ffi` passed.
  - `git diff --check` passed.
- Context: added `kittui_place_json_at(runtime, scene_json, x, y, out)`, which uses `Runtime::place_at` with the scene's original cols/rows and caller-supplied x/y. ABI minor bumped to 4 and `kittui.h` updated.

## Diff summary

- Code/content commit: `891bf20`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-ffi/src/lib.rs`, `crates/kittui-ffi/kittui.h`, `crates/kittui-ffi/tests/abi_snapshot.rs`
- Behavioural delta: FFI callers can move scenes without editing their JSON.

## Operator-takeaway

External platform hosts now have the same placement-override primitive as Rust and CLI integrations, making kittui more viable as a cross-language renderer substrate.
