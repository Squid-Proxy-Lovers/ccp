# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Body-free status proofs and scoped current observations, never authority leases."""

from __future__ import annotations

from dataclasses import asdict
from typing import Any

from fastmcp.exceptions import ToolError

from .workspace_status_request import (
    TARGET_FIELDS,
    StatusClear,
    StatusSet,
    StatusTarget,
    identifier,
    integer,
    shape,
    text,
    validate_target,
)

RECEIPT_FIELDS = {
    "workspace_id",
    "status_id",
    "operation_id",
    "client_id",
    "revision",
    "cleared",
    "updated_at",
}
INFO_FIELDS = TARGET_FIELDS | {
    "instance_id",
    "client_id",
    "expected_revision",
    "action",
    "state",
    "receipt",
}


def check(value: bool) -> None:
    if not value:
        raise ToolError("status metadata differs from its original target")


def receipt(value: Any) -> dict[str, Any]:
    row = shape(value, RECEIPT_FIELDS)
    for field in ["workspace_id", "status_id", "operation_id", "client_id"]:
        identifier(row[field])
    integer(row["revision"], 1)
    integer(row["updated_at"])
    check(type(row["cleared"]) is bool)
    return row


def operation_info(value: Any) -> dict[str, Any] | None:
    if value is None:
        return None
    info = shape(value, INFO_FIELDS)
    validate_target({key: info[key] for key in TARGET_FIELDS})
    identifier(info["instance_id"])
    identifier(info["client_id"])
    integer(info["expected_revision"], maximum=2**63 - 2)
    check(
        isinstance(info["action"], str)
        and info["action"] in {"set", "clear"}
        and isinstance(info["state"], str)
        and info["state"] in {"pending", "committed", "locally_forgotten"}
    )
    check((info["receipt"] is not None) == (info["state"] == "committed"))
    if info["receipt"] is not None:
        proof = receipt(info["receipt"])
        check(
            all(
                proof[key] == info[key]
                for key in ["workspace_id", "status_id", "operation_id", "client_id"]
            )
        )
        check(
            proof["revision"] == info["expected_revision"] + 1
            and proof["cleared"] == (info["action"] == "clear")
        )
    return info


def mutation_result(value: Any, expected: StatusSet | StatusClear | StatusTarget) -> dict[str, Any]:
    row = shape(value, RECEIPT_FIELDS | {"operation"})
    proof = receipt({key: row[key] for key in RECEIPT_FIELDS})
    info = operation_info(row["operation"])
    if info is None:
        raise ToolError("missing committed status operation metadata")
    check(info["state"] == "committed" and info["receipt"] == proof)
    check(
        all(
            row[key] == getattr(expected, key)
            for key in ["workspace_id", "status_id", "operation_id"]
        )
    )
    if isinstance(expected, StatusTarget):
        check({key: info[key] for key in TARGET_FIELDS} == asdict(expected))
    else:
        check(
            proof["revision"] == expected.expected_revision + 1
            and proof["cleared"] == isinstance(expected, StatusClear)
        )
    return {
        "workspace_id": proof["workspace_id"],
        "data": row,
        "target": {key: info[key] for key in TARGET_FIELDS},
    }


def status(value: Any, workspace: str, allow_cleared: bool) -> dict[str, Any]:
    row = shape(
        value,
        {
            "workspace_id",
            "status_id",
            "client_id",
            "client_name",
            "client_disabled",
            "revision",
            "agent",
            "message",
            "updated_at",
            "stale",
        },
    )
    for field in ["workspace_id", "status_id", "client_id"]:
        identifier(row[field])
    check(
        row["workspace_id"] == workspace
        and isinstance(row["client_name"], str)
        and bool(row["client_name"])
    )
    integer(row["revision"], 1)
    integer(row["updated_at"])
    check(type(row["stale"]) is bool and type(row["client_disabled"]) is bool)
    if row["message"] is None:
        check(allow_cleared and row["agent"] is None and row["stale"])
    else:
        text(row["message"], 4096)
    if row["agent"] is not None:
        text(row["agent"], 128)
    return row


def page(value: Any, workspace: str, after: str | None, limit: int) -> dict[str, Any]:
    row = shape(value, {"items", "next_cursor", "observed_at", "stale_after_seconds"})
    integer(row["observed_at"])
    integer(row["stale_after_seconds"], 1)
    check(isinstance(row["items"], list) and len(row["items"]) <= limit)
    previous = after
    for item in row["items"]:
        item = status(item, workspace, False)
        check(previous is None or previous < item["status_id"])
        previous = item["status_id"]
        age = row["observed_at"] - item["updated_at"]
        check(item["stale"] == (age < 0 or age >= row["stale_after_seconds"]))
    check(row["next_cursor"] is None or bool(row["items"]) and row["next_cursor"] == previous)
    return row
