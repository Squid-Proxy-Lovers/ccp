"""Real MCP guidance reads through the configured CLI and pinned workspace TLS."""

from uuid import uuid4

from mcp_smoke import data


async def call(client, **arguments):
    reply = await client.call_tool("workspace_guidance", arguments, raise_on_error=False)
    assert not reply.is_error
    return data(reply)


async def refused(client, arguments):
    reply = await client.call_tool("workspace_guidance", arguments, raise_on_error=False)
    assert reply.is_error
    assert "PRIVATE_GUIDANCE" not in str(reply)


def publish(fixture, workspace, revision, text):
    scope = (
        {"kind": "global"}
        if workspace is None
        else {"kind": "workspace", "workspace_id": workspace}
    )
    key = fixture.setup["instance_admin_key"]["secret"] if workspace is None else None
    return fixture.admin(
        "guidance/save",
        {"scope": scope, "operation_id": str(uuid4()), "expected_revision": revision, "text": text},
        key,
    )


async def check_guidance(fixture, client):
    workspace = fixture.setup["workspace_id"]
    scope = {"selection_id": client.selection_id}
    profile = fixture.profile / "workspace-profile.sqlite3"
    before = profile.read_bytes()
    empty = await call(client, profile="office", scope=scope)
    assert empty["global"]["revision"] == 0 and empty["workspace"]["revision"] == 0
    assert empty["global"]["publisher"] is None
    publish(fixture, None, 0, "Use UTC")
    publish(fixture, workspace, 0, "\0" * 8192)
    global_only = await call(client, profile="office")
    assert global_only["global"]["text"] == "Use UTC" and global_only["workspace"] is None
    value = await call(client, profile="office", scope=scope)
    assert value["precedence"] == "global_first"
    assert value["workspace"]["scope"] == {"kind": "workspace", "workspace_id": workspace}
    assert value["workspace"]["text"] == "\0" * 8192
    assert value["workspace"]["publisher"]["actor_kind"] == "key"
    assert set(value["workspace"]["publisher"]) == {
        "actor_kind",
        "actor_id",
        "principal_id",
        "recorded_at",
    }
    publish(fixture, workspace, 1, "é" * 4096)
    assert (await call(client, profile="office", scope={"workspace_id": workspace}))["workspace"][
        "text"
    ] == "é" * 4096
    publish(fixture, workspace, 2, "")
    cleared = await call(client, profile="office", scope=scope)
    assert cleared["workspace"]["revision"] == 3 and cleared["workspace"]["text"] == ""
    assert cleared["workspace"]["publisher"] is not None
    fixture.admin("workspaces/archive", {"workspace_id": workspace})
    assert (await call(client, profile="office", scope=scope))["workspace"]["revision"] == 3
    fixture.admin("workspaces/reactivate", {"workspace_id": workspace})
    await refused(client, {"profile": "office", "scope": {"workspace_id": str(uuid4())}})
    for arguments in [
        {"profile": "office", "text": "PRIVATE_GUIDANCE"},
        {
            "profile": "office",
            "scope": {"workspace_id": workspace, "publisher": "PRIVATE_GUIDANCE"},
        },
        {"profile": ["PRIVATE_GUIDANCE"]},
    ]:
        await refused(client, arguments)
    selected = await client.call_tool(
        "workspace_selection", {"profile": "office"}, raise_on_error=False
    )
    assert data(selected)["workspace_ids"] == [workspace]
    assert profile.read_bytes() == before
