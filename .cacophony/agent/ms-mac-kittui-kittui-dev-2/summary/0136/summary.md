# Session summary — SurfaceSpec placement contract

## Goal

Provide SDK apps with a single typed placement/readiness contract for a `SurfaceSpec`, bundling native readiness, composition plane, z-index, SDK entry, and kittui rendering entry.

## Bead(s)

- `bd-51b7c6` — kittwm-sdk: add SurfaceSpec placement contract

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime, browser, bar, or live session changes.

## Before state

- `SurfaceSpec` exposed native readiness, plane, and z-index as separate helpers.
- Apps that needed a complete placement contract had to combine multiple helpers and surface metadata manually.

## After state

- Added `SurfacePlacementContract` with:
  - `surface`
  - `surface_kind`
  - `sdk_entry`
  - `sdk_backed`
  - `kitty_graphics_native`
  - `native_ready`
  - `composition_plane`
  - `z_index`
  - `kittui_entry`
- Added `SurfaceSpec::placement_contract()`.
- Terminal/browser specs return populated placement contracts.
- `SurfaceKind::Other` returns `None`.
- Added serialization roundtrip coverage in SDK tests.

## Diff summary

- Code/content commits: `cab65d8` (`bd-51b7c6: add SurfaceSpec placement contract`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk surface_spec_native_readiness_uses_architecture_contract -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK apps can now call `SurfaceSpec::placement_contract()` for one authoritative bundle of placement/readiness metadata.
