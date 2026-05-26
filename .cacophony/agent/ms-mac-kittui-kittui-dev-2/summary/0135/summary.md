# Session summary — Composition plane ordering helpers

## Goal

Expose kittwm architecture contract composition ordering through typed SDK helpers so apps/tests can reason about app/chrome/overlay layering without hard-coded z-index comparisons.

## Bead(s)

- `bd-5cb43f` — kittwm-sdk: add composition plane ordering helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime/browser/bar/session changes.

## Before state

- `ArchitectureContract` exposed plane names and z-indexes.
- SDK callers had to manually compare z-index integers to reason about order.

## After state

- Added `CompositionPlane::is_above`.
- Added `CompositionPlane::is_below`.
- Added `ArchitectureContract::ordered_plane_names()`.
- Added `ArchitectureContract::plane_is_above(upper, lower)`.
- Strengthened tests to assert app < decorations < overlays and missing-plane behavior.

## Diff summary

- Code/content commits: `1f54366` (`bd-5cb43f: add composition plane ordering helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK consumers can now ask whether one WM composition plane is above another and iterate the contract plane order directly.
