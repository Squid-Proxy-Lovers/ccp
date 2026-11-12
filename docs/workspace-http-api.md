# Run the workspace HTTPS API

This unreleased API connects the reviewed workspace identity, key and certificate
foundations to direct TLS listeners. It supports invitations, enrollment,
discovery, existing-client admission, credential renewal and targeted recovery.
Initial [client trust/profiles, enrollment and discovery](workspace-client.md)
are connected, along with client lifecycle, explicit selections and workspace MCP
metadata. The [content HTTP API](workspace-content-http.md) now exposes hierarchy,
entries, revisions and shelf state/archive summaries on both authenticated
listeners, plus shelf transitions on admin HTTPS.
[Guidance reads and scoped live saves](#read-and-save-guidance) and
[explicit timed archive-retention configuration](workspace-retention.md) are also available.
The normal client now
supports [content reads](workspace-content-client.md) and
[entry mutations](workspace-content-mutations.md). The [workspace MCP bridge](workspace-mcp.md)
exposes content reads, shelf/book creation and guarded entry mutations. The
[browser-session API](workspace-browser-session.md) now supports scoped cookie
authentication and the bundled desktop admin panel. A
[client operation resolution fence](workspace-operation-resolution.md) supports
body-free outcome recovery without replaying a write under new credentials. Keep this
draft implementation separate from a production rollout.

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
or expose those legacy context contents. See the [mode and backup rules](workspace-runtime-mode.md).
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
it retains the directory lease and concurrency slot until completion. The
workspace retention worker has one separately tracked blocking-job slot and also
drains on shutdown. Only explicit timed policies make archives eligible; native
mode starts no retention worker.

TLS is required on both listeners. The client listener uses the built-in
mandatory client-certificate verifier. TLS termination with forwarded identity
headers is not supported; a pass-through connection must preserve the actual
client TLS peer. Server-leaf renewal remains the separate local
[renewal command](server-certificate-renewal.md) and requires the runtime to stop.
There is no automatic OS/browser trust installation or server update.

## Optional WebSocket carriers

The server can carry the existing CCP TLS protocol inside WebSockets on its HTTPS
listener, alongside the browser UI:

```bash
ccp-server workspace-serve --data-dir ./ccp-data --websocket-tunnels
```

This is an opt-in server transport. The current released/direct client setup does
not select it automatically; client profile and onboarding support are separate
work. No extra process runs on a connecting client, and the server never dials a
forwarding target. The server's direct listeners remain available.

The outer HTTP/1.1 upgrade uses two exact paths and subprotocol `ccp-tls-v1`:

| Path | Inner connection |
| --- | --- |
| `/ccp/tunnel/v1/enroll` | Server-authenticated CCP TLS; only `GET /api/v1/health`, `GET /api/v1/software-info`, `POST /api/v1/enroll` and `POST /api/v1/recover` |
| `/ccp/tunnel/v1/client` | Mandatory CCP mTLS and current registered-client permissions, as on the direct client listener |

The enrollment carrier cannot serve the admin shell, browser sessions or admin
APIs, even with a valid key or cookie. This preserves an operator's separate edge
Access policy for administration. Only the first HTTP request on a connection can
upgrade. Queries, request bodies, duplicate handshake headers, Origin-bearing
requests and extension offers are refused. The fixed subprotocol must match
exactly. Frames are binary and uncompressed; frame/message size is capped at
64KiB, while larger inner HTTP bodies stream across multiple frames. Delivered
Ping/Pong events and completed empty binary messages share a 128-per-second
limit. Empty non-final fragments are consumed inside the WebSocket parser and
are not counted by this message-level budget. The payload cap and connection
deadline remain enforced. Data reads continue while outgoing writes are
backpressured.

Each tunnel retains one of the existing 64 connection slots. A single 120-second
absolute deadline starts at admission and covers outer TLS, upgrade, inner TLS
and inner HTTP; both TLS handshakes also have five-second limits. The inner HTTP
parser retains its header/body limits and 100-request cap. Shutdown cancels the
tracked connection without promising a TLS or WebSocket closing handshake;
already dispatched database jobs still drain with their lease and slot.

For a TLS-terminating proxy, the outer public certificate and the inner CCP
CA/name/instance binding are separate trust decisions. An independently trusted
invitation must supply the inner anchor; do not learn it from an unverified proxy
response. Hosted browser code and admin keys still trust that proxy. Inner mTLS
protects the machine channel, and does not make proxy-served administration
private from the proxy.

No Cloudflare Access policy, DNS, origin-trust configuration or system trust store
is changed by this option. Public WebSocket/Access/origin behavior needs a real
Cloudflare rehearsal before a hosted setup is considered supported. Preserve
verified origin HTTPS and never substitute forwarded identity headers for the
inner TLS peer.

## Listener and authority table

| Listener | Method and path | Authority and result |
| --- | --- | --- |
| HTTPS or client mTLS | `GET /api/v1/health` | Public instance UUID and API version; the client listener still requires its TLS certificate |
| HTTPS or client mTLS | `GET /api/v1/software-info` | Public instance UUID, package version, native wire and workspace API contracts; no content/admin authority or database access; client listener still requires its TLS certificate |
| HTTPS | `GET /api/v1/admin/workspaces` | Current scoped admin Bearer key; accessible workspace metadata only |
| HTTPS | `POST /api/v1/admin/workspaces/create` or `/first-owner` | Current instance key; atomic workspace/first-owner key provisioning |
| HTTPS | `POST /api/v1/admin/workspaces/inspect` or `/grants` | Current admin read authority in the exact workspace; metadata or paged grant history |
| HTTPS | `POST /api/v1/admin/workspaces/overview` | Same authority as `/inspect`; aggregate-only dashboard counts for 1–90 UTC days (`days`): content totals, daily actions, top eight writers, access counts. No entry content or names |
| Client mTLS | `POST /api/v1/guidance/context` | Current client identity and optional workspace read grant; guidance and delivery-context IDs in one snapshot |
| HTTPS or client mTLS | `POST /api/v1/guidance/get` | Authenticated global guidance plus an optional workspace board under current context-read access |
| HTTPS | `POST /api/v1/admin/guidance/save` | Current global key for global guidance, or scoped workspace owner/admin key for its workspace; immediate revision-bound live save |
| HTTPS or client mTLS | `POST /api/v1/work/status/get` or `/list` | Current workspace read authority; explicit messages stay visible when stale |
| HTTPS | `POST /api/v1/admin/work/status/clear` | Current scoped owner/admin in an active workspace; clear-only CAS, retained client ownership and historical receipt |
| Client mTLS | `POST /api/v1/work/status/mutate` | Current read/write client; owned status set/clear, exact operation receipt; [status contract](workspace-status.md) |
| HTTPS | `POST /api/v1/admin/content/entries/purge` | Current scoped owner/admin key; explicitly confirmed, revision-bound purge of an individually archived entry, including in archived workspaces |
| HTTPS | `POST /api/v1/admin/session/login` | Existing admin key in strict JSON, matching Origin and initial CSRF marker; ephemeral cookie issued |
| HTTPS | `GET /api/v1/admin/session` | Current cookie session and live admin key; metadata and per-session CSRF token |
| HTTPS | `POST /api/v1/admin/session/logout` | Session cookie, matching Origin and CSRF; removes only this session |
| HTTPS | `POST /api/v1/admin/content/retention/get` | Current administrative workspace read access; current policy metadata |
| HTTPS | `POST /api/v1/admin/content/retention/change` | Current scoped workspace owner/admin; explicit confirmed timed enable or manual disable with nonce/revision |
| HTTPS | `POST /api/v1/admin/content/shelves/transition` | Current scoped workspace owner/admin key; exact revision-bound shelf archive/restore |
| HTTPS or client mTLS | `POST /api/v1/content/shelves/state` or `/archived` | Current workspace read authority; body-free state or safe paged archived-shelf summaries |
| HTTPS | `POST /api/v1/admin/workspaces/archive` or `/reactivate` | Current workspace owner or instance key; recoverable lifecycle change |
| HTTPS | `POST /api/v1/admin/grants/revoke` | Current owner/admin key for the grant's workspace; terminal membership revocation |
| HTTPS | `POST /api/v1/admin/clients/disable` | Current instance key; disables a client across the instance |
| HTTPS | `POST /api/v1/admin/team/appoint`, `/revoke` or `/list` | Current owner key; scoped admin appointment/revocation or roster metadata |
| HTTPS | `POST /api/v1/admin/keys/issue`, `/revoke` or `/list` | Current owner key; scoped key management or metadata |
| HTTPS | `POST /api/v1/admin/workspaces/transfer-owner` | Current owner key; transfer to an eligible provisioned scoped admin |
| HTTPS | `POST /api/v1/admin/invitations/initial` | Current workspace owner/admin key; bounded initial group link |
| HTTPS | `POST /api/v1/admin/invitations/individual` | Current workspace owner/admin key; individual link under existing window/expiry policy |
| HTTPS | `POST /api/v1/admin/invitations/link` | Current workspace owner/admin admission authority; [versioned link](invitation-link-issuance.md) with finite default/custom lifetime or explicitly confirmed, bounded no-expiry policy |
| HTTPS | `POST /api/v1/admin/enrollment/status` or `/close` | Scoped security authority; current initial-phase metadata or terminal closure |
| HTTPS | `POST /api/v1/admin/invitations/list`, `/status` or `/revoke` | Scoped security authority; paged metadata, individual status or revocation |
| HTTPS | `POST /api/v1/admin/invitations/readmission` | Current workspace owner/admin key; fresh targeted admission after revoked membership |
| HTTPS | `POST /api/v1/admin/recovery/issue` | Current global or scoped recovery authority; captures still-authorized grants |
| HTTPS | `POST /api/v1/admin/recovery/list`, `/status` or `/revoke` | Matching current global or scoped recovery authority; paged client/scope metadata or exact invitation operations |
| HTTPS | `POST /api/v1/enroll` | Invitation secret plus signed CSR; new registration or explicit targeted readmission |
| HTTPS | `POST /api/v1/recover` | Fresh targeted recovery secret plus signed CSR; no old certificate required |
| Client mTLS | `GET /api/v1/workspaces` | Actual registered current peer; accessible workspace metadata only |
| Client mTLS | `POST /api/v1/admissions` | Actual current peer plus invitation/CSR; reuse its registration for a new workspace grant |
| Client mTLS | `POST /api/v1/credentials/renew` | Actual peer proof and signed new CSR; atomic renewal or exact superseded-peer retry |

Instance keys retain only global powers. They can list all readable workspaces
but cannot use a principal's workspace-owner role to issue invitations. Use the
separate owner/admin key for that workspace. Client routes accept only the TLS
identity and refuse an Authorization header. Request bodies and proxy headers
cannot select an actor, client UUID or existing credential UUID.

CORS is not enabled. Supplied Origin must match the HTTPS origin in Host.
Bearer admin requests retain their existing behavior; the
[browser-session API](workspace-browser-session.md) adds bounded cookie sessions
with matching Origin and per-session CSRF on POSTs. Mixed credentials are refused.
There is no query-key login. The HTTPS [desktop panel](workspace-browser-session.md)
serves `/admin` and exchanges a reusable `#key` fragment in the browser.

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

### Inspect invitation inventory

POST `/api/v1/admin/invitations/list` with a workspace owner/admin key or its
current browser session. It uses the same scoped security authority as inspecting
one invitation; instance keys, client credentials and foreign workspace keys
cannot list it. Metadata remains accessible while archived. Example request:

```json
{"workspace_id":"WORKSPACE_UUID","after":null,"limit":100}
```

Replace the illustrative workspace UUID. The strict body rejects unknown and
duplicate fields. `limit` defaults to 100 and must be 1–500. Pass the returned
`data.next_cursor` UUID as `after` for another page; `null` means no next page.
The response contains `data.invitations` and `data.next_cursor`. Each item has:

- `kind`: `initial_group` or `individual`;
- `role`: the requested `read` or `read_write` grant role;
- `status`: the same metadata returned by `/invitations/status`, with
  `invitation_id`, `uses`, nullable `max_uses`/`expires_at`, `revoked`, `expired`,
  `initial_closed`, `issuer_authorized` and nullable `target_client_id`.

`issuer_authorized` is a current check, including workspace state and issuer
key/role authority. It becomes false while archived without implying permanent
invitation revocation. The desktop [invitation panel](workspace-browser-session.md#inspect-and-manage-invitations)
shows these states separately.

Readmission invitations have kind `individual` and their explicit client target.
No secret, verifier, certificate, issuer identity or credential is returned.
Secrets remain creation-only. The list can help investigate a lost creation
response or find an invitation to revoke, but is not a causal operation receipt
and cannot recover its secret. Do not assume a newly seen row proves which
request created it. Requested roles do not grant access until admission.

Pages are ordered by invitation UUID and are separate current snapshots, each
with current caller authorization and status checks. Cursors confer no access;
concurrent inserts can sort before an earlier cursor. Historical revoked, spent
and expired invitations remain visible. A list is not a redemption permission
lease; redemption rechecks all current workspace, issuer, target and credential
conditions. `initial_closed` describes that row’s initial-group closure, not an
overall enrollment-window state. An empty list does not establish eligibility to
open a new initial group; a window can have been closed without an invitation.

Like individual status inspection, listing persists observed expiry/initial-group
closure under the existing terminal rules, so moving the clock backward does not
reopen observed invitations. A page’s authorization, metadata and closure updates
commit together; failures roll them back. Only requested rows get individual
expiry observations, excluding the next-page lookahead; the existing workspace
initial-window refresh still applies when inspecting a returned row. Listing does
not create invitations, admit clients, reopen a window or run entry cleanup.

### Read the initial enrollment window

POST `/api/v1/admin/enrollment/status` with only `workspace_id`, using the same
current workspace owner/admin security authority as invitation inspection. It
also works while archived; instance keys, foreign scopes and client credentials
cannot use it. The response contains:

- `workspace_id`, `phase` (`unused`, `open` or `closed`) and nullable `closed_at`;
- nullable `invitation`, using the existing group invitation status fields;
- `has_admission_history`, including revoked grants;
- `workspace_archived` and current `can_open`.

This reads the persisted window and grant history atomically, refreshing existing
terminal deadline/capacity closure rules. A failed refresh rolls back the complete
observation. Moving the clock backward cannot reopen an observed closure. A window
closed before issuing a group link has phase `closed` with no invitation. A missing
window row can be `unused` while admission history still makes opening impossible.
Do not infer eligibility from an empty invitation inventory. `can_open` is only a
current observation, never a lease: issuing the initial invitation rechecks all
permissions, workspace state and persisted first-use conditions transactionally.
No secret or verifier is returned, and this endpoint cannot reopen a group.

The initial invitation body contains `workspace_id`, `role` (`read` or
`read_write`), `max_clients` and `expires_at` (Unix seconds). At least a positive
capacity or future deadline is required. Individual invitations require the
workspace UUID, role and a finite future expiry under the existing policy.
Closing the window is terminal. Individual and readmission deadlines must be
within seven days; an active group must first be explicitly closed.

Enrollment/admission bodies contain only `invitation`, `request_id` (UUID) and
`csr_pem`. Renewal contains only `request_id` and `csr_pem`. Admission derives
the existing credential from the actual peer; renewal never converts a
superseded peer into an ordinary actor. Preserve the request UUID and CSR key
to retry a lost enrollment/renewal response. Newly granted workspaces are
available for discovery; no routine context selection is made by these APIs.

## Inspect recovery invitations for one client and scope

POST `/api/v1/admin/recovery/list` with `client_id`, explicit `scope`, optional
`after` UUID and optional `limit` (default100, range1–500). The strict body rejects
unknown/duplicate fields. Scope is `{"kind":"instance"}` or
`{"kind":"workspace","workspace_id":"WORKSPACE_UUID"}`; replace the illustrative UUID.
Current authority matches recovery issuance/status in that exact scope. Instance
keys cannot borrow workspace recovery authority, and workspace keys cannot request
instance or foreign-workspace records. Client credentials cannot use this route.

The result contains `invitations` and nullable `next_cursor`. Each row has
`invitation_id` and the existing recovery `status`: client UUID, scope, deadline,
revoked/expired/redeemed flags and current issuer authority. No secret/verifier,
issuer identity, captured grant/workspace IDs, CSR, credential or certificate is
returned. Instance listing selects only instance-scoped invitations, not a union
of workspace-scoped history. Workspace listing selects only its exact workspace.
An unknown client UUID yields an empty page without disclosing other scopes.

Pages are ordered by invitation UUID. Authorization, bounded candidates and existing
terminal expiry observations commit atomically; a failure rolls back the whole page.
The lookahead row is not observed. Pages are current snapshots, not a full-history
snapshot or lease; concurrent inserts can sort before an earlier cursor. A newly
seen row is not a causal receipt for an uncertain creation. Secrets cannot be
retrieved. Security history remains manageable after client disable/revocation and
while archived under the current matching authority, without granting fresh recovery.

Issuer authority describes the issuer, not full current client/grant recoverability.
Invitation expiry/revocation does not describe already-issued credential expiry or
membership. Revoking a recovery invitation blocks redemption and public receipt
retrieval; it does not revoke an issued credential or grant. See the
[recovery contract](workspace-recovery.md).

## Manage workspaces and client access

Use authenticated HTTPS POST with an admin Bearer key and JSON. These endpoints
use current server permissions; a global key's read access does not give it
workspace grant-revocation, admission or team-management powers.

| Action suffix under `/api/v1/admin/` | JSON fields | Result |
| --- | --- | --- |
| `workspaces/create` | `name`, `owner`, optional `key_expires_at` | New workspace and one-time first owner key |
| `workspaces/first-owner` | `workspace_id`, `owner`, optional `key_expires_at` | Appoints an owner only when none is assigned; one-time first key |
| `workspaces/inspect` | `workspace_id` | `workspace` metadata and optional `owner_id` |
| `workspaces/grants` | `workspace_id`, optional `after`, optional `limit` | `grants` and optional `next_cursor` |
| `workspaces/archive`, `workspaces/reactivate` | `workspace_id` | `completed: true` |
| `grants/revoke` | `grant_id` | `completed: true` |
| `clients/disable` | `client_id` | `completed: true` |

The owner selector must be exactly `{"kind":"existing","principal_id":"UUID"}`
for an enabled existing principal, or
`{"kind":"new_principal","name":"OWNER_LABEL"}` for a fresh principal.
Replace illustrative values with the intended target. Names never claim an
existing identity. Omitted or null key expiry means valid until revoked;
a supplied expiry is a future Unix timestamp. Creation and first appointment
require an instance key and atomically return `workspace_id`, `owner_id`,
`owner_key_id` and `owner_key`. Deliver the secret privately to that owner.
An instance key cannot repeatedly retrieve the secret or replace an assigned
owner through this endpoint. See [first-owner provisioning](initial-workspace-owners.md).

There is no automatic retry guarantee for workspace creation. After a lost
response, inspect the workspace listing and its owner metadata before making
another request. Repeating first-owner appointment after success is denied.
Neither inspect nor grants returns admin secrets. A lost owner key requires
another working owner key or the explicit [physical-host reset](local-workspace-administration.md).
SQL conflicts, including a duplicate name, use the current generic 503 storage
error; inspect state before retrying a failed creation request.

Grant history includes each retained grant's `id`, `client_id`, `client_name`,
`role`, `revoked_at` and current `client_disabled`. It exposes no certificate,
credential ID, secret or membership in another workspace. Instance keys may
read any nonpurged workspace under global read authority; owner/admin keys
remain scoped. Data-client credentials cannot call these administrative reads.
Pagination defaults to 100, maximum 500, ordered by grant UUID. Pass the returned
UUID cursor as `after` in the next POST body. Each page is a current snapshot;
a cursor conveys no permissions, and concurrent new grants can sort before a
previous cursor. Revoked and disabled records remain visible to authorized
administrators, including while the workspace is archived.

Only the owner or global admin can archive/reactivate. Archiving preserves
memberships and data while freezing new admission and content writes; authorized
security administration remains available. Repeating an already applied
archive/reactivate transition is denied. Permanent deletion is not exposed by
this batch. Grant revocation derives its workspace from the stored grant and
requires that workspace's owner/admin key. It preserves history and other
workspace access. Global client disable instead blocks the client's identity
across the instance, including renewal and recovery, on the next protected
request over an existing connection. It retains stored grants and history.
There is no enable-client endpoint in this batch.

Owner-only team/key administration and transfer are documented in the
[owner administration guide](workspace-owner-administration.md). Global read does
not expose its roster/key inventory. Transfer requires an active workspace and
an enabled scoped admin with a live key record; former-owner workspace keys are
terminally revoked. Archived workspaces retain security key rotation/revocation
for existing roles, while team expansion and transfer stop.

## Manage invitations and recover a client

All management actions below use authenticated HTTPS POST with a JSON body and
an admin Bearer key. Status is POST because observing expiry can persist a
terminal closure. Successful close/revoke returns `data.completed: true`.
Revocation requires a currently unrevoked record; repeated close is idempotent.
Only listing endpoints accept query parameters.

| Action suffix under `/api/v1/admin/` | JSON fields |
| --- | --- |
| `enrollment/close` | `workspace_id` |
| `invitations/status`, `invitations/revoke` | `workspace_id`, `invitation_id` |
| `invitations/readmission` | `workspace_id`, `client_id`, `role`, `expires_at` |
| `recovery/issue` | `client_id`, `scope`, `expires_at` |
| `recovery/status`, `recovery/revoke` | `invitation_id` |

UUIDs are strings. Recovery `scope` must be exactly one of the following objects;
replace the illustrative workspace UUID with the intended target:

```json
{"kind":"instance"}
```

```json
{"kind":"workspace","workspace_id":"00000000-0000-4000-8000-000000000001"}
```

An instance key can issue, inspect or revoke instance recovery. An owner/admin
key can perform these operations only for its workspace. Neither key borrows its
principal's other roles. Invitation metadata, closure and revocation require
that workspace's current owner/admin security authority. Status returns no
secret or verifier. Invitation status includes uses, capacity, expiry, revoked,
expired, initial-window closure, current issuer authority and optional captured
target client UUID. Recovery status includes client UUID, exact scope, expiry,
revoked, expired, redeemed and current issuer authority. These observations do
not reserve authority for a later request.

[Recovery](workspace-recovery.md) replaces credentials for the captured client's
still-authorized grants. It never creates membership or restores revoked grants.
It requires a future deadline within seven days. Its fresh bearer secret is
returned once at issuance; retain it privately until delivered to the intended
client. Issue a new key and signed CSR locally, then POST exactly `invitation`,
`request_id` and `csr_pem` to `/api/v1/recover` on HTTPS. Do not send an
Authorization header, client UUID, grant list or old credential claim. A key
previously registered to that client is rejected. Recovery works with readable
archived workspaces and preserves unrelated bindings. Instance recovery excludes
grants revoked or workspaces purged since issuance and never adds later grants.

[Readmission](workspace-readmission.md) creates a new grant for an existing
client whose entire grant history in an active workspace is revoked. Its target
and complete prior history are server records. Redeem its fresh individual
invitation through `/api/v1/enroll` with a new client key, using the same three
fields. `/api/v1/recover` does not accept a readmission token. An intervening
admission permanently invalidates a pending readmission ticket, even if that
new grant is later revoked.

Preserve the exact request UUID and CSR key after a lost response. An exact
retry returns the committed public certificate receipt and no captured grant
list. Retry still checks current issuer/key, client disable, invitation
revocation, credential state and original access. Revoking a consumed invitation
blocks receipt retrieval but does not revoke an already issued certificate or
grant. Closing initial enrollment likewise preserves existing memberships.
Physical-host admin-key reset remains a local command, without an HTTP route.
The [workspace client](workspace-client.md#recover-into-a-separate-profile)
supports recovery into a separate profile. Workspace MCP recovery is available
through the MCP bridge. Browser recovery workflows are available in the bundled
desktop admin panel.

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

The [workspace client](workspace-client.md) connects trust, private profiles,
initial enrollment/readmission, discovery, same-registration joining, renewal and
separate-profile recovery. Explicit [caller selection](workspace-selection.md)
resolves current target metadata, including through the [workspace MCP bridge](workspace-mcp.md).
Content HTTP and normal-client [content reads](workspace-content-client.md) are
available, along with [entry mutations](workspace-content-mutations.md).
MCP content reads, shelf/book creation and guarded entry mutations are available. Native
commands keep their supported native-mode behavior.

## Read and save guidance

Use the trusted CA and protected admin header from [the request example](#try-a-protected-request).
A registered client can read guidance over mTLS with its current certificate;
publishing uses an admin Bearer key over HTTPS. Global keys publish the global
board. Workspace owner/admin keys publish their own workspace board; a global
key's read-all authority does not grant workspace publishing.

First POST to `/api/v1/guidance/get`. For global guidance alone, send `{}` or
`{"workspace_id":null}`. To include one authorized workspace, send:

```json
{"workspace_id":"11111111-1111-4111-8111-111111111111"}
```

The API 1 success envelope contains `data.precedence: "global_first"`, a `global`
board and an optional `workspace` board. Each board has `scope`, `revision`,
`text` and `publisher`. Scope is `{"kind":"global"}` or
`{"kind":"workspace","workspace_id":"..."}`. Publisher records `actor_kind`
(`key` or `admin`), `actor_id`, `principal_id` and `recorded_at` in Unix seconds.
An unpublished board has revision 0, empty text and null publisher. Global and
workspace sources share one authorized database snapshot. Board prose cannot
change their source labels or grant permissions; host instructions and actual
server permissions remain authoritative.

To save, retain a fresh operation UUID and POST the complete request to
`/api/v1/admin/guidance/save` with the appropriate scoped key. Read the current
board first and use its revision; 0 creates its first publication. This example
saves workspace guidance:

```json
{
  "scope":{"kind":"workspace","workspace_id":"11111111-1111-4111-8111-111111111111"},
  "operation_id":"22222222-2222-4222-8222-222222222222",
  "expected_revision":0,
  "text":"Keep workspace notes concise."
}
```

For a global save, set `scope` to `{"kind":"global"}` and use a global key.
Requests reject unknown fields, including claimed publisher identities and
workspace fields inside global scope. Text preserves exact controls and is
limited to 8192 UTF-8 bytes. The existing 64 KiB encoded request and 1 MiB
response limits apply; two fully escaped boards fit the response budget.

A successful new operation immediately makes the new revision live. Identical
text with a new operation records another save. Empty text explicitly clears
the board and retains a positive revision and publisher. The response is a
body-free historical receipt:

```json
{
  "api_version":1,
  "data":{
    "scope":{"kind":"workspace","workspace_id":"11111111-1111-4111-8111-111111111111"},
    "operation_id":"22222222-2222-4222-8222-222222222222",
    "revision":1
  }
}
```

If the response is lost, retry the exact request with its original key and
operation UUID while current publishing authority remains valid. This returns
the original receipt without reapplying. A changed request or publisher under
the same board and operation conflicts; board operation namespaces are separate.
A stale revision returns 409 `content_conflict`; reread and decide how to resolve
it before saving with a fresh operation. Failed saves leave live guidance intact.
Archived workspaces reject workspace saves and retries while preserving reads;
global updates remain readable alongside that preserved workspace guidance.

Current grant revocation and client disable block unauthorized reads. Storage
failures, including a missing published revision, return sanitized 503 without
substituting an empty board. This explicit read API adds no guidance freshness
gate to other operations. The normal client now supports
[explicit guidance reads](workspace-client.md#read-current-guidance). The MCP adapter returns
[explicit and automatic guidance](workspace-mcp.md#read-automatic-guidance-alongside-results),
including unavailable warnings beside successful primary results. The
[desktop guidance editors](workspace-browser-session.md#read-and-edit-guidance)
use the same live-save, scope and retry rules. See the
[storage and retry contract](workspace-foundation.md#guidance-publication-foundation).

## Retrieve an authenticated guidance context

Clients preparing guidance delivery can POST the same optional `workspace_id`
request to `/api/v1/guidance/context` on the client mTLS listener. Admin HTTPS
returns 404 for this endpoint; Bearer headers on mTLS remain rejected. Caller
claims such as client, credential or grant IDs are not accepted in the request.
The ordinary `guidance/get` and scoped admin Save routes are unchanged.

The response includes the current instance, client, credential and optional
workspace grant IDs alongside guidance, all from one authorized database snapshot.
The format is versioned separately from API 1:

```json
{
  "api_version":1,
  "data":{
    "format":1,
    "context":{
      "instance_id":"11111111-1111-4111-8111-111111111111",
      "client_id":"22222222-2222-4222-8222-222222222222",
      "credential_id":"33333333-3333-4333-8333-333333333333",
      "workspace_grant_id":null
    },
    "guidance":{
      "precedence":"global_first",
      "global":{"scope":{"kind":"global"},"revision":0,"text":"","publisher":null},
      "workspace":null
    }
  }
}
```

This example is global-only, so its grant ID and workspace board are null. An
explicit workspace requires current context-read authority and returns its exact
bound grant ID. Revocation, binding revocation, expiry, supersession and disable
remain enforced. Archived workspace guidance remains readable. Re-admission may
retain the credential UUID while replacing a revoked grant UUID; that changes the
context even when board text and revisions stay unchanged.

These identifiers support delivery bookkeeping. They are not access
tokens or permission leases, and do not identify a human or MCP conversation.
They do not prove that a model received or obeyed guidance. Emission
tracking must also isolate each caller and recheck current server access.
This endpoint records no delivery cursor and adds no requirement to acknowledge guidance before an operation. Existing 64 KiB
request and 1 MiB response limits apply. See the
[opt-in client output](workspace-client.md#retrieve-a-guidance-delivery-context).

## Limits and errors

HTTP/1.1 keepalive and bounded chunked JSON are handled by Hyper. Header limit is
8 KiB and 32 fields; body limit is 64 KiB, with a 10-second body deadline. The
exact content entry-mutation route accepts up to 2 MiB of encoded JSON.
TLS handshake deadline is 5 seconds, header deadline 10 seconds, connection
lifetime 120 seconds, and request budget 100 per connection. There are at most
64 admitted connections or outstanding jobs; a disconnected database job keeps
its slot. Responses are capped at 1 MiB, except content snapshots at 4 MiB
(full body plus original append contribution with JSON escaping). Content list
limits are 1–32 to fit encoded metadata within the usual response cap.
Early data is disabled. Supported
transfer coding is chunked; content compression and trailers are not accepted.
Malformed framing closes the connection under Hyper's HTTP rules, including
closing after a request with both Transfer-Encoding and Content-Length.

Admin authentication failure is 401 with a Bearer challenge. Client-registry,
profile, scope and operation refusals use 403 without internal details. Other
application errors include 400 invalid request, 404 route/target, 405 method,
409 content conflict/cancelled/conversion required, 408 body
timeout, 413 size, 415 media, 501 transfer coding and 503 unavailable storage or
response capacity. Malformed HTTP may receive Hyper's framing rejection without
the JSON envelope. Responses disable caching; no request bodies, keys, CSRs or
private transport material are logged. Tests exercise actual localhost TLS,
not a production deployment or a throughput guarantee.

## Permanently delete an archived workspace

The [confirmed deletion API](workspace-deletion.md) provides separate HTTPS admin
preview, start and body-free status routes. Only its owner or an instance admin
may start deletion of an already archived workspace. A start receipt confirms
the terminal access fence; cleanup completion is a separate observed fact.
