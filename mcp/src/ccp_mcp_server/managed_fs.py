# SPDX-License-Identifier: AGPL-3.0-or-later
"""Private local bundle storage; writer serialization is not server authority."""

from __future__ import annotations

import json
import os
import stat
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from .release_contract import read_file, require, unique


def directory(path: Path, create: bool = False) -> None:
    require(
        os.name == "posix" and path.is_absolute(), "Managed bundles require an absolute POSIX store"
    )
    if create:
        missing = []
        parent = path
        while not parent.exists() and not parent.is_symlink():
            missing.append(parent)
            parent = parent.parent
        for component in reversed(missing):
            component.mkdir(mode=0o700)
            sync(component.parent)
    info = path.lstat()
    require(
        stat.S_ISDIR(info.st_mode)
        and info.st_uid == os.getuid()
        and stat.S_IMODE(info.st_mode) & 0o077 == 0,
        "Bundle directory must be owner-only and not a symlink",
    )


def private_file(path: Path, maximum: int) -> bytes:
    info = path.lstat()
    require(
        stat.S_ISREG(info.st_mode)
        and info.st_uid == os.getuid()
        and stat.S_IMODE(info.st_mode) & 0o077 == 0,
        "Bundle metadata must be an owner-only regular file",
    )
    return read_file(path, maximum)


def sync(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def sync_tree(root: Path) -> None:
    for current, _directories, files in os.walk(root, topdown=False, followlinks=False):
        for name in files:
            path = Path(current) / name
            if not path.is_symlink():
                require(stat.S_ISREG(path.lstat().st_mode), "Unexpected candidate filesystem entry")
                sync(path)
        sync(Path(current))


def atomic(path: Path, value: dict[str, object]) -> None:
    directory(path.parent)
    if path.exists() or path.is_symlink():
        private_file(path, 64 * 1024)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".write-", delete=False) as stream:
            temporary = Path(stream.name)
            stream.write((json.dumps(value, sort_keys=True) + "\n").encode())
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        temporary = None
        sync(path.parent)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def load(path: Path) -> object:
    try:
        return json.loads(private_file(path, 64 * 1024), object_pairs_hook=unique)
    except (json.JSONDecodeError, UnicodeDecodeError, RecursionError):
        raise ValueError("Invalid managed bundle metadata") from None


@contextmanager
def locked(root: Path, *, shared: bool = False) -> Iterator[None]:
    directory(root, create=not shared)
    # Imported after the POSIX guard so unsupported hosts can still read CLI help.
    import fcntl

    path = root / ".lock"
    flags = os.O_RDONLY if shared else os.O_RDWR | os.O_CREAT
    descriptor = os.open(path, flags | os.O_NOFOLLOW, 0o600)
    try:
        private_file(path, 1024)
        try:
            mode = fcntl.LOCK_SH if shared else fcntl.LOCK_EX
            fcntl.flock(descriptor, mode | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError("Another managed client update is active; no change made") from None
        try:
            yield
        finally:
            fcntl.flock(descriptor, fcntl.LOCK_UN)
    finally:
        os.close(descriptor)
