# Session summary — SurfaceSpec to native surface contract mapping

## Goal

Strengthen the SDK architecture separation by connecting typed `SurfaceSpec` requests to the native surface coverage matrix in `ArchitectureContract`.

## Bead(s)

- `bd-7ffac2` — kittwm-sdk: map SurfaceSpec to native surface contract

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-dev` / `kittui-dev`.
- Kept this slice SDK-only; no `kittwm.rs` inspection, helper binary, Runtime/browser, bar, or live session changes.

## Before state

- SDK callers could query native surface coverage by name or kind.
- There was no direct helper mapping a typed `SurfaceSpec` (`terminal`, `browser`, `other`) to the contract entry that backs it.

## After state

- Added `ArchitectureContract::native_surface_for_spec(&SurfaceSpec)`.
- Terminal specs map to the `kittwm-terminal` contract.
- Browser specs map to the `kittwm-browser` contract.
- Unsupported `SurfaceKind::Other` specs return `None`.
- Extended SDK architecture tests to cover terminal/browser/other mappings.

## Diff summary

- Code/content commits: `7c83213` (`bd-7ffac2: map SurfaceSpec to native surface contract`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK apps can now ask whether a typed surface request is backed by a first-party SDK + kitty-graphics-native contract before spawning it.
