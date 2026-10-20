"""Real MCP durable entry intents, guarded outcomes and recovery without union."""

from __future__ import annotations

import asyncio
import json
import sqlite3
import sys
from pathlib import Path
from uuid import uuid4

from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from workspace_mcp_content_smoke import call, refused
from workspace_mcp_fixture import WorkspaceFixture


def count(fixture, table):
    queries = {
        "operations": "SELECT COUNT(*) FROM ccp_content_operations",
        "clients": "SELECT COUNT(*) FROM ccp_clients",
    }
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        return db.execute(queries[table]).fetchone()[0]


def profile_sql(fixture, statement):
    with sqlite3.connect(fixture.profile / "workspace-profile.sqlite3") as db:
        db.execute(statement)


def server_sql(fixture, statement):
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        db.execute(statement)


def request(fixture, change):
    return {
        "workspace_id": fixture.setup["workspace_id"],
        "operation_id": str(uuid4()),
        "agent": "fixture agent",
        "change": change,
    }


async def mutate(client, fixture, change):
    return await call(
        client, "workspace_mutate_entry", profile="office", request=request(fixture, change)
    )


async def five_actions(client, fixture):
    workspace = fixture.setup["workspace_id"]
    state = await call(client, "workspace_entry_pending", profile="office")
    assert state == {"pending": None, "last": None, "target": None}
    shelf = await call(
        client,
        "workspace_create_shelf",
        profile="office",
        request={"workspace_id": workspace, "name": "notes"},
    )
    book = await call(
        client,
        "workspace_create_book",
        profile="office",
        request={"workspace_id": workspace, "shelf_id": shelf["data"]["id"], "name": "book"},
    )
    created = await mutate(
        client,
        fixture,
        {
            "action": "create",
            "book_id": book["data"]["id"],
            "name": "entry",
            "content": {"text": "PRIVATE_BODY original", "labels": ["z", "a"]},
        },
    )
    entry = created["data"]["entry_id"]
    assert created["data"]["revision"] == 1
    append = request(
        fixture, {"action": "append", "entry_id": entry, "expected_revision": 1, "text": "+delta"}
    )
    added = await call(client, "workspace_mutate_entry", profile="office", request=append)
    repeated = await call(client, "workspace_mutate_entry", profile="office", request=append)
    assert (
        added == repeated and added["data"]["revision"] == 2 and count(fixture, "operations") == 2
    )
    assert "PRIVATE_BODY" not in json.dumps(added)
    stale = request(
        fixture,
        {
            "action": "revise",
            "entry_id": entry,
            "expected_revision": 1,
            "content": {"text": "stale"},
        },
    )
    await refused(client, "workspace_mutate_entry", profile="office", request=stale)
    cancelled = await call(client, "workspace_entry_pending", profile="office")
    assert cancelled["pending"] is None and cancelled["last"]["state"] == "cancelled"
    assert cancelled["target"]["operation_id"] == stale["operation_id"]
    proof = await call(
        client, "workspace_entry_resolve", profile="office", target=cancelled["target"]
    )
    assert proof["data"]["outcome"] == {"state": "cancelled"}
    revised = await mutate(
        client,
        fixture,
        {
            "action": "revise",
            "entry_id": entry,
            "expected_revision": 2,
            "content": {"text": "revised", "description": "exact", "labels": ["new"]},
        },
    )
    assert revised["data"]["revision"] == 3
    archived = await mutate(
        client, fixture, {"action": "archive", "entry_id": entry, "expected_revision": 3}
    )
    assert archived["data"]["revision"] == 4
    await refused(
        client,
        "workspace_entry",
        profile="office",
        scope={"workspace_id": workspace},
        query={"entry_id": entry},
    )
    restored = await mutate(
        client, fixture, {"action": "restore", "entry_id": entry, "expected_revision": 4}
    )
    assert restored["data"]["revision"] == 5
    wrong = {**restored["target"], "request_hash": "f" * 64}
    await refused(client, "workspace_entry_resume", profile="office", target=wrong)
    await refused(
        client, "workspace_entry_resolve", profile="office", target=wrong, via_profile="recovered"
    )
    assert count(fixture, "operations") == 5
    return entry, book["data"]["id"]


async def local_save_failure(client, fixture, entry):
    profile_sql(
        fixture,
        "CREATE TRIGGER fail_outcome BEFORE UPDATE ON content_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'fixture'); END",
    )
    intent = request(
        fixture,
        {
            "action": "append",
            "entry_id": entry,
            "expected_revision": 5,
            "text": "+PRIVATE_BODY last",
        },
    )
    await refused(client, "workspace_mutate_entry", profile="office", request=intent)
    assert count(fixture, "operations") == 6
    pending = await call(client, "workspace_entry_pending", profile="office")
    assert pending["pending"]["operation_id"] == intent["operation_id"]
    assert "PRIVATE_BODY" not in json.dumps(pending)
    original = pending["target"]
    profile_sql(fixture, "DROP TRIGGER fail_outcome")
    resumed = await call(client, "workspace_entry_resume", profile="office", target=original)
    assert resumed["data"]["revision"] == 6 and resumed["target"] == original
    again = await call(client, "workspace_entry_resume", profile="office", target=original)
    assert again == resumed and count(fixture, "operations") == 6
    state = await call(client, "workspace_entry_pending", profile="office")
    assert state["pending"] is None and state["last"]["state"] == "committed"


async def cancellation_fence(client, fixture, entry):
    server_sql(
        fixture,
        "CREATE TRIGGER fail_entry BEFORE UPDATE ON ccp_content_entries BEGIN SELECT RAISE(ABORT,'fixture'); END",
    )
    server_sql(
        fixture,
        "CREATE TRIGGER fail_cancel BEFORE INSERT ON ccp_content_cancellations BEGIN SELECT RAISE(ABORT,'fixture'); END",
    )
    intent = request(
        fixture,
        {"action": "append", "entry_id": entry, "expected_revision": 6, "text": "+uncommitted"},
    )
    await refused(client, "workspace_mutate_entry", profile="office", request=intent)
    pending = await call(client, "workspace_entry_pending", profile="office")
    assert pending["pending"]["operation_id"] == intent["operation_id"]
    server_sql(fixture, "DROP TRIGGER fail_entry")
    server_sql(fixture, "DROP TRIGGER fail_cancel")
    server_sql(fixture, "UPDATE ccp_grants SET role='read'")
    fixture.admin("workspaces/archive", {"workspace_id": fixture.setup["workspace_id"]})
    result = await call(
        client, "workspace_entry_resolve", profile="office", target=pending["target"]
    )
    assert result["data"]["outcome"] == {"state": "cancelled"}
    fixture.admin("workspaces/reactivate", {"workspace_id": fixture.setup["workspace_id"]})
    server_sql(fixture, "UPDATE ccp_grants SET role='read_write'")
    await refused(client, "workspace_mutate_entry", profile="office", request=intent)
    assert count(fixture, "operations") == 6


async def recovery_and_large(client, fixture, entry, book):
    large = await mutate(
        client,
        fixture,
        {"action": "create", "book_id": book, "name": "large", "content": {"text": ""}},
    )
    text = "\0" * (256 * 1024)
    await mutate(
        client,
        fixture,
        {
            "action": "append",
            "entry_id": large["data"]["entry_id"],
            "expected_revision": 1,
            "text": text,
        },
    )
    snapshot = await call(
        client,
        "workspace_entry",
        profile="office",
        scope={"workspace_id": fixture.setup["workspace_id"]},
        query={"entry_id": large["data"]["entry_id"]},
    )
    assert snapshot["data"]["content"]["text"] == text and snapshot["data"]["appended_text"] == text
    second = fixture.add_second_workspace()["workspace_id"]
    profile_sql(
        fixture,
        "CREATE TRIGGER fail_outcome BEFORE UPDATE ON content_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'fixture'); END",
    )
    await refused(
        client,
        "workspace_mutate_entry",
        profile="office",
        request=request(
            fixture,
            {"action": "append", "entry_id": entry, "expected_revision": 6, "text": "+recovery"},
        ),
    )
    pending = await call(client, "workspace_entry_pending", profile="office")
    original = pending["target"]
    profile_sql(fixture, "DROP TRIGGER fail_outcome")
    old_info = await call(client, "workspace_profile", profile="office")
    fixture.recover_first()
    await refused(client, "workspace_entry_resolve", profile="office", target=original)
    assert (await call(client, "workspace_entry_pending", profile="office"))["pending"] is not None
    resolved = await call(
        client,
        "workspace_entry_resolve",
        profile="office",
        target=original,
        via_profile="recovered",
    )
    assert resolved["data"]["outcome"]["revision"] == 7 and resolved["target"] == original
    assert (await call(client, "workspace_profile", profile="office"))["credential_id"] == old_info[
        "credential_id"
    ]
    await refused(client, "workspace_shelves", profile="recovered", scope={"workspace_id": second})
    assert (
        await call(client, "workspace_shelves", profile="office", scope={"workspace_id": second})
    )["data"]["items"] == []
    assert count(fixture, "clients") == 1
    fixture.admin(
        "clients/disable",
        {"client_id": fixture.client_id},
        fixture.setup["instance_admin_key"]["secret"],
    )
    await refused(
        client,
        "workspace_entry_resolve",
        profile="office",
        target=original,
        via_profile="recovered",
    )


async def check():
    with WorkspaceFixture() as fixture:
        config = fixture.root / "entry-bridge.json"
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
        scratch = fixture.root / "entry-scratch"
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
            assert {
                "workspace_mutate_entry",
                "workspace_entry_pending",
                "workspace_entry_resume",
                "workspace_entry_resolve",
            } <= {tool.name for tool in tools}
            entry, book = await five_actions(client, fixture)
            await local_save_failure(client, fixture, entry)
            await cancellation_fence(client, fixture, entry)
            await recovery_and_large(client, fixture, entry, book)
            assert not list(scratch.glob("ccp-workspace-mcp-*/request-*.json"))
        await transport.close()
        assert not list(scratch.glob("ccp-workspace-mcp-*"))
    print(
        "PASS: five MCP entry actions, guarded exact retries/cancellation, private large payload and scoped recovery"
    )


if __name__ == "__main__":
    asyncio.run(check())
