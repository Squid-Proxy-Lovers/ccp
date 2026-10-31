"""Initial acquisition never replaces a working store; worker ownership is exclusive."""

import importlib
import io
import threading
import unittest
from unittest.mock import patch

from managed_update_tests import (
    ManagedUpdateTests,
    build_module,
    fake_build,
    fs,
    managed,
    net,
    policy,
    worker,
)

initial = importlib.import_module("ccp_mcp_server.managed_install")
store_module = importlib.import_module("ccp_mcp_server.managed_store")


class ManagedInstallTests(unittest.TestCase):
    def setUp(self):
        self.f = ManagedUpdateTests()
        self.f.setUp()
        self.addCleanup(self.f.doCleanups)
        self.store = store_module.Store(self.f.root / "cold")

    def test_initial_official_install_leaves_existing_profiles_config_and_auto_policy_alone(self):
        config = self.f.config.read_bytes()
        existing = (self.f.store.root / "current.json").read_bytes()
        args = managed.parser().parse_args(
            [
                "--store",
                str(self.store.root),
                "install",
                "--channel",
                "stable",
                "--expected-revision",
                "0",
            ]
        )
        with self.f.boundaries(), patch.object(build_module, "run") as preflight:
            result = managed.run(args)
        preflight.assert_not_called()
        self.assertEqual(result["result"], "installed")
        self.assertEqual(result["generation"]["channel"], "stable")
        self.assertEqual(result["generation"]["tag"], "v0.1.0")
        self.assertEqual(result["pointer"]["revision"], 1)
        self.assertFalse(result["update_policy_created"])
        self.assertFalse(policy.present(self.store.root))
        self.assertEqual(policy.show(self.store), {"configured": False, "automatic": False})
        self.assertFalse((self.store.root / worker.NAME).exists())
        self.assertEqual(self.f.config.read_bytes(), config)
        self.assertEqual((self.f.store.root / "current.json").read_bytes(), existing)
        self.assertFalse(any(path.startswith("compare/") for path in self.f.api_calls))

    def test_active_store_refuses_initial_install_before_discovery_or_build(self):
        with self.f.boundaries():
            initial.install(self.store, "stable", None, 0)
        before = (self.store.root / "current.json").read_bytes()
        with (
            patch.object(net, "api") as api,
            patch.object(build_module, "build") as build,
            self.assertRaises(ValueError),
        ):
            initial.install(self.store, "stable", None, 0)
        api.assert_not_called()
        build.assert_not_called()
        self.assertEqual((self.store.root / "current.json").read_bytes(), before)

    def test_verification_failure_can_retry_empty_store_but_never_configures_or_selects(self):
        config = self.f.config.read_bytes()
        with (
            self.f.boundaries(),
            patch(
                "ccp_mcp_server.release_contract.verify_provenance",
                side_effect=ValueError("refused"),
            ),
            self.assertRaises(ValueError),
        ):
            initial.install(self.store, "stable", None, 0)
        self.assertEqual(self.store.pointer(), store_module.Pointer())
        self.assertFalse((self.store.root / "bundles").exists())
        self.assertFalse(policy.present(self.store.root))
        self.assertEqual(self.f.config.read_bytes(), config)
        with self.f.boundaries():
            result = initial.install(self.store, "stable", "v0.1.0", 0)
        self.assertEqual(result["generation"]["tag"], "v0.1.0")
        self.assertEqual(self.f.api_calls[-1], "releases/tags/v0.1.0")

    def test_unknown_previous_policy_or_invalid_input_never_starts_initial_download(self):
        for channel, tag, revision in [
            ("bad", None, 0),
            ("stable", "../bad", 0),
            ("stable", None, True),
            ("stable", None, 1),
        ]:
            with self.assertRaises(ValueError):
                initial.install(self.store, channel, tag, revision)
            self.assertFalse(self.store.root.exists())
        with fs.locked(self.store.root):
            self.store._initialize()
            fs.atomic(self.store.root / policy.MARKER, {"format": 1})
        marker = (self.store.root / policy.MARKER).read_bytes()
        with patch.object(net, "api") as api, self.assertRaises(ValueError):
            initial.install(self.store, "stable", None, 0)
        api.assert_not_called()
        self.assertEqual((self.store.root / policy.MARKER).read_bytes(), marker)
        self.assertEqual(self.store.pointer(), store_module.Pointer())
        (self.store.root / "current.json").unlink()
        with patch.object(net, "api") as api, self.assertRaises(ValueError):
            initial.install(self.store, "stable", None, 0)
        api.assert_not_called()
        self.assertFalse((self.store.root / "current.json").exists())
        self.assertEqual((self.store.root / policy.MARKER).read_bytes(), marker)

    def test_concurrent_manual_selection_wins_and_new_ready_candidate_is_retained(self):
        with self.f.boundaries():
            competing = self.store.stage(self.f.f.inputs, "stable", "v0.1.0")

        def select_competing(path, manifest):
            fake_build(path, manifest)
            self.store.activate(competing.generation_id, 0)

        with (
            self.f.boundaries(),
            patch.object(build_module, "build", side_effect=select_competing),
            self.assertRaises(ValueError),
        ):
            initial.install(self.store, "stable", None, 0)
        self.assertEqual(self.store.pointer().current, competing.generation_id)
        self.assertEqual(self.store.pointer().revision, 1)
        self.assertEqual(len(list((self.store.root / "bundles").iterdir())), 2)
        self.assertFalse(policy.present(self.store.root))

    def test_lost_initial_selection_acknowledgement_preserves_selected_pair_and_refuses_reinstall(
        self,
    ):
        atomic = fs.atomic

        def uncertain(path, value):
            atomic(path, value)
            if path.name == "current.json" and value["revision"] == 1:
                raise OSError("owned loss after initial selection")

        with (
            self.f.boundaries(),
            patch.object(fs, "atomic", side_effect=uncertain),
            self.assertRaises(OSError),
        ):
            initial.install(self.store, "stable", None, 0)
        pointer = self.store.pointer()
        self.assertEqual(pointer.revision, 1)
        self.assertTrue((self.store.root / "bundles" / pointer.current / "ready.json").exists())
        with patch.object(net, "api") as api, self.assertRaises(ValueError):
            initial.install(self.store, "stable", None, 0)
        api.assert_not_called()
        self.assertEqual(self.store.pointer(), pointer)


class ManagedOperationalTests(unittest.TestCase):
    def setUp(self):
        self.f = ManagedUpdateTests()
        self.f.setUp()
        self.addCleanup(self.f.doCleanups)

    def test_completed_worker_result_cannot_be_overwritten_by_a_late_finish(self):
        self.f.configure(True)
        revision, attempt = worker.claim(self.f.store)
        with patch(
            "ccp_mcp_server.managed_update.update",
            return_value={"result": "updated", "policy_revision": revision},
        ):
            worker.run(self.f.store, revision, attempt)
        confirmed = worker.status(self.f.store)
        self.assertEqual(confirmed["result"], "updated")
        worker.finish(self.f.store, attempt, "refused", revision)
        self.assertEqual(worker.status(self.f.store), confirmed)

    def test_same_attempt_race_has_only_one_runner_and_preserves_its_terminal_result(self):
        self.f.configure(True)
        revision, attempt = worker.claim(self.f.store)
        started, release = threading.Event(), threading.Event()
        calls, outcomes, errors = [], [], []

        def updating(_store, _revision):
            calls.append(True)
            if len(calls) > 1:
                return {"result": "refused", "policy_revision": revision}
            started.set()
            self.assertTrue(release.wait(10))
            return {"result": "updated", "policy_revision": revision}

        def first():
            try:
                outcomes.append(worker.run(self.f.store, revision, attempt))
            except (ValueError, OSError, AssertionError) as error:
                errors.append(str(error))

        with patch("ccp_mcp_server.managed_update.update", side_effect=updating):
            job = threading.Thread(target=first)
            job.start()
            try:
                self.assertTrue(started.wait(10))
                self.assertEqual(worker.run(self.f.store, revision, attempt)["result"], "refused")
                self.assertEqual(worker.status(self.f.store)["result"], "scheduled")
                self.assertEqual(len(calls), 1)
            finally:
                release.set()
                job.join(timeout=20)
        self.assertFalse(job.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(outcomes[0]["result"], "updated")
        self.assertEqual(worker.status(self.f.store)["result"], "updated")

    def test_incremental_body_reads_check_soft_deadline_without_waiting_to_fill_a_megabyte(self):
        class Incremental(io.BytesIO):
            headers = {"Content-Length": "3"}

            def read(self, _size):
                raise AssertionError("a fill read could stall progress checks")

            def read1(self, size):
                return io.BytesIO.read(self, min(1, size))

        sink = io.BytesIO()
        with patch.object(net.time, "monotonic", side_effect=[0, 1, 2, 3]):
            net.transfer(Incremental(b"abc"), sink, 3)
        self.assertEqual(sink.getvalue(), b"abc")
        sink = io.BytesIO()
        with (
            patch.object(net.time, "monotonic", side_effect=[0, 121]),
            self.assertRaisesRegex(ValueError, "time limit"),
        ):
            net.transfer(Incremental(b"abc"), sink, 3)
        self.assertEqual(sink.getvalue(), b"")
