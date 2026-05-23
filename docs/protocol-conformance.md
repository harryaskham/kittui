# kitty graphics protocol conformance — kittui

This document tracks `kittui`'s coverage of the kitty graphics protocol
(<https://sw.kovidgoyal.net/kitty/graphics-protocol/>), per spec section,
along with the bead that tracks any partial or unsupported items.

The protocol surface is owned by the `kittui-kitty` crate; identifiers like
`upload_still_ex`, `placement_command_ex`, `placeholder_text`, and the
`Quiet`, `UploadMedium`, `PlacementOptions`, `SubcellOffset` types come
from that crate's public API.

## Status legend

- ✅ **Supported** — implemented with grammar-pinned tests and visual proof.
- 🟡 **Partial** — encoder accepts the input but the renderer or transport
  does not exercise it end-to-end.
- ⛔ **Unsupported** — not yet implemented; tracked by a bead.

## Section coverage

| Spec section | Status | Notes |
|---|---|---|
| Transferring pixel data via the escape (`a=t,f=24/32/100`) | ✅ | PNG (`f=100`) uploads are covered by still/animation helpers; raw RGBA (`f=32`) is covered by the raw-frame path and exact grammar tests. Raw RGB (`f=24`) remains a possible future helper, but current renderer uploads use PNG or RGBA. |
| Local transmission (`t=d` direct base64) | ✅ | Default `UploadMedium::Direct`. Single chunked path covered by tests. |
| Local transmission (`t=f` regular file) | ✅ | `UploadMedium::File { path }`; encoded path goes into the `t=f` field. |
| Local transmission (`t=t` temp file) | ✅ | `UploadMedium::TempFile { path }`. |
| Local transmission (`t=s` shared memory) | ✅ | `UploadMedium::SharedMemory { name }` writes `t=s` + base64 name. |
| Chunked transmission (`m=1` / `m=0`) | ✅ | `encode_chunked` emits 4 KiB chunks with `m=1` until the final chunk's `m=0`; first chunk carries the verb. |
| Image ids (`i=`) and placement ids (`p=`) | ✅ | `PlacementOptions::placement_id` emits `p=`; `delete_placement` deletes by `(i, p)`. |
| Animation (`a=t` then `a=f`) | ✅ | `upload_animation_ex` uses `a=t` for frame 1 and `a=f` for frames 2..N, exactly as the spec describes. |
| Animation control (`a=a,s=loops,c=count,z=delay`) | ✅ | Emitted once after frame uploads; per-frame `z=` set via subsequent `a=a,r=N,z=ms` commands. |
| Suppress responses (`q=1` / `q=2`) | ✅ | `Quiet::SuppressAll` is default; `Quiet::SuppressOk` and `Quiet::Verbose` available. Without this, `Gi=…;ENOENT` lines leak into the terminal. |
| Placements with unicode placeholders (`U=1` + combining diacritics) | ✅ | `placeholder_text` emits the `(row, column, msb-of-image-id)` diacritic triple from the 297-entry `rowcolumn-diacritics.txt` table. |
| Absolute placement at cursor (no `U=1`) | ✅ | `PlacementOptions::absolute`. |
| Subcell offset (`X=` / `Y=`) | ✅ | `SubcellOffset { x_px, y_px }` on `PlacementOptions`. |
| Z-index (`z=`) | ✅ | `PlacementOptions::z_index`. |
| Delete by image id (`a=d,d=I,i=`) | ✅ | `delete`. |
| Delete by placement id (`a=d,d=I,i=,p=`) | ✅ | `delete_placement`. |
| Reading responses from the terminal | ⛔ | `bd-3dc8c7` epic — see "Open work" below. |
| Querying terminal capabilities (`a=q`) | ⛔ | `bd-3dc8c7` epic — see "Open work" below. |
| tmux passthrough wrapping | ✅ | `Transport::TmuxPassthrough` wraps every payload in `\ePtmux;…\e\\` with escape doubling. Auto-selected by `TerminalInfo::detect()` when `$TMUX` is set. |
| File / temp-file/shared-memory format hints (`f=100` PNG, `f=32` raw RGBA) | ✅ | PNG helpers use `f=100`; raw-frame file/temp/shared-memory paths use `f=32` and the kitty `t=f` / `t=t` / `t=s` grammar. |

## Test coverage

- `crates/kittui-kitty/src/lib.rs::tests` — 12 exact-grammar tests over every
  encoder function. Substring-only assertions were retired in bd-3dc8c7
  because the original leading-comma upload-chunk bug passed substring tests
  for months.
- `crates/kittui-cli/tests/proof_matrix.rs` — end-to-end smoke that asserts
  the `kittui proof` matrix prints all 9 expected sections; downstream
  byte-length drift becomes a hard test failure.
- `crates/kittui-cli/examples/showcase.rs` and
  `crates/kittui-cli/examples/ratatui_showcase.rs` — interactive proof
  programs the operator runs in a kitty-compatible terminal (Ghostty, kitty,
  tmux-on-kitty) to verify pixels actually appear.

## Open work

The remaining gaps are tracked under the protocol epic `bd-3dc8c7`:

- raw RGB (`f=24`) helper coverage if callers need it beyond current PNG/RGBA paths.
- terminal response reading (`OK`, `ENOENT`, capability queries).
- `a=q` capability probing into `TerminalInfo::detect()`.
- broader visual proof coverage for file/temp/shared-memory raw-frame transports across terminals.
- ratakittui complete widget coverage example + decoration matrix
  (`bd-6ccb5e`).

## How to reproduce visually

```sh
cargo build --release -p kittui-cli --examples
./target/release/kittui box -w 60 -h 6 --fg '#00d8ff' --bg '#0a1830ff' --radius 8 --border 2
./target/release/kittui proof
./target/release/kittui proof --emit --only "unicode placement"
./target/release/examples/showcase
./target/release/examples/ratatui_showcase
```

If you see actual filled images instead of placeholder glyph grids or
`Gi=…;ENOENT` messages, the protocol is anchoring correctly. If you only
see placeholder glyphs in a non-kitty terminal, that is expected — the
terminal must implement the kitty graphics protocol.
