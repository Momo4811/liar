"""Compare a scan against the committed baseline.

A checker change that triples the findings is a regression until someone has
looked at them and said otherwise. Failing here forces that look, rather than
letting the change be absorbed silently.

    python corpus/compare.py findings.txt corpus/baseline.json
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

# How far a count may move before the baseline has to be updated in the same
# commit. Generous enough that adding a package does not trip it, tight enough
# that a checker suddenly firing everywhere does.
TOLERANCE = 0.20

HEADING = re.compile(r"^(?:error|warning)\[(?P<code>[A-Za-z0-9]+)\]:")


def counts(findings: str) -> dict[str, int]:
    found: dict[str, int] = {}
    for line in findings.splitlines():
        match = HEADING.match(line)
        if match:
            code = match.group("code")
            found[code] = found.get(code, 0) + 1
    return found


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2

    findings = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
    baseline = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))

    expected: dict[str, int] = baseline["counts"]
    actual = counts(findings)

    problems = []
    for code in sorted(set(expected) | set(actual)):
        before = expected.get(code, 0)
        after = actual.get(code, 0)

        if before == after:
            print(f"  {code}: {after} (unchanged)")
            continue

        # A check moving off zero, or onto it, always deserves a look: a
        # percentage of nothing is not a meaningful tolerance.
        allowed = max(1, round(before * TOLERANCE))
        if abs(after - before) <= allowed:
            print(f"  {code}: {before} -> {after} (within tolerance)")
            continue

        problems.append(f"  {code}: {before} -> {after}")

    if problems:
        print("\nFinding counts moved beyond tolerance:")
        print("\n".join(problems))
        print(
            "\nLook at the new findings, triage them in corpus/triage.md, and "
            "update corpus/baseline.json in the same commit."
        )
        return 1

    print("\nWithin tolerance of the baseline.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
