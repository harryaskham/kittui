# Session summary — Add metadata-blocks-json mode

## Goal

Add a compact machine-readable `kittui-md --metadata-blocks-json` mode for tools that need parsed Markdown frontmatter/metadata block records without the full metadata JSON payload.

## Bead(s)

- `bd-01fb1f` — Add kittui-md metadata-blocks-json inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: metadata block records were available as text through `--metadata-blocks`/`--metadata`/`--frontmatter` and inside full metadata JSON, but there was no focused JSON metadata-block-only output.
- Context: frontmatter and metadata audit tooling may need indexed kind/source records without parsing text or full document metadata.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md metadata_blocks_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --metadata-blocks-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"metadata_blocks"|"kind": "yaml"|"source": "title: Kittui Markdown Proof'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--metadata-blocks-json` emits `schema_version: 1` plus indexed metadata block records with delimiter `kind` and `source`.

## Diff summary

- Code/content commits: `6f77a94`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and JSON output coverage for metadata blocks JSON.
- Behavioural delta: users and tools can request compact JSON metadata/frontmatter block records.

## Operator-takeaway

kittui-md now has both human-readable and machine-readable focused metadata/frontmatter inspection modes.
