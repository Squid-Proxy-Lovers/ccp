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


class TestIntegrityGateTests(unittest.TestCase):
    BASE = "#[test]\nfn checks() {\n    assert_eq!(1, 1);\n}\n"

    def run_check(self, change, allow=False):
        checker = load_script("check-test-integrity.py")
        git = shutil.which("git")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lib.rs").write_text(self.BASE)
            # Fixed Git commands initialize and commit only the fixture repository.
            for command in (
                ["init", "--quiet"],
                ["add", "lib.rs"],
                [
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "-m",
                    "base",
                ],
            ):
                subprocess.run([git, *command], cwd=root, check=True)  # noqa: S603
            (root / "lib.rs").write_text(change)
            environment = {"CCP_ALLOW_TEST_REDUCTION": "1"} if allow else {}
            with (
                patch.object(checker, "ROOT", root),
                patch.object(sys, "argv", ["check", "--base", "HEAD"]),
                patch.dict("os.environ", environment),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                checker.main()

    def test_unchanged_and_added_checks_pass(self):
        self.run_check(self.BASE)
        self.run_check(self.BASE + "#[test]\nfn more() {\n    assert!(true);\n}\n")

    def test_removed_assertion_fails(self):
        with self.assertRaises(SystemExit):
            self.run_check("#[test]\nfn checks() {}\n")

    def test_unrelated_additions_cannot_offset_file_reductions(self):
        checker = load_script("check-test-integrity.py")
        base = {"original.rs": checker.count("original.rs", self.BASE)}
        for replacement in ("", "#[test]\nfn checks() {}\n"):
            with self.subTest(replacement=replacement), contextlib.redirect_stdout(io.StringIO()):
                head = {
                    "original.rs": checker.count("original.rs", replacement),
                    "unrelated.rs": checker.count("unrelated.rs", self.BASE),
                }
                problems = checker.compare(base, head)
                self.assertIn("original.rs: assertions 1 -> 0", problems)
                if not replacement:
                    self.assertIn("original.rs: tests 1 -> 0", problems)

    def test_removed_skip_cannot_offset_a_new_skip_elsewhere(self):
        checker = load_script("check-test-integrity.py")
        skipped = self.BASE.replace("#[test]", "#[test]\n#[ignore]")
        base = {"original.rs": checker.count("original.rs", skipped)}
        head = {
            "original.rs": checker.count("original.rs", self.BASE),
            "unrelated.rs": checker.count("unrelated.rs", skipped),
        }
        with contextlib.redirect_stdout(io.StringIO()):
            problems = checker.compare(base, head)
        self.assertIn("unrelated.rs: skips 0 -> 1", problems)

    def test_new_ignore_fails_unless_reviewed(self):
        ignored = self.BASE.replace("#[test]", "#[test]\n#[ignore]")
        with self.assertRaises(SystemExit):
            self.run_check(ignored)
        self.run_check(ignored, allow=True)


class NativeResolverTests(unittest.TestCase):
    def test_cold_checkout_selects_each_binary_without_changing_configured_commands(self):
        from ccp_mcp_server import server

        with tempfile.TemporaryDirectory() as directory:
            for component in ("client", "server"):
                with self.subTest(component=component):
                    key = f"CCP_{component.upper()}_BIN"
                    resolver = getattr(server, f"_resolve_{component}_command")
                    with (
                        patch.dict("os.environ", {key: ""}),
                        patch.object(
                            server,
                            f"DEFAULT_{component.upper()}_BINARY",
                            Path(directory) / component,
                        ),
                        patch.object(
                            server, f"DEFAULT_{component.upper()}_MANIFEST", ROOT / "Cargo.toml"
                        ),
                        patch.object(server.shutil, "which", return_value="/trusted/cargo"),
                    ):
                        self.assertEqual(
                            resolver().argv,
                            [
                                "/trusted/cargo",
                                "run",
                                "--quiet",
                                "--manifest-path",
                                str(ROOT / "Cargo.toml"),
                                "--bin",
                                component,
                                "--",
                            ],
                        )
                        with patch.dict("os.environ", {key: "/configured/binary"}):
                            self.assertEqual(resolver().argv, ["/configured/binary"])


class CoverageGateTests(unittest.TestCase):
    def run_check(self, percent):
        checker = load_script("check-coverage.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "coverage.json"
            report.write_text(json.dumps({"data": [{"totals": {"lines": {"percent": percent}}}]}))
            baseline = root / "baseline.json"
            baseline.write_text(json.dumps({"rust_line_percent": 60.0}))
            with (
                patch.object(checker, "BASELINE", baseline),
                patch.object(checker, "ROOT", root),
                patch.object(sys, "argv", ["check", str(report)]),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                checker.main()

    def test_coverage_within_tolerance_passes(self):
        self.run_check(59.9)
        self.run_check(75.0)

    def test_coverage_drop_fails(self):
        with self.assertRaises(SystemExit):
            self.run_check(59.0)


if __name__ == "__main__":
    unittest.main()
