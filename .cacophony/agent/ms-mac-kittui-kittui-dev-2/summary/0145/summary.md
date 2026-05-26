# Session summary — Placement coverage gap helpers

## Goal

Expose SDK helpers that explain native placement coverage gaps from `ArchitectureContract`, instead of only reporting aggregate counts.

## Bead(s)

- `bd-41999d` — kittwm-sdk: add placement coverage gap helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only.
- Avoided kittui-dev kittwm.rs SESSION_JSON inspection work, plus helper binaries, Runtime/browser/bar/session internals.

## Before state

- `SurfacePlacementCoverage` reported aggregate placement/readiness counts.
- Diagnostics could not ask for the concrete not-ready placement contracts or native surfaces missing placement contracts.

## After state

- Added `ArchitectureContract::not_ready_placement_contracts()`.
- Added `ArchitectureContract::missing_placement_contract_surfaces()`.
- Tests assert both are empty for the current complete first-party coverage matrix.

## Diff summary

- Code/content commit: `d5e854f` (`bd-41999d: add placement coverage gap helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `rustfmt crates/kittwm-sdk/src/lib.rs`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Notes

- Initial combined validation hit the harness timeout during shared cargo/rustc contention; reran focused foreground validation successfully after checking process state.

## Operator-takeaway

SDK consumers can now report both placement coverage readiness and concrete gap lists for incomplete native kitty-graphics surface coverage.
