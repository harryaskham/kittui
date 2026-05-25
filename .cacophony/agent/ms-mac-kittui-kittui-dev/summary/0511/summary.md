# Session summary — mcp-cli / updatable-cli update plumbing

## Goal

Refactor kittui/kittwm to use shared `mcp-cli` and `updatable-cli` crates so released binaries can self-update from GitHub release assets and expose the same update surface over MCP.

## Bead(s)

- `bd-f7ff4b` — kittui/kittwm use mcp-cli and updatable-cli crates

## Changes

- Added workspace dependencies:
  - `mcp-cli` from `https://github.com/harryaskham/mcp-cli`
  - `updatable-cli` from `https://github.com/harryaskham/updatable-cli`
- Added `crates/kittui-cli/src/update.rs` with shared update plumbing:
  - `updater_config(tool)` using repository `harryaskham/kittui`
  - `maybe_apply_staged_update(tool)` startup hook
  - `run_update_command(tool, options)` for status/check/run
  - `serve_update_mcp(tool)` exposing updatable-cli self-update tools over MCP stdio
- Wired `kittui`:
  - `kittui update [--status|--check] [--repository OWNER/REPO] [--install-dir DIR]`
  - `kittui mcp`
  - startup staged-update promotion for `kittui_next`
  - JSON update output via `mcp-cli::JsonEnvelope`
- Wired `kittwm`:
  - `kittwm update [status|check|run] [--json] [--repository OWNER/REPO] [--install-dir DIR]`
  - `kittwm mcp`
  - startup staged-update promotion for `kittwm_next`
  - help/command catalog entries
- Added `.github/workflows/release.yml` for self-hosted runner release assets:
  - builds `kittui` and `kittwm` on self-hosted Darwin/Linux lanes
  - packages Tendril/updatable-cli-style assets `<tool>-<version>-<target>.tar.gz` and `.sha256`
  - uploads and publishes GitHub releases after expected assets exist

## Validation

- `cargo test -p kittui-cli update --lib -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittui update_args_map_to_shared_update_options -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittwm update_options_parse_status_check_json_and_paths -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui --bin kittwm` passed.
- `target/debug/kittui update --status --json | python3 -m json.tool` passed.
- `target/debug/kittwm update --status --json | python3 -m json.tool` passed.
- `git diff --check` passed.
