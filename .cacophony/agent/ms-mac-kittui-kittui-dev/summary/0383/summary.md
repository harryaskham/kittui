# Session summary — dirty-frame metrics in native status

## Goal

Expose dirty-frame metrics for native kittwm raw-frame surfaces so renderer behavior can be observed without enabling partial/overlay updates.

## Bead(s)

- `bd-1a778c` — kittwm: expose dirty-frame metrics in native status

## Before state

- Failing tests: none known.
- Relevant context: `bd-889f33` added opt-in `KITTWM_DIRTY_FRAMES=skip-unchanged` behavior, but dirty tile counts/fractions were not visible through native pane status.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_dirty_frame_policy_skips_only_identical_frames_when_enabled -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_pane_statuses_include_dirty_frame_metrics -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added serializable `NativeDirtyFrameStatus` with `changed_tiles`, `total_tiles`, `changed_fraction`, and `skipped_upload`.
  - `NativePaneStatus` now optionally includes `dirty_frame` metadata.
  - Native dirty policy now returns both upload decision and metrics.
  - Native panes retain latest dirty-frame metrics and publish them through `PANES_JSON`/`STATUS_JSON` detail.
  - No rendering behavior changed beyond already-landed opt-in skip-unchanged mode.

## Diff summary

- Code/content commit: `fd9e4e47`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/session.rs`
- Behavioural delta: status JSON can now include dirty-frame metrics when native graphics frames have been diffed.

## Operator-takeaway

Dirty-frame measurement is now observable. This enables future adaptive renderer policy and makes `KITTWM_DIRTY_FRAMES=skip-unchanged` debuggable through the existing native status surfaces.
