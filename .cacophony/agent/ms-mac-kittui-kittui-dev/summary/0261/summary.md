# Session summary — compose batch placement origins

## Goal

Improve kittui as a shell/platform renderer by allowing reusable JSON scene batches to be placed at runtime origins with `kittui compose --x/--y`.

## Bead(s)

- `bd-c53cc6` — kittui-cli: allow compose batch placement origins

## Before state

- Failing tests: none known.
- Relevant gap: `kittui compose` accepted JSON arrays for batch rendering, but rejected `--x/--y` for batches. Scripts could not build a reusable scene group and move the group to a requested terminal origin.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --test compose_batch -- --nocapture` passed.
  - `cargo test -p kittui-cli --test compose_at -- --nocapture` passed.
  - `git diff --check` passed.
- Context: batch `--x/--y` now treats the supplied coordinate as the group origin. The minimum x/y in the batch maps to that origin while other scenes preserve relative offsets. Single-scene override semantics remain unchanged.

## Diff summary

- Code/content commit: `210a02a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/compose_batch.rs`
- Behavioural delta: `kittui compose - --x X --y Y` now works for scene arrays in dry-run/JSON and normal output paths.

## Operator-takeaway

Scene arrays are more useful as reusable shell-renderer templates: scripts can now place a whole batch without rewriting every scene footprint themselves.
