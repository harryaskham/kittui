#!/usr/bin/env bash
# Showcase kittui as an inline-script UI substrate.
#
# This is intentionally plain bash: it composes first-party kittui chrome and
# inline components into a single terminal dashboard without a Rust app wrapper.
#
# Usage:
#   docs/examples/kittui-inline-script-ui-showcase.sh
#   docs/examples/kittui-inline-script-ui-showcase.sh --graphics --static
#   docs/examples/kittui-inline-script-ui-showcase.sh --text --no-clear
#   docs/examples/kittui-inline-script-ui-showcase.sh --export-dir /tmp/kittui-showcase
#
# Safety:
#   - Outside tmux, auto mode uses kitty graphics (because this is kittui).
#   - Inside tmux, auto mode uses text/ANSI fallback unless --graphics or
#     KITTUI_SHOWCASE_GRAPHICS=1 is set.

set -euo pipefail

MODE="${KITTUI_SHOWCASE_MODE:-auto}"
ANIMATED=1
CLEAR=1
EXPORT_DIR=""
TITLE="kittui-as-inline-script-ui"

usage() {
  sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --graphics) MODE=graphics ;;
    --text|--ansi) MODE=text ;;
    --static) ANIMATED=0 ;;
    --animated) ANIMATED=1 ;;
    --no-clear) CLEAR=0 ;;
    --export-dir)
      shift
      [[ $# -gt 0 ]] || { echo "--export-dir requires a path" >&2; exit 2; }
      EXPORT_DIR="$1"
      ;;
    --title)
      shift
      [[ $# -gt 0 ]] || { echo "--title requires text" >&2; exit 2; }
      TITLE="$1"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

find_kittui() {
  if [[ -n "${KITTUI_BIN:-}" ]]; then
    printf '%s\n' "$KITTUI_BIN"
  elif [[ -x ./target/debug/kittui ]]; then
    printf '%s\n' ./target/debug/kittui
  elif command -v kittui >/dev/null 2>&1; then
    command -v kittui
  else
    printf '%s\n' ""
  fi
}

KITTUI_BIN_RESOLVED="$(find_kittui)"
if [[ -z "$KITTUI_BIN_RESOLVED" ]]; then
  cat >&2 <<'EOF'
Could not find kittui.
Set KITTUI_BIN=/path/to/kittui, install kittui on PATH, or run this from the repo after `cargo build -p kittui-cli --bin kittui`.
EOF
  exit 127
fi

kittui() {
  "$KITTUI_BIN_RESOLVED" "$@"
}

if [[ "$MODE" == "auto" ]]; then
  if [[ -n "${TMUX:-}" && "${KITTUI_SHOWCASE_GRAPHICS:-0}" != "1" ]]; then
    MODE=text
  else
    MODE=graphics
  fi
fi

cols() {
  printf '%s\n' "${COLUMNS:-$(tput cols 2>/dev/null || printf 100)}"
}

lines() {
  printf '%s\n' "${LINES:-$(tput lines 2>/dev/null || printf 32)}"
}

C="$(cols)"
R="$(lines)"
if (( C < 84 )); then C=84; fi
if (( R < 28 )); then R=28; fi

cup() {
  # 1-based row/col, works even when TERM lacks terminfo.
  printf '\033[%s;%sH' "$1" "$2"
}

sgr() { printf '\033[%sm' "$1"; }
reset() { sgr 0; }
bold() { sgr 1; }
dim() { sgr 2; }
cyan() { sgr '38;2;136;192;208'; }
green() { sgr '38;2;163;190;140'; }
yellow() { sgr '38;2;235;203;139'; }
magenta() { sgr '38;2;180;142;173'; }
muted() { sgr '38;2;129;161;193'; }

maybe_clear() {
  if (( CLEAR )); then
    printf '\033[2J\033[H'
  fi
}

anim_args=()
if (( ANIMATED )); then
  anim_args=(--animated)
fi

render_at() {
  local row="$1" col="$2"
  shift 2
  cup "$row" "$col"
  kittui "$@"
}

text_at() {
  local row="$1" col="$2"
  shift 2
  cup "$row" "$col"
  printf '%s' "$*"
}

inline_row() {
  local format="$1" style="$2" tone="$3"
  shift 3
  kittui inline row --format "$format" --style "$style" --tone "$tone" --gap 1 "$@"
}

text_inline_row() {
  kittui inline row --format ansi --style chrome --tone assistant --gap 1 "$@" || printf '%s' "$*"
}

draw_text_showcase() {
  maybe_clear
  bold; cyan; printf '%s\n' "$TITLE"; reset
  text_inline_row \
    --item badge:TEXT \
    --item chip:tmux-safe \
    --item segment:prompt/statusline \
    --item divider:12:━ \
    --item badge:static
  printf '\n\n'
  cat <<'EOF'
╭──────────────────────────── controls / prompt builder ───────────────────────────╮
│ [▶ Run] [■ Stop] [☑ animated] [◉ nord]                                           │
│ query  kittui inline row --item chip:main --item badge:dirty --item segment:60fps │
│ slider fps  0 ━━━━━━━━━━━━━━━●━━━━ 60                                            │
│ styles glass | chrome | metal | neon                                             │
╰──────────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────── render pipeline ──────────────────────────────────────╮
│ Scene JSON → CPU/GPU renderer → kitty transport → prompt/statusline/footer        │
│ title-bar, panel, divider, chip, wm-chrome, inline badge/chip/segment/divider     │
╰──────────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────── status / logs ────────────────────────────────────────╮
│ ✓ animation constants 60fps / 180 frames / 3000ms                                 │
│ ✓ safe text fallback in tmux unless --graphics is explicit                        │
│ ✓ optional --export-dir writes animated render frames + manifest                  │
╰──────────────────────────────────────────────────────────────────────────────────╯
EOF
  printf '\n'
  text_inline_row \
    --item badge:footer \
    --item chip:kittui \
    --item segment:inline-script-ui \
    --item divider:16:─ \
    --item badge:"$(date +%H:%M)"
  printf '\n'
}

export_animation_frames() {
  [[ -n "$EXPORT_DIR" ]] || return 0
  mkdir -p "$EXPORT_DIR/frames"
  local scene_json="$EXPORT_DIR/animated-panel.scene.json"
  local manifest="$EXPORT_DIR/manifest.json"
  kittui panel --tone assistant -w 48 -h 8 --animated --scene-json > "$scene_json"
  if kittui render "$scene_json" --out-dir "$EXPORT_DIR/frames" --manifest "$manifest" > /dev/null 2> "$EXPORT_DIR/render-export.err"; then
    cup "$((R - 1))" 1
    dim; printf 'exported animated scene frames to %s/frames (manifest: %s)' "$EXPORT_DIR" "$manifest"; reset
  else
    cup "$((R - 1))" 1
    yellow; printf 'export skipped: this kittui binary does not support single animated Scene --out-dir export yet'; reset
    dim; printf ' (details: %s/render-export.err)' "$EXPORT_DIR"; reset
  fi
}

draw_graphics_showcase() {
  maybe_clear

  local left_w gap right_x right_w main_top main_h right_top_h right_bottom_y right_bottom_h footer_y
  gap=2
  left_w=$(( C * 38 / 100 ))
  right_x=$(( left_w + gap + 1 ))
  right_w=$(( C - left_w - gap ))
  main_top=5
  main_h=$(( R - 10 ))
  if (( main_h < 16 )); then main_h=16; fi
  right_top_h=$(( main_h / 2 - 1 ))
  if (( right_top_h < 7 )); then right_top_h=7; fi
  right_bottom_y=$(( main_top + right_top_h + 1 ))
  right_bottom_h=$(( main_h - right_top_h - 1 ))
  if (( right_bottom_h < 7 )); then right_bottom_h=7; fi
  footer_y=$(( main_top + main_h + 1 ))

  # Global chrome: animated titlebar, live inline row, and divider.
  render_at 1 1 title-bar -w "$C" -h 1 --left '#5e81ac' --right '#b48ead' "${anim_args[@]}"
  text_at 1 3 "$(bold)$(cyan)$TITLE$(reset)"
  text_at 1 $(( C - 31 )) "$(dim)bash + kittui chrome + inline$(reset)"

  render_at 2 2 inline row --item badge:LIVE --item chip:kitty-graphics --item segment:60fps/180f/3s --item divider:10:━ --item badge:scriptable --style neon --tone assistant "${anim_args[@]}"
  render_at 3 1 divider -w "$C" --left '#88c0d0' --right '#bf616a' "${anim_args[@]}"

  # Partitioned dashboard panels.
  render_at "$main_top" 1 panel --tone assistant -w "$left_w" -h "$main_h" "${anim_args[@]}"
  render_at "$main_top" 1 wm-chrome -w "$left_w" -h "$main_h" --title prompt-builder --focused "${anim_args[@]}"

  render_at "$main_top" "$right_x" panel --tone tool -w "$right_w" -h "$right_top_h" "${anim_args[@]}"
  render_at "$main_top" "$right_x" wm-chrome -w "$right_w" -h "$right_top_h" --title render-pipeline "${anim_args[@]}"

  render_at "$right_bottom_y" "$right_x" panel --tone user -w "$right_w" -h "$right_bottom_h" "${anim_args[@]}"
  render_at "$right_bottom_y" "$right_x" wm-chrome -w "$right_w" -h "$right_bottom_h" --title status-and-logs --floating "${anim_args[@]}"

  # Left pane contents: control-like affordances, prompt ingredients, styles.
  text_at $((main_top + 1)) 3 "$(bold)Controls as inline shell UI$(reset)"
  render_at $((main_top + 2)) 3 inline row --item chip:'▶ Run' --item chip:'■ Stop' --item badge:'☑ Anim' --item badge:'◉ Nord' --style chrome --tone tool "${anim_args[@]}"
  text_at $((main_top + 4)) 3 "$(muted)input$(reset)  $(sgr '48;2;24;29;39;38;2;216;222;233') kittui inline row --item chip:branch --item badge:dirty $(reset)"
  text_at $((main_top + 6)) 3 "$(muted)slider$(reset) fps 0 $(cyan)━━━━━━━━━━━━━━━●━━━━$(reset) 60"
  text_at $((main_top + 8)) 3 "$(muted)tabs$(reset)   $(sgr '48;2;94;129;172;38;2;236;239;244') prompt $(reset) $(sgr '48;2;59;66;82;38;2;216;222;233') tmux $(reset) $(sgr '48;2;59;66;82;38;2;216;222;233') footer $(reset)"
  text_at $((main_top + 10)) 3 "$(muted)styles$(reset)"
  render_at $((main_top + 11)) 3 inline chip --text glass --style glass --tone assistant "${anim_args[@]}"
  render_at $((main_top + 11)) 13 inline chip --text chrome --style chrome --tone tool "${anim_args[@]}"
  render_at $((main_top + 12)) 3 inline badge --text metal --style metal --tone user "${anim_args[@]}"
  render_at $((main_top + 12)) 13 inline segment --text neon --style neon --tone assistant "${anim_args[@]}"
  render_at $((main_top + 14)) 3 inline divider --width "$(( left_w - 8 ))" --glyph ─ --style neon --tone assistant "${anim_args[@]}"

  # Right top: render pipeline and primitive chrome swatches.
  text_at $((main_top + 1)) $((right_x + 2)) "$(bold)Scene → Renderer → Transport$(reset)"
  render_at $((main_top + 2)) $((right_x + 2)) inline row --item segment:'Scene JSON' --item divider:4:→ --item segment:'CPU/GPU' --item divider:4:→ --item segment:'kitty' --style metal --tone tool "${anim_args[@]}"
  text_at $((main_top + 4)) $((right_x + 2)) "$(muted)primitive chrome swatches$(reset)"
  render_at $((main_top + 5)) $((right_x + 2)) chip -w 12 -h 1 --bg '#5e81ac' --border '#88c0d0' "${anim_args[@]}"
  text_at $((main_top + 5)) $((right_x + 4)) "chip"
  render_at $((main_top + 5)) $((right_x + 17)) glow -w 10 -h 1 --color '#a3be8c' "${anim_args[@]}"
  text_at $((main_top + 5)) $((right_x + 19)) "glow"
  render_at $((main_top + 5)) $((right_x + 30)) box -w 10 -h 1 --fg '#ebcb8b' --bg '#3b4252' --radius 6 --border 1 "${anim_args[@]}"
  text_at $((main_top + 5)) $((right_x + 32)) "box"
  render_at $((main_top + 6)) $((right_x + 2)) gradient -w "$(( right_w - 6 ))" -h 1 --left '#81a1c1' --right '#b48ead' "${anim_args[@]}"
  text_at $((main_top + 6)) $((right_x + 4)) "animated gradient divider"

  # Right bottom: status/log surface.
  text_at $((right_bottom_y + 1)) $((right_x + 2)) "$(bold)Status surface / footer ingredients$(reset)"
  render_at $((right_bottom_y + 2)) $((right_x + 2)) inline row --item badge:OK --item chip:'hashes fixed' --item segment:'Nix + Cargo' --style glass --tone assistant "${anim_args[@]}"
  text_at $((right_bottom_y + 4)) $((right_x + 2)) "$(green)✓$(reset) title-bar / panel / wm-chrome / footer"
  text_at $((right_bottom_y + 5)) $((right_x + 2)) "$(green)✓$(reset) chip / badge / segment / divider rows"
  text_at $((right_bottom_y + 6)) $((right_x + 2)) "$(green)✓$(reset) animation metadata + terminal-side loops"
  if [[ -n "$EXPORT_DIR" ]]; then
    text_at $((right_bottom_y + 7)) $((right_x + 2)) "$(yellow)↳$(reset) exporting animated frames after draw"
  fi

  # Footer chrome.
  render_at "$footer_y" 1 divider -w "$C" --left '#a3be8c' --right '#ebcb8b' "${anim_args[@]}"
  render_at $((footer_y + 1)) 2 inline row --item badge:footer --item chip:'prompt-safe modes: zsh/bash/tmux/plain' --item segment:'--static for low-bandwidth' --item divider:8:─ --item badge:"$(date +%H:%M)" --style chrome --tone user "${anim_args[@]}"

  export_animation_frames
  cup "$R" 1
  reset
  printf '\n'
}

case "$MODE" in
  graphics) draw_graphics_showcase ;;
  text) draw_text_showcase ;;
  *) echo "invalid mode: $MODE" >&2; exit 2 ;;
esac
