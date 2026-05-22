# Session summary — Show heading anchors in rich status

## Goal

Make heading-anchor coverage visible in the kittui-md rich/interactive pager status line.

## Bead(s)

- `bd-278b36` — Show heading anchor count in kittui-md rich status

## Before state

- Failing tests: none known.
- Relevant metrics: rich status reported heading count but not heading-anchor count after anchor support was added.
- Context: rich/interactive mode should expose the same key metadata counts as stats where practical.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md rich_status_line_reports_offset_viewport_and_total_rows -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: rich status now includes `heading anchors` immediately after `headings`.

## Diff summary

- Code/content commits: `f8cbfbb`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`
- Tests: updated rich status unit expectation to include heading-anchor count.
- Behavioural delta: rich/interactive status now shows heading-anchor coverage.

## Operator-takeaway

The rich pager status line now surfaces heading-anchor metadata without requiring stats, JSON, or anchors mode.
