# Session summary — native send-file wrapper

## Goal

Make exact byte injection into native kittwm panes easier by allowing the CLI to read a file/stdin and base64-encode it internally.

## Bead(s)

- `bd-81ef66` — kittwm: add send-file byte injection wrapper

## Before state

- Failing tests: none known.
- Relevant gap: `SEND_BYTES_B64` supported exact arbitrary-byte injection, but users still had to run an external base64 encoder and quote the result to paste files/scripts/binary payloads into native panes.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittwm automation_request -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added CLI wrapper `kittwm --send-file <window|focused> <path|->`. It reads bytes from a file or stdin, base64-encodes internally, and sends the existing `SEND_BYTES_B64` socket request through default socket resolution. Added helper coverage and docs/README examples.

## Diff summary

- Code/content commit: `305ce06`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: automation scripts can inject exact file/stdin bytes without manually invoking base64.

## Operator-takeaway

Use `kittwm --send-file focused ./payload.txt` or `cat payload.bin | kittwm --send-file focused -` for exact byte injection.
