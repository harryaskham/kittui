# Session summary — route host mouse reports into native panes

## Goal

Make native kittwm behave more like a terminal WM by routing host SGR mouse reports into the pane under the pointer with pane-local coordinates.

## Bead(s)

- `bd-e287d9` — kittwm: route host mouse reports into native panes

## Before state

- Failing tests: none known.
- Relevant gap: the native terminal loop enables host SGR mouse reporting, but input was processed mostly byte-by-byte and could forward outer-terminal mouse reports raw to the focused PTY. Mouse-aware TUIs could not receive pane-local coordinates naturally, and pointer events were not targeted to the pane under the pointer.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli session::native_pane_tests::native_pane_at_host_cell_translates_to_local_coordinates -- --nocapture` passed.
  - `cargo test -p kittui-cli session::native_pane_tests::native_mouse_event_payload_requires_compatible_modes -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: The native stdin loop now uses `kittui_input::parse` to identify mouse events. Mouse events are consumed by the WM, mapped from host 1-indexed coordinates into the app area of the target pane, translated to pane-local SGR mouse payloads, and injected only when the target pane has compatible mouse reporting modes. Press/scroll events focus the target pane. Existing keyboard/prefix behavior was factored into helpers and preserved. docs/wm now notes host mouse routing.

## Diff summary

- Code/content commit: `8cc6ec4`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `docs/wm.md`
- Behavioural delta: users can interact with mouse-aware native PTY apps in the pane under the pointer instead of forwarding raw outer-terminal mouse bytes to the focused pane.

## Operator-takeaway

Native kittwm now has the core mouse-routing path for terminal TUIs: host SGR mouse reports are targeted, translated, and gated by per-pane mouse mode state.
