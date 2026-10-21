# SPDX-License-Identifier: AGPL-3.0-or-later
"""Explicit manager commands; no remote administrator can enable local updates."""

from __future__ import annotations

import argparse

from . import managed_policy as policy
from . import managed_update as updater
from . import managed_worker as worker
from .managed_store import Store


def add(commands: argparse._SubParsersAction[argparse.ArgumentParser]) -> None:
    commands.add_parser("policy-show")
    setup = commands.add_parser("policy-set")
    setup.add_argument("--config", required=True)
    setup.add_argument("--channel", choices=("stable", "nightly"), required=True)
    setup.add_argument("--pin-tag")
    setup.add_argument("--automatic", choices=("on", "off"), required=True)
    setup.add_argument("--expected-revision", type=int, required=True)
    disable = commands.add_parser("policy-disable")
    disable.add_argument("--expected-revision", type=int, required=True)
    commands.add_parser("update")
    commands.add_parser("update-status")
    due = commands.add_parser("update-due")
    due.add_argument("--expected-policy-revision", type=int, required=True)
    due.add_argument("--attempt", required=True)


def run(store: Store, args: argparse.Namespace) -> object:
    if args.command == "policy-show":
        return policy.show(store)
    if args.command == "policy-set":
        proposed = policy.Policy(
            0, args.automatic == "on", args.channel, args.pin_tag, args.config, "", ""
        )
        return policy.configure(store, proposed, args.expected_revision).value()
    if args.command == "policy-disable":
        return policy.disable(store, args.expected_revision).value()
    if args.command == "update":
        return updater.update(store)
    if args.command == "update-status":
        return worker.status(store)
    if args.command == "update-due":
        return worker.run(store, args.expected_policy_revision, args.attempt)
    raise ValueError("Unknown managed command")
