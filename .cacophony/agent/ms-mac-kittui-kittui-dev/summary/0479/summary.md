# Session summary — kittwm HELP_JSON CLI wrapper

## Goal

Expose the native `HELP_JSON` command catalog through a stable cooked-mode CLI wrapper.

## Bead(s)

- `bd-c706ec` — kittwm: CLI wrapper for HELP_JSON

## Before state

- Failing tests: none known.
- Relevant context: socket `HELP_JSON` and SDK `help_catalog()` / `help()` existed, but users still needed `kittwm --attach -c HELP_JSON` for CLI access.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --bin kittwm normalize_daemon_command_preserves_json_inspection_verbs -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `kittwm --help-json`, mapping to socket `HELP_JSON`.
  - Added CLI help text.
  - Existing `--attach -c HELP_JSON` path remains unchanged.
  - No daemon/session/SDK changes.

## Parallel coordination

- `kittui-dev-2` has docs-only follow-ups:
  - `bd-ad3f03` SDK shortcut catalog helper docs.
  - `bd-74cccd` docs for `kittwm --help-json` after this lands.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

Users can now run `kittwm --help-json` instead of spelling `kittwm --attach -c HELP_JSON` for the machine-readable command catalog.
