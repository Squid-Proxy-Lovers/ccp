"""Workspace bridge configuration, input boundaries, process cleanup and caller state."""

from __future__ import annotations

import asyncio
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch
from uuid import uuid4

from ccp_mcp_server.workspace import WorkspaceChoice, WorkspaceTools, build_server
from ccp_mcp_server.workspace_config import WorkspaceConfig, load_config
from ccp_mcp_server.workspace_process import WorkspaceProcess
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
from fastmcp import Client
from fastmcp.exceptions import ToolError


class WorkspaceConfigurationTests(unittest.TestCase):
    def write_config(self, root, text):
        path = root / "config.json"
        path.write_text(text)
        path.chmod(0o600)
        return path

    def test_config_requires_explicit_bounded_private_aliases_and_absolute_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = {"format": 1, "profiles": {"office": str(root / "profile")}}
            config = load_config(self.write_config(root, json.dumps(payload)), Path(sys.executable))
            self.assertEqual(config.profiles, {"office": root / "profile"})
            for profiles in [{}, {"../escape": str(root)}, {"office": "relative"}]:
                payload["profiles"] = profiles
                with self.assertRaises(ValueError):
                    load_config(self.write_config(root, json.dumps(payload)), Path(sys.executable))
            with self.assertRaises(ValueError):
                load_config(Path("relative"), Path(sys.executable))

    def test_duplicate_unknown_future_and_oversized_config_refuse(self):
        payloads = [
            '{"format":1,"format":1,"profiles":{}}',
            '{"format":2,"profiles":{}}',
            '{"format":true,"profiles":{}}',
            '{"format":1,"profiles":{},"unknown":1}',
            "x" * (64 * 1024 + 1),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for text in payloads:
                with self.assertRaises(ValueError):
                    load_config(self.write_config(root, text), Path(sys.executable))

    def test_config_refuses_symlinks_and_loose_unix_permissions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = self.write_config(
                root, json.dumps({"format": 1, "profiles": {"office": str(root)}})
            )
            link = root / "link.json"
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                load_config(link, Path(sys.executable))
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                load_config(path, Path(sys.executable))

    def test_choice_validation_prevents_option_path_and_default_injection(self):
        target = str(uuid4())
        choices = [
            WorkspaceChoice(["--selection-dir=/outside"], 0),
            WorkspaceChoice([target, target], 0),
            WorkspaceChoice([target], -1),
            WorkspaceChoice([target], 2**63),
            WorkspaceChoice([str(uuid4()) for _ in range(101)], 0),
            WorkspaceChoice([], 0, target),
        ]
        for choice in choices:
            with self.assertRaises(ToolError):
                choice.arguments()
        self.assertEqual(
            WorkspaceChoice([target], 0, target).arguments(),
            (
                "--expected-revision",
                "0",
                "--workspace-id",
                target,
                "--default-workspace-id",
                target,
            ),
        )


class WorkspaceProcessTests(unittest.IsolatedAsyncioTestCase):
    async def exercise_child(self, code, *, timeout=2, limit=1024):
        processes = []
        original = asyncio.create_subprocess_exec

        async def spawn(*_arguments, **options):
            processes.append(await original(sys.executable, "-c", code, **options))
            return processes[-1]

        runner = WorkspaceProcess(Path(sys.executable), timeout=timeout, output_limit=limit)
        with patch(
            "ccp_mcp_server.workspace_process.asyncio.create_subprocess_exec", side_effect=spawn
        ):
            try:
                return await runner.run("info")
            finally:
                self.assertEqual(len(processes), 1)
                self.assertIsNotNone(processes[0].returncode)

    async def test_child_success_uses_json_and_never_returns_stderr(self):
        result = await self.exercise_child(
            "import sys; print('private-path',file=sys.stderr); print('{\"ok\":true}')"
        )
        self.assertEqual(result, {"ok": True})

    async def test_failure_and_invalid_json_never_echo_child_output(self):
        for code in ["import sys; print('private-path'); sys.exit(7)", "print('private-path')"]:
            with self.assertRaises(ToolError) as error:
                await self.exercise_child(code)
            self.assertNotIn("private-path", str(error.exception))

    async def test_timeout_and_output_overflow_kill_and_reap_the_child(self):
        for code, timeout, limit in [
            ("import time; time.sleep(30)", 0.05, 1024),
            ("import time; print('x'*512,flush=True); time.sleep(30)", 2, 128),
        ]:
            with self.assertRaises(ToolError):
                await self.exercise_child(code, timeout=timeout, limit=limit)

    async def test_cancellation_kills_and_reaps_the_child(self):
        processes = []
        started = asyncio.Event()
        original = asyncio.create_subprocess_exec

        async def spawn(*_arguments, **options):
            processes.append(
                await original(sys.executable, "-c", "import time; time.sleep(30)", **options)
            )
            started.set()
            return processes[-1]

        runner = WorkspaceProcess(Path(sys.executable))
        with patch(
            "ccp_mcp_server.workspace_process.asyncio.create_subprocess_exec", side_effect=spawn
        ):
            task = asyncio.create_task(runner.run("info"))
            await asyncio.wait_for(started.wait(), 2)
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
        self.assertIsNotNone(processes[0].returncode)

    async def test_busy_process_budget_and_spawn_failure_are_bounded_and_sanitized(self):
        runner = WorkspaceProcess(Path("/missing-private-profile/client"))
        runner.slots = asyncio.Semaphore(0)
        with patch.object(runner, "_spawn", new_callable=AsyncMock) as spawn:
            with self.assertRaises(ToolError):
                await runner.run("info")
            spawn.assert_not_awaited()
        runner.slots = asyncio.Semaphore(8)
        with self.assertRaises(ToolError) as error:
            await runner.run("info")
        self.assertNotIn("missing-private-profile", str(error.exception))

    async def test_cancellation_during_launch_still_reaps_the_created_child(self):
        processes = []
        created, release = asyncio.Event(), asyncio.Event()
        original = asyncio.create_subprocess_exec

        async def delayed_launch(*_arguments, **options):
            process = await original(sys.executable, "-c", "import time; time.sleep(30)", **options)
            processes.append(process)
            created.set()
            await release.wait()
            return process

        runner = WorkspaceProcess(Path(sys.executable))
        with patch(
            "ccp_mcp_server.workspace_process.asyncio.create_subprocess_exec",
            side_effect=delayed_launch,
        ):
            task = asyncio.create_task(runner.run("info"))
            await asyncio.wait_for(created.wait(), 2)
            task.cancel()
            release.set()
            with self.assertRaises(asyncio.CancelledError):
                await task
        self.assertIsNotNone(processes[0].returncode)


class WorkspaceSessionTests(unittest.IsolatedAsyncioTestCase):
    def config(self):
        return WorkspaceConfig(Path(sys.executable), {"office": Path("/configured/profile")})

    async def test_unknown_alias_and_page_input_fail_before_spawning(self):
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock()
            tools = WorkspaceTools(runtime)
            with self.assertRaises(ToolError):
                await tools.workspace_profile("/outside/profile")
            for after, limit in [(None, 0), (None, 501), ("x" * 1025, 1), ("\x00", 1)]:
                with self.assertRaises(ToolError):
                    await tools.workspace_list("office", after, limit)
            runtime.process.run.assert_not_awaited()

    async def test_selection_initializes_once_with_internal_paths_and_enforces_handle_budget(self):
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock(return_value={"revision": 0})
            with self.assertRaises(ToolError):
                await runtime.selection("../../not-a-path", "office", "show")
            handle = runtime.open_selection()
            await runtime.selection(handle, "office", "show")
            await runtime.selection(handle, "office", "show")
            self.assertEqual(runtime.process.run.await_count, 3)
            path = runtime.process.run.await_args.args[-1]
            self.assertTrue(Path(path).is_relative_to(runtime.root))
            self.assertNotIn(handle, path)
            for _number in range(63):
                runtime.open_selection()
            with self.assertRaises(ToolError):
                runtime.open_selection()
            generated = runtime.root
        self.assertFalse(generated.exists())

    async def test_protocol_schema_hides_context_paths_and_admin_actions(self):
        with WorkspaceRuntime(self.config()) as runtime:
            async with Client(build_server(runtime)) as client:
                tools = await client.list_tools()
                self.assertEqual(len(tools), 8)
                self.assertTrue(all(tool.name.startswith("workspace_") for tool in tools))
                for tool in tools:
                    properties = tool.inputSchema.get("properties", {})
                    self.assertNotIn("ctx", properties)
                    self.assertNotIn("session_id", properties)
                    self.assertNotIn("profile_path", properties)
