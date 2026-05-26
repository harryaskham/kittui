# Session summary — Placement contract iterators

## Goal

Let SDK apps inspect all first-party app/chrome placement metadata from `ArchitectureContract` without manually mapping native surfaces one by one.

## Bead(s)

- `bd-696cd6` — kittwm-sdk: add placement contract iterators

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime/browser/bar/session changes.

## Before state

- Callers could build a placement contract for one native surface/spec/kind.
- There was no direct way to get the complete placement contract matrix for all first-party native surfaces.

## After state

- Added `ArchitectureContract::placement_contracts()`.
- Added `ArchitectureContract::ready_placement_contracts()`.
- Tests now assert the full matrix:
  - `kittwm-terminal` → `AppSurface`
  - `kittwm-browser` → `AppSurface`
  - `kittwm-bar` → `Decoration`
- Current ready placement contracts equal the full matrix.

## Diff summary

- Code/content commits: `eb9d254` (`bd-696cd6: add placement contract iterators`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK clients can now obtain the full first-party surface placement-contract matrix from the architecture contract in one call.
