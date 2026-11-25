# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Reject Linux binaries that need a newer glibc than the supported floor."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

CEILING = re.compile(r"\d+\.\d+(?:\.\d+)?")
GLIBC_TAG = re.compile(r"GLIBC_(\d+(?:\.\d+)*)")
SECTION = re.compile(r"^Version (\w+) section")
NAME = re.compile(r"\bName:\s+(\S+)")
ENTRY_COUNT = re.compile(r"contains (\d+) entr")
LIBRARY = re.compile(r"\bFile:\s+(\S+)\s+Cnt:\s+(\d+)")

Version = tuple[int, ...]


class CheckError(Exception):
    """A binary cannot be shown to meet the glibc floor."""


def parse_version(text: str) -> Version:
    """Parse dotted digits, padding a missing patch so 2.28 equals 2.28.0."""
    parts = tuple(int(part) for part in text.split("."))
    return parts + (0,) * (3 - len(parts))


def ceiling_argument(text: str) -> str:
    if not CEILING.fullmatch(text):
        raise argparse.ArgumentTypeError(f"expected MAJOR.MINOR[.PATCH], got {text!r}")
    return text


def readelf_output(path: Path) -> str:
    if not os.access(path, os.R_OK) or not path.is_file():
        raise CheckError(f"{path}: file is missing or unreadable")
    try:
        result = subprocess.run(  # noqa: S603 -- Fixed argv; no shell.
            ["readelf", "-W", "-V", str(path)],  # noqa: S607 -- GNU readelf from PATH.
            check=False,
            capture_output=True,
            text=True,
            env={**os.environ, "LC_ALL": "C"},
        )
    except OSError as error:
        raise CheckError(f"{path}: cannot run readelf: {error}") from error
    detail = result.stderr.strip()
    if result.returncode != 0:
        raise CheckError(f"{path}: readelf failed: {detail or f'exit status {result.returncode}'}")
    if detail:
        # readelf exits 0 after warning about a corrupt table it only partly read.
        raise CheckError(f"{path}: readelf reported problems: {detail}")
    return result.stdout


def needs_section(output: str) -> tuple[int | None, list[str]]:
    """Return the declared entry count and body lines of the Version needs section."""
    declared: int | None = None
    body: list[str] = []
    in_needs = False
    for line in output.splitlines():
        section = SECTION.match(line)
        if section:
            in_needs = section.group(1) == "needs"
            count = ENTRY_COUNT.search(line) if in_needs else None
            declared = int(count.group(1)) if count else declared
        elif in_needs:
            body.append(line)
    return declared, body


def needed_names(path: Path, output: str) -> list[str]:
    """Return every name in the Version needs section, checking the declared counts."""
    declared, body = needs_section(output)
    names: list[str] = []
    expected: list[tuple[str, int]] = []
    seen: list[int] = []
    for line in body:
        library = LIBRARY.search(line)
        name = NAME.search(line)
        if library:
            expected.append((library.group(1), int(library.group(2))))
            seen.append(0)
        elif name and seen:
            names.append(name.group(1))
            seen[-1] += 1
        elif name:
            raise CheckError(f"{path}: version name {name.group(1)} appears before any library")
    if declared is not None and declared != len(expected):
        raise CheckError(
            f"{path}: version needs section declares {declared} libraries, found {len(expected)}"
        )
    for (soname, count), found in zip(expected, seen, strict=True):
        if count != found:
            raise CheckError(f"{path}: {soname} declares {count} versions, found {found}")
    return names


def max_required(path: Path, output: str) -> tuple[str, Version]:
    """Return the highest GLIBC tag required, rejecting tags that are not numeric."""
    found: list[tuple[Version, str]] = []
    for name in needed_names(path, output):
        if not name.startswith("GLIBC_"):
            continue
        if not GLIBC_TAG.fullmatch(name):
            raise CheckError(f"{path}: unrecognised glibc version tag {name}")
        found.append((parse_version(name.removeprefix("GLIBC_")), name))
    if not found:
        raise CheckError(f"{path}: no GLIBC version requirements found; not a glibc binary?")
    version, name = max(found)
    return name, version


def check_binary(path: Path, ceiling: str) -> str:
    output = readelf_output(path)
    name, version = max_required(path, output)
    if version > parse_version(ceiling):
        raise CheckError(f"{path}: requires {name}, newer than the GLIBC_{ceiling} floor")
    return f"{path}: {name}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--max-glibc", required=True, type=ceiling_argument)
    parser.add_argument("binaries", nargs="+", type=Path)
    arguments = parser.parse_args()
    failed = False
    for binary in arguments.binaries:
        try:
            print(check_binary(binary, arguments.max_glibc))
        except CheckError as error:
            print(f"error: {error}", file=sys.stderr)
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
