# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Explicit status reads, durable client-owned updates and local-only forgetting."""

from __future__ import annotations

from dataclasses import asdict
from typing import Any

from fastmcp.exceptions import ToolError

from .workspace_content import ContentPage, ContentScope
from .workspace_runtime import WorkspaceRuntime
from .workspace_status_request import (
    TARGET_FIELDS,
    StatusClear,
    StatusSet,
    StatusTarget,
    identifier,
    shape,
)
from .workspace_status_values import check, mutation_result, operation_info, page, status
from .workspace_values import object_result


class WorkspaceStatusTools:
    def __init__(self, runtime: WorkspaceRuntime):
        self.runtime = runtime

    async def workspace_status_messages(
        self, profile: str, scope: ContentScope, query: ContentPage | None = None
    ) -> dict[str, Any]:
        """Read one bounded status page. Stale text remains; age is not liveness or completion."""
        workspace = await scope.resolve(self.runtime, profile)
        request = query or ContentPage()
        arguments = request.arguments()
        data = await self.runtime.status(profile, "list", ("--workspace-id", workspace, *arguments))
        return {
            "workspace_id": workspace,
            "data": page(data, workspace, request.after, request.limit),
        }

    async def workspace_status_message(
        self, profile: str, scope: ContentScope, status_id: str
    ) -> dict[str, Any]:
        """Read the explicit slot; a null-message tombstone means deliberately cleared, not stale."""
        identifier(status_id)
        workspace = await scope.resolve(self.runtime, profile)
        data = await self.runtime.status(
            profile, "get", ("--workspace-id", workspace, "--status-id", status_id)
        )
        value = status(data, workspace, True)
        check(value["status_id"] == status_id)
        return {"workspace_id": workspace, "data": value}

    async def _mutate(self, profile: str, request: StatusSet | StatusClear) -> dict[str, Any]:
        self.runtime.profile(profile)
        payload = request.payload()
        try:
            return mutation_result(await self.runtime.status_submit(profile, payload), request)
        except ToolError:
            raise ToolError(
                "status outcome unconfirmed; inspect body-free pending metadata and retain the original request; no automatic retry"
            ) from None

    async def workspace_update_status(self, profile: str, request: StatusSet) -> dict[str, Any]:
        """Save/send one explicit client-owned status UUID/CAS intent; retain its original operation."""
        return await self._mutate(profile, request)

    async def workspace_clear_status(self, profile: str, request: StatusClear) -> dict[str, Any]:
        """Explicitly clear an owned report at its current revision; staleness never clears it."""
        return await self._mutate(profile, request)

    async def workspace_status_pending(self, profile: str) -> dict[str, Any]:
        """Inspect body-free pending/last local metadata, not current authority or cancellation proof."""
        row = object_result(await self.runtime.run(profile, "info"))
        if not {"pending_status", "last_status"} <= set(row):
            raise ToolError("configured client lacks status operation metadata")
        pending, last = operation_info(row["pending_status"]), operation_info(row["last_status"])
        for info in [pending, last]:
            if info is not None:
                check(
                    info["instance_id"] == row.get("instance_id")
                    and info["client_id"] == row.get("client_id")
                )
        check(
            pending is None
            or pending["state"] == "pending"
            and pending["credential_id"] == row.get("credential_id")
        )
        check(last is None or last["state"] != "pending")
        return {
            "pending": pending,
            "last": last,
            "target": None if pending is None else {key: pending[key] for key in TARGET_FIELDS},
        }

    async def workspace_status_resume(self, profile: str, target: StatusTarget) -> dict[str, Any]:
        """Deliberately resend only the exact saved request in the original credential context."""
        try:
            value = await self.runtime.status(profile, "resume", target.arguments())
            return mutation_result(value, target)
        except ToolError:
            raise ToolError(
                "status resume unconfirmed; preserve the original target; no replacement UUID, CAS or credential was chosen"
            ) from None

    async def workspace_status_forget(
        self, profile: str, target: StatusTarget, confirm_local_forget: bool
    ) -> dict[str, Any]:
        """Forget locally only. The server request is NOT cancelled and may still complete."""
        if confirm_local_forget is not True:
            raise ToolError("explicit local-forget confirmation is required")
        row = shape(
            await self.runtime.status(
                profile, "forget", (*target.arguments(), "--confirm-local-forget")
            ),
            {"locally_forgotten", "cancelled", "operation"},
        )
        info = operation_info(row["operation"])
        check(row["locally_forgotten"] is True and row["cancelled"] is False)
        if info is None:
            raise ToolError("missing locally forgotten status operation metadata")
        check(
            info["state"] == "locally_forgotten"
            and {key: info[key] for key in TARGET_FIELDS} == asdict(target)
        )
        return {"workspace_id": target.workspace_id, "data": row, "target": asdict(target)}
