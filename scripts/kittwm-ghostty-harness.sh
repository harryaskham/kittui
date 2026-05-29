#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
kittwm-ghostty-harness — Ghostty/kittwm integration evidence runner

Usage:
  scripts/kittwm-ghostty-harness.sh --mode headless -- COMMAND...
  scripts/kittwm-ghostty-harness.sh --mode proof -- COMMAND...
  scripts/kittwm-ghostty-harness.sh --mode timelapse -- COMMAND...
  scripts/kittwm-ghostty-harness.sh --mode sampled -- COMMAND...
  scripts/kittwm-ghostty-harness.sh --mode app -- COMMAND...
  scripts/kittwm-ghostty-harness.sh --mode kittem -- KITtem-ARGS...

Options:
  --mode MODE       headless | proof | timelapse | app | kittem (default: headless)
  --out-dir DIR     Artifact directory (default: /tmp/kittwm-ghostty-harness-$PID)
  --cols N          Terminal columns for libghostty-vt/kittwm proof modes (default: 100)
  --rows N          Terminal rows for libghostty-vt/kittwm proof modes (default: 28)
  --chunk-lines N   Lines per frame in timelapse mode (default: 1)
  --sample-ms N     Milliseconds between frames in sampled mode (default: 250)
  --max-ms N        Maximum sampled-mode runtime before killing child (default: 10000)
  --scroll MODE     top | bottom | current for PNG preview modes (default: current)
  --pty-input TEXT  Input to send to PTY command in headless/proof/timelapse modes (escapes: \n \r \t \e \xHH)
  --pty-input-delay-ms N
                   Delay before sending --pty-input (default: 100)
  --app-name NAME   macOS application name for --mode app (default: Ghostty)
  --keep-app        Do not ask Ghostty.app to quit after --mode app capture
  --help            Show this help

Modes:
  headless   Run COMMAND in a PTY, feed captured VT bytes through kittui-ghostty,
             and write frame.png plus a manifest. Portable and CI-friendly.
  proof      Like headless, but uses kittui-ghostty --kittwm-proof-command with
             kittwm-friendly renderer environment for screenshot evidence.
  timelapse  Run COMMAND once in a PTY and emit frame-*.png plus manifest.json.
  sampled    Run COMMAND in a PTY and sample live VT state every --sample-ms, so
             alternate-screen TUIs can be captured before they exit.
  app        Best-effort macOS Ghostty.app smoke: launch Ghostty.app with COMMAND,
             capture a desktop screenshot, and write a manifest. Requires macOS
             GUI permissions; use as manual/interactive evidence only.
  kittem     Run an installed `kittem` command, capturing stdout/stderr/status in
             the artifact directory for terminal-emulator validation workflows.

Evidence notes:
  - Review generated PNGs before claiming visual proof.
  - Command-log screenshots are validation-only, not UI proof.
  - app mode is intentionally best-effort because macOS screenshot permissions
    and Ghostty.app CLI arguments vary across hosts.
USAGE
}

mode=headless
out_dir="/tmp/kittwm-ghostty-harness-$$"
cols=100
rows=28
scroll=current
chunk_lines=1
sample_ms=250
max_ms=10000
app_name=Ghostty
keep_app=0
pty_input=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      mode="${2:?--mode requires a value}"
      shift 2
      ;;
    --out-dir)
      out_dir="${2:?--out-dir requires a value}"
      shift 2
      ;;
    --cols)
      cols="${2:?--cols requires a value}"
      shift 2
      ;;
    --rows)
      rows="${2:?--rows requires a value}"
      shift 2
      ;;
    --chunk-lines)
      chunk_lines="${2:?--chunk-lines requires a value}"
      shift 2
      ;;
    --sample-ms)
      sample_ms="${2:?--sample-ms requires a value}"
      shift 2
      ;;
    --max-ms)
      max_ms="${2:?--max-ms requires a value}"
      shift 2
      ;;
    --scroll)
      scroll="${2:?--scroll requires a value}"
      shift 2
      ;;
    --pty-input)
      pty_input+=(--pty-input "${2:?--pty-input requires a value}")
      shift 2
      ;;
    --pty-input-delay-ms)
      pty_input+=(--pty-input-delay-ms "${2:?--pty-input-delay-ms requires a value}")
      shift 2
      ;;
    --app-name)
      app_name="${2:?--app-name requires a value}"
      shift 2
      ;;
    --keep-app)
      keep_app=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ $# -eq 0 ]]; then
  echo "missing command; pass it after --" >&2
  usage >&2
  exit 2
fi

mkdir -p "$out_dir"
command_text="$*"

json_escape() {
  python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
}

write_manifest() {
  local status="$1"
  local runner="$2"
  local artifact_json="$3"
  local escaped_command escaped_dir
  escaped_command=$(printf '%s' "$command_text" | json_escape)
  escaped_dir=$(printf '%s' "$out_dir" | json_escape)
  cat > "$out_dir/harness-manifest.json" <<MANIFEST
{
  "kind": "kittwm-ghostty-harness",
  "runner": "$runner",
  "status": $status,
  "command": $escaped_command,
  "out_dir": $escaped_dir,
  "cols": $cols,
  "rows": $rows,
  "artifacts": $artifact_json
}
MANIFEST
}

run_cargo_ghostty() {
  local ghostty_args=("$@")
  if [[ "${KITTWM_GHOSTTY_USE_NIX:-auto}" != "0" ]] && command -v nix >/dev/null 2>&1; then
    RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 \
      nix develop --command cargo run -p kittui-ghostty-vt --bin kittui-ghostty -- "${ghostty_args[@]}" \
      >"$out_dir/kittui-ghostty.stdout.txt" \
      2>"$out_dir/kittui-ghostty.stderr.txt"
  else
    RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 \
      cargo run -p kittui-ghostty-vt --bin kittui-ghostty -- "${ghostty_args[@]}" \
      >"$out_dir/kittui-ghostty.stdout.txt" \
      2>"$out_dir/kittui-ghostty.stderr.txt"
  fi
}

case "$mode" in
  headless)
    frame="$out_dir/frame.png"
    set +e
    run_cargo_ghostty --pty-command "$command_text" "${pty_input[@]}" --out "$frame" --cols "$cols" --rows "$rows" --scroll "$scroll"
    status=$?
    set -e
    write_manifest "$status" "headless-libghostty-vt" '["frame.png","kittui-ghostty.stdout.txt","kittui-ghostty.stderr.txt"]'
    exit "$status"
    ;;
  proof)
    frame="$out_dir/proof.png"
    set +e
    run_cargo_ghostty --kittwm-proof-command "$command_text" "${pty_input[@]}" --out "$frame" --cols "$cols" --rows "$rows" --scroll "$scroll"
    status=$?
    set -e
    write_manifest "$status" "kittwm-proof-libghostty-vt" '["proof.png","kittui-ghostty.stdout.txt","kittui-ghostty.stderr.txt"]'
    exit "$status"
    ;;
  timelapse)
    frames_dir="$out_dir/timelapse"
    set +e
    run_cargo_ghostty --pty-timelapse-command "$command_text" "${pty_input[@]}" --out-dir "$frames_dir" --cols "$cols" --rows "$rows" --chunk-lines "$chunk_lines"
    status=$?
    set -e
    write_manifest "$status" "pty-timelapse-libghostty-vt" '["timelapse/","kittui-ghostty.stdout.txt","kittui-ghostty.stderr.txt"]'
    exit "$status"
    ;;
  sampled)
    frames_dir="$out_dir/sampled"
    set +e
    run_cargo_ghostty --pty-sampled-command "$command_text" "${pty_input[@]}" --out-dir "$frames_dir" --cols "$cols" --rows "$rows" --sample-ms "$sample_ms" --max-ms "$max_ms"
    status=$?
    set -e
    write_manifest "$status" "pty-sampled-libghostty-vt" '["sampled/","kittui-ghostty.stdout.txt","kittui-ghostty.stderr.txt"]'
    exit "$status"
    ;;
  app)
    if [[ "$(uname -s)" != "Darwin" ]]; then
      echo "--mode app requires macOS" >&2
      write_manifest 78 "ghostty-app" '["harness-manifest.json"]'
      exit 78
    fi
    if ! /usr/bin/open -Ra "$app_name"; then
      echo "could not locate $app_name.app" >&2
      write_manifest 78 "ghostty-app" '["harness-manifest.json"]'
      exit 78
    fi
    app_script="$out_dir/ghostty-command.sh"
    cat > "$app_script" <<APP_SCRIPT
#!/usr/bin/env bash
set -euo pipefail
$command_text
echo
echo '[kittwm-ghostty-harness] command complete; sleeping for screenshot capture'
sleep 8
APP_SCRIPT
    chmod +x "$app_script"
    /usr/bin/open -na "$app_name" --args -e "$app_script"
    sleep 3
    /usr/sbin/screencapture -x "$out_dir/ghostty-app-screen.png"
    if [[ "$keep_app" -ne 1 ]]; then
      /usr/bin/osascript -e "tell application \"$app_name\" to quit" >/dev/null 2>&1 || true
    fi
    write_manifest 0 "ghostty-app" '["ghostty-command.sh","ghostty-app-screen.png"]'
    ;;
  kittem)
    if ! command -v kittem >/dev/null 2>&1; then
      echo "kittem not found in PATH" >&2
      write_manifest 127 "kittem" '["harness-manifest.json"]'
      exit 127
    fi
    set +e
    kittem "$@" >"$out_dir/kittem.stdout.txt" 2>"$out_dir/kittem.stderr.txt"
    status=$?
    set -e
    write_manifest "$status" "kittem" '["kittem.stdout.txt","kittem.stderr.txt"]'
    exit "$status"
    ;;
  *)
    echo "unknown mode: $mode" >&2
    usage >&2
    exit 2
    ;;
esac
