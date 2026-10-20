# MCP bridge

Use CCP from Claude, Cursor, Codex, or another MCP-compatible host. The Python
FastMCP bridge invokes the Rust client for data operations.

This guide covers the supported native `ccp-mcp-server` entry point. The separate,
unreleased [workspace MCP bridge](../docs/workspace-mcp.md) exposes workspace
discovery, explicit caller selections, content reads, shelf/book creation and
durable entry mutations with guarded outcome recovery through
`ccp-workspace-mcp`.

[Documentation index](../docs/README.md) · [Tool-call reference](../docs/tool-call-api.md)

## Install

Requires Python 3.10+ and FastMCP 2.14.7 through 4.x. From a repository checkout:

```bash
bash install.sh --client
```

The installer adds the client binary, creates a Python environment, and configures
supported agent hosts. For installation without a checkout or for nightlies, see
[the README](../README.md#install) and [nightly guide](../docs/nightly-builds.md).
Enroll with the server using the [client instructions](../docs/client.md#enroll-and-select-a-session).

## Agent workflow

The bridge supplies MCP instructions telling agents to search before writing,
share useful findings, and read `ccp://help`. That resource describes the data model,
workflow, tools, and organization examples. Host behavior determines when an agent
reads and follows those instructions.

## Tools and resources

| Group | Tools |
| --- | --- |
| Read | `list_entries`, `get_entry`, `get_history`, `get_entry_at`, `export_bundle` |
| Search | `find_entries`, `find_shelves`, `find_books`, `search_context`, `search_deleted_entries` |
| Write | `add_shelf`, `add_book`, `add_entry`, `append_entry` |
| Session and diagnostics | `enroll`, `sessions`, `brief_me`, `server_status`, `server_health` |

The [tool-call reference](../docs/tool-call-api.md) documents all arguments and
response shapes. `ccp://sessions` lists this client's enrolled sessions;
`ccp://help` supplies usage guidance.

Delete, delete-shelf, restore, import, revoke-cert, and server lifecycle operations
remain CLI-only. See the [client reference](../docs/client.md#cli-reference).

## Environment variables

| Variable | Purpose |
| --- | --- |
| `CCP_CLIENT_BIN` | Path to the client binary |
| `CCP_SERVER_BIN` | Path to the server binary for local diagnostics |
| `CCP_CLIENT_HOME` | Client enrollment storage directory |

`server_status` resolves both binaries. `server_health` runs a local server command
and needs access to the server's stored data. Installing a client for a remote server does not provide that data directory.

## Development verification

Use [CONTRIBUTING.md](../CONTRIBUTING.md) from the repository root. `make lint-python`
checks the bridge and smoke scripts; `make mcp-smoke` exercises actual tools through
the CLI and a temporary TLS server. CI repeats the smoke across supported
Python/FastMCP combinations.
