# Session summary — NativeFrame inspection helpers

## Goal

Add small non-payload inspection helpers for native frame metadata so runtime/composite code can avoid repetitive matches.

## Bead(s)

- `bd-fc7043` — kittui-wm: add NativeFrame inspection helpers

## Before state

- Failing tests: none known.
- Relevant context: callers could inspect `NativeFrame::width()` and `height()`, but had to match manually for format, payload length, RGBA/PNG predicates, and `SurfaceFrame` metadata.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-wm native_frame_and_surface_frame_helpers -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `NativeFrame::size()`.
  - Added `NativeFrame::format()` returning stable lowercase `rgba` / `png` labels.
  - Added `NativeFrame::payload_len()`.
  - Added `NativeFrame::is_rgba()` / `is_png()`.
  - Added `SurfaceFrame::frame_size()`, `format()`, and `payload_len()` delegating to the payload.
  - Helpers are pure metadata conveniences and do not expose additional payload contents.

## Parallel coordination

- `kittui-dev-2` is assigned to `bd-ca24f6`, docs-only adapter status refresh.

## Diff summary

- Code/content commit: `45a3ce45`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`

## Operator-takeaway

Native surface frame metadata is now easier and safer to inspect for future composite/runtime wiring.
