# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Bounded UTF-8 payloads and strict SDK shapes for private workspace requests."""

import json
from typing import Any, ClassVar

from fastmcp.exceptions import ToolError


def bounded_text(value: str, limit: int) -> None:
    try:
        if len(value) > limit or len(value.encode("utf-8")) > limit:
            raise ValueError("bound")
    except (ValueError, UnicodeError):
        raise ToolError("workspace request text is invalid or exceeds its byte limit") from None


def encode_request(payload: dict[str, Any], limit: int) -> bytes:
    try:
        data = json.dumps(
            payload, ensure_ascii=False, allow_nan=False, separators=(",", ":")
        ).encode("utf-8")
    except (ValueError, TypeError, UnicodeError):
        raise ToolError("invalid workspace JSON request") from None
    if len(data) > limit:
        raise ToolError("encoded workspace request exceeds its input budget")
    return data


class StrictRequest:
    __pydantic_config__: ClassVar[dict[str, Any]] = {
        "extra": "forbid",
        "hide_input_in_errors": True,
    }
