# Atomic workspace certificate enrollment

This unreleased internal batch redeems workspace invitations using signed
certificate requests. It creates a registered client identity, credential,
workspace grant, certificate binding, quota update, retry receipt and audit in
one SQLite write transaction. The limited [workspace HTTPS API](workspace-http-api.md)
now exposes new enrollment, existing-client admission and renewal using these
checks. Normal client profiles, automated enrollment and browser flows remain
later integration work.

## New admission and retries

A request supplies the invitation secret, a stable request UUID and a signed
PKCS#10 CSR. Signature verification proves possession of the corresponding key;
a claimed client name or UUID alone grants nothing. Client private keys stay on
the client. The issuer's current administrative role/key, workspace state,
invitation revocation, deadline, initial window and capacity are checked under
the same write lock as admission.

Each successful workspace admission consumes one slot. The final slot closes an
initial group immediately. Expiry observed during a denied request is committed
as a closure without creating any identity or consuming a slot. Group and
individual expiry cannot be undone by a later clock rollback. Closing a window
never resets its deadline or count.

Retries with the same invitation, request UUID and verified public key return
the identical committed certificate and IDs. Changing the public key or original
identity claim conflicts rather than creating another registration. A receipt
can be replayed after the invitation's deadline or group closure because it
returns an existing result, not another admission. It still requires a
nonrevoked invitation, a currently authorized scoped issuer, a live enabled
credential and the original current grant/binding. Revocation, expiry or global
disable cannot be bypassed through receipt replay. Archived workspaces permit
such existing-result retrieval but reject fresh admission.

Certificate, quota, receipt or audit failure rolls back the admission. A signed
certificate not committed to the credential registry is not an authorized
credential. Real client transport must verify both TLS possession and the
current registry on every applicable request.

## Reusing an existing registration

An optional existing-credential UUID is only a claim. The unauthenticated new-
admission method rejects it. Existing admission requires an authenticated mTLS
`Actor::Credential`, its live nonrevoked credential, an enabled client and a CSR
signed by the exact public key stored for that credential. The adapter must
verify private-key possession and the certificate registry before constructing
that actor; a public signed CSR alone cannot select an existing identity. Successful admission adds the newly authorized
workspace grant and keeps the same client identity, credential, certificate and
expiry. It does not extend certificate lifetime or add unrelated workspaces.
Workspace selection remains a separate explicit client operation.

If the client already has a grant history in that workspace, ordinary admission
does not rewrite its role, attach a different credential or resurrect a revoked
grant through an old invitation. Fresh targeted administrative readmission is
separate from this ordinary path. Expired
credentials and lost keys need the separate targeted-recovery flow; they cannot
renew themselves through this ordinary identity-reuse path. Routine renewal is
documented below. Targeted scoped/global recovery uses a
[fresh administrative recovery invitation](workspace-recovery.md). Explicit
targeted readmission of revoked members uses a
[fresh targeted individual invitation](workspace-readmission.md).

## Routine credential renewal

`renew_credential` is an internal transaction primitive. Its actor must come
from an actual verified client TLS connection, never a credential ID in JSON or
a public signed CSR. A live, certified, enabled credential can submit a new
signed CSR and a stable request UUID. The new key may differ from the old key;
the authenticated old connection authorizes that rotation. Clients retain the
new private key and request UUID until the result has been stored locally.

Renewal keeps the registered client ID and captures only currently readable
grants already bound to the presented credential. Other credentials' grants,
revoked grants and purged workspaces are excluded. Archived readable workspaces
can renew; their writes remain frozen. Grant roles stay in the database and
are rechecked for every operation, so a renewed certificate does not preserve
an obsolete write role.

Certificate, bindings, supersession receipt and audit commit atomically. Failure
leaves the old credential usable. Success ends its ordinary data, discovery and
admission authority. Old binding history is retained, and other credentials
belonging to the client are unaffected.

An exact retry returns the same public certificate, IDs and captured grant IDs
without another rotation. Request UUID or key changes conflict. The old
credential must still be unexpired and not administratively revoked; the new
credential must remain live, enabled, unsuperseded and bound to at least one
currently readable captured workspace. Later admissions do not change the receipt
or keep it usable after all captured access is revoked. A revoked captured grant
never regains access,
even if its ID remains in the original public receipt. Global disable and the
revocation of all readable bindings deny retry. Further renewal of the new
credential ends retrieval through the previous receipt.

Schema 9 records supersession separately from administrative revocation. Future
network integration needs a dedicated retry-only authorization path for a
verified superseded peer; it must never admit that peer to ordinary operations.
No such network endpoint or automatic client renewal scheduler exists yet.
An already expired credential requires
[fresh targeted administrative recovery](workspace-recovery.md).

## Certificate identity and trust

The issuing CA must be a CA, its private key must match its certificate, and
any declared key usage must permit signing. It must be valid at issuance.
Client certificates live for up to 30 days, bounded by the CA's validity.

CSR and issuing-CA inputs each contain exactly one correctly labelled PEM block;
only ASCII whitespace is allowed outside it. Each PEM is decoded once, and
signature/metadata checks consume the same complete DER bytes. The issued leaf
public-key hash must match the verified request and is the value stored in the
credential registry.

Caller-requested names, SANs, usages and privileges are discarded. The leaf has
ClientAuth/DigitalSignature only and server-assigned instance, client and
credential UUID URI identities. It contains no permanent workspace role.
Authorization continues to use current server-side grants and bindings.

Schema 8 stores the CSR public-key hash, certificate DER fingerprint and public
certificate against the credential, plus an immutable invitation retry receipt.
The [registered-peer adapter](registered-tls-peers.md) checks these records
against the actual completed TLS peer on every request; a JSON identity field
or certificate signature alone is insufficient. The
[foundation migration and backup rules](workspace-foundation.md) apply.

## Verification scope

Tests verify the real signed certificate against its CA, ignore hostile CSR
names/usages, reject invalid signatures and identity claims, preserve registration
on second-workspace admission, roll back failed audits, enforce quota under
concurrent admission, return one result under concurrent retry, retain receipts
across database reopening, and enforce current lifecycle/revocation/expiry.
Native CLI/MCP compatibility remains separately tested. Actual localhost HTTPS/mTLS enrollment, admission, discovery and renewal flows
are tested by the limited API batch. Normal client-profile bootstrap, automated
renewal, complete recovery management and browser UI remain later work.

Renewal tests cover current bound-scope capture, changed roles, archived reads,
revocation/disable/expiry, exact retry, competing and identical concurrent
rotations, audit/CA failure rollback, schema-8 upgrade and database reopening.

Recovery can revoke a credential's binding in one workspace while retaining its
other bindings. Both ordinary data authorization and renewal exclude revoked
bindings. Returning an old public enrollment or renewal receipt cannot restore
them. See [Client recovery](workspace-recovery.md) for the replacement contract.
