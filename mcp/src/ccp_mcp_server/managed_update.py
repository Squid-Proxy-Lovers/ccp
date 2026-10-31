# SPDX-License-Identifier: AGPL-3.0-or-later
"""Guarded official updates; software selection never migrates a workspace profile."""

from __future__ import annotations

import tempfile
from dataclasses import replace
from pathlib import Path

from . import managed_fs as fs
from . import managed_policy as policy
from . import managed_preflight as preflight
from . import managed_release as release
from . import release_contract as contract
from .managed_store import Generation, Pointer, Store


def capture(store: Store, expected: int | None) -> tuple[policy.Policy, Pointer, Generation]:
    with fs.locked(store.root, shared=True):
        current = policy.load(store.root)
        if expected is not None:
            contract.require(
                current.automatic and current.revision == expected,
                "Automatic update permission changed",
            )
        pointer = store.pointer()
        contract.require(pointer.current is not None, "Activate a verified bundle first")
        return current, pointer, store.generation(pointer.current or "")


def unchanged(store: Store, captured: policy.Policy, automatic: bool) -> None:
    contract.require(
        policy.load(store.root) == captured and (not automatic or captured.automatic),
        "Update policy changed",
    )


def select(
    store: Store, captured: policy.Policy, pointer: Pointer, generation: Generation, automatic: bool
) -> dict[str, object]:
    with fs.locked(store.root):
        unchanged(store, captured, automatic)
        contract.require(store.pointer() == pointer, "Active software changed during update")
        checked = store.generation(generation.generation_id)
        contract.require(checked == generation, "Candidate changed after compatibility checks")
        hashed, _profiles = preflight.config(Path(captured.config), generation.path / "client")
        contract.require(hashed == captured.config_sha256, "Update configuration changed")
        advanced = replace(
            captured,
            revision=contract.number(captured.revision + 1, 2**63 - 2),
            source_floor=generation.manifest.source_sha,
            pin_source=generation.manifest.source_sha if captured.pin_tag else None,
        )
        # Commit the antirollback floor first. Never lower it after an uncertain pointer write.
        fs.atomic(store.root / policy.NAME, advanced.value())
        selected = store._activate(generation.generation_id, pointer.revision)
        return {
            "result": "updated",
            "generation": generation.generation_id,
            "policy_revision": advanced.revision,
            "pointer": selected.value(),
        }


def update(store: Store, expected: int | None = None) -> dict[str, object]:
    captured, pointer, current = capture(store, expected)
    with fs.locked(store.root / "update-job"):
        return perform(store, captured, pointer, current, expected is not None)


def perform(
    store: Store, captured: policy.Policy, pointer: Pointer, current: Generation, automatic: bool
) -> dict[str, object]:
    hashed, profiles = preflight.config(Path(captured.config), current.path / "client")
    contract.require(hashed == captured.config_sha256, "Update scope changed; configure explicitly")
    selected = release.discover(captured.channel, captured.pin_tag)
    with tempfile.TemporaryDirectory(
        dir=store.root / "update-job", prefix="download-"
    ) as temporary:
        downloaded = release.download(Path(temporary), captured.channel, selected)
        release.ancestry(captured.source_floor, downloaded.source_sha)
        release.ancestry(current.manifest.source_sha, downloaded.source_sha)
        contract.require(
            captured.pin_source is None or captured.pin_source == downloaded.source_sha,
            "Pinned release source changed; explicit policy review required",
        )
        with fs.locked(store.root, shared=True):
            unchanged(store, captured, automatic)
            contract.require(store.pointer() == pointer, "Active software changed during discovery")
        if downloaded.source_sha == current.manifest.source_sha:
            return {
                "result": "up_to_date",
                "policy_revision": captured.revision,
                "generation": current.generation_id,
            }
        generation = store.stage(Path(temporary), captured.channel, selected)
    contract.require(
        generation.manifest == downloaded, "Staged candidate differs from verified source"
    )
    reports = preflight.inspect(generation, profiles)
    result = select(store, captured, pointer, generation, automatic)
    return {**result, "profiles": reports}
