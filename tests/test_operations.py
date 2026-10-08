"""Exercise script boundaries with isolated homes and mocked external programs."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ScriptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.bin = self.directory / "bin"
        self.bin.mkdir()
        self.log = self.directory / "calls.jsonl"
        self.environment = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                                CALL_LOG=str(self.log), CCP_SERVER_URL="http://configured.test:4312/",
                                CCP_ADMIN_KEY="admin-test", CCP_CLIENT_KEY="client-test")

    def executable(self, name, body):
        path = self.bin / name
        path.write_text(f"#!{sys.executable}\n" + body)
        path.chmod(0o755)
        return path

    def capture(self, name):
        return self.executable(name, "import json, os, sys\n"
                               "with open(os.environ['CALL_LOG'], 'a') as stream:\n"
                               "    stream.write(json.dumps({'args': sys.argv[1:], "
                               "'listener': os.getenv('CCP_HTTP_LISTENER_ADDR'), "
                               "'data': os.getenv('CCP_SERVER_DATA_DIR')}) + '\\n')\n"
                               "print('{}')\n")

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def run_script(self, script, *arguments, environment=None):
        return subprocess.run(["sh", str(ROOT / script), *arguments],
                              env=environment or self.environment,
                              capture_output=True, text=True, timeout=15)

    def test_management_encodes_selector_and_serializes_json(self):
        self.capture("curl")
        selector = 'topic /?#"\\\n雪'
        response = self.run_script("scripts/ccp-manage", "stats", selector)
        self.assertEqual(response.returncode, 0, response.stderr)
        url = self.calls()[0]["args"][-1]
        self.assertEqual(url, "http://configured.test:4312/v1/admin/sessions/"
                         "topic%20%2F%3F%23%22%5C%0A%E9%9B%AA/stats")
        response = self.run_script("scripts/ccp-manage", "add", selector)
        self.assertEqual(response.returncode, 0, response.stderr)
        arguments = self.calls()[-1]["args"]
        self.assertEqual(json.loads(arguments[arguments.index("--data") + 1]),
                         {"session_name": selector})
        self.assertIn("X-CCP-Admin-Key: admin-test", arguments)

    def test_management_requires_explicit_admin_key(self):
        self.capture("curl")
        environment = self.environment.copy()
        environment.pop("CCP_ADMIN_KEY")
        response = self.run_script("scripts/ccp-manage", "stats", "topic", environment=environment)
        self.assertNotEqual(response.returncode, 0)
        self.assertIn("Set CCP_ADMIN_KEY", response.stderr)
        self.assertFalse(self.log.exists())

    def test_failed_update_download_does_not_execute_partial_script(self):
        marker = self.directory / "executed"
        self.executable("curl", "from pathlib import Path\nimport sys\n"
                        f"Path(sys.argv[sys.argv.index('-o') + 1]).write_text('touch {marker}\\n')\n"
                        "sys.exit(22)\n")
        response = self.run_script("scripts/ccp-update")
        self.assertNotEqual(response.returncode, 0)
        self.assertFalse(marker.exists())

    def test_entrypoint_exports_configuration_for_all_invocation_modes(self):
        server = self.capture("server")
        environment = dict(self.environment, CCP_SERVER_BIN=str(server),
                           CCP_SERVER_DATA_DIR=str(self.directory / "data"),
                           CCP_HTTP_PORT="4444", CCP_ADVERTISE_HOST="host.test")
        for arguments, expected in [((), []), (("server", "--help"), ["--help"]),
                                    (("create-session", "topic"), ["create-session", "topic"]),
                                    (("topic", "--help"), ["topic", "--help"])]:
            response = self.run_script("docker/server-entrypoint.sh", *arguments, environment=environment)
            self.assertEqual(response.returncode, 0, response.stderr)
            call = self.calls()[-1]
            self.assertEqual(call["args"], expected)
            self.assertEqual(call["listener"], "0.0.0.0:4444")
            self.assertEqual(call["data"], str(self.directory / "data"))
        environment["CCP_SESSION_NAME"] = "initial-topic"
        response = self.run_script("docker/server-entrypoint.sh", environment=environment)
        self.assertEqual(response.returncode, 0, response.stderr)
        self.assertEqual(self.calls()[-1]["args"], ["initial-topic"])

    def test_hosted_setup_propagates_endpoint_key_and_client_path(self):
        home = self.directory / "home"
        home.mkdir()
        install = self.directory / "installed client"
        environment = dict(self.environment, HOME=str(home), CCP_INSTALL_DIR=str(install))
        client_body = (f"#!{sys.executable}\nimport json, os, sys\n"
                       "with open(os.environ['CALL_LOG'], 'a') as stream:\n"
                       "    stream.write(json.dumps({'program':'client','args':sys.argv[1:],"
                       "'server':os.getenv('CCP_SERVER_URL'),'key':os.getenv('CCP_CLIENT_KEY')}) + '\\n')\n")
        self.executable("curl", "from pathlib import Path\nimport sys\n"
                        "destination=Path(sys.argv[sys.argv.index('-o') + 1])\n"
                        f"destination.write_text({client_body!r} if '/downloads/ccp-client-' in sys.argv[2] else '#!/bin/sh\\nexit 0\\n')\n")
        self.executable("python3", "import sys\nfrom pathlib import Path\n"
                        "if sys.argv[1:3] == ['-m', 'venv']:\n"
                        "    bindir=Path(sys.argv[3])/'bin'; bindir.mkdir(parents=True, exist_ok=True)\n"
                        "    for name in ('python', 'pip', 'ccp-mcp-server'):\n"
                        "        path=bindir/name; path.write_text('#!/bin/sh\\nexit 0\\n'); path.chmod(0o755)\n")
        self.capture("codex")
        self.capture("claude")
        response = self.run_script("scripts/setup-client.sh", environment=environment)
        self.assertEqual(response.returncode, 0, response.stderr)
        calls = self.calls()
        self.assertEqual(calls[0]["args"], ["subscribe-all", "--server", "http://configured.test:4312"])
        self.assertEqual(calls[0]["server"], "http://configured.test:4312")
        self.assertEqual(calls[0]["key"], "client-test")
        additions = [call for call in calls if call["args"][:3] == ["mcp", "add", "ccp"]]
        self.assertEqual(len(additions), 2)
        for call in additions:
            self.assertIn(f"CCP_CLIENT_BIN={install}/ccp-client", call["args"])
            self.assertIn("CCP_SERVER_URL=http://configured.test:4312", call["args"])
            self.assertIn("CCP_CLIENT_KEY=client-test", call["args"])

    def test_failed_setup_download_preserves_installed_binary(self):
        install = self.directory / "installed"
        install.mkdir()
        client = install / "ccp-client"
        client.write_text("original binary")
        self.executable("curl", "from pathlib import Path\nimport sys\n"
                        "Path(sys.argv[sys.argv.index('-o') + 1]).write_text('partial binary')\n"
                        "sys.exit(22)\n")
        environment = dict(self.environment, CCP_INSTALL_DIR=str(install))
        response = self.run_script("scripts/setup-client.sh", environment=environment)
        self.assertNotEqual(response.returncode, 0)
        self.assertEqual(client.read_text(), "original binary")
        self.assertEqual(list(install.glob('.ccp-download.*')), [])

    def test_source_installer_preserves_paths_and_fails_failed_builds(self):
        checkout = self.directory / "checkout"
        checkout.mkdir()
        shutil.copy(ROOT / "install.sh", checkout / "install.sh")
        (checkout / "Cargo.toml").write_text("# test checkout\n")
        (checkout / "mcp").mkdir()
        home = self.directory / "home' with spaces"
        home.mkdir()
        (home / ".claude.json").write_text('{"existing": true}')
        (home / ".cursor").mkdir()
        (home / ".cursor/mcp.json").write_text('{"mcpServers":{"other":{"command":"existing"}}}')
        (home / ".codex").mkdir()
        (home / ".codex/config.toml").write_text("# existing config\n")
        install = home / 'bin"quoted'
        self.executable("cargo", "from pathlib import Path\nimport sys\n"
                        "target=Path('target/release'); target.mkdir(parents=True, exist_ok=True)\n"
                        "for name in ('client', 'server'):\n"
                        "    path=target/name; path.write_text('#!/bin/sh\\nexit 0\\n'); path.chmod(0o755)\n")
        self.executable("python3", "import os, sys\nfrom pathlib import Path\n"
                        "if sys.argv[1:3] == ['-m', 'venv']:\n"
                        "    bindir=Path(sys.argv[3])/'bin'; bindir.mkdir(parents=True, exist_ok=True)\n"
                        "    (bindir/'python').symlink_to(sys.argv[0])\n"
                        "    entry=bindir/'ccp-mcp-server'; entry.write_text('#!/bin/sh\\nexit 0\\n'); entry.chmod(0o755)\n"
                        "elif sys.argv[1:3] == ['-m', 'pip'] or sys.argv[1] == '-c':\n"
                        "    pass\n"
                        "else:\n"
                        f"    os.execv({sys.executable!r}, [{sys.executable!r}, *sys.argv[1:]])\n")
        environment = dict(self.environment, HOME=str(home))
        command = ["bash", str(checkout / "install.sh"), "--client", "--from-source",
                   "--install-dir", str(install)]
        response = subprocess.run(command, cwd=checkout, env=environment, capture_output=True,
                                  text=True, timeout=15)
        self.assertEqual(response.returncode, 0, response.stderr)
        for path in [home / ".claude.json", home / ".cursor/mcp.json"]:
            config = json.loads(path.read_text())
            self.assertEqual(config["mcpServers"]["ccp"]["env"]["CCP_CLIENT_BIN"], str(install.resolve() / "ccp-client"))
            self.assertEqual(config["mcpServers"]["ccp"]["env"]["CCP_SERVER_URL"], environment["CCP_SERVER_URL"])
        self.assertEqual(json.loads((home / ".cursor/mcp.json").read_text())["mcpServers"]["other"]["command"], "existing")
        import tomllib
        config = tomllib.loads((home / ".codex/config.toml").read_text())
        self.assertEqual(config["mcp_servers"]["ccp"]["env"]["CCP_CLIENT_BIN"], str(install.resolve() / "ccp-client"))
        self.executable("cargo", "import sys\nsys.exit(17)\n")
        response = subprocess.run(command, cwd=checkout, env=environment, capture_output=True,
                                  text=True, timeout=15)
        self.assertNotEqual(response.returncode, 0)
        self.assertIn("Build failed", response.stdout)


if __name__ == "__main__":
    unittest.main()
