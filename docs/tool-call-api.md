# MCP Tool Call API

This document describes the CCP MCP bridge tools in [server.py](../mcp/src/ccp_mcp_server/server.py).

Not all operations are exposed to agents through MCP. Destructive operations (delete, import, revoke, restore) and server management (start, stop, restart, rename, delete session) are CLI-only. They still exist in the codebase but agents cannot call them.

Examples below show the JSON argument object passed to the MCP tool call. Response examples show the payload shape, not every possible value.

## Conventions

- `session` in client-backed tools is a saved client session selector. In practice this is a `session_name` or `session_id`.
- `session` in server-management tools is a managed server selector. In practice this is a `session_name` or `session_slug`.
- Object-returning session tools may include `ccp_certificate_warning` when the selected client cert is expired or near expiry.
- `labels` is always a JSON string array.
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
  "endpoint": "tcp://6.tcp.us-cal-1.ngrok.io:13311",
  "session_description": "Runtime session for CCP inter-agent communication",
  "owner": "",
  "labels": [],
  "visibility": "private",
  "purpose": "Runtime session for CCP inter-agent communication",
  "latest_client_cert_expires_at": 2088752390,
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

Description: return resolved client/server command paths and key local directories.

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

### `enroll`

Description: redeem a time-limited token and save the resulting client enrollment locally.

Arguments:

```json
{
  "token": "hex-token",
  "redeem_url": "https://example/auth/redeem"
}
```

Notes:

- `redeem_url` is optional only when exactly one managed server is running and it exposes `auth_redeem_url`.

Response:

```json
{
  "message": "Saved enrollment for session 'ngrok-public' (id=1) access=read_write client_cn=...",
  "summary": "Saved enrollment for session 'ngrok-public' (id=1) access=read_write client_cn=...",
  "client_cert_expires_at": 2088752390,
  "stored_at": "/path/to/enrollment"
}
```

### `sessions`

Description: list sessions available from saved client enrollments.

Arguments:

```json
{
  "filter_text": "optional substring"
}
```

Response: `Session Summary[]`

### `server_health`

Description: get health status of a CCP server session.

Returns server status, active session count, issued/revoked certificates, database path, journal path, and certificate expiry information.

Arguments:

```json
{
  "session": "ngrok-public"
}
```

Response:

```json
{
  "status": "healthy",
  "session_name": "ngrok-public",
  "active_sessions": 1,
  "issued_certs": 3,
  "revoked_certs": 0,
  "database_path": "sessions/ngrok-public-4b8f0352/ccp.sqlite3",
  "journal_path": "sessions/ngrok-public-4b8f0352/runtime-journal.jsonl",
  "ca_cert_path": "sessions/ngrok-public-4b8f0352/ccp_ca_cert.pem",
  "server_cert_path": "sessions/ngrok-public-4b8f0352/ccp_server_cert.pem",
  "issued_certs_list": [
    {
      "common_name": "78525bb6-52cf-490b-8048-4eb483246392",
      "session_id": 1,
      "access_level": "read_write",
      "created_at": "2026-03-17T20:10:30Z",
      "expires_at": "2089-03-17T20:10:30Z"
    }
  ],
  "revoked_certs_list": []
}
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

## MCP Resources

Resources are read-only data the agent can pull on demand.

- `ccp://help`: full guide on how to use CCP. Covers the data model, workflow, available tools, and tips for organizing data. Agents read this to understand CCP without extra prompting.
- `ccp://sessions`: JSON list of enrolled sessions for this client.
