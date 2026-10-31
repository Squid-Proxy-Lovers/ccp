"""Original source-history boundaries across the workspace MCP adapter."""

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import AsyncMock
from uuid import uuid4

from ccp_mcp_server.workspace import build_server
from ccp_mcp_server.workspace_config import WorkspaceConfig
from ccp_mcp_server.workspace_content import ContentScope
from ccp_mcp_server.workspace_native import NativeHistoryRead, WorkspaceNativeTools, source_history
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
from fastmcp import Client
from fastmcp.exceptions import ToolError


class WorkspaceNativeTests(unittest.IsolatedAsyncioTestCase):
    def config(self):
        return WorkspaceConfig(Path(sys.executable), {"office": Path("/configured/profile")})

    def fixture(self):
        workspace, entry = str(uuid4()), str(uuid4())
        value = {
            "workspace_id": workspace,
            "entry_id": entry,
            "source": {"database_sha256": "a" * 64, "schema": 2, "converted_at": 1},
            "items": [
                {
                    "ordinal": 1,
                    "source_table": "message_history",
                    "source_key": "-42",
                    "row": {
                        "columns": ["id", "appended_content", "real", "blob", "bad_text", "null"],
                        "values": [
                            {"type": "integer", "value": "9223372036854775807"},
                            {"type": "text", "value": "PRIVATE_BODY\n\u0000"},
                            {"type": "real", "value": "8000000000000000"},
                            {"type": "blob", "value": list(range(256))},
                            {"type": "text_bytes", "value": [0, 255]},
                            {"type": "null"},
                        ],
                    },
                }
            ],
            "next_cursor": 1,
        }
        return workspace, NativeHistoryRead(entry), value

    async def test_explicit_and_selected_reads_capture_scope_fixed_argv_and_exact_values(self):
        workspace, query, value = self.fixture()
        expected = copy.deepcopy(value)
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock(return_value=value)
            tools = WorkspaceNativeTools(runtime)
            result = await tools.workspace_native_history("office", ContentScope(workspace), query)
            self.assertEqual(result, {"workspace_id": workspace, "data": expected})
            runtime.process.run.assert_awaited_with(
                "content",
                "--profile",
                "/configured/profile",
                "native-history",
                "--workspace-id",
                workspace,
                "--entry-id",
                query.entry_id,
                "--limit",
                "32",
            )
            runtime.selection = AsyncMock(return_value={"workspace": {"id": workspace}})
            selected = await tools.workspace_native_history(
                "office", ContentScope(selection_id=str(uuid4())), query
            )
            self.assertEqual(selected, result)
            self.assertEqual(runtime.process.run.await_count, 2)

    async def test_native_sdk_rejects_coerced_unknown_and_invalid_inputs_before_spawning(self):
        workspace, query, _value = self.fixture()
        good = {
            "profile": "office",
            "scope": {"workspace_id": workspace},
            "query": {"entry_id": query.entry_id},
        }
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock()
            async with Client(build_server(runtime)) as client:
                self.assertIn(
                    "workspace_native_history", {tool.name for tool in await client.list_tools()}
                )
                for key, bad in [
                    ("after", True),
                    ("after", "1"),
                    ("after", -1),
                    ("after", 2**63),
                    ("limit", 0),
                    ("limit", 33),
                    ("limit", 1.0),
                    ("entry_id", None),
                    ("secret", "PRIVATE_BODY"),
                ]:
                    arguments = copy.deepcopy(good)
                    arguments["query"][key] = bad
                    result = await client.call_tool(
                        "workspace_native_history", arguments, raise_on_error=False
                    )
                    self.assertTrue(result.is_error)
                    self.assertNotIn("PRIVATE_BODY", str(result.content))
            runtime.process.run.assert_not_awaited()

    def test_response_scope_header_types_and_partial_count_cursor_are_strict(self):
        workspace, query, original = self.fixture()
        self.assertEqual(source_history(copy.deepcopy(original), workspace, query), original)
        for change in [
            lambda v: v.update(workspace_id=str(uuid4())),
            lambda v: v.update(entry_id=str(uuid4())),
            lambda v: v.update(source=None),
            lambda v: v["source"].update(recorded_by="PRIVATE_BODY"),
            lambda v: v["source"].update(database_sha256="A" * 64),
            lambda v: v["items"][0].update(source_table="auth_tokens"),
            lambda v: v["items"][0].update(source_key="01"),
            lambda v: v["items"][0].update(ordinal=0),
            lambda v: v.update(next_cursor=2),
            lambda v: v["items"][0]["row"]["values"][0].update(value="+1"),
            lambda v: v["items"][0]["row"]["values"][2].update(value="nan"),
            lambda v: v["items"][0]["row"]["values"][3].update(value=[True]),
            lambda v: v["items"][0]["row"].update(columns=["id", "id"]),
        ]:
            value = copy.deepcopy(original)
            change(value)
            with self.assertRaises(ToolError) as error:
                source_history(value, workspace, query)
            self.assertNotIn("PRIVATE_BODY", str(error.exception))
        empty = copy.deepcopy(original)
        empty.update(source=None, items=[], next_cursor=None)
        self.assertEqual(source_history(empty, workspace, query), empty)
        self.assertEqual(
            NativeHistoryRead(query.entry_id, after=0).arguments()[-2:], ("--after", "0")
        )
        self.assertEqual(json.loads(json.dumps(original)), original)
