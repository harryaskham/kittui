# Session summary — local kittwm command catalog

## Goal

Let users and tools discover local `kittwm` CLI commands and aliases without requiring a running WM socket.

## Bead(s)

- `bd-e972ea` — kittwm: local command catalog

## Before state

- Failing tests: none known.
- Relevant context: socket `HELP_JSON` catalogs daemon verbs, but daily-driver local aliases like `spawn`, `read`, `focus`, `quickstart`, and `examples` needed an offline catalog.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `kittwm commands` grouped text catalog.
  - Added `kittwm commands-json` machine-readable catalog.
  - Catalog covers lifecycle, help, inspect, action, pane-control, app, session, and diagnostics commands/aliases.
  - Help now points at the command catalog.
  - Socket `HELP_JSON` is unchanged and still catalogs running-WM daemon verbs.

## Parallel coordination

- `kittui-dev-2` has actual source bead `bd-4812fc` for daily-driver hints in shortcut overlay/shared shortcuts, avoiding this local catalog work.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

Users can run `kittwm commands` or `kittwm commands-json` for an offline map of local daily-driver commands and aliases.
