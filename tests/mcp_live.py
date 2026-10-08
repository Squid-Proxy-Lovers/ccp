#!/usr/bin/env python3
"""Verify FastMCP -> real CLI -> HTTP -> persistent storage in isolation.

Build server/client first and install the MCP package, then run this script.
No existing subscriptions, server data, or configured agents are modified.
"""

# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

import asyncio
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
from urllib.parse import quote
from urllib.request import Request, urlopen
from uuid import uuid4


ROOT = Path(__file__).resolve().parents[1]
SESSION = "mcp-live"


def http_json(url, *, method="GET", data=None, admin_key=None):
    headers = {}
    if admin_key:
        headers["X-CCP-Admin-Key"] = admin_key
    if data is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(data).encode()
    with urlopen(Request(url, data=data, headers=headers, method=method), timeout=2) as response:
        return json.load(response)


def wait_ready(process, base_url):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"server exited during startup: {process.returncode}")
        try:
            if http_json(base_url + "/health")["status"] == "ok":
                return
        except (OSError, ValueError):
            pass
        time.sleep(0.05)
    raise TimeoutError("server did not become ready")


def stop(process):
    if process.poll() is None:
        process.terminate()
    try:
        code = process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
        raise TimeoutError("server did not complete graceful shutdown")
    if code != 0:
        raise RuntimeError(f"server shutdown failed: {code}")


async def exercise(base_url, admin_key, *, after_restart=False):
    from fastmcp import Client
    from ccp_mcp_server.server import mcp

    selector = f"{SESSION}@{base_url}"
    path = {"session": selector, "entry_name": "entry-one", "shelf_name": "team", "book_name": "notes"}
    initial = "-initial text\n\né 日本"
    appended = "--literal append\nfollow-up"

    async with Client(mcp) as client:
        async def call(name, **arguments):
            result = await client.call_tool(name, arguments)
            assert not result.is_error, (name, result)
            # Lists, including [], use a structured result wrapper. Read the
            # wire payload rather than FastMCP 2.x's generated Any-value models.
            structured = getattr(result, "structured_content", None)
            if structured is not None:
                if isinstance(structured, dict) and set(structured) == {"result"}:
                    return structured["result"]
                return structured
            return json.loads(result.content[0].text)

        if after_restart:
            entry = await call("get_entry", **path)
            assert entry["context"] == initial + "\n" + appended
            rows = await call("get_history", **path)
            assert len(rows) == 1 and rows[0]["agent_name"] == "live-agent"
            boards = await call("master_instructions", session=selector)
            assert boards["global"]["content"] == "global instruction"
            assert boards["session"]["content"] == "session instruction"
            return

        topics = await call("open_topics", server_url=base_url)
        assert any(topic["session_name"] == SESSION for topic in topics)
        await call("subscribe", topic=SESSION, server_url=base_url)
        saved = await call("sessions")
        assert len(saved) == 1 and saved[0]["endpoint"] == base_url
        assert (await call("server_health", session=selector))["status"] == "ok"

        http_json(base_url + "/v1/admin/master", method="PUT", data={"content": "global instruction"}, admin_key=admin_key)
        http_json(base_url + f"/v1/admin/sessions/{SESSION}/master", method="PUT", data={"content": "session instruction"}, admin_key=admin_key)
        boards = await call("master_instructions", session=selector)
        assert boards["global"]["content"] == "global instruction"
        resource = await client.read_resource("ccp://master/" + quote(selector, safe=""))
        assert json.loads(resource[0].text) == boards

        await call("add_shelf", session=selector, shelf_name="team", shelf_description="--team description")
        await call("add_book", session=selector, shelf_name="team", book_name="notes", book_description="notes")
        entry = await call("add_entry", **path, entry_description="--entry description", entry_data=initial, labels=["live", "日本"])
        assert entry["context"] == initial
        await call("append_entry", **path, content=appended, agent_name="live-agent", host_name="loopback", reason="integration test")
        entry = await call("get_entry", **path)
        assert entry["context"] == initial + "\n" + appended
        matches = await call("search_context", session=selector, query="follow-up")
        assert any(match["name"] == "entry-one" for match in matches)
        status = await call("set_status", session=selector, team="team", agent_name="-scout", status="--reviewing pipeline")
        assert status["agent_name"] == "-scout" and status["status"] == "--reviewing pipeline"
        statuses = await call("list_team_status", session=selector, team="team")
        assert len(statuses) == 1
        assert (await call("clear_status", session=selector, team="team", agent_name="-scout"))["cleared"]
        assert await call("list_team_status", session=selector, team="team") == []
        bundle = await call("export_bundle", session=selector)
        assert len(bundle["entries"]) == 1 and len(bundle["entries"][0]["history"]) == 1
        # Exercise a checkpointing mutation through CLI, then the advertised
        # query-less archive tool through actual MCP before restoring the entry.
        cli = os.environ["CCP_CLIENT_BIN"]
        deleted = subprocess.run([cli, "delete", "--shelf=team", "--book=notes", "--", selector, "entry-one"], check=True, capture_output=True, text=True, timeout=30)
        archived = await call("search_deleted_entries", session=selector)
        assert len(archived) == 1 and archived[0]["name"] == "entry-one"
        entry_key = json.loads(deleted.stdout)["entry_key"]
        subprocess.run([cli, "restore", "--", selector, entry_key], check=True, capture_output=True, text=True, timeout=30)
        assert (await call("get_entry", **path))["context"] == initial + "\n" + appended
        # Sessions/master reads must not rewrite the saved subscription endpoint.
        assert (await call("sessions"))[0]["endpoint"] == base_url


def main():
    server_bin = Path(os.environ.get("CCP_LIVE_SERVER_BIN", ROOT / "target/release/server")).resolve()
    client_bin = Path(os.environ.get("CCP_LIVE_CLIENT_BIN", ROOT / "target/release/client")).resolve()
    if not server_bin.is_file() or not client_bin.is_file():
        raise SystemExit("Build server and client first: cargo build --locked --release -p server -p client")
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        address = listener.getsockname()
    base_url = f"http://{address[0]}:{address[1]}"
    with tempfile.TemporaryDirectory(prefix="ccp-mcp-live-") as work:
        work = Path(work)
        admin_key = str(uuid4())
        os.environ.update({
            "CCP_SERVER_DATA_DIR": str(work / "server"),
            "CCP_CLIENT_HOME": str(work / "client"),
            "CCP_SERVER_HOME": str(work / "runtime"),
            "CCP_CLIENT_BIN": str(client_bin),
            "CCP_SERVER_BIN": str(server_bin),
            "CCP_SERVER_URL": base_url,
            "CCP_HTTP_BASE_URL": base_url,
            "CCP_HTTP_LISTENER_ADDR": f"{address[0]}:{address[1]}",
            "CCP_CLIENT_KEY": str(uuid4()),
            "CCP_ADMIN_KEY": admin_key,
            "CCP_SESSION_VISIBILITY": "public",
        })
        for after_restart in (False, True):
            with (work / "server.log").open("a") as log:
                process = subprocess.Popen([str(server_bin), SESSION], stdout=log, stderr=log)
                try:
                    wait_ready(process, base_url)
                    asyncio.run(exercise(base_url, admin_key, after_restart=after_restart))
                    stop(process)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
        print("Live MCP/CLI/HTTP flow and graceful restart persistence passed.")


if __name__ == "__main__":
    main()
