# Session summary — Add exit-code discovery outputs

## Goal

Add no-input `kittui-md` exit-code discovery outputs so users and tools can query process exit code meanings without consulting README text.

## Bead(s)

- `bd-85c098` — Add kittui-md exit code discovery outputs

## Before state

- Failing tests: none known.
- Relevant metrics: kittui-md exposed many discovery surfaces but did not document process exit code meanings via CLI output.
- Context: integrations that shell out to kittui-md benefit from a machine-readable exit-code catalog.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md exit_codes -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --exit-codes-json | rg '"schema_version": 1|"code": 0|"name": "success"|"code": 1|"name": "error"'` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --exit-codes | rg 'exit codes|0 success|1 error'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--exit-codes` emits text; `--exit-codes-json` emits `schema_version: 1` plus indexed `exit_codes` entries with `code`, `name`, and `description`.

## Diff summary

- Code/content commits: `3b5c8eb`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and text/JSON exit-code output coverage.
- Behavioural delta: exit-code metadata is now available through a focused no-input CLI surface.

## Operator-takeaway

kittui-md now self-documents process exit meanings for shell integrations in both text and JSON forms.
