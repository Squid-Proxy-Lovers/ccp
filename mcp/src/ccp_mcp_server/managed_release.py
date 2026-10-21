# SPDX-License-Identifier: AGPL-3.0-or-later
"""Select official immutable tag names and compare source commits, never dates."""

from __future__ import annotations

import re
from pathlib import Path
from typing import cast

from . import managed_build as build
from . import managed_network as network
from . import release_contract as contract


def tag(value: object, channel: str) -> str:
    pattern = (
        r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?"
        if channel == "stable"
        else r"nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9a-f]{12}"
    )
    contract.require(channel in {"stable", "nightly"}, "Unknown update channel")
    return contract.text(value, pattern)


def eligible(value: object, channel: str) -> str | None:
    contract.require(isinstance(value, dict), "Invalid official release metadata")
    row = cast(dict[str, object], value)
    contract.require(
        type(row.get("draft")) is bool and type(row.get("prerelease")) is bool,
        "Invalid official release flags",
    )
    name = contract.text(row.get("tag_name"), r"[0-9A-Za-z.+-]{1,128}")
    if row["draft"] or (channel == "stable" and row["prerelease"]):
        return None
    if channel == "nightly" and not re.fullmatch(
        r"nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9a-f]{12}", name
    ):
        return None
    return tag(name, channel)


def discover(channel: str, pin: str | None) -> str:
    if pin is not None:
        expected = tag(pin, channel)
        contract.require(
            eligible(network.api("releases/tags/" + expected), channel) == expected,
            "Pinned release is unavailable",
        )
        return expected
    if channel == "stable":
        selected = eligible(network.api("releases/latest"), channel)
        contract.require(selected is not None, "Stable release is unavailable")
        return cast(str, selected)
    for page in range(1, 4):
        rows = network.api(f"releases?per_page=100&page={page}")
        contract.require(isinstance(rows, list) and len(rows) <= 100, "Invalid release page")
        for row in cast(list[object], rows):
            selected = eligible(row, channel)
            if selected is not None:
                return selected
        if len(cast(list[object], rows)) < 100:
            break
    raise ValueError("Nightly release is unavailable within discovery bounds")


def ancestry(base: str, candidate: str) -> None:
    base = contract.text(base, "[0-9a-f]{40}")
    candidate = contract.text(candidate, "[0-9a-f]{40}")
    if base == candidate:
        return
    value = network.api(f"compare/{base}...{candidate}?per_page=1")
    contract.require(isinstance(value, dict), "Unknown source ancestry")
    row = cast(dict[str, object], value)
    commit = row.get("base_commit")
    merge = row.get("merge_base_commit")
    contract.require(
        isinstance(commit, dict)
        and commit.get("sha") == base
        and isinstance(merge, dict)
        and merge.get("sha") == base
        and row.get("status") == "ahead"
        and type(row.get("behind_by")) is int
        and row["behind_by"] == 0,
        "Update source is not a verified descendant; no automatic rollback",
    )


def download(directory: Path, channel: str, selected: str) -> contract.Manifest:
    tag(selected, channel)
    platform = build.host_platform()
    for name, maximum in [
        (contract.MANIFEST, 64 * 1024),
        (contract.PROVENANCE, 4 * 1024 * 1024),
    ]:
        network.asset(selected, name, directory / name, maximum)
    manifest = contract.decode(contract.read_file(directory / contract.MANIFEST, 64 * 1024))
    ref = contract.source_ref(channel, selected, manifest)
    contract.verify_provenance(directory, manifest, ref)
    build.compatible(manifest)
    names = (f"ccp-client-{platform}", "ccp-mcp.tar.gz", "build-info.json")
    for name in names:
        network.asset(selected, name, directory / name, manifest.artifacts[name].size)
    manifest.verify_files(directory, names)
    return manifest
