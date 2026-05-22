# Session summary — Add equations alias

## Goal

Improve kittui-md CLI ergonomics by adding `--equations` as a friendly alias for math-expression inspection.

## Bead(s)

- `bd-40cda9` — Add kittui-md equations alias for math

## Before state

- Failing tests: none known.
- Relevant metrics: math-expression inspection was available through `--math`, but not through common equations terminology.
- Context: kittui-md has focused inspection modes and aliases for common Markdown structures; math content is often searched for as equations.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md equations -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --equations docs/examples/kittui-md-proof.md | rg 'kittui-md math|kind=inline|kind=display|a\^2'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--equations` maps to the same `Mode::Math` output as `--math`, and conflict detection rejects both flags together.

## Diff summary

- Code/content commits: `8d6b696`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser coverage for alias acceptance and alias/mode conflict behavior.
- Behavioural delta: users can run `kittui-md --equations` for math-expression inspection.

## Operator-takeaway

Math inspection is now discoverable through both `--math` and the equation-oriented `--equations` alias.
