"""Status ownership intents, body-free recovery and hostile MCP inputs."""

from __future__ import annotations

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
from ccp_mcp_server.workspace_content import ContentPage, ContentScope
from ccp_mcp_server.workspace_delivery_values import targets
from ccp_mcp_server.workspace_runtime import WorkspaceRuntime
from ccp_mcp_server.workspace_status import WorkspaceStatusTools
from ccp_mcp_server.workspace_status_request import StatusClear, StatusSet, StatusTarget
from ccp_mcp_server.workspace_status_values import mutation_result, operation_info, page
from fastmcp import Client
from fastmcp.exceptions import ToolError


def request():
    return StatusSet(str(uuid4()), str(uuid4()), str(uuid4()), 0, "PRIVATE_STATUS\ufeffé", "\ufeff")


def metadata(req, state="committed"):
    proof = {
        "workspace_id": req.workspace_id,
        "status_id": req.status_id,
        "operation_id": req.operation_id,
        "client_id": str(uuid4()),
        "revision": req.expected_revision + 1,
        "cleared": isinstance(req, StatusClear),
        "updated_at": 100,
    }
    return {
        **{key: proof[key] for key in ["workspace_id", "status_id", "operation_id", "client_id"]},
        "instance_id": str(uuid4()),
        "credential_id": str(uuid4()),
        "request_hash": "a" * 64,
        "expected_revision": req.expected_revision,
        "action": "clear" if isinstance(req, StatusClear) else "set",
        "state": state,
        "receipt": proof if state == "committed" else None,
    }


def target(info):
    return StatusTarget(**{key: info[key] for key in StatusTarget.__dataclass_fields__})


def observation(req):
    return {
        "workspace_id": req.workspace_id,
        "status_id": req.status_id,
        "client_id": str(uuid4()),
        "client_name": "registered client",
        "client_disabled": True,
        "revision": 1,
        "agent": "\ufeff",
        "message": "\ufeff",
        "updated_at": 100,
        "stale": True,
    }


class WorkspaceStatusTests(unittest.IsolatedAsyncioTestCase):
    def config(self):
        return WorkspaceConfig(Path(sys.executable), {"office": Path("/host/original")})

    async def test_set_clear_use_private_payload_and_preserve_unicode_without_body_in_receipts(
        self,
    ):
        req = request()
        clear = StatusClear(req.workspace_id, req.status_id, str(uuid4()), 1)
        with WorkspaceRuntime(self.config()) as runtime:
            tools = WorkspaceStatusTools(runtime)
            for action in [req, clear]:
                info = metadata(action)
                runtime.process.run_input = AsyncMock(
                    return_value={**info["receipt"], "operation": info}
                )
                result = await (
                    tools.workspace_update_status("office", action)
                    if isinstance(action, StatusSet)
                    else tools.workspace_clear_status("office", action)
                )
                directory, body, arguments = runtime.process.run_input.await_args.args
                self.assertEqual(directory, runtime.root)
                self.assertEqual(arguments, ("status", "--profile", "/host/original", "mutate"))
                payload = json.loads(body)
                self.assertEqual(payload["expected_revision"], action.expected_revision)
                if isinstance(action, StatusSet):
                    self.assertEqual(payload["change"]["message"], action.message)
                    self.assertEqual(payload["change"]["agent"], "\ufeff")
                else:
                    self.assertEqual(payload["change"], {"action": "clear"})
                self.assertEqual(result["target"], asdict(target(info)))
                self.assertNotIn("PRIVATE_STATUS", json.dumps(result))
                runtime.process.run_input.assert_awaited_once()

    async def test_pending_last_and_local_forget_are_distinct_and_preserve_exact_guard(self):
        req = request()
        pending = metadata(req, "pending")
        expected = target(pending)
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock(
                return_value={
                    **{key: pending[key] for key in ["instance_id", "client_id", "credential_id"]},
                    "pending_status": pending,
                    "last_status": None,
                }
            )
            tools = WorkspaceStatusTools(runtime)
            self.assertEqual(
                await tools.workspace_status_pending("office"),
                {"pending": pending, "last": None, "target": asdict(expected)},
            )
            runtime.process.run.assert_awaited_with("info", "--profile", "/host/original")
            runtime.process.run.reset_mock()
            with self.assertRaises(ToolError):
                await tools.workspace_status_forget("office", expected, False)
            runtime.process.run.assert_not_awaited()
            forgotten = {**pending, "state": "locally_forgotten"}
            runtime.process.run.return_value = {
                "locally_forgotten": True,
                "cancelled": False,
                "operation": forgotten,
            }
            result = await tools.workspace_status_forget("office", expected, True)
            self.assertFalse(result["data"]["cancelled"])
            runtime.process.run.assert_awaited_with(
                "status",
                "--profile",
                "/host/original",
                "forget",
                *expected.arguments(),
                "--confirm-local-forget",
            )
            for state in [forgotten, metadata(req)]:
                runtime.process.run.return_value = {
                    **{key: state[key] for key in ["instance_id", "client_id", "credential_id"]},
                    "pending_status": None,
                    "last_status": state,
                }
                self.assertIsNone((await tools.workspace_status_pending("office"))["target"])

    async def test_resume_receipt_must_match_all_guard_fields_and_original_revision_action(self):
        req = request()
        info = metadata(req)
        expected = target(info)
        valid = {**info["receipt"], "operation": info}
        with WorkspaceRuntime(self.config()) as runtime:
            tools = WorkspaceStatusTools(runtime)
            runtime.process.run = AsyncMock(return_value=valid)
            result = await tools.workspace_status_resume("office", expected)
            self.assertEqual(result["target"], asdict(expected))
            runtime.process.run.assert_awaited_with(
                "status", "--profile", "/host/original", "resume", *expected.arguments()
            )
            for field in [
                "workspace_id",
                "status_id",
                "operation_id",
                "credential_id",
                "request_hash",
            ]:
                changed = copy.deepcopy(valid)
                changed["operation"][field] = "b" * 64 if field == "request_hash" else str(uuid4())
                runtime.process.run.return_value = changed
                with self.assertRaises(ToolError):
                    await tools.workspace_status_resume("office", expected)
            for field, value in [("revision", 2), ("cleared", True), ("client_id", str(uuid4()))]:
                changed = copy.deepcopy(valid)
                changed[field] = value
                with self.assertRaises(ToolError):
                    mutation_result(changed, req)

    async def test_reads_keep_stale_unicode_and_cleared_slots_but_reject_cross_scope_or_false_age(
        self,
    ):
        req = request()
        row = observation(req)
        payload = {
            "items": [row],
            "next_cursor": row["status_id"],
            "observed_at": 400,
            "stale_after_seconds": 300,
        }
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run = AsyncMock(return_value=payload)
            tools = WorkspaceStatusTools(runtime)
            scope = ContentScope(workspace_id=req.workspace_id)
            result = await tools.workspace_status_messages("office", scope, ContentPage(limit=1))
            self.assertEqual(result["data"], payload)
            runtime.process.run.return_value = {**row, "message": None, "agent": None}
            self.assertIsNone(
                (await tools.workspace_status_message("office", scope, req.status_id))["data"][
                    "message"
                ]
            )
            for changed in [
                {**payload, "items": [{**row, "workspace_id": str(uuid4())}]},
                {**payload, "items": [{**row, "stale": False}]},
                {**payload, "items": [{**row, "message": None, "agent": None}]},
                {**payload, "next_cursor": str(uuid4())},
                {**payload, "items": [row, row]},
            ]:
                with self.assertRaises(ToolError):
                    page(changed, req.workspace_id, None, 2)
            rollback = {**payload, "observed_at": 50}
            self.assertEqual(page(rollback, req.workspace_id, None, 1), rollback)
            with self.assertRaises(ToolError):
                page(payload, req.workspace_id, req.status_id, 1)

    async def test_malformed_proofs_and_pending_identity_never_become_empty_or_committed(self):
        req = request()
        pending = metadata(req, "pending")
        for field, value in [
            ("state", []),
            ("action", {}),
            ("receipt", metadata(req)["receipt"]),
            ("expected_revision", True),
            ("request_hash", "A" * 64),
        ]:
            with self.assertRaises(ToolError):
                operation_info({**pending, field: value})
        with WorkspaceRuntime(self.config()) as runtime:
            tools = WorkspaceStatusTools(runtime)
            for value in [
                {},
                {"pending_status": None},
                {
                    "pending_status": pending,
                    "last_status": None,
                    "instance_id": pending["instance_id"],
                    "client_id": pending["client_id"],
                    "credential_id": str(uuid4()),
                },
            ]:
                runtime.process.run = AsyncMock(return_value=value)
                with self.assertRaises(ToolError):
                    await tools.workspace_status_pending("office")

    async def test_sdk_rejects_private_invalid_claims_before_children_and_redacts_errors(self):
        req = asdict(request())
        bad = [
            {**req, field: value}
            for field, value in [
                ("expected_revision", True),
                ("expected_revision", 2**63 - 1),
                ("client_id", "PRIVATE_STATUS"),
                ("status_id", "00000000-0000-0000-0000-000000000000"),
                ("message", "PRIVATE_STATUS\0"),
                ("message", "é" * 2049),
                ("agent", "é" * 65),
            ]
        ]
        with WorkspaceRuntime(self.config()) as runtime:
            runtime.process.run_input, runtime.process.run = AsyncMock(), AsyncMock()
            async with Client(build_server(runtime)) as client:
                calls = [
                    ("workspace_update_status", {"profile": "office", "request": item})
                    for item in bad
                ]
                calls.append(("workspace_clear_status", {"profile": "office", "request": req}))
                calls.append(
                    (
                        "workspace_status_forget",
                        {
                            "profile": "office",
                            "target": asdict(target(metadata(request()))),
                            "confirm_local_forget": 1,
                        },
                    )
                )
                for name, arguments in calls:
                    result = await client.call_tool(name, arguments, raise_on_error=False)
                    self.assertTrue(result.is_error)
                    self.assertNotIn(
                        "PRIVATE_STATUS",
                        "".join(item.text for item in result.content if hasattr(item, "text")),
                    )
            runtime.process.run_input.assert_not_awaited()
            runtime.process.run.assert_not_awaited()

    async def test_guidance_scope_uses_explicit_success_and_pending_stays_global_only(self):
        ws = str(uuid4())
        self.assertEqual(targets("workspace_status_pending", {"pending": {"workspace_id": ws}}), [])
        for name in [
            "workspace_update_status",
            "workspace_clear_status",
            "workspace_status_resume",
            "workspace_status_forget",
            "workspace_status_messages",
            "workspace_status_message",
        ]:
            self.assertEqual(targets(name, {"workspace_id": ws}), [ws])
