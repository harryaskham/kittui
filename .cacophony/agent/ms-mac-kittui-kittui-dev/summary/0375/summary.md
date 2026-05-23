# Session summary — native surface frame bounds on resize

## Goal

Fix resize/layout flicker where native panes could briefly render at an old/full logical size after layout changes, and clarify that pane resize should resize/crop surfaces rather than implicitly scale them.

## Bead(s)

- `bd-73f654` — kittwm: enforce native surface frame bounds on layout resize

## Before state

- Failing tests: none known.
- User-visible gap: switching between columns and rows tiling could flicker badly because a pane capture might still be full-size while kittwm placed it into a smaller app area. The WM was not explicitly enforcing captured frame bounds before kitty placement.

## After state

- Failing tests: none in targeted checks.
- Validation:
  - `cargo test -p kittui-cli --lib session::native_pane_tests::fit_rgba_frame_to_cells_crops_and_pads_without_scaling -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_shell_terminal_renderer_draws_chrome_and_snapshots -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `fit_rgba_frame_to_cells(...)` in the native session renderer.
  - Before kitty placement, each RGBA capture is cropped/padded to exactly `layout.app_cols * 8` by `layout.app_rows * 16` pixels.
  - Oversized frames are cropped; undersized frames are padded with the terminal background; no scaling/zoom is applied.
  - Existing PTY resize calls remain in place so apps still receive the logical cell dimensions.
  - docs/wm now states the resize contract and notes zoom/scaling is separate future behavior.

## Diff summary

- Code/content commit: `aae0884`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `docs/wm.md`
- Behavioural delta: native kittwm enforces window bounds during layout transitions, reducing visible overflow/flicker from stale-size frames.

## Operator-takeaway

Pane layout resize now both tells surfaces their new logical size and clips their captured frames to the allocated area before rendering, matching the WM responsibility model.
