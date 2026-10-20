# Readmit a revoked workspace member

An authorized workspace owner or administrator can explicitly readmit an
existing registered client after its workspace grants have all been revoked.
This unreleased API creates a new scoped grant and credential for the same
client UUID. Old grant revocations remain terminal. The
[workspace HTTPS API](workspace-http-api.md) exposes scoped issuance/status/revoke
and redemption through `/api/v1/enroll`. Normal client/MCP commands and browser
workflows remain subsequent work. See the
[documentation index](README.md) for supported interfaces.

## Issue a fresh targeted invitation

`issue_readmission_invitation` requires current scoped `AdmitClient` authority,
an active workspace, an enabled existing client and prior grant history in that
workspace. Every existing grant there must be revoked. Instance keys cannot use
a principal's separate workspace-owner role; workspace keys remain limited to
their current scoped roles.

The administrator chooses the target client UUID and data role. The server
records that client, the workspace and the complete set of prior grant IDs with
the invitation. It rejects missing clients, clients without history in that
workspace, globally disabled clients and clients with a live grant there.
Ordinary initial-group or individual invitations do not become targeted because
a caller supplies a client name or credential ID.

This is an individual invitation: one use and a positive future deadline of at
most seven days. An active initial group must first be explicitly closed.
Starting individual mode seals an unused initial opportunity. Readmission never
reopens or extends initial group enrollment. The invitation secret has the same
verifier-only storage and issuance-only disclosure as other
[workspace invitations](workspace-invitation-state.md).

## Redeem for the captured identity

The normal `redeem_invitation` transaction recognizes the server's stored target.
Redemption takes a stable request UUID and a signed CSR with
`existing_credential` absent. Existing-credential claims are rejected for targeted
invitations. The target is determined by the issued invitation, never by a
redemption claim or knowledge of the old client UUID. A fresh, targeted bearer
invitation authorizes this operation; an expired certificate does not.

The CSR must use a key not previously registered to the target client, including
expired, revoked, superseded or other-workspace keys. Private keys stay with the
client. The server uses the same verified CSR, issued-key and certificate UUID
[identity contract](workspace-enrollment.md#certificate-identity-and-trust).

Redemption rechecks the issuer's current role/key, workspace lifecycle, global
client disable, invitation state and complete historical snapshot under one
SQLite write lock. If any grant has been created in that workspace since
issuance, the pending ticket is stale. This remains true after that intervening
grant is revoked. The administrator must issue a fresh targeted invitation after
reviewing the latest state. Two pending tickets cannot silently undo a later
admission and revocation.

Success creates a new grant ID with the selected data role and binds only that
grant to a new credential for the existing client UUID. It does not change old
grant records, attach the new grant to old credentials, copy other workspace
grants or revoke unrelated bindings. The UUID maintains identity continuity for
the later entry-ownership layer. Client selection and local credential activation
remain explicit future adapter operations.

Issuance commits the invitation, target, complete history snapshot and audit
atomically. Redemption commits the new certificate, grant, binding, one-use
counter, receipt and audit atomically. Failure rolls back that operation's new
state. In particular, grant or audit failure cannot
leave an orphan credential or consume a slot. Working access in another
workspace remains intact.

## Retry or revoke

The same token, request UUID and verified public key retrieve the same public
certificate and IDs without another grant or slot. Changed key, request UUID or
existing-credential claim conflicts. As with other invitation receipts, a
committed result can be retrieved after deadline or workspace archive if the
issuer's scoped security authority and the exact original grant/binding and
credential remain current. Archived context stays read-only. A stale pending
ticket cannot use that receipt path because it has no committed result.

Grant/binding revocation, client disable, credential expiry or supersession and
invitation revocation cannot be bypassed through retry. Use the existing scoped
`invitation_status` and `revoke_invitation` methods. Status includes the optional
captured `target_client_id`; metadata does not grant access or reserve authority.

[Recovery](workspace-recovery.md) remains separate. It replaces credentials for
still-authorized grants and can work in readable archived workspaces. It never
creates a new membership or restores a revoked grant. Targeted readmission is a
fresh admission into an active workspace.

## Storage and verification limits

Schema 11 adds targeted invitation records and complete historical grant
snapshots with same-client and workspace foreign keys. Schema-10 recovery and
earlier supported records are preserved under the
[foundation upgrade rules](workspace-foundation.md#database-upgrade).

Tests cover current scoped authority and key ceilings, immutable target and
historical snapshots, intervening admission/revocation, known-key rejection,
initial-group transitions, stale generic tokens, lifecycle, global disable,
exact/concurrent retries, rollback and schema-10 upgrade with recovery retained.
Actual localhost TLS tests also cover scoped keys, fresh CSR keys, exact retry,
server-captured target/history and unrelated access. Production deployment and
normal client/MCP and browser flows remain incomplete.
