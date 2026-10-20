# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Known guidance context and captured operation scopes, without cached prose."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from typing import Any

from fastmcp.exceptions import ToolError

from .workspace_guidance import check, guidance, identity
from .workspace_values import object_result


def encoded(value: Any) -> str:
    return json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":"))


def stamp(board: dict[str, Any]) -> str:
    return hashlib.sha256(encoded(board).encode("ascii")).hexdigest()


def delivery(value: Any, workspace: str | None) -> dict[str, Any]:
    row = object_result(value)
    check(type(row.get("format")) is int and row["format"] == 1)
    raw = object_result(row.get("context"))
    context: dict[str, Any] = {
        key: identity(raw.get(key)) for key in ("instance_id", "client_id", "credential_id")
    }
    grant = raw.get("workspace_grant_id")
    check((grant is None) == (workspace is None))
    context["workspace_grant_id"] = None if workspace is None else identity(grant)
    return {"context": context, "guidance": guidance(row.get("guidance"), workspace)}


def targets(name: str, result: dict[str, Any]) -> list[str]:
    if name in {"workspace_selection", "workspace_select"}:
        values = result.get("workspace_ids")
    elif name == "workspace_resolve":
        rows = result.get("targets")
        if not isinstance(rows, list) or len(rows) > 100:
            raise ToolError("invalid workspace guidance target list")
        values = [object_result(object_result(row).get("workspace")).get("id") for row in rows]
    elif name == "workspace_target":
        values = [object_result(result.get("workspace")).get("id")]
    elif name in {"workspace_profile", "workspace_list", "workspace_entry_pending"}:
        values = []
    else:
        values = [result.get("workspace_id")]
    if not isinstance(values, list) or len(values) > 100:
        raise ToolError("invalid workspace guidance target list")
    checked = [identity(value) for value in values]
    check(len(set(checked)) == len(checked))
    return checked


@dataclass(frozen=True)
class DeliveryTarget:
    profile: str
    handle: str | None
    workspaces: list[str]


def caller_handle(arguments: dict[str, Any]) -> str | None:
    scope = arguments.get("scope")
    value = scope.get("selection_id") if isinstance(scope, dict) else arguments.get("selection_id")
    return value if isinstance(value, str) else None


def source_rows(reads: list[dict[str, Any]]) -> list[dict[str, Any]]:
    first = reads[0]
    context = first["context"]
    global_board = first["guidance"]["global"]
    global_stamp = stamp(global_board)
    rows = [{"context": {**context, "workspace_grant_id": None}, "board": global_board}]
    for read in reads:
        current = read["context"]
        check(
            all(
                current[key] == context[key]
                for key in ("instance_id", "client_id", "credential_id")
            )
        )
        check(stamp(read["guidance"]["global"]) == global_stamp)
        local = read["guidance"]["workspace"]
        if local is not None:
            rows.append({"context": current, "board": local})
    return rows
