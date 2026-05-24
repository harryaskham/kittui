# Session summary — common kittwm action aliases

## Goal

Make common daily-driver kittwm actions available as memorable subcommands rather than long flags.

## Bead(s)

- `bd-fd22b5` — kittwm: common action subcommand aliases

## Before state

- Failing tests: none known.
- User feedback: kittwm should be easy to use as a daily-driver terminal WM.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --bin kittwm action_aliases_map_to_socket_commands -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm kittwm_help_is_grouped_for_daily_driver_use -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added subcommand aliases:
    - `kittwm spawn CMD [ARGS...]` -> `SPAWN_PTY <shell command>`
    - `kittwm read [WINDOW]` -> `READ_TEXT <window>` (default focused)
    - `kittwm read-json [WINDOW]` -> `READ_TEXT_JSON <window>`
    - `kittwm type [WINDOW] TEXT` -> `SEND_TEXT <window> <text>`
    - `kittwm line [WINDOW] TEXT` -> `SEND_LINE <window> <text>`
    - `kittwm key [WINDOW] KEY` -> `SEND_KEY <window> <key>`
    - `kittwm wait [WINDOW] TEXT` -> `WAIT_OUTPUT <window> <text>`
  - Updated grouped help to show the easy aliases.
  - Existing long flags remain unchanged.
  - No daemon/session runtime changes.

## Parallel coordination

- `kittui-dev-2` has source bead `bd-459edf` for pane-control aliases (`focus`, `close`, `layout`, `move`, `resize`, `balance`, `rename`) and was asked to avoid overlap with these action aliases.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

Daily-driver workflows are now shorter: e.g. `kittwm spawn htop`, `kittwm read-json focused`, `kittwm type hello`, and `kittwm wait Ready`.
