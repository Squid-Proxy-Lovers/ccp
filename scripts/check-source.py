# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Check repository shell scripts or secrets, including unstaged source changes."""

from __future__ import annotations

import argparse
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def source_files() -> list[Path]:
    # The executable and arguments are fixed; Git returns repository-relative paths.
    result = subprocess.run(  # noqa: S603
        [shutil.which("git"), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        capture_output=True,
        check=True,
    )
    return [
        Path(name.decode())
        for name in sorted(set(result.stdout.split(b"\0")))
        if name and (ROOT / name.decode()).is_file() and not (ROOT / name.decode()).is_symlink()
    ]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("check", choices=["shell", "secrets"])
    args = parser.parse_args()
    files = source_files()
    if args.check == "shell":
        shells = [
            str(path)
            for path in files
            if (ROOT / path).read_bytes().split(b"\n", 1)[0]
            in (b"#!/bin/sh", b"#!/bin/bash", b"#!/usr/bin/env bash", b"#!/usr/bin/env sh")
        ]
        command = ["shellcheck", "--severity=style", *shells]
        # ShellCheck receives filenames from Git as separate arguments, without a shell.
        subprocess.run(command, cwd=ROOT, check=True)  # noqa: S603
    else:
        # Copy only Git-visible files so ignored build/venv directories cannot
        # drown out findings. Include new and modified source before committing.
        with tempfile.TemporaryDirectory(prefix="ccp-secret-check-") as temporary:
            tree = Path(temporary)
            for path in files:
                output = tree / path
                output.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / path, output)
            command = [
                "gitleaks",
                "dir",
                "--redact",
                "--no-banner",
                "--config",
                str(ROOT / ".gitleaks.toml"),
                str(tree),
            ]
            # Gitleaks receives a fixed command and our temporary directory, without a shell.
            subprocess.run(command, cwd=ROOT, check=True)  # noqa: S603


if __name__ == "__main__":
    main()
