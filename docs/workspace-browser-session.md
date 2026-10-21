# Use the workspace browser-session API

This unreleased HTTPS API exchanges an existing admin key for an ephemeral browser
session. It supports login, current session metadata and logout, and authenticates
the existing HTTPS admin/content routes under the same current key scope.
The actual `/admin` login page, reusable `#key` exchange and desktop workspace
viewer/editor are subsequent frontend work; these endpoints alone do not provide
a working bookmark login screen yet.

[Documentation index](README.md) · [HTTPS setup](workspace-http-api.md) ·
[Admin keys](workspace-admin-keys.md)

## Exchange the existing key over HTTPS

Run the [workspace HTTPS server](workspace-http-api.md#prepare-and-start), trust
its CA independently and use your issued personal or shared scoped admin key.
The same key works with the existing Bearer API. No new user account or client
registration is created. A shared key identifies its key/principal, not a
particular human using it.

POST `/api/v1/admin/session/login` with exactly `{"key":"YOUR_ADMIN_KEY"}`.
Use `Content-Type: application/json`, the matching HTTPS `Origin`, and
`X-CCP-CSRF: login`. This initial custom header is a fixed marker, not the later
session's CSRF proof. Cross-origin requests and unknown identity claims are
rejected. The response sets a session cookie and returns API1 session metadata;
it never returns the admin key or the opaque cookie value in its JSON body.

For local API testing, first create an owner-only `browser-login.json` containing
the request with your issued key. Keep the key out of command arguments and logs.
Use your actual trusted HTTPS origin in both the URL and Origin header:

```bash
umask 077
curl --cacert ./ccp-ca.pem --header 'Origin: https://127.0.0.1:8443' \
  --header 'Content-Type: application/json' --header 'X-CCP-CSRF: login' \
  --data-binary @./browser-login.json --cookie-jar ./browser.cookies \
  https://127.0.0.1:8443/api/v1/admin/session/login
```

The JSON `data` contains `instance_id`, `expires_at`, `csrf_token` and `identity`.
Identity includes current key/principal IDs and labels plus either instance scope
or a workspace UUID with owner/admin role. These are current display facts, not
permission tokens, human identity proof or an authority lease. Session cookies
are not accepted as client mTLS identity.

## Reuse a session and protect writes

GET `/api/v1/admin/session` with the cookie to refresh current identity metadata
and recover that session's CSRF token after a page reload. A same-origin GET need
not send Origin. The response is not cached, sets no replacement cookie and does
not extend session lifetime.

```bash
curl --cacert ./ccp-ca.pem --cookie ./browser.cookies \
  https://127.0.0.1:8443/api/v1/admin/session
```

Cookie-authenticated POST requests, including read APIs that use POST, require the
matching HTTPS Origin and `X-CCP-CSRF` set to the session's returned token. Save
that header privately as `browser-csrf-header.txt` for command-line testing:

```bash
curl --cacert ./ccp-ca.pem --cookie ./browser.cookies \
  --header 'Origin: https://127.0.0.1:8443' --header @./browser-csrf-header.txt \
  --header 'Content-Type: application/json' --data-binary @./retention-target.json \
  https://127.0.0.1:8443/api/v1/admin/content/retention/get
```

The target-file shape is in the [retention guide](workspace-retention.md).
Use a consistent origin, including host and port. A cookie session is bound to
that origin; using another hostname or port requires a new login. CORS is not
enabled. Missing/foreign Origin or CSRF proofs are denied, and proofs from a
different session do not work.

Every request rechecks the key's current expiry, revocation, principal and roles.
Protected operations check authority again in their own transaction. Instance
keys retain global reads and selected instance administration powers; they do
not borrow workspace edit, invitation, retention or board-publication powers.
A revoked/demoted/expired key stops subsequent data access even while the browser
session's eight-hour deadline has not passed.

Existing Bearer requests keep their prior authentication behavior. A request
containing both Authorization and this instance's session cookie is rejected as
ambiguous; unrelated host cookies are ignored. Duplicate relevant cookies or
security headers are rejected. Public enrollment and registered-client mTLS
contracts stay separate.

## Expiry, replacement and logout

Browser sessions have an absolute eight-hour engineering default, checked against
both a monotonic elapsed-time deadline and the server UTC expiry. Either deadline
ends the session; requests do not slide them forward. Underlying admin-key lifetime
is unchanged: valid until revoked/rotated by default, with optional issuer expiry.
An earlier key expiry/revocation can end access sooner. Server restart drops all
sessions; the same still-valid key can log in again. The planned bookmark page
will use the original reusable key rather than make that key one-use.

A successful new login from a browser replaces its prior session after the new
key is validated. Failed key authentication does not invalidate the existing
session. Independent browsers retain independent sessions. Logout affects only
the presented session; it does not revoke the underlying key or other sessions.

POST `/api/v1/admin/session/logout` with `{}`, cookie, Origin and session CSRF
proof. The response clears the cookie with Max-Age 0 and returns
`{"signed_out":true}` in API1 `data`. Logout can clear a valid session whose key
was later revoked, without granting any data access. Unauthorized GET responses
never clear cookies as a side effect. Already dispatched authorized SQL can finish
after logout/expiry; preserve an original mutation intent after an uncertain reply.

## Storage and limits

Cookies use an instance-specific `__Host-ccp_admin_<instance UUID>` name with
Secure, HttpOnly, SameSite=Strict, Path=/ and no Domain. The instance suffix avoids
same-host instance cookie replacement. Cookies are not port-isolated by browsers;
the host operator remains trusted. Server-side origin binding and HTTPS remain
required.

The server retains only a hash of the random 256-bit session token, a key UUID,
a quota partition, origin and expiry. It stores no admin key or role lease.
A process-local HMAC secret derives and verifies per-session CSRF tokens. No
session data is persisted to SQLite or disk. This is not a forensic memory-erasure
claim.

The table is bounded to 1024 sessions, 32 per key and 128 per workspace, with 64
slots reserved for instance-scoped administration. Expired records are pruned.
A full budget returns a sanitized503 without removing a valid old session. Normal
64KiB request/1MiB response limits apply. Invalid shapes/duplicate credentials
return400; missing or invalid session/key returns401; Origin/CSRF failures return403;
internal failures return503. No SQL, secret key or raw request appears in errors.
