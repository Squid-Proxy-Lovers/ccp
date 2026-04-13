# Droplet File Format

A `.droplet` file is a JSON document containing CCP entries, their metadata, and optionally their full append history. It's produced by `ccp-client export` and consumed by `ccp-client import`.

The file is plain JSON. The `.droplet` extension is a convention so tools and people can tell it apart from random JSON files.

## Top-level structure

```json
{
  "session": { ... },
  "selector": { ... },
  "exported_at": "1775031101",
  "entries": [ ... ],
  "bundle_sha256": "a1b2c3..."
}
```

| Field | Type | Description |
|---|---|---|
| `session` | object | Metadata about the session this was exported from |
| `selector` | object | What was selected for export (scope + history flag) |
| `exported_at` | string | Unix timestamp of when the export happened |
| `entries` | array | The actual entries |
| `bundle_sha256` | string | SHA-256 hex digest over the JSON-serialized `entries` array. Verified on import. |

## Session

```json
{
  "session_name": "my-session",
  "session_id": 1,
  "description": "...",
  "owner": "...",
  "labels": [],
  "visibility": "private",
  "purpose": "..."
}
```

## Selector

The selector records what scope was used during export.

```json
{
  "scope": "Session",
  "include_history": true
}
```

Scope variants:

| Scope | Shape |
|---|---|
| Full session | `"Session"` |
| One shelf | `{"Shelf": {"shelf": "research"}}` |
| One book | `{"Book": {"shelf": "research", "book": "findings"}}` |
| Named entries | `{"Entries": {"shelf": "research", "book": "findings", "entries": ["day1", "day2"]}}` |

## Entry

Each entry in the `entries` array:

```json
{
  "name": "day1-findings",
  "description": "first day of research",
  "labels": ["recon", "api"],
  "context": "the actual content of the entry...",
  "shelf_name": "research",
  "book_name": "findings",
  "shelf_description": "collected research",
  "book_description": "key findings",
  "history": [ ... ]
}
```

| Field | Type | Description |
|---|---|---|
| `name` | string | Entry name (unique within shelf/book) |
| `description` | string | Short description |
| `labels` | string[] | Tags for search |
| `context` | string | The full content |
| `shelf_name` | string | Which shelf this belongs to |
| `book_name` | string | Which book this belongs to |
| `shelf_description` | string | Description of the shelf |
| `book_description` | string | Description of the book |
| `history` | array | Append history (empty if exported with `--no-history`) |

## History entry

Each item in an entry's `history` array:

```json
{
  "operation_id": "uuid",
  "client_common_name": "client-cn",
  "agent_name": "codex",
  "host_name": "workstation",
  "reason": "follow-up research",
  "appended_content": "the text that was appended",
  "created_at": "1775031101"
}
```

| Field | Type | Description |
|---|---|---|
| `operation_id` | string | Unique ID for this append operation |
| `client_common_name` | string | Which client cert made the append |
| `agent_name` | string or null | Agent name from append metadata |
| `host_name` | string or null | Host name from append metadata |
| `reason` | string or null | Why this append was made |
| `appended_content` | string | What was appended |
| `created_at` | string | Unix timestamp |

## Integrity

The `bundle_sha256` field is a lowercase hex SHA-256 digest of the JSON-serialized `entries` array. The server computes the hash independently on import and rejects the bundle if it doesn't match. This catches file corruption and tampering.

## Conflict policies

When importing a droplet into a session that already has entries with the same names:

| Policy | What happens |
|---|---|
| `error` | Abort the whole import. Nothing is changed. |
| `overwrite` | Replace existing entries with the ones from the droplet. |
| `skip` | Keep existing entries, only import new ones. |
| `merge-history` | Keep existing entry content, add any history rows from the droplet that aren't already present (matched by `operation_id`). |
