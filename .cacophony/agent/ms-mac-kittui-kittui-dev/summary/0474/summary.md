# Session summary — kittwm shortcut list CLI

## Goal

Expose the same clean first-launch native shortcut list used by `C-a ?` through a non-interactive CLI command.

## Bead(s)

- `bd-e9868c` — kittwm: CLI shortcut list for clean first launch

## Before state

- Failing tests: none known.
- Relevant context: `C-a ?` showed an in-session help overlay, but users had no cooked-mode CLI command to print that list before launching kittwm.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib shortcuts -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib native_shell_terminal_renderer_draws_empty_workspace_top_bar_and_help -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm shortcuts_command_uses_native_shortcut_list -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added shared `crates/kittui-cli/src/shortcuts.rs`.
  - In-session `C-a ?` help now reads from the shared shortcut list.
  - Added `kittwm shortcuts` subcommand and `kittwm --shortcuts` flag.
  - Added help text and focused CLI test.
  - No live input behavior changed.

## Parallel coordination

- `kittui-dev-2` landed `bd-2f0cb4` at `027a886`: kittwm-bar consumes typed chrome reservation status.

## Diff summary

- Code/content commit: `926cf73a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/shortcuts.rs`
  - `crates/kittui-cli/src/lib.rs`
  - `crates/kittui-cli/src/session.rs`
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

Users can now run `kittwm shortcuts` or `kittwm --shortcuts` to see the native first-launch shortcuts without entering raw/fullscreen mode.
