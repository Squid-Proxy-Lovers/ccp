#!/usr/bin/env python3
"""Check the real Unix hidden-paste prompt without touching the user's terminal."""

import argparse
import errno
import os
import pty
import select
import signal
import tempfile
import termios
import time
from contextlib import suppress
from pathlib import Path


def wait_prompt(master: int, deadline: float) -> bytes:
    output = b""
    while b"Paste enrollment link (hidden):" not in output:
        if time.monotonic() > deadline:
            raise AssertionError("hidden prompt missing")
        if select.select([master], [], [], 0.1)[0]:
            output += os.read(master, 4096)
    while termios.tcgetattr(master)[3] & termios.ECHO:
        if time.monotonic() > deadline:
            raise AssertionError("terminal echo stayed enabled")
        time.sleep(0.005)
    return output


def finish(pid: int, master: int, deadline: float) -> tuple[bytes, int]:
    output = b""
    closed = False
    while time.monotonic() < deadline:
        if not closed:
            try:
                if select.select([master], [], [], 0.1)[0]:
                    chunk = os.read(master, 4096)
                    closed = not chunk
                    output += chunk
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                closed = True
        done, status = os.waitpid(pid, os.WNOHANG)
        if done:
            return output, status
        if closed:
            time.sleep(0.01)
    raise AssertionError("hidden paste did not complete")


def run(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="ccp-hidden-paste-") as directory:
        secret = "ccp_join_" + "b" * 64
        link = "https://invalid.example/join#v=1&t=" + secret
        pid, master = pty.fork()
        if pid == 0:
            environment = dict(os.environ, CCP_CLIENT_HOME=directory)
            # The caller explicitly supplies the built binary; no shell is used.
            os.execve(binary, [str(binary), "enroll"], environment)  # noqa: S606
        try:
            deadline = time.monotonic() + 10
            output = wait_prompt(master, deadline)
            os.write(master, (link + "\n").encode())
            tail, status = finish(pid, master, deadline)
            output += tail
            assert os.waitstatus_to_exitcode(status) != 0
            assert link.encode() not in output and secret.encode() not in output
            assert not (Path(directory) / "workspaces").exists()
            print(
                "PASS: hidden paste disables echo, refuses malformed input privately, and creates no profile"
            )
        finally:
            # Reap first: never signal a PID that could already have been reused.
            with suppress(ChildProcessError):
                done, _ = os.waitpid(pid, os.WNOHANG)
                if not done:
                    with suppress(ProcessLookupError):
                        os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
            os.close(master)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", type=Path, required=True)
    run(parser.parse_args().client.resolve(strict=True))
