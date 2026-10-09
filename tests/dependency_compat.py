"""Check TLS/client interoperability and database reopening across dependency updates."""

from __future__ import annotations

import argparse
import json
import os
import signal
import socket
import subprocess
import tempfile
import time
from pathlib import Path


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def run(binary: Path, env: dict[str, str], *args: str) -> str:
    # Test binary and argv are supplied separately; no shell is used.
    result = subprocess.run(  # noqa: S603
        [str(binary), *args], env=env, text=True, capture_output=True, timeout=30
    )
    if result.returncode:
        raise RuntimeError(f"{binary.name} {args[0]} failed: {result.stderr}")
    return result.stdout


class Server:
    session = "dependency-compat"

    def __init__(self, binary: Path, env: dict[str, str], auth_port: int):
        self.binary, self.env, self.auth_port = binary, env, auth_port

    def __enter__(self):
        # The existing listener does not enable SO_REUSEADDR. Allow the OS's
        # TIME_WAIT interval when reopening the same endpoints after shutdown.
        deadline = time.monotonic() + 90
        ports = [
            int(self.env[key].rsplit(":", 1)[1])
            for key in ("CCP_AUTH_LISTENER_ADDR", "CCP_MTLS_LISTENER_ADDR")
        ]
        while True:
            sockets = []
            try:
                for port in ports:
                    sock = socket.socket()
                    sockets.append(sock)
                    sock.bind(("127.0.0.1", port))
                break
            except OSError as error:
                if time.monotonic() >= deadline:
                    raise RuntimeError(
                        "previous server endpoints did not become available"
                    ) from error
                time.sleep(0.2)
            finally:
                for sock in sockets:
                    sock.close()
        self.log = tempfile.TemporaryFile(mode="w+")
        # Start the explicitly selected test binary without shell interpretation.
        self.process = subprocess.Popen(  # noqa: S603
            [str(self.binary), self.session],
            env=self.env,
            stdout=self.log,
            stderr=subprocess.STDOUT,
        )
        try:
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if self.process.poll() is not None:
                    self.log.seek(0)
                    raise RuntimeError(f"server exited: {self.log.read()}")
                try:
                    with socket.create_connection(("127.0.0.1", self.auth_port), 0.2):
                        return self
                except OSError:
                    time.sleep(0.1)
            raise RuntimeError("server readiness timeout")
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def token(self) -> str:
        return json.loads(run(self.binary, self.env, "issue-token", self.session, "read_write"))[
            "token"
        ]

    def __exit__(self, *_):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.log.close()


def server_env(data: Path, auth: int, mtls: int) -> dict[str, str]:
    return dict(
        os.environ,
        CCP_SERVER_DATA_DIR=str(data),
        CCP_AUTH_LISTENER_ADDR=f"127.0.0.1:{auth}",
        CCP_MTLS_LISTENER_ADDR=f"127.0.0.1:{mtls}",
        CCP_AUTH_BASE_URL=f"http://127.0.0.1:{auth}",
        CCP_MTLS_BASE_URL=f"https://localhost:{mtls}",
    )


def check(args):
    old_server = args.old_server or args.server
    old_client = args.old_client or args.client
    with tempfile.TemporaryDirectory(prefix="ccp-dependency-") as temp:
        root = Path(temp)
        auth, mtls = free_port(), free_port()
        while mtls == auth:
            mtls = free_port()
        env = server_env(root / "data", auth, mtls)
        old_env = dict(env, CCP_CLIENT_HOME=str(root / "old-client"))
        new_env = dict(env, CCP_CLIENT_HOME=str(root / "new-client"))
        session = Server.session
        location = ["--shelf", "notes", "--book", "records"]

        def content(binary, client_env):
            return json.loads(run(binary, client_env, "get", session, "entry", *location))[
                "context"
            ]

        with Server(old_server, env, auth) as server:
            for binary, client_env in [(old_client, old_env), (args.client, new_env)]:
                run(
                    binary,
                    client_env,
                    "enroll",
                    "--redeem-url",
                    f"http://127.0.0.1:{auth}/auth/redeem",
                    "--token",
                    server.token(),
                )
            run(old_client, old_env, "add-shelf", session, "notes", "compatibility notes")
            run(
                old_client,
                old_env,
                "add-book",
                session,
                "--shelf",
                "notes",
                "records",
                "saved records",
            )
            run(
                old_client,
                old_env,
                "add-entry",
                session,
                *location,
                "--labels",
                "dependency,compatibility",
                "entry",
                "existing entry",
                "baseline content",
            )
            assert "baseline content" in content(args.client, new_env)
            run(args.client, new_env, "append", session, "entry", *location, "new client append")
            assert "new client append" in content(old_client, old_env)

        with Server(args.server, env, auth):
            assert "new client append" in content(old_client, old_env)
            run(old_client, old_env, "append", session, "entry", *location, "old client append")
            assert "old client append" in content(args.client, new_env)
            brief = json.loads(run(args.client, new_env, "brief-me", session))
            assert brief["total_entries"] == 1
            assert "dependency" in brief["frequent_labels"]

        with Server(old_server, env, auth):
            assert "old client append" in content(old_client, old_env)
            assert "old client append" in content(args.client, new_env)
    print(
        "PASS: old/new TLS clients, server upgrade/downgrade, existing enrollments and persisted content"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["server", "client", "old-server", "old-client"]:
        parser.add_argument(
            f"--{name}", type=lambda s: Path(s).resolve(), required=not name.startswith("old-")
        )
    check(parser.parse_args())
