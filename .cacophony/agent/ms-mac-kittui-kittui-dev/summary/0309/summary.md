# Session summary — kittui wm-session previews

## Goal

Let shell scripts and external renderer workflows turn a kittwm `SESSION_JSON` manifest into kittwm chrome preview/export scenes through the `kittui` CLI.

## Bead(s)

- `bd-a01bd1` — kittui-cli: render kittwm session manifest previews

## Before state

- Failing tests: none known.
- Relevant gap: kittwm could save/restore `SESSION_JSON`, and `kittui wm-chrome` could render one window frame, but there was no kittui-side command that rendered a whole session manifest/layout for previews or exports.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo run -q -p kittui-cli --bin kittui -- --scene-json wm-session /tmp/kittwm-session-preview.json -w 80 -h 24 | rg 'wm-chrome:tiled:logs'` passed.
  - `cargo test -p kittui-cli --bin kittui wm_session_scenes -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: Added `kittui wm-session <session.json|-> -w W -h H`. It parses native kittwm `SESSION_JSON` manifests and builds one `WindowChromeTheme` scene per pane, honoring:
  - `layout` (`columns` / `rows`)
  - pane order
  - pane weights
  - pane titles/window/command fallbacks
  - focused flags
  The command routes through the existing batch emit pipeline, so `--scene-json`, `--json`, dry-run, and placement output work consistently. README and docs/wm now mention `kittui wm-session`.

## Diff summary

- Code/content commit: `760617e`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: kittwm session manifests can be rendered/exported as kittui chrome scene arrays without attaching to a live WM UI.

## Operator-takeaway

`kittui --scene-json wm-session session.json -w 120 -h 30` now previews saved kittwm sessions using the same reusable WM chrome theme as the compositor.
