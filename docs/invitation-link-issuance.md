# Issue an enrollment link

Unreleased workspace API 1, server schema 27. This endpoint creates a versioned
enrollment link with explicit server-side lifetime and use limits. Panel and
client-command integration are separate work until their end-to-end checks pass.

[Documentation index](README.md) · [Link format](enrollment-links.md) · [Workspace API](workspace-http-api.md)

Send an authenticated admin HTTPS request to:

```text
POST /api/v1/admin/invitations/link
```

The current workspace owner or administrator key must have admission authority
in that workspace. An instance key's global read access does not supply it.
Archived or deleted workspaces cannot issue links. Existing cookie sessions use
the normal Origin/CSRF rules; CLI callers can use the existing Bearer-key flow.

## Default and custom lifetime

The minimal body issues one individual link with a 30-minute deadline measured
from the server's clock:

```json
{"workspace_id":"<workspace-uuid>","role":"read_write"}
```

Optional `admission` and `lifetime` objects default to `{"kind":"individual"}`
and `{"kind":"short"}`. A custom future Unix timestamp is explicit:

```json
{
  "workspace_id":"<workspace-uuid>",
  "role":"read",
  "lifetime":{"kind":"at","expires_at":2000000000,"confirm_long_lived":true}
}
```

The existing finite `/invitations/individual`, readmission and recovery APIs
keep their previous bounds and behavior. The new endpoint supports custom
deadlines without changing those callers.

A dated link more than seven days ahead requires `confirm_long_lived:true`.
The server audits that confirmation. Long-lived groups also require a positive
`max_clients` cap. Exactly seven days does not require the extra confirmation;
custom dates remain supported after confirmation.

## No expiry

**A leaked unused link without a deadline can remain usable until it is revoked.**
Prefer a short lifetime. Issuing one requires an explicit confirmation:

```json
{
  "workspace_id":"<workspace-uuid>",
  "role":"read",
  "lifetime":{"kind":"never","confirm_no_expiry":true}
}
```

The server stores a NULL deadline, rather than a distant sentinel timestamp,
and audits the confirmation in the issuance transaction. Individual links remain
single-use. Revocation, current issuer authority, workspace lifecycle and current
client/grant rules still apply. No expiry does not make a revoked key valid.

For the one initial group window, use
`"admission":{"kind":"initial_group","max_clients":3}`. A non-expiring group
must have a positive cap. Unlimited uses with no deadline is refused. Creating
individual invitations seals an unused initial phase; a closed group cannot
be reopened. The new endpoint does not create a later bulk-enrollment path.

## Routing, response and retries

By default the link uses the request's HTTPS origin and the server's actual bound
mTLS port. Optional `mtls_origin` supplies a complete HTTPS origin when a public
hostname or forwarded port differs, such as `https://clients.example:9444`.
It is validated as metadata, without opening an outbound connection. Clients
must verify its certificate, CA and instance identity before using it.

Optional `https_origin` overrides the admin connection's origin when the admin
uses a local forward or a proxy rewrites Host. It changes link routing metadata,
not authentication or Origin/CSRF checks. For example, use
`https://ccp.example:8443` for the public HTTPS origin and a separate
`mtls_origin` for its publicly reachable mTLS endpoint. The default port is the
actual bound listener port, which may differ from a forwarded public port.

The response returns `invitation_id`, `enrollment_link`, `workspace_id`,
`workspace_name`, `expires_at` and `max_uses`. Instance/workspace IDs and the CA
digest come from the actual server/workspace. Routing and display metadata are
validated before minting a bearer. The secret is shown only in this issuance
response; stored state contains its hash, never the link or plaintext secret.

Responses are no-store. Treat an uncertain issuance response as one-shot: inspect
invitation metadata and explicitly revoke an unwanted invitation before issuing
another. Redemption uses the existing durable request UUID/key/CSR semantics:
an exact committed retry can return its certificate after a deadline or spent
use, while a distinct redemption remains refused. Retrying never restores a
revoked grant or bypasses a disabled client.

Unknown fields, unsupported tagged settings, invalid deadlines, missing no-expiry
confirmation and malformed routing return a fixed refusal without minting a link.
Opening a link in a browser is separate from redeeming it through the client.

## Create a link in the panel

Sign in with the target workspace’s owner/admin key. Open **Clients & access**,
then **Invitations**. Choose read-only or read/write access and a lifetime:
30 minutes by default, a custom date in your local time, or explicitly confirmed
no expiry. Dates beyond seven days require a separate confirmation. The panel asks ten
minutes early, with the same group-cap requirement, to allow for clock differences. Individual
links admit one client. The single initial group can have a use cap; a cap is
required for no-expiry or dated links beyond seven days.

Expand **Advanced · public addresses** when public ports or hostnames differ from
those seen by your browser. Use HTTPS origins covered by the instance certificate;
the client currently requires direct mTLS connectivity.

After creation, copy the private enrollment link once. The recipient runs
`ccp enroll` and pastes it at the hidden prompt. The optional POSIX command copy
puts the secret on argv. Show the value and select it manually if clipboard access
fails. The public `/join` page strips its fragment and shows these instructions;
it does not submit the invitation, install software or change browser trust.
Refreshing the page loses its in-memory link; reopen the original complete link.

An uncertain issuance is never retried automatically. Read invitation metadata
and explicitly resolve the pending action before deliberately issuing another.
A lost initial-group link cannot be recovered by reopening the group window.
