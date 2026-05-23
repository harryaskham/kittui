# Session summary — RGBA frame NativeSurface adapter

## Goal

Add a safe capture-only native surface adapter for pre-rendered RGBA frames so renderer/composite code can participate in the same metadata/capture path as PTY/browser/X/Quartz/scene surfaces.

## Bead(s)

- `bd-64dbec` — kittui-wm: add RGBA frame surface adapter

## Before state

- Failing tests: none known.
- Relevant context: raw RGBA frames were supported by kitty transport, but kittui-wm had no simple `NativeSurface` wrapper for producer-owned RGBA frame streams.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-wm rgba_frame_surface -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `RgbaFrameSurface` in `kittui_wm::native`.
  - It wraps a validated RGBA payload with stable id/title metadata.
  - It exposes capture-only capabilities and current frame size.
  - `capture_surface()` returns `NativeFrame::Rgba` plus metadata.
  - `update_frame()` safely replaces dimensions/payload after validating `width * height * 4`.
  - Zero dimensions, overflow, and wrong payload lengths are rejected.
  - Resize/input return explicit unsupported errors.
  - No live kittwm session default wiring changed.

## Parallel coordination

- `kittui-dev-2` remains assigned to `bd-d582b7` for typed SDK `PaneFramePresented` event parsing/docs.

## Diff summary

- Code/content commit: `4655bc1b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`

## Operator-takeaway

The surface runtime now has both kittui scene and raw RGBA frame adapter building blocks for future composite/child-frame runtime wiring.
