# Session summary — render_json now returns PNG bytes

## Goal

Fix the render-only FFI path so `kittui_render_json` actually returns PNG bytes rather than terminal placement escape bytes.

## Bead(s)

- `bd-e3b1c5` — kittui-ffi: make render_json return PNG bytes

## Before state

- Failing tests: none known.
- Relevant gap: `kittui.h` and the Python binding described `kittui_render_json` as render-only bytes/PNG, but the implementation called `Runtime::place` and returned kitty upload/placement/embed escapes. This made render-only platform preview/non-terminal embedding workflows incorrect and also required terminal capability support.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui render_png_returns_png_without_terminal_support -- --nocapture` passed.
  - `cargo test -p kittui-ffi render_json_returns_png_bytes_without_terminal_support -- --nocapture` passed.
  - `PYTHONPATH=bindings/python python3 -m unittest discover bindings/python` passed.
  - `git diff --check` passed.
- Context: Added public `Runtime::render_png(&Scene) -> Vec<u8>` that renders without terminal support checks or placement state changes. `kittui_render_json` now uses that API and returns PNG bytes. Tests assert PNG signature, absence of kitty escapes, and success even when terminal support is disabled.

## Diff summary

- Code/content commit: `419586f`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`, `crates/kittui-ffi/src/lib.rs`
- Behavioural delta: FFI/Python render-only calls now produce real PNG bytes suitable for previews and non-terminal embedding.

## Operator-takeaway

This corrects a significant platform API semantic mismatch and gives external hosts a true render-only path.
