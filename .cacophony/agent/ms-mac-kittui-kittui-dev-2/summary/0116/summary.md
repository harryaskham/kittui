# Session summary — Native app/chrome z-plane and tiling invariants

## Goal

Address the user-reported kittwm fundamentals: panes/chrome fighting for the same z-index and split panes drawing over each other. Keep this slice scoped to core placement/z-plane and deterministic tiling invariants, coordinated with lead to avoid broader renderer overlap.

## Bead(s)

- `bd-add568` — kittwm: fix core tiling non-overlap invariants

## Coordination

- Sent direct coordination note to `ms-mac:kittui:ms-mac-kittui-kittui-dev` before proceeding.
- Scoped this change to runtime placement options plus kittwm native session layout/z-plane invariants.
- Avoided broader renderer/browser refactors in this slice.

## Before state

- Runtime placement helpers always used default kitty placement options, so hosts could not assign compositor z-planes for app frames vs chrome.
- Native kittwm app frames and kittui chrome were both placed at default z-index, causing flicker/fighting.
- Weighted layout used per-pane absolute shares that could leave uneven/ambiguous residual allocation for 3+ panes.

## After state

- Added runtime APIs:
  - `Runtime::place_at_with_options`
  - `Runtime::place_raw_frame_with_options`
  - `Runtime::place_uploaded_image_with_options`
- Native kittwm now places app frames on `NATIVE_APP_Z_INDEX = 0` and chrome on `NATIVE_CHROME_Z_INDEX = 20`.
- Reworked `native_weighted_spans` to allocate sequentially from remaining span/remaining weight while reserving minimum spans for later panes.
- Added deterministic three-pane weighted layout test across rows/columns to assert outer bounds exactly tile the available area and app bounds do not overlap.
- Reverted unrelated rustfmt-only churn in `crates/kittui/src/scene.rs`.

## Diff summary

- Code/content commits: `ccb8077` (`bd-add568: separate native app and chrome planes`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui/src/lib.rs`
  - `crates/kittui-cli/src/session.rs`
- Validation:
  - `cargo test -p kittui placement_options_allow_hosts_to_assign_z_planes -- --test-threads=1`
  - `cargo test -p kittui-cli native_pane_layouts_keep_three_weighted_panes_disjoint -- --test-threads=1`
  - `cargo test -p kittui-cli native_pane_layouts_split_columns_and_reserve_title_rows -- --test-threads=1`
  - `cargo test -p kittui-cli native_pane_layouts_split_rows_and_reserve_each_title_row -- --test-threads=1`
  - `cargo check -p kittui-cli`
  - `git diff --check`

## Operator-takeaway

This makes app surfaces and shell chrome distinct compositor planes and tightens the tiling math for multi-pane splits, reducing the chrome/app flicker and pane overlap failure mode the user reported.
