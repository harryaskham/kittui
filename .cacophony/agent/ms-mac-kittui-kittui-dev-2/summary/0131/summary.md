# Session summary — SurfaceSpec native readiness helpers

## Goal

Add SDK convenience methods so apps can ask whether a typed `SurfaceSpec` request is covered by the current first-party SDK + kitty-graphics-native contract without manually constructing/scanning `ArchitectureContract`.

## Bead(s)

- `bd-3a879c` — kittwm-sdk: add SurfaceSpec native readiness helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime/browser, bar, or live session changes.

## Before state

- `ArchitectureContract::native_surface_for_spec(&SurfaceSpec)` could map terminal/browser specs to native surface contracts.
- Callers still had to construct `ArchitectureContract::current()` directly to query a single spec.

## After state

- Added `SurfaceSpec::native_surface_contract()`.
- Added `SurfaceSpec::is_native_ready()`.
- Terminal and browser specs return cloned native surface contracts and are ready.
- `SurfaceKind::Other` specs return no contract and are not ready.
- Added focused SDK tests for terminal/browser/other readiness.

## Diff summary

- Code/content commits: `41144de` (`bd-3a879c: add SurfaceSpec native readiness helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk surface_spec_native_readiness_uses_architecture_contract -- --test-threads=1 --nocapture`
  - `target/debug/deps/kittwm_sdk-bcce546550d1d340 --exact tests::architecture_contract_exposes_wm_boundaries_for_apps --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK apps can now call `spec.is_native_ready()` before spawning to check whether the request is backed by the first-party kitty-graphics-native kittwm contract.
