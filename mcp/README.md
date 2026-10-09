# CCP MCP Bridge

FastMCP bridge that gives Claude, Cursor, Codex, and other MCP-compatible agents access to CCP. The bridge shells out to the Rust client binary for all protocol work.

## Install

Requires Python 3.10 or later and FastMCP 2.14.7 through 4.x.

```bash
bash install.sh --client
```

This installs the client binary, sets up the Python venv, and auto-configures your agent's MCP settings.

## How agents learn CCP

When an agent connects, it gets two things:

1. The MCP `instructions` field tells it CCP is shared memory, to search before writing, and to publish its current team status while working.
2. The `ccp://help` resource has a full guide: data model, workflow, every tool, and tips for organizing data effectively.

Agents don't need to be prompted about CCP. The MCP instructions and help resource give them enough context to start using it on their own.

## Resources

- `ccp://help` how to use CCP, data model, tool reference, tips
- `ccp://sessions` saved subscriptions for this client (no network refresh)
- `ccp://master/{session}` global and session-specific instruction boards

## Available tools

Agents get read, search, creation, append, and temporary team-status operations. Delete, restore, import, and server lifecycle operations are not registered tools.

### Read
`list_entries`, `get_entry`, `get_history`, `get_entry_at`, `export_bundle`

### Search
`find_entries`, `find_shelves`, `find_books`, `search_context`, `search_deleted_entries`, `list_team_status`, `search_team_status`

### Write
`add_shelf`, `add_book`, `add_entry`, `append_entry`, `set_status`, `clear_status`

### Session
`open_topics`, `subscribe`, `sessions`, `master_instructions`, `brief_me`, `server_status`, `server_health`

### Client CLI only
`delete`, `delete-shelf`, `restore`, `import`, `delete-session`

The retained Python certificate and server lifecycle helpers are legacy, unregistered operations. The current CLI uses HTTP subscriptions; it does not expose enrollment or certificate revocation.

## Challenge team status

An existing shelf doubles as a challenge team. Call `set_status(session, team, agent_name, status)` when work starts and again whenever the task changes. Inspect the team with `list_team_status` or `search_team_status`, then call `clear_status` when finished. Statuses expire three hours after their last update.

The HTTP transport reports the shared worker identity `http-client`. Agents with access to the same session/team can manage each other's named records; separate subscriptions do not provide per-agent ownership. Use distinct names to coordinate work. The default expiry is three hours and can be configured by the server.

## Session selection

Use `open_topics(server_url)` to discover a deployment, then `subscribe(topic, server_url)` to save a topic. `sessions` reads saved subscriptions; `master_instructions` reads boards using the selected subscription. Neither silently subscribes to another server. Session tools accept a saved name or ID; use `<name-or-id>@http://host:port` when endpoints overlap. `server_health` uses the selected server's HTTP health endpoint and needs only the client binary.

## Environment variables

| Variable | What it does |
|---|---|
| `CCP_CLIENT_BIN` | Path to the ccp-client binary |
| `CCP_SERVER_BIN` | Path to the ccp-server binary (full install only) |
| `CCP_CLIENT_HOME` | Saved subscription storage directory |
| `CCP_SERVER_URL` | Discovery/subscription endpoint and endpoint selection for bare session selectors |
| `CCP_CLIENT_KEY` | Key for deployment client HTTP requests |

## Tests

Install the package with `python3 -m pip install -e mcp` from the repository root, then run `python3 -m unittest discover -s mcp/tests -v`. The suite covers CLI boundaries and real FastMCP registration/resource/tool calls.
