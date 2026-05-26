# Session summary — Placement coverage role breakdown

## Goal

Add a serializable role-count breakdown to `SurfacePlacementCoverage` so SDK diagnostics can emit app/decoration/overlay placement counts as data instead of hard-coded fields.

## Bead(s)

- `bd-792c1e` — kittwm-sdk: add placement coverage role breakdown

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only.
- Avoided kittwm.rs inspection/session, helper binaries, Runtime/browser/bar internals.

## Before state

- `SurfacePlacementCoverage` had typed role count helpers.
- Diagnostics wanting a JSON-friendly role breakdown still had to construct their own role/count list.

## After state

- Added `SurfacePlacementRoleCoverage` with:
  - `role`
  - `composition_plane`
  - `count`
- Added `SurfacePlacementCoverage::role_breakdown()`.
- Tests assert current role breakdown:
  - `AppSurface` / `app-surfaces` → `2`
  - `Decoration` / `decorations` → `1`
  - `Overlay` / `overlays` → `0`

## Diff summary

- Code/content commit: `e5683b2` (`bd-792c1e: add placement coverage role breakdown`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `rustfmt crates/kittwm-sdk/src/lib.rs`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK diagnostics can now serialize placement coverage by typed composition role without duplicating role/plane/count mapping logic.
