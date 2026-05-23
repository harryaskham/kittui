# Session summary — dirty-grid kitty frame update investigation

## Goal

Investigate whether kitty animation/frame update semantics can support safer lower-bandwidth kittwm updates, and add a conservative dirty-grid prototype without changing default live rendering.

## Bead(s)

- `bd-510a36` — kittwm: investigate dirty-grid kitty frame updates
- Follow-ups filed:
  - `bd-889f33` — kittwm: use dirty grid to skip unchanged raw frame uploads
  - `bd-1a778c` — kittwm: expose dirty-frame metrics in native status
  - `bd-aab03f` — kittui-kitty: add explicit animation frame update primitives

## Before state

- Failing tests: none known.
- Relevant context: kittwm was uploading full bounded raw RGBA frames for graphics mode. Kitty animation protocol exists, but using it as a dirty/delta compositor may be subtle and buggy.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-wm dirty -- --nocapture` passed.
  - `cargo build -p kittui-wm` passed.
  - `git diff --check` passed.
- Context:
  - Added `docs/kittwm-dirty-frame-updates.md` documenting why full-frame replacement remains the safe default, what kitty animation does/does not solve, and which dirty strategies are safe vs experimental.
  - Added `crates/kittui-wm/src/dirty.rs` with terminal-agnostic dirty-grid helpers: `DirtyGrid`, `DirtyFrameDiff`, and `DirtyTile`.
  - Dirty grid hashes fixed-size RGBA tiles, identifies changed tiles, clips edge tiles, rejects invalid RGBA lengths, and reports changed fraction.
  - No kitty escape-code behavior or live kittwm renderer behavior changed.
  - Follow-up beads were filed for opt-in unchanged-frame skipping, dirty metrics, and explicit kitty animation/update primitives.

## Diff summary

- Code/content commit: `cb88cee3`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/lib.rs`, `crates/kittui-wm/src/dirty.rs`, `docs/kittwm-dirty-frame-updates.md`
- Behavioural delta: new dirty-grid helper and docs only; no default renderer changes.

## Operator-takeaway

Dirty-grid work is now grounded: safe next step is `skip-unchanged` full-frame suppression, not partial overlays by default. Kitty animation/overlay experiments are separated into follow-up beads to avoid buggy live WM behavior.
