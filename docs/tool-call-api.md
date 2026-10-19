# MCP tool-call reference

This document describes the CCP MCP bridge tools in [server.py](../mcp/src/ccp_mcp_server/server.py).

The bridge exposes the tools listed below. Delete, import, revoke, restore, and
server lifecycle operations remain CLI-only.

Pass the JSON argument object to the named tool. Response values are illustrative;
field names and types describe the payload. The sample transfer digest is a
placeholder; export a bundle to get a verified importable digest.

[Documentation index](README.md) · [MCP setup](../mcp/README.md)

## Tool index

| Group | Tool links |
| --- | --- |
| Session and diagnostics | [`server_status`](#server_status), [`enroll`](#enroll), [`sessions`](#sessions), [`server_health`](#server_health) |
| Read | [`list_entries`](#list_entries), [`get_entry`](#get_entry), [`get_history`](#get_history), [`get_entry_at`](#get_entry_at), [`brief_me`](#brief_me), [`export_bundle`](#export_bundle) |
| Search | [`find_entries`](#find_entries), [`find_shelves`](#find_shelves), [`find_books`](#find_books), [`search_context`](#search_context), [`search_deleted_entries`](#search_deleted_entries) |
| Write | [`add_shelf`](#add_shelf), [`add_book`](#add_book), [`add_entry`](#add_entry), [`append_entry`](#append_entry) |

[Response shapes](#shared-response-shapes) · [Resources](#mcp-resources)

## Conventions

- `session` in client-backed tools is a saved client session selector. In practice this is a `session_name` or `session_id`.
- `server_health` runs a local server command for the named session and requires access to its data directory.
- Object-returning session tools may include `ccp_certificate_warning` when the selected client cert is expired or near expiry.
- `labels` is always a JSON string array.
- Entry data is stored in the `context` field on the wire. The add API calls this input `entry_data`.

## Session and diagnostics

### `server_status`

Return resolved client/server command paths and key local directories. Both
commands must be resolvable; this diagnostic can fail on a client-only installation.

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

Redeem a time-limited token and save the resulting client enrollment locally.

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
  "message": "Saved enrollment for session 'my-session' (id=1) access=read_write client_cn=...",
  "summary": "Saved enrollment for session 'my-session' (id=1) access=read_write client_cn=...",
  "client_cert_expires_at": 2088752390,
  "stored_at": "/path/to/enrollment"
}
```

### `sessions`

List sessions available from saved client enrollments.

Arguments:

```json
{
  "filter_text": "optional substring"
}
```

Response: [Session Summary[]](#session-summary)

### `server_health`

Get health status of a CCP server session.

Returns server status, active session count, issued/revoked certificates, database path, journal path, and certificate expiry information.

Arguments:

```json
{
  "session": "my-session"
}
```

Response:

```json
{
  "status": "healthy",
  "session_name": "my-session",
  "protocol_version": 1,
  "schema_version": 10,
  "active_sessions": 1,
  "issued_certs": 3,
  "revoked_certs": 0,
  "database_path": "sessions/my-session-4b8f0352/ccp.sqlite3",
  "journal_path": "sessions/my-session-4b8f0352/runtime-journal.jsonl",
  "ca_cert_path": "sessions/my-session-4b8f0352/ccp_ca_cert.pem",
  "server_cert_path": "sessions/my-session-4b8f0352/ccp_server_cert.pem",
  "issued_certs_list": [
    {
      "common_name": "78525bb6-52cf-490b-8048-4eb483246392",
      "session_id": 1,
      "access_level": "read_write",
      "created_at": "2026-03-17T20:10:30Z",
      "expires_at": "2036-03-17T20:10:30Z"
    }
  ],
  "revoked_certs_list": []
}
```

## Session data tools

### `list_entries`

List entry summaries for a session.

Arguments:

```json
{
  "session": "my-session"
}
```

Response: [Entry Summary[]](#entry-summary)

### `find_entries`

Search entries by name, description, labels, shelf metadata, and book metadata.

Arguments:

```json
{
  "session": "my-session",
  "query": "release notes"
}
```

Response: [Entry Summary[]](#entry-summary)

### `find_shelves`

Search shelf names and descriptions.

Arguments:

```json
{
  "session": "my-session",
  "query": "engineering"
}
```

Response: [Shelf Summary[]](#shelf-summary)

### `find_books`

Search book names and descriptions.

Arguments:

```json
{
  "session": "my-session",
  "query": "runbook"
}
```

Response: [Book Summary[]](#book-summary)

### `search_context`

Search entry data and return snippet matches.

Arguments:

```json
{
  "session": "my-session",
  "query": "follow up"
}
```

Response: [Search Context Match[]](#search-context-match)

### `search_deleted_entries`

Search deleted entries, or list all deleted entries when `query` is omitted or empty.

Arguments:

```json
{
  "session": "my-session",
  "query": ""
}
```

Response: [Deleted Entry Summary[]](#deleted-entry-summary)

### `get_entry`

Fetch one full entry by name, optionally scoped to a shelf/book.

Arguments:

```json
{
  "session": "my-session",
  "entry_name": "build-notes",
  "shelf_name": "engineering",
  "book_name": "runbooks"
}
```

Response: [Message Entry](#message-entry)

### `add_shelf`

Create a shelf or update its description.

Arguments:

```json
{
  "session": "my-session",
  "shelf_name": "engineering",
  "shelf_description": "Platform engineering notes"
}
```

Response: [Shelf Add Result](#shelf-add-result)

### `add_book`

Create a book in an existing shelf or update its description.

Arguments:

```json
{
  "session": "my-session",
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "book_description": "Operational references"
}
```

Response: [Book Add Result](#book-add-result)

### `add_entry`

Create a new entry in an existing shelf/book using a `read_write` enrollment.

Arguments:

```json
{
  "session": "my-session",
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

Response: [Message Entry](#message-entry)

### `append_entry`

Append content to an existing entry.

Arguments:

```json
{
  "session": "my-session",
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

Return append history for one entry.

Arguments:

```json
{
  "session": "my-session",
  "entry_name": "build-notes",
  "shelf_name": "engineering",
  "book_name": "runbooks"
}
```

Response: [Message History Entry[]](#message-history-entry)

### `export_bundle`

Export a session, shelf, book, or named entries as a JSON bundle.

Arguments:

```json
{
  "session": "my-session",
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

Response without `output_path`: [Transfer Bundle](#transfer-bundle)

Response with `output_path`:

```json
{
  "written_path": "/tmp/export.json"
}
```

### `brief_me`

Return session structure, counts, up to ten recently updated entries, and up to
ten frequent labels. Use it when starting work in an enrolled session.

Arguments:

```json
{
  "session": "my-session"
}
```

Response:

```json
{
  "session_name": "my-session",
  "session_id": 1,
  "total_entries": 1,
  "total_shelves": 1,
  "total_books": 1,
  "shelves": [
    {"shelf_name": "engineering", "description": "Notes", "book_count": 1, "entry_count": 1}
  ],
  "recent_entries": [
    {"name": "build-notes", "description": "Example entry", "shelf_name": "engineering", "book_name": "runbooks", "updated_at": "1773392400"}
  ],
  "frequent_labels": ["demo"]
}
```

### `get_entry_at`

Reconstruct an active entry's content at a Unix timestamp in seconds. Use the
same shelf/book path as the entry; omitted locations select `main/default`.

Arguments:

```json
{
  "session": "my-session",
  "entry_name": "build-notes",
  "at_timestamp": "1773392400",
  "shelf_name": "engineering",
  "book_name": "runbooks"
}
```

Response: [Message Entry](#message-entry)

Reconstruction uses the current entry and retained append history. Imports or
incomplete history can limit reconstruction; retain original bundles and history
when you need an audit trail. A missing entry or a time before creation causes a
non-object response, which this MCP wrapper reports as an error.

## Shared response shapes

### Session summary

Returned by `sessions`.

```json
{
  "session_name": "my-session",
  "session_id": 1,
  "access": ["read_write"],
  "cert_count": 1,
  "endpoint": "https://localhost:1338",
  "session_description": "Runtime session for CCP inter-agent communication",
  "owner": "",
  "labels": [],
  "visibility": "private",
  "purpose": "Runtime session for CCP inter-agent communication",
  "latest_client_cert_expires_at": 2088752390,
  "cert_warning": null
}
```

### Shelf add result

Returned by `add_shelf`.

```json
{
  "shelf_name": "engineering",
  "description": "Platform engineering notes"
}
```

### Book add result

Returned by `add_book`.

```json
{
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "description": "Operational references"
}
```

### Entry summary

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

### Shelf summary

Returned by `find_shelves`.

```json
{
  "shelf_name": "engineering",
  "description": "Platform engineering notes",
  "book_count": 2,
  "entry_count": 14
}
```

### Book summary

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

### Message entry

Returned by `get_entry`, `get_entry_at`, and `add_entry`.

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

### Search context match

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

### Deleted entry summary

Returned by `search_deleted_entries`.

```json
{
  "entry_key": "engineering::runbooks::build-notes::1773392400000",
  "name": "build-notes",
  "description": "Example entry",
  "labels": ["demo"],
  "shelf_name": "engineering",
  "book_name": "runbooks",
  "shelf_description": "Platform engineering notes",
  "book_description": "Operational references",
  "deleted_at": "1773392400000",
  "deleted_by_client_common_name": "client-cn"
}
```

### Message history entry

Returned by `get_history`.

```json
{
  "operation_id": "op-123",
  "client_common_name": "client-cn",
  "agent_name": "codex",
  "host_name": "workstation",
  "reason": "follow-up",
  "appended_content": "new content",
  "created_at": "1773392400"
}
```

### Transfer bundle

Returned by `export_bundle` when `output_path` is omitted.

```json
{
  "session": {
    "session_name": "my-session",
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
  "exported_at": "1773392400",
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

## MCP resources

Resources are read-only data the agent can pull on demand.

- `ccp://help`: full guide on how to use CCP. Covers the data model, workflow, available tools, and tips for organizing data. The MCP instructions ask agents to read it before working with CCP.
- `ccp://sessions`: JSON list of enrolled sessions for this client.
