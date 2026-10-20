"""Real read-only shelf lifecycle metadata through MCP, the CLI and pinned TLS."""

from uuid import uuid4

from workspace_mcp_content_smoke import call, content_command


def transition(fixture, shelf, revision, archived):
    return fixture.admin(
        "content/shelves/transition",
        {
            "workspace_id": fixture.setup["workspace_id"],
            "shelf_id": shelf,
            "operation_id": str(uuid4()),
            "expected_revision": revision,
            "archived": archived,
        },
    )


async def check_shelf_reads(client, fixture, ids, scope):
    workspace = fixture.setup["workspace_id"]
    tools = {tool.name for tool in await client.list_tools()}
    assert {"workspace_shelf_state", "workspace_archived_shelves"} <= tools
    assert (
        not {"workspace_archive_shelf", "workspace_restore_shelf", "workspace_transition_shelf"}
        & tools
    )
    query = {"entry_id": ids["entry"]}
    before = await call(client, "workspace_entry", profile="office", scope=scope, query=query)
    state = await call(
        client, "workspace_shelf_state", profile="office", scope=scope, node_id=ids["shelf"]
    )
    assert (
        state["workspace_id"] == workspace
        and state["data"]["revision"] == 1
        and not state["data"]["archived"]
    )
    empty = await call(client, "workspace_archived_shelves", profile="office", scope=scope)
    assert empty["data"] == {"items": [], "next_cursor": None}
    extra = content_command(
        fixture,
        "create-shelf",
        {"workspace_id": workspace, "name": "é" * 128, "description": "PRIVATE_DESCRIPTION"},
    )
    transition(fixture, extra["id"], 1, True)
    transition(fixture, ids["shelf"], 1, True)
    active = await call(client, "workspace_shelves", profile="office", scope=scope)
    assert active["data"]["items"] == []
    state = await call(
        client, "workspace_shelf_state", profile="office", scope=scope, node_id=ids["shelf"]
    )
    assert state["data"] == {
        "workspace_id": workspace,
        "shelf_id": ids["shelf"],
        "revision": 2,
        "archived": True,
    }
    after, seen = None, []
    while True:
        page = await call(
            client,
            "workspace_archived_shelves",
            profile="office",
            scope=scope,
            query={"after": after, "limit": 1},
        )
        assert page["workspace_id"] == workspace
        summary = page["data"]["items"][0]
        assert set(summary) == {"id", "workspace_id", "name", "revision", "created_at"}
        seen.append(summary["id"])
        assert len(seen) <= 2
        after = page["data"]["next_cursor"]
        if after is None:
            break
    assert seen == sorted([ids["shelf"], extra["id"]])
    safe = await call(
        client,
        "workspace_archived_shelves",
        profile="office",
        scope={"workspace_id": workspace, "selection_id": "unused"},
    )
    assert any(item["name"] == "é" * 128 for item in safe["data"]["items"])
    assert all(
        "description" not in item and "content" not in item for item in safe["data"]["items"]
    )
    transition(fixture, ids["shelf"], 2, False)
    after = await call(client, "workspace_entry", profile="office", scope=scope, query=query)
    assert after == before
    state = await call(
        client, "workspace_shelf_state", profile="office", scope=scope, node_id=ids["shelf"]
    )
    assert not state["data"]["archived"] and state["data"]["revision"] == 3
