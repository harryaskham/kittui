# Session summary — deterministic z-order hit testing

## Goal

Make capture-backed kittwm pointer hit-testing deterministic for overlapping windows by respecting render order instead of iterating a HashMap.

## Bead(s)

- `bd-0f6779` — kittwm: make pointer hit-testing deterministic by z-order

## Before state

- Failing tests: none known.
- Relevant gap: `hit_test` claimed to iterate windows in z-order but actually iterated a `HashMap`, so overlapping windows routed pointer focus nondeterministically. This blocked meaningful raise/lower/swap semantics.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm hit_test_uses_last_rendered_window_as_topmost -- --nocapture` passed.
  - `cargo test -p kittui-wm raw_frames_update_hit_test_order -- --nocapture` passed.
  - `cargo test -p kittui-wm pointer_in_downscaled_window_maps_back_to_source_pixels -- --nocapture` passed.
  - `git diff --check` passed.
- Context: `Compositor` now records `placement_order` alongside placements for both Scene and raw-frame composition paths. `hit_test` walks that order in reverse so later-rendered windows are topmost for overlapping cells.

## Diff summary

- Code/content commit: `7ad0e9a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/lib.rs`
- Behavioural delta: overlapping capture-backed windows now route pointer input deterministically to the topmost rendered window.

## Operator-takeaway

This fixes a real WM correctness gap and creates a sound base for future raise/lower/swap actions.
