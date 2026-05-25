# Session summary — Kittui-rendered native split borders

## Goal

Complete bd-f32b5c by moving kittwm split/pane chrome further toward dogfooded kittui graphics: default graphical chrome should be on, and split panes should get kittui-rendered border/gutter scenes aligned to pane geometry.

## Bead(s)

- `bd-f32b5c` — kittwm: kittui-rendered split borders and gutters

## Before state

- Failing tests: none known for this slice.
- Relevant metrics: graphical/affordance chrome was opt-in via `KITTWM_NATIVE_CHROME_RENDERER=affordance-scene|kittui`; pane graphical scenes only covered title/button-like strips, and were emitted before app frames, so chrome could be obscured by app images.
- Context: lead agent is separately taking `bd-2949e9` to extract reusable Nord/glass style tokens in `kittui-affordances`; this slice avoids style-token extraction and focuses on runtime split border/gutter placement.

## After state

- Failing tests: none observed; targeted validation passed.
- Relevant metrics: graphical chrome is now the default unless explicitly disabled with `KITTWM_NATIVE_CHROME_RENDERER=terminal|text|ansi|ascii|off|0|false`. Each pane contributes a full-footprint `pane-N-border` kittui scene with labelled translucent title gutter and focused/unfocused border stroke layers. Chrome scenes are emitted after app frames so split borders remain visible over pane content.
- Context: changed only `crates/kittui-cli/src/session.rs`; terminal/text fallback remains available through env override and tmux still uses pure terminal renderer.

## Diff summary

- Code/content commits: `2ebf761` (`bd-f32b5c: render native split borders with kittui`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Tests: updated graphical chrome scene test to assert pane border scenes/layers; updated renderer selector test for default-on kittui graphics and explicit off modes.
- Behavioural delta: default kittwm graphics mode now overlays kittui-rendered pane borders/gutters/title backing over app frames; ASCII chrome remains fallback.
- Validation: `cargo test -p kittui-cli native_shell_affordance_renderer_builds_kittui_scenes -- --test-threads=1`; `cargo test -p kittui-cli native_chrome_renderer_selector_defaults_to_kittui_graphics -- --test-threads=1`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

This is a first runtime dogfood step: split pane boundaries are now kittui scene layers in the live graphics path, not only terminal text rows. Follow-on theme/token work can replace the temporary inline colors with shared Nord/glass tokens.
