# Session summary — Add input format discovery outputs

## Goal

Add no-input `kittui-md` input-format discovery outputs so users and tools can query supported input formats and extensions independently of the mode catalog.

## Bead(s)

- `bd-47794e` — Add kittui-md input format discovery outputs

## Before state

- Failing tests: none known.
- Relevant metrics: kittui-md documented Markdown input behavior, but did not expose a machine-readable input-format list.
- Context: integration tooling benefits from a focused no-input call that says which source formats/extensions are supported.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md input_formats -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --input-formats-json | rg '"schema_version": 1|"name": "markdown"|"md"'` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --input-formats | rg 'input formats|markdown|md'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--input-formats` emits text; `--input-formats-json` emits `schema_version: 1` plus indexed `input_formats` entries with `name`, `extensions`, and `description`.

## Diff summary

- Code/content commits: `637612d`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and text/JSON input-format output coverage.
- Behavioural delta: input-format probing is now available as a focused no-input CLI surface.

## Operator-takeaway

kittui-md now advertises supported input formats directly, which rounds out the discovery surfaces for version, capabilities, modes, schemas, and source formats.
