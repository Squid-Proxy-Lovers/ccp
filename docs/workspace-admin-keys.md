# Workspace admin keys

This unreleased internal batch adds admin-key issuance, authentication and
revocation to the workspace access foundation. It does not expose an HTTP
listener, browser login or session cookie yet. Local setup/reset commands exist;
network adapters must
preserve the checks here before they can serve users.

## Key scope and current authority

Keys authenticate a principal within one immutable scope: the instance, or one
workspace. They do not store a copy of the principal's role. A workspace key
cannot follow its principal's other workspace roles or global powers into
another scope. Instance keys exercise only global powers, even if the principal
also owns a workspace. Workspace keys exercise only that workspace's current
owner/admin powers, even if the principal is also a global administrator.
Each protected operation checks current key validity, scope,
principal enablement, role and workspace state inside its own transaction.

Only global administrators manage instance keys, and recipients must currently
be global administrators. Routine workspace-key management belongs to that
workspace's owner; recipients must already be its owner or an appointed admin.
The [initial-owner provisioning operation](initial-workspace-owners.md) can
deliver one owner key atomically with global-authorized creation or missing
first-owner appointment. It adds no ongoing global workspace-key power.
Issuance cannot appoint someone or add a role. A workspace admin cannot issue
keys, including another key for themselves. Personal and deliberately shared
keys use the same authority checks; a shared admin principal must hold an
explicit workspace role. Sharing a principal's secret shares that principal's
current powers within the key's scope.

Removing a principal's role invalidates its keys for that scope. A separate
global role does not revive a revoked workspace role. Disabling the principal
or revoking/expiring the key also invalidates previously authenticated actors
on their next operation. Global keys still require current global membership;
global read access alone confers no workspace publishing or admission power.

Keys have no expiry by default, with optional server-checked expiration. A
replacement is a separately issued key: rotation requires explicitly revoking
the old one. Owners can replace or revoke existing administrators' keys while
archived, without allowing content writes, new client admission or new admin
appointments. Purged workspaces invalidate their keys.

## Secret handling and audit

Issuance generates a cryptographically random 256-bit bearer secret and returns
it once. SQLite stores only its SHA-256 verifier, never the reusable secret.
The issued-key type deliberately lacks automatic Debug/Serialize output;
adapters must explicitly disclose the secret only in the issuance response.
No admin/invitation secret belongs in logs, discovery or audit payloads.

`authenticate_admin_key` returns an `Actor::AdminKey`, which adapters must keep
through authorization and mutations. Converting it into an unrestricted
`Actor::Admin` would discard the key's scope and revocation checks. The latter
variant is for trusted internal/local operations, not a caller-supplied UUID.
Authentication does not create an authority lease or a browser session.

Key creation/revocation and their audit records commit atomically. Actions made
with a key record both its principal and key ID, including self-revocation.
Local initial bootstrap commits the principal, global role, first key verifier
and audit together; it cannot be repeated once a global record exists. The
[local administration CLI](local-workspace-administration.md) now provides the
exclusive OS/data-directory boundary for setup and reset. No remote reset is
introduced by these primitives.

## Schema and verification

Schema 6 adds key verifiers and key attribution to access audit rows. Upgrade
from schema 5 preserves existing IDs, grants and audit records; old audit rows
have no key ID. The [foundation migration rules](workspace-foundation.md) still
apply, including refusal of historical HTTP schemas 3/4 and backup-based
rollback. Native wire protocol, client identity and enrollment are unchanged.

Tests exercise key scope across multiple roles/workspaces, owner-only issuance,
expiry/revocation, role loss/global demotion, disabled principals, archive
boundaries, secret/verifier separation, audit rollback, schema-5 upgrade and
revocation across database connections. HTTP, browser-session, fragment-login
and certificate flows are not established by those internal tests.

## Local setup and key recovery

The [local administration commands](local-workspace-administration.md) now
provision the first instance administrator and first workspace owner atomically
under exclusive maintenance. The same person receives two independently random
keys with separate scopes. Local reset replaces only an existing enabled
principal's instance keys or a workspace's current owner's scoped keys. It adds no routine global-key authority to issue workspace-owner credentials,
change ownership or re-enable a disabled principal. First-key delivery during
[initial provisioning](initial-workspace-owners.md) is a separate, one-time boundary.
