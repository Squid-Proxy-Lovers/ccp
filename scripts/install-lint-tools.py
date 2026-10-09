# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Install checksum-pinned native lint tools without changing system packages."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import platform
import tarfile
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def install(name: str, asset: dict[str, str], destination: Path) -> None:
    url = asset["url"]
    if not url.startswith("https://github.com/"):
        raise ValueError("lint tool downloads must use HTTPS GitHub release URLs")
    # S310: the checked-in manifest supplies a validated HTTPS release URL.
    with urllib.request.urlopen(url, timeout=60) as response:  # noqa: S310
        archive = response.read()
    if hashlib.sha256(archive).hexdigest() != asset["sha256"]:
        raise ValueError(f"checksum mismatch for {name}")
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as bundle:
        members = [
            member
            for member in bundle.getmembers()
            if member.isfile() and Path(member.name).name == name
        ]
        if len(members) != 1:
            raise ValueError(f"expected exactly one {name} executable")
        stream = bundle.extractfile(members[0])
        if stream is None:
            raise ValueError(f"missing {name} executable")
        with stream:
            binary = stream.read()
    destination.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=destination, delete=False) as output:
            temporary = Path(output.name)
            output.write(binary)
        temporary.chmod(0o755)
        os.replace(temporary, destination / name)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print(f"Installed {name} in {destination}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / ".lint-tools" / "bin")
    args = parser.parse_args()
    machine = {"arm64": "aarch64", "amd64": "x86_64"}.get(
        platform.machine().lower(), platform.machine().lower()
    )
    target = f"{platform.system().lower()}-{machine}"
    manifest = json.loads((ROOT / "tools" / "lint-tools.json").read_text())
    for name, tool in manifest.items():
        asset = tool["assets"].get(target)
        if asset is None:
            parser.error(f"{name} does not support {target}; use Linux or macOS")
        install(name, asset, args.bin_dir)


if __name__ == "__main__":
    main()
