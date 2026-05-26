# Session summary — apply chrome reservations to native layout

## Bead

- `bd-b49f1f` — apply kittwm chrome reservations to native layout

## Coordination

- dev-2 landed browser row clamp/absolute placement and architecture/platform contract work.
- This slice avoids kittwm_browser.rs, kittwm_launch.rs, architecture-json/platform-contract-json, and runtime placement/z-order internals.

## Changes

- Added `NativeSpawnQueue::chrome_reservation()` so the live native PTY session can consume the reservation state introduced by `RESERVE_CHROME_JSON`.
- Updated native session layout helpers to apply chrome reservations:
  - top/bottom row bands reduce tilable rows
  - left/right column bands reduce tilable columns
  - column gaps are inserted between column-split panes
  - row gaps are inserted between row-split panes
- The live native loop now detects reservation changes, logs requested reservation values, resizes panes, and redraws.
- Added regression coverage for top/bottom/side/gap reservation geometry.

## Validation

- `cargo test -p kittui-cli --lib native_layouts_apply_chrome_reservation_bands_and_gaps -- --nocapture`
- `cargo test -p kittui-cli --lib native_chrome_reservation_json_updates_drawable_contract -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
