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
from ccp_mcp_server.workspace_content import (
    BookPage,
    ContentPage,
    ContentScope,
    EntryPage,
    EntryRead,
    HistoryRead,
    WorkspaceContentTools,
)
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
    async def exercise_child(self, code, *, timeout=2, limit=1024, arguments=("info",)):
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
                return await runner.run(*arguments)
            finally:
                self.assertEqual(len(processes), 1)
                self.assertIsNotNone(processes[0].returncode)

    async def test_child_success_uses_json_and_never_returns_stderr(self):
        result = await self.exercise_child(
            "import sys; print('private-path',file=sys.stderr); print('{\"ok\":true}')"
        )
        self.assertEqual(result, {"ok": True})

    async def test_only_fixed_entry_snapshot_argv_gets_four_mib_and_overflow_reaps_child(self):
        snapshot = ("content", "--profile", "/configured/profile", "entry")
        code = "import json; print(json.dumps({'text':'x'*(2*1024*1024)}))"
        result = await self.exercise_child(code, arguments=snapshot)
        self.assertEqual(len(result["text"]), 2 * 1024 * 1024)
        for arguments in [
            ("info",),
            ("content", "--profile", "/configured/profile", "history"),
            ("content", "entry", "--profile", "/configured/profile"),
        ]:
            with self.assertRaises(ToolError):
                await self.exercise_child(code, arguments=arguments)
        with self.assertRaises(ToolError):
            await self.exercise_child(
                "import json; print(json.dumps({'text':'x'*(4*1024*1024)}))",
                arguments=snapshot,
            )

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
                self.assertEqual(len(tools), 15)
                self.assertTrue(
                    {
                        "workspace_profiles",
                        "workspace_profile",
                        "workspace_list",
                        "workspace_selection_open",
                        "workspace_selection",
                        "workspace_select",
                        "workspace_resolve",
                        "workspace_target",
                    }
                    <= {tool.name for tool in tools}
                )
                self.assertTrue(all(tool.name.startswith("workspace_") for tool in tools))
                for tool in tools:
                    properties = tool.inputSchema.get("properties", {})
                    self.assertNotIn("ctx", properties)
                    self.assertNotIn("session_id", properties)
                    self.assertNotIn("profile_path", properties)
                    encoded = json.dumps(tool.inputSchema)
                    for private in ['"profile_path"', '"executable"', '"admin_key"', '"url"']:
                        self.assertNotIn(private, encoded)


class WorkspaceContentTests(unittest.IsolatedAsyncioTestCase):
    def config(self):
        return WorkspaceConfig(Path(sys.executable), {"office": Path("/configured/profile")})

    async def test_rpc_invalid_nested_inputs_refuse_before_spawning(self):
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock()
            async with Client(build_server(runtime)) as client:
                target = str(uuid4())
                for name, arguments in [
                    ("workspace_shelves", {"scope": {"workspace_id": "--profile=/outside"}}),
                    (
                        "workspace_shelves",
                        {"scope": {"workspace_id": target}, "query": {"limit": 33}},
                    ),
                    (
                        "workspace_entry",
                        {
                            "scope": {"workspace_id": target},
                            "query": {"entry_id": target, "revision": 0},
                        },
                    ),
                    (
                        "workspace_books",
                        {
                            "scope": {"workspace_id": target},
                            "query": {"shelf_id": "--node-id=/outside"},
                        },
                    ),
                    (
                        "workspace_entries",
                        {
                            "scope": {"workspace_id": target},
                            "query": {"book_id": target, "after": "--after=evil"},
                        },
                    ),
                ]:
                    result = await client.call_tool(
                        name, {"profile": "office", **arguments}, raise_on_error=False
                    )
                    self.assertTrue(result.is_error)
            runtime.process.run.assert_not_awaited()

    async def test_all_seven_reads_use_fixed_argv_and_label_the_captured_workspace(self):
        workspace, parent, cursor = str(uuid4()), str(uuid4()), str(uuid4())
        scope = ContentScope(workspace_id=workspace)
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock(return_value={"items": []})
            tools = WorkspaceContentTools(runtime)
            calls = [
                (
                    tools.workspace_shelves,
                    ContentPage(cursor, 2),
                    "shelves",
                    ("--limit", "2", "--after", cursor),
                ),
                (
                    tools.workspace_books,
                    BookPage(parent),
                    "books",
                    ("--shelf-id", parent, "--limit", "32"),
                ),
                (tools.workspace_shelf, parent, "shelf", ("--node-id", parent)),
                (tools.workspace_book, parent, "book", ("--node-id", parent)),
                (
                    tools.workspace_entries,
                    EntryPage(parent, True),
                    "entries",
                    ("--book-id", parent, "--limit", "32", "--archived"),
                ),
                (
                    tools.workspace_entry,
                    EntryRead(parent, 2),
                    "entry",
                    ("--entry-id", parent, "--revision", "2"),
                ),
                (
                    tools.workspace_history,
                    HistoryRead(parent, 1, 2),
                    "history",
                    ("--entry-id", parent, "--limit", "2", "--after", "1"),
                ),
            ]
            for method, query, action, args in calls:
                value = await method("office", scope, query)
                self.assertEqual(value, {"workspace_id": workspace, "data": {"items": []}})
                runtime.process.run.assert_awaited_with(
                    "content",
                    "--profile",
                    "/configured/profile",
                    action,
                    "--workspace-id",
                    workspace,
                    *args,
                )
            self.assertEqual(runtime.process.run.await_count, 7)

    async def test_invalid_input_alias_or_scope_does_not_spawn_or_create_a_selection(self):
        workspace = str(uuid4())
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock()
            tools = WorkspaceContentTools(runtime)
            for profile, scope in [
                ("/outside/profile", ContentScope(workspace)),
                ("office", ContentScope()),
                ("office", ContentScope("--profile=/outside")),
                ("office", ContentScope(selection_id="../../outside")),
            ]:
                with self.assertRaises(ToolError):
                    await tools.workspace_shelves(profile, scope)
            for query in [
                ContentPage(None, 0),
                ContentPage(None, 33),
                ContentPage(None, True),
                ContentPage("--after=/outside", 1),
            ]:
                with self.assertRaises(ToolError):
                    await tools.workspace_shelves("office", ContentScope(workspace), query)
            for query in [
                EntryRead("invalid"),
                EntryRead(workspace, 0),
                EntryRead(workspace, 2**63),
                EntryRead(workspace, True),
            ]:
                with self.assertRaises(ToolError):
                    await tools.workspace_entry("office", ContentScope(workspace), query)
            for query in [
                HistoryRead(workspace, 0),
                HistoryRead(workspace, 2**63),
                HistoryRead(workspace, None, 33),
            ]:
                with self.assertRaises(ToolError):
                    await tools.workspace_history("office", ContentScope(workspace), query)
            runtime.process.run.assert_not_awaited()
            self.assertEqual(runtime._selections, {})

    async def test_default_is_captured_and_explicit_override_never_changes_handle_state(self):
        workspace = str(uuid4())
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.selection = AsyncMock(return_value={"workspace": {"id": workspace}})
            runtime.process.run = AsyncMock(return_value={"items": []})
            tools = WorkspaceContentTools(runtime)
            handle = runtime.open_selection()
            result = await tools.workspace_shelves("office", ContentScope(selection_id=handle))
            self.assertEqual(result["workspace_id"], workspace)
            runtime.selection.assert_awaited_once_with(handle, "office", "target")
            runtime.selection.reset_mock()
            override = str(uuid4())
            result = await tools.workspace_shelves("office", ContentScope(override, handle))
            self.assertEqual(result["workspace_id"], override)
            runtime.selection.assert_not_awaited()
            self.assertEqual(runtime._selections, {})

    async def test_malformed_target_and_cli_shapes_fail_without_echo_or_content_fallback(self):
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.selection = AsyncMock()
            runtime.process.run = AsyncMock()
            tools = WorkspaceContentTools(runtime)
            for value in [[], {}, {"workspace": {}}, {"workspace": {"id": "PRIVATE_VALUE"}}]:
                runtime.selection.return_value = value
                with self.assertRaises(ToolError) as error:
                    await tools.workspace_shelves("office", ContentScope(selection_id="handle"))
                self.assertNotIn("PRIVATE_VALUE", str(error.exception))
            runtime.process.run.assert_not_awaited()
            runtime.process.run.return_value = ["PRIVATE_VALUE"]
            with self.assertRaises(ToolError) as error:
                await tools.workspace_shelves("office", ContentScope(str(uuid4())))
            self.assertNotIn("PRIVATE_VALUE", str(error.exception))
