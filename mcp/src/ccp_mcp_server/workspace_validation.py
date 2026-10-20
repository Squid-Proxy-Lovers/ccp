# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Validate sensitive write arguments before SDK parsing and error logging."""

from typing import Any

from fastmcp.exceptions import ToolError, ValidationError
from fastmcp.server.middleware import CallNext, Middleware, MiddlewareContext

from .workspace_entry_request import validate_entry_arguments
from .workspace_guidance import validate_guidance_arguments
from .workspace_write import validate_creation_arguments


class WorkspaceValidationMiddleware(Middleware):
    async def on_call_tool(
        self, context: MiddlewareContext[Any], call_next: CallNext[Any, Any]
    ) -> Any:
        try:
            validate_guidance_arguments(context.message.name, context.message.arguments)
            validate_creation_arguments(context.message.name, context.message.arguments)
            validate_entry_arguments(context.message.name, context.message.arguments)
            return await call_next(context)
        except (ValueError, ValidationError):
            raise ToolError(
                "invalid workspace tool arguments; check the schema and original targets"
            ) from None
