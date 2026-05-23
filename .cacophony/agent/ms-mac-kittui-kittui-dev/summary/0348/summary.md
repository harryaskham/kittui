# Session summary — native mouse drag motion events

## Goal

Complete the first host-mouse routing path by preserving held-button motion events for TUIs that request button-motion mouse reporting.

## Bead(s)

- `bd-4c988d` — kittwm: support native mouse drag motion events

## Before state

- Failing tests: none known.
- Relevant gap: host mouse routing preserved clicks/scrolls and hover motion, but `MouseMove` events were always mapped to no-button `move` and required all-motion (`?1003`). TUIs often request button-motion (`?1002`) and expect drag reports with the held button encoded as `32 + button`.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli session::native_pane_tests::native_mouse_event_payload_requires_compatible_modes -- --nocapture` passed.
  - `cargo test -p kittui-cli session::native_pane_tests::native_mouse_event_mapping_preserves_drag_buttons -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Host `InputEvent::MouseMove` now maps left/middle/right held buttons to `move-left`, `move-middle`, and `move-right`, while no-button motion remains `move`. Payload encoding emits SGR `32/33/34` drag reports when `mouse_button_motion` or `mouse_all_motion` is enabled; no-button hover remains gated by all-motion. docs/wm now mentions button-drag routing.

## Diff summary

- Code/content commit: `a967b90`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `docs/wm.md`
- Behavioural delta: mouse drag operations in native panes can reach TUIs that enable button-motion mode, without requiring all-motion.

## Operator-takeaway

Native kittwm mouse routing now supports common click-drag TUI interactions in panes that request `?1002`/SGR mouse reporting.
