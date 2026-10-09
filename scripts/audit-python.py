# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Audit exact installed dependencies while excluding this verified local checkout."""

from __future__ import annotations

import importlib.metadata as metadata
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def requirements() -> list[str]:
    installed = []
    project_found = False
    for distribution in metadata.distributions():
        name = distribution.metadata["Name"]
        if name.lower().replace("_", "-") == "ccp-mcp-server":
            # Editable installs can expose both dist-info with origin metadata
            # and source egg-info without it. Require a verified origin overall.
            origin = distribution.read_text("direct_url.json")
            if origin is not None:
                direct_url = json.loads(origin)
                if direct_url.get("url") != (ROOT / "mcp").as_uri():
                    raise RuntimeError(
                        "Install this checkout's MCP package with pip install -e ./mcp"
                    )
                project_found = True
            continue
        installed.append(f"{name}=={distribution.version}")
    if not project_found:
        raise RuntimeError("Install the MCP package before auditing its runtime dependencies")
    return sorted(set(installed))


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="ccp-python-audit-") as directory:
        pinned = Path(directory) / "requirements.txt"
        pinned.write_text("\n".join(requirements()) + "\n")
        # No resolver/install step: audit every exact third-party version already
        # installed in this interpreter, including transitive and quality-tool deps.
        command = [
            sys.executable,
            "-m",
            "pip_audit",
            "--strict",
            "--disable-pip",
            "--no-deps",
            "-r",
            str(pinned),
        ]
        subprocess.run(command, check=True)  # noqa: S603 -- Fixed argv; no shell.


if __name__ == "__main__":
    main()
