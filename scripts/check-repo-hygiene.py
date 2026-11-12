# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Check tracked text without normalizing intentional Unicode test inputs."""

from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXCLUDED_FILES = {
    "Cargo.lock": "Cargo generates the dependency lockfile.",
    "crates/server/web/package-lock.json": "npm generates the dependency lockfile.",
    "crates/server/web/public/fonts/FragmentMono-OFL.txt": "Preserve the upstream licence verbatim.",
    "crates/server/web/public/fonts/OverusedGrotesk-OFL.txt": "Preserve the upstream licence verbatim.",
}
FROZEN_HEADER = b"## [0.1.1] - 2026-11-11\n"
# This published security entry is immutable; exempt only its exact original bytes.
FROZEN_SHA256 = "6e2cc26577b80e5eab119854f89cd60bf1b2f8a4cf2910c54f2f014aabe4ac12"
SCRIPT = re.compile(r"<script\b[^>]*>(.*?)</script\s*>", re.I | re.S)
JS_TOKEN = re.compile(
    r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'|`(?:\\.|[^`\\])*`'
    r"|(?P<comment>//[^\r\n]*|/\*.*?\*/)",
    re.S,
)
HTML_COMMENT = re.compile(r"<!--.*?-->", re.S)


def git(root: Path, *args: str) -> bytes:
    executable = shutil.which("git")
    if executable is None:
        raise ValueError("git is required to enumerate tracked text")
    # Fixed executable and argv, never shell evaluation of filenames or revisions.
    return subprocess.run(  # noqa: S603 -- Git arguments are passed separately.
        [executable, *args], cwd=root, capture_output=True, check=True
    ).stdout


def excluded(path: str) -> bool:
    # Cargo/Vite generates these embedded panel assets from checked source.
    return path in EXCLUDED_FILES or path.startswith("crates/server/web/dist/")


def editable_changelog(data: bytes) -> bytes:
    start = data.find(FROZEN_HEADER)
    if start < 0:
        raise ValueError("CHANGELOG.md: published 0.1.1 section is missing or renamed")
    following = re.search(rb"^## ", data[start + len(FROZEN_HEADER) :], re.M)
    end = start + len(FROZEN_HEADER) + following.start() if following else len(data)
    if hashlib.sha256(data[start:end]).hexdigest() != FROZEN_SHA256:
        raise ValueError("CHANGELOG.md: published 0.1.1 section differs from its frozen bytes")
    # Preserve line numbers and the real EOF; do not exempt any surrounding text.
    return data[:start] + re.sub(rb"[^\n]", b"", data[start:end]) + data[end:]


def comment_spans(text: str) -> list[tuple[int, int, str]]:
    comments: list[tuple[int, int, str]] = []
    offset = 0
    for script in SCRIPT.finditer(text):
        comments.extend(
            (offset + match.start(), offset + match.end(), match[0])
            for match in HTML_COMMENT.finditer(text[offset : script.start()])
        )
        comments.extend(
            (script.start(1) + match.start(), script.start(1) + match.end(), match[0])
            for match in JS_TOKEN.finditer(script[1])
            if match.group("comment") is not None
        )
        offset = script.end()
    comments.extend(
        (offset + match.start(), offset + match.end(), match[0])
        for match in HTML_COMMENT.finditer(text[offset:])
    )
    return comments


def suppression_findings(text: str) -> list[str]:
    problems = []
    previous_end = -1
    previous_reason = False
    for start, end, body in comment_spans(text):
        line = text.count("\n", 0, start) + 1
        content = re.sub(r"^(?://|/\*|<!--)|(?:\*/|-->)$", "", body).strip()
        directive = re.match(r"svelte-ignore\b(.*)", content, re.S)
        if directive and not (
            re.search(r"\s--\s+\S", directive[1]) or (previous_reason and previous_end == line - 1)
        ):
            problems.append(f"{line}: svelte-ignore needs an inline or preceding comment reason")
        previous_end = text.count("\n", 0, end) + 1
        previous_reason = directive is None and bool(re.search(r"[A-Za-z]", content))
    return problems


def line_findings(text: str) -> list[str]:
    problems = []
    for number, line in enumerate(text.split("\n"), 1):
        if "\r" in line:
            problems.append(f"{number}: use LF line endings")
        if re.search(r"[ \t]+$", line.removesuffix("\r")):
            problems.append(f"{number}: trailing whitespace")
        if "\u2014" in line:
            problems.append(f"{number}: rewrite the em dash as natural punctuation")
    return problems


def check_text(path: str, data: bytes) -> list[str]:
    if excluded(path) or b"\0" in data:
        return []
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        # Fonts and images are binary. All authored source uses UTF-8.
        if Path(path).suffix in {".woff2", ".png", ".jpg", ".ico", ".pdf"}:
            return []
        return [f"{path}: text must be valid UTF-8"]
    problems = []
    if data and not data.endswith(b"\n"):
        problems.append("EOF: missing final LF")
    if path == "CHANGELOG.md":
        text = editable_changelog(data).decode("utf-8")
    problems.extend(line_findings(text))
    if path.endswith(".svelte"):
        problems.extend(suppression_findings(text))
    return [f"{path}:{problem}" for problem in problems]


def tracked_files(root: Path, revision: str | None) -> list[str]:
    if revision is None:
        names = git(root, "ls-files", "-z", "--cached")
    else:
        names = git(root, "ls-tree", "-r", "--name-only", "-z", revision)
    return sorted({name.decode("utf-8") for name in names.split(b"\0") if name})


def findings(root: Path, revision: str | None = None) -> list[str]:
    problems = []
    for path in tracked_files(root, revision):
        if excluded(path):
            continue
        source = root / path
        if revision is None and source.is_symlink():
            continue
        if revision is None and not source.exists():
            # A tracked deletion has no remaining working text to check.
            continue
        data = git(root, "show", f"{revision}:{path}") if revision else source.read_bytes()
        problems.extend(check_text(path, data))
    return problems


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", help="inspect committed text instead of the working tree")
    args = parser.parse_args()
    try:
        problems = findings(ROOT, args.revision)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Repository hygiene could not be checked: {error}\n")
    if problems:
        parser.exit(1, "\n".join(problems) + "\n")
    print("Tracked text and Svelte suppression reasons pass.")


if __name__ == "__main__":
    main()
