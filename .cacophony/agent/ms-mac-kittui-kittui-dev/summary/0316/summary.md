# Session summary — kittwm app discovery CLI wrappers

## Goal

Expose native kittwm socket app discovery commands as first-class CLI wrappers, continuing the move away from raw `kittwm --attach -c ...` protocol strings for common controller workflows.

## Bead(s)

- `bd-0de0a2` — kittwm: add native app discovery CLI wrappers

## Before state

- Failing tests: none known.
- Relevant gap: app discovery still used raw socket commands in docs/examples (`APPS_JSON`, `APPS_FIRST`, `APPS_LAUNCH_FIRST`) while inspection, pane control, automation, and session persistence had gained CLI wrappers.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittwm pane_control_requests -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added CLI wrappers:
  - `--apps-json` -> `APPS_JSON`
  - `--apps-first <query>` -> `APPS_FIRST <query>`
  - `--apps-launch-first <query>` -> `APPS_LAUNCH_FIRST <query>`
  Queries preserve case/spaces through `protocol_payload_request`. Help text and docs/wm examples now mention the wrappers.

## Diff summary

- Code/content commit: `522aebd`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`, `docs/wm.md`
- Behavioural delta: native app discovery and first-match launch are directly scriptable via stable flags.

## Operator-takeaway

Use `kittwm --apps-json`, `kittwm --apps-first htop`, or `kittwm --apps-launch-first Safari` instead of raw socket protocol commands.
