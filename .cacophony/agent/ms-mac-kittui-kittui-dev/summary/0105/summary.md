# Session summary — Source metrics in metadata JSON

## Goal

Continue kittui-md metadata work by adding source document size metrics to `--metadata-json`.

## Bead(s)

- `bd-d6b5a7` — kittui-md metadata JSON includes source document metrics

## Before state

- Failing tests: none known.
- Relevant metrics: metadata JSON exposed rendered structure but not basic source size, so downstream consumers could not correlate metadata to input byte/line size without keeping their own counters.
- Context: schema versioning was just added; source metrics are another small stable metadata field.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md metadata_json -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - A stdin smoke check showed `source.bytes` and `source.lines`.
- Context: `write_metadata_json` now takes the source Markdown string and emits `source: { bytes, lines }`; the metadata JSON test asserts both values.

## Diff summary

- Code/content commits: `d6d89b4`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`
- Tests: extended `metadata_json_mode_reports_stable_shape` for source metrics.
- Behavioural delta: `--metadata-json` includes input source metrics alongside rendered metadata.

## Operator-takeaway

Metadata JSON now includes enough context to tell how large the input document was when the metadata was generated.
