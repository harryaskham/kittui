# Session summary — opt-in dirty-grid skip unchanged raw frames

## Goal

Use the dirty-grid helper to skip re-uploading byte-identical raw RGBA frames in native kittwm graphics mode, without enabling risky partial/overlay updates.

## Bead(s)

- `bd-889f33` — kittwm: use dirty grid to skip unchanged raw frame uploads

## Before state

- Failing tests: none known.
- Relevant context: `bd-510a36` added dirty-grid helpers but no runtime use. Native graphics mode still uploaded every raw frame even if pixels were unchanged.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui --lib place_uploaded_image_emits_no_upload -- --nocapture` passed.
  - `cargo test -p kittui --lib raw_frame_reupload_deletes_previous_image_payload -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_dirty_frame_policy_skips_only_identical_frames_when_enabled -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `Runtime::place_uploaded_image(...)`, which emits placement/embed text for an already-uploaded image id with an empty upload payload.
  - Native session now has `NativeDirtyFramePolicy`, enabled only by `KITTWM_DIRTY_FRAMES=skip-unchanged` or `skip_unchanged`.
  - When enabled, first frame and changed frames still call `place_raw_frame` and perform full delete+upload replacement.
  - Identical frames call `place_uploaded_image` so kittwm can redraw/move placement/embed text without re-uploading raw RGBA bytes.
  - Docs updated to mark `skip-unchanged` as landed opt-in behavior.

## Diff summary

- Code/content commit: `6e037f40`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`, `crates/kittui-cli/src/session.rs`, `docs/kittwm-dirty-frame-updates.md`
- Behavioural delta: only opt-in via `KITTWM_DIRTY_FRAMES=skip-unchanged`; default behavior remains full-frame upload.

## Operator-takeaway

There is now a safe dirty-grid runtime step: unchanged-frame upload suppression. No partial kitty overlays or animation/delta tricks are enabled by default.
