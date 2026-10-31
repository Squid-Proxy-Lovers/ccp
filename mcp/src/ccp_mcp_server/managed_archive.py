# SPDX-License-Identifier: AGPL-3.0-or-later
"""Bounded extraction of authenticated MCP source into a private generation."""

from __future__ import annotations

import gzip
import shutil
import tarfile
from pathlib import Path, PurePosixPath

from .managed_fs import directory
from .release_contract import require

MAX_EXPANDED = 32 * 1024 * 1024
MAX_MEMBERS = 512


def member_path(name: str) -> Path:
    raw = name.removesuffix("/")
    pure = PurePosixPath(raw)
    require(
        bool(raw) and bool(pure.parts) and pure.as_posix() == raw and pure.parts[0] == "ccp-mcp",
        "MCP archive has an unexpected root",
    )
    require(
        len(raw.encode()) <= 1024
        and not any(part in {".", ".."} for part in pure.parts)
        and not any(ord(c) < 32 or ord(c) == 127 or c in "\\:" for c in raw),
        "Invalid MCP archive path",
    )
    return Path(*pure.parts)


def expand(archive: Path, root: Path) -> Path:
    directory(root, create=True)
    plain = root / ".source.tar"
    try:
        total = 0
        with gzip.open(archive, "rb") as compressed, plain.open("xb") as output:
            while chunk := compressed.read(1024 * 1024):
                total += len(chunk)
                require(total <= MAX_EXPANDED, "MCP archive expands beyond limit")
                output.write(chunk)
        seen: set[Path] = set()
        total = 0
        with tarfile.open(plain, mode="r:") as source:
            for index, item in enumerate(source):
                require(
                    index < MAX_MEMBERS and (item.isdir() or item.isreg()),
                    "Unsupported MCP archive member",
                )
                relative = member_path(item.name)
                require(relative not in seen and item.size >= 0, "Duplicate MCP archive member")
                seen.add(relative)
                total += item.size
                require(total <= MAX_EXPANDED, "MCP archive exceeds content limit")
                target = root / relative
                if item.isdir():
                    directory(target, create=True)
                else:
                    extract(source, item, target)
        project = root / "ccp-mcp"
        require((project / "pyproject.toml").is_file(), "MCP package metadata is missing")
        return project
    except (tarfile.TarError, EOFError):
        raise ValueError("MCP archive could not be expanded") from None
    finally:
        plain.unlink(missing_ok=True)


def extract(source: tarfile.TarFile, item: tarfile.TarInfo, target: Path) -> None:
    directory(target.parent, create=True)
    stream = source.extractfile(item)
    if stream is None:
        raise ValueError("MCP archive file cannot be read")
    with stream, target.open("xb") as output:
        shutil.copyfileobj(stream, output, length=1024 * 1024)
    target.chmod(0o600)
    require(target.stat().st_size == item.size, "MCP archive file was truncated")
