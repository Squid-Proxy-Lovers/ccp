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
SOURCE_SUFFIXES = (".rs", ".py", ".sh", ".mjs")
PATTERNS = {
    "tests": {
        ".rs": re.compile(r"#\[(?:tokio::)?test\b"),
        ".py": re.compile(r"^\s*(?:async\s+)?def test_\w+", re.MULTILINE),
        # Bare test(...)/test.skip(...)/test.todo(...), and a subtest
        # invoked through its test-context parameter, e.g. t.test(...).
        # This repo's .mjs suites use node:test's test() exclusively, not
        # it()/describe(), so those are intentionally not counted here.
        ".mjs": re.compile(r"(?<![\w.])test(?:\.\w+)*\s*\(|\b\w+\.test(?:\.\w+)*\s*\("),
    },
    "skips": {
        ".rs": re.compile(r"#\[ignore\b"),
        ".py": re.compile(
            r"@unittest\.skip|\.skipTest\(|unittest\.expectedFailure|pytest\.mark\.(?:skip|xfail)"
        ),
        ".sh": re.compile(r"^\s*skip\s", re.MULTILINE),
        # .skip/.todo access (call, property read, or alias assignment
        # such as `const s = test.skip;`), plus a skip/todo key anywhere
        # in an options object, first position or not, quoted or bare,
        # keyed (`skip: true`) or shorthand (`{skip}`).
        ".mjs": re.compile(r"\.(?:skip|todo)\b|[{,]\s*['\"]?(?:skip|todo)['\"]?\s*[:,}]"),
    },
    "assertions": {
        ".rs": re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?!"),
        ".py": re.compile(r"\bself\.assert\w+\(|^\s*assert\s", re.MULTILINE),
        ".sh": re.compile(r"^\s*expect_(?:success|contains|not_contains)\b", re.MULTILINE),
        # Bare assert(x), assert.ok(x), and any chain depth such as
        # assert.strict.equal(...). Destructured or aliased imports
        # (`const {equal} = assert`) are not tracked; this repo's .mjs
        # suites always call through the `assert` member chain directly.
        ".mjs": re.compile(r"\bassert(?:\.\w+)*\s*\("),
    },
}


def _copy_string_literal(text: str, i: int, out: list[str], quote: str) -> int:
    """Copy a string literal starting at i (text[i:i+len(quote)] == quote)."""
    n = len(text)
    out.append(quote)
    i += len(quote)
    while i < n and text[i : i + len(quote)] != quote:
        if text[i] == "\\" and len(quote) == 1:
            out.append(text[i : i + 2])
            i += 2
            continue
        out.append(text[i])
        i += 1
    if i < n:
        out.append(quote)
        i += len(quote)
    return i


def _strip_python_comments(text: str) -> str:
    """Drop '#' comments, leaving string contents (e.g. URLs) untouched."""
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        char = text[i]
        if char in "'\"":
            quote = char * 3 if text[i : i + 3] == char * 3 else char
            i = _copy_string_literal(text, i, out, quote)
        elif char == "#":
            while i < n and text[i] != "\n":
                i += 1
        else:
            out.append(char)
            i += 1
    return "".join(out)


def _skip_block_comment(text: str, i: int, out: list[str]) -> int:
    """Skip a '/* ... */' comment starting at i, keeping its newlines."""
    n = len(text)
    i += 2
    while i < n and text[i : i + 2] != "*/":
        if text[i] == "\n":
            out.append("\n")
        i += 1
    return i + 2


def _strip_c_style_comments(text: str, *, template_strings: bool = False) -> str:
    """Drop '//' and '/* */' comments, leaving string contents untouched.

    Block comments keep their newlines so line-anchored patterns in the
    surrounding code are unaffected. Nested block comments (legal in Rust)
    and template-literal interpolation (`${...}` in JS) are not handled;
    this is a small scan, not a full lexer.
    """
    out: list[str] = []
    i, n = 0, len(text)
    quotes = "'\"" + ("`" if template_strings else "")
    while i < n:
        two = text[i : i + 2]
        char = text[i]
        if two == "//":
            while i < n and text[i] != "\n":
                i += 1
        elif two == "/*":
            i = _skip_block_comment(text, i, out)
        elif char in quotes:
            i = _copy_string_literal(text, i, out, char)
        else:
            out.append(char)
            i += 1
    return "".join(out)


def strip_comments(suffix: str, text: str) -> str:
    if suffix == ".py":
        return _strip_python_comments(text)
    if suffix == ".rs":
        return _strip_c_style_comments(text)
    if suffix == ".mjs":
        return _strip_c_style_comments(text, template_strings=True)
    return text


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
    code = strip_comments(suffix, text)
    totals: Counter[str] = Counter()
    for category, patterns in PATTERNS.items():
        pattern = patterns.get(suffix)
        if pattern is not None:
            totals[category] = len(pattern.findall(code))
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
        for path in sorted(set(base) | set(head)):
            old = base.get(path, Counter())[category]
            new = head.get(path, Counter())[category]
            if (category == "skips" and new > old) or (category != "skips" and new < old):
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
