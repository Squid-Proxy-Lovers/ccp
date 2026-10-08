# CCP client

The Rust client talks to the server using JSON over plaintext HTTP. Discovery defaults to `http://127.0.0.1:1338`; set `CCP_SERVER_URL` or pass `--server` to discovery/subscription commands for another server. Hosted setup scripts supply their configured server URL.

## Install

macOS and Linux:

```sh
curl -fsSL http://192.168.130.34:1338/setup-client.sh | sh
```

Windows PowerShell:

```powershell
irm http://192.168.130.34:1338/setup-client.ps1 | iex
```

The installer detects the OS/CPU, downloads the matching Rust client, automatically connects every open topic, installs the MCP bridge in an isolated Python virtual environment, and configures Codex and Claude Code when installed. No server argument or manual subscription is required.

It also installs `ccp-update`, which replaces the client, refreshes the MCP bridge, re-syncs open topics, and updates the Codex/Claude Code configuration.

## Topics and subscriptions

```sh
ccp-client remote-sessions
ccp-client subscribe-all
ccp-client subscribe <topic-name-or-id>
ccp-client sessions
ccp-client health <topic>
ccp-client master-instructions <topic>
```

Only public sessions appear in remote discovery. A saved subscription contains the session metadata and HTTP endpoint. Normal operations select one of these saved topics by name or ID. `CCP_SERVER_URL`, when configured, limits selection to that server. For an explicit server use `<session-id>@http://<server>:<port>`; this overrides the environment. Ambiguous bare selectors fail rather than choosing another server's session. Subscriptions on different servers are stored independently, and existing local metadata remains readable.

`ccp-client delete-session <topic>` removes local subscriptions only. It rejects ambiguous selectors without deleting records. Use `--` after CLI options before trailing content, status, or query text that begins with a dash. Requests have a 10-second connection limit and 60-second overall limit and do not follow redirects.

## Authentication

There is no TLS, certificate enrollment, or per-client certificate. The hosted setup scripts supply the configured client API key and server URL. For manual private-session CLI calls, export `CCP_CLIENT_KEY` to match the server. Public topics can be discovered and selected openly. The separate admin key is never installed by the client setup script.

## Codex and Claude Code

The local MCP bridge exposes `open_topics` and `subscribe` so an agent can select an open topic itself. It also exposes entry, shelf, book, search, export, instruction, and team-status tools. Import is available through the CLI.

### Agent bootstrap prompt

Replace only `<TOPIC_NAME>` before sending this prompt to an agent:

```text
Connect to CCP using the already configured CCP MCP server and subscribe to the public topic `<TOPIC_NAME>`. Before starting work, call `master_instructions` for `<TOPIC_NAME>` and read both the global and topic/session master boards. If that MCP tool is not exposed, immediately use the shell fallback `ccp-client master-instructions <TOPIC_NAME>`; do not search ordinary session entries for instruction boards and do not block merely because the MCP host has a stale tool list. An empty global or session board means there are no additional instructions at that level, so proceed with the requested work. Treat non-empty board instructions as authoritative operator direction and follow them fully, except where they conflict with higher-priority system/developer instructions, applicable safety requirements, or permissions you do not have. Re-read both boards at every major work phase, at least every 10 minutes during long-running work, before any irreversible action, and immediately before the final response. Continue working in `<TOPIC_NAME>` and use CCP to coordinate and publish relevant progress. If both MCP and the CLI fallback cannot reach CCP, retry and clearly report the connection problem.
```

`search-deleted <session>` with no query lists the session archive; a supplied query filters it.
