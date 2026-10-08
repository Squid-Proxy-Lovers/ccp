"""Exercise actual FastMCP registration, validation, tools, and resources."""

import importlib.util
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

HAS_FASTMCP = importlib.util.find_spec("fastmcp") is not None
if HAS_FASTMCP:
    from fastmcp import Client
    from ccp_mcp_server import server

EXPECTED_TOOLS = {
    "server_status", "open_topics", "subscribe", "sessions", "master_instructions",
    "list_entries", "find_entries", "find_shelves", "find_books", "search_context",
    "search_deleted_entries", "set_status", "clear_status", "list_team_status",
    "search_team_status", "get_entry", "add_shelf", "add_book", "add_entry",
    "append_entry", "get_history", "export_bundle", "server_health", "brief_me",
    "get_entry_at",
}


@unittest.skipUnless(HAS_FASTMCP, "install mcp package to run actual FastMCP contract tests")
class MCPContractTests(unittest.IsolatedAsyncioTestCase):
    async def test_registered_surface_and_documentation_match(self):
        async with Client(server.mcp) as client:
            tools = await client.list_tools()
            resources = await client.list_resources()
            templates = await client.list_resource_templates()
        self.assertEqual({tool.name for tool in tools}, EXPECTED_TOOLS)
        self.assertEqual({str(resource.uri) for resource in resources}, {"ccp://help", "ccp://sessions"})
        self.assertEqual({getattr(template, "uri_template", None) or template.uriTemplate for template in templates}, {"ccp://master/{session}"})
        status = next(tool for tool in tools if tool.name == "set_status")
        schema = getattr(status, "input_schema", None) or status.inputSchema
        self.assertEqual(set(schema["required"]), {"session", "team", "agent_name", "status"})

    async def test_tool_reference_matches_registration(self):
        reference = Path(__file__).resolve().parents[2] / "docs/tool-call-api.md"
        if not reference.exists():
            self.skipTest("repository tool reference is outside the packaged source distribution")
        api = reference.read_text()
        for name in EXPECTED_TOOLS:
            self.assertIn(f"### `{name}`", api)
        self.assertNotIn("### `enroll`", api)

    async def test_status_tool_roundtrip_uses_cli_contract(self):
        payload = {"team": "pwn", "agent_name": "octo", "status": "reviewing", "worker_id": "http-client", "updated_at": "1", "expires_at": "2"}
        with (
            patch.object(server, "_run_client_json", return_value=payload) as run,
            patch.object(server, "_attach_session_warning", side_effect=lambda _session, data: data),
        ):
            async with Client(server.mcp) as client:
                result = await client.call_tool("set_status", {"session": "ctf", "team": "pwn", "agent_name": "octo", "status": "reviewing"})
        run.assert_called_once_with("set-status", "--team=pwn", "--agent=octo", "--", "ctf", "reviewing")
        self.assertFalse(result.is_error)
        self.assertEqual(json.loads(result.content[0].text), payload)

    async def test_resources_read_same_contract_without_subscribing(self):
        sessions = [{"session_name": "ctf", "session_id": 1, "endpoint": "http://chosen:1338"}]
        boards = {"global": {"content": "Global", "updated_at": "1"}, "session": {"content": "Session", "updated_at": "2"}}
        with (
            patch.object(server, "_load_session_summaries", return_value=sessions),
            patch.object(server, "_run_client_json", return_value=boards) as run,
            patch.object(server, "_run_client") as subscribe,
        ):
            async with Client(server.mcp) as client:
                saved = await client.read_resource("ccp://sessions")
                master = await client.read_resource("ccp://master/ctf")
        self.assertEqual(json.loads(saved[0].text), sessions)
        self.assertEqual(json.loads(master[0].text), boards)
        run.assert_called_once_with("master-instructions", "--", "ctf")
        subscribe.assert_not_called()

    async def test_master_resource_decodes_endpoint_qualified_selector(self):
        with patch.object(server, "_run_client_json", return_value={}) as run:
            async with Client(server.mcp) as client:
                result = await client.read_resource("ccp://master/team%40http%3A%2F%2Fchosen%3A1338")
        self.assertEqual(json.loads(result[0].text), {})
        run.assert_called_once_with("master-instructions", "--", "team@http://chosen:1338")

    async def test_invalid_tool_input_is_rejected_before_cli(self):
        with patch.object(server, "_run_client_json") as run:
            async with Client(server.mcp) as client:
                result = await client.call_tool("set_status", {"session": "ctf", "team": "pwn", "agent_name": "octo"}, raise_on_error=False)
        self.assertTrue(result.is_error)
        run.assert_not_called()

    async def test_client_error_becomes_mcp_error_instead_of_empty_success(self):
        with patch.object(server, "_run_client_json", side_effect=server.CCPClientError("client returned invalid JSON")):
            async with Client(server.mcp) as client:
                result = await client.call_tool("master_instructions", {"session": "ctf"}, raise_on_error=False)
        self.assertTrue(result.is_error)
        self.assertIn("invalid JSON", result.content[0].text)


if __name__ == "__main__":
    unittest.main()
