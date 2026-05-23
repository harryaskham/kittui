# Session summary — kittui panel command

## Goal

Expose the existing reusable affordance-layer tonal panel chrome through the `kittui` CLI so shell scripts and external hosts can generate the same native kittui panel scenes used by ratakittui/kittwm-facing affordances.

## Bead(s)

- `bd-4d81bc` — kittui-cli: add tonal panel command backed by affordances

## Before state

- Failing tests: none known.
- Relevant gap: DESIGN/module docs referenced `kittui panel`, and `kittui-affordances::panel_chrome` existed, but the CLI had no `panel` command.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui panel_scene -- --nocapture` passed.
  - `cargo test -p kittui-cli --test panel_command -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui -- panel --tone assistant -w 20 -h 4 --scene-json | python3 -c '...'` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: `kittui panel --tone assistant|tool|user -w W -h H [--animate]` compiles `kittui_affordances::panel_chrome` through `Chrome::to_scene` and then uses existing emit modes including `--scene-json`, `--json`, channel filters, and dry-run.

## Diff summary

- Code/content commit: `8148330`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/panel_command.rs`, `README.md`, `DESIGN.md`
- Behavioural delta: scripts can generate tonal kittui panel chrome directly from CLI using the shared affordance implementation.

## Operator-takeaway

This starts aligning CLI affordances with the reusable kittui chrome layer, useful for both shell renderer workflows and future kittwm chrome/preview tooling.
