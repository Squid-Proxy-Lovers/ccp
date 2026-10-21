# Explicit workspace status messages (unreleased)

The workspace API, client CLI, MCP tools and desktop panel expose explicit status
messages. The native interface is unchanged.

[Documentation index](README.md) · [Workspace client](workspace-client.md) ·
[Workspace MCP](workspace-mcp.md)

A status message is a short, self-reported work description from a registered
client. It is not a task assignment, reservation, completion record or proof that
an agent is online. Normal CCP calls do not update its timestamp. Automatic
last-contact collection remains deferred.

## Identity and freshness

A caller chooses a stable status UUID for each independent message slot in a
workspace. The server derives ownership from the authenticated registered client;
requests cannot claim another client ID. Separate agents sharing a registration
can use separate slots. An optional agent label is display attribution and has no
independent authority. Two equal labels do not merge messages or identities.

An explicit status write records the server time. For the current engineering
default, a message is fresh for 300 seconds and stale at or beyond that age. A
clock rollback also marks the observation stale. Staleness never deletes or hides
the last message, and reading it never refreshes its age. Pages include the server
observation time and the freshness threshold. “Fresh” describes only this message’s
age, not current activity, connectivity or productivity.

Current workspace readers can see stored messages, including stale messages from
clients whose grants were later revoked or whose registration was disabled.
Those clients cannot post further updates without current authority. Global
administrators retain global read access. Archived workspaces stay readable and
reject status writes, clears and write replay.

## Current core operations

`AccessStore::mutate_work_status` requires a currently authorized client credential
and a read/write grant. A new status uses expected revision zero. Subsequent
updates use its exact current revision. The client owns its slots across renewal
or recovery; replacing credentials does not transfer ownership to another client.

A message contains 1–4096 UTF-8 bytes and must contain non-whitespace text. An
optional agent label contains 1–128 UTF-8 bytes and must also be nonblank. NUL is
refused in either field. Text otherwise remains exact. A client can have up to
64 uncleared slots per workspace. Lists use explicit UUID cursors and pages of
1–32 messages; they do not combine workspaces.

Clearing an owned message requires its current revision. It removes the current
message and label, leaving an ownership/revision tombstone. Clearing is explicit;
expiry alone never does this. A cleared slot can be deliberately republished by
its owning client using the tombstone’s current revision. Previous message text
is not stored as full revision history. Scoped owner/admin moderation is planned but not implemented yet.
The current human panel is a read-only status viewer.

## Exact operations and storage

Each new write has one operation UUID and expected revision. The server checks
current authority and atomically commits the head, body-free historical receipt
and audit record. An exact authorized retry under the original credential returns
its original receipt without reapplying the change or updating its timestamp.
Changed actor, target, text, label or expected revision under the same operation
UUID conflicts. Stale revisions also conflict without a replacement operation.
A new credential cannot replay an old credential’s operation, although the same
registered client can make a new update to its own slot.

Receipts contain workspace, status, operation and client UUIDs, revision, clear
state and original update time. They contain no message or label. A receipt is
historical proof, not the latest message. An old exact set replay after clearing
returns its old receipt and cannot resurrect text. Tombstones and receipts survive
clearing; they are not a full message-history or task-completion log.

Schema 23 adds empty status-head and receipt tables, with scoped identity,
ownership and receipt constraints. Migration does not publish a message, import
legacy reports, delete data or activate cleanup. Schema-22 software refuses the
new database. Rollback requires a complete matching schema-22 data-directory
backup, including its trust and registration material; old and new components
must not share a writable directory. See the
[workspace foundation](workspace-foundation.md) for the current storage boundaries.

## Read and update over TLS

All paths below start with `/api/v1/work/status/` and accept JSON POST only.
Reads use a current HTTPS admin key/session or an authenticated client mTLS
connection. Mutation exists only on the client mTLS listener: HTTPS returns 404,
even for a workspace owner or global administrator. A request cannot claim an
actor or client ID. [HTTPS security rules](workspace-http-api.md) and current
workspace permissions apply to every request.

| Path | JSON fields | Result in API1 `data` |
| --- | --- | --- |
| `get` | `workspace_id`, `status_id` | Latest status, including a body-free cleared tombstone |
| `list` | `workspace_id`; optional `after`, `limit` (1–32, default 32) | `items`, `next_cursor`, `observed_at`, `stale_after_seconds` |
| `mutate` | `workspace_id`, `status_id`, `operation_id`, `expected_revision`, `change` | Historical body-free receipt |

For set, `change` has `action: "set"`, optional `agent`, and `message`. For clear,
it contains only `action: "clear"`. Unknown fields are refused in both forms.
A first write uses revision zero and a chosen non-nil status/operation UUID.
Keep the exact original request and credential for deliberate retries; a read of
current state cannot attribute an uncertain operation. Never silently replace an
operation ID or expected revision.

A cleared `get` returns its retained client/revision with null `message` and
`agent`, and `stale: true`. It is not an active report, and `list` omits it.
Lists still include stale reports and disabled authors’ last messages. They are
separate bounded observations, not a complete snapshot across pages.

Invalid shapes, IDs, text, limits or live-slot quota return 400. Missing/expired
admin authentication returns 401; current scope or client authority failure
returns 403. A missing status is 404. Changed operation intent, stale revision or
write-time clock rollback is 409 `content_conflict`; SQL failures return sanitized
503 and commit nothing. Query strings are refused. Requests keep the existing
64 KiB encoded-body limit and 1 MiB response limit; status writes do not borrow
the larger entry-mutation budget. Bearer headers on client mTLS are rejected.

## Inspect reported work in the desktop panel

Choose a workspace, then **Reported work**. The view displays the registered
client UUID/name, optional self-reported agent label, exact message, original
report time, revision and slot ID. “Fresh at last read” and “Stale at last read”
refer to the displayed server observation time and threshold. They are not
present-tense liveness or completion claims. Use **Read current status messages**
to make another observation; there is no automatic polling or contact collection.

Use **Next status messages** and **First status messages** for independent pages.
An unavailable or mismatched response is shown as unavailable, never as an empty
workspace. Stale messages retain their last text/time until deliberately cleared
or updated by the authorized client. Global keys can read all workspaces; scoped
keys read only their own. Archived workspaces remain readable. The human view
publishes no client status and provides no administrative clear action.

Reported work is separate from recorded entry revisions in **Context & history**.
Labels and messages render as literal text. Workspace, key and tab changes retain
the existing dirty-draft guard and current-session checks. The browser stores no
status messages, identities or drafts in persistent browser storage.


## Send a status from the client

Use an enrolled private workspace profile. Create a mode-0600 regular request file
and save this JSON, replacing the illustrative UUIDs with the intended workspace
and fresh non-nil status/operation UUIDs:

```json
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "status_id": "22222222-2222-4222-8222-222222222222",
  "operation_id": "33333333-3333-4333-8333-333333333333",
  "expected_revision": 0,
  "change": {"action": "set", "message": "Checking the parser", "agent": "reviewer"}
}
```

```bash
ccp-client workspace status --profile ./my-profile mutate --request-file ./status.json
ccp-client workspace status --profile ./my-profile list --workspace-id "$WORKSPACE_ID" --limit 32
ccp-client workspace status --profile ./my-profile get --workspace-id "$WORKSPACE_ID" --status-id "$STATUS_ID"
ccp-client workspace status --profile ./my-profile pending
```

For a later update or clear, use a fresh operation UUID and the current revision.
A clear request replaces `change` with `{"action":"clear"}`. Request files must be
private regular files, not symlinks, and fit the 64 KiB encoded limit. Work text
never travels in argv. Mutation output contains the body-free receipt plus local
`operation` metadata; it does not return the message.

Before its first POST, the client saves the complete original request and its
instance, client and credential binding in the private profile. Normal preflight
reads may renew a due credential before that binding is captured. Once pending,
status/entry writes and credential lifecycle changes are blocked. Current reads
remain available under the original credential, with automatic renewal suppressed.
An error preserves pending state, including when the server committed but saving
the local result failed. No background or automatic retry chooses a new UUID,
revision or credential.

Inspect `pending` or the body-free `pending_status` field in `workspace info`.
Retain all five target fields: workspace, status, operation, credential UUIDs and
`request_hash`. The hash fingerprints the client's serialized original request;
it is a local resume guard, not a transferable server authorization token.
Deliberately resend that exact saved intent with:

```bash
ccp-client workspace status --profile ./my-profile resume \
  --workspace-id "$WORKSPACE_ID" --status-id "$STATUS_ID" \
  --operation-id "$OPERATION_ID" --credential-id "$CREDENTIAL_ID" \
  --request-hash "$REQUEST_HASH"
```

A mismatched or partial target is refused. Resume sends the saved request once;
current server authority still applies, including archive and revocation. A
committed `last_status` receipt is historical, and is not a pending resume target.
Expired/revoked keys cannot be silently substituted with a recovery profile.

If you deliberately choose to discard the local pending text, use the same five
flags with `forget` instead of `resume`, adding `--confirm-local-forget`. This
sends no server request and works independently of current server access. It
records `locally_forgotten: true`, `cancelled: false` and body-free last metadata.
**The original server request is not cancelled and may still complete.** Forgetting
is not proof that a replacement intent is safe. Server UUID/CAS checks continue
to apply. Logical removal does not erase SQLite free pages, snapshots or backups.
See [profile format 5](workspace-client.md#profile-storage-and-recovery-limits)
for migration and rollback limits.
