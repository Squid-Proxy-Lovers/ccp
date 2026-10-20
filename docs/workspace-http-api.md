# Run the workspace HTTPS API

This unreleased API connects the reviewed workspace identity, key and certificate
foundations to direct TLS listeners. It supports invitations, enrollment,
discovery, existing-client admission and credential renewal. Content endpoints,
complete admin/recovery management, browser login, workspace client profiles,
MCP integration and the human panel remain subsequent work. Keep this draft
implementation separate from a production rollout.

[Documentation index](README.md) · [Instance TLS](instance-tls-identity.md) ·
[Registered peers](registered-tls-peers.md)

## Prepare and start

Complete [local administration setup](local-workspace-administration.md) and
[instance TLS setup](instance-tls-identity.md) first. Keep a complete matching
backup. Choose names that clients will actually verify, including `127.0.0.1`
for the loopback examples below. Export the public CA PEM from the trusted
setup output into `ccp-ca.pem`, and verify its fingerprint through an independent
trusted channel before transmitting an admin or invitation secret.

The first start requires an explicit activation flag:

```bash
ccp-server workspace-serve --data-dir ./ccp-data --activate-workspace
```

Activation persists workspace mode, after which native startup, health and token
tools refuse this directory. It is not an in-place downgrade toggle. Dirty
legacy journals prevent activation. It does not migrate legacy entry ownership
or expose context contents. See the [mode and backup rules](workspace-runtime-mode.md).
Socket bind and TLS validation failures occur before initial activation; later
startup failures can leave the explicitly activated mode persisted.

Subsequent starts resume the existing mode:

```bash
ccp-server workspace-serve --data-dir ./ccp-data
```

Defaults are HTTPS on `127.0.0.1:8443` and client mTLS on `127.0.0.1:8444`.
Override with `--https-listen ADDRESS:PORT` and `--mtls-listen ADDRESS:PORT`.
The initial JSON reports the actual listener addresses, instance UUID and API
version, including when port zero chooses ephemeral ports. Ctrl-C stops accepts
and sessions and reports a connection-failure count without request details.
An already dispatched atomic database job may finish after a connection closes;
it retains the directory lease and concurrency slot until completion.

TLS is required on both listeners. The client listener uses the built-in
mandatory client-certificate verifier. TLS termination with forwarded identity
headers is not supported; a pass-through connection must preserve the actual
client TLS peer. Server-leaf renewal remains the separate local
[renewal command](server-certificate-renewal.md) and requires the runtime to stop.
There is no automatic OS/browser trust installation or server update.

## Listener and authority table

| Listener | Method and path | Authority and result |
| --- | --- | --- |
| HTTPS or client mTLS | `GET /api/v1/health` | Public instance UUID and API version; the client listener still requires its TLS certificate |
| HTTPS | `GET /api/v1/admin/workspaces` | Current scoped admin Bearer key; accessible workspace metadata only |
| HTTPS | `POST /api/v1/admin/invitations/initial` | Current workspace owner/admin key; bounded initial group link |
| HTTPS | `POST /api/v1/admin/invitations/individual` | Current workspace owner/admin key; individual link under existing window/expiry policy |
| HTTPS | `POST /api/v1/enroll` | Invitation secret plus signed CSR; new registration only |
| Client mTLS | `GET /api/v1/workspaces` | Actual registered current peer; accessible workspace metadata only |
| Client mTLS | `POST /api/v1/admissions` | Actual current peer plus invitation/CSR; reuse its registration for a new workspace grant |
| Client mTLS | `POST /api/v1/credentials/renew` | Actual peer proof and signed new CSR; atomic renewal or exact superseded-peer retry |

Instance keys retain only global powers. They can list all readable workspaces
but cannot use a principal's workspace-owner role to issue invitations. Use the
separate owner/admin key for that workspace. Client routes accept only the TLS
identity and refuse an Authorization header. Request bodies and proxy headers
cannot select an actor, client UUID or existing credential UUID.

There are no browser cookies or CORS grants. Supplied Origin must be the same
HTTPS origin as Host. JSON requests from another origin are refused. Admin
keys are supplied in the Authorization header, with no query-key login. The
selected `/admin#key=...` browser workflow is not served by this batch.

## JSON and pagination

`protocol::workspace::API_VERSION` is 1, independent of native binary
`PROTOCOL_VERSION` 1. Successful responses have `api_version` and `data`.
Application failures have `api_version` and `error.code`. Strict request DTOs
reject unknown or duplicate fields; they do not accept identity claims.
Certificate receipts return instance/client/credential UUIDs, public certificate
PEM and expiry. Private client keys remain with the caller. Internal captured
grant IDs remain authoritative but are not an unbounded certificate-response
payload.

Workspace listings return `data.workspaces` and `data.next_cursor`. Default
limit is 100, maximum 500. Pass `limit` and the returned opaque `after` value as
URL-encoded query parameters on listing endpoints. No other queries are
supported. The cursor is a bounded JSON string, not an authority token. Each
page rechecks current permissions; pages are current snapshots rather than a
snapshot spanning multiple calls. Concurrent name changes can change ordering.
Names and UUIDs form a stable order within a page. Client SQL candidates follow
its credential bindings; a workspace key cannot borrow global discovery.
Oversized metadata or responses can refuse service instead of returning a
misleading partial listing. Finer resource fairness remains future work.

The initial invitation body contains `workspace_id`, `role` (`read` or
`read_write`), `max_clients` and `expires_at` (Unix seconds). At least a positive
capacity or future deadline is required. Individual invitations require the
workspace UUID, role and a finite future expiry under the existing policy.
Closing/managing the window beyond these operations is not exposed yet.

Enrollment/admission bodies contain only `invitation`, `request_id` (UUID) and
`csr_pem`. Renewal contains only `request_id` and `csr_pem`. Admission derives
the existing credential from the actual peer; renewal never converts a
superseded peer into an ordinary actor. Preserve the request UUID and CSR key
to retry a lost enrollment/renewal response. Newly granted workspaces are
available for discovery; no routine context selection is made by these APIs.

## Try a protected request

Use the trusted CA file before sending secrets:

```bash
curl --cacert ./ccp-ca.pem https://127.0.0.1:8443/api/v1/health
curl --cacert ./ccp-ca.pem --header @./owner-auth-header.txt \
  https://127.0.0.1:8443/api/v1/admin/workspaces
```

Create `owner-auth-header.txt` privately with one Authorization header using the
issued workspace-owner key. Its format is `Authorization: Bearer OWNER_KEY`;
replace `OWNER_KEY` with the issued secret and restrict the file to its owner.
The command reads the header from that file. A client request also supplies its
enrolled public certificate and local key:

```bash
curl --cacert ./ccp-ca.pem --cert ./client-cert.pem --key ./client-key.pem \
  https://127.0.0.1:8444/api/v1/workspaces
```

The normal client CLI/MCP enrollment/profile flow remains separate integration
work; existing native commands keep their supported native-mode behavior.

## Limits and errors

HTTP/1.1 keepalive and bounded chunked JSON are handled by Hyper. Header limit is
8 KiB and 32 fields; decoded body limit is 64 KiB, with a 10-second body deadline.
TLS handshake deadline is 5 seconds, header deadline 10 seconds, connection
lifetime 120 seconds, and request budget 100 per connection. There are at most
64 admitted connections or outstanding jobs; a disconnected database job keeps
its slot. Responses are capped at 1 MiB. Early data is disabled. Supported
transfer coding is chunked; content compression and trailers are not accepted.
Malformed framing closes the connection under Hyper's HTTP rules, including
closing after a request with both Transfer-Encoding and Content-Length.

Admin authentication failure is 401 with a Bearer challenge. Client-registry,
profile, scope and operation refusals use 403 without internal details. Other
application errors include 400 invalid request, 404 route, 405 method, 408 body
timeout, 413 size, 415 media, 501 transfer coding and 503 unavailable storage or
response capacity. Malformed HTTP may receive Hyper's framing rejection without
the JSON envelope. Responses disable caching; no request bodies, keys, CSRs or
private transport material are logged. Tests exercise actual localhost TLS,
not a production deployment or a throughput guarantee.
