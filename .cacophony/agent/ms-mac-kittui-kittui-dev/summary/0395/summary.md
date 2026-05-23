# Session summary — opt-in affordance scene chrome renderer

## Goal

Add an opt-in live native chrome path that renders pane/footer chrome through kittui-affordance scenes while preserving default ANSI chrome behavior.

## Bead(s)

- `bd-c327ad` — kittwm: add opt-in affordance scene chrome renderer

## Before state

- Failing tests: none known.
- Relevant context: `bd-4c6401` added a testable `NativeShellView` -> affordance scene helper, but it was not wired into the live native session behind any selector.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_chrome_renderer_selector_is_opt_in -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_shell_affordance_renderer_builds_kittui_scenes -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `KITTWM_NATIVE_CHROME_RENDERER=affordance-scene|affordance_scene|kittui` selector.
  - Default native chrome remains the existing ANSI title/footer path.
  - Opt-in mode renders pane title/footer chrome scenes with `kittui-affordances` controls and places them through the existing `Runtime::place_at` path.
  - Title/footer affordance controls are clamped to one row so they do not intentionally overlap pane app frame placement.
  - Documented the selector in `docs/wm.md`.
  - Coordinated with kittui-dev-2: they handled raw-frame shm transport and closed `bd-4edcb2`.

## Diff summary

- Code/content commit: `806e9062`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `docs/wm.md`
- Behavioural delta: opt-in only via `KITTWM_NATIVE_CHROME_RENDERER`; no default behavior change.

## Operator-takeaway

Native chrome can now be tried through the kittui-affordance scene path without destabilizing the default terminal/ANSI chrome renderer.
