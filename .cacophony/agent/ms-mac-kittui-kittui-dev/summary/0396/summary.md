# Session summary — kittui-affordances control gallery

## Goal

Add a first-party gallery/example for the new kittui-affordances controls so docs, screenshots, smoke tests, and external renderer workflows have a compact palette source.

## Bead(s)

- `bd-bf8d91` — kittui-affordances: add first-party control gallery example

## Before state

- Failing tests: none known.
- Relevant context: first-party control builders existed, but there was no gallery/helper that exercised the complete palette in one reusable place.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-affordances gallery -- --nocapture` passed.
  - `cargo test -p kittui-affordances --example control_gallery -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `crates/kittui-affordances/src/gallery.rs`.
  - Added `control_gallery()` returning controls for button, checkbox, radio group, text input, text area, select/list, menu, slider, progress, tabs, and split pane.
  - Added `control_gallery_scenes(CellSize)` to lower the palette to primitive kittui scenes.
  - Re-exported gallery helpers from `kittui-affordances`.
  - Added `crates/kittui-affordances/examples/control_gallery.rs` with a small printable summary and example test.
  - Avoided unrelated rustfmt churn in existing components.

## Diff summary

- Code/content commit: `ce0a9510`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-affordances/src/gallery.rs`, `crates/kittui-affordances/src/lib.rs`, `crates/kittui-affordances/examples/control_gallery.rs`
- Behavioural delta: new affordance gallery API/example only.

## Operator-takeaway

The high-level control palette now has a reusable first-party gallery source that exercises every control and lowers to primitive scenes.
