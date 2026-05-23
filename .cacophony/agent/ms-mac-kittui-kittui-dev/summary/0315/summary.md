# Session summary — kittwm pane control CLI wrappers

## Goal

Expose native kittwm pane control socket commands as stable CLI wrappers for shell scripts and external controllers.

## Bead(s)

- `bd-ed52ca` — kittwm: add native pane control CLI wrappers

## Before state

- Failing tests: none known.
- Relevant gap: inspection, automation, and session save/restore had CLI wrappers, but pane control still required raw protocol strings such as `kittwm --attach -c 'MOVE_PANE focused last'`.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittwm pane_control_requests -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added CLI wrappers that route through default socket resolution and preserve payload case where needed:
  - `--spawn-pty <cmd>`
  - `--focus-pane <window>`
  - `--focus-next`
  - `--focus-prev`
  - `--close-pane <window|focused>`
  - `--layout <columns|rows>`
  - `--move-pane <window|focused> <left|right|up|down|first|last>`
  - `--resize-pane <window|focused> <grow|shrink|+N|-N>`
  - `--balance-panes`
  - `--rename-pane <window> <title>`
  Updated help text, README, and docs/wm examples.

## Diff summary

- Code/content commit: `30d0371`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: native kittwm pane control is directly scriptable via stable flags.

## Operator-takeaway

Use commands like `kittwm --spawn-pty htop`, `kittwm --move-pane focused last`, and `kittwm --resize-pane focused +2` instead of raw `--attach -c` protocol strings.
