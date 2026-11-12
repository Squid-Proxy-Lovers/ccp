# SPDX-License-Identifier: AGPL-3.0-or-later
"""Daily local opt-in checks, detached after the launcher captures its fixed pair."""

from __future__ import annotations

import subprocess
import sys
import time
from pathlib import Path
from uuid import uuid4

from . import managed_fs as fs
from . import managed_policy as policy
from . import managed_update as updater
from . import release_contract as contract
from .managed_store import Store, identity

NAME = "update-status.json"
INTERVAL = 24 * 60 * 60


def load(root: Path) -> dict[str, object] | None:
    path = root / NAME
    if not path.exists() and not path.is_symlink():
        return None
    row = contract.fields(
        fs.load(path), {"format", "attempt", "checked_at", "policy_revision", "result"}
    )
    contract.require(type(row["format"]) is int and row["format"] == 1, "Invalid update status")
    identity(row["attempt"])
    contract.number(row["checked_at"], 2**63 - 1)
    contract.number(row["policy_revision"], 2**63 - 2)
    contract.text(row["result"], "scheduled|up_to_date|updated|refused|start_failed")
    return row


def status(store: Store) -> object:
    with fs.locked(store.root, shared=True):
        return load(store.root) or {"result": "never_checked"}


def claim(store: Store) -> tuple[int, str] | None:
    with fs.locked(store.root, shared=True):
        if not policy.present(store.root):
            return None
        current = policy.load(store.root)
        if not current.automatic or not due(store.root):
            return None
    with fs.locked(store.root):
        if policy.load(store.root) != current or not due(store.root):
            return None
        now = int(time.time())
        attempt = str(uuid4())
        fs.atomic(
            store.root / NAME,
            {
                "format": 1,
                "attempt": attempt,
                "checked_at": now,
                "policy_revision": current.revision,
                "result": "scheduled",
            },
        )
        return current.revision, attempt


def due(root: Path) -> bool:
    old = load(root)
    now = int(time.time())
    contract.number(now, 2**63 - 1)
    return old is None or now >= contract.number(old["checked_at"], 2**63 - 1) + INTERVAL


def finish(store: Store, attempt: str, result: str, revision: int) -> None:
    with fs.locked(store.root):
        old = load(store.root)
        if old is not None and old["attempt"] == attempt and old["result"] == "scheduled":
            fs.atomic(store.root / NAME, {**old, "result": result, "policy_revision": revision})


def schedule(store: Store) -> None:
    claimed: tuple[int, str] | None = None
    try:
        claimed = claim(store)
        if claimed is None:
            return
        revision, attempt = claimed
        # Fixed bootstrap Python/module; no downloaded code, profile paths or shell argv.
        subprocess.Popen(  # noqa: S603
            [
                sys.executable,
                "-I",
                "-m",
                "ccp_mcp_server.managed",
                "--store",
                str(store.root),
                "update-due",
                "--expected-policy-revision",
                str(revision),
                "--attempt",
                attempt,
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
            start_new_session=True,
        )
    except (OSError, ValueError, KeyError):
        # Optional scheduling must never prevent execution of the already captured pair.
        if claimed is not None:
            try:
                finish(store, claimed[1], "start_failed", claimed[0])
            except (OSError, ValueError, KeyError):
                return


def run(store: Store, revision: int, attempt: str) -> dict[str, object]:
    identity(attempt)
    try:
        scheduled(store, revision, attempt)
        # Keep a separate process lease through completion, without blocking store readers.
        with fs.locked(store.root / "automatic-job"):
            scheduled(store, revision, attempt)
            return complete(store, revision, attempt)
    except (OSError, ValueError, KeyError):
        return {"result": "refused", "policy_revision": revision}


def scheduled(store: Store, revision: int, attempt: str) -> None:
    contract.number(revision, 2**63 - 2)
    with fs.locked(store.root, shared=True):
        old = load(store.root)
        contract.require(
            old is not None
            and old["attempt"] == attempt
            and old["policy_revision"] == revision
            and old["result"] == "scheduled",
            "Stale automatic attempt",
        )


def complete(store: Store, revision: int, attempt: str) -> dict[str, object]:
    result: dict[str, object] = {"result": "refused", "policy_revision": revision}
    try:
        result = updater.update(store, revision)
        return result
    finally:
        finish(
            store,
            attempt,
            str(result["result"]),
            contract.number(result["policy_revision"], 2**63 - 2),
        )
