"""Real stdio and independent MCP sessions through the Rust CLI and workspace TLS."""

from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path

from ccp_mcp_server.workspace import build_server
from ccp_mcp_server.workspace_config import WorkspaceConfig
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from mcp_smoke import data
from workspace_mcp_content_smoke import check as check_content
from workspace_mcp_delivery import check as check_delivery
from workspace_mcp_entry_smoke import check as check_entries
from workspace_mcp_fixture import WorkspaceFixture
from workspace_mcp_guidance import check_guidance
from workspace_mcp_write_smoke import check as check_writes


class SelectedClient:
    def __init__(self, client, selection_id):
        self.client = client
        self.selection_id = selection_id

    async def call_tool(self, name, arguments, *, raise_on_error=False):
        arguments = dict(arguments)
        if name in {"workspace_selection", "workspace_select", "workspace_resolve"} or (
            name == "workspace_target" and "workspace_id" not in arguments
        ):
            arguments["selection_id"] = self.selection_id
        return await self.client.call_tool(name, arguments, raise_on_error=raise_on_error)


async def selected(client):
    result = await client.call_tool("workspace_selection_open", {})
    return SelectedClient(client, data(result)["selection_id"])


async def call(client, name, **arguments):
    result = await client.call_tool(name, arguments, raise_on_error=False)
    assert not result.is_error, name
    return data(result)


async def refused(client, name, **arguments):
    result = await client.call_tool(name, arguments, raise_on_error=False)
    assert result.is_error, name
    return "".join(item.text for item in result.content if hasattr(item, "text"))


def choice(workspace, revision=0):
    return {
        "workspace_ids": [workspace],
        "default_workspace_id": workspace,
        "expected_revision": revision,
    }


async def stdio_protocol(fixture):
    config = fixture.root / "bridge.json"
    config.write_text(json.dumps({"format": 1, "profiles": {"office": str(fixture.profile)}}))
    config.chmod(0o600)
    scratch = fixture.root / "bridge-scratch"
    scratch.mkdir(mode=0o700)
    environment = {
        "CCP_WORKSPACE_CONFIG": str(config),
        "CCP_CLIENT_BIN": str(fixture.client),
        "TMPDIR": str(scratch),
        "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "mcp" / "src"),
    }
    transport = StdioTransport(
        command=sys.executable,
        args=["-m", "ccp_mcp_server.workspace"],
        env=environment,
        keep_alive=False,
    )
    first = fixture.setup["workspace_id"]
    async with Client(transport) as raw:
        client = await selected(raw)
        assert await call(client, "workspace_profiles") == ["office"]
        state = await call(client, "workspace_selection", profile="office")
        assert state["revision"] == 0 and state["workspace_ids"] == []
        await call(client, "workspace_select", profile="office", choice=choice(first))
        assert (await call(client, "workspace_selection", profile="office"))["revision"] == 1
        assert (await call(client, "workspace_resolve", profile="office"))["targets"][0][
            "workspace"
        ]["id"] == first
        assert (await call(client, "workspace_target", profile="office"))["workspace"][
            "id"
        ] == first
        assert (await call(client, "workspace_profile", profile="office"))[
            "client_id"
        ] == fixture.client_id
        await check_guidance(fixture, client)
    assert not list(scratch.glob("ccp-workspace-mcp-*"))
    async with Client(transport) as raw:
        old = await raw.call_tool(
            "workspace_selection",
            {"profile": "office", "selection_id": client.selection_id},
            raise_on_error=False,
        )
        assert old.is_error
        client = await selected(raw)
        assert (await call(client, "workspace_selection", profile="office"))["workspace_ids"] == []
    await transport.close()


async def independent_callers(fixture, server):
    first = fixture.setup["workspace_id"]
    async with Client(server) as raw_one, Client(server) as raw_two:
        one, two = await selected(raw_one), await selected(raw_two)
        assert (await call(one, "workspace_selection", profile="office"))["workspace_ids"] == []
        assert (await call(two, "workspace_selection", profile="office"))["workspace_ids"] == []
        await call(one, "workspace_select", profile="office", choice=choice(first))
        provisioned = fixture.add_second_workspace()
        second = provisioned["workspace_id"]
        page = await call(two, "workspace_list", profile="office", limit=1)
        assert len(page["workspaces"]) == 1 and page["next_cursor"] is not None
        next_page = await call(two, "workspace_list", profile="office", after=page["next_cursor"])
        assert len(next_page["workspaces"]) == 1
        assert (await call(one, "workspace_selection", profile="office"))["workspace_ids"] == [
            first
        ]
        assert (await call(two, "workspace_selection", profile="office"))["workspace_ids"] == []
        await call(two, "workspace_select", profile="office", choice=choice(second))
        extra = await selected(raw_one)
        assert (await call(extra, "workspace_selection", profile="office"))["workspace_ids"] == []
        shared = SelectedClient(raw_two, one.selection_id)
        assert (await call(shared, "workspace_selection", profile="office"))["workspace_ids"] == [
            first
        ]
        assert (await call(one, "workspace_target", profile="office", workspace_id=second))[
            "workspace"
        ]["id"] == second
        assert (await call(one, "workspace_target", profile="office"))["workspace"]["id"] == first
        await refused(one, "workspace_select", profile="office", choice=choice(second))
        await permission_changes(fixture, one, two, provisioned)
    async with Client(server) as raw:
        fresh = await selected(raw)
        assert (await call(fresh, "workspace_selection", profile="office"))["workspace_ids"] == []


async def permission_changes(fixture, one, two, provisioned):
    first, second = fixture.setup["workspace_id"], provisioned["workspace_id"]
    fixture.recover_first()
    assert (await call(one, "workspace_list", profile="recovered"))["workspaces"][0]["id"] == first
    assert (await call(one, "workspace_selection", profile="recovered"))["workspace_ids"] == []
    await refused(one, "workspace_target", profile="office")
    await refused(one, "workspace_target", profile="recovered", workspace_id=second)
    await refused(one, "workspace_guidance", profile="recovered", scope={"workspace_id": second})
    assert (
        await call(one, "workspace_guidance", profile="recovered", scope={"workspace_id": first})
    )["workspace"]["scope"]["workspace_id"] == first
    assert (await call(two, "workspace_target", profile="office"))["workspace"]["id"] == second
    error = await refused(one, "workspace_profile", profile=str(fixture.root))
    assert str(fixture.root) not in error
    await refused(
        one, "workspace_select", profile="recovered", choice=choice("--selection-dir=/outside")
    )
    grants = fixture.admin("workspaces/grants", {"workspace_id": second}, provisioned["owner_key"])
    fixture.admin(
        "grants/revoke", {"grant_id": grants["grants"][0]["id"]}, provisioned["owner_key"]
    )
    await refused(two, "workspace_target", profile="office")
    await refused(two, "workspace_guidance", profile="office", scope={"workspace_id": second})
    assert (await call(two, "workspace_selection", profile="office"))[
        "default_workspace_id"
    ] == second
    fixture.admin(
        "clients/disable",
        {"client_id": fixture.client_id},
        fixture.setup["instance_admin_key"]["secret"],
    )
    await refused(one, "workspace_list", profile="recovered")
    await refused(one, "workspace_guidance", profile="recovered")
    assert (await call(one, "workspace_selection", profile="office"))["workspace_ids"] == [first]


async def check():
    await check_delivery()
    await check_entries()
    await check_writes()
    await check_content()
    with WorkspaceFixture() as fixture:
        await stdio_protocol(fixture)
        config = WorkspaceConfig(
            fixture.client, {"office": fixture.profile, "recovered": fixture.recovered}
        )
        with WorkspaceRuntime(config) as runtime:
            await independent_callers(fixture, build_server(runtime))
    print(
        "PASS: workspace MCP stdio, independent caller handles, explicit selections, scoped recovery and current revocation over TLS"
    )


if __name__ == "__main__":
    asyncio.run(check())
