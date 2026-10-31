# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Post-success, bounded automatic guidance in model-visible MCP text content."""

from __future__ import annotations

import asyncio
from collections import OrderedDict
from typing import Any

from fastmcp.exceptions import ToolError
from fastmcp.server.middleware import CallNext, Middleware, MiddlewareContext
from mcp.types import TextContent

from .workspace_delivery_values import (
    DeliveryTarget,
    caller_handle,
    delivery,
    encoded,
    source_rows,
    stamp,
    targets,
)
from .workspace_guidance import check
from .workspace_runtime import WorkspaceRuntime
from .workspace_values import object_result

_LOCAL = {"workspace_profiles", "workspace_selection_open", "workspace_guidance"}
_WARNING = (
    "Current CCP guidance is unavailable or incomplete; previously returned guidance may be stale. "
    "The primary operation result is unchanged. Use workspace_guidance to refresh explicitly; "
    "do not retry a mutation to refresh guidance."
)


class WorkspaceDeliveryMiddleware(Middleware):
    def __init__(self, runtime: WorkspaceRuntime, *, timeout: float = 10, budget: int = 256 * 1024):
        self.runtime = runtime
        self.timeout = timeout
        self.budget = budget
        self._locks = {runtime.profile(alias): asyncio.Lock() for alias in runtime.aliases()}
        self._stamps: OrderedDict[tuple[str, ...], str] = OrderedDict()

    async def on_call_tool(
        self, context: MiddlewareContext[Any], call_next: CallNext[Any, Any]
    ) -> Any:
        # Primary errors/cancellation propagate. Guidance must never retry or
        # turn a successfully committed primary operation into a tool error.
        result = await call_next(context)
        name = context.message.name
        if name in _LOCAL or getattr(result, "is_error", False):
            return result
        arguments = context.message.arguments or {}
        target = DeliveryTarget(str(arguments.get("profile", "")), None, [])
        try:
            profile = arguments.get("via_profile") if name == "workspace_entry_resolve" else None
            target = DeliveryTarget(
                profile or target.profile,
                caller_handle(arguments),
                targets(name, object_result(result.structured_content)),
            )
            return await asyncio.wait_for(self._deliver(target, result), self.timeout)
        except (ToolError, ValueError, asyncio.TimeoutError):
            payload = self._envelope(target, "unavailable", [])
            payload["warning"] = _WARNING
            text = TextContent(type="text", text=encoded({"_ccp_guidance": payload}))
        result.content = [*result.content, text]
        return result

    async def _deliver(self, target: DeliveryTarget, result: Any) -> Any:
        path = self.runtime.profile(target.profile)
        async with self._locks[path]:
            text, updates = await self._prepare(target)
            result.content = [*result.content, text]
            self._remember(updates)
            return result

    def _remember(self, updates: list[tuple[tuple[str, ...], str]]) -> None:
        # No await between SDK handoff preparation and stamps. This does not
        # attest network receipt, model consumption or compliance with prose.
        for key, value in updates:
            self._stamps[key] = value
            self._stamps.move_to_end(key)
            if len(self._stamps) > 4096:
                self._stamps.popitem(last=False)

    def _envelope(
        self, target: DeliveryTarget, status: str, boards: list[dict[str, Any]]
    ) -> dict[str, Any]:
        return {
            "format": 1,
            "status": status,
            "precedence": "global_first",
            "workspace_ids": target.workspaces,
            "boards": boards,
        }

    async def _prepare(
        self, target: DeliveryTarget
    ) -> tuple[TextContent, list[tuple[tuple[str, ...], str]]]:
        reads = []
        scopes: list[str | None] = list(target.workspaces) or [None]
        for workspace in scopes:
            args = () if workspace is None else ("--workspace-id", workspace)
            value = await self.runtime.run(target.profile, "guidance", "--delivery-context", *args)
            reads.append(delivery(value, workspace))
        rows = source_rows(reads)
        boards, updates = self._changes(target, rows)
        payload = self._envelope(target, "current", boards)
        text = encoded({"_ccp_guidance": payload})
        check(len(text.encode("ascii")) <= self.budget)
        return TextContent(type="text", text=text), updates

    def _changes(
        self, target: DeliveryTarget, rows: list[dict[str, Any]]
    ) -> tuple[list[dict[str, Any]], list[tuple[tuple[str, ...], str]]]:
        boards: list[dict[str, Any]] = []
        updates: list[tuple[tuple[str, ...], str]] = []
        for row in rows:
            context, board = row["context"], row["board"]
            key = (
                target.handle or "",
                target.profile,
                context["instance_id"],
                context["client_id"],
                context["credential_id"],
                board["scope"].get("workspace_id", ""),
                context["workspace_grant_id"] or "",
            )
            value = stamp(board)
            known = self.runtime.known_selection(target.handle)
            if not known or self._stamps.get(key) != value:
                boards.append(row)
            if known:
                updates.append((key, value))
        return boards, updates
