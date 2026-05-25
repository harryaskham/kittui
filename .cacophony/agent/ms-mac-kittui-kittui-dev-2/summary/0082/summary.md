# Session summary — Native graphics pixel-density contract

## Goal

Complete bd-86b1b2 by making the native kittwm graphics cell/pixel contract explicit and tested for both chrome scenes and app raw frames.

## Bead(s)

- `bd-86b1b2` — kittwm: HiDPI and pixel-density contract for graphical WM surfaces

## Before state

- Failing tests: none known for this slice.
- Relevant metrics: raw app frame fitting used `NATIVE_CELL_WIDTH_PX` / `NATIVE_CELL_HEIGHT_PX`, while chrome scene placement used `CellSize::default()` directly. The values matched today but the relationship was implicit, risking scaled/blurred mismatches after resize or future DPI changes.
- Context: scoped to native kittwm graphics sizing; no renderer backend changes.

## After state

- Failing tests: none observed; targeted validation passed.
- Relevant metrics: added `native_cell_size()` derived from `NATIVE_CELL_WIDTH_PX` / `NATIVE_CELL_HEIGHT_PX` and used it for live chrome rendering and showcase scene generation. Added `native_graphics_cell_size_defines_pixel_density_contract`, asserting chrome scene pixel dimensions and raw frame fitting both use the same cell pixel constants.
- Context: changed only `crates/kittui-cli/src/session.rs`.

## Diff summary

- Code/content commits: `3c6f150` (`bd-86b1b2: define native graphics pixel contract`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Tests: added explicit native graphics cell-size / pixel-dimension coverage.
- Behavioural delta: graphical chrome and app frame fitting now share the same explicit native cell size helper, reducing risk of bitmap scaling mismatch.
- Validation: `cargo test -p kittui-cli native_graphics_cell_size_defines_pixel_density_contract -- --test-threads=1`; `cargo test -p kittui-cli native_showcase_scene_json_exports_reviewable_shell_artifact -- --test-threads=1`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

Kittwm now has an explicit pixel-density contract tying scene chrome and app frame raw RGBA dimensions to the same native cell metrics.
