# Session summary — Add schemas-json discovery

## Goal

Add a no-input discovery mode for `kittui-md` machine-readable outputs so tools can inspect available JSON payload families and their top-level keys without rendering a document.

## Bead(s)

- `bd-b33dd3` — Add kittui-md schemas-json discovery mode

## Before state

- Failing tests: none known.
- Relevant metrics: `--modes`/`--modes-json` listed output modes, but there was no dedicated catalog of JSON output shapes or top-level keys.
- Context: kittui-md now has many focused JSON outputs; tooling benefits from a compact schema summary for discovery and compatibility checks.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md schemas_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --schemas-json | rg '"schema_version": 1|"mode": "--metadata-json"|"top_level_keys"|"components_detail"|"mode": "--schemas-json"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--schemas-json` returns before reading input and emits `schema_version: 1` plus indexed schema-summary records with `mode`, `top_level_keys`, and `description`.

## Diff summary

- Code/content commits: `34744d1`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and JSON schema catalog output coverage.
- Behavioural delta: tools can discover kittui-md JSON output shapes natively.

## Operator-takeaway

kittui-md now exposes both a mode catalog and a compact JSON schema-summary catalog, improving self-describing CLI/tooling integration.
