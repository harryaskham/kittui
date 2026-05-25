# Session summary — fix inline script showcase animation UX

## Bead

- `bd-ffec0e` — make inline script showcase clean and visibly animated

## Changes

- Updated `docs/examples/kittui-inline-script-ui-showcase.sh` after visual dogfooding feedback.
- Default graphics layout is now `clean`: sequential, screenshot-friendly, and avoids absolutely painting text over graphics.
- The previous absolute/partitioned overlay layout remains available with `--layout absolute` / `--absolute`.
- Animation behavior now stays alive by default and drives an obvious script-side live pulse/status row, because some terminals show kitty animation frame uploads as static images once the producer exits.
- Added controls:
  - `--once` to draw once and exit
  - `--duration SECONDS` to run the live pulse for a bounded time
  - `--live` to force live mode
  - `--kitty-animated` / `--terminal-animated` to opt into kittui terminal-side `--animated` frame uploads
- Default no longer dumps huge terminal-side animation frame sets; kittui chrome/graphics remain in the dashboard, while visible motion is guaranteed by the bash-side live row.

## Validation

- `git diff --check`
- `bash -n docs/examples/kittui-inline-script-ui-showcase.sh`
- `TERM=xterm-256color KITTUI_SHOWCASE_MODE=text docs/examples/kittui-inline-script-ui-showcase.sh --once --no-clear >/tmp/showcase-text-once.out`
- `TERM=xterm-256color KITTUI_SHOWCASE_MODE=text docs/examples/kittui-inline-script-ui-showcase.sh --duration 1 --no-clear >/tmp/showcase-text-live.out`
- `TERM=xterm-kitty KITTUI_SHOWCASE_MODE=graphics docs/examples/kittui-inline-script-ui-showcase.sh --once --no-clear >/tmp/showcase-graphics-once.out`
- `TERM=xterm-kitty KITTUI_SHOWCASE_MODE=graphics docs/examples/kittui-inline-script-ui-showcase.sh --duration 1 --no-clear >/tmp/showcase-graphics-live.out`
