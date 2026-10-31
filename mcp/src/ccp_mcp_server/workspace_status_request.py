# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Explicit status requests and private pending-operation targets."""

from __future__ import annotations

from dataclasses import asdict, dataclass
from typing import Any
from uuid import UUID

from fastmcp.exceptions import ToolError

from .workspace_payload import StrictRequest, bounded_text, encode_request
from .workspace_values import canonical_uuid

TARGET_FIELDS = {"workspace_id", "status_id", "operation_id", "credential_id", "request_hash"}
HEADER_FIELDS = {"workspace_id", "status_id", "operation_id", "expected_revision"}


def shape(value: Any, required: set[str], optional: set[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, dict) or not required <= set(value) <= required | (optional or set()):
        raise ToolError("invalid status fields")
    return value


def identifier(value: Any) -> None:
    if not isinstance(value, str):
        raise ToolError("invalid status identity type")
    canonical_uuid(value)
    if UUID(value).int == 0:
        raise ToolError("status identities must be non-nil UUIDs")


def integer(value: Any, minimum: int = 0, maximum: int = 2**63 - 1) -> None:
    if type(value) is not int or not minimum <= value <= maximum:
        raise ToolError("invalid status integer bounds")


def text(value: Any, maximum: int) -> None:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise ToolError("invalid status text")
    bounded_text(value, maximum)
    # Blankness belongs to the Rust client/server; Python strip has a different set.


def headers(value: dict[str, Any]) -> None:
    for name in ["workspace_id", "status_id", "operation_id"]:
        identifier(value.get(name))
    integer(value.get("expected_revision"), maximum=2**63 - 2)


def validate_target(value: Any) -> None:
    target = shape(value, TARGET_FIELDS)
    for name in TARGET_FIELDS - {"request_hash"}:
        identifier(target[name])
    digest = target["request_hash"]
    if (
        not isinstance(digest, str)
        or len(digest) != 64
        or any(c not in "0123456789abcdef" for c in digest)
    ):
        raise ToolError("invalid status fingerprint")


@dataclass(frozen=True)
class StatusSet(StrictRequest):
    workspace_id: str
    status_id: str
    operation_id: str
    expected_revision: int
    message: str
    agent: str | None = None

    def payload(self) -> bytes:
        value = asdict(self)
        headers(value)
        text(self.message, 4096)
        if self.agent is not None:
            text(self.agent, 128)
        body = {
            key: value[key]
            for key in ("workspace_id", "status_id", "operation_id", "expected_revision")
        }
        body["change"] = {"action": "set", "agent": self.agent, "message": self.message}
        return encode_request(body, 64 * 1024)


@dataclass(frozen=True)
class StatusClear(StrictRequest):
    workspace_id: str
    status_id: str
    operation_id: str
    expected_revision: int

    def payload(self) -> bytes:
        value = asdict(self)
        headers(value)
        return encode_request({**value, "change": {"action": "clear"}}, 64 * 1024)


@dataclass(frozen=True)
class StatusTarget(StrictRequest):
    workspace_id: str
    status_id: str
    operation_id: str
    credential_id: str
    request_hash: str

    def arguments(self) -> tuple[str, ...]:
        value = asdict(self)
        validate_target(value)
        return tuple(
            part for key, item in value.items() for part in ("--" + key.replace("_", "-"), item)
        )


def validate_status_arguments(name: str, arguments: dict[str, Any] | None) -> None:
    if name in {"workspace_update_status", "workspace_clear_status"}:
        outer = shape(arguments, {"profile", "request"})
        required = HEADER_FIELDS | ({"message"} if name == "workspace_update_status" else set())
        value = shape(
            outer["request"], required, {"agent"} if name == "workspace_update_status" else set()
        )
        headers(value)
        if name == "workspace_update_status":
            text(value["message"], 4096)
            if value.get("agent") is not None:
                text(value["agent"], 128)
        encode_request(value, 64 * 1024)
    elif name in {"workspace_status_resume", "workspace_status_forget"}:
        fields = {"profile", "target"} | (
            {"confirm_local_forget"} if name.endswith("forget") else set()
        )
        outer = shape(arguments, fields)
        validate_target(outer["target"])
        if name.endswith("forget") and outer["confirm_local_forget"] is not True:
            raise ToolError(
                "explicit local-forget confirmation is required; this does not cancel the server request"
            )
    elif name == "workspace_status_pending":
        outer = shape(arguments, {"profile"})
    else:
        return
    if not isinstance(outer["profile"], str):
        raise ToolError("status tools require a configured profile alias")
