# Session summary — Raw frame flicker fix

## Goal

Fix the operator-reported kittwm flicker introduced by deleting the previous raw frame image immediately before reuploading the next frame with the same kitty image id.

## Bead(s)

- `bd-890426` — fix kittwm frame deletion flicker

## Before state

- Failing tests: no automated failure; Harry reported visible runtime flicker where every second frame appeared lost after the old-frame deletion change.
- Relevant metrics: `Runtime::place_raw_frame` emitted `a=d` delete for an already-uploaded image id before uploading the replacement raw RGBA payload. In terminals that process the delete and upload as separate visible operations, this can blank the image between frames.
- Context: this superseded the earlier docs-only wait because the operator asked for a source fix.

## After state

- Failing tests: targeted local validation passed.
- Relevant metrics: same-id raw frame reuploads now emit only the new raw frame upload. The existing WM/session code still deletes images when a window disappears or when a footprint move explicitly needs old placeholder cells cleared, but steady-state frame replacement no longer clears first.
- Context: one focused runtime fix in `kittui::Runtime`; no unrelated docs/runtime surfaces changed.

## Diff summary

- Code/content commits: `00a4e23` (`bd-890426: stop deleting raw frames before reupload`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`
- Tests: +0 / -0 / flipped 1 test expectation from delete-before-reupload to replace-without-delete
- Behavioural delta: raw RGBA frame replacement should stop flickering/blanking every other frame because it no longer deletes the displayed image before uploading its replacement.
- Validation: `cargo test -p kittui raw_frame_reupload_replaces_without_delete -- --test-threads=1`; `cargo test -p kittui-cli native -- --test-threads=1`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

The flicker was consistent with a delete-before-replace frame path: the terminal could briefly show nothing between frames. Reuploading the same image id without a preceding delete restores atomic-ish replacement while preserving explicit deletion for disappearing windows.
