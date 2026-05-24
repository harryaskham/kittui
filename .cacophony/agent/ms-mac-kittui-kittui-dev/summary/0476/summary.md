# Session summary — SDK chrome JSON helper

## Goal

Expose the native `CHROME_JSON` inspection surface through a typed `kittwm-sdk` convenience helper.

## Bead(s)

- `bd-f394a8` — kittwm-sdk: typed chrome JSON helper

## Before state

- Failing tests: none known.
- Relevant context: daemon and CLI exposed `CHROME_JSON` / `kittwm --chrome-json`, and `Status` / `PanesStatus` typed nested chrome metadata, but SDK clients had no direct helper for `CHROME_JSON` itself.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittwm-sdk chrome_helper_sends_expected_socket_command -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `Kittwm::chrome()` returning `ChromeReservationStatus` from `CHROME_JSON`.
  - Added alias `Kittwm::chrome_json()`.
  - Added focused Unix socket test verifying the command and parsed workspace/top_bar_rows/tilable_rows.
  - No daemon/session runtime changes.

## Parallel coordination

- Assigned `bd-be8304` to `kittui-dev-2` as docs-only follow-up after this source bead lands.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`

## Operator-takeaway

SDK clients can now call `client.chrome()` or `client.chrome_json()` for the same typed chrome reservation metadata as the daemon's `CHROME_JSON` socket command.
