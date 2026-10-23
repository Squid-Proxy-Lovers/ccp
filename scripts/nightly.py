#!/usr/bin/env python3
"""Package reviewed artifacts and publish main's opt-in nightly channel."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path

import release_contract

ROOT = Path(__file__).resolve().parents[1]
PLATFORMS = ("linux-x86_64", "linux-aarch64", "darwin-x86_64", "darwin-aarch64")
ASSETS = tuple(
    f"ccp-{binary}-{platform}" for platform in PLATFORMS for binary in ("client", "server")
) + ("ccp-mcp.tar.gz",)
TAG_PATTERN = re.compile(r"nightly-\d{4}-\d{2}-\d{2}-[0-9a-f]{12}")


def run(*args: str, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess[str]:
    # Only fixed executables and argv are used; release/ref values are validated below.
    return subprocess.run(args, check=check, text=True, capture_output=capture)  # noqa: S603


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_sha(sha: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("A full source commit SHA is required")


def prepare(dist: Path, sha: str) -> None:
    validate_sha(sha)
    for name in ASSETS:
        path = dist / name
        if not path.is_file() or path.is_symlink() or not path.stat().st_size:
            raise ValueError(f"Missing or empty release artifact: {name}")
    toolchain = re.search(
        r'^channel\s*=\s*"([^"]+)"', (ROOT / "rust-toolchain.toml").read_text(), re.M
    )
    if not toolchain:
        raise ValueError("Pinned Rust compiler is missing")
    metadata = {
        "source_sha": sha,
        "rust_toolchain": toolchain.group(1),
        "platforms": list(PLATFORMS),
        "artifacts": list(ASSETS),
    }
    (dist / "build-info.json").write_text(json.dumps(metadata, indent=2) + "\n")
    release_contract.prepare(ROOT, dist, sha)
    names = (*ASSETS, "build-info.json", release_contract.MANIFEST)
    (dist / "SHA256SUMS").write_text("".join(f"{digest(dist / name)}  {name}\n" for name in names))


def verify(dist: Path, sha: str) -> None:
    validate_sha(sha)
    if json.loads((dist / "build-info.json").read_text())["source_sha"] != sha:
        raise ValueError("Release artifacts belong to another commit")
    expected = set((*ASSETS, "build-info.json", release_contract.MANIFEST))
    seen = set()
    for line in (dist / "SHA256SUMS").read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([a-zA-Z0-9_.-]+)", line)
        if not match:
            raise ValueError("Invalid checksum manifest")
        checksum, name = match.groups()
        path = dist / name
        if name not in expected or name in seen or path.is_symlink() or digest(path) != checksum:
            raise ValueError(f"Invalid release artifact: {name}")
        seen.add(name)
    if seen != expected:
        raise ValueError("Incomplete checksum manifest")
    manifest = release_contract.decode(
        release_contract.read_file(dist / release_contract.MANIFEST, 64 * 1024)
    )
    if manifest.source_sha != sha:
        raise ValueError("Release contract belongs to another commit")
    manifest.verify_files(dist, release_contract.ASSETS)


def promote(image: str, tag: str) -> None:
    # Use the already-published image, including when repairing a failed alias update.
    run("docker", "pull", f"{image}:{tag}")
    run("docker", "tag", f"{image}:{tag}", f"{image}:nightly")
    run("docker", "push", f"{image}:nightly")


# C901: publication is one ordered, recoverable release transaction; see tests/test_nightly.py.
def publish(dist: Path, archive: Path) -> None:  # noqa: C901
    sha = os.environ["GITHUB_SHA"]
    repo = os.environ["GITHUB_REPOSITORY"]
    if os.environ.get("GITHUB_REF") != "refs/heads/main" or os.environ.get(
        "GITHUB_EVENT_NAME"
    ) not in (
        "schedule",
        "workflow_dispatch",
    ):
        raise ValueError("Nightlies can only be published by scheduled/manual main builds")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("Invalid GitHub repository")
    if repo != release_contract.REPOSITORY:
        raise ValueError("Nightly publication requires the official repository")
    verify(dist, sha)
    manifest = release_contract.decode(
        release_contract.read_file(dist / release_contract.MANIFEST, 64 * 1024)
    )
    release_contract.verify_provenance(dist, manifest, "refs/heads/main")
    pages = json.loads(
        run(
            "gh",
            "api",
            "--paginate",
            "--slurp",
            f"repos/{repo}/releases?per_page=100",
            capture=True,
        ).stdout
    )
    releases = [release for page in pages for release in page]
    nightlies = [
        release
        for release in releases
        if TAG_PATTERN.fullmatch(release["tag_name"])
        and release["prerelease"]
        and not release["draft"]
    ]
    latest = max(nightlies, key=lambda release: release["published_at"], default=None)
    image = f"ghcr.io/{repo.lower()}"
    existing = next((release for release in nightlies if release["target_commitish"] == sha), None)
    if existing:
        print(f"Reusing published nightly {existing['tag_name']}", flush=True)
        if existing == latest:
            promote(image, existing["tag_name"])
        return
    if latest:
        previous_sha = latest["target_commitish"]
        validate_sha(previous_sha)
        ancestry = run("git", "merge-base", "--is-ancestor", sha, previous_sha, check=False)
        if ancestry.returncode == 0:
            print("A newer main commit already has a nightly; skipping this older run.", flush=True)
            return
        if ancestry.returncode != 1:
            raise ValueError("Could not verify nightly commit ancestry")

    tag = f"nightly-{datetime.now(timezone.utc):%Y-%m-%d}-{sha[:12]}"
    draft = next((release for release in releases if release["tag_name"] == tag), None)
    if draft and (
        not draft["draft"] or draft["target_commitish"] != sha or not draft["prerelease"]
    ):
        raise ValueError("The nightly tag is already used by a different release")
    run("docker", "load", "--input", str(archive))
    local = f"ccp-ci:{sha}"
    revision = run(
        "docker",
        "image",
        "inspect",
        local,
        "--format",
        '{{ index .Config.Labels "org.opencontainers.image.revision" }}',
        capture=True,
    ).stdout.strip()
    if revision != sha:
        raise ValueError("Container image belongs to another commit")
    if not draft:
        with tempfile.TemporaryDirectory() as directory:
            notes = Path(directory) / "notes.md"
            notes.write_text(
                f"Nightly preview of main at [{sha}](https://github.com/{repo}/commit/{sha}).\n\n"
                f"Docker: `{image}:{tag}`. Client, server, and MCP archive share this source commit.\n"
                "See build-info.json and SHA256SUMS for build metadata and checksums.\n"
            )
            run(
                "gh",
                "release",
                "create",
                tag,
                "--repo",
                repo,
                "--target",
                sha,
                "--draft",
                "--prerelease",
                "--latest=false",
                "--title",
                tag,
                "--notes-file",
                str(notes),
            )
    run(
        "gh",
        "release",
        "upload",
        tag,
        "--repo",
        repo,
        "--clobber",
        *(
            str(dist / name)
            for name in (
                *ASSETS,
                "build-info.json",
                "SHA256SUMS",
                release_contract.MANIFEST,
                release_contract.PROVENANCE,
            )
        ),
    )
    run("docker", "tag", local, f"{image}:{tag}")
    run("docker", "push", f"{image}:{tag}")
    # Expose the release only after every asset and its pinned image are available.
    run("gh", "release", "edit", tag, "--repo", repo, "--draft=false", "--latest=false")
    promote(image, tag)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "publish"))
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--sha", default=os.environ.get("GITHUB_SHA"))
    parser.add_argument("--image", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            prepare(args.dist, args.sha or "")
        elif args.image:
            publish(args.dist, args.image)
        else:
            parser.error("publish requires --image")
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Nightly failed: {error}\n")


if __name__ == "__main__":
    main()
