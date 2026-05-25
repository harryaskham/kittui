# Session summary — Mouse routing across chrome and app bounds

## Goal

Complete bd-e50701 by distinguishing top-bar, pane chrome, and pane app-area mouse hits so app PTYs only receive app-local coordinates.

## Bead(s)

- `bd-e50701` — kittwm: mouse coordinate routing across chrome, top bar, and app area

## Before state

- Failing tests: none known for this slice.
- Relevant metrics: app-area hit testing only used `layout.app_*` bounds, which correctly avoided forwarding chrome clicks, but chrome clicks were swallowed without focusing the pane and there was no direct coverage for top-bar/title/border/app coordinate separation.
- Context: builds on the app-frame inset/split geometry work.

## After state

- Failing tests: none observed; targeted validation passed.
- Relevant metrics: added `native_pane_chrome_at_host_cell` over outer pane chrome footprint. Mouse routing still only sends SGR payloads for app-area hits, but a focus-worthy click in pane chrome now focuses that pane and sets redraw without sending app coordinates. Top-bar/outside hits remain swallowed and do not target an app. Added coverage for top bar, title row, border gutter, app area local coordinate translation, and second split app hit.
- Context: changed only `crates/kittui-cli/src/session.rs`.

## Diff summary

- Code/content commits: `bd00454` (`bd-e50701: separate mouse chrome and app hits`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Tests: added `native_mouse_hit_testing_separates_top_bar_chrome_and_app_area`.
- Behavioural delta: pane chrome clicks can focus panes without corrupting app input; app payload coordinates are only generated for inset app-area cells.
- Validation: `cargo test -p kittui-cli native_mouse_hit_testing_separates_top_bar_chrome_and_app_area -- --test-threads=1`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

Mouse routing now understands the graphical shell layers: top bar is non-app, chrome is focus-only, and app-area events translate to pane-local coordinates.
