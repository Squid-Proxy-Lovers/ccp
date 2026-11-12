# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Typed entry intents and body-free expected bindings, with pre-SDK checks."""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any, Literal
from uuid import UUID

from fastmcp.exceptions import ToolError

from .workspace_payload import StrictRequest, bounded_text, encode_request
from .workspace_values import canonical_uuid

MAX_REQUEST = 2 * 1024 * 1024
TARGET_FIELDS = {"workspace_id", "operation_id", "credential_id", "request_hash"}


@dataclass(frozen=True)
class EntryBody(StrictRequest):
    text: str
    description: str = ""
    labels: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class CreateChange(StrictRequest):
    action: Literal["create"]
    book_id: str
    name: str
    content: EntryBody


@dataclass(frozen=True)
class AppendChange(StrictRequest):
    action: Literal["append"]
    entry_id: str
    expected_revision: int
    text: str


@dataclass(frozen=True)
class ReviseChange(StrictRequest):
    action: Literal["revise"]
    entry_id: str
    expected_revision: int
    content: EntryBody


@dataclass(frozen=True)
class ArchiveChange(StrictRequest):
    action: Literal["archive"]
    entry_id: str
    expected_revision: int


@dataclass(frozen=True)
class RestoreChange(StrictRequest):
    action: Literal["restore"]
    entry_id: str
    expected_revision: int


@dataclass(frozen=True)
class EntryIntent(StrictRequest):
    workspace_id: str
    operation_id: str
    change: CreateChange | AppendChange | ReviseChange | ArchiveChange | RestoreChange
    agent: str | None = None

    def payload(self) -> bytes:
        value = {
            "workspace_id": self.workspace_id,
            "operation_id": self.operation_id,
            "change": asdict(self.change),
            "agent": self.agent,
        }
        validate_intent(value)
        return encode_request(value, MAX_REQUEST)


@dataclass(frozen=True)
class OperationTarget(StrictRequest):
    workspace_id: str
    operation_id: str
    credential_id: str
    request_hash: str

    def arguments(self) -> tuple[str, ...]:
        value = {
            "workspace_id": self.workspace_id,
            "operation_id": self.operation_id,
            "credential_id": self.credential_id,
            "request_hash": self.request_hash,
        }
        validate_target(value)
        return tuple(part for key in value for part in ("--" + key.replace("_", "-"), value[key]))


def shape(value: Any, required: set[str], optional: set[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, dict) or not required <= set(value) <= required | (optional or set()):
        raise ToolError("invalid entry request fields")
    return value


def text(value: Any, limit: int, *, label: bool = False) -> None:
    if not isinstance(value, str):
        raise ToolError("invalid entry text type")
    bounded_text(value, limit)
    if label and (not value or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value)):
        raise ToolError("entry labels must be nonempty without control characters")


def identity(value: Any, *, operation: bool = False) -> None:
    if not isinstance(value, str):
        raise ToolError("invalid entry identity type")
    canonical_uuid(value)
    if operation and UUID(value).int == 0:
        raise ToolError("entry operation UUID cannot be nil")


def validate_target(value: Any) -> None:
    target = shape(value, TARGET_FIELDS)
    for name in ["workspace_id", "operation_id", "credential_id"]:
        identity(target[name], operation=name == "operation_id")
    digest = target["request_hash"]
    if (
        not isinstance(digest, str)
        or len(digest) != 64
        or any(c not in "0123456789abcdef" for c in digest)
    ):
        raise ToolError("invalid original entry request hash")


def content(value: Any) -> None:
    body = shape(value, {"text"}, {"description", "labels"})
    text(body["text"], 256 * 1024)
    text(body.get("description", ""), 4096)
    labels = body.get("labels", [])
    if not isinstance(labels, list) or len(labels) > 32:
        raise ToolError("invalid entry label count")
    for label in labels:
        text(label, 128, label=True)
    if len(set(labels)) != len(labels):
        raise ToolError("duplicate entry labels")


def change(value: Any) -> None:
    if not isinstance(value, dict) or not isinstance(value.get("action"), str):
        raise ToolError("invalid entry action")
    action = value["action"]
    fields = {
        "create": {"action", "book_id", "name", "content"},
        "append": {"action", "entry_id", "expected_revision", "text"},
        "revise": {"action", "entry_id", "expected_revision", "content"},
        "archive": {"action", "entry_id", "expected_revision"},
        "restore": {"action", "entry_id", "expected_revision"},
    }
    if action not in fields:
        raise ToolError("unknown entry action")
    shape(value, fields[action])
    if action == "create":
        identity(value["book_id"])
        text(value["name"], MAX_REQUEST)
        text(value["name"].strip(), 256, label=True)
    else:
        identity(value["entry_id"])
        revision = value["expected_revision"]
        if type(revision) is not int or not 1 <= revision < 2**63 - 1:
            raise ToolError("invalid expected entry revision")
    if action in {"create", "revise"}:
        content(value["content"])
    if action == "append":
        text(value["text"], 256 * 1024)
        if not value["text"]:
            raise ToolError("entry append must be nonempty")


def validate_intent(value: Any) -> None:
    request = shape(value, {"workspace_id", "operation_id", "change"}, {"agent"})
    identity(request["workspace_id"])
    identity(request["operation_id"], operation=True)
    if request.get("agent") is not None:
        text(request["agent"], 256, label=True)
    change(request["change"])
    encode_request(request, MAX_REQUEST)


def validate_entry_arguments(name: str, arguments: dict[str, Any] | None) -> None:
    names = {
        "workspace_mutate_entry",
        "workspace_entry_pending",
        "workspace_entry_resume",
        "workspace_entry_resolve",
    }
    if name not in names:
        return
    required = {"profile"}
    if name == "workspace_mutate_entry":
        required.add("request")
    elif name != "workspace_entry_pending":
        required.add("target")
    args = shape(
        arguments, required, {"via_profile"} if name == "workspace_entry_resolve" else set()
    )
    if not isinstance(args["profile"], str) or (
        args.get("via_profile") is not None and not isinstance(args["via_profile"], str)
    ):
        raise ToolError("invalid workspace profile alias type")
    if name == "workspace_mutate_entry":
        validate_intent(args["request"])
    elif name != "workspace_entry_pending":
        validate_target(args["target"])
