# Session summary — kittui scene JSON/stdin compose pipeline

## Goal

Start addressing large post-autonomy gaps by making `kittui` more useful as a shell/external-platform renderer: generated scenes can now be emitted as JSON and piped directly into `kittui compose -`.

## Bead(s)

- `bd-e9e815` — kittui-cli: add scene-json/stdin compose pipeline

## Before state

- Failing tests: none known.
- Relevant gap: `DESIGN.md` described scene/compose pipelines, but actual `kittui compose` accepted only paths and scene-generating commands could not print reusable `Scene` JSON.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui scene_json -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittui compose_scene_reader -- --nocapture` passed.
  - `cargo test -p kittui-cli --test scene_pipeline -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui -- box -w 4 -h 2 --scene-json | cargo run -q -p kittui-cli --bin kittui -- compose - --dry-run --json | rg '"dry_run": true|"footprint"'` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: `--scene-json` is a global CLI flag, so simple scene-generating commands can output actual serializable `Scene` JSON. `compose -` reads JSON from stdin.

## Diff summary

- Code/content commit: `a1866f6`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/scene_pipeline.rs`, `README.md`, `DESIGN.md`
- Behavioural delta: shell pipelines like `kittui box --scene-json | kittui compose - --dry-run --json` now work.

## Operator-takeaway

This closes the first gap in making kittui usable as a renderer substrate for scripts and other platforms: scenes can be exported, transformed, and re-composed through stdin rather than requiring out-of-band Rust callers or temporary bespoke files.
