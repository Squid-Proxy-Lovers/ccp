#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Bind official release bytes to explicit contracts and GitHub build provenance."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import cast

REPOSITORY = "Squid-Proxy-Lovers/ccp"
WORKFLOW = REPOSITORY + "/.github/workflows/ci.yml"
MANIFEST = "ccp-release.json"
PROVENANCE = "ccp-provenance.json"
PLATFORMS = ("linux-x86_64", "linux-aarch64", "darwin-x86_64", "darwin-aarch64")
ASSETS = tuple(
    f"ccp-{binary}-{platform}" for platform in PLATFORMS for binary in ("client", "server")
) + ("ccp-mcp.tar.gz", "build-info.json")
CONTRACTS = {
    "native_wire",
    "workspace_api",
    "client_profile_read_min",
    "client_profile_write",
    "server_schema_write",
}
MAX_ARTIFACT = 256 * 1024 * 1024


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise ValueError(reason)


def fields(value: object, keys: set[str]) -> dict[str, object]:
    require(isinstance(value, dict) and set(value) == keys, "Invalid release fields")
    return cast(dict[str, object], value)


def number(value: object, maximum: int = 2**32 - 1) -> int:
    require(type(value) is int, "Invalid release integer")
    result = cast(int, value)
    require(1 <= result <= maximum, "Release integer out of bounds")
    return result


def text(value: object, pattern: str) -> str:
    require(isinstance(value, str), "Invalid release text")
    result = cast(str, value)
    require(re.fullmatch(pattern, result) is not None, "Invalid release identity")
    return result


def unique(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        require(key not in result, "Duplicate release field")
        result[key] = value
    return result


def read_file(path: Path, maximum: int) -> bytes:
    mode = path.lstat()
    require(stat.S_ISREG(mode.st_mode) and mode.st_size <= maximum, "Invalid release file")
    with path.open("rb") as stream:
        result = stream.read(maximum + 1)
    require(len(result) <= maximum, "Release file exceeds limit")
    return result


def digest(path: Path) -> tuple[int, str]:
    mode = path.lstat()
    require(
        stat.S_ISREG(mode.st_mode) and 0 < mode.st_size <= MAX_ARTIFACT, "Invalid artifact file"
    )
    total, hashed = 0, hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            total += len(chunk)
            require(total <= MAX_ARTIFACT, "Artifact exceeds limit")
            hashed.update(chunk)
    require(total == mode.st_size, "Artifact changed while reading")
    return total, hashed.hexdigest()


@dataclass(frozen=True)
class Artifact:
    size: int
    sha256: str


@dataclass(frozen=True)
class Manifest:
    source_sha: str
    package_version: str
    contracts: dict[str, int]
    artifacts: dict[str, Artifact]

    def value(self) -> dict[str, object]:
        return {
            "format": 1,
            "repository": REPOSITORY,
            "source_sha": self.source_sha,
            "package_version": self.package_version,
            "platforms": list(PLATFORMS),
            "contracts": self.contracts,
            "artifacts": {
                name: {"size": item.size, "sha256": item.sha256}
                for name, item in self.artifacts.items()
            },
        }

    def verify_files(self, directory: Path, names: tuple[str, ...]) -> None:
        for name in names:
            require(name in self.artifacts, "Unexpected release artifact")
            item = self.artifacts[name]
            require(
                digest(directory / name) == (item.size, item.sha256),
                "Release artifact differs from manifest",
            )


def decode(raw: bytes) -> Manifest:
    require(len(raw) <= 64 * 1024, "Release manifest exceeds limit")
    try:
        decoded = json.loads(raw, object_pairs_hook=unique)
    except (json.JSONDecodeError, UnicodeDecodeError, RecursionError):
        raise ValueError("Invalid release manifest JSON") from None
    data = fields(
        decoded,
        {
            "format",
            "repository",
            "source_sha",
            "package_version",
            "platforms",
            "contracts",
            "artifacts",
        },
    )
    require(
        type(data["format"]) is int and data["format"] == 1, "Unsupported release manifest format"
    )
    require(
        data["repository"] == REPOSITORY and data["platforms"] == list(PLATFORMS),
        "Unexpected release source or platforms",
    )
    contracts = {key: number(value) for key, value in fields(data["contracts"], CONTRACTS).items()}
    require(
        contracts["client_profile_read_min"] <= contracts["client_profile_write"],
        "Invalid client profile range",
    )
    artifacts: dict[str, Artifact] = {}
    for name, value in fields(data["artifacts"], set(ASSETS)).items():
        item = fields(value, {"size", "sha256"})
        artifacts[name] = Artifact(
            number(item["size"], MAX_ARTIFACT), text(item["sha256"], "[0-9a-f]{64}")
        )
    return Manifest(
        text(data["source_sha"], "[0-9a-f]{40}"),
        text(data["package_version"], r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?"),
        contracts,
        artifacts,
    )


def source_number(root: Path, filename: str, constant: str) -> int:
    raw = (root / filename).read_text()
    values = re.findall(rf"(?:pub(?:\([^)]*\))?\s+)?const {constant}: u32 = ([0-9]+);", raw)
    require(len(values) == 1, "Missing or duplicate source contract")
    return number(int(values[0]))


def source_version(root: Path) -> str:
    versions: list[str] = []
    for filename in (
        "crates/client/Cargo.toml",
        "crates/server/Cargo.toml",
        "crates/protocol/Cargo.toml",
        "mcp/pyproject.toml",
    ):
        matches = re.findall(r'^version = "([^"]+)"$', (root / filename).read_text(), re.M)
        require(len(matches) == 1, "Missing package version")
        versions.append(matches[0])
    require(len(set(versions)) == 1, "Client, server and MCP package versions differ")
    return versions[0]


def source_profile_min(root: Path) -> int:
    return source_number(root, "crates/client/src/workspace/store.rs", "PROFILE_MIN_FORMAT")


def prepare(root: Path, directory: Path, source_sha: str) -> None:
    contracts = {
        "native_wire": source_number(root, "crates/protocol/src/lib.rs", "PROTOCOL_VERSION"),
        "workspace_api": source_number(root, "crates/protocol/src/workspace.rs", "API_VERSION"),
        "client_profile_read_min": source_profile_min(root),
        "client_profile_write": source_number(
            root, "crates/client/src/workspace/store.rs", "PROFILE_FORMAT"
        ),
        "server_schema_write": source_number(root, "crates/server/src/init.rs", "SCHEMA_VERSION"),
    }
    artifacts = {name: Artifact(*digest(directory / name)) for name in ASSETS}
    value = Manifest(source_sha, source_version(root), contracts, artifacts).value()
    encoded = (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()
    decode(encoded).verify_files(directory, ASSETS)
    (directory / MANIFEST).write_bytes(encoded)


def source_ref(channel: str, tag: str, manifest: Manifest) -> str:
    if channel == "nightly":
        text(tag, r"nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9a-f]{12}")
        require(tag.endswith("-" + manifest.source_sha[:12]), "Nightly tag and source differ")
        return "refs/heads/main"
    require(channel == "stable", "Unsupported release channel")
    require(tag == "v" + manifest.package_version, "Stable tag and package version differ")
    return "refs/tags/" + tag


def verify_provenance(directory: Path, manifest: Manifest, ref: str) -> None:
    require(
        ref in ("refs/heads/main", "refs/tags/v" + manifest.package_version), "Untrusted source ref"
    )
    raw = read_file(directory / MANIFEST, 64 * 1024)
    require(decode(raw) == manifest, "Release manifest changed")
    bundle = read_file(directory / PROVENANCE, 4 * 1024 * 1024)
    # Immutable private copies bind CLI verification to the exact parsed bytes.
    with tempfile.TemporaryDirectory(prefix="ccp-provenance-") as temporary:
        stage = Path(temporary)
        for name, body in ((MANIFEST, raw), (PROVENANCE, bundle)):
            path = stage / name
            path.touch(mode=0o600)
            path.write_bytes(body)
        command = [
            "gh",
            "attestation",
            "verify",
            str(stage / MANIFEST),
            "--bundle",
            str(stage / PROVENANCE),
            "--hostname",
            "github.com",
            "--repo",
            REPOSITORY,
            "--cert-identity",
            f"https://github.com/{WORKFLOW}@{ref}",
            "--cert-oidc-issuer",
            "https://token.actions.githubusercontent.com",
            "--source-ref",
            ref,
            "--source-digest",
            manifest.source_sha,
            "--signer-digest",
            manifest.source_sha,
            "--deny-self-hosted-runners",
        ]
        try:
            # Fixed verifier executable; validated identities are argv, never shell code.
            subprocess.run(command, check=True, capture_output=True, timeout=60)  # noqa: S603
        except (OSError, subprocess.SubprocessError):
            raise ValueError(
                "Official release provenance could not be verified; no checksum fallback"
            ) from None


def verify(directory: Path, channel: str, tag: str, platform: str) -> Manifest:
    require(platform in PLATFORMS, "Unsupported client platform")
    manifest = decode(read_file(directory / MANIFEST, 64 * 1024))
    ref = source_ref(channel, tag, manifest)
    verify_provenance(directory, manifest, ref)
    manifest.verify_files(
        directory, (f"ccp-client-{platform}", "ccp-mcp.tar.gz", "build-info.json")
    )
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--channel", choices=("stable", "nightly"), required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    args = parser.parse_args()
    try:
        manifest = verify(args.directory, args.channel, args.tag, args.platform)
        print(json.dumps(manifest.value(), sort_keys=True))
    except (ValueError, OSError, KeyError):
        parser.exit(1, "Release verification failed; no software was installed.\n")


if __name__ == "__main__":
    main()
