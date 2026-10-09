"""Regression tests for CLI boundaries and explicit subscription behavior."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from support import load_unit_server

server = load_unit_server()


class BridgeTests(unittest.TestCase):
    def test_sessions_reads_existing_subscriptions_without_network_refresh(self):
        payload = [{"session_name": "team", "endpoint": "http://chosen:1338"}]
        with (
            patch.object(server, "_load_session_summaries", return_value=payload),
            patch.object(server, "_run_client") as run,
        ):
            self.assertEqual(server.sessions("chosen"), payload)
        run.assert_not_called()

    def test_master_and_health_use_selected_subscription_without_refresh(self):
        selector = "team@http://chosen:1338"
        for function, command in (
            (server.master_instructions, "master-instructions"),
            (server.server_health, "health"),
        ):
            with (
                patch.object(server, "_run_client_json", return_value={"status": "ok"}) as run,
                patch.object(server, "_run_client") as refresh,
            ):
                self.assertEqual(function(selector), {"status": "ok"})
            run.assert_called_once_with(command, "--", selector)
            refresh.assert_not_called()

    def test_discovery_passes_explicit_url(self):
        with patch.object(server, "_run_client_json", return_value=[]) as run:
            self.assertEqual(server.open_topics("http://chosen:1338"), [])
        run.assert_called_once_with("remote-sessions", "--server=http://chosen:1338", "--")

    def test_discovery_master_and_health_reject_wrong_shapes(self):
        for function, arguments, payload in (
            (server.open_topics, (), {}),
            (server.master_instructions, ("team",), []),
            (server.server_health, ("team",), []),
            (server.export_bundle, ("team",), []),
        ):
            with self.subTest(function=function.__name__):
                with patch.object(server, "_run_client_json", return_value=payload):
                    with self.assertRaises(server.CCPClientError):
                        function(*arguments)

    def test_json_boundary_rejects_empty_or_invalid_output(self):
        for output in ("", "not JSON", "{unfinished"):
            with patch.object(server, "_run_client", return_value=output):
                with self.assertRaisesRegex(server.CCPClientError, "invalid JSON"):
                    server._run_client_json("list", "team")

    def test_subprocess_errors_have_actionable_boundary_errors(self):
        command = server.LocalCommand(["client"], "test")
        for error, message in (
            (OSError("not executable"), "unable to run"),
            (subprocess.TimeoutExpired("client", 120), "timed out"),
        ):
            with (
                patch.object(server, "_resolve_client_command", return_value=command),
                patch.object(server.subprocess, "run", side_effect=error) as run,
            ):
                with self.assertRaisesRegex(server.CCPClientError, message):
                    server._run_client("sessions")
            self.assertEqual(run.call_args.kwargs["timeout"], server.CLIENT_TIMEOUT_SECONDS)

    def test_subprocess_keeps_arguments_and_environment_separate(self):
        command = server.LocalCommand(["client"], "test")
        result = subprocess.CompletedProcess([], 0, stdout="{}\n", stderr="")
        with (
            patch.object(server, "_resolve_client_command", return_value=command),
            patch.object(server.subprocess, "run", return_value=result) as run,
            patch.dict(os.environ, {"CCP_AGENT_NAME": "inherited"}),
        ):
            self.assertEqual(server._run_client("append", "team", "entry", "words with spaces", env_overrides={"CCP_AGENT_NAME": "octo"}), "{}")
        self.assertEqual(run.call_args.args[0], ["client", "append", "team", "entry", "words with spaces"])
        self.assertEqual(run.call_args.kwargs["env"]["CCP_AGENT_NAME"], "octo")
        self.assertNotIn("shell", run.call_args.kwargs)

    def test_server_status_works_in_client_only_install(self):
        with (
            patch.object(server, "_resolve_client_command", return_value=server.LocalCommand(["client"], "client")),
            patch.object(server, "_resolve_server_command", side_effect=server.CCPServerError("not installed")),
            patch.object(server, "_load_session_summaries", return_value=[]),
            patch.object(server, "_list_runtime_records", return_value=[]) as listing,
        ):
            result = server.server_status()
        self.assertIsNone(result["server_command"])
        self.assertEqual(result["server_resolution"], "not installed")
        listing.assert_called_once_with()

    def test_optional_warning_failure_cannot_hide_completed_status_update(self):
        with tempfile.TemporaryDirectory() as directory:
            metadata = Path(directory) / "enrollments/broken/metadata.json"
            metadata.parent.mkdir(parents=True)
            metadata.write_text("{broken", encoding="utf-8")
            payload = {"team": "pwn", "agent_name": "octo", "status": "reviewing"}
            with (
                patch.dict(os.environ, {"CCP_CLIENT_HOME": directory}),
                patch.object(server, "_run_client_json", return_value=payload),
            ):
                self.assertEqual(server.set_status("team", "pwn", "octo", "reviewing"), payload)

    def test_warning_selector_respects_endpoint_and_skips_ambiguity(self):
        summaries = [
            {"session_name": "team", "session_id": 1, "endpoint": "http://a:1338", "cert_warning": "first"},
            {"session_name": "team", "session_id": 1, "endpoint": "http://b:1338", "cert_warning": "second"},
        ]
        with (
            patch.object(server, "_load_session_summaries", return_value=summaries),
            patch.dict(os.environ, {}, clear=True),
        ):
            self.assertIsNone(server._session_warning_for_selector("team"))
            self.assertEqual(server._session_warning_for_selector("1@http://b:1338/"), "second")
            with patch.dict(os.environ, {"CCP_SERVER_URL": "http://a:1338"}):
                self.assertEqual(server._session_warning_for_selector("team"), "first")

    def test_arbitrary_text_stays_positional_and_scope_values_stay_intact(self):
        with (
            patch.object(server, "_run_client_json", return_value={}) as run,
            patch.object(server, "_attach_session_warning", side_effect=lambda _session, data: data),
        ):
            server.append_entry("-session", "--entry", "--team is content", shelf_name="--shelf", book_name="-book")
        run.assert_called_once_with(
            "append", "--shelf=--shelf", "--book=-book", "--", "-session", "--entry", "--team is content", env_overrides=None,
        )

    def test_export_keeps_selectors_and_written_path_contract(self):
        with (
            patch.object(server, "_run_client", return_value="-out.droplet\n") as run,
            patch.object(server, "_attach_session_warning", side_effect=lambda _session, data: data),
        ):
            result = server.export_bundle("-session", output_path="-out", shelf="-shelf", book="-book", entries=["--entry"], no_history=True)
        self.assertEqual(result, {"written_path": "-out.droplet"})
        run.assert_called_once_with("export", "--shelf=-shelf", "--book=-book", "--output=-out", "--entry=--entry", "--no-history", "--", "-session")

    def test_time_travel_forwards_empty_scope_for_normal_validation(self):
        with (
            patch.object(server, "_run_client_json", return_value={}) as run,
            patch.object(server, "_attach_session_warning", side_effect=lambda _session, data: data),
        ):
            server.get_entry_at("team", "entry", "2026-10-08T12:00:00Z", shelf_name="", book_name="")
        run.assert_called_once_with("get-entry-at", "--at=2026-10-08T12:00:00Z", "--shelf=", "--book=", "--", "team", "entry")

    def test_cargo_fallback_selects_workspace_package(self):
        for resolver, package, binary in (
            (server._resolve_client_command, "client", "DEFAULT_CLIENT_BINARY"),
            (server._resolve_server_command, "server", "DEFAULT_SERVER_BINARY"),
        ):
            with (
                patch.dict(os.environ, {}, clear=True),
                patch.object(server, binary, Path("/nonexistent/ccp-binary")),
                patch.object(server, f"DEFAULT_{package.upper()}_MANIFEST", Path(__file__)),
                patch.object(server.shutil, "which", side_effect=lambda name: "/cargo" if name == "cargo" else None),
            ):
                argv = resolver().argv
            self.assertEqual(argv[argv.index("--package") + 1], package)


if __name__ == "__main__":
    unittest.main()
