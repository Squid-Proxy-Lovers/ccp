"""Automatic sidebands over real stdio/CLI/TLS, including committed writes and authority loss."""

from __future__ import annotations

import json
import sqlite3
import sys
import time
from pathlib import Path
from uuid import uuid4

from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from mcp_smoke import data
from workspace_mcp_fixture import WorkspaceFixture
from workspace_mcp_guidance import publish


async def open_handle(client):
    reply = await client.call_tool("workspace_selection_open", {})
    assert len(reply.content) == 1
    return data(reply)["selection_id"]


def sideband(reply):
    assert not reply.is_error
    assert len(reply.content) == 2
    primary = data(reply)
    assert json.loads(reply.content[0].text) == primary
    assert "_ccp_guidance" not in primary
    value = json.loads(reply.content[1].text)["_ccp_guidance"]
    assert value["format"] == 1 and value["precedence"] == "global_first"
    return value


async def read(client, workspace, handle=None, profile="office"):
    scope = {"workspace_id": workspace}
    if handle is not None:
        scope["selection_id"] = handle
    reply = await client.call_tool("workspace_shelves", {"profile": profile, "scope": scope})
    assert data(reply)["workspace_id"] == workspace
    return sideband(reply)


async def changes(fixture, client, workspace, handle):
    initial = await read(client, workspace, handle)
    assert [row["board"]["revision"] for row in initial["boards"]] == [0, 0]
    assert (await read(client, workspace, handle))["boards"] == []
    publish(fixture, None, 0, "Use UTC")
    changed = await read(client, workspace, handle)
    assert len(changed["boards"]) == 1 and changed["boards"][0]["board"]["text"] == "Use UTC"
    publish(fixture, workspace, 0, "\0" * 8192)
    changed = await read(client, workspace, handle)
    assert len(changed["boards"]) == 1
    assert changed["boards"][0]["board"]["text"] == "\0" * 8192
    publish(fixture, workspace, 1, "")
    assert (await read(client, workspace, handle))["boards"][0]["board"]["revision"] == 2
    other = await open_handle(client)
    assert len((await read(client, workspace, other))["boards"]) == 2
    assert (await read(client, workspace, handle))["boards"] == []
    for _repeat in range(2):
        assert len((await read(client, workspace))["boards"]) == 2
    fixture.admin("workspaces/archive", {"workspace_id": workspace})
    assert (await read(client, workspace, handle))["status"] == "current"
    fixture.admin("workspaces/reactivate", {"workspace_id": workspace})
    fixture.command(fixture.client, "workspace", "renew", "--profile", fixture.profile)
    renewed = await read(client, workspace, handle)
    assert len(renewed["boards"]) == 2
    assert (
        renewed["boards"][0]["context"]["credential_id"]
        != initial["boards"][0]["context"]["credential_id"]
    )
    return renewed


def malformed_board(fixture):
    # Deliberately malformed publisher in this owned fixture: ordinary writes
    # remain authorized while the independent guidance read cannot be validated.
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        db.execute(
            "INSERT INTO ccp_guidance_versions(board_key,revision,operation_id,text,publisher_kind,publisher_id,principal_id,recorded_at) VALUES('global',2,?,'PRIVATE_GUIDANCE','key',?,?,?)",
            (
                str(uuid4()),
                "00000000-0000-0000-0000-000000000000",
                db.execute(
                    "SELECT principal_id FROM ccp_guidance_versions WHERE board_key='global' AND revision=1"
                ).fetchone()[0],
                int(time.time()),
            ),
        )
        db.execute("UPDATE ccp_guidance_heads SET revision=2 WHERE board_key='global'")


async def write_with_unavailable_guidance(fixture, client, workspace):
    malformed_board(fixture)
    reply = await client.call_tool(
        "workspace_create_shelf",
        {"profile": "office", "request": {"workspace_id": workspace, "name": "committed once"}},
    )
    primary = data(reply)
    warning = sideband(reply)
    assert primary["workspace_id"] == workspace and primary["data"]["name"] == "committed once"
    assert warning["status"] == "unavailable" and warning["boards"] == []
    assert "may be stale" in warning["warning"] and "do not retry a mutation" in warning["warning"]
    assert "PRIVATE_GUIDANCE" not in str(reply)
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        assert (
            db.execute(
                "SELECT count(*) FROM ccp_content_shelves WHERE name='committed once'"
            ).fetchone()[0]
            == 1
        )
        db.execute("UPDATE ccp_guidance_heads SET revision=1 WHERE board_key='global'")
        db.execute("DELETE FROM ccp_guidance_versions WHERE board_key='global' AND revision=2")


async def readmission_and_disable(fixture, client, workspace, handle, previous):
    grants = fixture.admin("workspaces/grants", {"workspace_id": workspace})
    live = [grant for grant in grants["grants"] if grant["revoked_at"] is None]
    for grant in live:
        fixture.admin("grants/revoke", {"grant_id": grant["id"]})
    # Local selection succeeds independently of current context-read access.
    selected = await client.call_tool(
        "workspace_select",
        {
            "profile": "office",
            "selection_id": handle,
            "choice": {"workspace_ids": [], "expected_revision": 0},
        },
    )
    assert data(selected)["workspace_ids"] == []
    # Model internal admit_existing_client's new grant/binding in the owned DB.
    # The public targeted bearer readmission flow issues a new credential;
    # there is no public adapter for this internal same-credential admin action.
    grant = str(uuid4())
    old = previous["boards"][1]["context"]
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        db.execute("PRAGMA foreign_keys=ON")
        db.execute(
            "INSERT INTO ccp_grants(id,workspace_id,client_id,role) VALUES(?,?,?,'read_write')",
            (grant, workspace, fixture.client_id),
        )
        db.execute(
            "INSERT INTO ccp_credential_grants(credential_id,grant_id,client_id) VALUES(?,?,?)",
            (old["credential_id"], grant, fixture.client_id),
        )
        assert (
            db.execute("SELECT count(*) FROM ccp_grants WHERE revoked_at IS NOT NULL").fetchone()[0]
            >= 1
        )
    current = await read(client, workspace, handle)
    assert len(current["boards"]) == 1
    context = current["boards"][0]["context"]
    old = previous["boards"][1]["context"]
    assert context["credential_id"] == old["credential_id"]
    assert context["workspace_grant_id"] != old["workspace_grant_id"]
    fixture.recover_first()
    recovered = await read(client, workspace, handle, "recovered")
    assert len(recovered["boards"]) == 2
    assert recovered["boards"][0]["context"]["client_id"] == fixture.client_id
    fixture.admin(
        "clients/disable",
        {"client_id": fixture.client_id},
        fixture.setup["instance_admin_key"]["secret"],
    )
    reply = await client.call_tool("workspace_profile", {"profile": "recovered"})
    assert data(reply)["client_id"] == fixture.client_id
    warning = sideband(reply)
    assert warning["status"] == "unavailable" and warning["boards"] == []
    denied = await client.call_tool(
        "workspace_shelves",
        {"profile": "recovered", "scope": {"workspace_id": workspace}},
        raise_on_error=False,
    )
    assert denied.is_error and len(denied.content) == 1


async def check():
    with WorkspaceFixture() as fixture:
        config = fixture.root / "delivery-bridge.json"
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
        transport = StdioTransport(
            command=sys.executable,
            args=["-m", "ccp_mcp_server.workspace"],
            env={
                "CCP_WORKSPACE_CONFIG": str(config),
                "CCP_CLIENT_BIN": str(fixture.client),
                "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "mcp/src"),
            },
            keep_alive=False,
        )
        async with Client(transport) as client:
            workspace = fixture.setup["workspace_id"]
            handle = await open_handle(client)
            previous = await changes(fixture, client, workspace, handle)
            await write_with_unavailable_guidance(fixture, client, workspace)
            await readmission_and_disable(fixture, client, workspace, handle, previous)
        await transport.close()
    print(
        "PASS: automatic MCP guidance, isolated callers, revision/credential/grant changes and committed writes with unavailable guidance over stdio/TLS"
    )
