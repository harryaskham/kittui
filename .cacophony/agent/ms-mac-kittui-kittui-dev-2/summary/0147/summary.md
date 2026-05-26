# Session summary — Placement coverage role helpers

## Goal

Add SDK convenience helpers on `SurfacePlacementCoverage` so diagnostics can query app/decoration/overlay coverage using `SurfacePlacementRole` rather than manually matching fields.

## Bead(s)

- `bd-55cc89` — kittwm-sdk: add placement coverage role helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only.
- Avoided kittwm.rs inspection/session, helper binaries, Runtime/browser/bar internals.

## Before state

- `SurfacePlacementCoverage` exposed role counts as fields (`app_surfaces`, `decorations`, `overlays`).
- SDK consumers had to match fields manually when working with typed `SurfacePlacementRole`.

## After state

- Added `SurfacePlacementCoverage::count_for_role(role)`.
- Added `SurfacePlacementCoverage::has_role(role)`.
- Tests assert current first-party coverage counts:
  - `AppSurface` → `2`
  - `Decoration` → `1`
  - `Overlay` → `0`

## Diff summary

- Code/content commit: `09d7386` (`bd-55cc89: add placement coverage role helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `rustfmt crates/kittwm-sdk/src/lib.rs`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK diagnostics can now query placement coverage role counts through the same typed role enum used for contracts and compositor metadata.
