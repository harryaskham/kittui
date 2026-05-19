#!/usr/bin/env bash
# kittui-tmux live hook installer.
#
# Wires tmux's pane-geometry change hooks to call `kittui-tmux` so the
# graphical pane separators repaint whenever the layout mutates. Source
# this file from your tmux.conf or copy the contents into the relevant
# `set-hook` lines:
#
#   source-file /path/to/kittui/crates/kittui-tmux/examples/tmux.hooks.conf
#
# Requires `kittui-tmux` on PATH (build with `cargo install --path
# crates/kittui-tmux`).
set -euo pipefail

emit_hook() {
  local hook="$1"
  printf "set-hook -g %s 'run-shell -b \"tmux list-panes -F \\\"#{pane_id} #{pane_left} #{pane_top} #{pane_width} #{pane_height}\\\" | kittui-tmux > /dev/tty\"'\n" "$hook"
}

# Emit the snippet to stdout so a tmux session can pipe it through
# `tmux source-file -`:
#
#   ./install-hooks.sh | tmux source-file -
for hook in \
  client-resized \
  pane-exited \
  pane-set-active \
  session-window-changed \
  window-layout-changed \
  window-resized
do
  emit_hook "$hook"
done
