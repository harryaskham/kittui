# Session summary — TerminalSurface extraction

## Goal

Start separating the native PTY terminal engine from `PtyTerminalApp` so future SDK and first-party terminal work can reuse terminal parsing, readback, resize, response, host-sequence, and RGBA rendering behavior without owning process lifecycle directly.

## Bead(s)

- `bd-099358` — kittwm: extract TerminalSurface engine from PtyTerminalApp

## Before state

- Failing tests: none known for the PTY-focused surface; a broader `native::tests` filter later timed out in the unrelated headless Chrome availability test.
- Relevant metrics: `PtyTerminalApp` directly owned PTY master/writer, parser thread state, cell sizing, terminal snapshots, input, resize, and capture.
- Context: `docs/kittwm-sdk-plan.md` listed terminal engine extraction as the next SDK architecture stage after the native socket event stream.

## After state

- Failing tests: none in targeted PTY/terminal validation.
- Relevant metrics: `cargo test -p kittui-wm native::tests::pty_terminal --lib` passed 3/3; `cargo test -p kittui-wm native::tests::terminal_state --lib` passed 33/33. A broader `cargo test -p kittui-wm native::tests --lib` was stopped by the harness after the unrelated `headless_browser_data_url_screenshot_when_chrome_available` test ran for over 60 seconds.
- Context: `TerminalSurface` now owns PTY read/write, parser thread, terminal state snapshots, host sequences, resize, byte/text input, and RGBA capture, while `PtyTerminalApp` keeps child process lifecycle and delegates terminal behavior.

## Diff summary

- Code/content commits: `dae9d5b`.
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA.
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/kittwm-sdk-plan.md`.
- Tests: no new tests added; existing PTY terminal and terminal-state test filters validate behavior preservation across the extraction.
- Behavioural delta: no intended runtime behavior change for `PtyTerminalApp`; the terminal engine now has a named reusable `TerminalSurface` boundary.

## Operator-takeaway

This is the first mechanical boundary for the future terminal SDK extraction: process lifecycle remains in `PtyTerminalApp`, but the terminal surface behavior is no longer embedded directly in that adapter.
