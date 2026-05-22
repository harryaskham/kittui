# Session summary — native replace and browser app

## Goal

Continue converting kittwm native-app support from proof commands into first-class behavior: add a `kittwm-browser` binary, add `kittwm replace ...` semantics for WM-aware processes, and keep the live native PTY sized to the host terminal.

## Bead(s)

- `bd-4b9c96` — Wire kittui-wm NativeApp surfaces into live compositor panes
- `bd-cddcf2` — kittwm native app protocol: replace current pane or spawn via socket context
- `bd-d75324` — kittwm-browser binary: first-class native browser app
- Related active proof bead: `bd-a9ec5b` — Prove: kitwm with no args opens a usable session, launches xterm, you can type into it

## Before state

- Failing tests: none known.
- Relevant metrics: default `kittwm` native PTY worked and tracked host resize, but native browser was still a proof command and there was no `replace` path for binaries launched inside a kittwm window.
- Context: Harry clarified that native apps should be normal binaries that inherit `KITTWM_SOCKET` / `KITTWM_WINDOW`, and `kittwm replace browser` should replace the current container rather than being a demo subcommand.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm native::tests -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm --bin kittwm-browser` passed.
  - `KITTWM_WINDOW=native-1 KITTWM_SOCKET=/tmp/kittwm-test.sock ./target/debug/kittwm replace /bin/echo replace-ok` printed `replace-ok`.
  - tmux smoke launched `kittwm-browser` against a data URL and showed the live browser footer/frame loop.
- Context: `kittwm-browser` is now a separate binary that uses the headless Chrome NativeApp implementation. `kittwm replace browser ...` maps to `kittwm-browser ...`, while arbitrary `kittwm replace <argv...>` execs in-place when `KITTWM_WINDOW` is present. Socket-only spawn remains a follow-up for the daemon protocol.

## Diff summary

- Code/content commits: `71e5fed`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/Cargo.toml`, `crates/kittui-cli/src/bin/kittwm.rs`, `crates/kittui-cli/src/bin/kittwm_browser.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-wm/src/native.rs`
- Tests: no new integration test file; reran native module tests and CLI build, plus direct replace smoke.
- Behavioural delta: native browser can be launched as a normal binary, and kittwm-aware processes have the first replace-current-window behavior when running inside a kittwm PTY environment.

## Operator-takeaway

The architecture is moving toward the requested DISPLAY-like model: default kittwm gives child processes WM context, and those processes can now exec/replace themselves with kittwm-native apps. The missing piece is daemon/socket-mediated spawn when not already inside a focused window, plus richer live multi-pane composition.
