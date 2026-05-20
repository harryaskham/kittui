# kittui

`kittui` is a Rust-native renderer for terminal graphics via the kitty graphics
protocol. It ships as:

- a Rust crate (`kittui`) — `Runtime`, `Scene`, builders.
- a C ABI shared library (`kittui-ffi`) — `libkittui_ffi.{so,dylib,dll}` for
  TypeScript / Python / Lua / shell callers.
- a CLI (`kittui`) — `kittui box`, `kittui gradient`, `kittui glow`,
  `kittui compose scene.json`, etc.
- a ratatui adapter (`ratakittui`) — widget decoration + lifecycle.

See [`DESIGN.md`](DESIGN.md) for the full design.

## Quick start

```sh
cargo run -p kittui-cli -- box -w 60 -h 8 --fg "#00d8ff" --bg "#08111fcc" --radius 6
cargo run -p kittui-cli --example showcase
```

## Configuration

Every user-facing CLI option can be supplied as an explicit flag, a `KITTUI_*`
environment variable, or a YAML default at `$XDG_CONFIG_HOME/kittui/config.yaml`
(falling back to `~/.config/kittui/config.yaml`). Precedence is always:

1. CLI flag / API override
2. environment variable
3. YAML default
4. built-in default

Examples:

```yaml
cache_dir: /var/tmp/kittui-cache
renderer: cpu
terminal_cols: 120
terminal_rows: 40
box:
  width: 60
  height: 8
  fg: "#00d8ff"
gradient:
  direction: vertical
cache:
  budget: 104857600
```

Use variables such as `KITTUI_CACHE_DIR`, `KITTUI_RENDERER`,
`KITTUI_BOX_WIDTH`, `KITTUI_GRADIENT_DIRECTION`, `KITTUI_GLOW_INTENSITY`, and
`KITTUI_CACHE_BUDGET` for script-local scopes. JSON output includes a
`config_sources` object so callers can see whether each resolved value came from
a flag, env var, YAML, or a built-in default.

## Crates

| Crate                | Purpose                                                  |
|----------------------|----------------------------------------------------------|
| `kittui-core`        | Scene, geometry, color, hashing, animation primitives    |
| `kittui-render-cpu`  | Reference CPU rasterizer + PNG/APNG encoders             |
| `kittui-render-gpu`  | wgpu-backed renderer (scaffold)                          |
| `kittui-kitty`       | kitty graphics protocol encoder + placeholder generation |
| `kittui-cache`       | Content-addressed PNG/APNG cache                         |
| `kittui`             | Public facade: `Runtime`, `Placement`, builders          |
| `kittui-cli`         | `kittui` binary + `examples/showcase`                    |
| `kittui-ffi`         | `libkittui_ffi` cdylib + staticlib                       |
| `ratakittui`         | ratatui adapter (decoration + lifecycle scaffold)        |

## Status

v0.2: kitty graphics protocol now spec-conformant and **proven visually**
in Ghostty (and any other kitty-compatible terminal):

- 297-entry unicode placeholder diacritic table with full `(row, col, msb)`
  encoding (spec compliance — previously bare `U+10EEEE` cells).
- `Quiet` `q=1`/`q=2` control to suppress terminal responses (no more
  `Gi=…;ENOENT` lines leaking into output).
- `UploadMedium::{Direct,File,TempFile,SharedMemory}` for `t=d/f/t/s` upload modes.
- Animation: `a=t` then `a=f` frame appends + `a=a` control + per-frame `z=` delays.
- `PlacementOptions` with `placement_id` (`p=`), subcell offset (`X=`/`Y=`),
  z-index, and unicode-placeholder toggle.
- Auto-detect `Transport::TmuxPassthrough` when running inside tmux.
- `kittui proof` CLI walks the full protocol matrix; `cargo test`
  grammar-pins every encoder and regression-tests the proof matrix.

See [docs/protocol-conformance.md](docs/protocol-conformance.md) for the
per-spec-section coverage table.
