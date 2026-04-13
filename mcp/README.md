# CCP MCP Bridge

FastMCP bridge that gives Claude, Cursor, Codex, and other MCP-compatible agents access to CCP. The bridge shells out to the Rust client binary for all protocol work.

## Install

```bash
bash install.sh --client
```

This installs the client binary, sets up the Python venv, and auto-configures your agent's MCP settings.

## How agents learn CCP

When an agent connects, it gets two things:

1. The MCP `instructions` field tells it CCP is shared memory, to search before writing, and to read the help resource.
2. The `ccp://help` resource has a full guide: data model, workflow, every tool, and tips for organizing data effectively.

Agents don't need to be prompted about CCP. The MCP instructions and help resource give them enough context to start using it on their own.

## Resources

- `ccp://help` how to use CCP, data model, tool reference, tips
- `ccp://sessions` enrolled sessions for this client

## Available tools

Agents get read, search, creation, and append. Destructive operations and server management are CLI-only.

### Read
`list_entries`, `get_entry`, `get_history`, `get_entry_at`, `export_bundle`

### Search
`find_entries`, `find_shelves`, `find_books`, `search_context`, `search_deleted_entries`

### Write
`add_shelf`, `add_book`, `add_entry`, `append_entry`

### Session
`enroll`, `sessions`, `brief_me`, `server_status`, `server_health`

### CLI-only (not exposed to agents)
`delete_entry`, `delete_shelf`, `restore_entry`, `import_bundle`, `revoke_certificate`, `start_server`, `stop_server`, `restart_server`, `rename_session`, `delete_session`

## Environment variables

| Variable | What it does |
|---|---|
| `CCP_CLIENT_BIN` | Path to the ccp-client binary |
| `CCP_SERVER_BIN` | Path to the ccp-server binary (full install only) |
| `CCP_CLIENT_HOME` | Client enrollment storage directory |
