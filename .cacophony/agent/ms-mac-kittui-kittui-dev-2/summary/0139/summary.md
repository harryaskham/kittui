# Session summary — Placement role z-index helpers

## Goal

Let SDK apps resolve composition plane and z-index directly from `SurfacePlacementRole` without converting back to raw plane strings.

## Bead(s)

- `bd-49cd24` — kittwm-sdk: add placement role z-index helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime/browser/bar/session changes.

## Before state

- `SurfacePlacementRole` exposed `plane_name()`.
- `ArchitectureContract` could resolve z-index by string plane name.
- Callers using the role enum still had to call `role.plane_name()` manually.

## After state

- Added `ArchitectureContract::composition_plane_for_role(role)`.
- Added `ArchitectureContract::z_index_for_role(role)`.
- Tests now assert:
  - `AppSurface` → z-index `0`
  - `Decoration` → z-index `20`
  - `Overlay` → z-index `30`
  - `Decoration` resolves to the `decorations` plane

## Diff summary

- Code/content commits: `9a25a25` (`bd-49cd24: add placement role z-index helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK apps can now ask the architecture contract for the z-index of a typed placement role directly.
