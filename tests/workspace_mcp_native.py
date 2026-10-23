"""Native source records over real MCP stdio, CLI and pinned client mTLS."""

import json
import sqlite3
import time

from mcp_smoke import data


def seed_source(fixture, entry):
    workspace = fixture.setup["workspace_id"]
    with sqlite3.connect(fixture.data / "ccp.sqlite3") as db:
        owner = db.execute(
            "SELECT owner_admin_id FROM ccp_workspaces WHERE id=?", (workspace,)
        ).fetchone()[0]
        db.execute(
            "INSERT INTO ccp_native_imports(workspace_id,source_sha256,source_schema,legacy_session_id,recorded_by,recorded_at) VALUES(?,?,?,?,?,?)",
            (workspace, "a" * 64, 2, 7, owner, int(time.time())),
        )
        for ordinal in range(1, 5):
            row = {
                "columns": ["id", "appended_content", "real", "blob", "text_bytes", "null"],
                "values": [
                    {"type": "integer", "value": str(ordinal)},
                    {"type": "text", "value": "x" * 1_200_000},
                    {"type": "real", "value": "8000000000000000"},
                    {"type": "blob", "value": [0, 127, 255]},
                    {"type": "text_bytes", "value": [0, 255]},
                    {"type": "null"},
                ],
            }
            db.execute(
                "INSERT INTO ccp_native_entry_rows(workspace_id,entry_id,ordinal,source_table,source_key,row_json) VALUES(?,?,?,?,?,?)",
                (
                    workspace,
                    entry,
                    ordinal,
                    "message_history",
                    str(ordinal),
                    json.dumps(row, separators=(",", ":")),
                ),
            )
        row = {
            "columns": ["token_value"],
            "values": [{"type": "text", "value": "PRIVATE_NATIVE_TOKEN"}],
        }
        db.execute(
            "INSERT INTO ccp_native_workspace_rows(workspace_id,ordinal,source_table,source_key,row_json) VALUES(?,1,'auth_tokens','77',?)",
            (workspace, json.dumps(row)),
        )
        db.execute("UPDATE ccp_native_imports SET sealed=1 WHERE workspace_id=?", (workspace,))


async def check_native_reads(client, fixture, ids, scope):
    seed_source(fixture, ids["entry"])
    arguments = {
        "profile": "office",
        "scope": scope,
        "query": {"entry_id": ids["entry"], "limit": 32},
    }
    result = await client.call_tool("workspace_native_history", arguments, raise_on_error=False)
    assert not result.is_error
    first = data(result)
    workspace = fixture.setup["workspace_id"]
    assert first["workspace_id"] == workspace == first["data"]["workspace_id"]
    assert first["data"]["entry_id"] == ids["entry"]
    assert len(first["data"]["items"]) == 3 and first["data"]["next_cursor"] == 3
    assert len(first["data"]["items"][0]["row"]["values"][1]["value"]) == 1_200_000
    assert first["data"]["items"][0]["row"]["values"][2] == {
        "type": "real",
        "value": "8000000000000000",
    }
    assert set(first["data"]["source"]) == {"database_sha256", "schema", "converted_at"}
    assert "PRIVATE_NATIVE_TOKEN" not in json.dumps(first)
    envelopes = [
        json.loads(item.text)["_ccp_guidance"]
        for item in result.content
        if hasattr(item, "text") and "_ccp_guidance" in item.text
    ]
    assert len(envelopes) == 1 and envelopes[0]["workspace_ids"] == [workspace]
    assert envelopes[0]["status"] == "current"
    arguments["query"]["after"] = 3
    tail = data(await client.call_tool("workspace_native_history", arguments))
    assert len(tail["data"]["items"]) == 1 and tail["data"]["items"][0]["ordinal"] == 4
    assert tail["data"]["next_cursor"] is None
    arguments["scope"] = {"workspace_id": workspace}
    explicit = data(await client.call_tool("workspace_native_history", arguments))
    assert explicit == tail
    arguments["scope"] = {"workspace_id": "ffffffff-ffff-4fff-8fff-ffffffffffff"}
    denied = await client.call_tool("workspace_native_history", arguments, raise_on_error=False)
    assert denied.is_error and "PRIVATE_NATIVE_TOKEN" not in str(denied.content)

    print(
        "PASS: native source-history MCP stdio/CLI/TLS, exact typed values, large partial pages, current scope and guidance"
    )
