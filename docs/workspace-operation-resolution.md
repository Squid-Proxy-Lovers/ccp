# Resolve an uncertain client entry mutation

For normal library/CLI use, follow [entry mutations](workspace-content-mutations.md).
That client saves the original request and exposes `resume` and `resolve` controls.

If an entry mutation loses its response, keep its original workspace, operation
UUID, credential UUID and exact typed request. This unreleased server endpoint
can confirm the original commit after renewal or recovery. Resolution is terminal:
if that intent has not committed, the same transaction records a permanent
cancellation before returning. A delayed request cannot apply that client intent
later. It does not execute or replay the mutation under a new credential.

[Documentation index](README.md) · [Content HTTP contract](workspace-content-http.md) ·
[Client renewal and recovery](workspace-client.md)

## Authenticate as the same registered client

POST `/api/v1/content/entries/resolve-operation` on the mandatory-mTLS listener
with a currently usable credential and current read access to the workspace.
The original credential is a target, not an authentication claim. Its persisted
client UUID must equal the authenticated client UUID. It may be expired, revoked
or superseded; the current credential must still be valid and authorized.

A read-only grant can resolve that client's own intents, including while the
workspace is archived. Cancellation only prevents future writes by that client
under the operation ID. It changes no entry, version, owner, grant or credential.
It cannot reserve another client's operation UUID. All agents using the same
registered client share this control, just as they share entry ownership.

Admin keys cannot use this client self-service endpoint. It does not impersonate
another client, restore revoked membership or bypass a global disable. Loss of
current read access refuses resolution too. A fresh recovery credential needs
its explicit current workspace grant. Normal client/CLI/MCP resolution and
mutation adapters remain subsequent work; raw HTTP callers manage these fields.

## Keep the original binding

Supply the original credential UUID, even when authenticating with its renewed
or recovered replacement. `request_hash` is the lowercase 64-character SHA-256
fingerprint described below. The UUIDs here are illustrative.

```json
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "operation_id": "22222222-2222-4222-8222-222222222222",
  "credential_id": "33333333-3333-4333-8333-333333333333",
  "request_hash": "0000000000000000000000000000000000000000000000000000000000000000"
}
```

The all-zero fingerprint is an illustrative placeholder, not a request digest.
Requests reject unknown fields, invalid UUIDs, nil operation UUIDs and fingerprints
that are not exactly 64 lowercase hexadecimal characters. The route keeps the
usual 64 KiB request and 1 MiB response limits.

The API-version-1 `data` response echoes those four fields and contains `outcome`:

| Outcome | Fields | Meaning |
| --- | --- | --- |
| `state: "committed"` | `entry_id`, `revision` | The exact original credential/request committed once |
| `state: "cancelled"` | No other outcome fields | This client intent cannot commit later |

Neither outcome contains text, descriptions, labels, contributions or history.
A committed receipt is available even when its entry is now archived; this does
not expose archived bodies. The revision identifies the original operation's
result, rather than the current head. Exact repeated resolution is idempotent.

A different original credential or fingerprint for an existing binding conflicts.
Keep the pending request if a response is lost or resolution fails. After a
confirmed cancellation, any desired write needs a fresh intent and operation ID;
read current state before deciding its expected revision. The cancelled operation
ID stays blocked for that registered client across all later credentials.

Different clients and administrators retain their own authority. If another
client used the same operation UUID, this endpoint does not expose or change its
receipt. The requesting client's intent is cancelled independently. A read-only
client therefore cannot block another client's write by cancelling its UUID.

## Compute the existing request fingerprint

The `entry-mutation-1` binding is SHA-256 of compact UTF-8 JSON for this array:

```text
["entry-mutation-1", action, agent-or-null, payload]
```

| Action | Payload array |
| --- | --- |
| `create` | `[book_id, name, content]` |
| `append` | `[entry_id, expected_revision, text]` |
| `revise` | `[entry_id, expected_revision, content]` |
| `archive`, `restore` | `[entry_id, expected_revision]` |

Use canonical lowercase hyphenated UUID strings, integer revisions and the exact
original name, agent, text, description and label order. Missing agent becomes
JSON null. `content` has its object keys sorted as `description`, `labels`, `text`.
Encoding follows `serde_json::to_vec` on a JSON value with no added whitespace;
the [server implementation](../crates/server/src/foundation/store/entries.rs)
defines this existing fingerprint version. The workspace/operation IDs select
the ledger row and the authenticated credential binds the actor separately.
The fingerprint itself grants no authority. A mismatched encoding conflicts
rather than guessing which mutation was intended.

## Failure and storage boundaries

Typed failures return only status and `error.code`: invalid input is 400,
insufficient current authority or a foreign/unknown original credential is 403,
a changed existing binding is 409 `content_conflict`, and a delayed exact
mutation after cancellation is 409 `operation_cancelled`. A delayed mutation
using changed input or another credential of that same client conflicts.
Current write checks still apply before any mutation replay, so permission loss
can produce 403 instead. Storage failures return a sanitized 503. Authorized
legacy workspaces still require verified conversion before content operations.

Authorization, original-client proof, receipt lookup and cancellation/audit share
one immediate SQLite transaction. A concurrent original mutation either commits
before resolution sees it, or encounters the cancellation before it can apply.
Audit failure rolls back the cancellation. SQL triggers enforce the same-client
receipt/cancellation exclusion as a backstop. Foreign-client records remain
independent. The cancellation binding rejects SQL UPDATE.

Server schema 17 adds body-free cancellation records while retaining schema-16
content, versions, receipts, identity and trust material. Keep a complete protected
pre-upgrade backup; a schema-16 server refuses schema 17, and rollback uses the
matching whole backup and software. Backup restoration can also roll back the
operation ledger. Future payload purge must retain body-free terminal records
while removing payloads; this slice implements no purge or cleanup worker.
Schema 18 also adds separate
[shelf lifecycle records](workspace-content.md#recover-a-whole-shelf); a schema-17
server refuses that schema. Schema 19 makes entry receipts independent of
version-row foreign keys, preserves their body-free proof and protects them
against UPDATE and DELETE. A schema-18 server refuses schema 19; keep the whole
matching pre-upgrade backup. No payload purge or new resolution permission is
introduced. No power-loss, deployment or throughput guarantee is claimed.
