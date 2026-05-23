# Session summary — SurfaceCapabilities accessor helpers

## Goal

Add stable convenience methods for inspecting extended `SurfaceCapabilities` metadata.

## Bead(s)

- `bd-d92b13` — kittui-wm: SurfaceCapabilities accessor helpers

## Before state

- Failing tests: none known.
- Relevant context: extended capability booleans existed (`exact_byte_input`, `focus_events`, `surface_events`), but callers still had to read fields directly.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-wm surface_capability_accessors -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `can_capture()`.
  - Added `can_send_text()`.
  - Added `can_send_bytes()`.
  - Added `can_receive_focus_events()`.
  - Added `can_emit_surface_events()`.
  - Added `can_resize()`.
  - Added `has_title()`.
  - Added `can_restore()`.
  - Added tests covering PTY metadata and capture-only RGBA metadata.
  - No runtime behavior changed.

## Parallel coordination

- `kittui-dev-2` is assigned to `bd-56f419` for docs-only extended capability metadata after `bd-fbccff` landed.

## Diff summary

- Code/content commit: `e2625581`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`

## Operator-takeaway

Downstream runtime/docs/SDK code now has stable method names for checking native surface capabilities without relying on raw field names.
