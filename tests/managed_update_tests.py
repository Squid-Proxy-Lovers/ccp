"""Official update policy, private-copy pipeline and optional detached checks."""

import importlib
import io
import json
import shutil
import socket
import subprocess
import sys
import threading
import unittest
import urllib.error
from contextlib import ExitStack
from unittest.mock import patch
from uuid import uuid4

from managed_bundle_tests import ROOT, ManagedBundleTests, build_module, contract, fake_build, fs

managed = importlib.import_module("ccp_mcp_server.managed")
net = importlib.import_module("ccp_mcp_server.managed_network")
policy = importlib.import_module("ccp_mcp_server.managed_policy")
preflight = importlib.import_module("ccp_mcp_server.managed_preflight")
release = importlib.import_module("ccp_mcp_server.managed_release")
updater = importlib.import_module("ccp_mcp_server.managed_update")
worker = importlib.import_module("ccp_mcp_server.managed_worker")


class ManagedUpdateTests(unittest.TestCase):
    def setUp(self):
        self.f = ManagedBundleTests()
        self.f.setUp()
        self.addCleanup(self.f.doCleanups)
        self.store, self.root = self.f.store, self.f.root
        self.first = self.f.stage()
        self.store.activate(self.first.generation_id, 0)
        self.config = self.root / "host.json"
        self.config.write_text(
            json.dumps(
                {
                    "format": 1,
                    "profiles": {
                        "office": str(self.root / "profile-a"),
                        "second": str(self.root / "profile-b"),
                    },
                }
            )
        )
        self.config.chmod(0o600)
        self.reply = {
            "format": 1,
            "profile_format": 5,
            "migration_required": False,
            "pending": False,
            "server": {
                "instance_id": str(uuid4()),
                "package_version": "0.1.0",
                "native_wire": 1,
                "workspace_api": 1,
            },
        }
        self.api_calls = []

    def configure(self, automatic=False, pin=None):
        expected = policy.load(self.store.root).revision if policy.present(self.store.root) else 0
        proposed = policy.Policy(0, automatic, "stable", pin, str(self.config), "", "0" * 40)
        return policy.configure(self.store, proposed, expected)

    def api(self, path):
        self.api_calls.append(path)
        if path.startswith("releases"):
            return {"tag_name": "v0.1.0", "draft": False, "prerelease": False}
        base, candidate = path.removeprefix("compare/").split("...")
        candidate = candidate.split("?")[0]
        return {
            "base_commit": {"sha": base},
            "merge_base_commit": {"sha": base},
            "status": "ahead" if base < candidate else "behind",
            "behind_by": 0 if base < candidate else 1,
        }

    def asset(self, _tag, name, target, _maximum):
        shutil.copyfile(self.f.inputs / name, target)
        target.chmod(0o600)

    def boundaries(self):
        stack = ExitStack()
        stack.enter_context(patch.object(contract, "verify_provenance"))
        stack.enter_context(patch.object(net, "api", side_effect=self.api))
        stack.enter_context(patch.object(net, "asset", side_effect=self.asset))
        stack.enter_context(patch.object(build_module, "build", side_effect=fake_build))
        stack.enter_context(
            patch.object(
                build_module, "run", side_effect=lambda _args: json.dumps(self.reply).encode()
            )
        )
        return stack

    def test_manual_by_default_policy_cas_disable_and_missing_policy_never_reset_floor(self):
        self.assertEqual(policy.show(self.store), {"configured": False, "automatic": False})
        with (
            fs.locked(self.store.root, shared=True),
            patch.object(worker.subprocess, "Popen") as spawn,
        ):
            worker.schedule(self.store)
        spawn.assert_not_called()
        self.assertFalse((self.store.root / worker.NAME).exists())
        first = self.configure()
        self.assertFalse(first.automatic)
        self.assertEqual(first.source_floor, "a" * 40)
        with self.assertRaises(ValueError):
            policy.configure(self.store, first, 0)
        enabled = self.configure(True)
        self.config.unlink()
        disabled = policy.disable(self.store, enabled.revision)
        self.assertFalse(disabled.automatic)
        self.assertEqual(disabled.source_floor, first.source_floor)
        marker = (self.store.root / policy.MARKER).read_bytes()
        (self.store.root / policy.NAME).unlink()
        with self.assertRaises(OSError):
            policy.load(self.store.root)
        with self.assertRaises(OSError):
            policy.configure(self.store, first, 0)
        self.assertFalse((self.store.root / policy.NAME).exists())
        self.assertEqual((self.store.root / policy.MARKER).read_bytes(), marker)

    def test_verified_update_checks_both_aliases_preserves_config_and_retains_old_pair(self):
        self.configure()
        self.f.prepare("b" * 40)
        config = self.config.read_bytes()
        with (
            self.boundaries(),
            patch.object(preflight, "inspect", wraps=preflight.inspect) as inspect,
        ):
            result = updater.update(self.store)
        self.assertEqual(result["result"], "updated")
        self.assertEqual([r["profile"] for r in result["profiles"]], ["office", "second"])
        self.assertEqual(inspect.call_count, 1)
        self.assertNotEqual(self.store.pointer().current, self.first.generation_id)
        self.assertEqual(self.store.pointer().previous, self.first.generation_id)
        self.assertEqual(policy.load(self.store.root).source_floor, "b" * 40)
        self.assertEqual(policy.load(self.store.root).revision, 2)
        self.assertTrue(self.first.path.exists())
        self.assertEqual(self.config.read_bytes(), config)

    def test_pending_migration_unknown_or_malformed_reports_never_select(self):
        self.configure()
        self.f.prepare("b" * 40)
        pointer = (self.store.root / "current.json").read_bytes()
        original = self.reply.copy()
        for change in [
            {"pending": True},
            {"migration_required": True},
            {"profile_format": 4},
            {"profile_format": True},
            {"format": True},
            {"extra": "PRIVATE_BODY"},
            {"pending": None},
        ]:
            self.reply = {**original, **change}
            with self.boundaries(), self.assertRaises(ValueError) as caught:
                updater.update(self.store)
            self.assertNotIn("PRIVATE_BODY", str(caught.exception))
            self.assertEqual((self.store.root / "current.json").read_bytes(), pointer)
            self.assertEqual(policy.load(self.store.root).source_floor, "a" * 40)
        self.assertTrue(self.first.path.exists())

    def test_floor_survives_manual_rollback_and_checks_current_and_highwater_ancestry(self):
        self.configure()
        self.f.prepare("b" * 40)
        with self.boundaries():
            updater.update(self.store)
        self.store.rollback(2)
        self.configure()
        self.assertEqual(policy.load(self.store.root).source_floor, "b" * 40)
        self.f.prepare("a" * 40)
        with self.boundaries(), self.assertRaisesRegex(ValueError, "descendant"):
            updater.update(self.store)
        self.f.prepare("c" * 40)
        with self.boundaries():
            updater.update(self.store)
        self.assertTrue(any("b" * 40 + "..." + "c" * 40 in path for path in self.api_calls))
        self.assertTrue(any("a" * 40 + "..." + "c" * 40 in path for path in self.api_calls))
        self.assertEqual(policy.load(self.store.root).source_floor, "c" * 40)

    def test_exact_tag_pin_retains_authenticated_source_even_when_current_is_manually_changed(self):
        self.configure(pin="v0.1.0")
        self.assertEqual(policy.load(self.store.root).pin_source, "a" * 40)
        self.f.prepare("b" * 40)
        with self.boundaries(), self.assertRaisesRegex(ValueError, "Pinned release source changed"):
            updater.update(self.store)
        second = self.f.stage()
        self.store.activate(second.generation_id, 1)
        self.configure(pin="v0.1.0")
        self.assertEqual(policy.load(self.store.root).pin_source, "a" * 40)

    def test_disable_during_preflight_revokes_automatic_selection_but_keeps_ready_candidate(self):
        self.configure(True)
        self.f.prepare("b" * 40)

        def response(_arguments):
            if policy.load(self.store.root).automatic:
                policy.disable(self.store, 1)
            return json.dumps(self.reply).encode()

        with (
            self.boundaries(),
            patch.object(build_module, "run", side_effect=response),
            self.assertRaisesRegex(ValueError, "policy changed"),
        ):
            updater.update(self.store, 1)
        self.assertEqual(self.store.pointer().current, self.first.generation_id)
        self.assertFalse(policy.load(self.store.root).automatic)
        self.assertEqual(len(list((self.store.root / "bundles").iterdir())), 2)
        self.assertEqual(policy.load(self.store.root).source_floor, "a" * 40)

    def test_changed_config_or_concurrent_pointer_never_overwrites_current_choice(self):
        self.configure()
        self.f.prepare("b" * 40)
        before = self.config.read_bytes()

        def change_config(generation, manifest):
            fake_build(generation, manifest)
            self.config.write_bytes(before + b" ")

        with (
            self.boundaries(),
            patch.object(build_module, "build", side_effect=change_config),
            self.assertRaisesRegex(ValueError, "configuration changed"),
        ):
            updater.update(self.store)
        self.assertEqual(self.store.pointer().current, self.first.generation_id)
        self.assertEqual(policy.load(self.store.root).source_floor, "a" * 40)
        self.config.write_bytes(before)
        concurrent = self.f.stage()

        def change_pointer(_arguments):
            self.store.activate(concurrent.generation_id, 1)
            return json.dumps(self.reply).encode()

        with (
            self.boundaries(),
            patch.object(build_module, "run", side_effect=change_pointer),
            self.assertRaisesRegex(ValueError, "Active software changed"),
        ):
            updater.update(self.store)
        self.assertEqual(self.store.pointer().current, concurrent.generation_id)
        self.assertEqual(policy.load(self.store.root).source_floor, "a" * 40)

    def test_unknown_floor_or_pointer_commit_retains_advanced_floor_and_resolves_next_attempt(self):
        self.configure()
        self.f.prepare("b" * 40)
        atomic = fs.atomic

        def lost_floor(path, value):
            atomic(path, value)
            if path.name == policy.NAME:
                raise OSError("owned lost floor acknowledgement")

        with (
            self.boundaries(),
            patch.object(fs, "atomic", side_effect=lost_floor),
            self.assertRaises(OSError),
        ):
            updater.update(self.store)
        self.assertEqual(self.store.pointer().current, self.first.generation_id)
        self.assertEqual(policy.load(self.store.root).source_floor, "b" * 40)

        def lost_pointer(path, value):
            atomic(path, value)
            if path.name == "current.json":
                raise OSError("owned lost pointer acknowledgement")

        with (
            self.boundaries(),
            patch.object(fs, "atomic", side_effect=lost_pointer),
            self.assertRaises(OSError),
        ):
            updater.update(self.store)
        selected = self.store.pointer().current
        self.assertNotEqual(selected, self.first.generation_id)
        self.assertEqual(policy.load(self.store.root).source_floor, "b" * 40)
        count = len(list((self.store.root / "bundles").iterdir()))
        with self.boundaries():
            result = updater.update(self.store)
        self.assertEqual(result["result"], "up_to_date")
        self.assertEqual(result["generation"], selected)
        self.assertEqual(len(list((self.store.root / "bundles").iterdir())), count)
        self.assertTrue(self.first.path.exists())

    def test_strict_policy_types_and_private_preflight_reports(self):
        configured = self.configure()
        path = self.store.root / policy.NAME
        for change in [
            {"automatic": 1},
            {"format": True},
            {"revision": True},
            {"source_floor": "bad"},
            {"pin_source": "a" * 40},
        ]:
            path.write_text(json.dumps({**configured.value(), **change}))
            with self.assertRaises(ValueError):
                policy.load(self.store.root)
        path.write_text(json.dumps(configured.value()))
        for server in [
            {**self.reply["server"], "native_wire": True},
            {**self.reply["server"], "workspace_api": 99},
            {**self.reply["server"], "instance_id": "../bad"},
        ]:
            with self.assertRaises(ValueError):
                preflight.validate({**self.reply, "server": server}, self.first.manifest)

    def test_policy_refuses_unloadable_config_path_before_creating_its_marker(self):
        bad = self.root / "bad\nhost.json"
        bad.write_bytes(self.config.read_bytes())
        bad.chmod(0o600)
        proposed = policy.Policy(0, False, "stable", None, str(bad), "", "")
        with self.assertRaises(ValueError):
            policy.configure(self.store, proposed, 0)
        self.assertFalse((self.store.root / policy.MARKER).exists())
        self.assertFalse((self.store.root / policy.NAME).exists())
        self.assertEqual(self.store.pointer().current, self.first.generation_id)

    def test_unknown_verified_contract_is_refused_before_executable_downloads(self):
        self.configure()
        self.f.prepare("b" * 40)
        path = self.f.inputs / contract.MANIFEST
        value = json.loads(path.read_bytes())
        value["contracts"]["client_profile_write"] = 6
        path.write_text(json.dumps(value))
        with (
            self.boundaries(),
            patch.object(net, "asset", side_effect=self.asset) as assets,
            patch.object(build_module, "build") as build,
            self.assertRaises(ValueError),
        ):
            updater.update(self.store)
        self.assertEqual(
            [call.args[1] for call in assets.call_args_list],
            [contract.MANIFEST, contract.PROVENANCE],
        )
        build.assert_not_called()
        self.assertEqual(self.store.pointer().current, self.first.generation_id)

    def test_failed_provenance_or_duplicate_preflight_report_cannot_reach_selection(self):
        self.configure()
        self.f.prepare("b" * 40)
        before = (self.store.root / "current.json").read_bytes()
        with (
            self.boundaries(),
            patch.object(contract, "verify_provenance", side_effect=ValueError("proof refused")),
            patch.object(net, "asset", side_effect=self.asset) as assets,
            patch.object(build_module, "build") as build,
            self.assertRaises(ValueError),
        ):
            updater.update(self.store)
        build.assert_not_called()
        self.assertEqual(
            [call.args[1] for call in assets.call_args_list],
            [contract.MANIFEST, contract.PROVENANCE],
        )
        with (
            patch.object(build_module, "run", return_value=b'{"format":1,"format":1}'),
            self.assertRaises(ValueError),
        ):
            preflight.inspect(self.first, {"office": self.root / "profile"})
        self.assertEqual((self.store.root / "current.json").read_bytes(), before)
        self.assertEqual(policy.load(self.store.root).source_floor, "a" * 40)

    def test_disabled_or_stale_worker_never_downloads_or_changes_current_software(self):
        self.configure(True)
        revision, attempt = worker.claim(self.store)
        policy.disable(self.store, revision)
        with patch.object(net, "api") as api:
            result = worker.run(self.store, revision, attempt)
        api.assert_not_called()
        self.assertEqual(result["result"], "refused")
        self.assertEqual(self.store.pointer().current, self.first.generation_id)
        self.assertEqual(worker.status(self.store)["result"], "refused")

    def test_concurrent_real_scheduler_claims_create_only_one_daily_attempt(self):
        self.configure(True)
        script = """import sys,json
sys.path.insert(0,sys.argv[1])
from pathlib import Path
from ccp_mcp_server.managed_store import Store
from ccp_mcp_server.managed_worker import claim
try: result=claim(Store(Path(sys.argv[2])))
except (ValueError,OSError): result=None
print(json.dumps(result))
"""
        # Fixed owned Python and script, operating only on this temporary policy/store.
        children = [
            subprocess.Popen(  # noqa: S603
                [sys.executable, "-c", script, str(ROOT / "mcp/src"), str(self.store.root)],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            for _ in range(2)
        ]
        replies = [child.communicate(timeout=10) for child in children]
        self.assertTrue(all(child.returncode == 0 for child in children))
        claimed = [json.loads(reply[0]) for reply in replies if json.loads(reply[0]) is not None]
        self.assertEqual(len(claimed), 1)
        self.assertEqual(worker.status(self.store)["attempt"], claimed[0][1])

    def test_worker_is_opt_in_daily_detached_and_does_not_erase_confirmed_status_on_replay(self):
        self.configure()
        with (
            fs.locked(self.store.root, shared=True),
            patch.object(worker.subprocess, "Popen") as spawn,
        ):
            worker.schedule(self.store)
        spawn.assert_not_called()
        enabled = self.configure(True)
        with (
            patch.object(worker.time, "time", return_value=1_800_000_000),
            patch.object(worker.subprocess, "Popen") as spawn,
        ):
            worker.schedule(self.store)
            worker.schedule(self.store)
        spawn.assert_called_once()
        argv = spawn.call_args.args[0]
        self.assertEqual(argv[:4], [sys.executable, "-I", "-m", "ccp_mcp_server.managed"])
        self.assertTrue(spawn.call_args.kwargs["start_new_session"])
        self.assertTrue(spawn.call_args.kwargs["close_fds"])
        self.assertEqual(spawn.call_args.kwargs["stdout"], worker.subprocess.DEVNULL)
        state = worker.status(self.store)
        with patch.object(
            updater,
            "update",
            return_value={"result": "up_to_date", "policy_revision": enabled.revision},
        ):
            worker.run(self.store, enabled.revision, state["attempt"])
        confirmed = worker.status(self.store)
        self.assertEqual(confirmed["result"], "up_to_date")
        worker.run(self.store, enabled.revision, state["attempt"])
        self.assertEqual(worker.status(self.store), confirmed)
        with patch.object(worker.time, "time", return_value=1_799_999_999):
            self.assertIsNone(worker.claim(self.store))

    def test_failed_scheduler_and_later_activation_never_change_captured_launch_pair(self):
        self.configure(True)
        with patch.object(worker.subprocess, "Popen", side_effect=OSError("owned spawn failure")):
            worker.schedule(self.store)
        self.assertEqual(worker.status(self.store)["result"], "start_failed")
        second = self.f.stage()

        def schedule(store):
            store.activate(second.generation_id, 1)

        args = managed.parser().parse_args(
            ["--store", str(self.store.root), "client", "--", "workspace", "software-info"]
        )
        with (
            patch.object(worker, "schedule", side_effect=schedule),
            patch.object(managed.os, "execve") as execute,
        ):
            managed.run(args)
        self.assertEqual(execute.call_args.args[0], self.first.path / "client")
        self.assertEqual(
            execute.call_args.args[2]["CCP_CLIENT_BIN"], str(self.first.path / "client")
        )
        self.assertEqual(self.store.pointer().current, second.generation_id)


class ManagedNetworkTests(unittest.TestCase):
    def test_real_urllib_callback_never_follows_redirect_before_manager_validation(self):
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        server.settimeout(5)
        owned = "http://127.0.0.1:" + str(server.getsockname()[1]) + "/owned"
        errors = []

        def reply():
            try:
                connection, _address = server.accept()
                with connection:
                    connection.settimeout(5)
                    self.assertTrue(connection.recv(4096))
                    connection.sendall(
                        b"HTTP/1.1 302 Found\r\nLocation: https://evil.invalid/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
            except (OSError, AssertionError) as error:
                errors.append(str(error))

        validate = net.validate_url
        seen = []

        def owned_boundary(url):
            seen.append(url)
            if url != owned:
                validate(url)

        handler = net.NoRedirect()
        job = threading.Thread(target=reply)
        job.start()
        try:
            # Only the owned initial HTTP URL replaces the production HTTPS boundary.
            # The actual urllib callback must return control before the second URL is opened.
            with (
                patch.object(net, "validate_url", side_effect=owned_boundary),
                patch.object(net, "NoRedirect", return_value=handler),
                patch.object(
                    handler, "redirect_request", wraps=handler.redirect_request
                ) as callback,
                self.assertRaisesRegex(ValueError, "Unexpected official"),
            ):
                net.receive(owned, io.BytesIO(), 32)
            callback.assert_called_once()
            self.assertEqual(seen, [owned, "https://evil.invalid/x"])
        finally:
            job.join(timeout=10)
            server.close()
        self.assertFalse(job.is_alive())
        self.assertEqual(errors, [])

    def test_only_fixed_https_hosts_and_bounded_unencoded_complete_streams_are_allowed(self):
        for value in [
            "http://github.com/x",
            "file:///tmp/x",
            "https://github.com.evil/x",
            "https://user@github.com/x",
            "https://github.com:444/x",
            "https://github.com/x#key",
            "https://github.com/x\n",
        ]:
            with self.assertRaises(ValueError):
                net.validate_url(value)
        for value in [
            "https://github.com/x",
            "https://release-assets.githubusercontent.com/signed?q=1",
        ]:
            net.validate_url(value)

        class Response(io.BytesIO):
            status = 200

        for body, headers in [
            (b"123456", {}),
            (b"123", {"Content-Length": "4"}),
            (b"123", {"Content-Encoding": "gzip"}),
            (b"", {}),
            (b"123", {"Content-Length": "999999999"}),
        ]:
            response = Response(body)
            response.headers = headers
            with self.assertRaises(ValueError):
                net.transfer(response, io.BytesIO(), 5)
        response = Response(b"exact")
        response.headers = {"Content-Length": "5"}
        sink = io.BytesIO()
        net.transfer(response, sink, 5)
        self.assertEqual(sink.getvalue(), b"exact")

    def test_redirects_are_revalidated_and_duplicate_api_json_is_refused(self):
        error = urllib.error.HTTPError(
            net.API, 302, "redirect", {"Location": "https://evil.invalid/x"}, io.BytesIO()
        )
        with patch.object(net.urllib.request, "build_opener") as factory:
            factory.return_value.open.side_effect = error
            with self.assertRaises(ValueError):
                net.receive(net.API + "releases/latest", io.BytesIO(), 32)
            self.assertEqual(factory.return_value.open.call_count, 1)

        def duplicate(_url, sink, _limit):
            sink.write(b'{"tag_name":"v0.1.0","tag_name":"v9.0.0"}')

        with patch.object(net, "receive", side_effect=duplicate), self.assertRaises(ValueError):
            net.api("releases/latest")

    def test_allowed_redirect_streams_fixed_bytes_without_forwarding_credentials(self):
        class Response(io.BytesIO):
            status = 200
            headers = {"Content-Length": "5"}

        redirect = urllib.error.HTTPError(
            net.DOWNLOAD,
            302,
            "redirect",
            {"Location": "https://release-assets.githubusercontent.com/owned?signed=fixture"},
            io.BytesIO(),
        )
        sink = io.BytesIO()
        with patch.object(net.urllib.request, "build_opener") as factory:
            factory.return_value.open.side_effect = [redirect, Response(b"exact")]
            net.receive(net.DOWNLOAD + "v0.1.0/ccp-mcp.tar.gz", sink, 5)
        self.assertEqual(sink.getvalue(), b"exact")
        calls = factory.return_value.open.call_args_list
        self.assertEqual(len(calls), 2)
        self.assertTrue(calls[0].args[0].full_url.startswith(net.DOWNLOAD))
        self.assertTrue(
            calls[1].args[0].full_url.startswith("https://release-assets.githubusercontent.com/")
        )
        self.assertFalse(
            any(
                key.lower() in {"authorization", "cookie"}
                for call in calls
                for key in call.args[0].headers
            )
        )

    def test_discovery_honors_channel_pin_bounds_and_never_uses_dates_for_ordering(self):
        stable = {"tag_name": "v0.1.0", "draft": False, "prerelease": False}
        nightly = {"tag_name": "nightly-1900-01-01-" + "b" * 12, "draft": False, "prerelease": True}
        with patch.object(net, "api", return_value=stable) as api:
            self.assertEqual(release.discover("stable", "v0.1.0"), "v0.1.0")
            api.assert_called_once_with("releases/tags/v0.1.0")
        with patch.object(net, "api", return_value=[stable, nightly]):
            self.assertEqual(release.discover("nightly", None), nightly["tag_name"])
        with (
            patch.object(net, "api", return_value=[stable] * 100) as api,
            self.assertRaises(ValueError),
        ):
            release.discover("nightly", None)
        self.assertEqual(api.call_count, 3)
        for row in [
            {**stable, "draft": True},
            {**stable, "prerelease": True},
            {**stable, "draft": 0},
        ]:
            with patch.object(net, "api", return_value=row), self.assertRaises(ValueError):
                release.discover("stable", "v0.1.0")
        with (
            patch.object(
                net,
                "api",
                return_value={
                    "base_commit": {"sha": "a" * 40},
                    "merge_base_commit": {"sha": "a" * 40},
                    "status": "diverged",
                    "behind_by": 0,
                },
            ),
            self.assertRaises(ValueError),
        ):
            release.ancestry("a" * 40, "b" * 40)
