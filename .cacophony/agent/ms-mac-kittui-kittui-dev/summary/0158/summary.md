# Session summary — Add grid alias

## Goal

Improve kittui-md CLI ergonomics by adding `--grid` as a friendly alias for table-layout inspection.

## Bead(s)

- `bd-e2b5eb` — Add kittui-md grid alias for tables

## Before state

- Failing tests: none known.
- Relevant metrics: table-layout inspection was available through `--tables`, but not through the common grid terminology.
- Context: kittui-md now has aliases for several focused inspection modes; tables naturally map to grid layout terminology.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md grid -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --grid docs/examples/kittui-md-proof.md | rg 'kittui-md tables|alignments|footprint'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--grid` maps to the same `Mode::Tables` output as `--tables`, and conflict detection rejects both flags together.

## Diff summary

- Code/content commits: `7958e9f`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser coverage for alias acceptance and alias/mode conflict behavior.
- Behavioural delta: users can run `kittui-md --grid` for table-layout inspection.

## Operator-takeaway

Table inspection is now discoverable through both `--tables` and the grid-oriented `--grid` alias.
