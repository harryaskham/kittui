# Session summary — KittuiSceneSurface adapter

## Goal

Add a first-class native surface adapter for kittui scenes so runtime/composite code can treat scene render artifacts like other capture surfaces.

## Bead(s)

- `bd-84ad27` — kittui-wm: add KittuiSceneSurface adapter

## Before state

- Failing tests: none known.
- Relevant context: kittui scenes were renderable via the CPU renderer but were not exposed through the `NativeSurface` abstraction used by PTY/browser/X/Quartz adapters.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-wm kittui_scene_surface -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `KittuiSceneSurface` in `kittui_wm::native`.
  - It wraps a `kittui::Scene` with stable id/title metadata.
  - It advertises `SurfaceKind::KittuiScene` and capture-only capabilities.
  - `capture_surface()` renders the scene through `kittui_render_cpu::render_still` and returns a PNG `NativeFrame` plus metadata.
  - Resize/input return explicit unsupported errors rather than silently mutating scene geometry.
  - No live kittwm session default wiring changed.

## Parallel coordination

- `kittui-dev-2` landed `bd-e7240d` at `dd9852f` and closed it; PTY shell resolution now prefers KITTWM_PTY_SHELL, SHELL, PATH sh/bash, /bin/sh, then bare sh.
- `kittui-dev-2` still has `bd-d582b7` for SDK/docs typed `PaneFramePresented` event.

## Diff summary

- Code/content commit: `52a1ce56`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`

## Operator-takeaway

Scene/composite runtime adapter work now has a concrete kittui scene surface building block without disturbing live kittwm defaults.
