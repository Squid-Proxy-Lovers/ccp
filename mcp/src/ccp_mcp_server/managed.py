# SPDX-License-Identifier: AGPL-3.0-or-later
"""Stage and activate verified client/MCP bundles."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from .managed_store import Store


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--store", type=Path, required=True)
    commands = result.add_subparsers(dest="command", required=True)
    stage = commands.add_parser("stage")
    stage.add_argument("--directory", type=Path, required=True)
    stage.add_argument("--channel", choices=("stable", "nightly"), required=True)
    stage.add_argument("--tag", required=True)
    activate = commands.add_parser("activate")
    activate.add_argument("--generation", required=True)
    activate.add_argument("--expected-revision", type=int, required=True)
    rollback = commands.add_parser("rollback")
    rollback.add_argument("--expected-revision", type=int, required=True)
    commands.add_parser("status")
    client = commands.add_parser("client")
    client.add_argument("arguments", nargs=argparse.REMAINDER)
    commands.add_parser("mcp")
    return result


def run(args: argparse.Namespace) -> object:
    store = Store(args.store)
    if args.command == "stage":
        return store.stage(args.directory, args.channel, args.tag).summary()
    if args.command == "activate":
        return store.activate(args.generation, args.expected_revision).value()
    if args.command == "rollback":
        return store.rollback(args.expected_revision).value()
    if args.command == "status":
        return store.status()
    arguments = tuple(args.arguments) if args.command == "client" else ()
    if arguments[:1] == ("--",):
        arguments = arguments[1:]
    executable, argv, environment = store.launch(args.command, arguments)
    # Only paths captured from a private ready generation are executed; no shell.
    os.execve(executable, argv, environment)  # noqa: S606
    return None


def main() -> None:
    cli = parser()
    args = cli.parse_args()
    try:
        print(json.dumps(run(args), sort_keys=True))
    except (OSError, ValueError, KeyError):
        cli.exit(
            1,
            "Managed operation unconfirmed or refused; inspect status and retain ready generations. No server or profile rollback was performed.\n",
        )


if __name__ == "__main__":
    main()
