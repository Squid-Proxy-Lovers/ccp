# SPDX-License-Identifier: AGPL-3.0-or-later
"""Inspect every explicitly configured profile through a verified candidate client."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from . import managed_build as build
from . import managed_fs as fs
from . import release_contract as contract
from .managed_store import Generation, identity
from .workspace_config import load_config


def config(path: Path, binary: Path) -> tuple[str, dict[str, Path]]:
    contract.require(path.is_absolute(), "Update config must be absolute")
    raw = fs.private_file(path, 64 * 1024)
    result = load_config(path, binary)
    contract.require(fs.private_file(path, 64 * 1024) == raw, "Update config changed")
    return hashlib.sha256(raw).hexdigest(), result.profiles


def inspect(generation: Generation, profiles: dict[str, Path]) -> list[dict[str, object]]:
    reports = []
    for alias, profile in profiles.items():
        raw = build.run(
            (
                str(generation.path / "client"),
                "workspace",
                "update-preflight",
                "--profile",
                str(profile),
            )
        )
        try:
            report = json.loads(raw, object_pairs_hook=contract.unique)
        except (ValueError, UnicodeDecodeError, RecursionError):
            raise ValueError("Invalid candidate preflight report") from None
        checked = validate(report, generation.manifest)
        reports.append({"profile": alias, **checked})
    return reports


def validate(value: object, manifest: contract.Manifest) -> dict[str, object]:
    row = contract.fields(
        value, {"format", "profile_format", "migration_required", "pending", "server"}
    )
    server = contract.fields(
        row["server"], {"instance_id", "package_version", "native_wire", "workspace_api"}
    )
    identity(server["instance_id"])
    contract.text(server["package_version"], r"[!-~]{1,64}")
    contract.require(
        type(row["format"]) is int
        and row["format"] == 1
        and contract.number(row["profile_format"]) == manifest.contracts["client_profile_write"]
        and row["migration_required"] is False
        and row["pending"] is False
        and contract.number(server["native_wire"]) == manifest.contracts["native_wire"]
        and contract.number(server["workspace_api"]) == manifest.contracts["workspace_api"],
        "Profile requires manual migration, pending resolution or compatibility review",
    )
    return row
