"""Entry MCP request bounds, receipt projections, guards and SDK privacy."""

from __future__ import annotations

import asyncio
import copy
import json
import sys
import unittest
from dataclasses import asdict
from pathlib import Path
from unittest.mock import AsyncMock
from uuid import uuid4

from ccp_mcp_server.workspace import build_server
from ccp_mcp_server.workspace_config import WorkspaceConfig
from ccp_mcp_server.workspace_entry import WorkspaceEntryTools, operation_info, receipt, resolution
from ccp_mcp_server.workspace_entry_request import (
    AppendChange,
    ArchiveChange,
    CreateChange,
    EntryBody,
    EntryIntent,
    OperationTarget,
    RestoreChange,
    ReviseChange,
    validate_intent,
)
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
from fastmcp import Client
from fastmcp.exceptions import ToolError


def info(request, state="committed"):
    return {
        "instance_id": str(uuid4()),
        "client_id": str(uuid4()),
        "credential_id": str(uuid4()),
        "workspace_id": request.workspace_id,
        "operation_id": request.operation_id,
        "request_hash": "a" * 64,
        "state": state,
    }


def target(metadata):
    return OperationTarget(
        **{
            key: metadata[key]
            for key in ["workspace_id", "operation_id", "credential_id", "request_hash"]
        }
    )


def intent():
    return EntryIntent(
        str(uuid4()),
        str(uuid4()),
        CreateChange(
            "create", str(uuid4()), "entry", EntryBody("PRIVATE_BODY\0é", labels=["z", "a"])
        ),
        "agent",
    )


class WorkspaceEntryTests(unittest.IsolatedAsyncioTestCase):
    def config(self):
        return WorkspaceConfig(
            Path(sys.executable),
            {"office": Path("/host/original"), "recovered": Path("/host/recovered")},
        )

    async def test_all_actions_capture_exact_private_payload_and_body_free_bound_receipt(self):
        create = intent()
        entry_id = str(uuid4())
        changes = [
            create.change,
            AppendChange("append", entry_id, 1, "+PRIVATE_BODY"),
            ReviseChange("revise", entry_id, 2, EntryBody("revision")),
            ArchiveChange("archive", entry_id, 3),
            RestoreChange("restore", entry_id, 4),
        ]
        with WorkspaceRuntime(self.config()) as runtime:
            tools = WorkspaceEntryTools(runtime)
            for index, change in enumerate(changes):
                request = EntryIntent(create.workspace_id, str(uuid4()), change, "agent")
                metadata = info(request)
                runtime.process.run_input = AsyncMock(
                    return_value={
                        "entry_id": entry_id,
                        "revision": index + 1,
                        "operation": metadata,
                    }
                )
                result = await tools.workspace_mutate_entry("office", request)
                self.assertEqual(result["workspace_id"], request.workspace_id)
                self.assertEqual(result["target"], asdict(target(metadata)))
                self.assertNotIn("PRIVATE_BODY", json.dumps(result))
                directory, body, arguments = runtime.process.run_input.await_args.args
                self.assertEqual(directory, runtime.root)
                self.assertEqual(json.loads(body), asdict(request))
                self.assertEqual(arguments, ("content", "--profile", "/host/original", "mutate"))
                runtime.process.run_input.assert_awaited_once()

    async def test_payload_is_captured_before_await_and_invalid_fields_never_stage(self):
        request = intent()
        before = asdict(request)
        metadata = info(request)
        started, release = asyncio.Event(), asyncio.Event()
        with WorkspaceRuntime(self.config()) as runtime:
            captured = []

            async def submit(_directory, body, _arguments):
                captured.append(json.loads(body))
                started.set()
                await release.wait()
                return {"entry_id": str(uuid4()), "revision": 1, "operation": metadata}

            runtime.process.run_input = AsyncMock(side_effect=submit)
            task = asyncio.create_task(
                WorkspaceEntryTools(runtime).workspace_mutate_entry("office", request)
            )
            await asyncio.wait_for(started.wait(), 2)
            request.change.content.labels.append("later")
            release.set()
            await task
            self.assertEqual(captured, [before])
            runtime.process.run_input.reset_mock()
            raw = before
            for path, value in [
                (("operation_id",), "00000000-0000-0000-0000-000000000000"),
                (("agent",), "bad\nagent"),
                (("change", "content", "text"), "x" * (256 * 1024 + 1)),
                (("change", "content", "labels"), ["duplicate", "duplicate"]),
                (("change", "content", "labels"), ["bad\nlabel"]),
                (("change", "owner_client_id"), "PRIVATE_BODY"),
            ]:
                changed = copy.deepcopy(raw)
                location = changed
                for part in path[:-1]:
                    location = location[part]
                location[path[-1]] = value
                with self.assertRaises(ToolError):
                    validate_intent(changed)
            runtime.process.run_input.assert_not_awaited()

    async def test_pending_projection_and_controls_use_complete_guard_and_host_recovery_alias(self):
        request = intent()
        metadata = info(request, "pending")
        expected = target(metadata)
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock(
                return_value={
                    "instance_id": metadata["instance_id"],
                    "client_id": metadata["client_id"],
                    "pending_content": metadata,
                    "last_content": None,
                }
            )
            tools = WorkspaceEntryTools(runtime)
            state = await tools.workspace_entry_pending("office")
            self.assertEqual(state, {"pending": metadata, "last": None, "target": asdict(expected)})
            self.assertNotIn("PRIVATE_BODY", json.dumps(state))
            runtime.process.run.assert_awaited_with("info", "--profile", "/host/original")
            done = {**metadata, "state": "committed"}
            runtime.process.run.return_value = {
                "entry_id": str(uuid4()),
                "revision": 2,
                "operation": done,
            }
            result = await tools.workspace_entry_resume("office", expected)
            self.assertEqual(result["target"], asdict(expected))
            runtime.process.run.assert_awaited_with(
                "content", "--profile", "/host/original", "resume", *expected.arguments()
            )
            proof = {**asdict(expected), "outcome": {"state": "cancelled"}}
            runtime.process.run.return_value = proof
            result = await tools.workspace_entry_resolve("office", expected, "recovered")
            self.assertEqual(result["data"], proof)
            runtime.process.run.assert_awaited_with(
                "content",
                "--profile",
                "/host/original",
                "resolve",
                *expected.arguments(),
                "--via-profile",
                "/host/recovered",
            )
            with self.assertRaises(ToolError):
                await tools.workspace_entry_resolve("office", expected, "/outside/path")

    async def test_malformed_metadata_and_wrong_receipts_never_look_empty_or_successful(self):
        request = intent()
        metadata = info(request)
        expected = target(metadata)
        valid = {"entry_id": str(uuid4()), "revision": 1, "operation": metadata}
        for value in [
            [],
            {},
            {**valid, "extra": "PRIVATE_BODY"},
            {**valid, "revision": 2},
            {**valid, "operation": {**metadata, "state": []}},
            {**valid, "operation": {**metadata, "operation_id": str(uuid4())}},
        ]:
            with self.assertRaises(ToolError) as error:
                receipt(value, request)
            self.assertNotIn("PRIVATE_BODY", str(error.exception))
        with self.assertRaises(ToolError):
            operation_info({**metadata, "request_hash": "INVALID"})
        with self.assertRaises(ToolError):
            resolution(
                {**asdict(expected), "request_hash": "b" * 64, "outcome": {"state": "cancelled"}},
                expected,
            )
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock(return_value={})
            tools = WorkspaceEntryTools(runtime)
            with self.assertRaises(ToolError):
                await tools.workspace_entry_pending("office")
            runtime.process.run.return_value = {"pending_content": None, "last_content": None}
            self.assertEqual(
                await tools.workspace_entry_pending("office"),
                {"pending": None, "last": None, "target": None},
            )

    async def test_sensitive_rpc_claims_types_and_nested_changes_refuse_without_logs_or_children(
        self,
    ):
        request = asdict(intent())
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run_input = AsyncMock()
            runtime.process.run = AsyncMock()
            async with Client(build_server(runtime)) as client:
                bad_requests = []
                for path in [
                    ("owner_client_id",),
                    ("change", "owner_client_id"),
                    ("change", "content", "unknown"),
                ]:
                    changed = copy.deepcopy(request)
                    location = changed
                    for part in path[:-1]:
                        location = location[part]
                    location[path[-1]] = "PRIVATE_BODY"
                    bad_requests.append(changed)
                bad_requests.append(
                    {
                        **request,
                        "change": {
                            "action": "append",
                            "entry_id": str(uuid4()),
                            "expected_revision": True,
                            "text": "PRIVATE_BODY",
                        },
                    }
                )
                for changed in bad_requests:
                    result = await client.call_tool(
                        "workspace_mutate_entry",
                        {"profile": "office", "request": changed},
                        raise_on_error=False,
                    )
                    self.assertTrue(result.is_error)
                    self.assertNotIn(
                        "PRIVATE_BODY",
                        "".join(item.text for item in result.content if hasattr(item, "text")),
                    )
            runtime.process.run_input.assert_not_awaited()
            runtime.process.run.assert_not_awaited()
