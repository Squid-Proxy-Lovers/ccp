"""Automatic guidance preserves primary outcomes, caller isolation and bounded delivery."""

from __future__ import annotations

import asyncio
import copy
import json
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch
from uuid import uuid4

from ccp_mcp_server.workspace import build_server
from ccp_mcp_server.workspace_config import WorkspaceConfig
from ccp_mcp_server.workspace_delivery import WorkspaceDeliveryMiddleware
from ccp_mcp_server.workspace_delivery_values import delivery, encoded, targets
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
from fastmcp import Client
from fastmcp.exceptions import ToolError
from mcp.types import TextContent
from workspace_guidance_tests import bundle


class WorkspaceDeliveryTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.runtime = WorkspaceRuntime(
            WorkspaceConfig(Path(sys.executable), {"office": Path("/configured/profile")})
        )
        self.addCleanup(self.runtime.__exit__, None, None, None)
        self.middleware = WorkspaceDeliveryMiddleware(self.runtime)
        self.handle = self.runtime.open_selection()
        self.workspace, self.other = str(uuid4()), str(uuid4())
        self.context = {
            "instance_id": str(uuid4()),
            "client_id": str(uuid4()),
            "credential_id": str(uuid4()),
            "workspace_grant_id": str(uuid4()),
        }
        self.base = bundle(self.workspace)
        self.runtime.run = AsyncMock(side_effect=self.read)

    async def read(self, _profile, *arguments):
        workspace = arguments[-1] if "--workspace-id" in arguments else None
        boards = copy.deepcopy(self.base)
        if workspace is None:
            boards["workspace"] = None
        else:
            boards["workspace"]["scope"]["workspace_id"] = workspace
        context = dict(self.context)
        if workspace is None:
            context["workspace_grant_id"] = None
        return {"format": 1, "context": context, "guidance": boards}

    async def invoke(self, name="workspace_shelves", payload=None, arguments=None):
        if payload is None:
            payload = {"workspace_id": self.workspace, "data": {"items": []}}
        if arguments is None:
            arguments = {"profile": "office", "scope": {"selection_id": self.handle}}
        result = SimpleNamespace(
            structured_content=payload,
            content=[TextContent(type="text", text=json.dumps(payload))],
            meta={"unchanged": True},
            is_error=False,
        )
        original_text = result.content[0]
        next_call = AsyncMock(return_value=result)
        context = SimpleNamespace(message=SimpleNamespace(name=name, arguments=arguments))
        returned = await self.middleware.on_call_tool(context, next_call)
        self.assertIs(returned, result)
        self.assertIs(returned.structured_content, payload)
        self.assertIs(returned.content[0], original_text)
        self.assertFalse(returned.is_error)
        self.assertEqual(returned.meta, {"unchanged": True})
        next_call.assert_awaited_once_with(context)
        if len(result.content) == 1:
            return None
        self.assertEqual(len(result.content), 2)
        return json.loads(result.content[-1].text)["_ccp_guidance"]

    async def test_initial_unchanged_changed_and_cleared_boards_with_fresh_reads(self):
        first = await self.invoke()
        self.assertEqual(len(first["boards"]), 2)
        self.assertEqual(first["status"], "current")
        self.assertEqual((await self.invoke())["boards"], [])
        self.assertEqual(self.runtime.run.await_count, 2)
        self.base["global"].update(revision=2, text="use UTC")
        changed = await self.invoke()
        self.assertEqual(len(changed["boards"]), 1)
        self.assertEqual(changed["boards"][0]["board"]["text"], "use UTC")
        self.base["workspace"].update(revision=3, text="")
        cleared = await self.invoke()
        self.assertEqual(cleared["boards"][0]["board"]["text"], "")
        self.assertEqual(cleared["boards"][0]["board"]["revision"], 3)
        self.assertNotIn("use UTC", repr(self.middleware._stamps))
        self.runtime.run.assert_awaited_with(
            "office", "guidance", "--delivery-context", "--workspace-id", self.workspace
        )

    async def test_independent_shared_stateless_and_unknown_handles_do_not_leak_suppression(self):
        await self.invoke()
        other_handle = self.runtime.open_selection()
        independent = {"profile": "office", "scope": {"selection_id": other_handle}}
        self.assertEqual(len((await self.invoke(arguments=independent))["boards"]), 2)
        self.assertEqual((await self.invoke())["boards"], [])
        for handle in [None, str(uuid4())]:
            args = {"profile": "office", "scope": {"workspace_id": self.workspace}}
            if handle:
                args["scope"]["selection_id"] = handle
            for _repeat in range(2):
                self.assertEqual(len((await self.invoke(arguments=args))["boards"]), 2)
        self.assertEqual(self.runtime._selections, {})
        self.assertEqual(len(self.runtime._handles), 2)

    async def test_new_target_grant_credential_and_equal_or_lower_revision_emit_current_text(self):
        await self.invoke()
        self.context["workspace_grant_id"] = str(uuid4())
        self.assertEqual(len((await self.invoke())["boards"]), 1)
        self.context["credential_id"] = str(uuid4())
        self.assertEqual(len((await self.invoke())["boards"]), 2)
        for revision, text in [(2, "same revision replacement"), (1, "restored lower revision")]:
            self.base["workspace"].update(revision=revision, text=text)
            changed = await self.invoke()
            self.assertEqual(changed["boards"][0]["board"]["text"], text)
        changed = await self.invoke(payload={"workspace_id": self.other, "data": {}})
        self.assertEqual(changed["boards"][0]["board"]["scope"]["workspace_id"], self.other)

    async def test_local_and_explicit_refresh_tools_do_not_fetch_or_advance_stamps(self):
        for name in ["workspace_profiles", "workspace_selection_open", "workspace_guidance"]:
            self.assertIsNone(await self.invoke(name=name))
        self.runtime.run.assert_not_awaited()
        self.assertEqual(self.middleware._stamps, {})
        current = await self.invoke(name="workspace_profile", payload={"client_id": "metadata"})
        self.assertEqual(len(current["boards"]), 1)
        self.runtime.run.assert_awaited_once_with("office", "guidance", "--delivery-context")
        self.assertEqual(current["workspace_ids"], [])

    async def test_captured_default_and_selection_result_scopes_are_not_reresolved(self):
        self.runtime.selection = AsyncMock(side_effect=AssertionError("must not retarget"))
        result = await self.invoke(payload={"workspace_id": self.other, "data": {}})
        self.assertEqual(result["workspace_ids"], [self.other])
        for name in ["workspace_selection", "workspace_select"]:
            result = await self.invoke(
                name=name, payload={"workspace_ids": [self.workspace, self.other]}
            )
            self.assertEqual(result["workspace_ids"], [self.workspace, self.other])
        result = await self.invoke(
            name="workspace_resolve",
            payload={"targets": [{"workspace": {"id": self.other}}]},
        )
        self.assertEqual(result["workspace_ids"], [self.other])
        self.runtime.selection.assert_not_awaited()
        self.assertEqual(
            targets("workspace_target", {"workspace": {"id": self.workspace}}), [self.workspace]
        )
        for payload in [
            {"workspace_ids": [self.workspace] * 2},
            {"workspace_ids": [str(uuid4()) for _i in range(101)]},
        ]:
            with self.assertRaises(ToolError):
                targets("workspace_selection", payload)

    async def test_current_revocation_or_malformed_context_returns_warning_without_cached_prose(
        self,
    ):
        await self.invoke()
        self.base["workspace"].update(revision=3, text="new private guidance")
        before = dict(self.middleware._stamps)
        for failure in [ToolError("PRIVATE_GUIDANCE"), {"format": 99}, None]:
            self.runtime.run.side_effect = failure if isinstance(failure, ToolError) else None
            self.runtime.run.return_value = failure
            warning = await self.invoke()
            self.assertEqual(warning["status"], "unavailable")
            self.assertEqual(warning["boards"], [])
            self.assertIn("may be stale", warning["warning"])
            self.assertNotIn("PRIVATE_GUIDANCE", json.dumps(warning))
            self.assertEqual(self.middleware._stamps, before)
        self.runtime.run.side_effect = self.read
        self.assertEqual(
            (await self.invoke())["boards"][0]["board"]["text"], "new private guidance"
        )

    async def test_successful_mutation_is_not_retried_or_failed_by_guidance_failure(self):
        self.runtime.run.side_effect = ToolError("PRIVATE_BODY")
        primary = {
            "workspace_id": self.workspace,
            "data": {"revision": 1},
            "target": {"operation_id": str(uuid4())},
        }
        warning = await self.invoke(
            name="workspace_mutate_entry",
            payload=primary,
            arguments={"profile": "office", "request": {"workspace_id": self.workspace}},
        )
        self.assertEqual(warning["status"], "unavailable")
        self.assertIn("do not retry a mutation", warning["warning"])
        self.runtime.run.assert_awaited_once()
        self.assertEqual(primary["data"]["revision"], 1)
        self.assertEqual(self.middleware._stamps, {})

    async def test_primary_errors_and_cancellation_are_preserved_without_guidance_calls(self):
        context = SimpleNamespace(message=SimpleNamespace(name="workspace_shelves", arguments={}))
        for failure in [ToolError("safe primary failure"), asyncio.CancelledError()]:
            with self.assertRaises(type(failure)) as caught:
                await self.middleware.on_call_tool(context, AsyncMock(side_effect=failure))
            self.assertIs(caught.exception, failure)
        result = SimpleNamespace(is_error=True)
        self.assertIs(
            await self.middleware.on_call_tool(context, AsyncMock(return_value=result)), result
        )
        self.runtime.run.assert_not_awaited()

    async def test_guidance_cancellation_and_whole_collection_deadline_do_not_advance(self):
        started, stopped = asyncio.Event(), asyncio.Event()

        async def wait(*_args):
            started.set()
            try:
                await asyncio.Future()
            finally:
                stopped.set()

        self.runtime.run.side_effect = wait
        task = asyncio.create_task(self.invoke())
        await asyncio.wait_for(started.wait(), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(stopped.is_set())
        self.assertEqual(self.middleware._stamps, {})
        self.middleware.timeout = 0.01
        warning = await self.invoke()
        self.assertEqual(warning["status"], "unavailable")
        self.assertEqual(self.middleware._stamps, {})
        lock = self.middleware._locks[Path("/configured/profile")]
        async with lock:
            self.assertEqual((await self.invoke())["status"], "unavailable")
        self.assertFalse(lock.locked())

    async def test_whole_board_budget_and_inconsistent_global_or_identity_warn_without_partial_delivery(
        self,
    ):
        self.middleware.budget = 256
        warning = await self.invoke()
        self.assertEqual(warning["boards"], [])
        self.assertEqual(warning["status"], "unavailable")
        self.assertEqual(self.middleware._stamps, {})
        self.middleware.budget = 256 * 1024
        for field in ["global", "credential_id"]:

            async def inconsistent(profile, *args, field=field):
                value = await self.read(profile, *args)
                if args[-1] == self.other:
                    if field == "global":
                        value["guidance"]["global"]["text"] = "racing global"
                    else:
                        value["context"]["credential_id"] = str(uuid4())
                return value

            self.runtime.run.side_effect = inconsistent
            warning = await self.invoke(
                name="workspace_selection", payload={"workspace_ids": [self.workspace, self.other]}
            )
            self.assertEqual(warning["status"], "unavailable")
            self.assertEqual(warning["boards"], [])
            self.assertEqual(self.middleware._stamps, {})

    async def test_scope_failure_prevents_partial_advancement_and_recovery_alias_is_used(self):
        async def partial(profile, *args):
            if args[-1] == self.other:
                raise ToolError("PRIVATE_GUIDANCE")
            return await self.read(profile, *args)

        self.runtime.run.side_effect = partial
        warning = await self.invoke(
            name="workspace_selection", payload={"workspace_ids": [self.workspace, self.other]}
        )
        self.assertEqual(warning["workspace_ids"], [self.workspace, self.other])
        self.assertEqual(warning["boards"], [])
        self.assertEqual(self.middleware._stamps, {})
        self.runtime._profiles["recovered"] = Path("/configured/profile")
        self.runtime.run.side_effect = self.read
        result = await self.invoke(
            name="workspace_entry_resolve",
            arguments={"profile": "office", "via_profile": "recovered"},
        )
        self.assertEqual(result["status"], "current")
        self.assertEqual(self.runtime.run.await_args.args[0], "recovered")

    async def test_lru_is_bounded_and_eviction_forces_redelivery(self):
        for number in range(4096):
            self.middleware._stamps[(str(number),)] = "hash"
        await self.invoke()
        self.assertEqual(len(self.middleware._stamps), 4096)
        self.assertNotIn(("0",), self.middleware._stamps)
        self.middleware._stamps.clear()
        self.assertEqual(len((await self.invoke())["boards"]), 2)

    async def test_overlapping_calls_emit_reverted_board_instead_of_advancing_an_unseen_stamp(self):
        await self.invoke()
        restored = await self.read("office", "guidance", "--workspace-id", self.workspace)
        changed = copy.deepcopy(restored)
        changed["guidance"]["workspace"].update(revision=3, text="new revision")
        self.runtime.run.side_effect = [changed, restored]
        prepare = self.middleware._prepare

        async def delayed(target):
            result = await prepare(target)
            # Exercise the scheduling boundary between preparation and handoff.
            await asyncio.sleep(0)
            return result

        with patch.object(self.middleware, "_prepare", side_effect=delayed):
            first, second = await asyncio.gather(self.invoke(), self.invoke())
        self.assertEqual(len(first["boards"]), 1)
        self.assertEqual(first["boards"][0]["board"]["text"], "new revision")
        self.assertEqual(len(second["boards"]), 1)
        self.assertEqual(second["boards"][0]["board"]["text"], "local")
        self.assertEqual(second["boards"][0]["board"]["revision"], 2)
        self.runtime.run.side_effect = self.read
        self.assertEqual((await self.invoke())["boards"], [])

    async def test_hundred_selected_scopes_are_bounded_and_global_is_returned_once(self):
        workspaces = [str(uuid4()) for _index in range(100)]
        result = await self.invoke(
            name="workspace_selection", payload={"workspace_ids": workspaces}
        )
        self.assertEqual(result["status"], "current")
        self.assertEqual(result["workspace_ids"], workspaces)
        self.assertEqual(len(result["boards"]), 101)
        self.assertEqual(self.runtime.run.await_count, 100)
        self.assertEqual(
            sum(row["board"]["scope"]["kind"] == "global" for row in result["boards"]), 1
        )

    async def test_large_primary_snapshot_is_preserved_with_separate_guidance_budget(self):
        body = "\0" * (512 * 1024)
        payload = {"workspace_id": self.workspace, "data": {"text": body}}
        result = await self.invoke(name="workspace_entry", payload=payload)
        self.assertEqual(result["status"], "current")
        self.assertEqual(payload["data"]["text"], body)
        self.assertGreater(len(json.dumps(payload)), 3 * 1024 * 1024)
        self.assertLess(len(json.dumps(result)), 256 * 1024)

    async def test_collection_deadline_cancels_and_reaps_real_cli_child(self):
        original = asyncio.create_subprocess_exec
        started, processes = asyncio.Event(), []

        async def spawn(*_arguments, **options):
            process = await original(sys.executable, "-c", "import time; time.sleep(30)", **options)
            processes.append(process)
            started.set()
            return process

        self.runtime.run = WorkspaceRuntime.run.__get__(self.runtime)
        self.middleware.timeout = 0.2
        with patch(
            "ccp_mcp_server.workspace_process.asyncio.create_subprocess_exec", side_effect=spawn
        ):
            task = asyncio.create_task(self.invoke())
            await asyncio.wait_for(started.wait(), 1)
            result = await task
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(len(processes), 1)
        self.assertIsNotNone(processes[0].returncode)
        self.assertEqual(self.runtime.process.slots._value, 8)
        self.assertEqual(self.middleware._stamps, {})

    async def test_context_format_scope_and_identity_validation_are_sanitized(self):
        value = await self.read("office", "guidance", "--workspace-id", self.workspace)
        for key, bad in [("format", True), ("format", 2), ("context", {}), ("guidance", None)]:
            altered = copy.deepcopy(value)
            altered[key] = bad
            with self.assertRaises(ToolError):
                delivery(altered, self.workspace)
        for key in self.context:
            altered = copy.deepcopy(value)
            altered["context"][key] = "PRIVATE_GUIDANCE"
            with self.assertRaises(ToolError) as caught:
                delivery(altered, self.workspace)
            self.assertNotIn("PRIVATE_GUIDANCE", str(caught.exception))
        with self.assertRaises(ToolError):
            delivery(value, None)
        value["context"]["workspace_grant_id"] = None
        with self.assertRaises(ToolError):
            delivery(value, self.workspace)

    async def test_actual_sdk_content_reaches_model_with_unchanged_primary_structured_data(self):
        async def process(*arguments):
            if arguments[0] == "guidance":
                return await self.read("office", *arguments)
            return {"client_id": self.context["client_id"], "unchanged": True}

        self.runtime.process.run = AsyncMock(side_effect=process)
        self.runtime.run = WorkspaceRuntime.run.__get__(self.runtime)
        async with Client(build_server(self.runtime)) as client:
            result = await client.call_tool("workspace_profile", {"profile": "office"})
            self.assertFalse(result.is_error)
            self.assertEqual(
                result.data, {"client_id": self.context["client_id"], "unchanged": True}
            )
            self.assertEqual(len(result.content), 2)
            sideband = json.loads(result.content[1].text)["_ccp_guidance"]
            self.assertEqual(sideband["boards"][0]["board"]["text"], "global")
            self.assertEqual(json.loads(result.content[0].text), result.data)

            def fail_current(value):
                if value["_ccp_guidance"]["status"] == "current":
                    raise ValueError("PRIVATE_GUIDANCE serialization")
                return encoded(value)

            with patch("ccp_mcp_server.workspace_delivery.encoded", side_effect=fail_current):
                warning = await self.invoke()
            self.assertEqual(warning["status"], "unavailable")
            self.assertNotIn("PRIVATE_GUIDANCE", str(warning))
            self.assertEqual(self.middleware._stamps, {})
