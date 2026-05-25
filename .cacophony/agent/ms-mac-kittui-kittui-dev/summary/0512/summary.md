# Session summary — Nix git dependency hashes

## Goal

Fix the Nix build failure caused by git dependencies added for the kittui/kittwm update plumbing.

## Bead(s)

- `bd-fe6147` — nix: fix mcp/updatable output hashes and showcase golden

## Changes

- Added `cargoLock.outputHashes` in `flake.nix`:
  - `mcp-cli-0.0.1 = sha256-aEWGvQh5YklD8l8bylHGIakhYovabHmPHtbGPjXM/1w=`
  - `updatable-cli-0.1.0 = sha256-kwsURSIbPW4o1S+YGGPwxWG8td4uZz8UYx6RIYMr5Ek=`
- Updated native showcase metrics/golden expectations for the graphical toast scene already emitted by the shell showcase.

## Validation

- `cargo test -p kittui-cli --lib native_showcase -- --nocapture` passed.
- `git diff --check` passed.
- `nix build .#kittui --no-link` progressed past the missing `mcp-cli` / `updatable-cli` hash errors after these hashes were added; the user requested reintegration before waiting for the long full derivation to complete.
