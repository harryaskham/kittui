# Session summary — default kittwm native PTY terminal

## Goal

Convert the native PTY work from a proof subcommand into the beginning of first-class kittwm behavior: running `kittwm` with no backend flags should open a native PTY terminal sized to the host terminal and inject kittwm context into the child environment.

## Bead(s)

- `bd-ff3c1d` — kittwm default windows are native PTY terminals with KITTWM_SOCKET env
- Related active proof bead: `bd-a9ec5b` — Prove: kitwm with no args opens a usable session, launches xterm, you can type into it

## Before state

- Failing tests: none known.
- Relevant metrics: native PTY/browser proof commands worked, but `kittwm` no-args still selected the old capture-backed backend path unless explicitly running proof commands.
- Context: Harry clarified that native apps should be baked in, not exposed as messy demo subcommands; default windows should be PTY terminals whose child env includes WM socket/display/window context.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm native::tests::pty_terminal -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - tmux smoke launched `KITTWM_TERMINAL_CMD=cat KITTUI_WM_FPS=3 ./target/debug/kittwm` and typed through to the PTY.
- Context: `kittwm` with no `--backend`, `--capture`, or `--pick-window` now runs `run_native_terminal_loop`. The loop sizes itself from the host TTY (minus footer rows), polls size each frame, resizes the PTY on host resize, and injects `KITTWM_SOCKET`, `KITTWM_DISPLAY`, and `KITTWM_WINDOW` into the child environment. Explicit `--backend fake|quartz|xvfb` still reaches the older capture-backed modes.

## Diff summary

- Code/content commits: `491ac6b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-wm/src/native.rs`
- Tests: added PTY environment-injection coverage and reran native PTY tests.
- Behavioural delta: default `kittwm` is now a backend-independent native PTY terminal instead of a demo gallery/capture session; the PTY tracks host terminal rows/cols and carries kittwm context variables.

## Operator-takeaway

This is the first step from proofs to product behavior: `kittwm` now starts as a native terminal container that can run normal terminal apps, and the environment contract needed for future `kittwm replace ...` / `kittwm-browser` style apps is present in the child process.
