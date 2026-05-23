# Session summary — readable native terminal glyphs and title redraw caching

## Goal

Fix the native kittwm PTY UX issue where interactive blitted terminal panes showed printable characters as pseudo-box literals, and reduce static title/footer flicker from unnecessary ANSI redraws.

## Bead(s)

- `bd-b518fc` — kittwm: render readable native terminal glyphs and reduce title flicker

## Before state

- Failing tests: none known.
- User-visible gap: native PTY panes were interactive and blitted via kitty graphics, but terminal glyphs used synthetic pseudo-box strokes, making zsh/shell text unreadable. Native pane title/status rows were also rewritten every frame, causing visible flicker/movement.

## After state

- Failing tests: none in targeted checks.
- Validation:
  - `cargo test -p kittui-wm terminal_renderer_draws_readable_bitmap_glyphs -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_renderer_uses_sgr_foreground_and_background -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_pane_layouts_split_columns_and_reserve_title_rows -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Replaced pseudo glyph hashing with a small deterministic built-in bitmap renderer for common ASCII terminal characters.
  - Lowercase currently reuses uppercase bitmaps, but glyphs are now readable instead of opaque boxes.
  - Added vector drawing for common Unicode box-drawing glyphs produced by DEC Special Graphics.
  - Preserved existing SGR foreground/background colors and cursor rendering.
  - Added cached native pane title/footer redraws. Static title rows are only rewritten when clear/layout/focus/title changes; footer no longer includes a changing frame counter.

## Diff summary

- Code/content commit: `15d388c`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `crates/kittui-wm/src/native.rs`
- Behavioural delta: interactive native kittwm terminal panes should now show readable text and less flickery static chrome.

## Operator-takeaway

The blitted native terminal surface remains graphics-based, but printable terminal contents are now rendered with readable bitmap glyphs instead of pseudo boxes.
