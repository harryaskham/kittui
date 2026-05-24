# Session summary — kittwm quickstart command

## Goal

Give new users a first-run checklist for using kittwm as a daily-driver terminal WM.

## Bead(s)

- `bd-816016` — kittwm: quickstart command for first-run daily-driver use

## Before state

- Failing tests: none known.
- User feedback: kittwm should be easy to use; the command tree/help alone is not enough for first-run daily-driver adoption.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --bin kittwm quickstart_teaches_daily_driver_path -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm kittwm_help_is_grouped_for_daily_driver_use -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added cooked-mode `kittwm quickstart`.
  - The quickstart covers starting kittwm, in-session shortcuts, `kittwm info`, aliases for spawn/read/type/line/key/wait, pane management aliases, save/restore, and topic help.
  - Updated grouped `--help` to point at `quickstart`.
  - No daemon/session runtime changes.

## Parallel coordination

- `kittui-dev-2` is doing actual source work on `bd-459edf` pane-control subcommand aliases and was asked to avoid action aliases already landed by this agent.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

New users can now run `kittwm quickstart` to see the concrete sequence for starting, inspecting, controlling, automating, and saving a kittwm session.
