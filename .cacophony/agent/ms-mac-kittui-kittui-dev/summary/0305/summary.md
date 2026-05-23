# Session summary — native session manifest JSON

## Goal

Add a stable persistence-oriented native kittwm session manifest distinct from live status/geometry/process snapshots.

## Bead(s)

- `bd-6d6173` — kittwm: add native session manifest JSON

## Before state

- Failing tests: none known.
- Relevant gap: native status surfaces exposed rich live state, but controllers lacked a stable geometry-free manifest suitable for save/restore tooling.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added native socket command `SESSION_JSON`. It returns schema-versioned JSON:
  - `schema_version: 1`
  - `kind: "kittwm-native-session"`
  - `layout`
  - `focus`
  - ordered `panes[]` with `index`, `window`, `title`, `command`, `weight`, and `focused`
  The manifest intentionally excludes transient pid, geometry, and text snapshots. HELP/HELP_JSON plus README/docs now mention `SESSION_JSON`.

## Diff summary

- Code/content commit: `d2d35d7`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: controllers can save a stable native session layout/order/focus/command/weight manifest.

## Operator-takeaway

Native kittwm now has the read-side foundation for future session restore: `SESSION_JSON` captures durable pane/session state without live-only noise.
