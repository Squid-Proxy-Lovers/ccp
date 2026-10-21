"""Private staging, failure preservation, file CAS and a real pinned child process."""

from __future__ import annotations

import hashlib
import importlib
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from uuid import UUID

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "mcp/src"))
store_module = importlib.import_module("ccp_mcp_server.managed_store")
build_module = importlib.import_module("ccp_mcp_server.managed_build")
archive_module = importlib.import_module("ccp_mcp_server.managed_archive")
fs = importlib.import_module("ccp_mcp_server.managed_fs")
contract = importlib.import_module("ccp_mcp_server.release_contract")


def archive(path, entries=None):
    with tarfile.open(path, "w:gz") as stream:
        for name, kind, body in entries or [
            ("ccp-mcp/pyproject.toml", "file", b"[project]\nname='fixture'"),
            ("ccp-mcp/src/module.py", "file", b"print('exact source')"),
        ]:
            item = tarfile.TarInfo(name)
            if kind == "symlink":
                item.type = tarfile.SYMTYPE
                item.linkname = "/outside"
            elif kind == "hardlink":
                item.type = tarfile.LNKTYPE
                item.linkname = "/outside"
            elif kind == "device":
                item.type = tarfile.CHRTYPE
            else:
                item.size = len(body)
            stream.addfile(item, io.BytesIO(body) if kind == "file" else None)


def fake_build(generation, _manifest):
    # Real bounded archive extraction; only pip/venv health is replaced by an owned IPC fixture.
    archive_module.expand(generation / "release/ccp-mcp.tar.gz", generation / "source")
    fs.directory(generation / "venv/bin", create=True)
    program = generation / "venv/bin/python"
    program.write_text(
        "#!"
        + sys.executable
        + "\nimport os,sys,json\nfor line in sys.stdin:\n print(json.dumps({'binary':os.environ['CCP_CLIENT_BIN'],'config':os.environ.get('CCP_WORKSPACE_CONFIG')}),flush=True)\n"
    )
    program.chmod(0o700)


class ManagedBundleTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.store = store_module.Store(self.root / "managed")
        self.inputs = self.root / "input"
        self.inputs.mkdir()
        for name in contract.ASSETS:
            (self.inputs / name).write_bytes((name + " fixture").encode())
        archive(self.inputs / "ccp-mcp.tar.gz")
        self.prepare()
        (self.inputs / contract.PROVENANCE).write_bytes(b"owned mocked provenance boundary")

    def prepare(self, sha="a" * 40):
        contract.prepare(ROOT, self.inputs, sha)

    def stage(self):
        with (
            patch.object(contract, "verify_provenance"),
            patch.object(build_module, "build", side_effect=fake_build),
        ):
            return self.store.stage(self.inputs, "stable", "v0.1.0")

    def test_private_stage_reopen_activation_noop_and_software_only_rollback(self):
        first = self.stage()
        self.assertEqual(self.store.status()["pointer"]["revision"], 0)
        self.assertEqual(first.path.stat().st_mode & 0o777, 0o700)
        self.assertEqual((first.path / "ready.json").stat().st_mode & 0o777, 0o600)
        self.assertEqual((first.path / "release" / contract.MANIFEST).stat().st_mode & 0o777, 0o600)
        pointer = self.store.activate(first.generation_id, 0)
        self.assertEqual(pointer.revision, 1)
        self.assertEqual(self.store.activate(first.generation_id, 0), pointer)
        second = self.stage()
        pointer = self.store.activate(second.generation_id, 1)
        self.assertEqual(pointer.previous, first.generation_id)
        self.assertEqual(self.store.rollback(2).current, first.generation_id)
        self.assertEqual(self.store.status()["pointer"]["revision"], 3)
        self.assertTrue(first.path.is_dir() and second.path.is_dir())
        with self.assertRaises(ValueError):
            self.store.rollback(2)
        self.assertEqual(store_module.Store(self.store.root).status(), self.store.status())

    def test_failed_proof_build_or_health_preserves_every_active_byte_and_removes_only_new_stage(
        self,
    ):
        first = self.stage()
        self.store.activate(first.generation_id, 0)
        pointer = (self.store.root / "current.json").read_bytes()
        for phase in ["proof", "build"]:
            with (
                patch.object(
                    contract,
                    "verify_provenance",
                    side_effect=ValueError("refused") if phase == "proof" else None,
                ),
                patch.object(
                    build_module,
                    "build",
                    side_effect=ValueError("health failed") if phase == "build" else fake_build,
                ),
                self.assertRaises(ValueError),
            ):
                self.store.stage(self.inputs, "stable", "v0.1.0")
            self.assertEqual((self.store.root / "current.json").read_bytes(), pointer)
            self.assertEqual(
                [p.name for p in (self.store.root / "bundles").iterdir()], [first.generation_id]
            )
            self.assertTrue((first.path / "ready.json").exists())

    def test_client_contract_or_pip_or_mcp_health_failure_never_activates_candidate(self):
        first = self.stage()
        self.store.activate(first.generation_id, 0)
        pointer = (self.store.root / "current.json").read_bytes()
        info = {
            "format": 1,
            "package_version": "0.1.0",
            "native_wire": 1,
            "workspace_api": 1,
            "client_profile_read_min": 1,
            "client_profile_write": 5,
        }
        cases = [
            [json.dumps({**info, "native_wire": True}).encode()],
            [json.dumps({**info, "client_profile_write": 6}).encode()],
            [json.dumps(info).encode(), b"", ValueError("pip failed")],
            [json.dumps(info).encode(), b"", b"", ValueError("MCP health failed")],
        ]
        for replies in cases:
            with (
                patch.object(contract, "verify_provenance"),
                patch.object(build_module, "run", side_effect=replies),
                self.assertRaises(ValueError),
            ):
                self.store.stage(self.inputs, "stable", "v0.1.0")
            self.assertEqual((self.store.root / "current.json").read_bytes(), pointer)
            self.assertEqual(
                [p.name for p in (self.store.root / "bundles").iterdir()], [first.generation_id]
            )

    def test_copy_then_verify_never_installs_again_from_mutable_download_directory(self):
        original = (self.inputs / "ccp-mcp.tar.gz").read_bytes()

        def verified(directory, _manifest, _ref):
            self.assertNotEqual(directory, self.inputs)
            self.assertEqual((directory / "ccp-mcp.tar.gz").read_bytes(), original)
            (self.inputs / "ccp-mcp.tar.gz").write_bytes(b"substituted caller download")

        with (
            patch.object(contract, "verify_provenance", side_effect=verified),
            patch.object(build_module, "build", side_effect=fake_build),
        ):
            generation = self.store.stage(self.inputs, "stable", "v0.1.0")
        self.assertEqual((generation.path / "release/ccp-mcp.tar.gz").read_bytes(), original)
        self.assertTrue((generation.path / "source/ccp-mcp/src/module.py").exists())

    def test_build_cannot_substitute_client_and_manifest_after_private_verification(self):
        first = self.stage()
        self.store.activate(first.generation_id, 0)
        before = (self.store.root / "current.json").read_bytes()

        def substitute(generation, manifest):
            fake_build(generation, manifest)
            client = generation / "client"
            client.write_bytes(b"substituted after verification")
            path = generation / "release" / contract.MANIFEST
            data = json.loads(path.read_bytes())
            data["artifacts"]["ccp-client-" + build_module.host_platform()] = {
                "size": client.stat().st_size,
                "sha256": hashlib.sha256(client.read_bytes()).hexdigest(),
            }
            path.write_text(json.dumps(data))

        with (
            patch.object(contract, "verify_provenance"),
            patch.object(build_module, "build", side_effect=substitute),
            self.assertRaises(ValueError),
        ):
            self.store.stage(self.inputs, "stable", "v0.1.0")
        self.assertEqual((self.store.root / "current.json").read_bytes(), before)
        self.assertEqual(
            [p.name for p in (self.store.root / "bundles").iterdir()], [first.generation_id]
        )

    def test_changed_bytes_or_symlink_inputs_never_reach_build_or_touch_active_pointer(self):
        first = self.stage()
        self.store.activate(first.generation_id, 0)
        client = self.inputs / ("ccp-client-" + build_module.host_platform())
        client.write_bytes(b"corrupt client")
        with (
            patch.object(contract, "verify_provenance"),
            patch.object(build_module, "build") as builder,
            self.assertRaises(ValueError),
        ):
            self.store.stage(self.inputs, "stable", "v0.1.0")
        builder.assert_not_called()
        client.unlink()
        client.symlink_to(self.inputs / "build-info.json")
        with self.assertRaises(ValueError):
            self.stage()
        self.assertEqual(self.store.pointer().current, first.generation_id)

    def test_unsupported_contract_and_corrupt_ready_or_client_never_activate(self):
        generation = self.stage()
        path = generation.path / "release" / contract.MANIFEST
        value = json.loads(path.read_text())
        value["contracts"]["client_profile_write"] = 6
        path.write_text(json.dumps(value))
        with self.assertRaises(ValueError):
            self.store.activate(generation.generation_id, 0)
        self.assertEqual(self.store.pointer().revision, 0)
        with self.assertRaises(ValueError):
            build_module.compatible(contract.decode(path.read_bytes()))
        with self.assertRaises(ValueError):
            self.store.activate("../escape", 0)
        self.assertIsNone(self.store.pointer().current)
        self.assertEqual(self.store.pointer().revision, 0)

    def test_loose_permissions_symlink_metadata_and_invalid_pointer_do_not_regenerate(self):
        generation = self.stage()
        self.store.activate(generation.generation_id, 0)
        path = self.store.root / "current.json"
        original = path.read_bytes()
        path.chmod(0o644)
        with self.assertRaises(ValueError):
            self.store.status()
        path.chmod(0o600)
        for value in [
            {"format": 1, "revision": 1, "current": None, "previous": None},
            {"format": True, "revision": 1, "current": generation.generation_id, "previous": None},
            {"format": 1, "revision": True, "current": generation.generation_id, "previous": None},
        ]:
            path.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                self.store.status()
        path.write_bytes(original)
        path.unlink()
        with self.assertRaises(ValueError):
            self.store.status()
        self.assertFalse(path.exists())
        path.symlink_to(generation.path / "ready.json")
        with self.assertRaises(ValueError):
            self.store.status()
        self.assertTrue(path.is_symlink())
        self.assertTrue((generation.path / "ready.json").exists())

    def test_generation_name_collision_cannot_delete_or_change_an_existing_ready_bundle(self):
        generation = self.stage()
        original = (generation.path / "ready.json").read_bytes()
        with (
            patch.object(store_module, "uuid4", return_value=UUID(generation.generation_id)),
            self.assertRaises(FileExistsError),
        ):
            self.stage()
        self.assertEqual((generation.path / "ready.json").read_bytes(), original)

    def test_pointer_failure_after_rename_retains_both_generations_and_inspection_resolves_state(
        self,
    ):
        first, second = self.stage(), self.stage()
        self.store.activate(first.generation_id, 0)
        original_sync = fs.sync

        def failure(path):
            if path == self.store.root:
                raise OSError("fixture directory sync failure after rename")
            original_sync(path)

        with patch.object(fs, "sync", side_effect=failure), self.assertRaises(OSError):
            self.store.activate(second.generation_id, 1)
        self.assertTrue(first.path.exists() and second.path.exists())
        self.assertEqual(self.store.status()["pointer"]["current"], second.generation_id)
        self.assertEqual(self.store.activate(second.generation_id, 1).revision, 2)

    def test_real_child_keeps_original_client_and_external_config_after_new_activation(self):
        first, second = self.stage(), self.stage()
        self.store.activate(first.generation_id, 0)
        with patch.dict(
            os.environ,
            CCP_WORKSPACE_CONFIG="/unchanged/private/config",
            CCP_CLIENT_BIN="/wrong/moving/client",
        ):
            executable, arguments, environment = self.store.launch("mcp", ())
        # Fixed captured executable/argv belong to the owned private fixture.
        child = subprocess.Popen(  # noqa: S603
            arguments,
            executable=executable,
            env=environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )
        try:
            child.stdin.write("before\n")
            child.stdin.flush()
            before = json.loads(child.stdout.readline())
            self.store.activate(second.generation_id, 1)
            child.stdin.write("after\n")
            child.stdin.flush()
            after = json.loads(child.stdout.readline())
            self.assertEqual(before, after)
            self.assertEqual(after["binary"], str(first.path / "client"))
            self.assertEqual(after["config"], "/unchanged/private/config")
            self.assertEqual(self.store.launch("mcp", ())[0], second.path / "venv/bin/python")
        finally:
            child.stdin.close()
            child.wait(timeout=10)
            child.stdout.close()

    def test_different_concurrent_cas_activations_accept_only_one(self):
        first, a, b = self.stage(), self.stage(), self.stage()
        self.store.activate(first.generation_id, 0)
        script = "import sys;sys.path.insert(0,sys.argv[1]);from pathlib import Path;from ccp_mcp_server.managed_store import Store;Store(Path(sys.argv[2])).activate(sys.argv[3],1)"
        # These children call only the owned fixture store with fixed Python/argv.
        children = [
            subprocess.Popen(  # noqa: S603
                [
                    sys.executable,
                    "-c",
                    script,
                    str(ROOT / "mcp/src"),
                    str(self.store.root),
                    item.generation_id,
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            for item in [a, b]
        ]
        outputs = [child.communicate(timeout=10) for child in children]
        self.assertEqual(sorted(child.returncode for child in children), [0, 1])
        self.assertEqual(self.store.pointer().revision, 2)
        self.assertIn(self.store.pointer().current, [a.generation_id, b.generation_id])
        self.assertTrue(all(item.path.exists() for item in [first, a, b]))
        self.assertTrue(any(output[1] for output in outputs))

    def test_shared_reader_allows_child_launch_and_status_but_refuses_writers(self):
        import fcntl

        first, second = self.stage(), self.stage()
        self.store.activate(first.generation_id, 0)
        before = (self.store.root / "current.json").read_bytes()
        script = "import sys,json;sys.path.insert(0,sys.argv[1]);from pathlib import Path;from ccp_mcp_server.managed_store import Store;s=Store(Path(sys.argv[2]));print(json.dumps({'client':str(s.launch('client',())[0]),'mcp':str(s.launch('mcp',())[0]),'status':s.status()}))"
        with (self.store.root / ".lock").open("rb") as reader:
            fcntl.flock(reader, fcntl.LOCK_SH | fcntl.LOCK_NB)
            # Fixed Python and script inspect only this owned temporary store.
            child = subprocess.run(  # noqa: S603
                [sys.executable, "-c", script, str(ROOT / "mcp/src"), str(self.store.root)],
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(child.returncode, 0, child.stderr)
            result = json.loads(child.stdout)
            self.assertEqual(result["client"], str(first.path / "client"))
            self.assertEqual(result["mcp"], str(first.path / "venv/bin/python"))
            self.assertEqual(result["status"]["pointer"]["current"], first.generation_id)
            for writer in [
                lambda: self.store.stage(self.inputs, "stable", "v0.1.0"),
                lambda: self.store.activate(second.generation_id, 1),
                lambda: self.store.rollback(1),
            ]:
                with self.assertRaisesRegex(ValueError, "Another managed"):
                    writer()
            self.assertEqual((self.store.root / "current.json").read_bytes(), before)
        self.assertEqual(self.store.activate(second.generation_id, 1).revision, 2)

    def test_readers_never_create_a_store_lock_or_initial_pointer(self):
        for state in ["missing", "empty", "lock_only"]:
            store = store_module.Store(self.root / state)
            if state != "missing":
                fs.directory(store.root, create=True)
            if state == "lock_only":
                with fs.locked(store.root):
                    pass
            before = sorted(store.root.iterdir()) if store.root.exists() else None
            for read in [
                store.status,
                lambda store=store: store.launch("client", ()),
                lambda store=store: store.launch("mcp", ()),
            ]:
                with self.assertRaises((ValueError, OSError)):
                    read()
                self.assertEqual(
                    sorted(store.root.iterdir()) if store.root.exists() else None, before
                )
                self.assertFalse((store.root / "current.json").exists())

    def test_archive_traversal_links_devices_duplicates_and_limits_are_refused(self):
        entries = [
            [(".", "file", b"bad")],
            [("../outside", "file", b"bad")],
            [("/outside", "file", b"bad")],
            [("ccp-mcp/../outside", "file", b"bad")],
            [("ccp-mcp/link", "symlink", b"")],
            [("ccp-mcp/link", "hardlink", b"")],
            [("ccp-mcp/device", "device", b"")],
            [
                ("ccp-mcp/pyproject.toml", "file", b"first"),
                ("ccp-mcp/pyproject.toml", "file", b"duplicate"),
            ],
            [("ccp-mcp/bad\\name", "file", b"bad")],
        ]
        for index, members in enumerate(entries):
            path = self.root / f"archive-{index}.tar.gz"
            archive(path, members)
            with self.subTest(index=index), self.assertRaises((ValueError, OSError)):
                archive_module.expand(path, self.root / f"unpack-{index}")
        self.assertFalse((self.root / "outside").exists())
        path = self.root / "large.tar.gz"
        archive(path, [("ccp-mcp/pyproject.toml", "file", b"x" * 16384)])
        with patch.object(archive_module, "MAX_EXPANDED", 8192), self.assertRaises(ValueError):
            archive_module.expand(path, self.root / "large")
