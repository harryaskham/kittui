# Session summary — Inline animation flags

## Goal

Complete `bd-b9864c` by adding shared CLI animation controls for inline kittui elements.

## Bead(s)

- `bd-b9864c` — inline affordances: add shared --animated/--fps/--frames flags

## Inventory filed

Inline elements needing animation support:
- `kittui inline chip`
- `kittui inline badge`
- `kittui inline segment`
- `kittui inline divider`
- `kittui inline row`

Follow-up beads filed:
- `bd-6aecf6` — style-specific animated glare/pulse/reflection effects
- `bd-0696b1` — document and smoke-test animated prompt/statusline output

## Before state

- Inline kitty scenes always had `animation: None`.
- Chip/badge/segment/divider/row had no `--animated`, `--fps`, or `--frames` flags.
- Existing kittui scene/kitty infrastructure already supported all-frames-up-front native kitty animation.

## After state

- Added shared flattened `InlineAnimationArgs` with:
  - `--animated`
  - `--fps` default `60`
  - `--frames` default `180`
- Default animated period is 180 frames at 60fps = 3000ms / 3 seconds.
- Attached `Scene.animation` to inline chip/badge/segment/divider/row kitty scene paths.
- Animation curve is `PhaseCurve::Pulse`, which closes the loop for seamless repeat.
- Dry-run/JSON payloads report `inline_animated` and `inline_animation` metadata.
- Inline examples mention `--animated`, `--fps`, and `--frames`.

## Diff summary

- Code/content commits: `8e7cd04` (`bd-b9864c: add inline animation flags`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`
- Validation:
  - `cargo test -p kittui-cli --bin kittui inline_animation_defaults_to_three_second_looping_period -- --test-threads=1`
  - `cargo test -p kittui-cli --bin kittui inline_animation_flags_clamp_to_safe_kitty_frame_contract -- --test-threads=1`
  - `cargo test -p kittui-cli --bin kittui inline_examples_cover_prompt_status_and_fallback_modes -- --test-threads=1`
  - `cargo check -p kittui-cli`
  - `git diff --check`

## Operator-takeaway

Inline graphics can now opt into kitty-native looping animation with all frames uploaded in one shot; style-specific visual motion remains tracked separately in `bd-6aecf6`.
