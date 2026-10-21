"""Workspace bridge configuration, input boundaries, process cleanup and caller state."""

from __future__ import annotations

import asyncio
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
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
from ccp_mcp_server.workspace_validation import WorkspaceValidationMiddleware
from ccp_mcp_server.workspace_write import (
    BookDraft,
    ShelfDraft,
    WorkspaceWriteTools,
)
from fastmcp import Client
from fastmcp.exceptions import ToolError, ValidationError
from workspace_delivery_tests import WorkspaceDeliveryTests as WorkspaceDeliveryTests
from workspace_entry_tests import WorkspaceEntryTests as WorkspaceEntryTests
from workspace_guidance_tests import WorkspaceGuidanceTests as WorkspaceGuidanceTests
from workspace_status_tests import WorkspaceStatusTests as WorkspaceStatusTests


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
                self.assertEqual(len(tools), 31)
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
                        "workspace_guidance",
                        "workspace_status_messages",
                        "workspace_status_message",
                        "workspace_update_status",
                        "workspace_clear_status",
                        "workspace_status_pending",
                        "workspace_status_resume",
                        "workspace_status_forget",
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

    async def test_shelf_lifecycle_readers_capture_scope_and_use_only_fixed_read_commands(self):
        workspace, node = str(uuid4()), str(uuid4())
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.selection = AsyncMock(return_value={"workspace": {"id": workspace}})
            runtime.process.run = AsyncMock(return_value={"revision": 2, "archived": True})
            tools = WorkspaceContentTools(runtime)
            result = await tools.workspace_shelf_state(
                "office", ContentScope(selection_id="handle"), node
            )
            self.assertEqual(result["workspace_id"], workspace)
            runtime.selection.assert_awaited_once_with("handle", "office", "target")
            runtime.process.run.assert_awaited_once_with(
                "content",
                "--profile",
                "/configured/profile",
                "shelf-state",
                "--workspace-id",
                workspace,
                "--node-id",
                node,
            )
            runtime.selection.reset_mock()
            runtime.process.run.reset_mock()
            runtime.process.run.return_value = {"items": [], "next_cursor": None}
            result = await tools.workspace_archived_shelves(
                "office", ContentScope(workspace, "unused"), ContentPage(node, 1)
            )
            self.assertEqual(
                result, {"workspace_id": workspace, "data": {"items": [], "next_cursor": None}}
            )
            runtime.selection.assert_not_awaited()
            runtime.process.run.assert_awaited_once_with(
                "content",
                "--profile",
                "/configured/profile",
                "archived-shelves",
                "--workspace-id",
                workspace,
                "--limit",
                "1",
                "--after",
                node,
            )
            self.assertEqual(runtime._selections, {})

    async def test_shelf_reader_bad_targets_aliases_and_pages_refuse_before_children(self):
        workspace, node = str(uuid4()), str(uuid4())
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock()
            tools = WorkspaceContentTools(runtime)
            for profile, scope in [
                ("/outside", ContentScope(workspace)),
                ("office", ContentScope()),
                ("office", ContentScope("invalid")),
            ]:
                with self.assertRaises(ToolError):
                    await tools.workspace_shelf_state(profile, scope, node)
                with self.assertRaises(ToolError):
                    await tools.workspace_archived_shelves(profile, scope)
            with self.assertRaises(ToolError):
                await tools.workspace_shelf_state(
                    "office", ContentScope(workspace), "--node-id=/outside"
                )
            for query in [
                ContentPage(None, 0),
                ContentPage(None, 33),
                ContentPage(None, True),
                ContentPage("invalid", 1),
            ]:
                with self.assertRaises(ToolError):
                    await tools.workspace_archived_shelves("office", ContentScope(workspace), query)
            runtime.process.run.assert_not_awaited()
            runtime.process.run.return_value = ["PRIVATE_BODY"]
            with self.assertRaises(ToolError) as caught:
                await tools.workspace_shelf_state("office", ContentScope(workspace), node)
            self.assertNotIn("PRIVATE_BODY", str(caught.exception))


class WorkspaceWriteTests(unittest.IsolatedAsyncioTestCase):
    def config(self):
        return WorkspaceConfig(Path(sys.executable), {"office": Path("/configured/profile")})

    async def test_validation_hook_redacts_sdk_errors_but_keeps_cancellation_and_tool_errors(self):
        middleware = WorkspaceValidationMiddleware()
        context = SimpleNamespace(message=SimpleNamespace(name="workspace_profiles", arguments={}))
        for request in [
            {"workspace_id": str(uuid4()), "name": "valid", "owner_client_id": "PRIVATE_BODY"},
            {"workspace_id": str(uuid4()), "name": 1, "description": "PRIVATE_BODY"},
            {"workspace_id": str(uuid4()), "name": "valid", "description": None},
        ]:
            invalid = SimpleNamespace(
                message=SimpleNamespace(
                    name="workspace_create_shelf",
                    arguments={"profile": "office", "request": request},
                )
            )
            next_call = AsyncMock()
            with self.assertRaises(ToolError) as caught:
                await middleware.on_call_tool(invalid, next_call)
            self.assertNotIn("PRIVATE_BODY", str(caught.exception))
            next_call.assert_not_awaited()
        for error in [ValueError("PRIVATE_BODY"), ValidationError("PRIVATE_BODY")]:
            next_call = AsyncMock(side_effect=error)
            with self.assertRaises(ToolError) as caught:
                await middleware.on_call_tool(context, next_call)
            self.assertNotIn("PRIVATE_BODY", str(caught.exception))
            next_call.assert_awaited_once_with(context)
        intentional = ToolError("safe static message")
        with self.assertRaises(ToolError) as caught:
            await middleware.on_call_tool(context, AsyncMock(side_effect=intentional))
        self.assertIs(caught.exception, intentional)
        with self.assertRaises(asyncio.CancelledError):
            await middleware.on_call_tool(context, AsyncMock(side_effect=asyncio.CancelledError))
        self.assertEqual(
            await middleware.on_call_tool(context, AsyncMock(return_value={"ok": True})),
            {"ok": True},
        )

    async def test_explicit_payload_uses_private_generated_path_without_data_in_argv(self):
        workspace, parent = str(uuid4()), str(uuid4())
        with WorkspaceRuntime(self.config()) as runtime:
            captured = []

            async def spawn(arguments):
                path = Path(arguments[-1])
                self.assertEqual(
                    arguments[:-2], ("content", "--profile", "/configured/profile", "create-book")
                )
                self.assertEqual(arguments[-2], "--request-file")
                self.assertTrue(path.is_relative_to(runtime.root))
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
                self.assertNotIn("PRIVATE_BODY", " ".join(arguments))
                captured.append(path)
                return json.loads(path.read_text())

            runtime.process._spawn = AsyncMock(side_effect=spawn)
            result = await WorkspaceWriteTools(runtime).workspace_create_book(
                "office", BookDraft(workspace, parent, " notes ", "PRIVATE_BODY\0é")
            )
            self.assertEqual(result["workspace_id"], workspace)
            self.assertEqual(result["data"]["shelf_id"], parent)
            self.assertEqual(result["data"]["description"], "PRIVATE_BODY\0é")
            self.assertTrue(captured)
            self.assertFalse(captured[0].exists())
            self.assertEqual(list(runtime.root.glob("request-*.json")), [])
            self.assertEqual(runtime._selections, {})

    async def test_invalid_or_unknown_alias_refuses_before_staging_and_preserves_scope(self):
        workspace = str(uuid4())
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run_input = AsyncMock()
            tools = WorkspaceWriteTools(runtime)
            for profile, request in [
                ("/outside", ShelfDraft(workspace, "valid")),
                ("office", ShelfDraft("--profile=/outside", "valid")),
                ("office", ShelfDraft(workspace, " \n ")),
                ("office", ShelfDraft(workspace, "bad\nname")),
                ("office", ShelfDraft(workspace, "é" * 129)),
                ("office", ShelfDraft(workspace, "valid", "x" * 4097)),
                ("office", ShelfDraft(workspace, "valid", "\ud800")),
                ("office", ShelfDraft(workspace, " " * (64 * 1024) + "n")),
                ("office", ShelfDraft(workspace, "\t" * 40000 + "n")),
            ]:
                with self.assertRaises(ToolError):
                    await tools.workspace_create_shelf(profile, request)
            with self.assertRaises(ToolError):
                await tools.workspace_create_book(
                    "office", BookDraft(workspace, "invalid", "valid")
                )
            runtime.process.run_input.assert_not_awaited()
            self.assertEqual(list(runtime.root.iterdir()), [])

    async def test_unknown_rpc_fields_fail_without_spawning_or_echoing_input_values(self):
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run_input = AsyncMock()
            async with Client(build_server(runtime)) as client:
                for claim in ["profile_path", "owner_client_id", "admin_key", "operation_id"]:
                    result = await client.call_tool(
                        "workspace_create_shelf",
                        {
                            "profile": "office",
                            "request": {
                                "workspace_id": str(uuid4()),
                                "name": "valid",
                                claim: "PRIVATE_BODY",
                            },
                        },
                        raise_on_error=False,
                    )
                    self.assertTrue(result.is_error)
                    text = "".join(item.text for item in result.content if hasattr(item, "text"))
                    self.assertNotIn("PRIVATE_BODY", text)
            runtime.process.run_input.assert_not_awaited()

    async def test_child_errors_are_uncertain_without_retry_path_or_body_echo(self):
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run_input = AsyncMock(
                side_effect=ToolError("PRIVATE_BODY /private/path")
            )
            with self.assertRaises(ToolError) as error:
                await WorkspaceWriteTools(runtime).workspace_create_shelf(
                    "office", ShelfDraft(str(uuid4()), "valid")
                )
            self.assertIn("inspect shelves/books", str(error.exception))
            self.assertNotIn("PRIVATE_BODY", str(error.exception))
            runtime.process.run_input.assert_awaited_once()


class WorkspacePrivateInputTests(unittest.IsolatedAsyncioTestCase):
    async def exercise(self, code, *, timeout=2, limit=1024):
        original = asyncio.create_subprocess_exec
        processes, files = [], []
        with tempfile.TemporaryDirectory() as directory:
            runner = WorkspaceProcess(Path(sys.executable), timeout=timeout, output_limit=limit)

            async def spawn(*arguments, **options):
                path = Path(arguments[-1])
                files.append(path)
                self.assertEqual(json.loads(path.read_text()), {"name": "PRIVATE_BODY"})
                process = await original(sys.executable, "-c", code, str(path), **options)
                processes.append(process)
                return process

            with patch(
                "ccp_mcp_server.workspace_process.asyncio.create_subprocess_exec", side_effect=spawn
            ):
                try:
                    return await runner.run_input(
                        Path(directory),
                        b'{"name":"PRIVATE_BODY"}',
                        ("content", "--profile", "/profile", "create-shelf"),
                    )
                finally:
                    self.assertEqual(len(processes), 1)
                    self.assertIsNotNone(processes[0].returncode)
                    self.assertTrue(files)
                    self.assertFalse(files[0].exists())
                    self.assertEqual(runner.slots._value, 8)

    async def test_file_is_closed_readable_by_child_and_removed_on_every_exit(self):
        value = await self.exercise(
            "import json,sys; assert json.load(open(sys.argv[1]))['name']=='PRIVATE_BODY'; print('{\"ok\":true}')"
        )
        self.assertEqual(value, {"ok": True})
        for code, timeout, limit in [
            ("import sys; print('PRIVATE_BODY'); sys.exit(7)", 2, 1024),
            ("print('PRIVATE_BODY')", 2, 1024),
            ("import time; time.sleep(30)", 0.05, 1024),
            ("import time; print('x'*512,flush=True); time.sleep(30)", 2, 128),
        ]:
            with self.assertRaises(ToolError) as error:
                await self.exercise(code, timeout=timeout, limit=limit)
            self.assertNotIn("PRIVATE_BODY", str(error.exception))

    async def test_no_file_before_slot_and_spawn_or_staging_failure_cleans_up(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = WorkspaceProcess(Path("/private-missing-client"))
            runner.slots = asyncio.Semaphore(0)
            with patch("ccp_mcp_server.workspace_process.private_request") as stage:
                with self.assertRaises(ToolError):
                    await runner.run_input(root, b"{}", ("content",))
                stage.assert_not_called()
            runner.slots = asyncio.Semaphore(8)
            with self.assertRaises(ToolError) as error:
                await runner.run_input(root, b"{}", ("content",))
            self.assertNotIn("private-missing-client", str(error.exception))
            self.assertEqual(list(root.iterdir()), [])
            with patch(
                "ccp_mcp_server.workspace_request.tempfile.mkstemp",
                side_effect=OSError("/PRIVATE_BODY"),
            ):
                with self.assertRaises(ToolError) as error:
                    await runner.run_input(root, b"{}", ("content",))
                self.assertNotIn("PRIVATE_BODY", str(error.exception))
            self.assertEqual(runner.slots._value, 8)
            for payload in [b"", b"x" * (2 * 1024 * 1024 + 1)]:
                with self.assertRaises(ToolError):
                    await runner.run_input(root, payload, ("content",))
            self.assertEqual(list(root.iterdir()), [])

    async def test_delayed_launch_cancellation_reaps_child_before_deleting_request(self):
        original = asyncio.create_subprocess_exec
        created, release = asyncio.Event(), asyncio.Event()
        processes, files = [], []
        with tempfile.TemporaryDirectory() as directory:
            runner = WorkspaceProcess(Path(sys.executable))

            async def spawn(*arguments, **options):
                files.append(Path(arguments[-1]))
                processes.append(
                    await original(sys.executable, "-c", "import time; time.sleep(30)", **options)
                )
                created.set()
                await release.wait()
                return processes[-1]

            with patch(
                "ccp_mcp_server.workspace_process.asyncio.create_subprocess_exec", side_effect=spawn
            ):
                task = asyncio.create_task(runner.run_input(Path(directory), b"{}", ("content",)))
                await asyncio.wait_for(created.wait(), 2)
                task.cancel()
                self.assertTrue(files[0].exists())
                release.set()
                with self.assertRaises(asyncio.CancelledError):
                    await task
            self.assertIsNotNone(processes[0].returncode)
            self.assertFalse(files[0].exists())
            self.assertEqual(runner.slots._value, 8)

    async def test_eight_slots_bound_staged_files_and_cancelled_waiter_creates_none(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = WorkspaceProcess(Path(sys.executable))
            all_started, release = asyncio.Event(), asyncio.Event()
            files = []

            async def spawn(arguments):
                files.append(Path(arguments[-1]))
                if len(files) == 8:
                    all_started.set()
                await release.wait()
                return {"ok": True}

            runner._spawn = AsyncMock(side_effect=spawn)
            tasks = [
                asyncio.create_task(runner.run_input(root, b"{}", ("content",)))
                for _number in range(8)
            ]
            await asyncio.wait_for(all_started.wait(), 2)
            extra = asyncio.create_task(runner.run_input(root, b"{}", ("content",)))
            await asyncio.sleep(0)
            self.assertEqual(len(list(root.glob("request-*.json"))), 8)
            extra.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await extra
            release.set()
            self.assertEqual(await asyncio.gather(*tasks), [{"ok": True}] * 8)
            self.assertEqual(list(root.iterdir()), [])
            self.assertEqual(runner.slots._value, 8)

    async def test_read_cancellation_cleans_only_after_child_reap_and_cleanup_failure_is_static(
        self,
    ):
        original = asyncio.create_subprocess_exec
        reading = asyncio.Event()
        processes, files = [], []
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = WorkspaceProcess(Path(sys.executable))
            original_read = runner._read

            async def spawn(*arguments, **options):
                files.append(Path(arguments[-1]))
                processes.append(
                    await original(sys.executable, "-c", "import time; time.sleep(30)", **options)
                )
                return processes[-1]

            async def read(process, limit):
                reading.set()
                return await original_read(process, limit)

            with (
                patch(
                    "ccp_mcp_server.workspace_process.asyncio.create_subprocess_exec",
                    side_effect=spawn,
                ),
                patch.object(runner, "_read", side_effect=read),
            ):
                task = asyncio.create_task(runner.run_input(root, b"{}", ("content",)))
                await asyncio.wait_for(reading.wait(), 2)
                self.assertTrue(files[0].exists())
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
            self.assertIsNotNone(processes[0].returncode)
            self.assertFalse(files[0].exists())
            runner._spawn = AsyncMock(return_value={"ok": True})
            with patch(
                "ccp_mcp_server.workspace_request.Path.unlink", side_effect=OSError("PRIVATE_BODY")
            ):
                with self.assertRaises(ToolError) as error:
                    await runner.run_input(root, b"{}", ("content",))
                self.assertNotIn("PRIVATE_BODY", str(error.exception))
            leftovers = list(root.glob("request-*.json"))
            self.assertEqual(len(leftovers), 1)
            self.assertEqual(leftovers[0].stat().st_mode & 0o777, 0o600)
            self.assertEqual(runner.slots._value, 8)
