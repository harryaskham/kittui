# Session summary — inline script UI showcase

## Bead

- `bd-a47a03` — add inline script UI showcase bash script

## Changes

- Added executable `docs/examples/kittui-inline-script-ui-showcase.sh`.
- The script composes a single bash-driven kittui dashboard using:
  - title bar, divider, panel, wm-chrome, chip, gradient, glow, box chrome
  - inline chip/badge/segment/divider/row components
  - control-like shell UI affordances, footer/status area, partitioned panes
  - glass/chrome/metal/neon inline styles and assistant/tool/user tones
  - animated kitty graphics by default outside tmux
- Added safe modes:
  - `--text` / `KITTUI_SHOWCASE_MODE=text`
  - `--graphics` / `KITTUI_SHOWCASE_GRAPHICS=1`
  - `--static`
  - `--no-clear`
  - `--export-dir DIR` to exercise animated scene frame export when supported by the resolved kittui binary

## Validation

- `bash -n docs/examples/kittui-inline-script-ui-showcase.sh`
- `TERM=xterm-256color KITTUI_SHOWCASE_MODE=text docs/examples/kittui-inline-script-ui-showcase.sh --no-clear >/tmp/kittui-showcase-text.out`
- `TERM=xterm-kitty KITTUI_SHOWCASE_MODE=graphics docs/examples/kittui-inline-script-ui-showcase.sh --static --no-clear >/tmp/kittui-showcase-graphics.out`
- `TERM=xterm-kitty KITTUI_SHOWCASE_MODE=graphics docs/examples/kittui-inline-script-ui-showcase.sh --static --no-clear --export-dir /tmp/kittui-showcase-export-test >/tmp/kittui-showcase-export.out`
