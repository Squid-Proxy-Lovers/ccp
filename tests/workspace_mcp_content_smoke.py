"""Actual workspace content MCP stdio -> CLI -> pinned mTLS read scenarios."""

from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path
from uuid import uuid4

from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from mcp_smoke import data
from workspace_mcp_fixture import WorkspaceFixture


async def call(client, name, **arguments):
    result = await client.call_tool(name, arguments, raise_on_error=False)
    assert not result.is_error, name
    return data(result)


async def refused(client, name, **arguments):
    result = await client.call_tool(name, arguments, raise_on_error=False)
    assert result.is_error, name
    text = "".join(item.text for item in result.content if hasattr(item, "text"))
    assert "PRIVATE_BODY" not in text
    return text


def content_command(fixture, action, payload):
    return fixture.command(
        fixture.client,
        "workspace",
        "content",
        "--profile",
        fixture.profile,
        action,
        "--request-file",
        fixture.secret_file(json.dumps(payload)),
    )


def mutate(fixture, change):
    return content_command(
        fixture,
        "mutate",
        {
            "workspace_id": fixture.setup["workspace_id"],
            "operation_id": str(uuid4()),
            "agent": "fixture agent",
            "change": change,
        },
    )


def populate(fixture):
    workspace = fixture.setup["workspace_id"]
    shelf = content_command(
        fixture,
        "create-shelf",
        {
            "workspace_id": workspace,
            "name": "notes",
            "description": "exact\nmetadata",
        },
    )
    book = content_command(
        fixture,
        "create-book",
        {
            "workspace_id": workspace,
            "shelf_id": shelf["id"],
            "name": "book",
            "description": "book metadata",
        },
    )
    entry = mutate(
        fixture,
        {
            "action": "create",
            "book_id": book["id"],
            "name": "entry",
            "content": {
                "text": "PRIVATE_BODY original",
                "description": "exact description",
                "labels": ["z", "a"],
            },
        },
    )
    mutate(
        fixture,
        {
            "action": "append",
            "entry_id": entry["entry_id"],
            "expected_revision": 1,
            "text": "+contribution",
        },
    )
    return {"shelf": shelf["id"], "book": book["id"], "entry": entry["entry_id"]}


async def browse(client, fixture, ids, scope):
    profile, workspace = "office", fixture.setup["workspace_id"]
    shelves = await call(client, "workspace_shelves", profile=profile, scope=scope)
    assert shelves["workspace_id"] == workspace
    assert shelves["data"]["items"][0]["id"] == ids["shelf"]
    shelf = await call(
        client, "workspace_shelf", profile=profile, scope=scope, node_id=ids["shelf"]
    )
    assert shelf["data"]["description"] == "exact\nmetadata"
    books = await call(
        client, "workspace_books", profile=profile, scope=scope, query={"shelf_id": ids["shelf"]}
    )
    assert books["data"]["items"][0]["id"] == ids["book"]
    book = await call(client, "workspace_book", profile=profile, scope=scope, node_id=ids["book"])
    assert book["data"]["shelf_id"] == ids["shelf"]
    entries = await call(
        client, "workspace_entries", profile=profile, scope=scope, query={"book_id": ids["book"]}
    )
    assert entries["data"]["items"][0]["id"] == ids["entry"]
    entry = await call(
        client, "workspace_entry", profile=profile, scope=scope, query={"entry_id": ids["entry"]}
    )
    assert entry["data"]["content"]["text"] == "PRIVATE_BODY original+contribution"
    assert entry["data"]["entry"]["owner_client_id"] == fixture.client_id
    assert entry["data"]["appended_text"] == "+contribution"
    old = await call(
        client,
        "workspace_entry",
        profile=profile,
        scope=scope,
        query={"entry_id": ids["entry"], "revision": 1},
    )
    assert old["data"]["content"]["text"] == "PRIVATE_BODY original"
    assert old["data"]["entry"]["revision"] == 2 and old["data"]["revision"]["revision"] == 1
    history = await call(
        client,
        "workspace_history",
        profile=profile,
        scope=scope,
        query={"entry_id": ids["entry"], "limit": 1},
    )
    assert history["data"]["items"][0]["action"] == "create"
    assert history["data"]["next_cursor"] == 1 and history["workspace_id"] == workspace
    tail = await call(
        client,
        "workspace_history",
        profile=profile,
        scope=scope,
        query={"entry_id": ids["entry"], "after": 1},
    )
    assert tail["data"]["items"][0]["action"] == "append"
    assert tail["data"]["next_cursor"] is None


async def archives_and_large_snapshot(client, fixture, ids, scope):
    mutate(fixture, {"action": "archive", "entry_id": ids["entry"], "expected_revision": 2})
    query = {"entry_id": ids["entry"]}
    await refused(client, "workspace_entry", profile="office", scope=scope, query=query)
    await refused(
        client, "workspace_entry", profile="office", scope=scope, query={**query, "revision": 1}
    )
    await refused(client, "workspace_history", profile="office", scope=scope, query=query)
    archived = await call(
        client,
        "workspace_entries",
        profile="office",
        scope=scope,
        query={"book_id": ids["book"], "archived": True},
    )
    assert archived["data"]["items"][0]["archived"]
    assert archived["data"]["items"][0]["id"] == ids["entry"]
    assert "PRIVATE_BODY" not in json.dumps(archived)
    mutate(fixture, {"action": "restore", "entry_id": ids["entry"], "expected_revision": 3})
    large = mutate(
        fixture,
        {
            "action": "create",
            "book_id": ids["book"],
            "name": "large",
            "content": {"text": "", "description": "", "labels": []},
        },
    )
    text = "\0" * (256 * 1024)
    mutate(
        fixture,
        {"action": "append", "entry_id": large["entry_id"], "expected_revision": 1, "text": text},
    )
    snapshot = await call(
        client,
        "workspace_entry",
        profile="office",
        scope=scope,
        query={"entry_id": large["entry_id"]},
    )
    assert snapshot["data"]["content"]["text"] == text
    assert snapshot["data"]["appended_text"] == text
    assert len(json.dumps(snapshot)) > 1024 * 1024
    fixture.admin("workspaces/archive", {"workspace_id": fixture.setup["workspace_id"]})
    readable = await call(client, "workspace_entry", profile="office", scope=scope, query=query)
    assert readable["data"]["entry"]["revision"] == 4
    fixture.admin("workspaces/reactivate", {"workspace_id": fixture.setup["workspace_id"]})


async def scope_and_permission_changes(client, fixture, ids, handle):
    first = fixture.setup["workspace_id"]
    scope = {"selection_id": handle}
    independent = (await call(client, "workspace_selection_open"))["selection_id"]
    await refused(
        client, "workspace_shelves", profile="office", scope={"selection_id": independent}
    )
    second = fixture.add_second_workspace()["workspace_id"]
    empty = await call(
        client,
        "workspace_shelves",
        profile="office",
        scope={"workspace_id": second, "selection_id": handle},
    )
    assert empty == {"workspace_id": second, "data": {"items": [], "next_cursor": None}}
    assert (await call(client, "workspace_selection", profile="office", selection_id=handle))[
        "workspace_ids"
    ] == [first]
    assert (await call(client, "workspace_selection", profile="office", selection_id=independent))[
        "workspace_ids"
    ] == []
    await refused(
        client,
        "workspace_books",
        profile="office",
        scope={"workspace_id": second},
        query={"shelf_id": ids["shelf"]},
    )
    fixture.recover_first()
    await refused(client, "workspace_shelves", profile="office", scope=scope)
    recovered = await call(
        client, "workspace_shelves", profile="recovered", scope={"workspace_id": first}
    )
    assert recovered["data"]["items"][0]["id"] == ids["shelf"]
    await refused(client, "workspace_shelves", profile="recovered", scope={"workspace_id": second})
    await refused(client, "workspace_shelves", profile="recovered", scope=scope)
    assert (
        await call(client, "workspace_shelves", profile="office", scope={"workspace_id": second})
    )["data"]["items"] == []
    fixture.admin(
        "clients/disable",
        {"client_id": fixture.client_id},
        fixture.setup["instance_admin_key"]["secret"],
    )
    await refused(
        client,
        "workspace_entry",
        profile="recovered",
        scope={"workspace_id": first},
        query={"entry_id": ids["entry"]},
    )
    await refused(client, "workspace_shelves", profile="office", scope={"workspace_id": second})


async def check():
    with WorkspaceFixture() as fixture:
        ids = populate(fixture)
        config = fixture.root / "content-bridge.json"
        config.write_text(
            json.dumps(
                {
                    "format": 1,
                    "profiles": {
                        "office": str(fixture.profile),
                        "recovered": str(fixture.recovered),
                    },
                }
            )
        )
        config.chmod(0o600)
        scratch = fixture.root / "content-scratch"
        scratch.mkdir(mode=0o700)
        transport = StdioTransport(
            command=sys.executable,
            args=["-m", "ccp_mcp_server.workspace"],
            env={
                "CCP_WORKSPACE_CONFIG": str(config),
                "CCP_CLIENT_BIN": str(fixture.client),
                "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "mcp/src"),
                "TMPDIR": str(scratch),
            },
            keep_alive=False,
        )
        async with Client(transport) as client:
            tools = await client.list_tools()
            assert len(tools) == 21
            names = {tool.name for tool in tools}
            assert {
                "workspace_shelves",
                "workspace_books",
                "workspace_shelf",
                "workspace_book",
                "workspace_entries",
                "workspace_entry",
                "workspace_history",
            } <= names
            handle = (await call(client, "workspace_selection_open"))["selection_id"]
            await call(
                client,
                "workspace_select",
                profile="office",
                selection_id=handle,
                choice={
                    "workspace_ids": [fixture.setup["workspace_id"]],
                    "default_workspace_id": fixture.setup["workspace_id"],
                    "expected_revision": 0,
                },
            )
            scope = {"selection_id": handle}
            await browse(client, fixture, ids, scope)
            await archives_and_large_snapshot(client, fixture, ids, scope)
            await scope_and_permission_changes(client, fixture, ids, handle)
        await transport.close()
        assert not list(scratch.glob("ccp-workspace-mcp-*"))
    print(
        "PASS: seven workspace content tools over stdio/CLI/TLS, large snapshots, archive privacy and scope isolation"
    )


if __name__ == "__main__":
    asyncio.run(check())
