# Session summary — socket spawn request path

## Goal

Finish the first usable slice of the kittwm-native app protocol by making `kittwm replace ...` work both inside a current kittwm window and from a socket-only context.

## Bead(s)

- `bd-cddcf2` — kittwm native app protocol: replace current pane or spawn via socket context
- Related active proof bead: `bd-a9ec5b` — Prove: kitwm with no args opens a usable session, launches xterm, you can type into it

## Before state

- Failing tests: none known.
- Relevant metrics: `kittwm replace /bin/echo ...` worked when `KITTWM_WINDOW` was present, but socket-only spawn was still reported as a follow-up.
- Context: Harry wants DISPLAY-like semantics: a WM-aware binary should replace its current container when inside a kittwm window, or ask the host socket to spawn a new window/process when only a socket/display context exists.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::spawn_command_returns_pid -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm --bin kittwm-browser` passed.
- Context: the daemon protocol now accepts `SPAWN <argv>` and returns `SPAWNED pid=...`; `kittwm replace ...` sends that request when `KITTWM_WINDOW` is absent but `KITTWM_SOCKET` is set. In-window replacement still uses Unix `exec`.

## Diff summary

- Code/content commits: `3485717`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`, `crates/kittui-cli/src/daemon.rs`
- Tests: added daemon test coverage for `SPAWN` returning a pid.
- Behavioural delta: kittwm-native app launchers now have both replace-current-window and socket-spawn code paths at the protocol level.

## Operator-takeaway

The kittwm-native app model now has its first DISPLAY-like primitive: apps can inherit a socket/window context and either replace themselves or ask the host daemon to spawn a process. It is still process-level spawn, not yet fully integrated pane bookkeeping.
