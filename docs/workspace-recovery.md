# Recover an existing workspace client

This unreleased internal API lets an authorized administrator issue a fresh
individual recovery invitation for an existing registered client. It preserves
the client UUID and current grants. It exposes no web endpoint, CLI command or
browser workflow yet. The future adapters must authenticate administrative
actors, establish server trust before transmitting secrets, and use these same
transactions. See the [documentation index](README.md) for supported interfaces.

## Choose the authorized scope

Instance administrators can recover across a client's currently authorized
workspaces. Workspace owners/admins can recover only their own workspace.
Instance keys exercise global powers only, and workspace keys exercise their
own scoped powers only, even when a principal holds both roles. A globally
disabled client cannot receive or redeem recovery; recovery never re-enables it.

`issue_recovery_invitation` takes the existing immutable client UUID, explicit
instance/workspace scope and a future deadline of at most seven days. The server
captures the currently live grant IDs in that scope, including readable archived
workspaces. It excludes purged workspaces and revoked grants. An empty scope is
rejected. No new membership is created and initial group enrollment is unchanged.

The returned 256-bit bearer secret is available only at issuance. The database
stores its verifier, target client, scope, captured grants and issuing identity
and key. No secret is implicitly logged or serialized. Treat each unused
recovery invitation as a credential until it expires or is revoked; issuing
another invitation does not automatically revoke earlier invitations.

## Redeem with a new client key

`redeem_recovery_invitation` takes the recovery secret, stable request UUID and
signed PKCS#10 CSR for a client-held private key. The target client and workspace
are server records, never redemption claims. An old expired certificate or
knowledge of a client UUID does not authorize this operation. The CSR key must
not have been registered previously to this client, including on expired,
revoked or superseded credentials or in another workspace. Reusing such a key
is rejected before any credential or binding changes. Exact retry of a committed
recovery still returns its original public certificate.

The transaction rechecks the invitation, current issuer/key authority, enabled
client and captured grants. A grant revoked or workspace purged since issuance
is excluded; a later grant is never added. At least one captured grant must
remain authorized. Current grant roles apply, including changes made after
issuance. Archived context remains readable and content writes stay frozen.

The server issues a new certificate using the same UUID-based
[certificate identity and trust contract](workspace-enrollment.md#certificate-identity-and-trust).
It revokes all older credential bindings for the recovered grant IDs and binds
only those grants to the new credential. Binding revocations retain the old
records, timestamp and recovery-invitation ID. It does not revoke unrelated
workspace bindings, revoke the whole client identity or restore a revoked grant.

For example, scoped recovery of A gives the new credential A only. An old
credential that also has B keeps its authorized B binding. Its A binding cannot
be copied into a subsequent renewal, and old invitation receipts cannot restore
it. Recovery of an expired client works through the fresh administrative bearer
invitation, without accepting its expired certificate as authentication.

Certificate, replacement bindings, receipt and audit commit under one immediate
SQLite write transaction. Signing, SQL or audit failure leaves the original
working bindings intact. The same client UUID preserves identity continuity for
the later entry-ownership layer; this batch does not implement that content layer.

## Retry, inspect or revoke

The same recovery secret, request UUID and verified CSR public key retrieve the
identical committed certificate, IDs and recovered grant IDs. A different UUID
or public key conflicts; no second certificate is minted. Identical concurrent
retries share one committed result. A receipt can be retrieved after the
invitation deadline because it returns public material already committed.

Retrieval still requires a nonrevoked invitation, current issuer/key authority,
a live enabled unsuperseded new credential and at least one currently readable
original recovered binding. Later admissions never change the old receipt or
keep it available after all recovered access is revoked. Revocation, expiry,
global disable, subsequent renewal or replacement of all recovered bindings
cannot be bypassed through retry.

`recovery_invitation_status` returns scoped metadata without a secret.
Observing expiry through status or an otherwise authorized fresh redemption
persists closure, so rolling the clock back cannot reopen the invitation.
`revoke_recovery_invitation` requires the corresponding current scoped/global
recovery authority. Revoking an invitation denies redemption and receipt
retrieval; it does not revoke an already-issued certificate or membership.
Use grant revocation, another authorized recovery or global client disable for
the corresponding access change.

## Storage and verification limits

Schema 10 adds verifier-only recovery invitations, issuance-time grant capture,
retained binding revocations and immutable recovery receipts. Same-client and
credential/grant foreign keys constrain the records. The
[foundation upgrade and backup rules](workspace-foundation.md#database-upgrade)
apply; schema-9 renewal and earlier supported native identities are preserved.

Tests cover scoped/global recovery, expired clients, current role/key loss,
archived reads, partial grant revocation, immutable capture, exact retries,
competing requests, rollback, schema-9 upgrade and restart. Full network identity
verification, server-trust setup, local operator reset and explicit targeted
readmission of a revoked workspace member remain subsequent work.
