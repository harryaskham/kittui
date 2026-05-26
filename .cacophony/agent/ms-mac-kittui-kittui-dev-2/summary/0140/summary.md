# Session summary — Native surface placement contract helpers

## Goal

Allow SDK apps to build `SurfacePlacementContract` directly from native surface entries in `ArchitectureContract`, including chrome surfaces such as `kittwm-bar` that are not spawned through `SurfaceSpec`.

## Bead(s)

- `bd-e40394` — kittwm-sdk: add native surface placement contract helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime/browser/bar/session changes.

## Before state

- `SurfaceSpec::placement_contract()` could build placement contracts for terminal/browser surface requests.
- Chrome/native surface entries in `ArchitectureContract` did not have direct placement-contract builders.
- The placement-contract construction logic lived inline in `SurfaceSpec`.

## After state

- Added `SurfacePlacementContract::from_native_surface(surface, contract)`.
- Refactored `SurfaceSpec::placement_contract()` to use the architecture helper.
- Added `ArchitectureContract::placement_contract_for_surface(name)`.
- Added `ArchitectureContract::placement_contract_for_kind(kind)`.
- Added `ArchitectureContract::placement_contract_for_spec(spec)`.
- Tests now cover `kittwm-bar` chrome placement contracts, browser kind placement contracts, and missing lookups.

## Diff summary

- Code/content commits: `5f5ace1` (`bd-e40394: add native surface placement contract helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk surface_spec_native_readiness_uses_architecture_contract -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK clients can now derive placement contracts for both spawnable app surfaces and chrome/native surfaces from the same architecture contract.
