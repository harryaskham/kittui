# Session summary — pure terminal native shell renderer

## Goal

Continue the SDK/surface plan by adding a pure ANSI/text renderer for the native shell view model as a fallback to kitty graphics frame blitting.

## Bead(s)

- `bd-1b4f3c` — kittwm: add pure terminal renderer for shell view model

## Before state

- Failing tests: none known.
- Relevant gap: the native shell view model existed, but live native mode still only rendered via kitty image placement plus ANSI chrome. There was no old-terminal/text fallback path consuming the view model.

## After state

- Failing tests: none in targeted checks.
- Validation:
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_shell_terminal_renderer_draws_chrome_and_snapshots -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_shell_view_builds_presentation_agnostic_chrome -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - `NativeShellView` now carries app text snapshots and app geometry in `NativePaneChrome`.
  - Added `render_native_shell_view_terminal(...)` to render pane chrome, clipped text snapshots, and footer with ANSI positioning.
  - Added `KITTWM_NATIVE_RENDERER=terminal|text|ansi|dec` opt-in path in the native session loop. When set, kittwm skips kitty image capture/placement and writes the pure terminal renderer output.
  - docs/wm documents `KITTWM_NATIVE_RENDERER=terminal`.

## Diff summary

- Code/content commit: `aa2c3f5`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `docs/wm.md`
- Behavioural delta: default kitty graphics mode remains unchanged; users can opt into a pure terminal/text renderer for native PTY panes.

## Operator-takeaway

The shell view model now has a second renderer, proving the presentation split and giving an old-terminal fallback path behind `KITTWM_NATIVE_RENDERER=terminal`.
