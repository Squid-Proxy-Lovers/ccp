# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Workspace discovery and caller selection, served only over local stdio."""

from __future__ import annotations

import sys
from dataclasses import dataclass
from typing import Any
from uuid import UUID

from fastmcp import FastMCP
from fastmcp.exceptions import ToolError

from .workspace_config import environment_config
from .workspace_runtime import WorkspaceRuntime


@dataclass(frozen=True)
class WorkspaceChoice:
    workspace_ids: list[str]
    expected_revision: int
    default_workspace_id: str | None = None

    def arguments(self) -> tuple[str, ...]:
        if len(self.workspace_ids) > 100 or not 0 <= self.expected_revision <= 2**63 - 1:
            raise ToolError("select at most 100 workspaces with a valid expected revision")
        for value in self.workspace_ids:
            canonical_uuid(value)
        if len(set(self.workspace_ids)) != len(self.workspace_ids):
            raise ToolError("duplicate workspace selection")
        if (
            self.default_workspace_id is not None
            and self.default_workspace_id not in self.workspace_ids
        ):
            raise ToolError("the default workspace must be explicitly selected")
        args = ["--expected-revision", str(self.expected_revision)]
        for value in self.workspace_ids:
            args.extend(["--workspace-id", value])
        if self.default_workspace_id is not None:
            args.extend(["--default-workspace-id", self.default_workspace_id])
        return tuple(args)


def canonical_uuid(value: str) -> None:
    try:
        if str(UUID(value)) != value:
            raise ValueError("noncanonical")
    except (ValueError, AttributeError):
        raise ToolError("workspace targets must be canonical UUIDs") from None


def object_result(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ToolError("workspace client returned an unexpected response shape")
    return value


class WorkspaceTools:
    def __init__(self, runtime: WorkspaceRuntime):
        self.runtime = runtime

    async def workspace_profiles(self) -> list[str]:
        """List host-configured aliases, without exposing local profile paths."""
        return self.runtime.aliases()

    async def workspace_profile(self, profile: str) -> dict[str, Any]:
        """Inspect public pinned profile metadata; this does not prove current access."""
        return object_result(await self.runtime.run(profile, "info"))

    async def workspace_list(
        self, profile: str, after: str | None = None, limit: int = 100
    ) -> dict[str, Any]:
        """List currently accessible workspace metadata without selecting it."""
        if not 1 <= limit <= 500 or (after is not None and (len(after) > 1024 or "\x00" in after)):
            raise ToolError("workspace page bounds are invalid")
        arguments: tuple[str, ...] = ("workspaces", "--limit", str(limit))
        if after is not None:
            arguments += (f"--after={after}",)
        return object_result(await self.runtime.run(profile, *arguments))

    async def workspace_selection_open(self) -> dict[str, str]:
        """Open an empty selection handle; retain its ID for later calls or deliberate sharing."""
        return {"selection_id": self.runtime.open_selection()}

    async def workspace_selection(self, profile: str, selection_id: str) -> dict[str, Any]:
        """Inspect this handle's choices and revision, initially empty for each profile alias."""
        return object_result(await self.runtime.selection(selection_id, profile, "show"))

    async def workspace_select(
        self, profile: str, choice: WorkspaceChoice, selection_id: str
    ) -> dict[str, Any]:
        """Replace this handle's selected set/default using its expected revision."""
        return object_result(
            await self.runtime.selection(selection_id, profile, "set", choice.arguments())
        )

    async def workspace_resolve(self, profile: str, selection_id: str) -> dict[str, Any]:
        """Resolve every selected target's current metadata, or fail without partial results."""
        targets = await self.runtime.selection(selection_id, profile, "resolve")
        if not isinstance(targets, list):
            raise ToolError("workspace client returned an unexpected target list")
        return {"targets": targets}

    async def workspace_target(
        self, profile: str, workspace_id: str | None = None, selection_id: str | None = None
    ) -> dict[str, Any]:
        """Resolve one explicit UUID or a selection handle's default, without changing choices."""
        if workspace_id is not None:
            canonical_uuid(workspace_id)
            return object_result(await self.runtime.explicit_target(profile, workspace_id))
        if selection_id is None:
            raise ToolError("an explicit workspace UUID or selection handle is required")
        return object_result(await self.runtime.selection(selection_id, profile, "target"))


def build_server(runtime: WorkspaceRuntime) -> FastMCP:
    mcp = FastMCP(
        "ccp-workspace",
        instructions="Use configured profile aliases to discover accessible workspaces. Open a selection handle and retain its selection_id for later choices and resolution. Open separate handles for independent callers; share a handle only deliberately. An explicit target does not change choices. Selection is a preference; the CCP server enforces current access.",
    )
    tools = WorkspaceTools(runtime)
    mcp.tool(tools.workspace_profiles)
    mcp.tool(tools.workspace_profile)
    mcp.tool(tools.workspace_list)
    mcp.tool(tools.workspace_selection_open)
    mcp.tool(tools.workspace_selection)
    mcp.tool(tools.workspace_select)
    mcp.tool(tools.workspace_resolve)
    mcp.tool(tools.workspace_target)
    return mcp


def main() -> None:
    try:
        config = environment_config()
    except (OSError, ValueError):
        print(
            "invalid workspace MCP configuration; check the private config and binary",
            file=sys.stderr,
        )
        raise SystemExit(2) from None
    with WorkspaceRuntime(config) as runtime:
        build_server(runtime).run(transport="stdio")


if __name__ == "__main__":
    main()
