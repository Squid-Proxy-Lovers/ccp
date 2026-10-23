# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Explicit guidance retrieval with structural provenance and bounded text."""

from __future__ import annotations

import re
from typing import Any

from fastmcp.exceptions import ToolError

from .workspace_content import ContentScope
from .workspace_runtime import WorkspaceRuntime
from .workspace_values import canonical_uuid, object_result

_MAX = 2**63 - 1
_NIL = "00000000-0000-0000-0000-000000000000"


def check(valid: bool) -> None:
    if not valid:
        raise ToolError("invalid or mismatched workspace guidance response")


def identity(value: Any) -> str:
    check(isinstance(value, str) and value != _NIL)
    canonical_uuid(value)
    return str(value)


def publisher(value: Any) -> dict[str, Any]:
    row = object_result(value)
    actor = identity(row.get("actor_id"))
    principal = identity(row.get("principal_id"))
    kind = row.get("actor_kind")
    recorded = row.get("recorded_at")
    check(kind in ("admin", "key"))
    check(type(recorded) is int and 0 <= recorded <= _MAX)
    check(kind != "admin" or actor == principal)
    return {
        "actor_kind": kind,
        "actor_id": actor,
        "principal_id": principal,
        "recorded_at": recorded,
    }


def board(value: Any, workspace: str | None) -> dict[str, Any]:
    row = object_result(value)
    scope = (
        {"kind": "global"}
        if workspace is None
        else {"kind": "workspace", "workspace_id": workspace}
    )
    check(row.get("scope") == scope)
    revision, text, author = row.get("revision"), row.get("text"), row.get("publisher")
    check(type(revision) is int and 0 <= revision <= _MAX)
    if not isinstance(text, str):
        raise ToolError("invalid workspace guidance text")
    try:
        check(len(text.encode("utf-8")) <= 8192)
    except UnicodeError:
        raise ToolError("invalid workspace guidance text encoding") from None
    if revision == 0:
        check(text == "" and author is None)
        author = None
    else:
        author = publisher(author)
    return {"scope": scope, "revision": revision, "text": text, "publisher": author}


def guidance(value: Any, workspace: str | None) -> dict[str, Any]:
    row = object_result(value)
    check(row.get("precedence") == "global_first")
    global_board = board(row.get("global"), None)
    local = row.get("workspace")
    if workspace is None:
        check(local is None)
    else:
        identity(workspace)
        local = board(local, workspace)
    return {"precedence": "global_first", "global": global_board, "workspace": local}


def validate_guidance_arguments(name: str, arguments: Any) -> None:
    if name != "workspace_guidance":
        return
    if not isinstance(arguments, dict) or set(arguments) - {"profile", "scope"}:
        raise ToolError("invalid workspace guidance arguments")
    alias = arguments.get("profile")
    if not isinstance(alias, str) or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", alias) is None:
        raise ToolError("guidance requires a configured profile alias")
    scope = arguments.get("scope")
    if scope is None:
        return
    if not isinstance(scope, dict) or set(scope) - {"workspace_id", "selection_id"}:
        raise ToolError("invalid workspace guidance scope")
    if not any(scope.get(key) is not None for key in ("workspace_id", "selection_id")):
        raise ToolError("guidance scope requires a workspace or selection handle")
    for key in ("workspace_id", "selection_id"):
        if scope.get(key) is not None:
            identity(scope[key])


class WorkspaceGuidanceTools:
    def __init__(self, runtime: WorkspaceRuntime):
        self.runtime = runtime

    async def workspace_guidance(
        self, profile: str, scope: ContentScope | None = None
    ) -> dict[str, Any]:
        """Read labeled CCP guidance; omission means global only, without changing choices.

        Global CCP guidance wins workspace conflicts; host instructions and actual
        server permissions remain authoritative. Publishing is a separate admin action.
        """
        self.runtime.profile(profile)
        workspace = None if scope is None else await scope.resolve(self.runtime, profile)
        args = () if workspace is None else ("--workspace-id", workspace)
        return guidance(await self.runtime.run(profile, "guidance", *args), workspace)
