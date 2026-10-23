"""Owned mTLS relay commits a real status, then damages only its response."""

from __future__ import annotations

import contextlib
import json
import socket
import sqlite3
import ssl
import threading
from http.client import HTTPSConnection
from urllib.parse import urlsplit
from uuid import uuid4


def profile_sql(fixture, statement, parameters=()):
    with sqlite3.connect(fixture.profile / "workspace-profile.sqlite3") as db:
        return db.execute(statement, parameters).fetchone()


def damage(proof, fault):
    if fault in {"workspace_id", "client_id"}:
        proof[fault] = str(uuid4())
    elif fault == "revision":
        proof[fault] += 1
    elif fault == "cleared":
        proof[fault] = True
    elif fault == "updated_at":
        proof[fault] = -1


def relay(fixture, fault):
    bundle = json.loads((fixture.data / "ccp-instance-tls.json").read_text())
    credential = json.loads(profile_sql(fixture, "SELECT credential FROM profile")[0])
    server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    server_context.load_cert_chain(
        fixture.secret_file(bundle["server_certificate"]), fixture.secret_file(bundle["server_key"])
    )
    server_context.verify_mode = ssl.CERT_REQUIRED
    server_context.load_verify_locations(cafile=str(fixture.ca))
    client_context = ssl.create_default_context(cafile=str(fixture.ca))
    client_context.load_cert_chain(
        fixture.secret_file(credential["receipt"]["certificate_pem"]),
        fixture.secret_file(credential["key_pem"]),
    )
    captured = []

    listener = socket.create_server(("127.0.0.1", 0))
    listener.settimeout(0.1)
    stop = threading.Event()

    def serve():
        while not stop.is_set():
            try:
                stream, _peer = listener.accept()
            except TimeoutError:
                continue
            with server_context.wrap_socket(stream, server_side=True) as tls:
                exchange(tls, fixture, fault, captured, client_context)

    thread = threading.Thread(target=serve, daemon=True)
    thread.start()
    return listener, stop, thread, captured


def read_request(stream):
    with stream.makefile("rb") as reader:
        method, path, _version = reader.readline().decode("ascii").split()
        headers = {}
        while (line := reader.readline()) != b"\r\n":
            assert line
            key, value = line.decode("ascii").split(":", 1)
            headers[key.lower()] = value.strip()
        body = reader.read(int(headers.get("content-length", 0)))
    assert (method, path) in {
        ("GET", "/api/v1/health"),
        ("POST", "/api/v1/work/status/list"),
        ("POST", "/api/v1/work/status/mutate"),
    }
    return method, path, body


def exchange(stream, fixture, fault, captured, client_context):
    method, path, body = read_request(stream)
    with contextlib.closing(
        HTTPSConnection(
            "127.0.0.1", urlsplit(fixture.mtls).port, context=client_context, timeout=10
        )
    ) as connection:
        connection.request(method, path, body or None, {"Content-Type": "application/json"})
        with connection.getresponse() as reply:
            assert reply.status == 200
            payload = json.loads(reply.read())
    mutate = path == "/api/v1/work/status/mutate"
    if mutate:
        captured.append({"request": json.loads(body), "receipt": dict(payload["data"])})
        damage(payload["data"], fault)
    encoded = b"{}" if mutate and fault == "truncated" else json.dumps(payload).encode()
    length = len(encoded) + (100 if mutate and fault == "truncated" else 0)
    status = "503 Unavailable" if mutate and fault == "refused" else "200 OK"
    header = f"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    stream.sendall(header.encode("ascii") + encoded)


@contextlib.contextmanager
def routed(fixture, fault):
    original = profile_sql(fixture, "SELECT trust FROM profile")[0]
    listener, stop, thread, captured = relay(fixture, fault)
    trust = json.loads(original)
    trust["mtls_origin"] = "https://127.0.0.1:" + str(listener.getsockname()[1]) + "/"
    profile_sql(fixture, "UPDATE profile SET trust=?", (json.dumps(trust),))
    try:
        yield captured
    finally:
        stop.set()
        thread.join(timeout=10)
        listener.close()
        assert not thread.is_alive()
        profile_sql(fixture, "UPDATE profile SET trust=?", (original,))


def check(fixture):
    workspace = fixture.setup["workspace_id"]
    for fault in [
        "truncated",
        "refused",
        "workspace_id",
        "client_id",
        "revision",
        "cleared",
        "updated_at",
    ]:
        request = {
            "workspace_id": workspace,
            "status_id": str(uuid4()),
            "operation_id": str(uuid4()),
            "expected_revision": 0,
            "change": {
                "action": "set",
                "agent": "\ufeff",
                "message": "PRIVATE_STATUS response fault",
            },
        }
        with routed(fixture, fault) as captured:
            try:
                fixture.command(
                    fixture.client,
                    "workspace",
                    "status",
                    "--profile",
                    fixture.profile,
                    "mutate",
                    "--request-file",
                    fixture.secret_file(json.dumps(request)),
                )
            except RuntimeError:
                pass
            else:
                raise AssertionError("damaged status receipt accepted: " + fault)
            pending = fixture.command(
                fixture.client, "workspace", "status", "--profile", fixture.profile, "pending"
            )
            assert pending["pending"]["operation_id"] == request["operation_id"]
            assert "PRIVATE_STATUS" not in json.dumps(pending)
            assert len(captured) == 1 and captured[0]["request"] == request
            proof = captured[0]["receipt"]
            target = pending["pending"]
        arguments = []
        for field in ["workspace_id", "status_id", "operation_id", "credential_id", "request_hash"]:
            arguments.extend(["--" + field.replace("_", "-"), target[field]])
        done = fixture.command(
            fixture.client,
            "workspace",
            "status",
            "--profile",
            fixture.profile,
            "resume",
            *arguments,
        )
        assert {key: done[key] for key in proof} == proof
        assert done["operation"]["request_hash"] == target["request_hash"]
        row = fixture.command(
            fixture.client,
            "workspace",
            "status",
            "--profile",
            fixture.profile,
            "get",
            "--workspace-id",
            workspace,
            "--status-id",
            request["status_id"],
        )
        assert row["revision"] == 1 and row["message"] == request["change"]["message"]
