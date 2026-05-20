# Session summary — Image node + CLI completion + FFI completeness

## Goal

Close out the remaining P2 component beads: Image node rendering with the
`kittui image` CLI, the broader v1 CLI flags from DESIGN.md, and the full
FFI ABI surface required by external bindings.

## Bead(s)

- `bd-ca05d4` — Implement Image node rasterization and `kittui image` CLI
- `bd-ac9d7e` — Finish v1 CLI surface from DESIGN.md (dry-run + channel selectors)
- `bd-d5ce20` — Complete FFI ABI surface and generated header workflow

## Before state

- Failing tests: none. But `Node::Image` returned `UnsupportedImage`,
  `kittui-cli` had no `image` subcommand, and there were no global
  `--dry-run` / `--upload-only` / `--placement-only` / `--embed-only`
  flags.
- Relevant metrics: `kittui-ffi` exposed only 5 entry points
  (`abi_version`, `runtime_new`, `runtime_free`, `place_json`,
  `string_free`); no length-explicit byte API, no runtime probe, no
  configure, no last-error reader.

## After state

- Failing tests: none across the workspace.
- Relevant metrics:
  - `kittui-render-cpu` gains `image-decoders` feature (default on)
    backed by the `image` crate; `Node::Image` now rasterizes PNG/JPEG
    paths and inline bytes with `Fit::{Contain,Cover,Stretch,None}` and
    optional `tint` multiplication.
  - New `kittui image --src PATH -w W -h H --fit ... --tint #rrggbb`
    CLI subcommand. New global `--dry-run`, `--upload-only`,
    `--placement-only`, `--embed-only` flags channel emission through a
    single `EmitMode` plumbing.
  - `kittui-ffi` ABI is now 0.2 (minor bump), exporting
    `kittui_abi_version_check`, `kittui_runtime_configure`,
    `kittui_render_json` (explicit length), `kittui_unplace`,
    `kittui_probe_json`, `kittui_last_error`, `kittui_bytes_free` in
    addition to the existing 5. `kittui.h` updated; ABI snapshot test
    enforces both header and symbol expectations.

## Diff summary

- Code/content commits this session:
  - `a7be8f3` — Image node rasterization + kittui image CLI + global emit-channel flags
  - `03622b1` — kittui FFI bytes_free, abi_version_check, last_error, unplace, probe_json, runtime_configure, render_json; ABI bump to 0.2
- Files touched: `crates/kittui-render-cpu/{Cargo.toml, src/lib.rs,
  src/rasterize.rs}`, `crates/kittui-cli/src/main.rs`, `crates/kittui/src/lib.rs`,
  `crates/kittui-ffi/{src/lib.rs, kittui.h, tests/abi_snapshot.rs}`,
  `Cargo.lock`.
- Tests: +1 CPU `image_node_rasterizes_inline_bytes`, +2 FFI suite
  expansion (header symbol list, round-trip place/render/unplace/probe).
- Behavioural delta: kittui now decodes and rasterizes images
  end-to-end on the CPU path; the CLI surfaces them and supports
  channel-selective + dry-run output; the C ABI now mirrors DESIGN.md's
  intended surface area.

## Embedded artefacts

- `screenshots/bd-ca05d4-image-contain.png`,
  `screenshots/bd-ca05d4-image-cover.png` — kittui image command driven
  from the adjacent tmux pane (rendered images are positioned at the
  top of pane 2; visible region in the screenshots is the prompt area).

## Operator-takeaway

The CPU renderer is now feature-complete relative to DESIGN.md's v1
scope: Rect, Gradient, Glow, Scanlines, Image, Group, Composite,
Mask, Clip all rasterize correctly. The CLI exposes the documented
flag surface. The C ABI is broad enough to drive bindings beyond
place_json.
