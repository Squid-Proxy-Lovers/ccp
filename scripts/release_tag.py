#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Validate a version tag against source before enabling either publisher."""

from __future__ import annotations

import argparse
from pathlib import Path

import release_contract as contract
from ccp_mcp_server.release_contract import tag_channel


def classify(root: Path, tag: str) -> dict[str, str]:
    channel = tag_channel(tag)
    contract.require(tag == "v" + contract.source_version(root), "Tag and source version differ")
    return {
        "tag": tag,
        "channel": channel,
        "prerelease": "true" if channel == "rc" else "false",
        # Empty input preserves the pinned action's existing GitHub API default.
        "make_latest": "false" if channel == "rc" else "",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    values = classify(args.root, args.tag)
    with args.output.open("a") as output:
        for key, value in values.items():
            output.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
