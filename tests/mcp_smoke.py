"""Exercise FastMCP's protocol transport through the real CCP CLI and TLS server."""

from __future__ import annotations

import asyncio
import importlib
import json
import os
import tempfile
from pathlib import Path

from dependency_compat import Server, free_port, server_env


def data(result):
    value = getattr(result, "data", None)
    if value is None:
        value = getattr(result, "structured_content", None)
    if value is None:
        value = json.loads("".join(item.text for item in result.content if hasattr(item, "text")))
    while hasattr(value, "root"):
        value = value.root
    if hasattr(value, "model_dump"):
        value = value.model_dump(mode="json")
    return value


async def check():
    from fastmcp import Client

    root = Path(__file__).resolve().parents[1]
    binary = Path(os.environ.get("CCP_SERVER_BIN", root / "target/release/server")).resolve()
    client_binary = Path(os.environ.get("CCP_CLIENT_BIN", root / "target/release/client")).resolve()
    with tempfile.TemporaryDirectory(prefix="ccp-mcp-dependency-") as temp:
        temp_root = Path(temp)
        auth, mtls = free_port(), free_port()
        while mtls == auth:
            mtls = free_port()
        env = server_env(temp_root / "data", auth, mtls)
        os.environ.update(CCP_CLIENT_BIN=str(client_binary), CCP_SERVER_BIN=str(binary),
                          CCP_CLIENT_HOME=str(temp_root / "client"), CCP_SERVER_HOME=str(temp_root / "managed"))
        bridge = importlib.import_module("ccp_mcp_server.server")
        with Server(binary, env, auth) as server:
            async with Client(bridge.mcp) as client:
                tools = await client.list_tools()
                names = {tool.name for tool in tools}
                assert {"enroll", "add_shelf", "add_book", "add_entry", "append_entry", "get_entry", "brief_me"} <= names
                resources = await client.read_resource("ccp://help")
                assert any("session" in item.text for item in resources if hasattr(item, "text"))

                async def call(name, **arguments):
                    result = await client.call_tool(name, arguments)
                    assert not getattr(result, "is_error", False), name
                    return data(result)

                session = Server.session
                await call("enroll", token=server.token(), redeem_url=f"http://127.0.0.1:{auth}/auth/redeem")
                await call("add_shelf", session=session, shelf_name="notes", shelf_description="MCP notes")
                await call("add_book", session=session, shelf_name="notes", book_name="records", book_description="MCP records")
                location = dict(session=session, shelf_name="notes", book_name="records", entry_name="entry")
                await call("add_entry", **location, entry_description="MCP entry", entry_data="MCP initial", labels=["dependency"])
                await call("append_entry", **location, content="MCP appended")
                entry = await call("get_entry", **location)
                assert "MCP initial" in entry["context"] and "MCP appended" in entry["context"]
                brief = await call("brief_me", session=session)
                assert brief["total_entries"] == 1
                assert "dependency" in brief["frequent_labels"]
    print("PASS: MCP tool discovery, help, enrollment, writes, append, reads and briefing over TLS")


if __name__ == "__main__":
    asyncio.run(check())
