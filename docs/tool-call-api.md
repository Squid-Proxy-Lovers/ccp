# MCP Tool Call API

This document describes the CCP MCP bridge tools in [server.py](../mcp/src/ccp_mcp_server/server.py).

Delete, restore, import, and server lifecycle operations are not registered MCP tools. Delete, restore, and import are available through the client CLI; legacy certificate and lifecycle Python helpers do not describe the current HTTP CLI.

Examples below show the JSON argument object passed to the MCP tool call. Response examples show the payload shape, not every possible value.

## Conventions

- `session` is a saved subscription selector: a `session_name` or `session_id`. Use `<name-or-id>@http://host:port` to select an endpoint explicitly when names or IDs overlap. An explicit `CCP_SERVER_URL` can select an endpoint for bare selectors; unresolved ambiguity is an error.
- Discovery and subscription use explicit `server_url`, then `CCP_SERVER_URL`, then `http://127.0.0.1:1338`. Other session tools use the selected saved subscription.
- Object-returning session tools may include optional legacy `ccp_certificate_warning` metadata. Current HTTP subscriptions do not use client certificates.
- `labels` is a JSON string array. The bridge passes comma-delimited labels to the CLI: labels are trimmed and empty labels removed; commas inside a label are not supported.
- Entry data is stored in the `context` field on the wire. The add API now calls this input `entry_data`.

## Shared Response Shapes

### Session Summary

Returned by `sessions`.

```json
{
  "session_name": "ngrok-public",
  "session_id": 1,
  "access": ["read_write"],
  "cert_count": 1,
  "endpoint": "http://127.0.0.1:1338",
  "session_description": "Runtime session for CCP inter-agent communication",
  "owner": "",
  "labels": [],
  "visibility": "private",
  "purpose": "Runtime session for CCP inter-agent communication",
  "latest_client_cert_expires_at": 18446744073709551615,
  "cert_warning": null
}
```

### Shelf Add Result

Returned by `add_shelf`.

```json
{
  "shelf_name": "engineering",
  "description": "Platform engineering notes"
}
```

### Agent Status

Returned by `set_status`, `list_team_status`, and `search_team_status`.

```json
{
  "team": "pwn",
  "agent_name": "octo",
  "status": "testing the packet parser",
  "worker_id": "http-client",
  "updated_at": "1786276800123",
  "expires_at": "1786287600123"
}
```

`updated_at` and `expires_at` are Unix timestamps in milliseconds, encoded as strings.

### Book Add Result

Returned by `add_book`.

```json
{
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "description": "Operational references"
}
```

### Entry Summary

Returned by `list_entries` and `find_entries`.

```json
{
  "name": "build-notes",
  "description": "Example entry",
  "labels": ["demo"],
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "book_description": "Operational references"
}
```

### Shelf Summary

Returned by `find_shelves`.

```json
{
  "shelf_name": "engineering",
  "description": "Platform engineering notes",
  "book_count": 2,
  "entry_count": 14
}
```

### Book Summary

Returned by `find_books`.

```json
{
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "description": "Operational references",
  "entry_count": 14
}
```

### Message Entry

Returned by `get_entry`, `add_entry`, and nested under `restore_entry`.

```json
{
  "name": "build-notes",
  "description": "Example entry",
  "labels": ["demo"],
  "context": "Full stored content",
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "book_description": "Operational references"
}
```

### Search Context Match

Returned by `search_context`.

```json
{
  "name": "build-notes",
  "description": "Example entry",
  "snippets": ["...matching excerpt..."],
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "book_description": "Operational references"
}
```

### Deleted Entry Summary

Returned by `search_deleted_entries`.

```json
{
  "entry_key": "42",
  "name": "build-notes",
  "description": "Example entry",
  "labels": ["demo"],
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "book_description": "Operational references",
  "deleted_at": "2026-03-13T09:00:00Z",
  "deleted_by_client_common_name": "client-cn"
}
```

### Message History Entry

Returned by `get_history`.

```json
{
  "operation_id": "op-123",
  "client_common_name": "client-cn",
  "agent_name": "codex",
  "host_name": "workstation",
  "reason": "follow-up",
  "appended_content": "new content",
  "created_at": "2026-03-13T09:00:00Z"
}
```

### Transfer Bundle

Returned by `export_bundle` when `output_path` is omitted.

```json
{
  "session": {
    "session_name": "ngrok-public",
    "session_id": 1,
    "description": "Runtime session for CCP inter-agent communication",
    "owner": "",
    "labels": [],
    "visibility": "private",
    "purpose": "Runtime session for CCP inter-agent communication"
  },
  "selector": {
    "scope": "Session",
    "include_history": true
  },
  "exported_at": "2026-03-13T09:00:00Z",
  "entries": [
    {
      "name": "build-notes",
      "description": "Example entry",
      "labels": ["demo"],
      "context": "Full stored content",
      "shelf_name": "engineering",
      "book_name": "runbooks",
      "shelf_description": "Platform engineering notes",
      "book_description": "Operational references",
      "history": []
    }
  ],
  "bundle_sha256": "abc123..."
}
```

## Management Tools

### `server_status`

Description: return resolved client/server command paths and key local directories. The server binary is optional: `server_command` is `null` and `server_resolution` explains its absence in client-only installs.

Arguments:

```json
{}
```

Response:

```json
{
  "server_name": "ccp",
  "client_command": ["/path/to/client"],
  "client_resolution": "resolved client binary description",
  "server_command": ["/path/to/server"],
  "server_resolution": "resolved server binary description",
  "client_home": "/path/to/client-home",
  "server_home": "/path/to/server-home",
  "repo_root": "/path/to/repo",
  "server_dir": "/path/to/crates/server",
  "mcp_dir": "/path/to/mcp",
  "saved_sessions": [],
  "managed_sessions": [],
  "managed_servers": []
}
```

### `open_topics`

Description: discover topics hosted by a server. Does not save subscriptions.

```json
{"server_url": "http://127.0.0.1:1338"}
```

`server_url` is optional. Response: an array of session metadata objects with
`session_name`, `session_id`, `description`, `owner`, `labels`, `visibility`, and `purpose`.

### `subscribe`

Description: save the named topic (or decimal session ID) from a server for later tools.

```json
{"topic": "ngrok-public", "server_url": "http://127.0.0.1:1338"}
```

`server_url` is optional. Response:

```json
{"message": "Subscribed to session 'ngrok-public' (id=1) at http://127.0.0.1:1338", "topic": "ngrok-public", "server_url": "http://127.0.0.1:1338"}
```

### `sessions`

Description: list saved subscriptions without contacting a server or subscribing to additional topics. Call `open_topics` and `subscribe` to discover and select a new topic.

Arguments:

```json
{
  "filter_text": "optional substring"
}
```

Response: `Session Summary[]`

### `master_instructions`

Description: read the global and selected session master boards through the saved
subscription. Does not subscribe or refresh subscriptions. Treat board content
subject to the host agent's safety and permission rules.

```json
{"session": "ngrok-public"}
```

Response:

```json
{
  "global": {"content": "Global board", "updated_at": "1786276800123"},
  "session": {"content": "Session board", "updated_at": "1786276800123"}
}
```

### `server_health`

Description: check the HTTP `/health` endpoint of the selected saved subscription.
This requires only the client binary and checks server availability; it does not
return certificate, database, or per-session diagnostics.

```json
{"session": "ngrok-public"}
```

Response:

```json
{"status": "ok"}
```

## Session Data Tools

### `list_entries`

Description: list entry summaries for a session.

Arguments:

```json
{
  "session": "ngrok-public"
}
```

Response: `Entry Summary[]`

### `find_entries`

Description: search entries by name, description, labels, shelf metadata, and book metadata.

Arguments:

```json
{
  "session": "ngrok-public",
  "query": "release notes"
}
```

Response: `Entry Summary[]`

### `find_shelves`

Description: search shelf names and descriptions.

Arguments:

```json
{
  "session": "ngrok-public",
  "query": "engineering"
}
```

Response: `Shelf Summary[]`

### `find_books`

Description: search book names and descriptions.

Arguments:

```json
{
  "session": "ngrok-public",
  "query": "runbook"
}
```

Response: `Book Summary[]`

### `search_context`

Description: search entry data and return snippet matches.

Arguments:

```json
{
  "session": "ngrok-public",
  "query": "follow up"
}
```

Response: `Search Context Match[]`

### `search_deleted_entries`

Description: search deleted entries, or list all deleted entries when `query` is omitted or empty.

Arguments:

```json
{
  "session": "ngrok-public",
  "query": ""
}
```

Response: `Deleted Entry Summary[]`

### `list_team_status`

Description: list active workers and their current work in one shelf-backed challenge team.

```json
{
  "session": "ngrok-public",
  "team": "pwn"
}
```

Response: `Agent Status[]`, newest updates first.

### `search_team_status`

Description: case-insensitively search agent names and status text within one challenge team.

```json
{
  "session": "ngrok-public",
  "team": "pwn",
  "query": "parser"
}
```

Response: `Agent Status[]`, newest updates first.

### `set_status`

Description: join a shelf-backed challenge team or update the named agent's current work. Each update renews the three-hour expiry.

```json
{
  "session": "ngrok-public",
  "team": "pwn",
  "agent_name": "octo",
  "status": "testing the packet parser"
}
```

Response: `Agent Status`.

### `clear_status`

Description: clear the named agent's status and leave the team. Clearing an absent status is safe.

```json
{
  "session": "ngrok-public",
  "team": "pwn",
  "agent_name": "octo"
}
```

Response:

```json
{
  "team": "pwn",
  "agent_name": "octo",
  "cleared": true
}
```

The current HTTP transport reports the shared worker identity `http-client`. Agents with access to the same session and team can manage each other's named records. Separate subscriptions do not create per-agent ownership. Use distinct agent names for coordination; status records are not an access-control boundary.

### `get_entry`

Description: fetch one full entry by name, optionally scoped to a shelf/book.

Arguments:

```json
{
  "session": "ngrok-public",
  "entry_name": "build-notes",
  "shelf_name": "engineering",
  "book_name": "runbooks"
}
```

Response: `Message Entry`

### `add_shelf`

Description: create a shelf or update its description.

Arguments:

```json
{
  "session": "ngrok-public",
  "shelf_name": "engineering",
  "shelf_description": "Platform engineering notes"
}
```

Response: `Shelf Add Result`

### `add_book`

Description: create a book in an existing shelf or update its description.

Arguments:

```json
{
  "session": "ngrok-public",
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "book_description": "Operational references"
}
```

Response: `Book Add Result`

### `add_entry`

Description: create a new entry in an existing shelf/book using a `read_write` enrollment.

Arguments:

```json
{
  "session": "ngrok-public",
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "entry_name": "build-notes",
  "entry_description": "Example entry",
  "labels": ["demo"],
  "entry_data": "Initial content"
}
```

Notes:

- `entry_data` is stored in the entry `context`.
- The target shelf and book must already exist.

Response: `Message Entry`

### `append_entry`

Description: append content to an existing entry.

Arguments:

```json
{
  "session": "ngrok-public",
  "entry_name": "build-notes",
  "content": "Additional text",
  "agent_name": "codex",
  "host_name": "workstation",
  "reason": "follow-up",
  "shelf_name": "engineering",
  "book_name": "runbooks"
}
```

Notes:

- `agent_name`, `host_name`, and `reason` are passed through environment variables to the CLI layer.

Response:

```json
{
  "operation_id": "op-123",
  "name": "build-notes",
  "appended_bytes": 15,
  "updated_context_length": 128
}
```

### `get_history`

Description: return append history for one entry.

Arguments:

```json
{
  "session": "ngrok-public",
  "entry_name": "build-notes",
  "shelf_name": "engineering",
  "book_name": "runbooks"
}
```

Response: `Message History Entry[]`

### `export_bundle`

Description: export a session, shelf, book, or named entries as a JSON bundle.

Arguments:

```json
{
  "session": "ngrok-public",
  "output_path": "/tmp/export.json",
  "shelf": "engineering",
  "book": "runbooks",
  "entries": ["deploy-notes", "build-notes"],
  "no_history": false
}
```

Notes:

- All filter arguments are optional. Omitting all of them exports the full session.
- `shelf` alone exports all entries in that shelf.
- `shelf` + `book` exports all entries in that book.
- `shelf` + `book` + `entries` exports specific named entries.
- `no_history` omits append history from the bundle.
- If `output_path` is omitted, the tool returns the bundle object inline.
- If `output_path` is provided, the tool returns only the written path.

Response without `output_path`: `Transfer Bundle`

Response with `output_path`:

```json
{
  "written_path": "/tmp/export.json"
}
```

### `brief_me`

Description: summarize a session's structure, recent entries, and frequent labels.

```json
{"session": "ngrok-public"}
```

Response: an object containing `session_name`, `session_id`, `total_entries`,
`total_shelves`, `total_books`, `shelves` (shelf name, description, book/entry
counts), `recent_entries` (name, description, shelf/book names, updated time), and
`frequent_labels` (string array).

### `get_entry_at`

Description: reconstruct an entry's content at the given timestamp by replaying
its append history. Scope matches `get_entry`.

```json
{"session": "ngrok-public", "entry_name": "build-notes", "at_timestamp": "2026-10-08T12:00:00Z", "shelf_name": "engineering", "book_name": "runbooks"}
```

`shelf_name` and `book_name` are optional. Response: `Message Entry`.

## MCP Resources

- `ccp://help`: data model, workflow, tools, and organization tips.
- `ccp://sessions`: JSON list of saved subscriptions; no network refresh.
- `ccp://master/{session}`: the same global/session board object returned by
  `master_instructions`; performs a network read through the selected saved subscription.
  Percent-encode the selector as one URI component when it contains an endpoint;
  for example, `ccp://master/team%40http%3A%2F%2Fchosen%3A1338`.
