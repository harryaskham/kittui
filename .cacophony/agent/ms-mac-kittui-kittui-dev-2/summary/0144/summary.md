# Session summary — Placement coverage summary

## Goal

Expose a compact SDK summary of first-party native surface placement coverage so apps/diagnostics can report kittwm kitty-graphics-native readiness without recomputing counts manually.

## Bead(s)

- `bd-8494af` — kittwm-sdk: add placement coverage summary

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only.
- Avoided open kittwm session inspection work (`bd-bbf6a9`), kittwm.rs inspection, helper binaries, Runtime/browser/bar/session changes.

## Before state

- SDK callers could retrieve full/ready/role-filtered/compositor-ordered placement contract lists.
- Callers still had to compute high-level coverage counts and all-ready status themselves.

## After state

- Added `SurfacePlacementCoverage` with:
  - `total_surfaces`
  - `placement_contracts`
  - `ready_placement_contracts`
  - `app_surfaces`
  - `decorations`
  - `overlays`
  - `all_native_surfaces_ready`
  - `all_placement_contracts_ready`
- Added `ArchitectureContract::placement_coverage()`.
- Tests now assert current coverage:
  - total surfaces: `3`
  - app surfaces: `2`
  - decorations: `1`
  - overlays: `0`
  - all native and placement contracts ready: `true`

## Diff summary

- Code/content commit: `62bbf1a` (`bd-8494af: add placement coverage summary`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK consumers now have one compact coverage summary for native placement/readiness across kittwm terminal, browser, and bar surfaces.
