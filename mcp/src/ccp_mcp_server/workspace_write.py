# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Explicit container creation through private files and the pinned Rust client."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from typing import Any, ClassVar

from fastmcp.exceptions import ToolError, ValidationError
from fastmcp.server.middleware import CallNext, Middleware, MiddlewareContext

from .workspace_runtime import WorkspaceRuntime
from .workspace_values import canonical_uuid, object_result

MAX_REQUEST = 64 * 1024


class WorkspaceValidationMiddleware(Middleware):
    async def on_call_tool(
        self, context: MiddlewareContext[Any], call_next: CallNext[Any, Any]
    ) -> Any:
        try:
            validate_creation_arguments(context.message.name, context.message.arguments)
            return await call_next(context)
        except (ValueError, ValidationError):
            # FastMCP 2 raises Pydantic's ValueError subclass; 4 wraps it.
            # The outer callable's error formatting can ignore nested config.
            raise ToolError(
                "invalid workspace tool arguments; check the schema and original targets"
            ) from None


def bounded_text(value: str, limit: int) -> None:
    try:
        if len(value) > limit or len(value.encode("utf-8")) > limit:
            raise ValueError("bound")
    except (ValueError, UnicodeError):
        raise ToolError("workspace request text is invalid or exceeds its byte limit") from None


def node_input(workspace: str, name: str, description: str) -> None:
    canonical_uuid(workspace)
    bounded_text(name, MAX_REQUEST)
    bounded_text(description, 4096)
    normalized = name.strip()
    bounded_text(normalized, 256)
    if not normalized or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in normalized):
        raise ToolError("workspace node names must be nonempty without control characters")


def encode(payload: dict[str, Any]) -> bytes:
    try:
        data = json.dumps(
            payload, ensure_ascii=False, allow_nan=False, separators=(",", ":")
        ).encode("utf-8")
    except (ValueError, TypeError, UnicodeError):
        raise ToolError("invalid workspace creation request") from None
    if len(data) > MAX_REQUEST:
        raise ToolError("encoded workspace creation request exceeds 64 KiB")
    return data


class StrictRequest:
    # Standard dataclasses retain this documented Pydantic configuration when
    # FastMCP validates them. Avoid echoing input values in validation messages.
    __pydantic_config__: ClassVar[dict[str, Any]] = {
        "extra": "forbid",
        "hide_input_in_errors": True,
    }


@dataclass(frozen=True)
class ShelfDraft(StrictRequest):
    workspace_id: str
    name: str
    description: str = ""

    def payload(self) -> bytes:
        node_input(self.workspace_id, self.name, self.description)
        return encode(asdict(self))


@dataclass(frozen=True)
class BookDraft(StrictRequest):
    workspace_id: str
    shelf_id: str
    name: str
    description: str = ""

    def payload(self) -> bytes:
        canonical_uuid(self.shelf_id)
        node_input(self.workspace_id, self.name, self.description)
        return encode(asdict(self))


def validate_creation_arguments(name: str, arguments: dict[str, Any] | None) -> None:
    if name not in {"workspace_create_shelf", "workspace_create_book"}:
        return
    if not isinstance(arguments, dict) or set(arguments) != {"profile", "request"}:
        raise ToolError("creation requires a profile alias and explicit request")
    request = arguments["request"]
    required = {"workspace_id", "name"}
    if name == "workspace_create_book":
        required.add("shelf_id")
    if (
        not isinstance(arguments["profile"], str)
        or not isinstance(request, dict)
        or not required <= set(request) <= required | {"description"}
        or not all(isinstance(value, str) for value in request.values())
    ):
        raise ToolError("invalid creation request fields or types")
    # Reject before older SDK versions can log validation errors with input values.
    node_input(request["workspace_id"], request["name"], request.get("description", ""))
    if name == "workspace_create_book":
        canonical_uuid(request["shelf_id"])
    encode(request)


class WorkspaceWriteTools:
    def __init__(self, runtime: WorkspaceRuntime):
        self.runtime = runtime

    async def _create(
        self, profile: str, action: str, request: ShelfDraft | BookDraft
    ) -> dict[str, Any]:
        self.runtime.profile(profile)
        payload = request.payload()
        try:
            value = object_result(await self.runtime.submit(profile, action, payload))
        except ToolError:
            raise ToolError(
                "creation outcome unconfirmed; inspect shelves/books in the original explicit workspace and parent before retrying"
            ) from None
        return {"workspace_id": request.workspace_id, "data": value}

    async def workspace_create_shelf(self, profile: str, request: ShelfDraft) -> dict[str, Any]:
        """Create one shelf at an explicit UUID; no receipt/retry. Inspect shelves after uncertainty."""
        return await self._create(profile, "create-shelf", request)

    async def workspace_create_book(self, profile: str, request: BookDraft) -> dict[str, Any]:
        """Create one book at explicit workspace/parent UUIDs; inspect books after uncertainty."""
        return await self._create(profile, "create-book", request)
