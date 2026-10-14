# Droplet guide and format

A `.droplet` file is a JSON document containing CCP entries, their metadata, and optionally their full append history. It's produced by `ccp-client export` and consumed by `ccp-client import`.

[Documentation index](README.md) · [Client guide](client.md)

- [Export and import](#export-and-import)
- [Top-level structure](#top-level-structure)
- [Fields](#session)
- [Integrity](#integrity)
- [Conflict policies](#conflict-policies)

## Export and import

Export from a session you can read and import into a session where you have write
access. The CLI prints JSON to stdout unless you provide `--output`:

```bash
# Full session
ccp-client export my-session --output research.droplet
# One shelf
ccp-client export my-session --shelf recon --output recon.droplet
# One book
ccp-client export my-session --shelf recon --book endpoints --output endpoints.droplet
# Named entries in a book; repeat --entry to select more
ccp-client export my-session --shelf recon --book endpoints --entry routes --output routes.droplet
# Content without append history
ccp-client export my-session --shelf recon --no-history --output recon-clean.droplet
```

Enroll in the target session before importing. Choose a conflict policy deliberately:

```bash
ccp-client import another-session research.droplet
ccp-client import another-session research.droplet --policy overwrite
ccp-client import another-session research.droplet --policy skip
ccp-client import another-session research.droplet --policy merge-history
```

The default `error` policy aborts on collisions. `overwrite` replaces existing
entries; `skip` keeps them. `merge-history` keeps existing content and adds missing
history records by operation ID. A book selector requires a shelf; entry selectors
require both. Keep history when handing off append attribution or temporal context.

The `.droplet` extension identifies the JSON bundle. The server verifies the entry
digest before import, rolls back in-memory import changes if persistence fails,
and records the transfer. Only import bundles from sources you trust.


## Top-level structure

This complete example exports an empty session. The digest covers its empty entry
array. Examples of entries and history follow below.

```json
{
  "session": {
    "session_name": "my-session",
    "session_id": 1,
    "description": "Research context",
    "owner": "",
    "labels": [],
    "visibility": "private",
    "purpose": "Research"
  },
  "selector": {
    "scope": "Session",
    "include_history": true
  },
  "exported_at": "1775031101",
  "entries": [],
  "bundle_sha256": "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
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
  "history": []
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

The `bundle_sha256` field is a lowercase hex SHA-256 digest of the JSON-serialized `entries` array. The server computes the hash independently on import and rejects the bundle if it doesn't match. This detects mismatches and file corruption. A sender can change the entries and recompute the digest, so authenticity depends on trusting the sender separately.

## Conflict policies

When importing a droplet into a session that already has entries with the same names:

| Policy | What happens |
|---|---|
| `error` | Abort the whole import. Nothing is changed. |
| `overwrite` | Replace existing entries with the ones from the droplet. |
| `skip` | Keep existing entries, only import new ones. |
| `merge-history` | Keep existing entry content, add any history rows from the droplet that aren't already present (matched by `operation_id`). |
