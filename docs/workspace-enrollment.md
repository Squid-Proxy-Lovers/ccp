# Atomic workspace certificate enrollment

This unreleased internal batch redeems workspace invitations using signed
certificate requests. It creates a registered client identity, credential,
workspace grant, certificate binding, quota update, retry receipt and audit in
one SQLite write transaction. It still exposes no network listener, CLI command
or browser enrollment flow; those adapters need verified server trust and must
use the same transaction and identity checks.

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
renew themselves through this ordinary identity-reuse path. Renewal and scoped
recovery are not implemented by this batch.

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
These records are required inputs to the future mTLS adapter; trusting a JSON
identity field or a certificate signature alone is insufficient. The
[foundation migration and backup rules](workspace-foundation.md) apply.

## Verification scope

Tests verify the real signed certificate against its CA, ignore hostile CSR
names/usages, reject invalid signatures and identity claims, preserve registration
on second-workspace admission, roll back failed audits, enforce quota under
concurrent admission, return one result under concurrent retry, retain receipts
across database reopening, and enforce current lifecycle/revocation/expiry.
Native CLI/MCP compatibility remains separately tested. The new HTTPS and mTLS
network flows, server-trust bootstrap, renewal/recovery and browser UI remain
unverified until their adapters are implemented.
