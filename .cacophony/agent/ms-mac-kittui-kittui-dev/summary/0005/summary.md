# Session summary — kitty graphics protocol conformance + visual proof

## Goal

Make kittui actually conformant to the kitty graphics protocol
(<https://sw.kovidgoyal.net/kitty/graphics-protocol/>) and prove visually
that it renders correctly end-to-end in Ghostty, including under tmux.

## Bead(s)

- `bd-3dc8c7` (epic) — Complete kitty graphics protocol conformance + ratakittui coverage
- `bd-703e47` — Encode kitty unicode-placeholder diacritics per spec
- `bd-0cbd14` — Set `q=2` quiet flag on all kitty control commands
- `bd-2700e0` — Implement `Transport::File` and `Transport::Memory` upload paths
- `bd-48a54c` — Fix animation upload to use `a=t`/`a=f` frame appends and `a=a` control
- `bd-e8ddc7` — Support `placement_id` (`p=`) and subcell offset (`X=`, `Y=`) on `placement_command`
- `bd-f92450` — Replace substring-based protocol tests with exact-grammar regressions
- `bd-d9f27d` — Add `kittui proof` CLI command and Rust harness
- `bd-83b342` — Showcase: cover every protocol mode end-to-end
- `bd-6ccb5e` — ratakittui: complete widget coverage example + decoration matrix
- `bd-1d966f` — Document kitty spec deviations and conformance status
- Side bead: `bd-3581fb` — `kittui-cache` tests flake in parallel (filed, not yet fixed)

## Before state

- Failing tests: none in `cargo test`, yet the showcase produced
  `Gi=<id>;ENOENT: image not found` for every scene in Ghostty.
- Relevant metrics: `kittui-kitty` tests asserted substrings of the
  current malformed output, so the spec-incorrect upload chunk syntax,
  bare placeholder cells without diacritics, no `q=` field, no
  `Transport::File`/`Memory` implementations, and wrong animation
  verbs were all "green".
- Context: Operator confirmed the repo was supposed to be a careful
  extraction of the Cacophony TUI graphics lib against the kitty
  graphics spec, but real terminal rendering had never produced an
  image — only placeholder glyph grids.

## After state

- Failing tests: none across the workspace with `--test-threads=1`
  (one known unrelated parallel-race in `kittui-cache` filed as
  `bd-3581fb`).
- Relevant metrics: `kittui-kitty` now has 12 exact-grammar tests;
  `kittui-cli` has the `proof_matrix` integration test pinning every
  protocol-matrix section; workspace tests are 72 passing.
- Context: Running `./target/release/kittui box -w 60 -h 8 --fg ...
  --bg ... --radius 8` in Ghostty (via tmux passthrough auto-detected
  from `$TMUX`) now renders a real image, captured via tendril and
  committed under
  `.cacophony/agent/<agent>/summary/pending/screenshots/bd-703e47-box-vivid.png`
  (green box with magenta border). The affordance showcase and the
  ratakittui showcase also render.

## Diff summary

- Code/content commits (this session, in order):
  - `ef82643` — kitty protocol overhaul + tmux auto-detect + proof CLI + grammar tests
  - `ee49fa8` — auto-detect tmux in showcase examples
  - `4c07245` — ratakittui cursor positioning, proof matrix test, conformance docs, README v0.2
- Summary artefact commit: intentionally omitted; this file must not
  self-reference its own mutable SHA.
- Files touched (net):
  - `crates/kittui-kitty/src/lib.rs` rewritten with grammar-pinned tests
  - `crates/kittui-kitty/src/diacritics.rs` + `data/rowcolumn-diacritics.txt` added (297-entry kitty placeholder table)
  - `crates/kittui-core/src/terminal.rs` adds `TerminalInfo::detect()` for tmux auto-detect
  - `crates/kittui-cli/src/main.rs` adds `kittui proof` subcommand
  - `crates/kittui-cli/tests/proof_matrix.rs` adds matrix regression test
  - `crates/kittui-cli/examples/{showcase,ratatui_showcase}.rs` use `TerminalInfo::detect()` and emit cursor-positioned placements
  - `crates/ratakittui/src/lifecycle.rs` `finalize_frame` now positions cursor per effect
  - `docs/protocol-conformance.md` enumerates per-spec-section coverage
  - `README.md` updated to v0.2 status block
- Tests: +12 kittui-kitty grammar tests, +1 kittui-cli proof matrix test.
- Behavioural delta: kittui now produces spec-correct kitty graphics
  protocol bytes for upload (Direct + File + SharedMemory + TempFile +
  Tmux-passthrough), placement (with id, subcell offset, z-index,
  unicode-placeholder toggle), animation (verb-correct frame appends +
  control + per-frame delays), and delete (by image id and by placement
  id). Auto-detects tmux from `$TMUX`.

## Embedded artefacts

- `screenshots/bd-703e47-box-vivid.png` — green box with magenta border, real image in pane-2 of agent terminal (Ghostty over tmux).
- `screenshots/showcase.png`, `screenshots/showcase-top.png` — affordance showcase rendering three tonal panels.
- `screenshots/ratatui-showcase-v2.png`, `…-v3.png` — ratakittui showcase rendering title bar + divider, reports `233 placements, 1641 bytes of placement, 2912 bytes of upload`.
- `screenshots/proof-report.png` — `kittui proof` matrix listing all 9 protocol combinations with byte lengths and hex prefixes.
- `screenshots/proof-animation*.png` — animation matrix section in `--emit` mode.
- `screenshots/box-animated.png` — animated kittui box rendering through the protocol.

## Operator-takeaway

kittui's protocol layer is now proven end-to-end against Ghostty over
tmux: substring tests are gone, exact-grammar regressions are in place,
a `kittui proof` matrix walks every combination, conformance docs
enumerate what's implemented vs tracked under the protocol epic. The
remaining open work is raw RGB(A) transmission, terminal response
reading, `a=q` capability probing, the parallel-test cache race, and
the full ratakittui decoration matrix — each tracked under
`bd-3dc8c7` and its children.
