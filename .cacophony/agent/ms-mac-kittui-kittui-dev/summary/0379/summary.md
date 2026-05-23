# Session summary — kittui-affordances first-party controls

## Goal

Add first-party form/control affordance builders in `kittui-affordances` to support semantic kittwm surfaces and SDK apps without moving high-level controls into primitive-only `kittui-core`.

## Bead(s)

- `bd-0337ce` — kittui-affordances: add first-party form and control components

## Before state

- Failing tests: none known.
- Relevant context: `kittui-affordances` had document/chrome components, tables, panels, markdown, and inline chrome, but no reusable form/control palette for buttons, checkboxes, radio groups, text inputs, selects/lists, progress, tabs, or split panes.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-affordances --lib -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `crates/kittui-affordances/src/controls.rs`.
  - Added `ControlKind`, `ControlState`, `ControlOption`, and `ControlComponent`.
  - Added builders/free functions for button, checkbox, radio, radio_group, text_input, text_area, select_list, menu, slider, progress, tabs, and split_pane.
  - Controls expose semantic state such as focused, disabled, active, selected, and checked.
  - Controls lower to primitive kittui scenes via `ControlComponent::to_scene(CellSize)` without adding high-level controls to `kittui-core`.
  - Re-exported control builders/types from `kittui-affordances::lib`.
  - Coordinated with kittui-dev-2: they remain on `bd-3aca3c` Xvfb/XQuartz native surface adapter work; this slice touched only affordances files.

## Diff summary

- Code/content commit: `826bc7dc`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-affordances/src/controls.rs`, `crates/kittui-affordances/src/lib.rs`
- Behavioural delta: new affordance API only; no kittwm runtime behavior change yet.

## Operator-takeaway

The semantic UI plan now has first reusable controls in `kittui-affordances`. The next dependent bead is `bd-586ce3`, which can render synthetic semantic component trees through these shared controls instead of duplicating button/radio/input drawing in kittwm.
