# Read and edit workspace content over HTTPS

This unreleased server API exposes workspace shelves, books and client-owned
entries. Start the [workspace HTTPS server](workspace-http-api.md#prepare-and-start)
and verify its instance CA first. The normal workspace client CLI and workspace
MCP bridge expose access and selections; the [normal client](workspace-content-client.md)
now also supports content reads and [entry mutations](workspace-content-mutations.md).
The normal client and [workspace MCP bridge](workspace-mcp.md) also read shelf
lifecycle state and archive summaries. The bridge supports shelf/book creation
and guarded entry mutations. The human panel remains later work. Native commands keep their native-mode API.

[Documentation index](README.md) · [Content storage and ownership](workspace-content.md) ·
[Workspace HTTPS setup](workspace-http-api.md)

## Choose the authenticated listener

Use the mandatory-mTLS listener with a registered client's valid certificate and
key. Its current workspace grant determines access. Do not send an Authorization
header there. Use the separate HTTPS listener with an admin bearer key for
administration. Both listeners expose authenticated content reads and entry actions
through the same store checks. Shelf transitions are admin HTTPS only. The client
self-service `entries/resolve-operation` route requires mTLS client identity;
admin keys receive 403 for that operation.
An instance-wide admin key can read all workspaces but cannot create or edit
content, even if its principal also owns the workspace. Use that workspace's
scoped owner/admin key for those writes. No anonymous or cookie login is added.

Every content request is POST with `Content-Type: application/json`. Targets are
explicit workspace and record UUIDs. There are no query parameters or implicit
selected/default workspaces in this server API. Unknown fields are rejected,
including claimed client, owner or administrator IDs. The server derives identity
from the actual TLS peer or current admin key; `agent` is attribution only.

For an admin API call, reuse the verified `ccp-ca.pem` and private
`owner-auth-header.txt` described in the [HTTPS examples](workspace-http-api.md).
The header file contains `Authorization: Bearer OWNER_KEY`, with your issued
workspace-scoped key and owner-only file permissions. Save a JSON request in
`content-request.json`, replacing the illustrative IDs below, then run:

```bash
curl --cacert ./ccp-ca.pem --header @owner-auth-header.txt \
  --header 'Content-Type: application/json' \
  --data-binary @content-request.json \
  https://127.0.0.1:8443/api/v1/content/entries/get
```

## Create the hierarchy and read entries

All paths below start with `/api/v1/content/`. Each request includes
`workspace_id`. Names and exact metadata follow the [storage bounds](workspace-content.md).

| Path | Other request fields | Result in `data` |
| --- | --- | --- |
| `shelves/create` | `name`, `description` | Shelf |
| `books/create` | `shelf_id`, `name`, `description` | Book |
| `shelves/get`, `books/get` | `node_id` | Shelf or book |
| `shelves/list` | Optional `after`, `limit` | `items`, `next_cursor` |
| `shelves/state` | `node_id` | Current shelf ID, workspace, lifecycle revision and archive state |
| `shelves/archived` | Optional `after`, `limit` | Safe shelf-summary `items`, `next_cursor` |
| `books/list` | `shelf_id`; optional `after`, `limit` | `items`, `next_cursor` |
| `entries/get` | `entry_id`; optional `revision` | Entry snapshot |
| `entries/list` | `book_id`, `archived`; optional `after`, `limit` | Summary `items`, `next_cursor` |
| `entries/history` | `entry_id`; optional `after`, `limit` | Revision metadata `items`, `next_cursor` |
| `entries/resolve-operation` | Original `credential_id`, `operation_id`, `request_hash` | Body-free committed receipt or cancellation |
| `entries/mutate` | `operation_id`, optional `agent`, `change` | `entry_id`, `revision` receipt |

A current or historical snapshot request looks like this. Omit `revision` or use
null for the current version; supply a positive revision for a historical one.

```json
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "entry_id": "22222222-2222-4222-8222-222222222222",
  "revision": 2
}
```

A snapshot separates `entry` (current identity, owner, head revision and times)
from `revision` (the requested version's revision, action, authenticated actor,
client/admin principal, agent and recorded time), `content` (text, description,
labels) and `appended_text` (the exact original contribution for an append).
The current entry revision can therefore differ from the historical version's
revision. History lists contain revision metadata without bodies.

Lists default to 32 items and accept limits 1–32. Copy `next_cursor` into `after`
until it is null. Hierarchy/entry cursors are UUID strings; history cursors are
revision numbers and start at 0. Record limits bound encoded JSON even when
hierarchy descriptions require heavy escaping. Reads use current permissions;
record IDs or prior ownership cannot bypass revocation or disabling.

## Mutate without losing another client's work

Use a new non-nil operation UUID for each intended mutation. For creation,
`change` contains `action: "create"`, `book_id`, `name` and `content`:

```json
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "operation_id": "33333333-3333-4333-8333-333333333333",
  "agent": "reviewer",
  "change": {
    "action": "create",
    "book_id": "44444444-4444-4444-8444-444444444444",
    "name": "findings",
    "content": {
      "text": "Initial findings",
      "description": "Review notes",
      "labels": ["review"]
    }
  }
}
```

For other actions, `change` contains `entry_id` and `expected_revision`:

| Action | Additional fields | Current authority |
| --- | --- | --- |
| `append` | `text` | Any workspace writer |
| `revise` | Full `content` object | Owning client with write, or scoped admin |
| `archive` | None | Owning client with write, or scoped admin |
| `restore` | None | Owning client with write, or scoped admin |

Append adds exact text without a separator. Revise replaces text, description and
labels together while preserving earlier snapshots and contributions. An ordinary
client cannot claim an admin-created entry, which has no client owner. Names,
parents and ownership cannot be changed by these requests.

On a revision conflict, read current state and decide what change is still
appropriate. Do not blindly retry an edit against a new head. If a transport
failure leaves a mutation's outcome uncertain, retry the same typed request with
the same operation UUID, agent and credential/key. A successful exact retry
returns the original receipt without a second append. Reusing its operation UUID
with different input or identity conflicts. Lost write access blocks replay too.
Renewal or recovery retains client ownership but changes credentials; do not
assume a new credential can replay an uncertain old operation. Use the [client operation resolution fence](workspace-operation-resolution.md)
to confirm the old commit or cancel the original intent before deciding on a
fresh operation. The [normal client adapter](workspace-content-mutations.md)
saves the original request before sending and exposes resume/resolve controls.

Shelf/book creation has no operation receipt: a repeated name conflicts instead
of overwriting an existing node. Inspect the existing node after an uncertain
creation response; do not assume a retry is a second successful creation.

## Archive and restore

Archived entry listings contain only IDs, name, owner, revision, status and times.
They contain no text, descriptions or labels. Full current/historical snapshot
and history reads return not-found while the entry is archived, for all roles.
Restore preserves ID, owner and versions. An active same-name entry in the
original book causes a conflict and is never replaced. Resolve that collision
separately before retrying the uncommitted restore operation.

Archived workspaces remain readable, including archive summaries, but reject all
content mutations and operation replay. Permanent purge, optional timed retention
and verified legacy conversion remain subsequent work.
No cleanup worker starts in this slice. Workspaces mapped from native sessions
return `content_conversion_required` until their journal-aware conversion exists;
continue using the original native data under the [mode rules](workspace-runtime-mode.md).

## Archive or restore a shelf

Use an authorized workspace owner/admin key on the HTTPS listener. POST to
`/api/v1/admin/content/shelves/transition`; it is unavailable on client mTLS,
including for ordinary writers. Global instance keys cannot borrow workspace
archive-management powers. The workspace must be active.

First read `/api/v1/content/shelves/state` with `workspace_id` and `node_id` for
its current lifecycle revision. Current readers and scoped admins can read
state and page `/api/v1/content/shelves/archived`; global admins can read across
workspaces. Summaries include only ID, workspace, name, lifecycle revision and
creation time, with no descriptions or descendant content. Pages default to 32,
accept 1–32, and use UUID `after` cursors.

Choose an operation UUID once. Save this request in `content-request.json`,
replacing the illustrative IDs and revision:

```json
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "shelf_id": "22222222-2222-4222-8222-222222222222",
  "operation_id": "33333333-3333-4333-8333-333333333333",
  "expected_revision": 1,
  "archived": true
}
```

Use the same verified CA and private header file from the setup above:

```bash
curl --cacert ./ccp-ca.pem --header @owner-auth-header.txt \
  --header 'Content-Type: application/json' \
  --data-binary @content-request.json \
  https://127.0.0.1:8443/api/v1/admin/content/shelves/transition
```

The `data` contains `operation_id` and a body-free `result`: `shelf_id`,
`workspace_id`, lifecycle `revision` and `archived`. To restore, deliberately
choose a new operation UUID, use the current lifecycle revision and set
`archived` false. Entry revisions are independent of shelf lifecycle revisions.
Archive hides descendants together while preserving IDs, ownership, metadata,
history and individual archive state. Restore leaves individually archived
entries archived. An active same-name shelf causes a conflict without renaming,
overwriting or partially restoring anything.

After a lost response, preserve the exact original key and request. An authorized
exact retry returns its historical receipt without reapplying, even after a later
operation changed visibility. The receipt does not prove current shelf state;
fetch state separately. Changing the original key, shelf, revision or desired
state under that UUID conflicts. Revoked authority also blocks retries. Reading
current state does not prove which uncertain operation produced it. Do not
blindly replace a retry's UUID or revision. Bounds stay 64 KiB encoded requests
and 1 MiB responses. This operation does not purge payloads or start timed cleanup.

## Bounds and failures

Entry mutation JSON is capped at 2 MiB, including escaping. Other request bodies
remain capped at 64 KiB. Entry snapshot responses are capped at 4 MiB to include
both a full body and its original append delta with worst-case escaping. Other
responses retain the 1 MiB cap. Field bounds still apply independently; larger
transport ceilings do not permit larger stored text. Existing HTTP framing,
header/body deadlines, admission budgets and current authentication remain in
force. An eager oversized uploader may see a connection reset after the early
413 response closes the connection.

Responses use JSON API version 1 with `data` or `error.code`; the native binary
wire is unchanged. Content failures contain no SQL, private paths, secrets or
request bodies. Common status/code pairs are:

| Status | Code | Meaning |
| --- | --- | --- |
| 400 | `invalid_request` | Invalid fields, UUIDs, bounds, cursor or request shape |
| 401 | `unauthorized` | Missing or invalid admin bearer key |
| 403 | `forbidden` | Current identity has insufficient access |
| 404 | `not_found` | Missing, foreign or archived full-content target; unknown route |
| 409 | `content_conflict` | Stale revision, duplicate active name, restore collision or mismatched operation replay |
| 409 | `operation_cancelled` | Exact original mutation was terminally cancelled |
| 409 | `content_conversion_required` | Authorized legacy workspace awaits verified conversion |
| 413 | `request_too_large` | Encoded request exceeds its route's byte limit |
| 503 | `service_unavailable` | Storage or response-capacity failure |

The [common API guide](workspace-http-api.md#limits-and-errors) documents method,
media, framing and timeout errors. Exact text and descriptions may contain
controls; consumers must escape them when rendering. Real localhost TLS tests
cover this contract; no deployment, throughput or power-loss guarantee is made.
