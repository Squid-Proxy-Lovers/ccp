# SPDX-License-Identifier: AGPL-3.0-or-later
"""Initial verified acquisition; never replace an active store or configure a host."""

from __future__ import annotations

import tempfile
from pathlib import Path

from . import managed_fs as fs
from . import managed_policy as policy
from . import managed_release as release
from . import release_contract as contract
from .managed_store import Generation, Pointer, Store


def empty(store: Store, captured: Pointer) -> None:
    contract.require(
        captured == Pointer() and store.pointer() == captured,
        "Initial install requires an empty store; inspect status or use update",
    )
    contract.require(not policy.present(store.root), "Existing update policy must be preserved")


def install(store: Store, channel: str, tag: str | None, expected: int) -> dict[str, object]:
    contract.require(
        type(expected) is int and expected == 0, "Initial install requires revision zero"
    )
    contract.text(channel, "stable|nightly")
    if tag is not None:
        release.tag(tag, channel)
    with fs.locked(store.root):
        contract.require(not policy.present(store.root), "Existing update policy must be preserved")
        store._initialize()
        captured = store.pointer()
        empty(store, captured)
    with fs.locked(store.root / "install-job"):
        with fs.locked(store.root, shared=True):
            empty(store, captured)
        selected = release.discover(channel, tag)
        with tempfile.TemporaryDirectory(
            dir=store.root / "install-job", prefix="download-"
        ) as temporary:
            manifest = release.download(Path(temporary), channel, selected)
            generation = store.stage(Path(temporary), channel, selected)
        contract.require(
            generation.manifest == manifest, "Initial candidate changed after verification"
        )
        return select(store, captured, generation)


def select(store: Store, captured: Pointer, generation: Generation) -> dict[str, object]:
    with fs.locked(store.root):
        empty(store, captured)
        contract.require(
            store.generation(generation.generation_id) == generation,
            "Initial candidate changed before activation",
        )
        pointer = store._activate(generation.generation_id, captured.revision)
        return {
            "result": "installed",
            "generation": generation.summary(),
            "pointer": pointer.value(),
            "update_policy_created": False,
        }
