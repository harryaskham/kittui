# Session summary — in-session picker overlay

## Goal

Burn down the canonical picker/operator UI bead by adding an in-session picker overlay and keymap action, while merging the narrower picker proof/docs beads into that canonical item.

## Bead(s)

- `bd-6a36e1` — Operator UI: live backend picker + window picker in kittui-wm
- Duplicates merged into it: `bd-ed1c33`, `bd-97c7a3`, `bd-9c76ac`

## Before state

- Failing tests: none known.
- Relevant metrics: launcher overlay existed, but there was no separate picker action/panel for backend/window surfacing. The requested picker work was spread across several open beads.
- Context: the default keymap used `Ctrl-A Space` for no picker action yet.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli keymap::tests::default_keymap_has_requested_chords -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
- Context: keymap action `picker.open` is bound to `Ctrl-A Space`. The session loop now has a `PickerOverlay` with keyboard navigation, Enter selection, Esc close, and a floating kittui-style panel listing native backends/current window plus Quartz windows when built with the Quartz feature.

## Diff summary

- Code/content commits: `d7f5ebc`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/keymap.rs`, `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/src/session.rs`
- Tests: keymap regression rerun; build verified.
- Behavioural delta: kittwm now has a first in-session picker overlay path rather than only the app launcher.

## Operator-takeaway

The picker is still a lightweight first cut — it does not yet attach/pin selected windows — but the UI/action surface is in place and can be iterated from inside the live session.
