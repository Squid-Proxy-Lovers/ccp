#!/usr/bin/env python3
"""Exercise the real installer with isolated Docker/Git command doubles."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class DockerInstallTests(unittest.TestCase):
    def run_installer(self, *, checkout=False, failure="", source=False, overrides=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / "scripts"
            scripts.mkdir()
            installer = scripts / "install.sh"
            shutil.copyfile(ROOT / "install.sh", installer)
            if checkout:
                (scripts / "Dockerfile").touch()
                (scripts / "Cargo.toml").touch()
            bin_dir = root / "bin"
            bin_dir.mkdir()
            log = root / "commands.jsonl"
            stub = """#!/usr/bin/env python3
import json, os, pathlib, sys
command = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['INSTALL_TEST_LOG'], 'a') as log:
    log.write(json.dumps([command, *args]) + '\\n')
operation = args[0] if command == 'docker' else 'clone'
if operation == os.environ.get('INSTALL_TEST_FAILURE'):
    print('simulated ' + operation + ' failure', file=sys.stderr)
    sys.exit(1)
if command == 'git':
    target = pathlib.Path(args[-1])
    (target / 'Dockerfile').touch()
    (target / 'Cargo.toml').touch()
"""
            for command in ("docker", "git"):
                path = bin_dir / command
                path.write_text(stub)
                path.chmod(0o755)
            env = os.environ.copy()
            for name in (
                "CCP_AUTH_PORT",
                "CCP_MTLS_PORT",
                "CCP_ADVERTISE_HOST",
                "CCP_PUBLISH_HOST",
            ):
                env.pop(name, None)
            env.update(
                PATH=f"{bin_dir}:{env['PATH']}",
                INSTALL_TEST_LOG=str(log),
                INSTALL_TEST_FAILURE=failure,
                TMPDIR=str(root),
            )
            env.update(overrides or {})
            args = ["bash", str(installer), "--docker", "--session", "session with spaces"]
            if source:
                args.append("--from-source")
            # Fixed installer argv runs with isolated Docker/Git doubles, without a shell.
            result = subprocess.run(args, cwd=root, env=env, capture_output=True, text=True)  # noqa: S603
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            builds = [call for call in calls if call[:2] == ["docker", "build"]]
            for call in builds:
                if not checkout:
                    self.assertFalse(Path(call[-1]).exists(), "temporary source must be cleaned")
            return result, calls

    def test_pulled_image_runs_without_compose_or_checkout(self):
        result, calls = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[1] for call in calls], ["info", "pull", "run"])
        run = calls[-1]
        self.assertEqual(run[-1], "ghcr.io/squid-proxy-lovers/ccp:latest")
        self.assertIn("CCP_SESSION_NAME=session with spaces", run)
        self.assertIn("127.0.0.1:1337:1337", run)
        self.assertIn("127.0.0.1:1338:1338", run)
        self.assertIn("ccp-server-data:/var/lib/ccp/server", run)

    def test_custom_ports_and_advertised_host(self):
        result, calls = self.run_installer(
            overrides={
                "CCP_AUTH_PORT": "2337",
                "CCP_MTLS_PORT": "2338",
                "CCP_PUBLISH_HOST": "192.0.2.10",
                "CCP_ADVERTISE_HOST": "ccp.example",
            }
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        run = calls[-1]
        for value in (
            "CCP_AUTH_PORT=2337",
            "CCP_MTLS_PORT=2338",
            "CCP_ADVERTISE_HOST=ccp.example",
            "192.0.2.10:2337:2337",
            "192.0.2.10:2338:2338",
        ):
            self.assertIn(value, run)

    def test_failed_pull_suggests_build_without_cloning_or_starting(self):
        for checkout in (False, True):
            with self.subTest(checkout=checkout):
                result, calls = self.run_installer(checkout=checkout, failure="pull")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual([call[1] for call in calls], ["info", "pull"])
                self.assertIn("--docker --from-source --session", result.stdout)
                self.assertIn("simulated pull failure", result.stderr)
                self.assertNotIn("Server container started.", result.stdout)

    def test_explicit_checkout_build_does_not_clone(self):
        result, calls = self.run_installer(checkout=True, source=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[1] for call in calls], ["info", "build", "run"])
        self.assertEqual(calls[-1][-1], "cephalopod-coordination-protocol-server:local")

    def test_from_source_skips_pull(self):
        result, calls = self.run_installer(source=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[:2] == ["docker", "pull"] for call in calls))
        self.assertEqual([call[1] for call in calls], ["info", "clone", "build", "run"])
        self.assertEqual(calls[-1][-1], "cephalopod-coordination-protocol-server:local")

    def test_failures_exit_without_success_message(self):
        for failure in ("info", "clone", "build", "run"):
            with self.subTest(failure=failure):
                result, calls = self.run_installer(
                    source=failure in ("clone", "build"), failure=failure
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Server container started.", result.stdout)
                if failure != "run":
                    self.assertFalse(any(call[:2] == ["docker", "run"] for call in calls))


if __name__ == "__main__":
    unittest.main()
