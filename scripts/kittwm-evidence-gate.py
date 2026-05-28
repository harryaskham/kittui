#!/usr/bin/env python3
"""Lightweight close-time evidence gate for kittwm bead summaries.

The gate intentionally checks structure, not truth. It forces agents to write an
explicit evidence assessment that reviewers can challenge. A summary that has no
verdict, has a FAIL verdict, or tries to use command/test output as UI proof
without marking it VALIDATION_ONLY fails this check.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REQUIRED_PHRASES = [
    "## Evidence assessment",
    "Claim:",
    "Artifacts:",
    "What it shows:",
    "Why it supports the claim:",
    "Broken/ambiguous output noticed:",
    "Closure decision:",
]

VERDICT_RE = re.compile(r"verdict:\s*(PASS|VALIDATION_ONLY|FAIL)\b", re.IGNORECASE)
COMMAND_ONLY_RE = re.compile(r"\b(cargo test|cargo check|git diff --check|test result: ok)\b", re.IGNORECASE)
UI_CLAIM_RE = re.compile(
    r"\b(ui|ux|scene|surface|chrome|browser|terminal|status|footer|bar|help|doctor|info|architecture|native surfaces?)\b",
    re.IGNORECASE,
)


def fail(message: str) -> int:
    print(f"kittwm evidence gate: FAIL: {message}", file=sys.stderr)
    return 1


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: scripts/kittwm-evidence-gate.py <summary.md>", file=sys.stderr)
        return 2

    path = Path(argv[1])
    if not path.exists():
        return fail(f"summary not found: {path}")

    text = path.read_text(encoding="utf-8")
    missing = [phrase for phrase in REQUIRED_PHRASES if phrase not in text]
    if missing:
        return fail("missing required evidence assessment fields: " + ", ".join(missing))

    verdicts = [match.group(1).upper() for match in VERDICT_RE.finditer(text)]
    if not verdicts:
        return fail("no artifact verdict found; expected verdict: PASS, VALIDATION_ONLY, or FAIL")
    if "FAIL" in verdicts:
        return fail("summary contains FAIL evidence; do not close until fixed or followed up")

    has_command_only_signals = COMMAND_ONLY_RE.search(text) is not None
    looks_like_ui_claim = UI_CLAIM_RE.search(text) is not None
    has_validation_only = "VALIDATION_ONLY" in verdicts
    has_visual_artifact = re.search(r"\.(png|apng|gif|webp)\b", text, re.IGNORECASE) is not None

    if has_command_only_signals and looks_like_ui_claim and not has_validation_only and not has_visual_artifact:
        return fail(
            "UI/UX-looking claim mentions only command/test validation; add real kittwm output or mark validation-only with rationale"
        )

    print(
        "kittwm evidence gate: PASS "
        f"({len(verdicts)} verdict(s): {', '.join(verdicts)})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
