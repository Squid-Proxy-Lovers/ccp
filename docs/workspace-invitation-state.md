# Invitation policy records

This unreleased internal batch persists invitation policies and initial-window
lifecycle. It does not redeem tokens, consume capacity, issue certificates or
admit clients through invitations yet. Those operations must be integrated in
one transaction in the next batch before an HTTP enrollment route is exposed.
The existing native enrollment path is unchanged.

## One initial group window

A workspace can open one initial group window before its first new-model
admission. It must have a positive client-capacity bound, a future deadline, or
both. Capacity/deadline and use counters are stored in SQLite. Closing a window
is terminal: issuing a new link or restarting the server cannot reopen it.
Existing grants, including revoked ones, prevent starting a later initial phase.

An active group must be explicitly closed before individual invitations can be
issued. Starting individual admission before any group window seals that unused
initial opportunity. Individual invitations have capacity one and a finite
expiry. This is an initial setup transition, not a way to rotate a group link
and reset its limits.

Administrative metadata checks persist observed deadline/capacity closure. Once
observed closed, a clock rollback cannot reopen the group. Automatic lifecycle
facts go into enrollment events; administrative issue/revoke/close actions also
write scoped access audit records. Repeated manual close is idempotent.

## Authority and secrets

Workspace owners/admins issue invitations only within their current scope.
Global read/lifecycle powers alone do not issue invitations. Workspace keys
retain their scope and role ceilings. Archived workspaces reject new invitations,
while scoped revocation/closure and metadata checks remain available.

Invitations record the issuing principal or exact admin key; metadata reports
whether that issuer is still authorized to admit clients. That boolean is not
an authorization lease. Redemption must independently recheck current issuer,
key, workspace, token and quota state in its own transaction. Metadata includes
no secret and must not be used as a substitute for token verification.

The 256-bit invitation secret is returned only at issuance. SQLite stores its
SHA-256 verifier. Issuance has no automatic Debug/Serialize disclosure. Revocation
is terminal and closing/revoking a group never resets its recorded policy.

## Schema and remaining integration

Schema 7 adds invitation, initial-enrollment and enrollment-event records to
schema 6 without replacing admin keys, identities, grants or access audit rows.
The [foundation backup/upgrade rules](workspace-foundation.md) apply.

Tests exercise bounds, scope, issuer freshness, audit rollback, archived security,
expiry and stored-capacity closure, migration and reopening persistence. The
capacity-transition test seeds an exhausted counter directly; it does not prove
atomic redemption or quota consumption. Before exposing enrollment, implement
and test proof of key possession, atomic identity/grant/certificate issuance,
idempotent retries, race-safe slot consumption, current revocation and bounded
recovery. No browser session or network listener is established by these tests.
