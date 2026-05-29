# kittwm user-facing analysis — 2026-05-29 (bd-5eedb3)

This note records a critical user-facing review of the currently available
Ghostty/libghostty-vt kittwm proof artifacts and command outputs. The goal was
not to claim new UI proof, but to identify what a user or reviewer would see as
broken, misleading, or insufficiently usable.

## Inputs reviewed

Generated with `scripts/kittwm-ghostty-harness.sh --mode proof` using the local
`target/debug/kittwm` and `target/debug/kittui-ghostty` binaries:

- `/tmp/kittwm-user-analysis/help/proof.png`
- `/tmp/kittwm-user-analysis/shortcuts/proof.png`
- `/tmp/kittwm-user-analysis/native-surfaces/proof.png`
- `/tmp/kittwm-user-analysis/empty/proof.png`
- `/tmp/kittwm-user-analysis/empty-unique/proof.png`
- `/tmp/kittwm-user-analysis/empty-unique2/proof.png`
- each directory's `harness-manifest.json`, `kittui-ghostty.stdout.txt`, and
  `kittui-ghostty.stderr.txt`

## Findings

### 1. Live kittwm session proof captures do not show the live UI

Running a bounded live session through the proof harness, for example:

```sh
scripts/kittwm-ghostty-harness.sh --mode proof --out-dir /tmp/kittwm-user-analysis/empty-unique2 -- \
  env KITTWM_SOCKET=/tmp/kittwm-analysis-empty.sock timeout 2s target/debug/kittwm
```

produced a PNG that only shows the shell command and `[exit 124]`. It does not
show the expected empty workspace/top bar/shortcut hint. From a user-facing
proof perspective this is **regression evidence**, not UI proof: the artifact
cannot demonstrate that the live native WM surface is usable.

Likely cause: the live session uses alternate-screen/raw terminal lifecycle and
`timeout` restores/leaves the captured terminal state at the shell scrollback
rather than at a representative live frame.

Follow-up priority: high. The harness needs either a live-frame sampling mode or
a first-class kittwm snapshot/proof mode that exits after rendering a stable
frame without erasing the UI.

### 2. Harness manifests mask inner command failures

The proof harness manifest status is `0` for cases where the rendered terminal
clearly shows an inner failure, for example:

- `kittwm` against an already-listening default socket shows:
  `kittwm native spawn queue is already listening on /tmp/kittwm-harryaskham.sock`
  and `[exit 1]`.
- bounded live-session attempts show `[exit 124]` from `timeout`.

However, `harness-manifest.json` records status `0` because `kittui-ghostty`
succeeded at rendering the failed command's terminal output. This is dangerous
for automation: a CI or agent could treat a failed kittwm run as successful just
because PNG generation succeeded.

Follow-up priority: high. The harness and/or `kittui-ghostty` should expose the
inner PTY command exit code in a machine-readable artifact and optionally fail
when the inner command fails.

### 3. Default socket collision is a poor first-run experience

Running `kittwm` while another default socket exists produces a terse failure:

```text
kittwm native spawn queue is already listening on /tmp/kittwm-harryaskham.sock
```

This is accurate but not actionable enough for a user. It does not suggest:

- `kittwm stop`
- using a unique socket with `KITTWM_SOCKET=/tmp/... kittwm`
- how to inspect the existing session
- whether the socket is stale versus an active listener

Follow-up priority: medium-high. Improve the error text and, if safe, detect and
explain stale socket files separately from active sessions.

### 4. Static command-output proofs are readable but not sufficient for live UX

`native-surfaces/proof.png` is readable and correctly shows the native-surface
coverage table. That kind of artifact is useful for documentation and command
output validation. It is not sufficient for claims about live kittwm UI
behavior, input routing, flicker, or frame stability.

The help/shortcuts proof attempts were also affected by command selection and
socket state. They should be classified carefully as validation or regression
artifacts, not automatic PASS UI proof.

### 5. No direct SPP usability conclusion from current captures

The available captures did not exercise SPP/kitty graphics image transfer in a
way that lets this review conclude whether SPP is end-user usable. The reviewed
artifacts are mostly text-mode libghostty proof captures and failed live-session
captures. A separate targeted SPP/graphics scenario should be captured before
making SPP claims.

## Recommended follow-up beads

1. Make kittwm proof harness preserve/report inner PTY command exit status.
2. Add a kittwm live-session proof mode that captures a representative frame
   before raw/alternate-screen teardown.
3. Improve default socket collision error text with actionable next commands.
4. Capture and analyze a targeted graphics/SPP scenario before claiming graphics
   protocol usability.

## Evidence classification

All artifacts reviewed here are **VALIDATION_ONLY** or **regression evidence**.
None should be used as PASS visual proof for live kittwm UI behavior without
additional live-frame capture support and manual review.
