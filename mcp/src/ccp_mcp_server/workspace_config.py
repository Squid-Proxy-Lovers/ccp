# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Explicit host configuration for the workspace-only stdio bridge."""

from __future__ import annotations

import json
import os
import re
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class WorkspaceConfig:
    binary: Path
    profiles: dict[str, Path]


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate configuration field")
        result[key] = value
    return result


def load_config(path: Path, binary: Path) -> WorkspaceConfig:
    if not path.is_absolute() or not binary.is_absolute():
        raise ValueError("workspace config and client binary must use absolute paths")
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size > 64 * 1024:
        raise ValueError("workspace configuration must be a bounded regular file")
    if os.name == "posix" and stat.S_IMODE(info.st_mode) & 0o077:
        raise ValueError("workspace configuration must have owner-only permissions")
    with path.open("rb") as stream:
        payload = stream.read(64 * 1024 + 1)
    if len(payload) > 64 * 1024:
        raise ValueError("workspace configuration exceeds the size limit")
    data = json.loads(payload, object_pairs_hook=unique_object)
    if (
        not isinstance(data, dict)
        or set(data) != {"format", "profiles"}
        or type(data["format"]) is not int
        or data["format"] != 1
    ):
        raise ValueError("unsupported workspace configuration")
    profiles = data["profiles"]
    if not isinstance(profiles, dict) or not 1 <= len(profiles) <= 32:
        raise ValueError("configure between one and 32 workspace profile aliases")
    result: dict[str, Path] = {}
    for alias, directory in profiles.items():
        if (
            not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", alias)
            or not isinstance(directory, str)
            or not Path(directory).is_absolute()
        ):
            raise ValueError("profile aliases need simple names and absolute directories")
        result[alias] = Path(directory)
    if not binary.is_file():
        raise ValueError("configured workspace client binary is missing")
    return WorkspaceConfig(binary, result)


def environment_config() -> WorkspaceConfig:
    path = os.environ.get("CCP_WORKSPACE_CONFIG")
    binary = os.environ.get("CCP_CLIENT_BIN")
    if not path or not binary:
        raise ValueError("set CCP_WORKSPACE_CONFIG and CCP_CLIENT_BIN explicitly")
    return load_config(Path(path), Path(binary))
