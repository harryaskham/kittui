# Session summary — SDK composition plane z-index helpers

## Goal

Expose the kittwm architecture contract's compositor planes through typed SDK helper methods so apps and first-party helpers can avoid hard-coding kitty graphics z-index values.

## Bead(s)

- `bd-6e8ea7` — kittwm-sdk: add composition plane z-index helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime, browser, bar, or live session changes.

## Before state

- `ArchitectureContract::current().composition_order` listed app/decorations/overlay planes and z-indexes.
- SDK callers had to manually scan the vector and hard-code plane names.

## After state

- Added `ArchitectureContract::composition_plane(plane)`.
- Added `ArchitectureContract::z_index_for_plane(plane)`.
- Added convenience helpers:
  - `app_surface_z_index()` → `Some(0)`
  - `decoration_z_index()` → `Some(20)`
  - `overlay_z_index()` → `Some(30)`
- Strengthened architecture tests for plane lookup and missing-plane handling.

## Diff summary

- Code/content commits: `826324c` (`bd-6e8ea7: add composition plane z-index helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK/first-party apps can now derive app/chrome/overlay z-indexes from the shared architecture contract instead of baking in magic numbers.
