# SPDX-License-Identifier: AGPL-3.0-or-later
"""Build and inspect a candidate without profiles, enrollment or server requests."""

from __future__ import annotations

import json
import os
import platform
import subprocess
import sys
from pathlib import Path

from .managed_archive import expand
from .release_contract import PLATFORMS, Manifest, fields, require

MCP_HEALTH = """
import asyncio, importlib.metadata, sys
from pathlib import Path
from fastmcp import Client
from ccp_mcp_server.workspace import build_server
from ccp_mcp_server.workspace_config import WorkspaceConfig
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
if importlib.metadata.version("ccp-mcp-server") != sys.argv[1]:
    raise SystemExit("MCP package version differs")
async def check():
    config = WorkspaceConfig(Path(sys.argv[2]), {"health":Path(sys.argv[3])})
    with WorkspaceRuntime(config) as runtime:
        async with Client(build_server(runtime)) as client:
            tools = {tool.name for tool in await client.list_tools()}
            if not {"workspace_profile", "workspace_target", "workspace_mutate_entry", "workspace_status_messages"} <= tools:
                raise SystemExit("MCP workspace tool contract differs")
asyncio.run(check())
"""


def host_platform() -> str:
    system = {"Linux": "linux", "Darwin": "darwin"}.get(platform.system(), "unsupported")
    arch = {"x86_64": "x86_64", "amd64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(
        platform.machine(), "unsupported"
    )
    result = system + "-" + arch
    require(result in PLATFORMS, "Unsupported managed client platform")
    return result


def compatible(manifest: Manifest) -> None:
    require(
        manifest.contracts["native_wire"] == 1
        and manifest.contracts["workspace_api"] == 1
        and manifest.contracts["client_profile_write"] == 5,
        "Client contract requires a manual manager/migration upgrade",
    )


def run(arguments: tuple[str, ...], timeout: int = 60) -> bytes:
    environment = os.environ.copy()
    environment.pop("PYTHONPATH", None)
    environment.pop("PYTHONHOME", None)
    try:
        # Executables are the local Python, fixed pip module or private verified client.
        result = subprocess.run(  # noqa: S603
            arguments,
            env=environment,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            check=True,
            timeout=timeout,
        )
    except (OSError, subprocess.SubprocessError):
        raise ValueError(
            "Candidate build or health check failed; active software was not changed"
        ) from None
    require(len(result.stdout) <= 1024 * 1024, "Candidate health output exceeds limit")
    return result.stdout


def build(generation: Path, manifest: Manifest) -> None:
    compatible(manifest)
    client = generation / "client"
    value = fields(
        json.loads(run((str(client), "workspace", "software-info"))),
        {
            "format",
            "package_version",
            "native_wire",
            "workspace_api",
            "client_profile_read_min",
            "client_profile_write",
        },
    )
    expected: dict[str, object] = {
        "format": 1,
        "package_version": manifest.package_version,
        **{
            key: manifest.contracts[key]
            for key in [
                "native_wire",
                "workspace_api",
                "client_profile_read_min",
                "client_profile_write",
            ]
        },
    }
    require(
        all(type(value[key]) is int for key in expected if key != "package_version")
        and value == expected,
        "Client executable differs from release contracts",
    )
    project = expand(generation / "release/ccp-mcp.tar.gz", generation / "source")
    run((sys.executable, "-I", "-m", "venv", "--copies", str(generation / "venv")), 120)
    python = str(generation / "venv/bin/python")
    run(
        (
            python,
            "-I",
            "-m",
            "pip",
            "--disable-pip-version-check",
            "--no-input",
            "install",
            "--only-binary=:all:",
            str(project),
        ),
        300,
    )
    run(
        (
            python,
            "-I",
            "-c",
            MCP_HEALTH,
            manifest.package_version,
            str(client),
            str(generation / "health-profile"),
        )
    )
