#!/usr/bin/env python3
"""Exercise the real installer with isolated Docker/Git command doubles."""

import json
import os
import shutil
import subprocess
import sys
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


class NativeSourceInstallTests(unittest.TestCase):
    @staticmethod
    def prepare_alias(installed, binaries, alias):
        if alias == "foreign":
            (installed / "ccp").write_text("foreign tool")
        elif alias == "dangling":
            (installed / "ccp").symlink_to("other-tool")
        elif alias == "own":
            (installed / "ccp").symlink_to("ccp-client")
        elif alias == "external":
            (binaries / "ccp").write_text("#!/bin/sh\nexit 0\n")
            (binaries / "ccp").chmod(0o755)

    def run_source(self, *, client=False, failure="", alias=""):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts, binaries, home = root / "scripts", root / "bin", root / "home"
            for path in (scripts, binaries, home):
                path.mkdir()
            installer = scripts / "install.sh"
            shutil.copyfile(ROOT / "install.sh", installer)
            installed = home / ".local/bin"
            installed.mkdir(parents=True)
            (installed / "ccp-client").write_text("previous client")
            self.prepare_alias(installed, binaries, alias)
            log = root / "calls.jsonl"
            stub = r"""
import json, os, pathlib, sys
command = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["CCP_SOURCE_TEST_LOG"], "a") as stream:
    stream.write(json.dumps([command, *args]) + "\n")
if command == os.environ["CCP_SOURCE_TEST_FAILURE"]:
    sys.exit(73)
if command == "node":
    if os.environ["CCP_SOURCE_TEST_FAILURE"] == "old-node":
        sys.exit(1)
elif command == "git":
    checkout = pathlib.Path(args[-1])
    (checkout / "Cargo.toml").parent.mkdir(parents=True, exist_ok=True)
    (checkout / "Cargo.toml").touch()
    (checkout / "mcp").mkdir()
    (checkout / "crates/server/web").mkdir(parents=True)
elif command == "npm":
    assert args[0] == "ci" and "--ignore-scripts" in args
    (pathlib.Path(args[args.index("--prefix") + 1]) / "node_modules").mkdir()
elif command == "cargo":
    if os.environ["CCP_SOURCE_TEST_CLIENT"] != "1":
        assert pathlib.Path("crates/server/web/node_modules").is_dir(), "frontend not installed first"
    for name in ("client", "server"):
        binary = pathlib.Path("target/release") / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_text("#!/bin/sh\nexit 0\n")
elif command == "python3":
    assert args[:2] == ["-m", "venv"]
    pip = pathlib.Path(args[2]) / "bin/pip"
    pip.parent.mkdir(parents=True, exist_ok=True)
    pip.write_text("#!/bin/sh\nexit 0\n")
    pip.chmod(0o755)
"""
            for command in ("git", "cargo", "node", "npm", "python3"):
                path = binaries / command
                path.write_text("#!" + sys.executable + "\n" + stub)
                path.chmod(0o755)
            env = dict(
                os.environ,
                PATH=f"{binaries}:/usr/bin:/bin",
                HOME=str(home),
                TMPDIR=str(root),
                CCP_SOURCE_TEST_LOG=str(log),
                CCP_SOURCE_TEST_FAILURE=failure,
                CCP_SOURCE_TEST_CLIENT="1" if client else "0",
            )
            args = ["bash", str(installer), "--from-source"]
            if client:
                args.append("--client")
            # Only tool doubles run; child HOME and session isolate all installer effects.
            result = subprocess.run(  # noqa: S603
                args,
                cwd=root,
                env=env,
                capture_output=True,
                text=True,
                start_new_session=True,
                timeout=20,
            )
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            state = {
                "client": (installed / "ccp-client").read_text(),
                "server": (installed / "ccp-server").exists(),
            }
            alias_path = installed / "ccp"
            if alias and alias != "own":
                self.assertIn("existing ccp", result.stdout)
                if alias == "foreign":
                    self.assertEqual(alias_path.read_text(), "foreign tool")
                elif alias == "dangling":
                    self.assertEqual(os.readlink(alias_path), "other-tool")
                else:
                    self.assertFalse(alias_path.exists())
                    self.assertEqual((binaries / "ccp").read_text(), "#!/bin/sh\nexit 0\n")
            elif result.returncode == 0:
                self.assertTrue(alias_path.is_symlink())
                self.assertEqual(os.readlink(alias_path), "ccp-client")
            return result, calls, state

    def test_alias_never_replaces_an_existing_file_or_dangling_symlink(self):
        for alias in ("foreign", "dangling", "external", "own"):
            with self.subTest(alias=alias):
                result, _, _ = self.run_source(client=True, alias=alias)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_fresh_source_server_installs_locked_frontend_before_cargo(self):
        result, calls, state = self.run_source()
        self.assertEqual(result.returncode, 0, result.stderr)
        names = [call[0] for call in calls]
        self.assertLess(names.index("npm"), names.index("cargo"))
        npm = calls[names.index("npm")]
        self.assertEqual(npm[1], "ci")
        self.assertIn("--ignore-scripts", npm)
        cargo = calls[names.index("cargo")]
        self.assertEqual(
            cargo[1:], ["build", "--release", "--locked", "-p", "client", "-p", "server"]
        )
        self.assertTrue(state["server"])
        self.assertNotEqual(state["client"], "previous client")

    def test_client_source_build_does_not_need_frontend_or_node(self):
        result, calls, state = self.run_source(client=True, failure="node")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] in ("npm", "node") for call in calls))
        cargo = next(call for call in calls if call[0] == "cargo")
        self.assertEqual(cargo[1:], ["build", "--release", "--locked", "-p", "client"])
        self.assertFalse(state["server"])
        self.assertNotEqual(state["client"], "previous client")

    def test_frontend_or_compiler_failure_keeps_installed_binaries(self):
        for failure in ("npm", "cargo", "old-node"):
            with self.subTest(failure=failure):
                result, calls, state = self.run_source(failure=failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Done.", result.stdout)
                self.assertEqual(state, {"client": "previous client", "server": False})
                if failure != "cargo":
                    self.assertFalse(any(call[0] == "cargo" for call in calls))
                if failure == "old-node":
                    self.assertIn("Node.js 24", result.stdout)
                    self.assertFalse(any(call[0] == "npm" for call in calls))


if __name__ == "__main__":
    unittest.main()
