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

v0.1: scaffold + CPU renderer + cache + protocol + CLI + showcase + ratakittui
lifecycle skeleton + FFI scaffold all build and test green. GPU backend, full
ratatui decoration matrix, and the tmux border showcase land next per
`DESIGN.md`.
