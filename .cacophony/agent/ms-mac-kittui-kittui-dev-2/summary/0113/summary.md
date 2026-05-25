# Session summary — Render animated scene frames

## Goal

Continue animation coverage by making `kittui render` useful for animated Scene JSON artifacts.

## Bead(s)

- `bd-163179` — kittui render: export animation frames for animated scenes

## Before state

- Scene builders could emit animated Scene JSON.
- `kittui render` rendered a still PNG for a single scene and rejected `--out-dir` for single scenes.
- There was no offline CLI path to export all frames of an animated scene to PNG files.

## After state

- `kittui render <scene.json> --out-dir DIR` now supports single animated scenes.
- It renders every animation frame via `kittui_render_cpu::render_animation`.
- Outputs `frame-00000.png`, `frame-00001.png`, ... into the directory.
- Manifest / JSON metadata includes:
  - frame count
  - pixel width/height
  - loops
  - per-frame byte counts
  - per-frame delay_ms
  - per-frame output path
  - optional `png_base64` when `--json-bytes` is set
- Static single-scene behavior remains unchanged; `--out-dir` on a non-animated single scene now errors clearly.

## Diff summary

- Code/content commits: `62e678c` (`bd-163179: render animated scene frames`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/main.rs`
  - `crates/kittui-cli/tests/render_command.rs`
- Validation:
  - `cargo test -p kittui-cli --test render_command render_single_animated_scene_writes_frame_directory -- --test-threads=1`
  - `cargo test -p kittui-cli --test render_command render_json_reports_metadata_without_writing_on_dry_run -- --test-threads=1`
  - `cargo check -p kittui-cli`
  - `git diff --check`

## Operator-takeaway

Animated kittui elements can now be exported offline as per-frame PNG artifacts, making visual QA/golden generation possible without terminal playback.
