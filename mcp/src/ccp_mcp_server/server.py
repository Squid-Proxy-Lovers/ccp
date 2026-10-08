# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import signal
import sqlite3
import socket
import subprocess
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from fastmcp import FastMCP

SERVER_NAME = "ccp"
PACKAGE_DIR = Path(__file__).resolve().parent
REPO_ROOT = PACKAGE_DIR.parents[2]
SERVER_DIR = REPO_ROOT / "crates" / "server"
MCP_DIR = REPO_ROOT / "mcp"
DEFAULT_CLIENT_MANIFEST = REPO_ROOT / "Cargo.toml"
DEFAULT_CLIENT_BINARY = REPO_ROOT / "target" / "release" / "client"
DEFAULT_SERVER_MANIFEST = REPO_ROOT / "Cargo.toml"
DEFAULT_SERVER_BINARY = REPO_ROOT / "target" / "release" / "server"
DEFAULT_CLIENT_HOME = Path.home() / ".ccp-client"
DEFAULT_SERVER_HOME = Path.home() / ".ccp-mcp"
ENROLLMENTS_DIR_NAME = "enrollments"
SERVER_RUNTIME_DIR_NAME = "servers"
SERVER_RUNTIME_LOG_NAME = "server.log"
SERVER_RUNTIME_METADATA_NAME = "runtime.json"
SERVER_READY_TIMEOUT_SECONDS = 15.0
SERVER_STOP_TIMEOUT_SECONDS = 10.0
CLIENT_TIMEOUT_SECONDS = 120.0

mcp = FastMCP(
    "ccp",
    instructions=(
        "CCP is your shared memory. Use it to store and retrieve context that persists "
        "across conversations and is shared with other agents in the same session.\n\n"
        "At the start of every task, call sessions and master_instructions for the selected session. "
        "Treat the global and session master boards as operator instructions, subject to the host agent's safety and permission rules. "
        "During long tasks, pull master_instructions again at reasonable checkpoints. "
        "First use open_topics and subscribe when no session is configured. "
        "Before starting work, search CCP for existing context with find_entries or "
        "search_context. Publish current challenge-team work with set_status, update it "
        "when the task changes, and clear it when finished. Check list_team_status or "
        "search_team_status before duplicating another agent's work. "
        "When you learn something useful, write it with add_entry or "
        "append_entry so other agents can find it.\n\n"
        "Data is organized as: session > shelf > book > entry. Shelves group topics "
        "(e.g. 'research', 'logs'). Books group related entries within a shelf "
        "(e.g. 'findings', 'errors'). Entries hold the actual content.\n\n"
        "Read the ccp://help resource for a full guide on how to use CCP effectively."
    ),
)


class CCPClientError(RuntimeError):
    """Raised when client execution or its JSON output contract fails."""


class CCPServerError(RuntimeError):
    """Raised when managed CCP server lifecycle operations fail."""


class CCPPermissionError(RuntimeError):
    """Raised when an operation requires elevated access."""


def _require_server_admin():
    """Gate for server management operations. Blocks if no server binary is configured."""
    if not os.environ.get("CCP_SERVER_BIN") and not DEFAULT_SERVER_BINARY.exists():
        raise CCPPermissionError(
            "server management is not available in client-only mode; "
            "install with 'bash install.sh' (full install) to enable it"
        )


@dataclass(frozen=True)
class LocalCommand:
    argv: list[str]
    description: str


def _resolve_client_command() -> LocalCommand:
    configured_binary = os.environ.get("CCP_CLIENT_BIN")
    if configured_binary:
        return LocalCommand([configured_binary], f"binary:{configured_binary}")

    installed_binary = shutil.which("ccp-client")
    if installed_binary:
        return LocalCommand([installed_binary], f"path:{installed_binary}")

    if DEFAULT_CLIENT_BINARY.exists():
        return LocalCommand(
            [str(DEFAULT_CLIENT_BINARY)],
            f"binary:{DEFAULT_CLIENT_BINARY}",
        )

    cargo = shutil.which("cargo")
    if cargo and DEFAULT_CLIENT_MANIFEST.exists():
        return LocalCommand(
            [
                cargo,
                "run",
                "--quiet",
                "--package",
                "client",
                "--manifest-path",
                str(DEFAULT_CLIENT_MANIFEST),
                "--",
            ],
            f"cargo:{DEFAULT_CLIENT_MANIFEST}",
        )

    raise CCPClientError(
        "unable to resolve the CCP client binary; set CCP_CLIENT_BIN or build crates/client"
    )


def _resolve_server_command() -> LocalCommand:
    configured_binary = os.environ.get("CCP_SERVER_BIN")
    if configured_binary:
        return LocalCommand([configured_binary], f"binary:{configured_binary}")

    if DEFAULT_SERVER_BINARY.exists():
        return LocalCommand(
            [str(DEFAULT_SERVER_BINARY)],
            f"binary:{DEFAULT_SERVER_BINARY}",
        )

    cargo = shutil.which("cargo")
    if cargo and DEFAULT_SERVER_MANIFEST.exists():
        return LocalCommand(
            [
                cargo,
                "run",
                "--quiet",
                "--package",
                "server",
                "--manifest-path",
                str(DEFAULT_SERVER_MANIFEST),
                "--",
            ],
            f"cargo:{DEFAULT_SERVER_MANIFEST}",
        )

    raise CCPServerError(
        "unable to resolve the CCP server binary; set CCP_SERVER_BIN or build crates/server"
    )


def _client_args(
    command: str,
    *positionals: str,
    options: list[tuple[str, str | None]] | None = None,
    switches: list[str] | None = None,
) -> list[str]:
    """Keep arbitrary tool text out of the CLI option parser."""
    return [
        command,
        *(f"{flag}={value}" for flag, value in options or [] if value is not None),
        *(switches or []),
        "--",
        *positionals,
    ]


def _run_client(*args: str, env_overrides: dict[str, str] | None = None) -> str:
    command = _resolve_client_command()
    env = os.environ.copy()
    if env_overrides:
        env.update(env_overrides)
    try:
        process = subprocess.run(
            [*command.argv, *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=CLIENT_TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise CCPClientError("CCP client command timed out") from error
    except OSError as error:
        raise CCPClientError(f"unable to run CCP client: {error}") from error
    if process.returncode != 0:
        detail = (process.stderr or process.stdout).strip() or "unknown client failure"
        raise CCPClientError(detail)
    return process.stdout.strip()


def _run_client_json(*args: str, env_overrides: dict[str, str] | None = None) -> Any:
    output = _run_client(*args, env_overrides=env_overrides)
    try:
        return json.loads(output)
    except json.JSONDecodeError as error:
        raise CCPClientError("client returned invalid JSON") from error


def _client_home() -> Path:
    return Path(os.environ.get("CCP_CLIENT_HOME", DEFAULT_CLIENT_HOME))


def _server_home() -> Path:
    return Path(os.environ.get("CCP_SERVER_HOME", DEFAULT_SERVER_HOME))


def _enrollments_dir() -> Path:
    return _client_home() / ENROLLMENTS_DIR_NAME


def _server_runtime_root() -> Path:
    return _server_home() / SERVER_RUNTIME_DIR_NAME


def _session_slug(session_name: str) -> str:
    normalized = re.sub(r"[^a-z0-9]+", "-", session_name.lower()).strip("-")
    if not normalized:
        normalized = "session"
    digest = hashlib.sha256(session_name.encode("utf-8")).hexdigest()[:8]
    return f"{normalized}-{digest}"


def _runtime_dir_for_session(session_name: str) -> Path:
    return _server_runtime_root() / _session_slug(session_name)


def _runtime_metadata_path(session_name: str) -> Path:
    return _runtime_dir_for_session(session_name) / SERVER_RUNTIME_METADATA_NAME


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _current_unix_timestamp() -> int:
    return int(time.time())


def _cert_warning_for_expiry(expires_at: int | None) -> str | None:
    if not expires_at:
        return None
    now = _current_unix_timestamp()
    if now >= expires_at:
        return (
            f"client certificate expired at unix={expires_at}; "
            "request a new enrollment token and re-enroll"
        )
    warning_window = int(
        os.environ.get("CCP_CERT_WARNING_WINDOW_SECONDS", "0")
    )
    if warning_window <= 0:
        return None
    if expires_at - now <= warning_window:
        return (
            f"client certificate expires soon at unix={expires_at}; "
            "request a new enrollment token before it expires"
        )
    return None


def _pid_is_running(pid: int | None) -> bool:
    if pid is None or pid <= 0:
        return False
    try:
        waited_pid, _ = os.waitpid(pid, os.WNOHANG)
        if waited_pid == pid:
            return False
    except ChildProcessError:
        pass
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    return True


def _reserve_local_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        return int(sock.getsockname()[1])


def _wait_for_tcp_listener(host: str, port: int, timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.2):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def _parse_host_port(address: str) -> tuple[str, int]:
    if address.startswith("["):
        end = address.find("]")
        if end == -1:
            raise CCPServerError(f"invalid listener address: {address}")
        host = address[1:end]
        port_text = address[end + 1 :]
        if not port_text.startswith(":"):
            raise CCPServerError(f"invalid listener address: {address}")
        return host, int(port_text[1:])

    host, separator, port_text = address.rpartition(":")
    if separator != ":" or not host or not port_text:
        raise CCPServerError(f"invalid listener address: {address}")
    return host, int(port_text)


def _load_runtime_auth_info(record: dict[str, Any]) -> dict[str, Any]:
    data_dir = Path(record["data_dir"])
    db_path = data_dir / "ccp.sqlite3"
    if not db_path.exists():
        return {}

    session_id = _query_session_id(db_path, record["session_name"])
    if session_id is None:
        return {}

    auth_base_url = str(record["auth_base_url"]).rstrip("/")
    info: dict[str, Any] = {
        "session_id": session_id,
        "auth_redeem_url": f"{auth_base_url}/auth/redeem",
    }
    return info


def _session_db_path(record: dict[str, Any]) -> Path:
    return Path(record["data_dir"]) / "ccp.sqlite3"


def _query_session_id(db_path: Path, session_name: str) -> int | None:
    with sqlite3.connect(db_path) as connection:
        row = connection.execute(
            "SELECT id FROM sessions WHERE name = ?",
            (session_name,),
        ).fetchone()
    return int(row[0]) if row else None


def _load_runtime_record(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def _write_runtime_record(path: Path, record: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _refresh_runtime_record(record: dict[str, Any], persist: bool = True) -> dict[str, Any]:
    pid = record.get("pid")
    is_running = _pid_is_running(pid if isinstance(pid, int) else None)
    if not is_running and record.get("status") == "running":
        record["status"] = "stopped"
        record.setdefault("stopped_at", _utc_now())

    auth_info = _load_runtime_auth_info(record)
    record["is_running"] = is_running
    if auth_info:
        record.update(auth_info)
    if persist:
        persisted = dict(record)
        persisted.pop("initial_tokens", None)
        _write_runtime_record(Path(record["metadata_path"]), persisted)
    return record


def _describe_runtime_record(record: dict[str, Any]) -> dict[str, Any]:
    refreshed = _refresh_runtime_record(dict(record))
    return {
        "session_name": refreshed["session_name"],
        "session_slug": refreshed["session_slug"],
        "pid": refreshed.get("pid"),
        "status": refreshed.get("status", "unknown"),
        "is_running": refreshed.get("is_running", False),
        "started_at": refreshed.get("started_at"),
        "stopped_at": refreshed.get("stopped_at"),
        "auth_port": refreshed["auth_port"],
        "mtls_port": refreshed["mtls_port"],
        "auth_base_url": refreshed["auth_base_url"],
        "auth_redeem_url": refreshed.get("auth_redeem_url"),
        "mtls_base_url": refreshed["mtls_base_url"],
        "session_id": refreshed.get("session_id"),
        "token_policy": {
            "one_time": False,
            "mode": "multi_use_until_expiry",
            "default_ttl_seconds": int(os.environ.get("CCP_ENROLLMENT_TOKEN_TTL_SECONDS", "3600")),
        },
        "cert_policy": {
            "client_cert_ttl_seconds": int(os.environ.get("CCP_CLIENT_CERT_TTL_SECONDS", str(3650 * 24 * 60 * 60))),
            "warning_window_seconds": int(os.environ.get("CCP_CERT_WARNING_WINDOW_SECONDS", "0")),
            "ca_ttl_days": int(os.environ.get("CCP_CA_CERT_TTL_DAYS", "3650")),
        },
        "runtime_dir": refreshed["runtime_dir"],
        "data_dir": refreshed["data_dir"],
        "log_path": refreshed["log_path"],
        "metadata_path": refreshed["metadata_path"],
    }


def _list_runtime_records() -> list[dict[str, Any]]:
    root = _server_runtime_root()
    if not root.exists():
        return []

    records: list[dict[str, Any]] = []
    for child in sorted(root.iterdir()):
        if not child.is_dir():
            continue
        record = _load_runtime_record(child / SERVER_RUNTIME_METADATA_NAME)
        if record is None:
            continue
        records.append(_describe_runtime_record(record))
    return records


def _find_runtime_record_raw(session: str) -> dict[str, Any] | None:
    root = _server_runtime_root()
    if not root.exists():
        return None
    for child in sorted(root.iterdir()):
        if not child.is_dir():
            continue
        record = _load_runtime_record(child / SERVER_RUNTIME_METADATA_NAME)
        if record is None:
            continue
        if record["session_name"] == session or record["session_slug"] == session:
            return _refresh_runtime_record(record)
    return None


def _find_runtime_record(session: str) -> dict[str, Any] | None:
    for record in _list_runtime_records():
        if record["session_name"] == session or record["session_slug"] == session:
            return record
    return None


def _filter_records(records: list[dict[str, Any]], filter_text: str | None) -> list[dict[str, Any]]:
    if not filter_text:
        return records
    needle = filter_text.lower()
    return [
        record
        for record in records
        if needle in json.dumps(record, sort_keys=True).lower()
    ]


def _rename_session_metadata(record: dict[str, Any], new_session_name: str) -> None:
    db_path = _session_db_path(record)
    if not db_path.exists():
        raise CCPServerError(f"missing session database at {db_path}")

    with sqlite3.connect(db_path) as connection:
        row = connection.execute("SELECT id FROM sessions ORDER BY id LIMIT 1").fetchone()
        if row is None:
            raise CCPServerError(f"no session row found in {db_path}")
        connection.execute(
            "UPDATE sessions SET name = ? WHERE id = ?",
            (new_session_name, int(row[0])),
        )
        connection.commit()

    binding_path = Path(record["data_dir"]) / "active_session.json"
    if binding_path.exists():
        binding = json.loads(binding_path.read_text(encoding="utf-8"))
        binding["session_name"] = new_session_name
        binding_path.write_text(
            json.dumps(binding, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )


def _read_log_tail(log_path: Path, lines: int) -> str:
    if lines <= 0:
        raise CCPServerError("lines must be greater than zero")
    if not log_path.exists():
        return ""
    content = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
    return "\n".join(content[-lines:])


def _build_runtime_record(
    session_name: str,
    auth_listener_addr: str | None = None,
    mtls_listener_addr: str | None = None,
    auth_base_url: str | None = None,
    mtls_base_url: str | None = None,
) -> dict[str, Any]:
    runtime_dir = _runtime_dir_for_session(session_name)
    data_dir = runtime_dir / "data"
    log_path = runtime_dir / SERVER_RUNTIME_LOG_NAME
    metadata_path = runtime_dir / SERVER_RUNTIME_METADATA_NAME
    if auth_listener_addr is None:
        auth_port = _reserve_local_port()
        auth_listener_addr = f"127.0.0.1:{auth_port}"
    else:
        _, auth_port = _parse_host_port(auth_listener_addr)

    if mtls_listener_addr is None:
        mtls_port = _reserve_local_port()
        while mtls_port == auth_port:
            mtls_port = _reserve_local_port()
        mtls_listener_addr = f"127.0.0.1:{mtls_port}"
    else:
        _, mtls_port = _parse_host_port(mtls_listener_addr)

    if auth_base_url is None:
        auth_base_url = f"http://127.0.0.1:{auth_port}"
    if mtls_base_url is None:
        mtls_base_url = f"https://localhost:{mtls_port}"

    return {
        "session_name": session_name,
        "session_slug": _session_slug(session_name),
        "status": "starting",
        "pid": None,
        "started_at": _utc_now(),
        "auth_port": auth_port,
        "mtls_port": mtls_port,
        "auth_listener_addr": auth_listener_addr,
        "mtls_listener_addr": mtls_listener_addr,
        "auth_base_url": auth_base_url,
        "mtls_base_url": mtls_base_url,
        "runtime_dir": str(runtime_dir),
        "data_dir": str(data_dir),
        "log_path": str(log_path),
        "metadata_path": str(metadata_path),
    }


def _start_subprocess(command: LocalCommand, session_name: str, record: dict[str, Any]) -> subprocess.Popen[str]:
    runtime_dir = Path(record["runtime_dir"])
    runtime_dir.mkdir(parents=True, exist_ok=True)
    Path(record["data_dir"]).mkdir(parents=True, exist_ok=True)
    log_handle = Path(record["log_path"]).open("a", encoding="utf-8")

    env = os.environ.copy()
    env.update(
        {
            "CCP_SERVER_DATA_DIR": record["data_dir"],
            "CCP_AUTH_BASE_URL": record["auth_base_url"],
            "CCP_MTLS_BASE_URL": record["mtls_base_url"],
            "CCP_AUTH_LISTENER_ADDR": record["auth_listener_addr"],
            "CCP_MTLS_LISTENER_ADDR": record["mtls_listener_addr"],
            "CCP_AUTO_ISSUE_INITIAL_TOKENS": "0",
        }
    )

    kwargs: dict[str, Any] = {
        "args": [*command.argv, session_name],
        "cwd": str(SERVER_DIR),
        "env": env,
        "stdout": log_handle,
        "stderr": subprocess.STDOUT,
        "text": True,
    }
    if os.name == "nt":
        kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        kwargs["start_new_session"] = True

    try:
        process = subprocess.Popen(**kwargs)
    finally:
        log_handle.close()
    return process


def _run_server_admin(record: dict[str, Any], *args: str) -> dict[str, Any]:
    command = _resolve_server_command()
    env = os.environ.copy()
    env.update(
        {
            "CCP_SERVER_DATA_DIR": record["data_dir"],
            "CCP_AUTH_BASE_URL": record["auth_base_url"],
            "CCP_MTLS_BASE_URL": record["mtls_base_url"],
            "CCP_AUTH_LISTENER_ADDR": record["auth_listener_addr"],
            "CCP_MTLS_LISTENER_ADDR": record["mtls_listener_addr"],
            "CCP_AUTO_ISSUE_INITIAL_TOKENS": "0",
        }
    )
    process = subprocess.run(
        [*command.argv, *args],
        cwd=str(SERVER_DIR),
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if process.returncode != 0:
        detail = (process.stderr or process.stdout).strip() or "unknown server admin failure"
        raise CCPServerError(detail)
    output = process.stdout.strip()
    if not output:
        return {}
    return json.loads(output)


def _wait_for_server_ready(record: dict[str, Any], timeout_seconds: float = SERVER_READY_TIMEOUT_SECONDS) -> None:
    pid = record.get("pid")
    deadline = time.monotonic() + timeout_seconds
    auth_host = "127.0.0.1"
    auth_port = int(record["auth_port"])
    db_path = Path(record["data_dir"]) / "ccp.sqlite3"

    while time.monotonic() < deadline:
        if not _pid_is_running(pid if isinstance(pid, int) else None):
            log_tail = _read_log_tail(Path(record["log_path"]), 40)
            raise CCPServerError(
                "managed CCP server exited during startup"
                + (f"\n\nRecent log output:\n{log_tail}" if log_tail else "")
            )
        if db_path.exists() and _wait_for_tcp_listener(auth_host, auth_port, 0.25):
            return
        time.sleep(0.1)

    raise CCPServerError(
        f"managed CCP server did not become ready within {timeout_seconds:.1f}s"
    )


def _signal_process(pid: int, signum: int) -> None:
    try:
        os.kill(pid, signum)
    except ProcessLookupError:
        return


def _wait_for_process_exit(pid: int, timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if not _pid_is_running(pid):
            return True
        time.sleep(0.1)
    return not _pid_is_running(pid)


def _stop_runtime_record(record: dict[str, Any], force: bool = False) -> dict[str, Any]:
    pid = record.get("pid")
    if not isinstance(pid, int) or pid <= 0:
        record["status"] = "stopped"
        record.setdefault("stopped_at", _utc_now())
        _write_runtime_record(Path(record["metadata_path"]), record)
        return _describe_runtime_record(record)

    if not _pid_is_running(pid):
        record["status"] = "stopped"
        record.setdefault("stopped_at", _utc_now())
        _write_runtime_record(Path(record["metadata_path"]), record)
        return _describe_runtime_record(record)

    if force:
        _signal_process(pid, signal.SIGKILL)
        _wait_for_process_exit(pid, 2.0)
    else:
        _signal_process(pid, signal.SIGINT)
        _wait_for_process_exit(pid, SERVER_STOP_TIMEOUT_SECONDS)
        if _pid_is_running(pid):
            _signal_process(pid, signal.SIGTERM)
            _wait_for_process_exit(pid, 3.0)
        if _pid_is_running(pid):
            _signal_process(pid, signal.SIGKILL)
            _wait_for_process_exit(pid, 2.0)

    record["status"] = "stopped"
    record["stopped_at"] = _utc_now()
    _write_runtime_record(Path(record["metadata_path"]), record)
    return _describe_runtime_record(record)


def _load_session_summaries() -> list[dict[str, Any]]:
    enrollments_dir = _enrollments_dir()
    if not enrollments_dir.exists():
        return []

    grouped: dict[tuple[str, int, str], dict[str, Any]] = {}
    for child in sorted(enrollments_dir.iterdir()):
        if not child.is_dir():
            continue
        metadata_path = child / "metadata.json"
        if not metadata_path.exists():
            continue
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        key = (
            metadata["session_name"],
            int(metadata["session_id"]),
            metadata["mtls_endpoint"],
        )
        summary = grouped.setdefault(
            key,
            {
                "session_name": metadata["session_name"],
                "session_id": int(metadata["session_id"]),
                "access": [],
                "cert_count": 0,
                "endpoint": metadata["mtls_endpoint"],
                "session_description": metadata.get("session_description", ""),
                "owner": metadata.get("owner", ""),
                "labels": metadata.get("labels", []),
                "visibility": metadata.get("visibility", "private"),
                "purpose": metadata.get("purpose", ""),
                "latest_client_cert_expires_at": int(metadata.get("client_cert_expires_at", 0)),
                "cert_warning": _cert_warning_for_expiry(
                    int(metadata.get("client_cert_expires_at", 0))
                ),
            },
        )
        if metadata["access"] not in summary["access"]:
            summary["access"].append(metadata["access"])
            summary["access"].sort()
        summary["cert_count"] += 1
        expires_at = int(metadata.get("client_cert_expires_at", 0))
        if expires_at >= int(summary.get("latest_client_cert_expires_at", 0)):
            summary["latest_client_cert_expires_at"] = expires_at
            summary["cert_warning"] = _cert_warning_for_expiry(expires_at)

    return list(grouped.values())


def _session_warning_for_selector(session: str) -> str | None:
    name, separator, endpoint = session.rpartition("@")
    if not separator or not endpoint.startswith("http://"):
        name = session
        endpoint = os.environ.get("CCP_SERVER_URL", "")
    candidates = [
        summary for summary in _load_session_summaries()
        if name in {summary["session_name"], str(summary["session_id"])}
        and (not endpoint or summary["endpoint"].rstrip("/") == endpoint.rstrip("/"))
    ]
    # Legacy warning metadata must not be attributed to a different endpoint.
    if len(candidates) != 1:
        return None
    warning = candidates[0].get("cert_warning")
    return warning if isinstance(warning, str) and warning else None


def _attach_session_warning(session: str, payload: dict[str, Any]) -> dict[str, Any]:
    try:
        warning = _session_warning_for_selector(session)
    except (OSError, ValueError, KeyError, TypeError):
        # Optional legacy certificate metadata must not hide a completed mutation.
        return payload
    if warning:
        payload = dict(payload)
        payload["ccp_certificate_warning"] = warning
    return payload


def _parse_enroll_output(output: str) -> dict[str, Any]:
    result: dict[str, Any] = {"message": output}
    for line in output.splitlines():
        stripped = line.strip()
        if stripped.startswith("Saved enrollment for session '"):
            result["summary"] = stripped
        elif stripped.startswith("Stored at "):
            result["stored_at"] = stripped.removeprefix("Stored at ").strip()
        elif stripped.startswith("Client certificate expires at unix="):
            result["client_cert_expires_at"] = int(
                stripped.removeprefix("Client certificate expires at unix=").strip()
            )
    return result


@mcp.tool()
def server_status() -> dict[str, Any]:
    """Return the resolved CCP client command and key local paths."""

    client_command = _resolve_client_command()
    try:
        server_command = _resolve_server_command()
        server_resolution = server_command.description
    except CCPServerError as error:
        server_command = None
        server_resolution = str(error)
    managed_servers = _list_runtime_records()
    return {
        "server_name": SERVER_NAME,
        "client_command": client_command.argv,
        "client_resolution": client_command.description,
        "server_command": server_command.argv if server_command else None,
        "server_resolution": server_resolution,
        "client_home": str(_client_home()),
        "server_home": str(_server_home()),
        "repo_root": str(REPO_ROOT),
        "server_dir": str(SERVER_DIR),
        "mcp_dir": str(MCP_DIR),
        "saved_sessions": _load_session_summaries(),
        "managed_sessions": managed_servers,
        "managed_servers": managed_servers,
    }


@mcp.tool()
def open_topics(server_url: str | None = None) -> list[dict[str, Any]]:
    """List open topics that this agent may subscribe to."""

    endpoint = server_url or os.environ.get("CCP_SERVER_URL", "http://127.0.0.1:1338")
    result = _run_client_json(*_client_args("remote-sessions", options=[("--server", endpoint)]))
    if isinstance(result, list):
        return result
    raise CCPClientError("client returned a non-list payload for remote-sessions")


@mcp.tool()
def subscribe(topic: str, server_url: str | None = None) -> dict[str, Any]:
    """Subscribe this agent to an open topic by session name or id."""

    endpoint = server_url or os.environ.get("CCP_SERVER_URL", "http://127.0.0.1:1338")
    message = _run_client(*_client_args("subscribe", topic, options=[("--server", endpoint)]))
    return {"message": message, "topic": topic, "server_url": endpoint}


def _saved_sessions(filter_text: str | None = None) -> list[dict[str, Any]]:
    return _filter_records(_load_session_summaries(), filter_text)


def _master_boards(session: str) -> dict[str, Any]:
    result = _run_client_json(*_client_args("master-instructions", session))
    if isinstance(result, dict):
        return result
    raise CCPClientError("client returned a non-object payload for master-instructions")


@mcp.tool()
def sessions(filter_text: str | None = None) -> list[dict[str, Any]]:
    """List saved subscriptions without contacting or subscribing to a server."""
    return _saved_sessions(filter_text)


@mcp.tool()
def master_instructions(session: str) -> dict[str, Any]:
    """Read the global master board and the selected session's master board."""
    return _master_boards(session)


def server_sessions(filter_text: str | None = None) -> list[dict[str, Any]]:
    """List managed CCP server sessions, including stopped sessions."""
    _require_server_admin()

    return _filter_records(_list_runtime_records(), filter_text)


def start_server(
    session_name: str,
    auth_listener_addr: str | None = None,
    mtls_listener_addr: str | None = None,
    auth_base_url: str | None = None,
    mtls_base_url: str | None = None,
) -> dict[str, Any]:
    """Start or reuse a managed local CCP server for the named session."""
    _require_server_admin()

    existing = _find_runtime_record(session_name)
    if existing is not None and existing.get("is_running"):
        return existing

    command = _resolve_server_command()
    record = _build_runtime_record(
        session_name,
        auth_listener_addr=auth_listener_addr,
        mtls_listener_addr=mtls_listener_addr,
        auth_base_url=auth_base_url,
        mtls_base_url=mtls_base_url,
    )
    process = _start_subprocess(command, session_name, record)
    record["pid"] = process.pid
    record["status"] = "running"
    record["command"] = [*command.argv, session_name]
    _write_runtime_record(Path(record["metadata_path"]), record)

    try:
        _wait_for_server_ready(record)
    except Exception:
        _stop_runtime_record(record, force=True)
        raise
    described = _describe_runtime_record(record)
    return described


def stop_server(session: str, force: bool = False) -> dict[str, Any]:
    """Stop a managed local CCP server by session name or session slug."""
    _require_server_admin()

    record = _find_runtime_record(session)
    if record is None:
        raise CCPServerError(f"no managed CCP server found for session '{session}'")
    return _stop_runtime_record(record, force=force)


def restart_server(session: str, force: bool = False) -> dict[str, Any]:
    """Restart a managed CCP server by session name or session slug."""
    _require_server_admin()

    record = _find_runtime_record_raw(session)
    if record is None:
        raise CCPServerError(f"no managed CCP server found for session '{session}'")
    if record.get("is_running"):
        _stop_runtime_record(record, force=force)
    return start_server(
        str(record["session_name"]),
        auth_listener_addr=record.get("auth_listener_addr"),
        mtls_listener_addr=record.get("mtls_listener_addr"),
        auth_base_url=record.get("auth_base_url"),
        mtls_base_url=record.get("mtls_base_url"),
    )


def running_servers() -> list[dict[str, Any]]:
    """List CCP server processes managed by this MCP bridge."""
    _require_server_admin()

    return _list_runtime_records()


def tail_server_logs(session: str, lines: int = 200) -> str:
    """Return the most recent log lines for a managed CCP server."""
    _require_server_admin()

    record = _find_runtime_record(session)
    if record is None:
        raise CCPServerError(f"no managed CCP server found for session '{session}'")
    return _read_log_tail(Path(record["log_path"]), lines)


def rename_session(session: str, new_session_name: str, force: bool = False) -> dict[str, Any]:
    """Rename a managed CCP session and its runtime directory."""
    _require_server_admin()

    if not new_session_name.strip():
        raise CCPServerError("new_session_name must not be empty")
    if _find_runtime_record_raw(new_session_name) is not None:
        raise CCPServerError(f"a managed CCP session named '{new_session_name}' already exists")

    record = _find_runtime_record_raw(session)
    if record is None:
        raise CCPServerError(f"no managed CCP server found for session '{session}'")

    if record.get("is_running"):
        _stop_runtime_record(record, force=force)

    _rename_session_metadata(record, new_session_name)

    current_runtime_dir = Path(record["runtime_dir"])
    target_runtime_dir = _runtime_dir_for_session(new_session_name)
    if target_runtime_dir.exists() and current_runtime_dir != target_runtime_dir:
        raise CCPServerError(
            f"target runtime directory already exists for session '{new_session_name}'"
        )

    if current_runtime_dir != target_runtime_dir:
        target_runtime_dir.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(str(current_runtime_dir), str(target_runtime_dir))

    updated = dict(record)
    updated["session_name"] = new_session_name
    updated["session_slug"] = _session_slug(new_session_name)
    updated["runtime_dir"] = str(target_runtime_dir)
    updated["data_dir"] = str(target_runtime_dir / "data")
    updated["log_path"] = str(target_runtime_dir / SERVER_RUNTIME_LOG_NAME)
    updated["metadata_path"] = str(target_runtime_dir / SERVER_RUNTIME_METADATA_NAME)
    command = updated.get("command")
    if isinstance(command, list) and command:
        command[-1] = new_session_name

    _write_runtime_record(Path(updated["metadata_path"]), updated)
    return _describe_runtime_record(updated)


def delete_session(session: str, force: bool = False) -> dict[str, Any]:
    """Delete a managed CCP session and remove its local runtime directory."""
    _require_server_admin()

    record = _find_runtime_record_raw(session)
    if record is None:
        raise CCPServerError(f"no managed CCP server found for session '{session}'")

    if record.get("is_running"):
        _stop_runtime_record(record, force=force)

    runtime_dir = Path(record["runtime_dir"])
    if runtime_dir.exists():
        shutil.rmtree(runtime_dir)

    return {
        "session_name": record["session_name"],
        "session_slug": record["session_slug"],
        "runtime_dir": str(runtime_dir),
        "data_dir": record["data_dir"],
        "deleted": True,
    }


@mcp.tool()
def list_entries(session: str) -> list[dict[str, Any]]:
    """List message entries for a session, including shelf and book metadata."""

    data = _run_client_json(*_client_args("list", session))
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for list")


@mcp.tool()
def find_entries(session: str, query: str) -> list[dict[str, Any]]:
    """Search entry names and descriptions for keywords."""

    data = _run_client_json(*_client_args("search-entries", session, query))
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for search-entries")


@mcp.tool()
def find_shelves(session: str, query: str) -> list[dict[str, Any]]:
    """Search shelf names and descriptions for keywords."""

    data = _run_client_json(*_client_args("search-shelves", session, query))
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for search-shelves")


@mcp.tool()
def find_books(session: str, query: str) -> list[dict[str, Any]]:
    """Search book names and descriptions for keywords."""

    data = _run_client_json(*_client_args("search-books", session, query))
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for search-books")


@mcp.tool()
def search_context(session: str, query: str) -> list[dict[str, Any]]:
    """Search entry context for keywords and return snippets."""

    data = _run_client_json(*_client_args("search-context", session, query))
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for search-context")


@mcp.tool()
def search_deleted_entries(session: str, query: str = "") -> list[dict[str, Any]]:
    """Search deleted entries by name and description, or list them all with an empty query."""

    args = _client_args("search-deleted", session)
    if query:
        args.append(query)
    data = _run_client_json(*args)
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for search-deleted")


@mcp.tool()
def set_status(
    session: str,
    team: str,
    agent_name: str,
    status: str,
) -> dict[str, Any]:
    """Publish or refresh an agent's current work in a shelf-backed challenge team."""

    data = _run_client_json(
        *_client_args("set-status", session, status, options=[("--team", team), ("--agent", agent_name)])
    )
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for set-status")


@mcp.tool()
def clear_status(session: str, team: str, agent_name: str) -> dict[str, Any]:
    """Clear an agent's current status and leave the shelf-backed team."""

    data = _run_client_json(
        *_client_args("clear-status", session, options=[("--team", team), ("--agent", agent_name)])
    )
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for clear-status")


@mcp.tool()
def list_team_status(session: str, team: str) -> list[dict[str, Any]]:
    """List active agent statuses in one shelf-backed challenge team."""

    data = _run_client_json(*_client_args("team-status", session, options=[("--team", team)]))
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for team-status")


@mcp.tool()
def search_team_status(
    session: str,
    team: str,
    query: str,
) -> list[dict[str, Any]]:
    """Search agent names and current work within one challenge team."""

    data = _run_client_json(*_client_args("search-team-status", session, query, options=[("--team", team)]))
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for search-team-status")


@mcp.tool()
def get_entry(
    session: str,
    entry_name: str,
    shelf_name: str | None = None,
    book_name: str | None = None,
) -> dict[str, Any]:
    """Fetch a full message entry, including context, by session and chapter path."""

    args = _client_args(
        "get", session, entry_name,
        options=[("--shelf", shelf_name), ("--book", book_name)],
    )
    data = _run_client_json(*args)
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for get")


@mcp.tool()
def add_shelf(session: str, shelf_name: str, shelf_description: str) -> dict[str, Any]:
    """Create or describe a shelf in a session using a read_write enrollment."""

    data = _run_client_json(*_client_args("add-shelf", session, shelf_name, shelf_description))
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for add-shelf")


@mcp.tool()
def add_book(
    session: str,
    shelf_name: str,
    book_name: str,
    book_description: str,
) -> dict[str, Any]:
    """Create or describe a book in an existing shelf using a read_write enrollment."""

    data = _run_client_json(
        *_client_args("add-book", session, book_name, book_description, options=[("--shelf", shelf_name)]),
    )
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for add-book")


@mcp.tool()
def add_entry(
    session: str,
    shelf_name: str,
    book_name: str,
    entry_name: str,
    entry_description: str,
    entry_data: str,
    labels: list[str] | None = None,
) -> dict[str, Any]:
    """Create a new entry in an existing shelf/book using a read_write enrollment."""

    args = _client_args(
        "add-entry", session, entry_name, entry_description, entry_data,
        options=[("--shelf", shelf_name), ("--book", book_name),
                 ("--labels", ",".join(labels) if labels else None)],
    )
    data = _run_client_json(*args)
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for add-entry")


@mcp.tool()
def append_entry(
    session: str,
    entry_name: str,
    content: str,
    agent_name: str | None = None,
    host_name: str | None = None,
    reason: str | None = None,
    shelf_name: str | None = None,
    book_name: str | None = None,
) -> dict[str, Any]:
    """Append content to an existing chapter using a read_write enrollment."""

    env_overrides = {
        key: value
        for key, value in {
            "CCP_AGENT_NAME": agent_name,
            "CCP_AGENT_HOST": host_name,
            "CCP_APPEND_REASON": reason,
        }.items()
        if value is not None
    }
    data = _run_client_json(
        *_client_args("append", session, entry_name, content,
                      options=[("--shelf", shelf_name), ("--book", book_name)]),
        env_overrides=env_overrides or None,
    )
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for append")


def delete_entry(
    session: str,
    entry_name: str,
    shelf_name: str | None = None,
    book_name: str | None = None,
) -> dict[str, Any]:
    """Delete a message entry from a session using a read_write enrollment."""

    args = _client_args(
        "delete", session, entry_name,
        options=[("--shelf", shelf_name), ("--book", book_name)],
    )
    data = _run_client_json(*args)
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for delete")


def restore_entry(session: str, entry_key: str) -> dict[str, Any]:
    """Restore a deleted message entry by deleted primary key."""

    data = _run_client_json(*_client_args("restore", session, entry_key))
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for restore")


@mcp.tool()
def get_history(
    session: str,
    entry_name: str,
    shelf_name: str | None = None,
    book_name: str | None = None,
) -> list[dict[str, Any]]:
    """Return the append history for a specific chapter."""

    args = _client_args(
        "history", session, entry_name,
        options=[("--shelf", shelf_name), ("--book", book_name)],
    )
    data = _run_client_json(*args)
    if isinstance(data, list):
        return data
    raise CCPClientError("client returned a non-list payload for history")


@mcp.tool()
def export_bundle(
    session: str,
    output_path: str | None = None,
    shelf: str | None = None,
    book: str | None = None,
    entries: list[str] | None = None,
    no_history: bool = False,
) -> dict[str, Any]:
    """Export a session, shelf, book, or named entries as a JSON bundle.

    Scope is determined by which filters are provided:
    - no filters: full session export
    - shelf only: all entries in that shelf
    - shelf + book: all entries in that book
    - shelf + book + entries: specific named entries
    """

    options = [("--shelf", shelf), ("--book", book), ("--output", output_path)]
    options.extend(("--entry", entry) for entry in entries or [])
    args = _client_args("export", session, options=options,
                        switches=["--no-history"] if no_history else [])
    if output_path is not None:
        written_path = _run_client(*args).strip()
        return _attach_session_warning(session, {"written_path": written_path})

    data = _run_client_json(*args)
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for export")


def import_bundle(
    session: str,
    bundle_path: str,
    policy: str = "error",
) -> dict[str, Any]:
    """Import a JSON bundle into a session.

    policy controls collision handling: error (default), overwrite, skip, merge-history.
    """

    args = _client_args("import", session, bundle_path, options=[("--policy", policy)])
    data = _run_client_json(*args)
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for import")


def revoke_certificate(session: str, client_common_name: str) -> dict[str, Any]:
    """Revoke a previously issued client certificate for a session."""

    data = _run_client_json("revoke-cert", session, client_common_name)
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for revoke-cert")


@mcp.tool()
def server_health(session: str) -> dict[str, Any]:
    """Check the HTTP health endpoint of the selected saved subscription."""

    data = _run_client_json(*_client_args("health", session))
    if isinstance(data, dict):
        return data
    raise CCPClientError("client returned a non-object payload for health")


@mcp.tool()
def brief_me(session: str) -> dict[str, Any]:
    """Get a quick overview of a session in one call.

    Returns session structure (shelves, books, entry counts), the 10 most
    recently updated entries, and the most common labels. Use this when
    starting work on a session to understand what's already there.
    """

    data = _run_client_json(*_client_args("brief-me", session))
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for brief-me")


@mcp.tool()
def get_entry_at(
    session: str,
    entry_name: str,
    at_timestamp: str,
    shelf_name: str | None = None,
    book_name: str | None = None,
) -> dict[str, Any]:
    """Get an entry's content as it was at a specific point in time.

    Reconstructs the entry by replaying appends up to the given timestamp.
    Useful for understanding what was known at a particular moment.
    """

    args = _client_args(
        "get-entry-at", session, entry_name,
        options=[("--at", at_timestamp), ("--shelf", shelf_name), ("--book", book_name)],
    )
    data = _run_client_json(*args)
    if isinstance(data, dict):
        return _attach_session_warning(session, data)
    raise CCPClientError("client returned a non-object payload for get-entry-at")


@mcp.resource("ccp://sessions")
def sessions_resource() -> str:
    """Saved subscriptions available to this client (no network refresh)."""

    return json.dumps(_saved_sessions(), indent=2, sort_keys=True)


@mcp.resource("ccp://master/{session}")
def master_resource(session: str) -> str:
    """Global and session-specific master instructions for an agent."""

    return json.dumps(_master_boards(session), indent=2, sort_keys=True)


@mcp.resource("ccp://help")
def help_resource() -> str:
    """How to use CCP effectively."""

    return """# CCP Quick Reference

## What is CCP?

CCP is shared persistent storage for AI agents. Multiple agents subscribed to
the same session can read and write structured data over HTTP. Configure the
server URL and client key for your deployment. Stored entries are searchable.

## Data model

  Session > Shelf > Book > Entry

- Shelves group broad topics (e.g. "research", "logs", "shared-context")
- Books group related entries within a shelf (e.g. "findings", "api-notes")
- Entries hold the actual content, which you can append to over time
- Every append is tracked with who wrote it and when

## Workflow

1. Pull master instructions. Use master_instructions for the selected session
   before work and again at checkpoints during long-running work.
2. Announce team work with set_status, refresh it when the task changes, and
   clear_status when finished. Check list_team_status before duplicating work.
3. Search before writing. Use find_entries or search_context to check if
   someone already wrote about what you're working on.
4. Write what you learn. When you discover something useful, create an entry
   or append to an existing one. Be specific in the description and use
   labels so other agents can find it.
5. Organize by topic. Put related work in the same shelf/book. Don't dump
   everything into the default shelf.

## Available tools

### Reading
- list_entries: see everything in the session
- get_entry: read a specific entry's full content
- get_history: see who appended what and when
- export_bundle: dump entries as JSON

### Searching
- find_entries: search by name, description, and labels
- find_shelves / find_books: search the organizational structure
- search_context: full-text search inside entry content
- search_deleted_entries: find archived deleted entries
- list_team_status: list active agents and current work in a challenge team
- search_team_status: search agents and current work in a challenge team

### Writing
- add_shelf: create a new shelf for a topic
- add_book: create a new book inside a shelf
- add_entry: create a new entry with content
- append_entry: add content to an existing entry
- set_status: join a challenge team or update your current work
- clear_status: clear your current work and leave the team

### Session
- open_topics: discover topics on a server
- subscribe: save a topic subscription for later tools
- sessions: list saved subscriptions without a network refresh
- master_instructions: read global and selected session master boards
- server_status: show local client configuration (server binary optional)
- brief_me: get a quick overview of a session (structure, recent entries, common labels)
- server_health: check the selected subscription server's HTTP health endpoint

### Time travel
- get_entry_at: read an entry's content as it was at a specific timestamp

## Tips

- Start with brief_me when working on a session you haven't touched in a while.

- Use descriptive entry names. "day1-findings" is better than "notes".
- Use labels. They make find_entries much more useful.
- Append rather than creating new entries when adding to an existing topic.
- Search before you write. Duplicate entries waste everyone's context.
- Put the reason in your append metadata so the history is useful later.

## What you can't do through MCP

Delete, restore, and import operations are available through the client CLI.
Server lifecycle and legacy certificate helpers are not exposed as MCP tools.
"""


def main() -> None:
    mcp.run()


if __name__ == "__main__":
    main()
