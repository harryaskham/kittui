# Session summary — Add examples discovery outputs

## Goal

Add no-input `kittui-md` example discovery outputs so users and tools can list common invocation examples in text or JSON form.

## Bead(s)

- `bd-16f22e` — Add kittui-md examples discovery outputs

## Before state

- Failing tests: none known.
- Relevant metrics: README listed many examples, and the CLI exposed mode/default/format discovery, but there was no focused machine-readable examples catalog.
- Context: example discovery helps users and integrations bootstrap common invocations without scraping README text.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md examples -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --examples-json | rg '"schema_version": 1|"name": "component-json"|"--components-json"'` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --examples | rg 'kittui-md examples|rich-file|components-json'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--examples` emits text; `--examples-json` emits `schema_version: 1` plus indexed `examples` entries with `name`, `argv`, and `description`.

## Diff summary

- Code/content commits: `0a27edd`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and text/JSON examples output coverage.
- Behavioural delta: common invocation examples are now available through a focused no-input CLI surface.

## Operator-takeaway

kittui-md now exposes example invocations natively, completing another piece of its self-describing CLI/tooling surface.
