# Session summary — native shell chrome affordance scenes

## Goal

Add an initial testable renderer/helper that maps native kittwm shell chrome into kittui/kittui-affordances scenes, without changing the default live ANSI/terminal renderers.

## Bead(s)

- `bd-4c6401` — kittwm: render native shell chrome as kittui affordance scenes

## Before state

- Failing tests: none known.
- Relevant context: native shell view/chrome existed and pure terminal rendering existed, but live chrome remained direct ANSI/title/footer strings with no kittui-affordance scene bridge.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_shell_affordance_renderer_builds_kittui_scenes -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `NativeShellChromeScene` as a scene+position wrapper for native shell chrome.
  - Added `render_native_shell_view_affordance_scenes(...)`, which maps each pane title to an affordance button scene and the footer to an affordance text-input/status scene.
  - Focused/unfocused pane state is represented through `ControlState`.
  - Added a targeted test proving pane/footer chrome produces primitive kittui scenes with control-background layers.
  - Existing live renderers remain unchanged by default.
  - Coordinated with kittui-dev-2: they are assigned `bd-4edcb2` POSIX shm allocator transport work.

## Diff summary

- Code/content commit: `2c438509`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Behavioural delta: new helper/test only; no default runtime behavior change.

## Operator-takeaway

There is now a first bridge from `NativeShellView` chrome to kittui-affordance scenes. The next chrome step can make this an opt-in live renderer path or move the helper into a more public renderer module.
