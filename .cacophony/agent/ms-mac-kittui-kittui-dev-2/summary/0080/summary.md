# Session summary — App frame footprint contract

## Goal

Complete bd-1e63f9 by making the terminal/app frame footprint explicit and tested against the containing split app bounds.

## Bead(s)

- `bd-1e63f9` — kittwm: make terminal/app surfaces fit their containing split

## Before state

- Failing tests: none known for this slice.
- Relevant metrics: `bd-a32779` had just inset app bounds inside pane chrome, and the render loop placed raw frames using an inline `CellRect::new(layout.app_x, layout.app_y, layout.app_cols, layout.app_rows)`. There was no named contract/test that app frame placement and PTY app rows/cols use the same split app geometry.
- Context: builds on the split-overlap fix; no additional chrome or input changes.

## After state

- Failing tests: none observed; targeted validation passed.
- Relevant metrics: added `native_app_frame_footprint(layout)` and uses it for raw frame placement. Added `native_app_frame_footprint_matches_split_app_bounds`, asserting the frame footprint equals layout app bounds, is inset from pane chrome, is narrower than full pane cols, and stays disjoint from the neighboring split's app frame.
- Context: changed only `crates/kittui-cli/src/session.rs`; existing `resize_native_panes` already resizes PTYs to `layout.app_cols/app_rows`.

## Diff summary

- Code/content commits: `724483d` (`bd-1e63f9: enforce app frame footprint`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Tests: added explicit app-frame footprint/split-boundary contract coverage.
- Behavioural delta: raw frame placement now uses a named app-footprint helper tied to split app bounds, documenting/enforcing that terminal/app content fits inside its containing split.
- Validation: `cargo test -p kittui-cli native_app_frame_footprint_matches_split_app_bounds -- --test-threads=1`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

The app surface sizing contract is now explicit: PTY/app frames are placed and resized to the same app bounds inside pane chrome, not a disconnected full-pane/fixed-square area.
