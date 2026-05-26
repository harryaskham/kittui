# Session summary — Compositor-ordered placement contracts

## Goal

Let SDK apps and diagnostics inspect first-party placement contracts in kitty/kittui compositor z-index order, without hard-coded sorting or manual z-index handling.

## Bead(s)

- `bd-e47616` — kittwm-sdk: add compositor-ordered placement contracts

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice SDK-only.
- Avoided kittui-dev kittwm.rs help/inspection work, plus helper binaries, Runtime/browser/bar/session changes.

## Before state

- `ArchitectureContract::placement_contracts()` exposed the full native placement matrix.
- Callers needing compositor order still had to sort manually by raw z-index.

## After state

- Added `ArchitectureContract::placement_contracts_in_composition_order()`.
- Added `ArchitectureContract::ready_placement_contracts_in_composition_order()`.
- Tests now assert compositor order:
  - `kittwm-terminal` z-index `0`
  - `kittwm-browser` z-index `0`
  - `kittwm-bar` z-index `20`
- Current ready compositor-ordered contracts equal the full compositor-ordered contract matrix.

## Diff summary

- Code/content commit: `282c85b` (`bd-e47616: add compositor-ordered placement contracts`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK clients can now retrieve the native placement contract matrix already sorted for kitty/kittui composition.
