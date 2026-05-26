# Session summary — SurfaceSpec composition plane helpers

## Goal

Let SDK apps derive the kittwm architecture composition plane and z-index for typed surface requests directly from `SurfaceSpec`.

## Bead(s)

- `bd-f4698e` — kittwm-sdk: add SurfaceSpec composition plane helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime, browser, bar, or live session changes.

## Before state

- `SurfaceSpec` could report native readiness and native surface contract.
- Callers still had to fetch the native surface contract and architecture contract separately to derive plane/z-index for a typed surface request.

## After state

- Added `SurfaceSpec::composition_plane()`.
- Added `SurfaceSpec::z_index()`.
- Terminal and browser specs resolve to `app-surfaces` / z-index `0`.
- `SurfaceKind::Other` specs resolve to no plane or z-index.
- Strengthened SDK tests for terminal/browser/other mappings.

## Diff summary

- Code/content commits: `b95b370` (`bd-f4698e: add SurfaceSpec composition plane helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk surface_spec_native_readiness_uses_architecture_contract -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK apps can now call `spec.composition_plane()` and `spec.z_index()` to choose the right kitty graphics placement layer without hard-coded WM z-index values.
