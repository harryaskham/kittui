# Session summary — machine-readable kittwm shortcut catalog

## Goal

Expose native first-launch shortcuts as structured JSON while preserving the existing text shortcut list and in-session `C-a ?` overlay.

## Bead(s)

- `bd-605ca4` — kittwm: machine-readable shortcut catalog

## Before state

- Failing tests: none known.
- Relevant context: `kittwm shortcuts` / `--shortcuts` and `C-a ?` shared a text list, but clients had no structured shortcut catalog.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib shortcuts -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib native_spawn_queue_serves_shortcuts_json -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm shortcuts_json_command_uses_native_shortcut_catalog -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added structured `NativeShortcut` entries and `render_native_shortcuts_json()`.
  - Added `kittwm shortcuts-json` and `kittwm --shortcuts-json` cooked-mode local output.
  - Added socket `SHORTCUTS_JSON` plus HELP/HELP_JSON/error-help catalog entries.
  - Text shortcut output and live keybindings are unchanged.

## Parallel coordination

- Assigned `bd-736c85` to `kittui-dev-2` as docs-only follow-up after this source bead lands.
- `bd-be8304` docs-only SDK chrome helper follow-up remains with `kittui-dev-2`.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/shortcuts.rs`
  - `crates/kittui-cli/src/bin/kittwm.rs`
  - `crates/kittui-cli/src/daemon.rs`

## Operator-takeaway

Automation and docs tooling can now inspect first-launch shortcuts with `kittwm --shortcuts-json` or socket `SHORTCUTS_JSON`, while users keep the same text list and `C-a ?` overlay.
