# Session summary — SDK native surface coverage fields

## Goal

Continue improving kittwm toward a clean kitty-graphics-backed WM by making the typed SDK architecture contract explicitly report first-party native surface coverage: SDK-backed status, kitty-graphics-native status, and the kittui/kittwm rendering entry point.

## Bead(s)

- `bd-32bc26` — kittwm-sdk: add native surface coverage fields to architecture contract

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice limited to `kittwm-sdk` contract/test fields; no kittwm-bar implementation, docs/help, Runtime/browser, or live session reservation/control-plane changes.

## Before state

- `ArchitectureContract::current().first_party_native_surfaces` named the surfaces and SDK entry points, but did not explicitly indicate coverage/completeness.
- Consumers could not mechanically tell whether a listed first-party surface was SDK-backed or kitty-graphics-native.

## After state

- Extended `NativeSurfaceContract` with:
  - `sdk_backed: bool`
  - `kitty_graphics_native: bool`
  - `kittui_entry: String`
- Updated built-in coverage for:
  - `kittwm-terminal`: `SurfaceSpec::terminal`, `PtyTerminalApp -> Runtime::place_raw_frame_with_options`
  - `kittwm-browser`: `SurfaceSpec::browser`, `HeadlessBrowserApp -> Runtime::place_png_frame_with_options`
  - `kittwm-bar`: chrome reservation SDK path, `BarModel::scene -> Runtime::place_at_with_options`
- Strengthened SDK tests so all listed first-party native surfaces must be SDK-backed and kitty-graphics-native.

## Diff summary

- Code/content commits: `405b699` (`bd-32bc26: add native surface coverage to SDK contract`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

The SDK architecture contract now doubles as a first-party native surface coverage matrix, making it easier to keep kittwm-terminal, kittwm-browser, and kittwm-bar on SDK + kitty-graphics-native paths.
