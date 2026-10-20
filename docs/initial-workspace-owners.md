# Provision the first workspace owner

The unreleased workspace foundation can atomically appoint the first owner and
deliver that owner's first scoped admin key. The
[workspace HTTPS API](workspace-http-api.md) exposes creation and first-owner
appointment with the same transactions. The browser creation flow remains later.

[Documentation index](README.md) · [Scoped admin keys](workspace-admin-keys.md)

## Create or appoint once

A currently authorized instance/server administrator can use
`create_workspace_with_owner_key` to create a workspace, appoint its owner and
receive the initial owner key in one transaction. Use
`assign_initial_owner_with_key` for a workspace whose owner is still unassigned,
such as a migrated legacy workspace. This requires global creation authority;
a workspace admin, workspace-scoped owner key or client cannot do it.

Choose the owner explicitly through `OwnerProvisioning`:

| Selection | Result |
| --- | --- |
| `InitialOwner::Existing(UUID)` | Appoint that existing, enabled administrative principal |
| `InitialOwner::NewPrincipal(name)` | Create a fresh, unprivileged principal with that display label, then appoint it |

Names do not match existing people or claim a legacy `sessions.owner` label.
The new principal receives no instance role or other workspace memberships.
An existing global principal can own the new workspace, but its owner key still
has only that workspace's scope. The original instance key retains only global
powers.

New workspaces start active. First appointment can also apply to an unowned
archived workspace: the new owner can read and perform permitted security or
lifecycle administration, while content writes and admission stay frozen.
Purged or missing workspaces refuse appointment. Existing contents, legacy
display labels, client grants and closed enrollment remain unchanged.

## Deliver the secret explicitly

`ProvisionedWorkspace` contains the workspace UUID and an `IssuedAdminKey`
whose principal UUID identifies the owner. The result has no automatic
Debug, Serialize or Clone implementation. HTTPS adapters explicitly
return the key once, for delivery to the selected owner. Do not log it or put it
in audit records, discovery output or ordinary workspace metadata.

The key contains 256 bits of fresh randomness. The database stores only its
SHA-256 verifier. It remains valid until revoked by default; optional
`key_expires_at` must be later than the trusted server time. Key authentication
continues to check current scope, roles, principal enablement and workspace
state.

The principal, workspace/appointment, key verifier and audits commit together.
SQL, key-generation or audit failure rolls everything back. Audits retain the
original global principal and authenticating key ID; provisioning never converts
a global key into an unrestricted owner actor. Concurrent first appointments
commit exactly one owner/key and leave no losing principals behind.

## Continue under owner authority

Once the first appointment commits, it cannot be repeated to retrieve a lost
secret, issue another key or transfer ownership. A global key alone cannot
perform routine workspace-key issuance or reset. An authorized workspace owner
uses the normal owner-only key operations to manage their team and credentials.
Owner-initiated ownership transfer remains separate implementation work.

If delivery is lost, inspect the current IDs through the authenticated workspace
listing/inspection API or [local administration](local-workspace-administration.md). A working owner key
can issue a replacement. If all owner keys are lost, the trusted host operator
can run the existing scoped local reset. It preserves the owner UUID and other
keys/scopes, and requires exclusive data-directory access. No remote universal
reset, first-visitor claim, reusable onboarding token or deterministic secret
reconstruction is introduced.

Older low-level creation/appointment primitives remain available for trusted
embedded callers, but they do not deliver credentials. The HTTPS creation routes
use these composite operations to preserve the first-key delivery boundary.
Browser session, CSRF and human secret-delivery workflows remain pending.
