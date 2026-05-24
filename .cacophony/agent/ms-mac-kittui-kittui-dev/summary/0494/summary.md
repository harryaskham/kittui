# Session summary — kittui inline chip renderer

## Goal

Add a first kittui inline component for shell prompts and tmux statuslines: a one-line text chip that renders to plain, ANSI, or tmux status format output.

## Bead(s)

- `bd-101ccb` — kittui: inline chip component for prompts/statuslines

## Before state

- kittui had graphical `chip` scene/chrome generation, but no simple prompt/statusline-oriented inline text component output.
- User wanted a P10K-like shell-prompt use case where kittui can generate styled text chip segments from shell scripts.

## After state

- Added `kittui inline chip --text TEXT`.
- Supported formats:
  - `--format plain` => `[ text ]`
  - `--format ansi` => 24-bit ANSI styled chip text, no trailing newline
  - `--format tmux` => tmux statusline `#[fg=...,bg=...]` syntax, escaping `#`
- Added `--tone assistant|tool|user` and `--padding N`.
- Kept this first slice deterministic and text/statusline based: no kitty/ghostty graphics side effects yet.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_and_tmux_formats -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- Foreground smoke:
  - `target/debug/kittui inline chip --text main --format plain`
  - `target/debug/kittui inline chip --text main --format tmux`
- `git diff --check` passed.

## Components currently visible in code

- Low-level kittui scenes/nodes: boxes/rounded rects, gradients, glow, image placement, composition/rendering.
- Affordance chrome: chip, divider, panel, title-bar, kittwm chrome/session preview.
- Higher-level affordance components: textbox, h1/h2/h3, title, banner, header, footer, textchip.
- Markdown renderer maps headings, paragraphs/text boxes, links/text chips, images/placeholders, tables, lists, code blocks, math/html placeholders, metadata blocks, etc.
