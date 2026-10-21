# Explicit workspace status messages (unreleased)

The workspace API and desktop panel expose explicit status messages. Dedicated
client/MCP status commands are not implemented yet. The native interface is unchanged.

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
is not stored as full revision history. Administrative moderation remains unimplemented pending its policy decision.
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
