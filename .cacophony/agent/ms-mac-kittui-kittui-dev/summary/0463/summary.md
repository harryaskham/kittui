# Session summary — NativeSurface extended capability metadata

## Goal

Make `SurfaceCapabilities` advertise the newer `NativeSurface` hooks explicitly instead of relying on the coarse `input` flag.

## Bead(s)

- `bd-fbccff` — kittui-wm: advertise NativeSurface extended capabilities

## Before state

- Failing tests: none known.
- Relevant context: `NativeSurface` now supports exact bytes, focus notifications, and side-effect event draining, but `SurfaceCapabilities` still only exposed coarse capture/input/resize/title/restore booleans.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-wm pty_terminal_advertises_native_surface_metadata -- --nocapture` passed.
  - `cargo test -p kittui-wm kittui_scene_surface_adapts_scene_capture_metadata -- --nocapture` passed.
  - `cargo test -p kittui-wm rgba_frame_surface_validates_updates_and_captures -- --nocapture` passed.
  - `cargo test -p kittui-wm composite_frame_surface_composes_rgba_children -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `SurfaceCapabilities::exact_byte_input`.
  - Added `SurfaceCapabilities::focus_events`.
  - Added `SurfaceCapabilities::surface_events`.
  - Defaults/capture-only helpers keep the new flags false.
  - `PtyTerminalApp` metadata now advertises exact byte input, focus notifications, and surface event draining.
  - Capture-only adapters explicitly test these new flags remain false.
  - No socket/live session behavior changed.

## Parallel coordination

- `kittui-dev-2` confirmed docs bead `bd-56f419` is claimed but waiting for this source bead to land before docs update. They are avoiding native.rs/runtime.

## Diff summary

- Code/content commit: `13a08155`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`

## Operator-takeaway

NativeSurface metadata now distinguishes text input from exact-byte input, focus notifications, and side-effect event draining, allowing downstream docs/SDK/runtime code to reason about adapter support precisely.
