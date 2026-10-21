"""Real MCP private-file container writes, current scope and pending-state guards."""

from __future__ import annotations

import asyncio
import json
import sqlite3
import sys
from pathlib import Path
from uuid import uuid4

from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from workspace_mcp_content_smoke import call, mutate, refused
from workspace_mcp_fixture import WorkspaceFixture


def count(fixture, table):
    queries = {
        "ccp_content_shelves": "SELECT COUNT(*) FROM ccp_content_shelves",
        "ccp_content_books": "SELECT COUNT(*) FROM ccp_content_books",
    }
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        return db.execute(queries[table]).fetchone()[0]


def no_requests(scratch):
    assert not list(scratch.glob("ccp-workspace-mcp-*/request-*.json"))


async def build_hierarchy(client, fixture, scratch):
    workspace = fixture.setup["workspace_id"]
    shelf_request = {
        "workspace_id": workspace,
        "name": "  notes  ",
        "description": "PRIVATE_BODY\0é",
    }
    shelf = await call(client, "workspace_create_shelf", profile="office", request=shelf_request)
    assert shelf["workspace_id"] == workspace and shelf["data"]["workspace_id"] == workspace
    assert shelf["data"]["name"] == "notes"
    assert shelf["data"]["description"] == shelf_request["description"]
    no_requests(scratch)
    error = await refused(client, "workspace_create_shelf", profile="office", request=shelf_request)
    assert "inspect shelves/books" in error and count(fixture, "ccp_content_shelves") == 1
    book_request = {
        "workspace_id": workspace,
        "shelf_id": shelf["data"]["id"],
        "name": "é" * 128,
        "description": "\0" * 4096,
    }
    book = await call(client, "workspace_create_book", profile="office", request=book_request)
    assert book["data"]["shelf_id"] == shelf["data"]["id"]
    assert book["data"]["description"] == book_request["description"]
    await refused(client, "workspace_create_book", profile="office", request=book_request)
    assert count(fixture, "ccp_content_books") == 1
    entry = mutate(
        fixture,
        {
            "action": "create",
            "book_id": book["data"]["id"],
            "name": "entry",
            "content": {"text": "human text", "description": "", "labels": []},
        },
    )
    result = await call(
        client,
        "workspace_entry",
        profile="office",
        scope={"workspace_id": workspace},
        query={"entry_id": entry["entry_id"]},
    )
    assert result["data"]["content"]["text"] == "human text"
    assert result["data"]["entry"]["owner_client_id"] == fixture.client_id
    no_requests(scratch)
    return {"shelf": shelf["data"]["id"], "book": book["data"]["id"], "entry": entry["entry_id"]}


async def pending_guard(client, fixture, scratch, ids):
    with sqlite3.connect(fixture.profile / "workspace-profile.sqlite3") as db:
        db.execute(
            "CREATE TRIGGER fail_content BEFORE UPDATE ON content_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'fixture'); END;"
        )
    request = {
        "workspace_id": fixture.setup["workspace_id"],
        "operation_id": str(uuid4()),
        "agent": None,
        "change": {
            "action": "append",
            "entry_id": ids["entry"],
            "expected_revision": 1,
            "text": "PRIVATE_BODY delta",
        },
    }
    process = await asyncio.create_subprocess_exec(
        str(fixture.client),
        "workspace",
        "content",
        "--profile",
        str(fixture.profile),
        "mutate",
        "--request-file",
        str(fixture.secret_file(json.dumps(request))),
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await asyncio.wait_for(process.communicate(), 30)
    assert process.returncode != 0 and not stdout and b"PRIVATE_BODY" not in stderr
    before = count(fixture, "ccp_content_shelves")
    await refused(
        client,
        "workspace_create_shelf",
        profile="office",
        request={"workspace_id": fixture.setup["workspace_id"], "name": "blocked"},
    )
    await refused(
        client,
        "workspace_create_book",
        profile="office",
        request={
            "workspace_id": fixture.setup["workspace_id"],
            "shelf_id": ids["shelf"],
            "name": "blocked",
        },
    )
    metadata = await call(client, "workspace_profile", profile="office")
    assert metadata["pending_content"]["operation_id"] == request["operation_id"]
    assert count(fixture, "ccp_content_shelves") == before
    assert count(fixture, "ccp_content_books") == 1
    no_requests(scratch)
    with sqlite3.connect(fixture.profile / "workspace-profile.sqlite3") as db:
        db.execute("DROP TRIGGER fail_content")
    outcome = fixture.command(
        fixture.client, "workspace", "content", "--profile", fixture.profile, "resolve"
    )
    assert outcome["outcome"]["state"] == "committed"
    assert outcome["operation_id"] == request["operation_id"]


async def scopes_and_current_authority(client, fixture, scratch, ids):
    first = fixture.setup["workspace_id"]
    handle = (await call(client, "workspace_selection_open"))["selection_id"]
    await call(
        client,
        "workspace_select",
        profile="office",
        selection_id=handle,
        choice={"workspace_ids": [first], "default_workspace_id": first, "expected_revision": 0},
    )
    second = fixture.add_second_workspace()["workspace_id"]
    await call(
        client,
        "workspace_select",
        profile="office",
        selection_id=handle,
        choice={"workspace_ids": [second], "default_workspace_id": second, "expected_revision": 1},
    )
    created = await call(
        client,
        "workspace_create_shelf",
        profile="office",
        request={"workspace_id": first, "name": "after default switch"},
    )
    assert created["workspace_id"] == first
    assert (await call(client, "workspace_target", profile="office", selection_id=handle))[
        "workspace"
    ]["id"] == second
    await refused(
        client,
        "workspace_create_shelf",
        profile="office",
        request={"workspace_id": second, "name": "read-only denied"},
    )
    await refused(
        client,
        "workspace_create_book",
        profile="office",
        request={"workspace_id": second, "shelf_id": ids["shelf"], "name": "foreign parent"},
    )
    assert count(fixture, "ccp_content_shelves") == 2 and count(fixture, "ccp_content_books") == 1
    fixture.admin("workspaces/archive", {"workspace_id": first})
    await refused(
        client,
        "workspace_create_shelf",
        profile="office",
        request={"workspace_id": first, "name": "archived denied"},
    )
    fixture.admin("workspaces/reactivate", {"workspace_id": first})
    fixture.recover_first()
    await refused(
        client,
        "workspace_create_shelf",
        profile="office",
        request={"workspace_id": first, "name": "old binding denied"},
    )
    recovered = await call(
        client,
        "workspace_create_shelf",
        profile="recovered",
        request={"workspace_id": first, "name": "recovered writer"},
    )
    assert recovered["workspace_id"] == first
    await refused(
        client,
        "workspace_create_shelf",
        profile="recovered",
        request={"workspace_id": second, "name": "no authority union"},
    )
    fixture.admin(
        "clients/disable",
        {"client_id": fixture.client_id},
        fixture.setup["instance_admin_key"]["secret"],
    )
    await refused(
        client,
        "workspace_create_book",
        profile="recovered",
        request={"workspace_id": first, "shelf_id": ids["shelf"], "name": "disabled denied"},
    )
    assert count(fixture, "ccp_content_shelves") == 3 and count(fixture, "ccp_content_books") == 1
    no_requests(scratch)


async def check():
    with WorkspaceFixture() as fixture:
        config = fixture.root / "write-bridge.json"
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
        scratch = fixture.root / "write-scratch"
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
            assert len(tools) == 31
            assert {"workspace_create_shelf", "workspace_create_book"} <= {
                tool.name for tool in tools
            }
            ids = await build_hierarchy(client, fixture, scratch)
            await pending_guard(client, fixture, scratch, ids)
            await scopes_and_current_authority(client, fixture, scratch, ids)
        await transport.close()
        assert not list(scratch.glob("ccp-workspace-mcp-*"))
    print(
        "PASS: MCP private shelf/book writes, pending guards, immutable explicit targets and current scope over TLS"
    )


if __name__ == "__main__":
    asyncio.run(check())
