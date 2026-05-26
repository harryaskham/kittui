# Session summary — SDK native surface kind helpers

## Goal

Continue improving the kittwm SDK/kittui-native coverage contract by letting apps query first-party native surface readiness by SDK/control-plane surface kind.

## Bead(s)

- `bd-0c0ae4` — kittwm-sdk: add native surface kind coverage helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept the slice SDK-only; no CLI/docs/bar/session/runtime changes.

## Before state

- SDK callers could query native surface coverage by binary/surface name and iterate ready surfaces.
- There was no helper for looking up coverage by surface kind (`terminal`, `browser`, `chrome`).
- There was no convenience predicate for whether all listed first-party native surfaces are ready.

## After state

- Added `ArchitectureContract::native_surface_by_kind(kind)`.
- Added `ArchitectureContract::native_surfaces_by_kind(kind)`.
- Added `ArchitectureContract::all_native_surfaces_ready()`.
- Strengthened the SDK architecture test to assert browser/chrome kind lookup and full readiness.

## Diff summary

- Code/content commits: `c777031` (`bd-0c0ae4: add native surface kind helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK consumers can now verify native surface readiness by semantic surface kind, not only by first-party binary name.
