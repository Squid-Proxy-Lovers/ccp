# Manage a workspace's administrators and keys

This unreleased HTTPS API lets a workspace owner appoint administrators, rotate
scoped keys and transfer ownership. Use the owner's workspace key and verify the
server CA and name before sending secrets. See the [workspace HTTPS guide](workspace-http-api.md)
for startup, request limits and the JSON envelope, and the [documentation index](README.md).
Normal client/MCP and browser workflows remain subsequent work.

## Appoint and deliver a key

POST `/api/v1/admin/team/appoint` with `workspace_id`, `principal` and optional
`key_expires_at`. Choose the principal explicitly:

- `{"kind":"new_principal","name":"ADMIN_LABEL"}` creates a fresh principal.
- `{"kind":"existing","principal_id":"UUID"}` selects that enabled principal.

Replace illustrative values with the intended label or UUID. Labels do not
identify existing people. The recipient must not already own or administer the
workspace. Appointment requires an active workspace and its current owner;
instance keys and scoped admin keys cannot perform it, including when their
principal holds another role elsewhere.

Principal creation, the scoped admin role, its first key and audit records commit
in one transaction. The response contains `data.key_id`, `data.principal_id` and
`data.secret`. Deliver the secret privately to the intended person. It grants
that workspace's admin powers, including client admission/data management, but
no owner team/key/transfer powers or global role. Keys default to valid until
revoked; a supplied expiry must be a future Unix timestamp. Private verifier
storage and current-role checks follow the [admin-key contract](workspace-admin-keys.md).

After a lost response, inspect the roster and key metadata before retrying.
Repeating a new label creates another identity; it cannot recover the first
secret. Reappointing the existing principal while its role remains current is
denied. The owner can instead issue a fresh key and revoke the undelivered one.
There is no secret retrieval or remote physical-reset endpoint.

## Inspect, rotate or revoke

All routes below use authenticated HTTPS POST and reject unknown fields. UUIDs
are strings. `workspace_id` always selects an explicit workspace scope.

| Route under `/api/v1/admin/` | JSON fields | Result |
| --- | --- | --- |
| `team/list` | `workspace_id`, optional `after`, optional `limit` | `admins`, `next_cursor` |
| `keys/list` | `workspace_id`, optional `after`, optional `limit` | `keys`, `next_cursor` |
| `keys/issue` | `workspace_id`, `principal_id`, `name`, optional `expires_at` | One-time `key_id`, `principal_id`, `secret` |
| `keys/revoke` | `workspace_id`, `key_id` | `completed: true` |
| `team/revoke` | `workspace_id`, `principal_id` | `completed: true` |

These actions require the current owner, using that workspace's key. Global
read access and ordinary scoped admin access do not expose this security roster
or key inventory. Metadata contains no secret or verifier. Roster entries have
`id`, `name`, `enabled` and `is_owner`; keys have `id`, `principal_id`, `name`,
`expires_at`, `revoked_at` and `role_active`. The last flag describes the
principal's enabled scoped role, not the key's expiry/revocation or a lease of
future authority. Every operation rechecks current permissions.

Pagination defaults to 100, maximum 500, ordered by UUID. Pass `next_cursor` as
`after` in the next body. Cursors confer no access. Pages are separate current
snapshots; concurrent inserts may sort before an earlier cursor. Historical
revoked keys remain visible to the authorized owner.

A new key requires an enabled current owner/admin recipient and does not change
that recipient's roles. Key revocation affects only the selected key in the
specified workspace. Administrator revocation removes that scoped admin role
and terminally revokes all its keys in that workspace; unrelated/global roles
and keys remain intact. An owner must transfer ownership instead of revoking
itself through `team/revoke`. Reappointing a former admin issues a new key and
also closes any historical unrevoked keys left from older role removal, so old
keys and their pending invitations cannot regain authority.

Archived workspaces permit owner metadata reads and key rotation/revocation for
existing roles, as well as administrator revocation. New administrator
appointment and ownership transfer require an active workspace.

## Transfer ownership

First appoint the intended recipient as a scoped administrator, deliver its key
and confirm that the recipient can use it. Then POST
`/api/v1/admin/workspaces/transfer-owner` with `workspace_id` and `new_owner_id`.
The current owner must authorize this request with its workspace key. An
instance key, scoped admin key or a key for another workspace cannot transfer.

The recipient must be a different, enabled current scoped admin with at least
one unrevoked, unexpired workspace key record. This registry check does not
prove that the person received the secret; verify delivery before transferring.
The transaction changes the single owner, removes the recipient's redundant
admin row, removes any former-owner admin row in this workspace and terminally
revokes all former-owner workspace keys. It preserves unrelated roles, keys,
client grants and history. Audit failure rolls back every change.

The recipient's existing key now exercises owner powers; no new owner secret is
returned. Requests using keys registered to the former owner's principal are
denied, including on an already open connection. Bearer authentication cannot
distinguish people sharing copies of another principal's key. The recipient can
issue a new key and revoke previously shared keys after transfer.
Any separately held instance key
retains its global authority, including global read, but gains no routine
workspace administration. Pending invitations issued through revoked keys also
lose their issuing authority.

After a lost response, do not assume the old key can retry. The recipient can
inspect ownership with its key, a global admin can read the workspace's owner
metadata, or the host operator can use [local administration](local-workspace-administration.md).
Physical key recovery resets keys for the current owner, not the former owner.
Two competing transfers from the same old owner produce one winner; there is
no ownerless intermediate state. Permanent workspace deletion remains separate.

## Verification limits

Core and actual localhost TLS tests exercise owner-only scope, one-time key
output, metadata pagination, role/key expiry and revocation, reappointment,
archived security, rollback, concurrent transfer and authority changes on an
existing connection. Native compatibility and MCP smoke remain separate tests.
These checks do not establish a browser workflow, production rollout,
throughput guarantee or physical key delivery.
