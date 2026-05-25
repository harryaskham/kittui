# Session summary — Split overlap/layout inset fix

## Goal

Complete bd-a32779 by preventing app frames from occupying the same cells as graphical pane chrome and by making split geometry explicitly disjoint.

## Bead(s)

- `bd-a32779` — kittwm: fix split overlap and host-resize logical reflow

## Before state

- Failing tests: none known for this slice.
- Relevant metrics: pane layouts reserved a title row but app frames still used the full pane width and all rows below title. With graphical borders/gutters layered over app frames, the app surface could visually overlap pane chrome. Column/row split tests did not assert app regions were disjoint from neighboring pane chrome.
- Context: scoped to native pane layout sizing; host resize already recomputes layouts and resizes panes in the main loop, but this patch makes the recomputed app surface dimensions exclude graphical chrome.

## After state

- Failing tests: none observed; targeted validation passed.
- Relevant metrics: introduced pane chrome sizing constants and inset app surfaces by 1 cell left/right plus title row and bottom border. App cols/rows are still clamped to at least 1 for tiny panes. Updated split/weight/top-bar layout tests to assert app regions exclude borders and do not overlap neighboring panes.
- Context: changed only `crates/kittui-cli/src/session.rs`; PTY resize now receives the inset app rows/cols through existing `resize_native_panes_for_layout`.

## Diff summary

- Code/content commits: `23ecc51` (`bd-a32779: inset app frames inside pane chrome`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Tests: updated `native_pane_layouts_*` and `native_layouts_reserve_top_bar_chrome_band` expectations for chrome-inset app geometry.
- Behavioural delta: split app frames are smaller than the full pane chrome footprint and occupy disjoint app regions, reducing visual overlap/flicker between app content and graphical pane borders/gutters.
- Validation: `cargo test -p kittui-cli native_pane_layouts -- --test-threads=1`; `cargo test -p kittui-cli native_layouts_reserve_top_bar_chrome_band -- --test-threads=1`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

Pane app surfaces now live inside the graphical pane chrome instead of underneath it; split app regions are tested as disjoint and are recomputed through the existing resize path.
