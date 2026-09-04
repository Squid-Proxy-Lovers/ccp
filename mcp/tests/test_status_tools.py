"""Focused tests for the MCP agent-status command wrappers."""

from __future__ import annotations

import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch


class _FakeFastMCP:
    def __init__(self, *_args, **_kwargs):
        pass

    @staticmethod
    def tool():
        return lambda function: function

    @staticmethod
    def resource(_uri):
        return lambda function: function


fake_fastmcp = types.ModuleType("fastmcp")
fake_fastmcp.FastMCP = _FakeFastMCP
sys.modules.setdefault("fastmcp", fake_fastmcp)
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ccp_mcp_server import server  # noqa: E402


class StatusToolTests(unittest.TestCase):
    def test_set_status_invokes_exact_cli_and_attaches_warning(self):
        payload = {"team": "pwn", "agent_name": "octo", "status": "testing parser"}
        with (
            patch.object(server, "_run_client_json", return_value=payload) as run,
            patch.object(
                server,
                "_attach_session_warning",
                return_value={**payload, "warning": "soon"},
            ) as attach,
        ):
            result = server.set_status("ctf", "pwn", "octo", "testing parser")

        run.assert_called_once_with(
            "set-status",
            "ctf",
            "--team",
            "pwn",
            "--agent",
            "octo",
            "testing parser",
        )
        attach.assert_called_once_with("ctf", payload)
        self.assertEqual(result["warning"], "soon")

    def test_clear_status_invokes_exact_cli_and_attaches_warning(self):
        payload = {"team": "pwn", "agent_name": "octo", "cleared": True}
        with (
            patch.object(server, "_run_client_json", return_value=payload) as run,
            patch.object(server, "_attach_session_warning", return_value=payload) as attach,
        ):
            self.assertIs(server.clear_status("ctf", "pwn", "octo"), payload)

        run.assert_called_once_with(
            "clear-status", "ctf", "--team", "pwn", "--agent", "octo"
        )
        attach.assert_called_once_with("ctf", payload)

    def test_list_team_status_invokes_exact_cli(self):
        payload = [{"team": "pwn", "agent_name": "octo", "status": "testing"}]
        with patch.object(server, "_run_client_json", return_value=payload) as run:
            self.assertIs(server.list_team_status("ctf", "pwn"), payload)

        run.assert_called_once_with("team-status", "ctf", "--team", "pwn")

    def test_search_team_status_invokes_exact_cli(self):
        payload = [{"team": "pwn", "agent_name": "octo", "status": "parser"}]
        with patch.object(server, "_run_client_json", return_value=payload) as run:
            self.assertIs(
                server.search_team_status("ctf", "pwn", "parser"), payload
            )

        run.assert_called_once_with(
            "search-team-status", "ctf", "--team", "pwn", "parser"
        )

    def test_object_tools_reject_non_object_payloads(self):
        with patch.object(server, "_run_client_json", return_value=[]):
            with self.assertRaisesRegex(server.CCPClientError, "set-status"):
                server.set_status("ctf", "pwn", "octo", "testing")
            with self.assertRaisesRegex(server.CCPClientError, "clear-status"):
                server.clear_status("ctf", "pwn", "octo")

    def test_list_tools_reject_non_list_payloads(self):
        with patch.object(server, "_run_client_json", return_value={}):
            with self.assertRaisesRegex(server.CCPClientError, "team-status"):
                server.list_team_status("ctf", "pwn")
            with self.assertRaisesRegex(server.CCPClientError, "search-team-status"):
                server.search_team_status("ctf", "pwn", "parser")


if __name__ == "__main__":
    unittest.main()
