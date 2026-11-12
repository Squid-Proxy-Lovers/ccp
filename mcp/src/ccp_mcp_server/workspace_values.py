# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Shared validation for public workspace tool inputs and CLI JSON shapes."""

from typing import Any
from uuid import UUID

from fastmcp.exceptions import ToolError


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
