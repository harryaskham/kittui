# Session summary — document native move keybindings

## Goal

Document the local native kittwm pane move keybindings that complement socket `MOVE_PANE`.

## Bead(s)

- `bd-c4e477` — docs: document native pane move keybindings

## Before state

- Failing tests: none known.
- Relevant gap: local move keybindings (`Ctrl-A [`/`,` and `Ctrl-A ]`/`.`) existed in the native PTY loop, but README/docs only mentioned socket move and the footer hint.

## After state

- Failing tests: none.
- Relevant metrics:
  - `git diff --check` passed.
- Context: README native status bullet and `docs/wm.md` opening operator summary now document `Ctrl-A [` / `Ctrl-A ,` and `Ctrl-A ]` / `Ctrl-A .` for moving the focused pane.

## Diff summary

- Code/content commit: `0abf76b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `README.md`, `docs/wm.md`
- Behavioural delta: operator docs now match the native local pane movement keymap.

## Operator-takeaway

Users can discover all native pane move options from the docs, not just socket commands.
