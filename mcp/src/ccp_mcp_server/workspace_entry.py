# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Durable entry writes and body-free controls bound to the exact saved intent."""

from __future__ import annotations

from dataclasses import asdict
from typing import Any

from fastmcp.exceptions import ToolError

from .workspace_entry_request import (
    TARGET_FIELDS,
    CreateChange,
    EntryIntent,
    OperationTarget,
    identity,
    shape,
    validate_target,
)
from .workspace_runtime import WorkspaceRuntime
from .workspace_values import object_result


def operation_info(value: Any) -> dict[str, Any] | None:
    if value is None:
        return None
    info = shape(value, TARGET_FIELDS | {"instance_id", "client_id", "state"})
    validate_target({key: info[key] for key in TARGET_FIELDS})
    identity(info["instance_id"])
    identity(info["client_id"])
    if not isinstance(info["state"], str) or info["state"] not in {
        "pending",
        "committed",
        "cancelled",
        "conflict",
    }:
        raise ToolError("invalid saved entry operation state")
    return info


def target_of(info: dict[str, Any] | None) -> dict[str, Any] | None:
    return {key: info[key] for key in TARGET_FIELDS} if info is not None else None


def receipt(value: Any, expected: EntryIntent | OperationTarget) -> dict[str, Any]:
    data = shape(value, {"entry_id", "revision", "operation"})
    identity(data["entry_id"])
    if type(data["revision"]) is not int or not 1 <= data["revision"] <= 2**63 - 1:
        raise ToolError("invalid entry receipt revision")
    info = operation_info(data["operation"])
    if info is None or info["state"] != "committed":
        raise ToolError("missing committed entry binding")
    if (
        info["workspace_id"] != expected.workspace_id
        or info["operation_id"] != expected.operation_id
    ):
        raise ToolError("entry receipt differs from the intended operation")
    if isinstance(expected, OperationTarget):
        if target_of(info) != asdict(expected):
            raise ToolError("entry receipt differs from the saved binding")
    elif isinstance(expected.change, CreateChange):
        if data["revision"] != 1:
            raise ToolError("invalid created entry revision")
    elif (
        data["entry_id"] != expected.change.entry_id
        or data["revision"] != expected.change.expected_revision + 1
    ):
        raise ToolError("entry receipt differs from the intended target revision")
    return {"workspace_id": info["workspace_id"], "data": data, "target": target_of(info)}


def resolution(value: Any, expected: OperationTarget) -> dict[str, Any]:
    data = shape(value, TARGET_FIELDS | {"outcome"})
    if {key: data[key] for key in TARGET_FIELDS} != asdict(expected):
        raise ToolError("entry resolution differs from the saved binding")
    outcome = object_result(data["outcome"])
    if outcome.get("state") == "committed":
        shape(outcome, {"state", "entry_id", "revision"})
        identity(outcome["entry_id"])
        if type(outcome["revision"]) is not int or not 1 <= outcome["revision"] <= 2**63 - 1:
            raise ToolError("invalid resolved entry revision")
    elif outcome.get("state") == "cancelled":
        shape(outcome, {"state"})
    else:
        raise ToolError("invalid entry resolution state")
    return {"workspace_id": expected.workspace_id, "data": data, "target": asdict(expected)}


class WorkspaceEntryTools:
    def __init__(self, runtime: WorkspaceRuntime):
        self.runtime = runtime

    async def workspace_mutate_entry(self, profile: str, request: EntryIntent) -> dict[str, Any]:
        """Save/send one explicit five-action intent. Keep its UUID/revision; never invent a retry."""
        self.runtime.profile(profile)
        payload = request.payload()
        try:
            value = await self.runtime.submit(profile, "mutate", payload)
            return receipt(value, request)
        except ToolError:
            raise ToolError(
                "entry outcome unconfirmed; inspect pending/profile metadata and resume or resolve the original target"
            ) from None

    async def workspace_entry_pending(self, profile: str) -> dict[str, Any]:
        """Inspect body-free pending/last metadata and a ready-to-copy target; no authority proof."""
        value = object_result(await self.runtime.run(profile, "info"))
        if not {"pending_content", "last_content"} <= set(value):
            raise ToolError("configured workspace client lacks entry-operation metadata")
        pending = operation_info(value["pending_content"])
        last = operation_info(value["last_content"])
        if (pending is not None and pending["state"] != "pending") or (
            last is not None and last["state"] == "pending"
        ):
            raise ToolError("inconsistent saved entry operation metadata")
        for info in [pending, last]:
            if info is not None and (
                info["instance_id"] != value.get("instance_id")
                or info["client_id"] != value.get("client_id")
            ):
                raise ToolError("entry operation metadata differs from the profile identity")
        return {"pending": pending, "last": last, "target": target_of(pending or last)}

    async def workspace_entry_resume(self, profile: str, target: OperationTarget) -> dict[str, Any]:
        """Resume only the expected saved binding under the profile lease; never retarget a retry."""
        arguments = target.arguments()
        self.runtime.profile(profile)
        try:
            value = await self.runtime.control(profile, "resume", arguments)
            return receipt(value, target)
        except ToolError:
            raise ToolError(
                "entry resume not confirmed; preserve the original target and inspect pending/profile metadata"
            ) from None

    async def workspace_entry_resolve(
        self, profile: str, target: OperationTarget, via_profile: str | None = None
    ) -> dict[str, Any]:
        """Resolve/cancel the expected original binding, optionally through a same-client recovery alias."""
        arguments = target.arguments()
        self.runtime.profile(profile)
        if via_profile is not None:
            arguments += ("--via-profile", str(self.runtime.profile(via_profile)))
        try:
            value = await self.runtime.control(profile, "resolve", arguments)
            return resolution(value, target)
        except ToolError:
            raise ToolError(
                "entry resolution not confirmed; keep the original target and use a currently authorized same-client profile"
            ) from None
