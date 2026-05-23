# Session summary — separate scene render from placement

## Goal

Address a core kittui library gap for shell/external-platform hosts: allow rendered/cached scene content to be moved in the terminal without mutating the scene footprint or changing the scene/image identity.

## Bead(s)

- `bd-9f415f` — kittui: separate render scene from terminal placement

## Before state

- Failing tests: none known.
- Relevant gap: `CompositionEntry` carried a placement footprint, but `Composer::apply` called `Runtime::place(&scene)`, so the scene's own footprint controlled terminal placement. Moving content required changing the scene and its cache/image id, or produced placement bytes at the old location.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui place_at -- --nocapture` passed.
  - `cargo test -p kittui moved_entry -- --nocapture` passed.
  - `cargo test -p kittui place_caches -- --nocapture` passed earlier in this bead.
  - `cargo build -p kittui` passed.
  - `git diff --check` passed.
- Context: `Runtime::place_at(scene, footprint)` renders/caches based on the scene-local footprint but emits terminal placement at the supplied footprint when dimensions match. `Composer::apply` now uses `place_at` so `CompositionEntry.footprint` is honored.

## Diff summary

- Code/content commit: `95518cc`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`, `crates/kittui/src/composition.rs`, `README.md`
- Behavioural delta: moving a composition entry changes placement position without changing scene id/image id or re-uploading the cached image.

## Operator-takeaway

This makes kittui materially better as a renderer substrate: hosts can render assets once, then place/move them independently, which is essential for shell-script composition, WM chrome reuse, and external platform integrations.
