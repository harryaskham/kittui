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

## Developer validation notes

The repository currently has a known rustfmt-baseline mismatch: an unconditional
`cargo fmt --check` can report formatting diffs in files unrelated to a narrow
change. Until the baseline is normalized, prefer a touched-file formatting check
for Rust files changed by your branch:

```bash
git diff --name-only --diff-filter=ACMRT origin/main...HEAD -- '*.rs' \
  | xargs -r rustfmt --edition 2021 --check
```

If you intentionally run `cargo fmt`, inspect the diff before committing and
keep only formatting changes that are part of the current task. Do not fold a
large workspace-wide rustfmt sweep into an unrelated kittui/kittwm feature fix.

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
| `kittui-cli`         | `kittui`, `kittwm`, `kittwm-browser` binaries + examples  |
| `kittui-ffi`         | `libkittui_ffi` cdylib + staticlib                       |
| `ratakittui`         | ratatui adapter (decoration + lifecycle scaffold)        |

## Status

v0.3: kittwm now includes backend-independent native app foundations:

- `kittwm` with no backend flags starts a native PTY terminal sized to the host terminal.
- PTY children inherit `KITTWM_SOCKET`, `KITTWM_DISPLAY`, `KITTUI_WM_DISPLAY`, and `KITTWM_WINDOW`.
- `kittwm replace ...` can exec in the current window context or ask a socket context to spawn.
- `kittwm-browser` is a first-class native browser app backed by local headless Chrome screenshots and DevTools input.
- X backends include FakeServer, Xvfb, Quartz/SCK, and XQuartz wrapper support.
  On macOS, XQuartz proof runs require host-installed XQuartz and xterm
  (`brew install --cask xquartz && brew install xterm`); see `docs/wm.md`.

Try:

```sh
cargo run -p kittui-cli --bin kittwm
KITTWM_TERMINAL_CMD=htop cargo run -p kittui-cli --bin kittwm
cargo run -p kittui-cli --bin kittwm-browser -- https://example.com
```

### kittui-md Markdown viewer

`kittui-md` is the standalone Markdown viewer built on the optional
`kittui-affordances` component layer. It can be used as a normal terminal
program outside kittwm, or from inside a kittwm native terminal.

```sh
cargo run -p kittui-cli --bin kittui-md -- docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --plain docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --components docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --outline docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --links docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --references docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --footnotes docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --images docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --tables docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --code-blocks docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --definitions docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --math docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --html docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --stats docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --metadata-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --interactive docs/examples/kittui-md-proof.md
```

Modes:

- `--rich` (default) renders kittui/kitty graphics components with text overlays.
- `--plain` prints component records and metadata sections for text-only logs.
- `--components` prints only generated component records for conversion inspection.
- `--interactive` opens a raw-mode rich pager for file inputs; use `j/k`, arrow
  keys, PageUp/PageDown, Home/End, `g/G`, and `q`.
- `--outline` prints only the heading outline for quick document scanning.
- `--links` prints only parsed Markdown links with labels and URLs.
- `--references` prints only links, image references, footnote references, and
  footnote definitions for a human-readable reference audit.
- `--footnotes` prints only footnote references and definitions.
- `--images` prints only parsed image references with alt text and URLs.
- `--tables` prints parsed table rows, alignments, column widths, and footprint
  metrics for table layout debugging.
- `--code-blocks` prints only parsed code blocks with language labels and source
  text for snippet extraction.
- `--definitions` prints only definition-list term/body pairs for glossary
  inspection.
- `--math` prints only inline/display math expressions with kind and source.
- `--html` prints only preserved inline/block HTML placeholders with kind and
  source.
- `--stats` prints concise source/component/metadata counts for quick checks.
- `--metadata-json` emits schema-versioned JSON for tools. It includes source
  byte/line counts, component details, outline, links, images, footnotes,
  definitions, math, HTML placeholders, code blocks, and table layout metrics.

The proof gallery at `docs/examples/kittui-md-proof.md` exercises headings,
paragraphs, links, images, blockquotes, lists, task lists, fenced code,
definition lists, aligned tables, math, HTML placeholders, footnotes, and the
metadata surfaces above.

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
