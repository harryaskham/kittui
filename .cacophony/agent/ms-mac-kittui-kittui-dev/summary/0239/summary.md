# Session summary — compose placement overrides

## Goal

Expose `Runtime::place_at` through the CLI so shell scripts can keep scene JSON scene-local while overriding terminal placement at compose time.

## Bead(s)

- `bd-126c24` — kittui-cli: expose compose placement overrides

## Before state

- Failing tests: none known.
- Relevant gap: `kittui compose` always placed a scene at the footprint embedded in the JSON. Scripts had to mutate scene JSON to move output, which changes scene/cache identity.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui compose_placement -- --nocapture` passed.
  - `cargo test -p kittui-cli --test compose_at -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui -- box -w 4 -h 2 --scene-json | cargo run -q -p kittui-cli --bin kittui -- compose - --x 5 --y 6 --dry-run --json | python3 -c '...'` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: `kittui compose <scene.json>|- --x X --y Y` overrides only terminal placement x/y, preserving the scene's cols/rows and render identity. Existing compose behavior is unchanged without overrides.

## Diff summary

- Code/content commit: `e904dc9`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/compose_at.rs`, `DESIGN.md`
- Behavioural delta: shell pipelines can now move composed scenes without editing the scene JSON.

## Operator-takeaway

This tightens the CLI story around kittui as a renderer substrate: scene JSON can remain reusable, while placement is controlled by the host/script at output time.
