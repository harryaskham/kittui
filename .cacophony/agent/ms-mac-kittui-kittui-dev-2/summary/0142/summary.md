# Session summary — Role-filtered placement contracts

## Goal

Let SDK apps inspect first-party placement contracts by typed `SurfacePlacementRole` without string filtering or manual loops.

## Bead(s)

- `bd-9d987d` — kittwm-sdk: add role-filtered placement contracts

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only.
- Avoided kittui-dev active `bd-9f5367` kittwm.rs launcher preview work, plus helper binaries, Runtime/browser/bar/session changes.

## Before state

- `ArchitectureContract::placement_contracts()` and `ready_placement_contracts()` exposed the full native placement matrix.
- Callers still had to manually filter the matrix by role.

## After state

- Added `ArchitectureContract::placement_contracts_for_role(role)`.
- Added `ArchitectureContract::app_surface_placement_contracts()`.
- Added `ArchitectureContract::decoration_placement_contracts()`.
- Added `ArchitectureContract::overlay_placement_contracts()`.
- Tests now assert:
  - app surfaces: `kittwm-terminal`, `kittwm-browser`
  - decorations: `kittwm-bar`
  - overlays: currently empty

## Diff summary

- Code/content commit: `98a7d41` (`bd-9d987d: add role-filtered placement contracts`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK clients can now get app, decoration, or overlay placement contracts directly from the architecture contract.
