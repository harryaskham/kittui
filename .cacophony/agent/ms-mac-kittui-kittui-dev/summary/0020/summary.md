# Session summary — kitwm attach one-shot command

## Goal

Ship a small but visible daemon-client improvement for kitwm: let operators run one daemon command non-interactively from the shell instead of entering the interactive attach REPL.

## Bead(s)

- `bd-e813de` — kitwm --attach -c CMD: one-shot daemon command for scripting

## Before state

- Failing tests: none; workspace smoke was green at wake start.
- Context: `kitwm --serve`, `--status`, `--kill`, and interactive `--attach` already existed, but automation still had to pipe a scripted REPL session into stdin.

## After state

- Failing tests: none in `cargo test --workspace --lib --bins --tests -- --test-threads=2`.
- Relevant metrics: `kitwm_smoke` now has 13 tests, including one-shot attach coverage; workspace tests complete green.
- Context: `kitwm --attach -c STATUS`, `DISPLAYS`, and `WINDOWS` can be called directly against the daemon.

## Diff summary

- Code/content commits: pending final squash SHA from reintegration receipt
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kitwm.rs`, `crates/kittui-cli/tests/kitwm_smoke.rs`, `.cacophony/agent/ms-mac-kittui-kittui-dev/summary/pending/screenshots/bd-e813de-attach-command.png`
- Tests: +1 smoke test / -0 / flipped 0
- Behavioural delta: `--attach` accepts `-c`/`--command CMD`, sends one uppercase daemon command, prints the reply, and exits nonzero on daemon `ERR` replies.

## Embedded artefacts

- `screenshots/bd-e813de-attach-command.png` — tmux/tendril proof of one-shot `STATUS`, `DISPLAYS`, and `WINDOWS` commands against a live `kitwm --serve` daemon.

## Operator-takeaway

The daemon is now scriptable without an interactive REPL, which makes the `--serve`/`--attach` protocol useful for shell automation and follow-on attach-client work.
