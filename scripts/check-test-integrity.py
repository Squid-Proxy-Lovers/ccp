# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Fail when a change removes tests or assertions, or skips tests, relative to its base.

Making a failing suite pass by deleting or disabling checks is a common way for
changes to hide regressions. Counts are compared per file against the merge base.
A reviewed exception is allowed when CCP_ALLOW_TEST_REDUCTION=1, which CI sets
only for pull requests carrying the `allow-test-reduction` label.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_SUFFIXES = (".rs", ".py", ".sh")
PATTERNS = {
    "tests": {
        ".rs": re.compile(r"#\[(?:tokio::)?test\b"),
        ".py": re.compile(r"^\s*(?:async\s+)?def test_\w+", re.MULTILINE),
    },
    "skips": {
        ".rs": re.compile(r"#\[ignore\b"),
        ".py": re.compile(
            r"@unittest\.skip|\.skipTest\(|unittest\.expectedFailure|pytest\.mark\.(?:skip|xfail)"
        ),
        ".sh": re.compile(r"^\s*skip\s", re.MULTILINE),
    },
    "assertions": {
        ".rs": re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?!"),
        ".py": re.compile(r"\bself\.assert\w+\(|^\s*assert\s", re.MULTILINE),
        ".sh": re.compile(r"^\s*expect_(?:success|contains|not_contains)\b", re.MULTILINE),
    },
}


def git(*args: str) -> bytes:
    executable = shutil.which("git")
    if executable is None:
        raise SystemExit("git is required to compare test counts")
    # The executable is fixed and arguments are Git revisions or tracked paths.
    return subprocess.run(  # noqa: S603
        [executable, *args], cwd=ROOT, capture_output=True, check=True
    ).stdout


def is_source(path: str) -> bool:
    # The checker itself defines the patterns it counts.
    return path.endswith(SOURCE_SUFFIXES) and path != "scripts/check-test-integrity.py"


def count(path: str, text: str) -> Counter[str]:
    suffix = Path(path).suffix
    totals: Counter[str] = Counter()
    for category, patterns in PATTERNS.items():
        pattern = patterns.get(suffix)
        if pattern is not None:
            totals[category] = len(pattern.findall(text))
    return totals


def read(revision: str | None, path: str) -> str:
    if revision is None:
        return (ROOT / path).read_text(errors="replace")
    return git("show", f"{revision}:{path}").decode(errors="replace")


def snapshot(revision: str | None) -> dict[str, Counter[str]]:
    """Count per file at a revision, or in the working tree when revision is None."""
    if revision is None:
        names = git("ls-files", "-z", "--cached", "--others", "--exclude-standard")
    else:
        names = git("ls-tree", "-r", "-z", "--name-only", revision)
    paths = [name.decode() for name in names.split(b"\0") if name]
    return {
        path: count(path, read(revision, path))
        for path in paths
        if is_source(path) and (revision is not None or (ROOT / path).is_file())
    }


def compare(base: dict[str, Counter[str]], head: dict[str, Counter[str]]) -> list[str]:
    problems = []
    for category in PATTERNS:
        before = sum(counts[category] for counts in base.values())
        after = sum(counts[category] for counts in head.values())
        print(f"{category}: {before} -> {after}")
        increased_skips = category == "skips" and after > before
        reduced_checks = category != "skips" and after < before
        if not (increased_skips or reduced_checks):
            continue
        for path in sorted(set(base) | set(head)):
            old = base.get(path, Counter())[category]
            new = head.get(path, Counter())[category]
            if (increased_skips and new > old) or (reduced_checks and new < old):
                problems.append(f"{path}: {category} {old} -> {new}")
    return problems


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/main", help="branch the change merges into")
    args = parser.parse_args()
    merge_base = git("merge-base", args.base, "HEAD").decode().strip()
    problems = compare(snapshot(merge_base), snapshot(None))
    if not problems:
        return
    print("Tests, assertions, or skips changed in a way that weakens the suite:")
    for problem in problems:
        print(f"  {problem}")
    if os.environ.get("CCP_ALLOW_TEST_REDUCTION") == "1":
        print("Allowed by the reviewed allow-test-reduction label.")
        return
    raise SystemExit(
        "Restore the checks, or explain the reduction in the PR and ask a maintainer "
        "to apply the allow-test-reduction label."
    )


if __name__ == "__main__":
    main()
