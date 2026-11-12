# SPDX-License-Identifier: AGPL-3.0-or-later
"""Explicit local update scope and a sticky source floor, separate from profile state."""

from __future__ import annotations

from dataclasses import asdict, dataclass, replace
from pathlib import Path

from . import managed_fs as fs
from . import managed_preflight as preflight
from . import managed_release as release
from . import release_contract as contract
from .managed_store import Generation, Store

NAME = "update-policy.json"
MARKER = "update-policy-created.json"


@dataclass(frozen=True)
class Policy:
    revision: int
    automatic: bool
    channel: str
    pin_tag: str | None
    config: str
    config_sha256: str
    source_floor: str
    pin_source: str | None = None

    def value(self) -> dict[str, object]:
        return {"format": 1, **asdict(self)}


def present(root: Path) -> bool:
    return any((root / name).exists() or (root / name).is_symlink() for name in [NAME, MARKER])


def load(root: Path) -> Policy:
    marker = contract.fields(fs.load(root / MARKER), {"format"})
    contract.require(
        type(marker["format"]) is int and marker["format"] == 1, "Invalid update policy marker"
    )
    row = contract.fields(
        fs.load(root / NAME),
        {
            "format",
            "revision",
            "automatic",
            "channel",
            "pin_tag",
            "config",
            "config_sha256",
            "source_floor",
            "pin_source",
        },
    )
    contract.require(
        type(row["format"]) is int and row["format"] == 1 and type(row["automatic"]) is bool,
        "Invalid update policy",
    )
    config = contract.text(row["config"], r"[^\x00-\x1f]{1,4096}")
    contract.require(Path(config).is_absolute(), "Update config must be absolute")
    channel = contract.text(row["channel"], "stable|nightly")
    pin = None if row["pin_tag"] is None else release.tag(row["pin_tag"], channel)
    pinned = None if row["pin_source"] is None else contract.text(row["pin_source"], "[0-9a-f]{40}")
    contract.require(pin is not None or pinned is None, "Invalid pinned source")
    return Policy(
        contract.number(row["revision"], 2**63 - 2),
        row["automatic"] is True,
        channel,
        pin,
        config,
        contract.text(row["config_sha256"], "[0-9a-f]{64}"),
        contract.text(row["source_floor"], "[0-9a-f]{40}"),
        pinned,
    )


def show(store: Store) -> dict[str, object]:
    with fs.locked(store.root, shared=True):
        return (
            load(store.root).value()
            if present(store.root)
            else {"configured": False, "automatic": False}
        )


def configure(store: Store, proposed: Policy, expected: int) -> Policy:
    contract.require(type(expected) is int and 0 <= expected < 2**63 - 2, "Invalid policy revision")
    contract.require(type(proposed.automatic) is bool, "Invalid automatic opt-in")
    with fs.locked(store.root):
        pointer = store.pointer()
        contract.require(
            pointer.current is not None, "Activate a verified bundle before configuring updates"
        )
        generation = store.generation(pointer.current or "")
        old = load(store.root) if present(store.root) else None
        contract.require((old.revision if old else 0) == expected, "Update policy changed")
        channel = contract.text(proposed.channel, "stable|nightly")
        pin = None if proposed.pin_tag is None else release.tag(proposed.pin_tag, channel)
        config = contract.text(proposed.config, r"[^\x00-\x1f]{1,4096}")
        hashed, _profiles = preflight.config(Path(config), generation.path / "client")
        result = replace(
            proposed,
            revision=expected + 1,
            channel=channel,
            pin_tag=pin,
            config=config,
            config_sha256=hashed,
            source_floor=old.source_floor if old else generation.manifest.source_sha,
            pin_source=pinned_source(old, generation, channel, pin),
        )
        if old is None:
            fs.atomic(store.root / MARKER, {"format": 1})
        fs.atomic(store.root / NAME, result.value())
        return result


def pinned_source(
    old: Policy | None, generation: Generation, channel: str, pin: str | None
) -> str | None:
    if pin is None:
        return None
    if old and old.channel == channel and old.pin_tag == pin and old.pin_source is not None:
        return old.pin_source
    if channel == generation.channel and pin == generation.tag:
        return generation.manifest.source_sha
    return None


def disable(store: Store, expected: int) -> Policy:
    with fs.locked(store.root):
        old = load(store.root)
        contract.require(
            type(expected) is int and old.revision == expected, "Update policy changed"
        )
        result = replace(old, revision=contract.number(expected + 1, 2**63 - 2), automatic=False)
        fs.atomic(store.root / NAME, result.value())
        return result
