# Verify registered TLS peers

The unreleased workspace server has internal adapters that bind an established
client TLS connection to its current registered identity. The separate
[workspace HTTPS API](workspace-http-api.md) now uses them for every protected
client request. Normal client profiles, complete management and browser
flows remain later work.

[Documentation index](README.md) · [Instance TLS setup](instance-tls-identity.md)

## Ordinary requests

`InstanceTls::authenticate_client` takes a Rustls `ServerConnection` and an
`AccessStore`. It reads the certificate from the established connection's
`peer_certificates()` and refuses incomplete handshakes or absent client
certificates. Request JSON, display names, certificates supplied in a body, and
forwarded identity headers cannot create its private proof type.

The Hyper runtime captures an opaque certificate source from that completed
connection before handing the stream to its HTTP driver. It stays in that
connection's private service context; it cannot be constructed from raw JSON or
headers. Each request revalidates it and the current registry instead of caching
an authenticated actor. Pending database jobs retain the connection admission
slot and runtime lease even if the TCP connection closes.

The certificate is revalidated against this instance's CA and current server
clock, including root expiry. The direct leaf may be followed by the exact
instance root; other chains are refused. The leaf is limited to 16 KiB. Its
profile must have digitalSignature and ClientAuth only, with no CA capability,
and exactly three canonical UUID URI names:

```text
ccp://instance/INSTANCE_UUID
ccp://client/CLIENT_UUID
ccp://credential/CREDENTIAL_UUID
```

These are illustrative identifiers. Actual URIs contain lowercase, hyphenated
UUID strings. Extra, missing, duplicate or other subject alternative names are
refused. The serial number must represent the credential UUID. The common name
and administrative display labels grant no authority. CCP treats certificate
expiry as exclusive, consistent with its credential records.

A read transaction matches the actual leaf to the database instance UUID,
credential/client UUIDs, certificate SHA-256 fingerprint, public-key SHA-256,
exact expiry and stored certificate PEM. The client must be enabled and the
credential unrevoked, unexpired and unsuperseded. Missing certificate registry
rows, mismatches and SQL errors refuse authentication.

Success returns `Actor::Credential`, retaining the credential UUID. It never
returns a principal or admin actor. Success establishes identity only; it does
not imply access to any workspace. Scoped operations still check current bound
grants, revocations and workspace state in their own protected transaction.
Archived workspaces remain readable and refuse content writes. An enabled
client without readable grants can authenticate but discovers no workspaces
and cannot renew or read context.

Authenticate each request, including requests on an already-open or resumed TLS
connection. Do not cache an actor as permission to skip later authentication.
Revoking a credential or disabling a client applies to the next request even
when its TLS handshake succeeded earlier. A grant revocation changes effective
workspace access independently of client identity.

## Renewal and exact retries

`InstanceTls::renew_client` accepts the actual TLS connection and a signed
renewal request, returning only the public renewal receipt. It rechecks the
private peer proof and registry inside the same IMMEDIATE transaction as the
[existing renewal operation](workspace-enrollment.md). The TLS peer certificate
must still be unexpired and unrevoked, with an enabled client and matching
registry. This method does not return an actor.

A superseded credential fails ordinary authentication. Through this separate
method, it can retrieve only the exact committed receipt for the same request
UUID and CSR key. It cannot start a new renewal, discover workspaces, admit a
client, or obtain a general actor. The replacement credential and at least one
captured readable grant must remain valid; later admissions cannot enlarge the
receipt. Revoking either credential, disabling the client, or losing all captured
access refuses retries. Database or audit failure preserves the old working
credential without issuing an orphan.

## Integration limits

The older `AccessStore` methods that accept an `Actor` are trusted building
blocks for server code and embedded callers. They do not authenticate a remote
claim. HTTP routes must derive actors from these TLS adapters and from
the separate scoped admin-key authentication. They must normalize internal
errors, enforce parser/body/concurrency limits and preserve the documented
transaction boundaries. No plaintext or proxy-header identity fallback is
included here. External client enrollment, browser sessions, explicit workspace
selection and certificate renewal management are separate integration work.

Rustls documents that [peer certificates belong to the established or resumed
connection](https://docs.rs/rustls/0.23.46/rustls/struct.CommonState.html#method.peer_certificates).
The adapter requires the completed private-key proof provided by the server's
mandatory-mTLS configuration; possessing a public certificate alone is not an
authentication mechanism.
