# Session summary — Show source path in stats

## Goal

Align `kittui-md --stats` with metadata JSON provenance by including the source path when input comes from a file.

## Bead(s)

- `bd-90c61c` — Show source path in kittui-md stats

## Before state

- Failing tests: none known.
- Relevant metrics: `--metadata-json` included `source.path`, but `--stats` only showed bytes and line counts.
- Context: stats mode is the concise text summary and should include enough source provenance to be useful in logs.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md stats_mode -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --stats docs/examples/kittui-md-proof.md | rg 'source.path=docs/examples/kittui-md-proof.md|components='` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: stats now prints `source.path=<stdin>` for stdin and the actual path for file inputs.

## Diff summary

- Code/content commits: `83914bf`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added stats source-path coverage and updated existing stats assertions.
- Behavioural delta: `kittui-md --stats` now includes source path provenance.

## Operator-takeaway

Stats output now identifies whether it summarized stdin or a specific Markdown file, matching the provenance already exposed in JSON.
