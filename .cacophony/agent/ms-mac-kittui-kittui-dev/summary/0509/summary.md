# Session summary — graphical toast scene

## Goal

Add a graphical notification/toast surface so kittwm action/error status can be represented as kittui-rendered chrome instead of only footer/log text.

## Bead(s)

- `bd-00cded` — kittwm: graphical notification/toast surface for actions and errors

## Changes

- Added `native_toast_scene(...)` in `crates/kittui-cli/src/session.rs`.
- When graphical shell chrome has footer/status text, `render_native_shell_view_affordance_scenes` now also emits a positioned `toast` scene above the footer.
- Toast scene includes:
  - translucent backdrop,
  - accent rail,
  - highlight layer,
  - metadata text layer label for readable/action parity.
- Error-ish messages use chrome styling; normal messages use neon styling.
- Terminal fallback remains unchanged.

## Validation

- `cargo test -p kittui-cli --lib native_shell_affordance_renderer_builds_kittui_scenes -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittwm` passed.
- `git diff --check` passed.
