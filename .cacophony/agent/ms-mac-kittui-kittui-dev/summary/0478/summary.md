# Session summary — SDK typed shortcuts catalog helper

## Goal

Expose the native `SHORTCUTS_JSON` machine-readable shortcut catalog through typed `kittwm-sdk` APIs.

## Bead(s)

- `bd-b52ea9` — kittwm-sdk: typed shortcuts catalog helper

## Before state

- Failing tests: none known.
- Relevant context: `SHORTCUTS_JSON` and `kittwm --shortcuts-json` existed after the previous source bead, but SDK clients had no typed helper.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittwm-sdk shortcuts_helper_sends_expected_socket_command -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `ShortcutCatalog` and `ShortcutEntry` typed structs.
  - Added `Kittwm::shortcuts()` and alias `Kittwm::shortcuts_json()`.
  - Helper reads socket `SHORTCUTS_JSON` and is `ReadText` capability-gated like other inspection catalogs.
  - Added focused Unix socket test verifying command string and parsing of launch/help/exit entries.
  - No daemon/session/CLI runtime changes.

## Parallel coordination

- `kittui-dev-2` has docs-only follow-ups:
  - `bd-736c85` for shortcut JSON CLI/socket docs.
  - `bd-ad3f03` for SDK shortcut helper docs after this lands.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`

## Operator-takeaway

SDK clients can now call `client.shortcuts()` or `client.shortcuts_json()` to inspect stable shortcut ids, key chords, and descriptions without raw socket requests.
