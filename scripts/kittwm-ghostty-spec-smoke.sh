#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
kittwm-ghostty-spec-smoke — run core kittwm Ghostty/libghostty smoke scenarios

Usage:
  scripts/kittwm-ghostty-spec-smoke.sh [--out-dir DIR] [--cols N] [--rows N]

Scenarios:
  help              kittwm --help through the kittwm proof harness
  shortcuts         kittwm --shortcuts through the kittwm proof harness
  live-first-launch kittwm first-launch empty workspace in a unique socket,
                    captured as a sampled live PTY sequence with Ctrl-] exit input

The script writes scenario artifacts plus summary.md in --out-dir. Review PNGs
before treating any scenario as visual proof; summary.md records expected text
and flags first-launch blank frames as a visible failure to investigate.
USAGE
}

out_dir="/tmp/kittwm-ghostty-spec-smoke-$$"
cols=100
rows=28

while [[ $# -gt 0 ]]; do
  case "$1" in
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
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
harness="$repo_root/scripts/kittwm-ghostty-harness.sh"
mkdir -p "$out_dir"

run_scenario() {
  local name="$1"
  shift
  echo "[kittwm-ghostty-spec-smoke] $name"
  "$harness" "$@"
}

run_scenario help \
  --mode proof \
  --out-dir "$out_dir/help" \
  --cols "$cols" \
  --rows "$rows" \
  --scroll top \
  -- "cargo run -p kittui-cli --bin kittwm -- --help"

run_scenario shortcuts \
  --mode proof \
  --out-dir "$out_dir/shortcuts" \
  --cols "$cols" \
  --rows "$rows" \
  --scroll top \
  -- "cargo run -p kittui-cli --bin kittwm -- --shortcuts"

live_socket="/tmp/kittwm-spec-smoke-$$.sock"
rm -f "$live_socket"
run_scenario live-first-launch \
  --mode sampled \
  --out-dir "$out_dir/live-first-launch" \
  --cols "$cols" \
  --rows "$rows" \
  --sample-ms 250 \
  --max-ms 5000 \
  --pty-input '\x1d' \
  --pty-input-delay-ms 3500 \
  -- "KITTWM_SOCKET=$live_socket cargo run -p kittui-cli --bin kittwm --"

{
  echo "# kittwm Ghostty spec smoke summary"
  echo
  echo "- Harness: scripts/kittwm-ghostty-harness.sh"
  echo "- Columns: $cols"
  echo "- Rows: $rows"
  echo
  echo "## Scenarios"
  echo
  echo "| Scenario | Artifact root | Expected review |"
  echo "| --- | --- | --- |"
  echo "| help | help/proof.png | Shows kittwm help derived from parser/catalog. |"
  echo "| shortcuts | shortcuts/proof.png | Shows shortcut catalog including launch, split, focus, close, exit hints. |"
  echo "| live-first-launch | live-first-launch/sampled/frame-*.png | Should show first-launch empty workspace top bar and shortcut hint before exit. If frames are blank/near-blank, treat as FAIL and investigate kittwm drawing/proof replay. |"
  echo
  echo "## Evidence classification guidance"
  echo
  echo "- Help/shortcuts screenshots are validation artifacts unless the reviewed image is"
  echo "  being used specifically to prove user-facing help text."
  echo "- Live first-launch screenshots can be PASS only if they visibly show the empty"
  echo "  workspace/top-bar state. Blank or stale frames are FAIL and should block closure"
  echo "  of visual UX claims."
} > "$out_dir/summary.md"

echo "wrote $out_dir/summary.md"
