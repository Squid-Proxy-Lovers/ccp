# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Runtime-owned selection handles and directories, separate from MCP request state."""

from __future__ import annotations

import asyncio
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from types import TracebackType
from typing import Any
from uuid import uuid4

from fastmcp.exceptions import ToolError

from .workspace_config import WorkspaceConfig
from .workspace_process import WorkspaceProcess


@dataclass
class CallerSelection:
    directory: Path
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    initialized: bool = False


class WorkspaceRuntime:
    def __init__(self, config: WorkspaceConfig):
        self._temporary = tempfile.TemporaryDirectory(prefix="ccp-workspace-mcp-")
        self.root = Path(self._temporary.name)
        self._profiles = dict(config.profiles)
        self.process = WorkspaceProcess(config.binary)
        self._handles: dict[str, str] = {}
        self._selections: dict[tuple[str, str], CallerSelection] = {}

    def __enter__(self) -> WorkspaceRuntime:
        return self

    def __exit__(
        self,
        _kind: type[BaseException] | None,
        _error: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self._temporary.cleanup()

    def aliases(self) -> list[str]:
        return sorted(self._profiles)

    def profile(self, alias: str) -> Path:
        path = self._profiles.get(alias)
        if path is None:
            raise ToolError("unknown workspace profile alias; list the configured profiles")
        return path

    async def run(self, alias: str, *arguments: str) -> Any:
        return await self.process.run(*arguments, "--profile", str(self.profile(alias)))

    async def content(
        self, alias: str, workspace: str, action: str, arguments: tuple[str, ...] = ()
    ) -> Any:
        return await self.process.run(
            "content",
            "--profile",
            str(self.profile(alias)),
            action,
            "--workspace-id",
            workspace,
            *arguments,
        )

    async def explicit_target(self, alias: str, workspace: str) -> Any:
        return await self.process.run(
            "selection",
            "--profile",
            str(self.profile(alias)),
            "target",
            "--workspace-id",
            workspace,
        )

    def open_selection(self) -> str:
        if len(self._handles) >= 64:
            raise ToolError("workspace selection budget is exhausted; restart the bridge")
        handle = str(uuid4())
        self._handles[handle] = uuid4().hex
        return handle

    def _scope(self, handle: str, alias: str) -> CallerSelection:
        self.profile(alias)
        if handle not in self._handles:
            raise ToolError("unknown selection handle; open a new selection explicitly")
        key = (handle, alias)
        if key not in self._selections:
            directory = self.root / self._handles[handle] / alias
            self._selections[key] = CallerSelection(directory)
        return self._selections[key]

    async def selection(
        self, handle: str, alias: str, action: str, arguments: tuple[str, ...] = ()
    ) -> Any:
        scope = self._scope(handle, alias)
        prefix = ("selection", "--profile", str(self.profile(alias)))
        async with scope.lock:
            if not scope.initialized:
                await self.process.run(*prefix, "init", "--selection-dir", str(scope.directory))
                scope.initialized = True
            return await self.process.run(
                *prefix, action, "--selection-dir", str(scope.directory), *arguments
            )
