# SPDX-License-Identifier: AGPL-3.0-or-later
"""Verified private generations and an atomic, reversible software pointer."""

from __future__ import annotations

import hashlib
import os
import shutil
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import cast
from uuid import UUID, uuid4

from . import managed_build as build
from . import managed_fs as fs
from . import release_contract as contract


def identity(value: object) -> str:
    result = contract.text(value, "[0-9a-f-]{36}")
    parsed = UUID(result)
    contract.require(str(parsed) == result and parsed.int != 0, "Invalid generation identity")
    return result


def copy_file(source: Path, target: Path, maximum: int) -> None:
    info = source.lstat()
    contract.require(
        stat.S_ISREG(info.st_mode) and info.st_size <= maximum,
        "Candidate input must be a bounded regular file",
    )
    total = 0
    with source.open("rb") as reader, target.open("xb") as writer:
        target.chmod(0o600)
        while chunk := reader.read(1024 * 1024):
            total += len(chunk)
            contract.require(total <= maximum, "Candidate input exceeds limit")
            writer.write(chunk)
        writer.flush()
        os.fsync(writer.fileno())
    contract.require(total == info.st_size, "Candidate input changed while copying")


@dataclass(frozen=True)
class Generation:
    path: Path
    generation_id: str
    channel: str
    tag: str
    platform: str
    manifest: contract.Manifest

    def summary(self) -> dict[str, object]:
        return {
            "generation": self.generation_id,
            "channel": self.channel,
            "tag": self.tag,
            "platform": self.platform,
            "source_sha": self.manifest.source_sha,
            "package_version": self.manifest.package_version,
        }


@dataclass(frozen=True)
class Pointer:
    revision: int = 0
    current: str | None = None
    previous: str | None = None

    def value(self) -> dict[str, object]:
        return {
            "format": 1,
            "revision": self.revision,
            "current": self.current,
            "previous": self.previous,
        }


class Store:
    def __init__(self, root: Path):
        self.root = root

    def pointer(self) -> Pointer:
        fs.directory(self.root)
        path = self.root / "current.json"
        contract.require(
            path.exists() or path.is_symlink(),
            "Activation metadata is missing; no reset was attempted",
        )
        row = contract.fields(fs.load(path), {"format", "revision", "current", "previous"})
        contract.require(
            type(row["format"]) is int and row["format"] == 1, "Unsupported activation metadata"
        )
        revision = row["revision"]
        contract.require(
            type(revision) is int and 0 <= revision < 2**63, "Invalid activation revision"
        )
        current = None if row["current"] is None else identity(row["current"])
        previous = None if row["previous"] is None else identity(row["previous"])
        contract.require(
            (revision == 0) == (current is None)
            and (current is not None or previous is None)
            and (previous is None or previous != current),
            "Invalid activation state",
        )
        return Pointer(cast(int, revision), current, previous)

    def _initialize(self) -> None:
        path = self.root / "current.json"
        if not path.exists() and not path.is_symlink():
            bundles = self.root / "bundles"
            contract.require(
                not bundles.exists() and not bundles.is_symlink(),
                "Activation metadata is missing; preserve existing generations",
            )
            fs.atomic(path, Pointer().value())
        self.pointer()

    def generation(self, value: str) -> Generation:
        generation_id = identity(value)
        fs.directory(self.root / "bundles")
        path = self.root / "bundles" / generation_id
        fs.directory(path)
        fs.directory(path / "release")
        row = contract.fields(
            fs.load(path / "ready.json"),
            {"format", "generation", "channel", "tag", "platform", "manifest_sha256"},
        )
        contract.require(
            type(row["format"]) is int
            and row["format"] == 1
            and row["generation"] == generation_id,
            "Invalid generation metadata",
        )
        raw = fs.private_file(path / "release" / contract.MANIFEST, 64 * 1024)
        contract.require(
            hashlib.sha256(raw).hexdigest() == row["manifest_sha256"], "Stored manifest changed"
        )
        manifest = contract.decode(raw)
        channel = contract.text(row["channel"], "stable|nightly")
        tag = contract.text(row["tag"], "[0-9A-Za-z.-]+")
        contract.source_ref(channel, tag, manifest)
        platform = contract.text(row["platform"], "[a-z0-9_-]+")
        contract.require(
            platform == build.host_platform(), "Stored bundle belongs to another platform"
        )
        build.compatible(manifest)
        item = manifest.artifacts[f"ccp-client-{platform}"]
        contract.require(
            contract.digest(path / "client") == (item.size, item.sha256), "Stored client changed"
        )
        contract.require((path / "venv/bin/python").is_file(), "Stored MCP environment is missing")
        return Generation(path, generation_id, channel, tag, platform, manifest)

    def stage(self, source: Path, channel: str, tag: str) -> Generation:
        with fs.locked(self.root):
            self._initialize()
            fs.directory(self.root / "bundles", create=True)
            generation_id = str(uuid4())
            path = self.root / "bundles" / generation_id
            path.mkdir(mode=0o700)
            fs.directory(path)
        try:
            row = self._prepare(source, path, channel, tag)
            with fs.locked(self.root):
                fs.atomic(path / "ready.json", row)
                fs.sync(self.root / "bundles")
                return self.generation(path.name)
        finally:
            # A visible ready marker may already have been selected after a lost sync.
            if not (path / "ready.json").exists() and not (path / "ready.json").is_symlink():
                shutil.rmtree(path)

    def _prepare(self, source: Path, path: Path, channel: str, tag: str) -> dict[str, object]:
        platform = build.host_platform()
        release = path / "release"
        fs.directory(release, create=True)
        for name, maximum in [
            (contract.MANIFEST, 64 * 1024),
            (contract.PROVENANCE, 4 * 1024 * 1024),
            (f"ccp-client-{platform}", contract.MAX_ARTIFACT),
            ("ccp-mcp.tar.gz", contract.MAX_ARTIFACT),
            ("build-info.json", 64 * 1024),
        ]:
            copy_file(source / name, release / name, maximum)
        manifest = contract.verify(release, channel, tag, platform)
        verified_manifest = fs.private_file(release / contract.MANIFEST, 64 * 1024)
        contract.require(
            contract.decode(verified_manifest) == manifest, "Verified manifest changed"
        )
        build.compatible(manifest)
        copy_file(release / f"ccp-client-{platform}", path / "client", contract.MAX_ARTIFACT)
        (path / "client").chmod(0o700)
        build.build(path, manifest)
        contract.require(
            fs.private_file(release / contract.MANIFEST, 64 * 1024) == verified_manifest,
            "Manifest changed during candidate build",
        )
        item = manifest.artifacts[f"ccp-client-{platform}"]
        contract.require(
            contract.digest(path / "client") == (item.size, item.sha256),
            "Client changed during candidate build",
        )
        manifest.verify_files(
            release, (f"ccp-client-{platform}", "ccp-mcp.tar.gz", "build-info.json")
        )
        row: dict[str, object] = {
            "format": 1,
            "generation": path.name,
            "channel": channel,
            "tag": tag,
            "platform": platform,
            "manifest_sha256": hashlib.sha256(verified_manifest).hexdigest(),
        }
        fs.sync_tree(path)
        return row

    def activate(self, generation_id: str, expected_revision: int) -> Pointer:
        with fs.locked(self.root):
            self._initialize()
            self.generation(generation_id)
            return self._activate(generation_id, expected_revision)

    def _activate(self, generation_id: str, expected_revision: int) -> Pointer:
        contract.require(
            type(expected_revision) is int and 0 <= expected_revision < 2**63 - 1,
            "Invalid expected activation revision",
        )
        current = self.pointer()
        if current.current == generation_id and current.revision in {
            expected_revision,
            expected_revision + 1,
        }:
            return current
        contract.require(
            current.revision == expected_revision,
            "Activation changed; inspect status before another decision",
        )
        result = Pointer(current.revision + 1, generation_id, current.current)
        fs.atomic(self.root / "current.json", result.value())
        return result

    def rollback(self, expected_revision: int) -> Pointer:
        with fs.locked(self.root):
            self._initialize()
            current = self.pointer()
            if current.previous is None:
                raise ValueError("No previous managed generation")
            self.generation(current.previous)
            return self._activate(current.previous, expected_revision)

    def status(self) -> dict[str, object]:
        with fs.locked(self.root, shared=True):
            pointer = self.pointer()
            return {
                "pointer": pointer.value(),
                "active": None
                if pointer.current is None
                else self.generation(pointer.current).summary(),
                "previous": None
                if pointer.previous is None
                else self.generation(pointer.previous).summary(),
            }

    def launch(
        self, mode: str, arguments: tuple[str, ...]
    ) -> tuple[Path, tuple[str, ...], dict[str, str]]:
        contract.require(mode in {"client", "mcp"}, "Unknown managed launch mode")
        with fs.locked(self.root, shared=True):
            pointer = self.pointer()
            if pointer.current is None:
                raise ValueError("Activate a verified managed bundle first")
            generation = self.generation(pointer.current)
            client = generation.path / "client"
            environment = os.environ.copy()
            environment["CCP_CLIENT_BIN"] = str(client)
            if mode == "client":
                return client, (str(client), *arguments), environment
            contract.require(
                not arguments,
                "Managed workspace MCP takes its host configuration from the environment",
            )
            python = generation.path / "venv/bin/python"
            return python, (str(python), "-I", "-m", "ccp_mcp_server.workspace"), environment
