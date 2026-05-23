# Session summary — Python ctypes binding

## Goal

Add a first-party Python binding over the existing kittui C ABI so Python shell/platform automation can use kittui as a renderer substrate without hand-rolling ctypes calls or spawning the CLI per frame.

## Bead(s)

- `bd-5e4e56` — bindings-python: add ctypes platform binding

## Before state

- Failing tests: none known.
- Relevant gap: kittui had Rust, CLI, C ABI, and TypeScript surfaces, but no Python platform binding. Python users had to manually load `libkittui_ffi` and manage owned strings/status codes themselves.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `PYTHONPATH=bindings/python python3 -m unittest discover bindings/python` passed.
  - `git diff --check` passed.
- Context: Added `bindings/python/kittui/__init__.py`, a stdlib-only ctypes wrapper supporting runtime config, ABI version, probe, unplace, place, place_at, place_many, place_many_at, and place_many_channels. Added fake-CDLL unittest coverage, package test marker, and README usage examples. No Rust ABI changes were needed.

## Diff summary

- Code/content commit: `7397850`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `bindings/python/README.md`, `bindings/python/kittui/__init__.py`, `bindings/python/tests/__init__.py`, `bindings/python/tests/test_kittui.py`
- Behavioural delta: Python hosts can use kittui's FFI renderer API directly, including channelized batch placement.

## Operator-takeaway

kittui is now available as a renderer substrate from Python as well as Rust/CLI/C/TypeScript, improving platform and shell automation reach.
