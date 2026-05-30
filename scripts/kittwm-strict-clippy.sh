#!/usr/bin/env bash
#
# kittwm-strict-clippy.sh — guard the strict-clippy baseline (bd-dc44f1).
#
# Runs `cargo clippy -p <crate> --all-targets -- -D warnings` over the set of
# crates that are currently strict-clippy clean, and reports a per-crate
# PASS/FAIL summary. Exits non-zero if any baseline crate regresses, so CI and
# agents can keep the baseline green crate-by-crate.
#
# The heavier crate kittui-cli is intentionally NOT in the baseline yet: its
# strict-clippy failures come entirely from kittui-quartz lints that only
# compile under cli's quartz feature (see bd-c42fce); run non-strict clippy
# there until that lands. kittwm-sdk (bd-3a7f70) and kittui-wm (bd-258008)
# joined the baseline once their libs were made strict-clippy clean.
#
# Usage:
#   scripts/kittwm-strict-clippy.sh            # check the whole baseline
#   scripts/kittwm-strict-clippy.sh kittui-kitty kittui-core   # subset
#
# Environment:
#   LIBGHOSTTY_VT_NO_PKG_CONFIG  defaults to 1 (stub libghostty-vt build).
#   CARGO                        cargo binary (default: cargo).

set -u

CARGO="${CARGO:-cargo}"
export LIBGHOSTTY_VT_NO_PKG_CONFIG="${LIBGHOSTTY_VT_NO_PKG_CONFIG:-1}"

# Strict-clippy-clean baseline crates (bd-dc44f1). Keep alphabetical-ish by layer.
BASELINE_CRATES=(
  kittui-core
  kittui-cache
  kittui-kitty
  kittui-render-cpu
  kittui-render-gpu
  kittui
  kittui-overlay
  kittui-tmux
  ratakittui
  kittui-ghostty-vt
  kittui-ffi
  kittui-quartz
  kittui-xvfb
  kittwm-sdk
  kittui-wm
)

# Crates intentionally excluded from the strict baseline (peer-owned / pre-existing warnings).
NOT_YET_CLEAN=(
  kittui-cli
)

if [ "$#" -gt 0 ]; then
  CRATES=("$@")
else
  CRATES=("${BASELINE_CRATES[@]}")
fi

echo "kittwm strict-clippy baseline guard (LIBGHOSTTY_VT_NO_PKG_CONFIG=$LIBGHOSTTY_VT_NO_PKG_CONFIG)"
echo "command: $CARGO clippy -p <crate> --all-targets -- -D warnings  (the only valid strict-clippy signal; plain/--lib runs give false-clean)"
echo "checking ${#CRATES[@]} crate(s); not-yet-clean (excluded): ${NOT_YET_CLEAN[*]}"
echo

failed=()
for crate in "${CRATES[@]}"; do
  printf '  %-20s ' "$crate"
  if "$CARGO" clippy -p "$crate" --all-targets -- -D warnings >/tmp/kittwm-strict-clippy-"$crate".log 2>&1; then
    pass=1
    # Feature-gated code is only linted when compiled: re-check crates whose
    # dependents enable extra features (false-clean trap, see bd-89422b/bd-c42fce).
    if [ "$crate" = "kittui-quartz" ]; then
      if ! "$CARGO" clippy -p "$crate" --features sck --all-targets -- -D warnings \
          >/tmp/kittwm-strict-clippy-"$crate"-sck.log 2>&1; then
        pass=0
      fi
    fi
    if [ "$pass" = 1 ]; then echo "PASS"; else echo "FAIL (feature-gated)"; failed+=("$crate"); fi
  else
    echo "FAIL"
    failed+=("$crate")
  fi
done

echo
if [ "${#failed[@]}" -eq 0 ]; then
  echo "strict-clippy baseline: PASS (all ${#CRATES[@]} crate(s) clean)"
  exit 0
fi

echo "strict-clippy baseline: FAIL (${#failed[@]} regressed: ${failed[*]})"
echo "see /tmp/kittwm-strict-clippy-<crate>.log for details"
exit 1
