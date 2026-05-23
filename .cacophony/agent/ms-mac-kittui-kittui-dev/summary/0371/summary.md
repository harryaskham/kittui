# Session summary — native shell/chrome view model

## Goal

Continue the SDK/surface plan by splitting the default native shell chrome into a presentation-agnostic view model before rendering it with ANSI/kitty output.

## Bead(s)

- `bd-0957d6` — kittwm: extract presentation-agnostic shell view and chrome model

## Before state

- Failing tests: none known.
- Relevant gap: the live native session loop directly formatted pane title rows and footer strings in the rendering path. This made future pure-terminal, kittui-scene, or headless renderers harder to add.

## After state

- Failing tests: none in targeted checks.
- Validation:
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_shell_view_builds_presentation_agnostic_chrome -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_pane_layouts_split_columns_and_reserve_title_rows -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `NativeShellView`, `NativePaneChrome`, and `NativeFooterChrome`.
  - Added `native_shell_view(...)` to build pane chrome/footer state from panes, layouts, focus, socket, and log path.
  - The live renderer now consumes this view model and writes ANSI from `NativePaneChrome` rather than formatting directly inside the main loop.
  - Existing title/footer redraw caching is preserved via `cache_key` fields.

## Diff summary

- Code/content commit: `d431451`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Behavioural delta: no intended UX change; chrome state is now separated from its ANSI rendering path.

## Operator-takeaway

The native shell now has a small presentation-agnostic chrome/view model, which is a stepping stone toward pure terminal rendering and kittui-scene/live chrome renderers.
