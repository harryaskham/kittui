# Session summary — Add defaults discovery outputs

## Goal

Add no-input `kittui-md` defaults discovery outputs so users and tools can query default mode, input behavior, width bounds, and interactive defaults without rendering a document.

## Bead(s)

- `bd-57c267` — Add kittui-md defaults discovery outputs

## Before state

- Failing tests: none known.
- Relevant metrics: kittui-md exposed version, capabilities, input formats, output formats, modes, and schemas, but did not have a focused default-settings discovery surface.
- Context: integrations often need default behavior metadata before deciding which flags to pass.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md defaults -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --defaults-json | rg '"schema_version": 1|"mode": "rich"|"max": 200|"input": "stdin-or-one-file"'` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --defaults | rg 'kittui-md defaults|mode=rich|width.max=200'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--defaults` emits text; `--defaults-json` emits `schema_version: 1` plus a `defaults` object covering mode, width limits, offset, interactive, and input behavior.

## Diff summary

- Code/content commits: `72c7e0a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and text/JSON defaults output coverage.
- Behavioural delta: default-setting probing is now available as a focused no-input CLI surface.

## Operator-takeaway

kittui-md now exposes default behavior explicitly, complementing the discovery APIs for versions, capabilities, modes, schemas, and IO formats.
