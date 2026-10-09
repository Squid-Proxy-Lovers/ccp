# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Regression coverage for tool integrity, source discovery, and CLI accounting."""

from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]


def load_script(filename):
    spec = importlib.util.spec_from_file_location(filename, ROOT / "scripts" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def archive(member="tool", symlink=False):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as bundle:
        info = tarfile.TarInfo(member)
        if symlink:
            info.type = tarfile.SYMTYPE
            info.linkname = "outside"
            bundle.addfile(info)
        else:
            info.size = 3
            bundle.addfile(info, io.BytesIO(b"new"))
    return output.getvalue()


class ToolIntegrityTests(unittest.TestCase):
    def install(self, payload, checksum, directory):
        installer = load_script("install-lint-tools.py")
        asset = {"url": "https://github.com/example/tool/release", "sha256": checksum}
        with (
            patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(payload)),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            installer.install("tool", asset, directory)

    def test_verified_binary_replaces_only_selected_executable(self):
        payload = archive("../../tool")
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "bin"
            self.install(payload, hashlib.sha256(payload).hexdigest(), destination)
            self.assertEqual((destination / "tool").read_bytes(), b"new")
            self.assertEqual(list(Path(directory).iterdir()), [destination])

    def test_checksum_failure_preserves_existing_tool(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory)
            (destination / "tool").write_bytes(b"old")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                self.install(archive(), "0" * 64, destination)
            self.assertEqual((destination / "tool").read_bytes(), b"old")

    def test_archive_symlink_cannot_be_installed(self):
        payload = archive(symlink=True)
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory)
            with self.assertRaisesRegex(ValueError, "exactly one"):
                self.install(payload, hashlib.sha256(payload).hexdigest(), destination)
            self.assertEqual(list(destination.iterdir()), [])


class PythonAuditTests(unittest.TestCase):
    def test_audit_requires_the_current_checkout_and_keeps_third_party_versions(self):
        auditor = load_script("audit-python.py")
        local = Mock(metadata={"Name": "ccp-mcp-server"}, version="0.1.0")
        dependency = Mock(metadata={"Name": "example-package"}, version="1.2.3")
        shadow = Mock(metadata={"Name": "ccp-mcp-server"}, version="0.1.0")
        shadow.read_text.return_value = None
        local.read_text.return_value = json.dumps({"url": (auditor.ROOT / "mcp").as_uri()})
        with patch.object(
            auditor.metadata, "distributions", return_value=[local, shadow, dependency]
        ):
            self.assertEqual(auditor.requirements(), ["example-package==1.2.3"])
            local.read_text.return_value = json.dumps({"url": "file:///another-checkout"})
            with self.assertRaisesRegex(RuntimeError, "this checkout"):
                auditor.requirements()


class SourceGateTests(unittest.TestCase):
    def run_check(self, check, files):
        scanner = load_script("check-source.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # Fixed test commands run without a shell; Git initializes only the fixture.
            subprocess.run([shutil.which("git"), "init", "--quiet", str(root)], check=True)  # noqa: S603
            for name, content in files.items():
                (root / name).write_text(content)
            shutil.copyfile(ROOT / ".gitleaks.toml", root / ".gitleaks.toml")
            with patch.object(scanner, "ROOT", root), patch.object(sys, "argv", ["check", check]):
                scanner.main()

    def test_shell_gate_rejects_unquoted_variable_in_extensionless_script(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_check("shell", {"install-helper": '#!/bin/sh\nword="two words"\necho $word\n'})

    def test_secret_gate_rejects_untracked_credentials(self):
        # Deliberately synthetic token, assembled so the test source is not a credential.
        token = "ghp_" + hashlib.sha256(b"synthetic lint regression token").hexdigest()[:36]
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_check("secrets", {"config.json": json.dumps({"token": token})})

    def test_cli_helper_preserves_parent_counts_and_raw_json(self):
        runner = (ROOT / "tests" / "run.sh").read_text()
        helpers = runner[runner.index("RED=") : runner.index("# ── Temp dir")]
        program = (
            helpers
            + """
expect_success success printf '%s' '{"ok":true}'
[[ "$OUT" == '{"ok":true}' && "$PASS_COUNT" == 1 && "$FAIL_COUNT" == 0 ]] || exit 1
expect_success failure bash -c 'exit 7'
[[ "$PASS_COUNT" == 1 && "$FAIL_COUNT" == 1 ]] || exit 1
"""
        )
        # The shell executes checked-in helper code and fixed regression commands.
        subprocess.run([shutil.which("bash"), "-c", program], check=True, capture_output=True)  # noqa: S603


if __name__ == "__main__":
    unittest.main()
