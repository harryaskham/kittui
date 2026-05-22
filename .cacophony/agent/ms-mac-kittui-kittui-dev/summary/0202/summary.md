# Session summary — Add output format discovery outputs

## Goal

Add no-input `kittui-md` output-format discovery outputs so users and tools can query supported output families independently of the mode catalog.

## Bead(s)

- `bd-bfd2c0` — Add kittui-md output format discovery outputs

## Before state

- Failing tests: none known.
- Relevant metrics: kittui-md exposed many modes and JSON schemas, plus input-format discovery, but did not summarize supported output families such as rich kitty graphics, plain text, and JSON.
- Context: integration tooling benefits from a focused no-input call that describes output families and their related mode categories.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md output_formats -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --output-formats-json | rg '"schema_version": 1|"name": "json"|"rich-kitty-graphics"'` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --output-formats | rg 'output formats|rich-kitty-graphics|json'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--output-formats` emits text; `--output-formats-json` emits `schema_version: 1` plus indexed `output_formats` entries with `name`, `mode_categories`, and `description`.

## Diff summary

- Code/content commits: `64317a7`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and text/JSON output-format output coverage.
- Behavioural delta: output-family probing is now available as a focused no-input CLI surface.

## Operator-takeaway

kittui-md now advertises both input formats and output families directly, rounding out the discovery APIs for integrations.
