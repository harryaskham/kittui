# Session summary — kittwm inspection CLI wrappers

## Goal

Expose native kittwm inspection socket surfaces as first-class CLI flags instead of requiring raw `kittwm --attach -c ...` protocol strings.

## Bead(s)

- `bd-2d7a34` — kittwm: add native inspection CLI wrappers

## Before state

- Failing tests: none known.
- Relevant gap: automation and save/restore had CLI wrappers, but common inspection still required commands like `kittwm --attach -c PANES_JSON` or `SESSION_JSON`.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittwm normalize_daemon_command_preserves_json_inspection_verbs -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added first-class flags that route through default socket resolution and print replies:
  - `--status-json` -> `STATUS_JSON`
  - `--panes` -> `PANES`
  - `--panes-json` -> `PANES_JSON`
  - `--session-json` -> `SESSION_JSON`
  Help text, README, and docs/wm now prefer these wrappers in examples while raw `--attach -c` remains available.

## Diff summary

- Code/content commit: `0a7cbf8`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: native kittwm inspection is directly scriptable via stable flags.

## Operator-takeaway

Use `kittwm --panes-json`, `kittwm --status-json`, or `kittwm --session-json` for common controller inspection instead of raw protocol commands.
