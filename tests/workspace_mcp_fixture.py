"""Owned temporary workspace server/profiles for real MCP transport tests."""

from __future__ import annotations

import contextlib
import json
import os
import signal
import ssl
import subprocess
import tempfile
import time
import urllib.request
from pathlib import Path


class WorkspaceFixture:
    def __enter__(self):
        with contextlib.ExitStack() as stack:
            self.root = Path(
                stack.enter_context(tempfile.TemporaryDirectory(prefix="ccp-workspace-mcp-test-"))
            )
            repo = Path(__file__).resolve().parents[1]
            self.server = Path(
                os.environ.get("CCP_SERVER_BIN", repo / "target/release/server")
            ).resolve()
            self.client = Path(
                os.environ.get("CCP_CLIENT_BIN", repo / "target/release/client")
            ).resolve()
            self.profile = self.root / "original"
            self.recovered = self.root / "recovered"
            self.data = self.root / "server"
            self.setup = self.command(
                self.server,
                "workspace-init",
                "--data-dir",
                self.data,
                "--operator",
                "fixture",
                "--workspace",
                "first",
            )
            self.trust = self.command(
                self.server, "workspace-tls-init", "--data-dir", self.data, "--name", "127.0.0.1"
            )
            self.ca = self.root / "ca.pem"
            self.ca.write_text(self.trust["ca_certificate_pem"])
            self.log = stack.enter_context(tempfile.TemporaryFile(mode="w+"))
            self.process = self.start_server()
            stack.callback(self.stop_server)
            self.startup = self.wait_ready()
            self.https = "https://" + self.startup["https_address"]
            self.mtls = "https://" + self.startup["mtls_address"]
            self.opener = urllib.request.build_opener(
                urllib.request.ProxyHandler({}),
                urllib.request.HTTPSHandler(
                    context=ssl.create_default_context(cafile=str(self.ca))
                ),
            )
            self.trust_profile(self.profile)
            token = self.admin(
                "invitations/initial",
                {
                    "workspace_id": self.setup["workspace_id"],
                    "role": "read_write",
                    "max_clients": 1,
                    "expires_at": int(time.time()) + 3600,
                },
            )["secret"]
            enrolled = self.enroll_or_join(self.profile, "enroll", token)
            self.client_id = enrolled["client_id"]
            self.stack = stack.pop_all()
        return self

    def __exit__(self, _kind, _error, _traceback):
        self.stack.close()

    def command(self, binary, *arguments):
        # Fixture binaries and argv are explicit; no shell or user directories.
        result = subprocess.run(  # noqa: S603
            [str(binary), *map(str, arguments)], capture_output=True, text=True, timeout=30
        )
        if result.returncode:
            raise RuntimeError("workspace fixture command failed: " + str(arguments[0]))
        return json.loads(result.stdout)

    def start_server(self):
        # The fixture owns the data directory and selected executable.
        return subprocess.Popen(  # noqa: S603
            [
                str(self.server),
                "workspace-serve",
                "--data-dir",
                str(self.data),
                "--activate-workspace",
                "--https-listen",
                "127.0.0.1:0",
                "--mtls-listen",
                "127.0.0.1:0",
            ],
            stdout=self.log,
            stderr=self.log,
        )

    def wait_ready(self):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError("workspace fixture stopped before ready")
            self.log.seek(0)
            try:
                return json.loads(self.log.read())
            except ValueError:
                time.sleep(0.05)
        raise RuntimeError("workspace fixture startup timed out")

    def stop_server(self):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)

    def trust_profile(self, profile):
        return self.command(
            self.client,
            "workspace",
            "trust",
            "--profile",
            profile,
            "--instance-id",
            self.trust["instance_id"],
            "--https-origin",
            self.https,
            "--mtls-origin",
            self.mtls,
            "--ca-file",
            self.ca,
        )

    def secret_file(self, token):
        path = self.root / f"invitation-{time.monotonic_ns()}.txt"
        with open(
            path, "x", opener=lambda filename, flags: os.open(filename, flags, 0o600)
        ) as output:
            output.write(token)
        return path

    def enroll_or_join(self, profile, action, token):
        return self.command(
            self.client,
            "workspace",
            action,
            "--profile",
            profile,
            "--invitation-file",
            self.secret_file(token),
        )

    def admin(self, route, payload, key=None):
        key = key or self.setup["workspace_owner_key"]["secret"]
        # The origin is the fixture-owned HTTPS listener; every route is a test literal.
        request = urllib.request.Request(  # noqa: S310
            self.https + "/api/v1/admin/" + route,
            data=json.dumps(payload).encode(),
            headers={"content-type": "application/json", "authorization": "Bearer " + key},
        )
        with self.opener.open(request, timeout=10) as response:
            result = json.load(response)
        assert result["api_version"] == 1
        return result["data"]

    def add_second_workspace(self):
        provisioned = self.admin(
            "workspaces/create",
            {
                "name": "second",
                "owner": {"kind": "new_principal", "name": "second-owner"},
                "key_expires_at": None,
            },
            self.setup["instance_admin_key"]["secret"],
        )
        token = self.admin(
            "invitations/individual",
            {
                "workspace_id": provisioned["workspace_id"],
                "role": "read",
                "expires_at": int(time.time()) + 3600,
            },
            provisioned["owner_key"],
        )["secret"]
        self.enroll_or_join(self.profile, "join", token)
        return provisioned

    def recover_first(self):
        token = self.admin(
            "recovery/issue",
            {
                "client_id": self.client_id,
                "scope": {"kind": "workspace", "workspace_id": self.setup["workspace_id"]},
                "expires_at": int(time.time()) + 3600,
            },
        )["secret"]
        self.trust_profile(self.recovered)
        return self.command(
            self.client,
            "workspace",
            "recover",
            "--profile",
            self.recovered,
            "--expected-client-id",
            self.client_id,
            "--invitation-file",
            self.secret_file(token),
        )
