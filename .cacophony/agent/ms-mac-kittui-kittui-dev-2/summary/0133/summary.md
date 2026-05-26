# Session summary — Native surface to composition plane mapping

## Goal

Let SDK apps derive composition plane/z-index for first-party native surface contracts from the shared kittwm architecture contract instead of hard-coding placement layers.

## Bead(s)

- `bd-9a38b4` — kittwm-sdk: map native surfaces to composition planes

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime, browser, bar, or live session changes.

## Before state

- `ArchitectureContract` exposed composition planes and z-index helpers.
- `NativeSurfaceContract` entries did not tell callers which composition plane they belong to.

## After state

- Added `NativeSurfaceContract::composition_plane()`.
  - `terminal` / `browser` → `app-surfaces`
  - `chrome` → `decorations`
  - unknown/future kinds → `None`
- Added `NativeSurfaceContract::z_index(&ArchitectureContract)`.
- Strengthened architecture tests so browser resolves to app z-index `0` and bar/chrome resolves to decoration z-index `20`.

## Diff summary

- Code/content commits: `2baae29` (`bd-9a38b4: map native surfaces to composition planes`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK/native surface metadata now carries enough information for apps to choose the correct app/chrome placement plane through the architecture contract.
