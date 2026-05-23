# Session summary — document Python platform binding

## Goal

Update top-level docs so the newly added Python ctypes binding is described as a first-party kittui platform surface rather than future/generic FFI work.

## Bead(s)

- `bd-4f4921` — docs: document Python binding as first-party platform surface

## Before state

- Failing tests: none known.
- Relevant gap: README and DESIGN still described Python mostly as a generic/future C ABI consumer, despite `bindings/python` now existing with package metadata and a module entrypoint.

## After state

- Failing tests: none.
- Relevant metrics:
  - `git diff --check` passed.
- Context: README shipped surfaces now include `bindings/ts` and `bindings/python`, quick start includes `python -m kittui --find-library`, and the crate table lists both platform binding directories. DESIGN now has a Python bindings section covering ctypes, runtime config, probe/ABI discovery, batch/origin/channelized placement APIs, package metadata, and `python -m kittui`; future bindings now only cover remaining non-first-party languages.

## Diff summary

- Code/content commit: `6c99775`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `README.md`, `DESIGN.md`
- Behavioural delta: documentation now accurately reflects the current Python platform binding and channelized API story.

## Operator-takeaway

The platform-renderer story is less stale: Python is now documented alongside Rust/CLI/C/TypeScript as a first-party way to drive kittui.
