"""Real stdio status intents, retained uncertainty and guidance after local forgetting."""

from __future__ import annotations

import json
import sqlite3
import sys
from pathlib import Path
from uuid import uuid4

from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from workspace_mcp_content_smoke import call, refused
from workspace_mcp_delivery import sideband
from workspace_mcp_fixture import WorkspaceFixture
from workspace_status_faults import check as check_response_faults


def profile_sql(fixture, sql):
    with sqlite3.connect(fixture.profile / "workspace-profile.sqlite3") as db:
        db.execute(sql)


def server_sql(fixture, sql):
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        db.execute(sql)


def operations(fixture):
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        return db.execute("SELECT count(*) FROM ccp_work_status_operations").fetchone()[0]


def request(workspace, revision=0, status=None):
    return {
        "workspace_id": workspace,
        "status_id": status or str(uuid4()),
        "operation_id": str(uuid4()),
        "expected_revision": revision,
        "message": "PRIVATE_STATUS é\ufeff",
        "agent": "\ufeff",
    }


async def lifecycle(client, fixture):
    workspace = fixture.setup["workspace_id"]
    scope = {"workspace_id": workspace}
    req = request(workspace)
    initial = await client.call_tool(
        "workspace_update_status", {"profile": "office", "request": req}
    )
    from mcp_smoke import data

    first = data(initial)
    assert first["data"]["revision"] == 1 and operations(fixture) == 1
    assert "PRIVATE_STATUS" not in str(initial)
    assert [row["board"]["revision"] for row in sideband(initial)["boards"]] == [0, 0]
    row = (
        await call(
            client,
            "workspace_status_message",
            profile="office",
            scope=scope,
            status_id=req["status_id"],
        )
    )["data"]
    assert row["message"] == req["message"] and row["agent"] == "\ufeff"
    server_sql(fixture, "UPDATE ccp_work_status SET updated_at=1")
    page = (
        await call(
            client, "workspace_status_messages", profile="office", scope=scope, query={"limit": 1}
        )
    )["data"]
    assert page["items"][0]["stale"] and page["items"][0]["message"] == req["message"]
    clear = {
        key: value
        for key, value in request(workspace, 1, req["status_id"]).items()
        if key not in {"message", "agent"}
    }
    assert (await call(client, "workspace_clear_status", profile="office", request=clear))["data"][
        "revision"
    ] == 2
    replay = await call(client, "workspace_update_status", profile="office", request=req)
    assert replay == first and operations(fixture) == 2
    assert (await call(client, "workspace_status_messages", profile="office", scope=scope))["data"][
        "items"
    ] == []
    assert (
        await call(
            client,
            "workspace_status_message",
            profile="office",
            scope=scope,
            status_id=req["status_id"],
        )
    )["data"]["message"] is None
    second = fixture.add_second_workspace()["workspace_id"]
    assert (
        await call(
            client, "workspace_status_messages", profile="office", scope={"workspace_id": second}
        )
    )["data"]["items"] == []
    await refused(client, "workspace_update_status", profile="office", request=request(second))
    await refused(
        client,
        "workspace_status_message",
        profile="office",
        scope={"workspace_id": second},
        status_id=req["status_id"],
    )
    # A read-only scope can pass preflight but reject the write: retain that uncertainty.
    read_only = await call(client, "workspace_status_pending", profile="office")
    assert read_only["target"]["workspace_id"] == second
    await call(
        client,
        "workspace_status_forget",
        profile="office",
        target=read_only["target"],
        confirm_local_forget=True,
    )
    return workspace


async def uncertain(client, fixture, workspace):
    req = request(workspace)
    profile_sql(
        fixture,
        "CREATE TRIGGER fail_status_finish BEFORE UPDATE ON status_state WHEN NEW.pending IS NULL BEGIN SELECT RAISE(ABORT,'PRIVATE_STATUS save'); END",
    )
    error = await refused(client, "workspace_update_status", profile="office", request=req)
    assert "PRIVATE_STATUS" not in error and operations(fixture) == 3
    saved = await call(client, "workspace_status_pending", profile="office")
    assert saved["target"]["operation_id"] == req["operation_id"]
    profile_sql(fixture, "DROP TRIGGER fail_status_finish")
    for field in saved["target"]:
        wrong = {**saved["target"], field: "f" * 64 if field == "request_hash" else str(uuid4())}
        await refused(client, "workspace_status_resume", profile="office", target=wrong)
        assert operations(fixture) == 3
    done = await call(client, "workspace_status_resume", profile="office", target=saved["target"])
    assert done["data"]["revision"] == 1 and operations(fixture) == 3
    assert (await call(client, "workspace_status_pending", profile="office"))["target"] is None
    server_sql(
        fixture,
        "CREATE TRIGGER fail_status_write BEFORE INSERT ON ccp_work_status BEGIN SELECT RAISE(ABORT,'PRIVATE_STATUS unavailable'); END",
    )
    await refused(client, "workspace_update_status", profile="office", request=request(workspace))
    saved = await call(client, "workspace_status_pending", profile="office")
    assert saved["target"] is not None
    server_sql(fixture, "UPDATE ccp_clients SET disabled=1")
    reply = await client.call_tool(
        "workspace_status_forget",
        {"profile": "office", "target": saved["target"], "confirm_local_forget": True},
    )
    from mcp_smoke import data

    forgotten = data(reply)
    assert forgotten["data"]["cancelled"] is False
    assert forgotten["data"]["operation"]["state"] == "locally_forgotten"
    assert sideband(reply)["status"] == "unavailable"
    assert operations(fixture) == 3
    assert (await call(client, "workspace_status_pending", profile="office"))["target"] is None
    server_sql(fixture, "UPDATE ccp_clients SET disabled=0")
    server_sql(fixture, "DROP TRIGGER fail_status_write")
    fixture.admin("workspaces/archive", {"workspace_id": workspace})
    scope = {"workspace_id": workspace}
    assert (
        len(
            (await call(client, "workspace_status_messages", profile="office", scope=scope))[
                "data"
            ]["items"]
        )
        == 1
    )
    await refused(client, "workspace_update_status", profile="office", request=request(workspace))
    assert operations(fixture) == 3


async def check():
    with WorkspaceFixture() as fixture:
        check_response_faults(fixture)
    with WorkspaceFixture() as fixture:
        config = fixture.root / "status-bridge.json"
        config.write_text(json.dumps({"format": 1, "profiles": {"office": str(fixture.profile)}}))
        config.chmod(0o600)
        scratch = fixture.root / "status-scratch"
        scratch.mkdir(mode=0o700)
        transport = StdioTransport(
            command=sys.executable,
            args=["-m", "ccp_mcp_server.workspace"],
            env={
                "CCP_WORKSPACE_CONFIG": str(config),
                "CCP_CLIENT_BIN": str(fixture.client),
                "TMPDIR": str(scratch),
                "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "mcp/src"),
            },
            keep_alive=False,
        )
        async with Client(transport) as client:
            workspace = await lifecycle(client, fixture)
            await uncertain(client, fixture, workspace)
        await transport.close()
        assert not list(scratch.glob("ccp-workspace-mcp-*"))
    print(
        "PASS: workspace status MCP stdio/private CLI/TLS, Unicode, CAS, guarded resume, scoped reads and local-only forgetting with unavailable guidance"
    )
