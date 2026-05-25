# Session summary — kittui controls in kittwm help overlay

## Goal

Dogfood first-party `kittui-affordances` controls inside a kittwm in-session panel so controls become part of the polished graphical shell surfaces.

## Bead(s)

- `bd-8783c9` — kittwm: dogfood kittui controls in settings/help panels

## Changes

- The graphical `C-a ?` help overlay now includes actual kittui control scenes:
  - a focused/selected `button` for close/toggle-help,
  - a `text_input` placeholder/filter control.
- Added a no-op action metadata layer labelled `help-overlay-control-action:toggle-help:C-a ?` to make the command path explicit for future routing.
- Control scenes are offset into the help overlay and relabelled with stable `help-overlay-control-*` prefixes.
- Text fallback remains unchanged.
- Existing readable text hints remain while full kittui text/font rendering matures.

## Validation

- `cargo test -p kittui-cli --lib native_help_overlay_builds_graphical_panel_and_key_chips -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittwm` passed.
- `git diff --check` passed.
