# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Bounded asynchronous execution of the host-selected Rust workspace client."""

from __future__ import annotations

import asyncio
import contextlib
import json
from pathlib import Path
from typing import Any

from fastmcp.exceptions import ToolError


class OutputLimitError(RuntimeError):
    """A child exceeded its public JSON output budget."""


async def stop_child(process: asyncio.subprocess.Process) -> None:
    if process.returncode is None:
        with contextlib.suppress(ProcessLookupError):
            process.kill()
    await process.wait()


class WorkspaceProcess:
    def __init__(self, binary: Path, *, timeout: float = 60, output_limit: int = 1024 * 1024):
        self.binary = binary
        self.timeout = timeout
        self.output_limit = output_limit
        self.slots = asyncio.Semaphore(8)

    async def run(self, *arguments: str) -> Any:
        try:
            await asyncio.wait_for(self.slots.acquire(), timeout=2)
        except asyncio.TimeoutError:
            raise ToolError("workspace client is busy; retry the request") from None
        try:
            return await self._spawn(arguments)
        finally:
            self.slots.release()

    async def _spawn(self, arguments: tuple[str, ...]) -> Any:
        try:
            process = await self._launch(arguments)
        except (OSError, ValueError):
            raise ToolError("unable to start the configured workspace client") from None
        try:
            output = await asyncio.wait_for(self._read(process), timeout=self.timeout)
        except asyncio.CancelledError:
            await stop_child(process)
            raise
        except (asyncio.TimeoutError, OutputLimitError, OSError):
            await stop_child(process)
            raise ToolError("workspace client timed out or exceeded its output budget") from None
        if process.returncode != 0:
            raise ToolError(
                "workspace request refused; inspect selection or use the CLI for details"
            )
        try:
            return json.loads(output)
        except (ValueError, UnicodeError):
            raise ToolError("workspace client returned invalid JSON") from None

    async def _launch(self, arguments: tuple[str, ...]) -> asyncio.subprocess.Process:
        # Only the host config selects the executable; validated values stay in
        # separate argv entries, with no shell or inherited stdin. Shield launch
        # so cancellation cannot lose a child created before its handle arrives.
        launch = asyncio.create_task(
            asyncio.create_subprocess_exec(
                str(self.binary),
                "workspace",
                *arguments,
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.DEVNULL,
            )
        )
        try:
            return await asyncio.shield(launch)
        except asyncio.CancelledError:
            try:
                process = await launch
            except (OSError, ValueError):
                raise asyncio.CancelledError from None
            await stop_child(process)
            raise

    async def _read(self, process: asyncio.subprocess.Process) -> bytes:
        if process.stdout is None:
            raise OutputLimitError("missing stdout")
        chunks: list[bytes] = []
        size = 0
        while chunk := await process.stdout.read(64 * 1024):
            size += len(chunk)
            if size > self.output_limit:
                raise OutputLimitError("output budget exceeded")
            chunks.append(chunk)
        await process.wait()
        return b"".join(chunks)
