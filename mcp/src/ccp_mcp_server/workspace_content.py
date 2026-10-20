# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Explicit workspace content reads through the host-selected Rust client."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from fastmcp.exceptions import ToolError

from .workspace_runtime import WorkspaceRuntime
from .workspace_values import canonical_uuid, object_result


@dataclass(frozen=True)
class ContentScope:
    workspace_id: str | None = None
    selection_id: str | None = None

    async def resolve(self, runtime: WorkspaceRuntime, profile: str) -> str:
        runtime.profile(profile)
        if self.workspace_id is not None:
            canonical_uuid(self.workspace_id)
            return self.workspace_id
        if self.selection_id is None:
            raise ToolError("content reads require an explicit workspace or selection handle")
        target = object_result(await runtime.selection(self.selection_id, profile, "target"))
        workspace = object_result(target.get("workspace")).get("id")
        if not isinstance(workspace, str):
            raise ToolError("workspace client returned an invalid content target")
        canonical_uuid(workspace)
        return workspace


def revision(value: int) -> None:
    if type(value) is not int or not 1 <= value <= 2**63 - 1:
        raise ToolError("content revisions must be positive signed 64-bit integers")


def page(after: str | None, limit: int) -> tuple[str, ...]:
    if type(limit) is not int or not 1 <= limit <= 32:
        raise ToolError("content page limits must be between 1 and 32")
    if after is not None:
        canonical_uuid(after)
    return ("--limit", str(limit), *(("--after", after) if after is not None else ()))


@dataclass(frozen=True)
class ContentPage:
    after: str | None = None
    limit: int = 32

    def arguments(self) -> tuple[str, ...]:
        return page(self.after, self.limit)


@dataclass(frozen=True)
class BookPage:
    shelf_id: str
    after: str | None = None
    limit: int = 32

    def arguments(self) -> tuple[str, ...]:
        canonical_uuid(self.shelf_id)
        return ("--shelf-id", self.shelf_id, *page(self.after, self.limit))


@dataclass(frozen=True)
class EntryPage:
    book_id: str
    archived: bool = False
    after: str | None = None
    limit: int = 32

    def arguments(self) -> tuple[str, ...]:
        canonical_uuid(self.book_id)
        if type(self.archived) is not bool:
            raise ToolError("archived must be a boolean")
        return (
            "--book-id",
            self.book_id,
            *page(self.after, self.limit),
            *(("--archived",) if self.archived else ()),
        )


@dataclass(frozen=True)
class EntryRead:
    entry_id: str
    revision: int | None = None

    def arguments(self) -> tuple[str, ...]:
        canonical_uuid(self.entry_id)
        if self.revision is not None:
            revision(self.revision)
        return (
            "--entry-id",
            self.entry_id,
            *(("--revision", str(self.revision)) if self.revision is not None else ()),
        )


@dataclass(frozen=True)
class HistoryRead:
    entry_id: str
    after: int | None = None
    limit: int = 32

    def arguments(self) -> tuple[str, ...]:
        canonical_uuid(self.entry_id)
        page(None, self.limit)
        if self.after is not None:
            revision(self.after)
        return (
            "--entry-id",
            self.entry_id,
            "--limit",
            str(self.limit),
            *(("--after", str(self.after)) if self.after is not None else ()),
        )


class WorkspaceContentTools:
    def __init__(self, runtime: WorkspaceRuntime):
        self.runtime = runtime

    async def _read(
        self, profile: str, scope: ContentScope, action: str, arguments: tuple[str, ...]
    ) -> dict[str, Any]:
        workspace = await scope.resolve(self.runtime, profile)
        payload = await self.runtime.content(profile, workspace, action, arguments)
        return {"workspace_id": workspace, "data": object_result(payload)}

    async def workspace_shelves(
        self, profile: str, scope: ContentScope, query: ContentPage | None = None
    ) -> dict[str, Any]:
        """Read shelves in an explicit workspace or handle default; return workspace_id and data."""
        return await self._read(profile, scope, "shelves", (query or ContentPage()).arguments())

    async def workspace_books(
        self, profile: str, scope: ContentScope, query: BookPage
    ) -> dict[str, Any]:
        """Read books under an explicit shelf, labeled with the resolved workspace_id."""
        return await self._read(profile, scope, "books", query.arguments())

    async def workspace_shelf(
        self, profile: str, scope: ContentScope, node_id: str
    ) -> dict[str, Any]:
        """Read one shelf's metadata; explicit targets never change selection preferences."""
        canonical_uuid(node_id)
        return await self._read(profile, scope, "shelf", ("--node-id", node_id))

    async def workspace_book(
        self, profile: str, scope: ContentScope, node_id: str
    ) -> dict[str, Any]:
        """Read one book's metadata with current profile access and explicit scope."""
        canonical_uuid(node_id)
        return await self._read(profile, scope, "book", ("--node-id", node_id))

    async def workspace_entries(
        self, profile: str, scope: ContentScope, query: EntryPage
    ) -> dict[str, Any]:
        """List active or archived entry summaries, without archived bodies or history."""
        return await self._read(profile, scope, "entries", query.arguments())

    async def workspace_entry(
        self, profile: str, scope: ContentScope, query: EntryRead
    ) -> dict[str, Any]:
        """Read exact current/historical entry content and attribution; archived entries refuse."""
        return await self._read(profile, scope, "entry", query.arguments())

    async def workspace_history(
        self, profile: str, scope: ContentScope, query: HistoryRead
    ) -> dict[str, Any]:
        """Read ordered entry revision metadata without bodies, labeled with workspace_id."""
        return await self._read(profile, scope, "history", query.arguments())
