# Session summary — kittwm start/stop aliases

## Goal

Add obvious lifecycle commands for daily-driver users.

## Bead(s)

- `bd-9de4fd` — kittwm: start and stop lifecycle aliases

## Before state

- Failing tests: none known.
- Relevant context: `kittwm` with no args starts the WM and `--kill` stops a daemon, but users naturally try `kittwm start` / `kittwm stop`.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --bin kittwm lifecycle_aliases_map_to_modes -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm quickstart_teaches_daily_driver_path -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `kittwm start` as an explicit alias for the default session start.
  - Added `kittwm stop` as an alias for daemon kill/QUIT behavior.
  - Updated help and quickstart text.
  - Existing no-arg start and `--kill` behavior remain unchanged.

## Parallel coordination

- `kittui-dev-2` has source bead `bd-de5591` for `kittwm doctor` daily-driver readiness hints.
- `kittui-dev-2` was asked to avoid overlapping lifecycle aliases.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

Users can now type `kittwm start` and `kittwm stop`, matching the intuitive lifecycle vocabulary used by daily-driver CLIs.
