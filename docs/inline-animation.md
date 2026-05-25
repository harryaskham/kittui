# Animated inline kittui affordances

`kittui inline` components can emit kitty-native animation frames for prompt and
statusline graphics. The CLI renders every frame once, uploads them in one shot,
and asks the terminal to loop the animation. After that, the animation continues
inside the terminal even if the `kittui` process has exited.

## Supported inline elements

All first-party inline graphics elements support the same animation flags:

- `kittui inline chip`
- `kittui inline badge`
- `kittui inline segment`
- `kittui inline divider`
- `kittui inline row`

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
