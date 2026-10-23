"""Guidance projection and pre-SDK boundaries without echoing invalid prose."""

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
from ccp_mcp_server.workspace_guidance import WorkspaceGuidanceTools, guidance
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
from fastmcp import Client
from fastmcp.exceptions import ToolError


def bundle(workspace=None):
    publisher = {
        "actor_kind": "key",
        "actor_id": str(uuid4()),
        "principal_id": str(uuid4()),
        "recorded_at": 100,
    }
    global_board = {
        "scope": {"kind": "global"},
        "revision": 1,
        "text": "global",
        "publisher": publisher,
    }
    local = (
        None
        if workspace is None
        else {
            "scope": {"kind": "workspace", "workspace_id": workspace},
            "revision": 2,
            "text": "local",
            "publisher": publisher,
        }
    )
    return {"precedence": "global_first", "global": global_board, "workspace": local}


class WorkspaceGuidanceTests(unittest.IsolatedAsyncioTestCase):
    def runtime(self):
        return WorkspaceRuntime(
            WorkspaceConfig(Path(sys.executable), {"office": Path("/configured/profile")})
        )

    def test_guidance_known_metadata_and_scope_cannot_be_spoofed_by_prose(self):
        workspace = str(uuid4())
        good = bundle(workspace)
        self.assertEqual(guidance(good, workspace), good)
        bads = []
        for key, value in [("precedence", "workspace_first"), ("workspace", None), ("global", [])]:
            bad = copy.deepcopy(good)
            bad[key] = value
            bads.append(bad)
        for key, value in [
            ("scope", {"kind": "global"}),
            ("scope", {"kind": "workspace", "workspace_id": str(uuid4())}),
            (
                "scope",
                {"kind": "workspace", "workspace_id": workspace, "extra": "PRIVATE_GUIDANCE"},
            ),
            ("revision", True),
            ("revision", -1),
            ("revision", 2**63),
            ("text", "PRIVATE_GUIDANCE" * 1000),
            ("text", "\ud800"),
            ("publisher", None),
        ]:
            bad = copy.deepcopy(good)
            bad["workspace"][key] = value
            bads.append(bad)
        for key, value in [
            ("actor_kind", "credential"),
            ("actor_kind", "admin"),
            ("actor_id", "00000000-0000-0000-0000-000000000000"),
            ("actor_id", "PRIVATE_GUIDANCE"),
            ("principal_id", "bad"),
            ("recorded_at", True),
            ("recorded_at", -1),
            ("recorded_at", 2**63),
        ]:
            bad = copy.deepcopy(good)
            bad["workspace"]["publisher"][key] = value
            bads.append(bad)
        for bad in bads:
            with self.assertRaises(ToolError) as error:
                guidance(bad, workspace)
            self.assertNotIn("PRIVATE_GUIDANCE", str(error.exception))
        with self.assertRaises(ToolError):
            guidance(good, None)
        admin = bundle(workspace)
        admin["global"]["publisher"]["actor_kind"] = "admin"
        admin["global"]["publisher"]["actor_id"] = admin["global"]["publisher"]["principal_id"]
        self.assertEqual(guidance(admin, workspace)["global"]["publisher"]["actor_kind"], "admin")

    def test_guidance_empty_states_bounds_and_additive_fields_are_projected_deliberately(self):
        workspace = str(uuid4())
        for text in ["\0" * 8192, "é" * 4096, ""]:
            value = bundle(workspace)
            value["workspace"]["text"] = text
            value["global"].update(revision=0, text="", publisher=None)
            value["extra"] = value["workspace"]["extra"] = "UNKNOWN_PRIVATE_FIELD"
            value["workspace"]["publisher"]["extra"] = "UNKNOWN_PRIVATE_FIELD"
            projected = guidance(value, workspace)
            self.assertEqual(projected["workspace"]["text"], text)
            self.assertNotIn("UNKNOWN_PRIVATE_FIELD", json.dumps(projected))
        bad = bundle(workspace)
        bad["global"].update(revision=0)
        with self.assertRaises(ToolError):
            guidance(bad, workspace)
        bad["global"].update(text="", publisher=None)
        guidance(bad, workspace)
        bad["global"]["publisher"] = bundle()["global"]["publisher"]
        with self.assertRaises(ToolError):
            guidance(bad, workspace)

    async def test_guidance_captures_default_once_and_keeps_aliases_and_explicit_scope_separate(
        self,
    ):
        workspace, other = str(uuid4()), str(uuid4())
        with self.runtime() as runtime:
            runtime.run = AsyncMock(return_value=bundle(workspace))
            runtime.selection = AsyncMock(return_value={"workspace": {"id": workspace}})
            tools = WorkspaceGuidanceTools(runtime)
            scope = ContentScope(selection_id=runtime.open_selection())
            self.assertEqual(
                (await tools.workspace_guidance("office", scope))["workspace"]["scope"][
                    "workspace_id"
                ],
                workspace,
            )
            runtime.selection.assert_awaited_once()
            runtime.run.assert_awaited_once_with("office", "guidance", "--workspace-id", workspace)
            runtime.run.return_value = bundle(other)
            await tools.workspace_guidance("office", ContentScope(other, scope.selection_id))
            runtime.selection.assert_awaited_once()
            runtime.run.return_value = bundle()
            self.assertIsNone((await tools.workspace_guidance("office"))["workspace"])
            runtime.run.reset_mock()
            with self.assertRaises(ToolError):
                await tools.workspace_guidance("/PRIVATE_GUIDANCE")
            runtime.run.assert_not_awaited()
            runtime.run.return_value = {"global": "PRIVATE_GUIDANCE"}
            with self.assertRaises(ToolError) as caught:
                await tools.workspace_guidance("office")
            self.assertNotIn("PRIVATE_GUIDANCE", str(caught.exception))

    async def test_guidance_rpc_rejects_unknown_claims_and_types_before_sdk_or_children(self):
        with self.runtime() as runtime:
            runtime.run = AsyncMock(return_value=bundle())
            async with Client(build_server(runtime)) as client:
                for arguments in [
                    {"profile": "office", "text": "PRIVATE_GUIDANCE"},
                    {"profile": ["PRIVATE_GUIDANCE"]},
                    {
                        "profile": "office",
                        "scope": {"workspace_id": str(uuid4()), "publisher": "PRIVATE_GUIDANCE"},
                    },
                    {"profile": "office", "scope": {"workspace_id": True}},
                    {"profile": "office", "scope": {}},
                    {"profile": "office", "scope": {"selection_id": "PRIVATE_GUIDANCE"}},
                ]:
                    reply = await client.call_tool(
                        "workspace_guidance", arguments, raise_on_error=False
                    )
                    self.assertTrue(reply.is_error)
                    self.assertNotIn("PRIVATE_GUIDANCE", str(reply))
                runtime.run.assert_not_awaited()
