# Session summary — Expose metadata blocks in kittui-md outputs

## Goal

Fix the follow-up gap from Markdown metadata preservation by ensuring `kittui-md` exposes preserved metadata blocks through its user/tool output surfaces.

## Bead(s)

- `bd-1510b8` — kittui-md exposes preserved metadata blocks in outputs

## Before state

- Failing tests: none known.
- Relevant metrics: `MarkdownDocument` preserved metadata blocks, and README documented them, but `kittui-md --metadata-json`, `--stats`, and plain metadata sections did not expose them.
- Context: frontmatter preservation is only useful to downstream tools if it appears in the output contracts.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md metadata_blocks -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittui-md metadata_json_mode -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: metadata blocks now appear in JSON as `metadata_blocks`, in stats as `metadata_blocks=<n>`, and in plain output under `metadata blocks:`.

## Diff summary

- Code/content commits: `112d5e5`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`
- Tests: added metadata-block coverage for JSON, stats, and plain output.
- Behavioural delta: kittui-md output surfaces now match the renderer's metadata preservation.

## Operator-takeaway

Frontmatter now survives all the way to the CLI/tooling outputs instead of being preserved only internally.
