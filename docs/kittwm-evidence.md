# kittwm evidence quality gate

kittwm evidence is review material, not ceremony. An artifact that merely shows a
command ran — or worse, shows the feature broken — must not be used as proof that
a bead is complete.

This page defines the close-time evidence contract for kittwm work.

## Evidence verdicts

Every kittwm bead summary must classify each artifact with one of these verdicts:

- `PASS` — the artifact visibly or mechanically demonstrates the claimed behavior.
- `VALIDATION_ONLY` — the artifact only shows tests/checks/logs for an internal
  code-path change. It is acceptable for SDK/request-builder/allocation-only work,
  but it is not UI proof.
- `FAIL` — the artifact contradicts the claim, shows a broken surface, or is too
  ambiguous to support the claim.

`FAIL` evidence blocks closure. Keep the bead open, fix the issue, replace the
artifact, or file a regression bead before closing.

## Required evidence assessment block

Each pending summary for a kittwm bead must include an assessment block like this:

```md
## Evidence assessment

Claim:
- <one sentence describing the behavior or code path changed>

Artifacts:
- `file-...` — verdict: PASS | VALIDATION_ONLY | FAIL
  - What it shows:
  - Where to look:
  - Why it supports the claim:
  - Broken/ambiguous output noticed:
  - If VALIDATION_ONLY, why visual proof is not applicable:

Closure decision:
- PASS: evidence supports closure.
- or FAIL: bead remains open / follow-up filed as `bd-...`.
```

A summary with no explicit verdict has not evaluated its evidence.

## What counts as proof

### UI/UX/chrome/scene/surface claims

Evidence must show the affected kittwm output, not just a test log. Prefer:

```sh
target/debug/kittui-ghostty \
  --kittwm-proof-command 'target/debug/kittwm <actual command>' \
  --cols 120 --rows 32 --out /tmp/kittwm-evidence/<bead>.png
```

The rendered output should visibly demonstrate the changed surface. The summary
must say where to look in the artifact.

Do not close a UI/UX bead if the artifact shows any of these without explanation
and follow-up:

- blank or mostly blank kittwm output;
- wrapped/truncated labels that contradict a bounded-output claim;
- stale ANSI/kitty artifacts, overlapping chrome, or cursor garbage;
- error text, panic output, missing command, or failed attach;
- only `cargo test ... ok` for a visual behavior claim.

### SDK/internal/direct-builder claims

Focused tests and code diffs may be the primary evidence when the claim is only
about an internal request string or allocation path. In that case, classify test
output as `VALIDATION_ONLY`, and state why visual kittwm output is not applicable.

For these beads, the strongest evidence is usually:

- exact request-string tests;
- the code diff showing removal of the `format!`/allocation path;
- targeted `cargo check` or unit tests.

Do not label a screenshot of test output as UI proof.

## Broken artifacts are regression evidence

If an artifact shows the feature is broken, record it honestly:

```md
- `file-...` — verdict: FAIL
  - What it shows: the status surface wraps into the pane area at 80 columns.
  - Why this blocks closure: the bead claims bounded status output.
  - Follow-up: filed `bd-...` / bead remains open.
```

A broken artifact is useful, but only as evidence of a bug.

## Suggested close-time checklist

Before closing a kittwm bead:

- [ ] Summary includes `## Evidence assessment`.
- [ ] Every artifact has a verdict.
- [ ] UI/UX claims have actual affected kittwm output.
- [ ] Internal-only claims mark test/log artifacts as `VALIDATION_ONLY`.
- [ ] No `FAIL` artifact is being used as proof.
- [ ] Any broken output has a filed follow-up or keeps the bead open.

Use `scripts/kittwm-evidence-gate.py <summary.md>` as a lightweight local check
before committing a pending summary.
