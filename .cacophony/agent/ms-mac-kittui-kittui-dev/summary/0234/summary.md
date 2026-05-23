# Session summary — native PTY row/column layouts

## Goal

Make native `kittwm` pane splitting more WM-like by supporting both side-by-side columns and stacked row layouts instead of only vertical columns.

## Bead(s)

- `bd-4f2c7e` — kittwm: add native PTY row and column split layouts

## Before state

- Failing tests: none known.
- Relevant gap: native PTY splits always used a side-by-side column layout. There was no stacked/editor-over-shell style row layout.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli native_pane_layouts -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: the native session tracks a `NativePaneLayoutAxis`. `Ctrl-A %` / `Ctrl-A |` / `Ctrl-A v` selects column layout and spawns a side-by-side pane. `Ctrl-A -` / `Ctrl-A h` / `Ctrl-A "` selects row layout and spawns a stacked pane. Existing close, resize, and socket spawn flows reflow through the active axis.

## Diff summary

- Code/content commit: `33b3c97`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: native PTY panes can now be tiled either horizontally or vertically.

## Operator-takeaway

The default kittwm terminal WM now supports a core layout choice: side-by-side panes or stacked rows, making it more usable for real terminal workflows.
