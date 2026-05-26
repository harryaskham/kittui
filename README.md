# kittui

`kittui` is a Rust-native renderer for terminal graphics via the kitty graphics
protocol. It ships as:

- a Rust crate (`kittui`) — `Runtime`, `Scene`, builders, and `Runtime::place_at` for moving cached scenes without re-rendering.
- a C ABI shared library (`kittui-ffi`) — `libkittui_ffi.{so,dylib,dll}` for
  TypeScript / Python / Lua / shell callers.
- platform bindings: `bindings/ts` (`koffi`) and `bindings/python` (`ctypes`)
  wrap the C ABI for JS/Python hosts, including render-only PNG bytes,
  render-many manifests with base64 PNG entries, batch placement, and
  channelized `upload` / `placement` / `embed` output via
  `kittui_place_many_json_channels` / `placeManyChannels`.
- a CLI (`kittui`) — `kittui box`, `kittui gradient`, `kittui glow`,
  `kittui panel --tone assistant`, `kittui chip`, `kittui divider`,
  `kittui title-bar`, `kittui wm-chrome`, `kittui wm-session`, `kittui image --src -`, `kittui place --id 0x...`,
  `kittui delete --id 0x...`, `kittui compose scene.json`, `kittui render
  scene.json --out preview.png --manifest preview.json`, `kittui render scenes.json --out-dir previews/ --manifest previews/manifest.json`,
  and shell pipelines such as `kittui box --scene-json | kittui compose -`.
- a ratatui adapter (`ratakittui`) — widget decoration + lifecycle.

See [`DESIGN.md`](DESIGN.md) for the full design, and [`docs/README.md`](docs/README.md) for the docs map covering semantic surfaces, browser/accessibility adapters, graphics transport, and kittwm SDK architecture.

## Quick start

```sh
cargo run -p kittui-cli -- box -w 60 -h 8 --fg "#00d8ff" --bg "#08111fcc" --radius 6
cargo run -p kittui-cli -- box -w 20 -h 4 --scene-json | cargo run -p kittui-cli -- compose - --dry-run --json
cargo run -p kittui-cli -- box -w 20 -h 4 --scene-json | cargo run -p kittui-cli -- render - --out /tmp/kittui-preview.png
printf '[%s]' "$(cargo run -q -p kittui-cli -- box -w 8 -h 2 --scene-json)" | cargo run -q -p kittui-cli -- render - --out-dir /tmp/kittui-previews
PYTHONPATH=bindings/python python3 -m kittui --find-library
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
| `kittui-cli`         | `kittui`, `kittwm`, `kittwm-browser`, `kittwm-terminal`, `kittwm-launch` binaries + examples  |
| `kittwm-sdk`         | typed client/window/surface handles for kittwm's socket/DISPLAY control plane |
| `kittui-ffi`         | `libkittui_ffi` cdylib + staticlib                       |
| `bindings/ts`        | TypeScript/JavaScript koffi binding over the C ABI        |
| `bindings/python`    | Python stdlib ctypes binding over the C ABI               |
| `ratakittui`         | ratatui adapter (decoration + lifecycle scaffold)        |

## Status

v0.3: kittwm now includes backend-independent native app foundations:

- `kittwm` with no backend flags starts a native PTY terminal sized to the host terminal; `Ctrl-A %` creates side-by-side panes, `Ctrl-A -` creates stacked panes, `Ctrl-A +/-` resizes the focused pane weight, `Ctrl-A [`/`]` (or `,`/`.`) moves the focused pane, `Ctrl-A b` / socket `BALANCE_PANES` balances pane weights, `Ctrl-A Tab` cycles focus, and `Ctrl-A x` closes the focused pane. Native PTY rendering handles scroll regions, DEC origin mode, and application cursor-key mode for TUI body/status/input layouts.
- PTY children inherit `KITTWM_SOCKET`, `KITTWM_DISPLAY`, `KITTUI_WM_DISPLAY`, and `KITTWM_WINDOW`. `KITTWM_TERMINAL_CMD` (or `KITTWM_TERMINAL_BINARY` for config-system handoff) selects what `Ctrl-A t` launches; set `KITTWM_TERMINAL_BACKEND=ghostty` or `KITTWM_TERMINAL_APP=ghostty` to render that terminal through the libghostty-backed surface while keeping the normal kittwm chrome/layout.
- `kittwm replace ...` can exec in the current window context or ask a socket context to spawn.
- `kittwm-browser` is a first-class native browser app backed by local headless Chrome screenshots and DevTools input.
- X backends include FakeServer, Xvfb, Quartz/SCK, and XQuartz wrapper support.
  On macOS, XQuartz proof runs require host-installed XQuartz and xterm
  (`brew install --cask xquartz && brew install xterm`); see `docs/wm.md`.

Try:

```sh
# Nix flakes expose kittui plus explicit kittwm/kittwm-browser/kittwm-terminal/kittwm-launch app targets.
# The kittui package builds kittwm with platform-native backend features by
# default: SCK/Quartz on macOS, Xvfb on Linux.
nix run .#kittui -- --help
nix run .#kittwm
KITTWM_TERMINAL_CMD=htop nix run .#kittwm
nix run .#kittwm-browser -- https://example.com
nix run .#kittwm-terminal -- --title shell --command 'zsh -l'
nix run .#kittwm-launch -- --terminal --title monitor -- htop

cargo run -p kittui-cli --bin kittwm
KITTWM_TERMINAL_CMD=htop cargo run -p kittui-cli --bin kittwm
KITTWM_TERMINAL_BACKEND=ghostty KITTWM_TERMINAL_CMD='zsh -l' cargo run -p kittui-cli --bin kittwm
cargo run -p kittui-cli --bin kittwm-browser -- https://example.com
# From another shell while native kittwm is running:
kittwm --spawn-pty htop
kittwm --resize-pane focused +2
kittwm --move-pane focused last
kittwm --balance-panes
kittwm --send-line focused 'echo hello from controller'
kittwm --send-key focused ctrl-c
kittwm --send-mouse focused press-left 7 9
kittwm --send-bytes-b64 focused aGkKAA==
kittwm --send-file focused ./payload.txt
kittwm --paste-file focused ./payload.txt
kittwm --read-text focused
kittwm --read-scrollback focused
kittwm --wait-text focused ready
kittwm --wait-output focused 'previous output'
kittwm --wait-text-ms 15000 focused 'build finished'
kittwm --wait-output-ms 15000 focused 'scrolled sentinel'
kittwm --save-session session.json
kittwm --restore-session session.json
kittwm --session-json  # persistence-oriented layout/order/focus manifest
kittwm --panes-json    # includes weight, pid/command, and title/app cell geometry
kittwm --events-ms 5000 # stream JSON status/pane/focus/layout events for SDK clients
kittwm --attach -c 'RESTORE_SESSION_JSON {"layout":"rows","panes":[{"command":"htop","title":"htop","weight":1,"focused":true}]}'
```

### kittui-md Markdown viewer

`kittui-md` is the standalone Markdown viewer built on the optional
`kittui-affordances` component layer. It can be used as a normal terminal
program outside kittwm, or from inside a kittwm native terminal.

```sh
cargo run -p kittui-cli --bin kittui-md -- docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --plain docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --mode components-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --components docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --widgets docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --components-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --outline docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --toc docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --headings docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --outline-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --anchors docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --slugs docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --anchors-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --links docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --urls docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --links-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --references docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --refs docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --references-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --footnotes docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --notes docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --footnotes-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --images docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --pictures docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --images-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --tables docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --grid docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --tables-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --code-blocks docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --snippets docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --code-blocks-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --metadata-blocks docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --metadata docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --frontmatter docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --metadata-blocks-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --definitions docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --glossary docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --definitions-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --math docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --equations docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --math-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --html docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --markup docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --html-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --modes
cargo run -p kittui-cli --bin kittui-md -- --modes-json
cargo run -p kittui-cli --bin kittui-md -- --schemas-json
cargo run -p kittui-cli --bin kittui-md -- --mode-info widgets
cargo run -p kittui-cli --bin kittui-md -- --mode-info-json stats-json
cargo run -p kittui-cli --bin kittui-md -- --mode-search table
cargo run -p kittui-cli --bin kittui-md -- --mode-search-json json
cargo run -p kittui-cli --bin kittui-md -- --mode-category inspect
cargo run -p kittui-cli --bin kittui-md -- --mode-category-json json
cargo run -p kittui-cli --bin kittui-md -- --mode-categories
cargo run -p kittui-cli --bin kittui-md -- --mode-categories-json
cargo run -p kittui-cli --bin kittui-md -- --about
cargo run -p kittui-cli --bin kittui-md -- --about-json
cargo run -p kittui-cli --bin kittui-md -- --capabilities
cargo run -p kittui-cli --bin kittui-md -- --capabilities-json
cargo run -p kittui-cli --bin kittui-md -- --version
cargo run -p kittui-cli --bin kittui-md -- --version-json
cargo run -p kittui-cli --bin kittui-md -- --input-formats
cargo run -p kittui-cli --bin kittui-md -- --input-formats-json
cargo run -p kittui-cli --bin kittui-md -- --output-formats
cargo run -p kittui-cli --bin kittui-md -- --output-formats-json
cargo run -p kittui-cli --bin kittui-md -- --defaults
cargo run -p kittui-cli --bin kittui-md -- --defaults-json
cargo run -p kittui-cli --bin kittui-md -- --examples
cargo run -p kittui-cli --bin kittui-md -- --examples-json
cargo run -p kittui-cli --bin kittui-md -- --limits
cargo run -p kittui-cli --bin kittui-md -- --limits-json
cargo run -p kittui-cli --bin kittui-md -- --keybindings
cargo run -p kittui-cli --bin kittui-md -- --keybindings-json
cargo run -p kittui-cli --bin kittui-md -- --exit-codes
cargo run -p kittui-cli --bin kittui-md -- --exit-codes-json
cargo run -p kittui-cli --bin kittui-md -- --counts docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --counts-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --stats docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --summary docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --stats-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --metadata-json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --json docs/examples/kittui-md-proof.md
cargo run -p kittui-cli --bin kittui-md -- --interactive docs/examples/kittui-md-proof.md
```

Modes:

- `--rich` (default) renders kittui/kitty graphics components with text overlays.
- `--mode NAME` selects any output mode by canonical name or alias (for example
  `--mode components-json`, `--mode widgets`, or `--mode --stats-json`).
- `--plain` prints component records and metadata sections for text-only logs,
  including optional link/image title attributes when present.
- `--components` prints only generated component records for conversion inspection;
  `--widgets` is a friendly alias for the same mode. `--components-json` emits
  the same component records as machine-readable JSON.
- `--interactive` opens a raw-mode rich pager for file inputs; use `j/k`, arrow
  keys, PageUp/PageDown, Home/End, `g/G`, `h`/`?` for in-pager help, `o` to
  toggle the document outline, `l` to inspect document links, `i` to inspect
  document image references, `t` to inspect parsed table summaries, `s` to
  inspect code snippets, `f` to inspect footnotes, `d` to inspect definition-list
  entries, `m` to inspect math expressions, `x` to inspect preserved HTML,
  `r` to reload the file from disk, `c` to clear the current status message, and `q`.
  The footer shows the source path, current offset/max offset, viewport size,
  and total rendered rows; reloads also report an in-pager status message for
  both success and transient file errors.
- `--outline` prints only the heading outline plus stable `#anchor` slugs for
  quick document scanning; `--toc` and `--headings` are friendly aliases for the
  same mode. `--outline-json` emits the same heading outline records as
  machine-readable JSON.
- `--anchors` prints only heading levels, stable anchor slugs, and heading text
  for navigation/indexing tools; `--slugs` is a concise alias for the same mode.
  `--anchors-json` emits the same anchor records as machine-readable JSON.
- `--links` prints only parsed Markdown links with labels, URLs, and optional
  title attributes; `--urls` is a friendly alias for the same mode.
  `--links-json` emits the same link records as machine-readable JSON.
- `--references` prints only links, image references, footnote references, and
  footnote definitions for a human-readable reference audit, including optional
  link/image title attributes when present; `--refs` is a concise alias for the
  same mode. `--references-json` emits the same combined reference records as
  machine-readable JSON.
- `--footnotes` prints only footnote references and definitions; `--notes` is
  a friendly alias for the same mode. `--footnotes-json` emits the same
  footnote reference/definition records as machine-readable JSON.
- `--images` prints only parsed image references with alt text, URLs, and
  optional title attributes; `--pictures` is a friendly alias for the same mode.
  `--images-json` emits the same image records as machine-readable JSON.
- `--tables` prints parsed table rows, alignments, column widths, and footprint
  metrics for table layout debugging; `--grid` is a friendly alias for the same
  mode. `--tables-json` emits the same table records as machine-readable JSON.
- `--code-blocks` prints only parsed code blocks with language labels and source
  text for snippet extraction; `--snippets` is a friendly alias for the same
  mode. `--code-blocks-json` emits the same code block records as
  machine-readable JSON.
- `--metadata-blocks` prints only YAML/pluses metadata/frontmatter blocks with
  delimiter kind and source; `--metadata` and `--frontmatter` are friendly
  aliases for the same inspection mode. `--metadata-blocks-json` emits the same
  metadata/frontmatter block records as machine-readable JSON.
- `--definitions` prints only definition-list term/body pairs for glossary
  inspection; `--glossary` is a friendly alias for the same mode.
  `--definitions-json` emits the same definition records as machine-readable JSON.
- `--math` prints only inline/display math expressions with kind and source;
  `--equations` is a friendly alias for the same mode. `--math-json` emits the
  same math records as machine-readable JSON.
- `--html` prints only preserved inline/block HTML placeholders with kind and
  source; `--markup` is a friendly alias for the same mode. `--html-json`
  emits the same HTML fragment records as machine-readable JSON.
- `--modes` lists available output modes, aliases, and descriptions without
  reading a document; `--modes-json` emits the same mode catalog as JSON.
  `--schemas-json` emits a compact catalog of JSON output modes, categories,
  and top-level keys for tooling discovery. `--mode-info NAME` and
  `--mode-info-json NAME` describe one mode by canonical name or alias.
  `--mode-search QUERY` and `--mode-search-json QUERY` search modes by flag,
  alias, or description; `--mode-categories` and `--mode-categories-json` list
  supported categories and counts, while `--mode-category CATEGORY` and
  `--mode-category-json CATEGORY` list modes in one category. JSON mode catalog,
  mode-info, and search results include mode categories plus schema summaries for
  matching JSON output modes. `--about`
  and `--about-json` report the binary version, default mode, and high-level
  capabilities without reading a document. `--capabilities` and
  `--capabilities-json` list just the high-level capability names; `--version`
  and `--version-json` report just the binary/package version. `--input-formats`
  and `--input-formats-json` list supported input formats and extensions;
  `--output-formats` and `--output-formats-json` list supported output families;
  `--defaults` and `--defaults-json` report default mode/input/width settings;
  `--examples` and `--examples-json` list common invocations; `--limits` and
  `--limits-json` list numeric CLI bounds; `--keybindings` and
  `--keybindings-json` list interactive pager controls; `--exit-codes` and
  `--exit-codes-json` list process exit code meanings.
- `--counts` prints only concise component/metadata counts; `--counts-json`
  emits the same counts as a minimal machine-readable JSON object.
- `--stats` prints concise source path/size, render width,
  component/metadata counts (including heading-anchor count) for quick checks;
  `--summary` is a friendly alias for the same mode. `--stats-json` emits the
  same source/render/count summary as compact machine-readable JSON.
- `--metadata-json` emits schema-versioned JSON for tools; `--json` is a
  concise alias for the same mode. It includes top-level document counts, source
  byte/line/path data, render mode/width, indexed component details, indexed
  outline entries with stable anchors, indexed links and images (including
  optional link/image title attributes), indexed footnotes,
  definitions, math, HTML placeholders, metadata blocks, code blocks, and table
  layout metrics.

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
