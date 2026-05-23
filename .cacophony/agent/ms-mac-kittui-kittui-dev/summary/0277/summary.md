# Session summary — Python render_json bytes API

## Goal

Expose the lower-level `kittui_render_json` FFI entry point through the Python binding so Python hosts can obtain PNG bytes without emitting terminal placement escapes.

## Bead(s)

- `bd-7e0551` — bindings-python: expose render_json PNG bytes

## Before state

- Failing tests: none known.
- Relevant gap: Python binding had placement and channelized batch APIs, but no render-only API for preview/test/embed workflows outside a terminal stream.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `PYTHONPATH=bindings/python python3 -m unittest discover bindings/python` passed.
  - `git diff --check` passed.
- Context: `Kittui.render(scene) -> bytes` now calls `kittui_render_json`, copies the returned PNG bytes, and releases the FFI buffer with `kittui_bytes_free`. ctypes signatures now include `kittui_render_json` and `kittui_bytes_free`. Fake-CDLL tests cover success, buffer-free ownership, and render failure details. README API/example includes `render(scene)`.

## Diff summary

- Code/content commit: `017a27d`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `bindings/python/kittui/__init__.py`, `bindings/python/tests/test_kittui.py`, `bindings/python/README.md`
- Behavioural delta: Python platform hosts can render scenes to PNG bytes directly.

## Operator-takeaway

Python can now use kittui both as a terminal placement engine and as a render-only PNG producer for previews or non-terminal embedding.
