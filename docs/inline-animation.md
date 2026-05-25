# Animated inline kittui affordances

`kittui inline` components can emit kitty-native animation frames for prompt and
statusline graphics. The CLI renders every frame once, uploads them in one shot,
and asks the terminal to loop the animation. After that, the animation continues
inside the terminal even if the `kittui` process has exited.

## Supported elements

All first-party inline graphics elements support the same animation flags:

- `kittui inline chip`
- `kittui inline badge`
- `kittui inline segment`
- `kittui inline divider`
- `kittui inline row`

Top-level affordance and primitive scene commands also support the same flags:

- `kittui panel`
- `kittui chip`
- `kittui divider`
- `kittui title-bar`
- `kittui box`
- `kittui gradient`
- `kittui glow`
- `kittui image`
- `kittui wm-chrome`
- `kittui wm-session`

Text fallbacks (`--format plain`, `--format ansi`, `--format tmux`) remain static.
Kitty/prompt formats (`kitty`, `prompt-zsh`, `prompt-bash`) can animate.

## Flags

- `--animated` enables kitty-native animation.
- `--fps <n>` sets playback rate. Default: `60`.
- `--frames <n>` sets frames in one loop. Default: `180`.

The default period is therefore `180 / 60 = 3s`. The animation uses a looping
pulse phase curve, so frame 0 and the final frame meet cleanly without a hard cut.

## Style effects

Each inline style maps to a labelled phase-reactive scene layer:

| Style | Effect layer | Visual intent |
| --- | --- | --- |
| `glass` | `inline-effect-glass-glare` | periodic soft glare |
| `neon` | `inline-effect-neon-pulse` | pulsing glow |
| `metal` | `inline-effect-metal-reflection` | reflection shimmer |
| `chrome` | `inline-effect-chrome-sheen` | crisp travelling sheen |

These layers are only added for animated scenes; non-animated scenes keep the
same compact static layer set.

Top-level affordance commands use labelled pulse/glow layers named
`affordance-panel-animation`, `affordance-chip-animation`,
`affordance-divider-animation`, and `affordance-title-bar-animation`.
Primitive scene commands use `primitive-box-animation`,
`primitive-gradient-animation`, and `primitive-glow-animation`. Image scenes use
`image-animation`. WM chrome scene commands use `wm-chrome-animation` and
`wm-session-animation`.

## Examples

```sh
# Default 3-second glass glare loop.
kittui inline chip --text "main" --animated

# Neon prompt-safe animation for zsh.
PROMPT='$(kittui inline chip --format prompt-zsh --style neon --text "dev" --animated) %~ %# '

# Bash prompt-safe animation.
PS1='$(kittui inline badge --format prompt-bash --style glass --text "ok" --animated) \w \$ '

# Animated row: chip + divider + segment, 60fps and 180 frames explicitly.
kittui inline row \
  --item chip:main \
  --item divider:4 \
  --item segment:dev \
  --animated --fps 60 --frames 180

# Top-level affordance and primitive scene commands use the same contract.
kittui chip -w 10 --bg '#001122' --border '#00d8ff' --animated
kittui panel -w 40 -h 8 --animated --fps 60 --frames 180
kittui gradient -w 40 --animated
kittui glow -w 20 -h 4 --animated
kittui image --src ./badge.png -w 20 -h 8 --animated
kittui wm-chrome -w 40 -h 6 --title logs --focused --animated
```

## Inspection

Use `--scene-json` to inspect the generated scene without uploading it:

```sh
kittui inline chip --text main --style metal --animated --scene-json
```

Dry-run JSON reports animation metadata:

```sh
kittui --json --dry-run inline chip --text main --animated
```

The payload contains:

- `inline_animated: true`
- `inline_animation.fps`
- `inline_animation.frames`
- `inline_animation.cycle_ms`
- `inline_animation.loops` (`0` means loop forever)

## Offline frame export

`kittui render` can turn an animated scene JSON artifact into one PNG per
animation frame. This is useful for visual QA, golden fixtures, and checking a
loop without relying on live terminal playback.

```sh
# Build an animated scene artifact.
kittui inline chip --text main --style glass --animated --scene-json > /tmp/main-chip.scene.json

# Export every frame to PNG files.
kittui render /tmp/main-chip.scene.json --out-dir /tmp/main-chip-frames \
  --manifest /tmp/main-chip-frames/manifest.json
```

A single animated scene writes files named `frame-00000.png`, `frame-00001.png`,
and so on. The manifest includes frame count, pixel dimensions, loop count,
per-frame byte sizes, per-frame `delay_ms`, and output paths. Add `--json` to
print that metadata to stdout as well, or `--json-bytes` to include base64 PNG
bytes in JSON dry-runs.

Static single-scene rendering remains unchanged: use `--out FILE` to write a
single PNG. Passing `--out-dir` for a non-animated single scene returns a clear
error because there are no animation frames to enumerate.
