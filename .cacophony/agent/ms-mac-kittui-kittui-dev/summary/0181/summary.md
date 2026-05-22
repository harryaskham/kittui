# Session summary — Add html-json mode

## Goal

Add a compact machine-readable `kittui-md --html-json` mode for tools that need parsed Markdown HTML placeholder fragments without the full metadata JSON payload.

## Bead(s)

- `bd-5db881` — Add kittui-md html-json inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: HTML fragment records were available as text through `--html`/`--markup` and inside full metadata JSON, but there was no focused JSON HTML-only output.
- Context: HTML/markup audit tooling may need indexed kind/source records without parsing text or full document metadata.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md html_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --html-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"kind": "inline"|"kind": "block"|"source": "<kbd>'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--html-json` emits `schema_version: 1` plus indexed HTML fragment records with `kind` and `source`.

## Diff summary

- Code/content commits: `671b5fe`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and JSON output coverage for HTML JSON.
- Behavioural delta: users and tools can request compact JSON HTML fragment records.

## Operator-takeaway

kittui-md now has both human-readable and machine-readable focused HTML/markup inspection modes.
