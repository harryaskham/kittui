# Session summary — expose drag events in SEND_MOUSE

## Goal

Make the public native mouse injection API match the internal drag-motion support so external controllers can inject button-drag events.

## Bead(s)

- `bd-1c3dbe` — kittwm: expose drag events in SEND_MOUSE

## Before state

- Failing tests: none known.
- Relevant gap: host mouse routing internally supported `move-left`, `move-middle`, and `move-right`, but the public `SEND_MOUSE` socket command and `kittwm --send-mouse` wrapper still only accepted generic `move`. Controllers could not inject drag motions without raw bytes.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_parses_focus_close_layout_and_rename_commands -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request_preserves_payload_case_and_spaces -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `SEND_MOUSE` and `--send-mouse` now accept `move-left`, `move-middle`, and `move-right` in addition to `press-left`, `press-middle`, `press-right`, `release`, `move`, `scroll-up`, and `scroll-down`. Tests verify queueing and CLI request generation. docs/wm lists the accepted events.

## Diff summary

- Code/content commit: `36f8983`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/bin/kittwm.rs`, `docs/wm.md`
- Behavioural delta: external native-pane controllers can inject drag mouse events through stable kittwm wrappers.

## Operator-takeaway

Use `kittwm --send-mouse focused move-left COL ROW` for drag motion automation when a pane has requested button-motion/all-motion mouse reporting.
