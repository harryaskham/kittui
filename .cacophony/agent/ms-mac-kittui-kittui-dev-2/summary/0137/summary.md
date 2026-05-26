# Session summary — SurfacePlacementContract role helpers

## Goal

Add typed role/z-order helpers on `SurfacePlacementContract` so SDK apps can reason about placement metadata without string-matching plane names or comparing raw z-index integers.

## Bead(s)

- `bd-979078` — kittwm-sdk: add placement contract role helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime/browser/bar/session changes.

## Before state

- `SurfacePlacementContract` bundled surface placement/readiness metadata.
- Consumers still had to compare `composition_plane` strings or z-index values directly for common role/order checks.

## After state

- Added `SurfacePlacementContract::is_app_surface()`.
- Added `SurfacePlacementContract::is_decoration()`.
- Added `SurfacePlacementContract::is_overlay()`.
- Added `SurfacePlacementContract::is_above(other)`.
- Added `SurfacePlacementContract::is_below(other)`.
- Strengthened SDK tests for terminal/browser app-plane contracts and a synthetic decoration placement.

## Diff summary

- Code/content commits: `3ee18c5` (`bd-979078: add placement contract role helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk surface_spec_native_readiness_uses_architecture_contract -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK consumers can now use semantic placement role/order helpers rather than raw string/z-index comparisons.
