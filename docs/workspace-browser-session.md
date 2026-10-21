# Use the workspace browser-session API

This unreleased HTTPS API exchanges an existing admin key for an ephemeral browser
session. It supports login, current session metadata and logout, and authenticates
the existing HTTPS admin/content routes under the same current key scope.
Open `/admin` on the HTTPS listener for the desktop panel. It provides manual
key login, reusable `#key` links, workspace/client access and a human context
viewer/editor with revision history. Native-mode administration is separate.

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
sessions; the same still-valid key can log in again. The bookmark page
uses the original reusable key; that key is not made one-use.

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


## Use the desktop panel

Open `https://YOUR_TRUSTED_HOST/admin` after independently trusting the instance
CA. Enter your key. To save a reusable bookmark, choose **Copy reusable login
link** before signing in and save the copied link privately. If clipboard access
is denied, select and copy the displayed private link manually. It contains the
same key as manual login. Opening `/admin#key=...` removes the fragment from the
active URL before exchanging the key; the saved original bookmark remains
reusable while its key is valid. Query-string keys are refused. The panel does
not store keys, drafts or sessions in localStorage/sessionStorage.

The header shows the active key/principal and scope. These labels do not prove
which person is using a shared key. Choose a workspace, then **Clients & access**
or **Context & history**. Instance keys can create a workspace with a new owner
principal and copy its owner key once. They can read workspace grants/context
and disable a listed client instance-wide, but cannot invite or edit through
borrowed workspace roles. Sign in using a workspace key for scoped actions.

Workspace keys can issue individual client invitations and revoke that
workspace’s grants. Creation secrets appear only in that result view; copy them
privately before navigating away. After a lost response to workspace/container
or invitation creation, inspect the current list or API state before repeating;
these actions do not have a browser retry mechanism. Initial group enrollment,
readmission/recovery, ownership transfer and
reported work are not yet exposed in this first panel.
Use their existing APIs where available. Whole-workspace deletion and legacy
conversion remain separate unfinished work.

A confirmed workspace creation displays its owner key before refreshing the
workspace list; a failed list read keeps the key available for copying. If the
session cannot be verified after a confirmed workspace or invitation creation,
the panel locks and explicitly reports that creation succeeded but its secret
is unavailable. Recover a lost owner key through the host operator’s
[local owner-key reset](local-workspace-administration.md); an instance key
cannot reset it remotely. For an invitation, use the reported invitation and
workspace IDs with the authorized
[invitation revocation API](workspace-http-api.md#manage-invitations-and-recover-a-client) before deliberately
creating a replacement. Neither path automatically repeats the creation.

For human context work, select an active shelf and book. Lists use explicit pages;
**Next** means more records exist. Create containers/entries, edit an entry,
append a separate note or inspect recorded revisions. A human admin-created
entry has no registered-client owner; edits preserve an existing client owner
and record the actual admin key. Agent labels are self-reported attribution.
All text displays literally, including markup; the panel runs no entry HTML.
Archived entries expose summaries only. Restore before reading their bodies or
history. Archived workspaces remain readable but cannot receive content writes.
Archived shelves are not included in this first active hierarchy browser.

Unsaved entry edits block navigation. Save or deliberately discard/reload before
leaving; entry edits and separate append notes cannot silently replace each
other. The page asks before closing with unsaved or uncertain work, but browser
closure/crash still loses in-memory drafts. It offers no durable offline queue.

Each entry write captures its UUID, exact request, expected revision and current
session proof. The page never automatically retries a mutation. If a response is
lost or conflicts, the original request remains visible and the editor freezes.
**Check / retry exact write** deliberately repeats those same bytes under the
same session, allowing the server to return an existing receipt. It never
creates a replacement UUID. **Forget local attempt** only discards local tracking;
it does not cancel server execution. Copy its operation ID for investigation.
A confirmed write stays confirmed even if the subsequent view refresh fails.

Every operation refreshes current session metadata. A key/session change,
permission loss or unavailable session locks the data view; only unsaved local
text/original intent is offered for copying. Explicitly discard it before a new
login. The panel never resends it with a new key or CSRF proof. Already-authorized
server transactions may finish after logout. Browser-side view clearing does
not erase text that was already delivered or copied elsewhere.

The desktop shell and scripts are bundled with the server, without external
fonts, scripts or telemetry. Assets are public only on the HTTPS listener;
API data remains authenticated. CSP disallows inline scripts/styles, external
connections, framing and form navigation; headers also disable caching and
referrers. JavaScript and a browser supporting ES modules are required. Desktop
resizing and keyboard navigation are supported; phone-specific parity is deferred.


## Read and edit guidance

Choose **Global guidance** without selecting a workspace, or open a workspace’s
**Guidance** tab to read both sources. The panel shows the loaded revision and
actual publisher key/principal and timestamp. Revision 0 is unpublished. It does
not poll or claim agents read or followed a board. Global guidance wins conflicts
within CCP guidance; host instructions and actual permissions still take priority.

An instance key can edit the global board only from **Global guidance**.
Workspace **Guidance** tabs always show the global board read-only.
Workspace owner/admin keys can
edit their own active workspace board only. Archived workspace guidance stays
readable and cannot be saved, including retries, until reactivation. Global
updates remain available independently. **Save guidance** immediately makes an
accepted revision live; there is no separate Publish step. Empty text explicitly
clears the board after confirmation and still records a revision. The editor
counts UTF-8 bytes and accepts at most 8192, including Unicode text.

Each save freezes its original board, UUID, expected revision, exact text and
session proof. Dirty drafts block navigation. Conflict or unknown responses
preserve the draft and original request, freeze editing, and never retry
automatically. **Compare current guidance** loads a separate current server
snapshot without changing the draft or its base. **Check / retry exact guidance
save** deliberately resends the same request under the same session. **Forget
local guidance attempt** does not cancel execution. Copy/merge your draft,
deliberately discard/reload current guidance, then paste and review before a new
Save. There is no force overwrite, implicit rebase or guidance history/rollback interface.

A receipt confirms the original saved revision; another authorized save may
already have superseded it. The panel reports that acknowledgement separately
from the refreshed current revision. A failed refresh never relabels a confirmed
save as unsaved. Session changes/loss still clear authorized views under the
same draft/intent copying rules. Guidance drafts remain memory-only.


## Workspace settings

Open a workspace’s **Settings** tab. Its owner or an authorized instance key can
**Archive workspace** or **Reactivate workspace**, with explicit confirmation.
A workspace admin key can manage active retention but cannot change workspace
lifecycle. Archived context remains readable; content writes and new admission
stop, and timed cleanup pauses. Reactivation resumes the saved remaining time.
Permanent whole-workspace deletion is not exposed by this panel or its current API.

Retention is manual by default. Current policy metadata is available to scoped
owners/admins and instance keys; only an active workspace’s owner/admin key can
configure it. If the policy is unavailable, the panel does not assume a default,
and keeps independently authorized lifecycle actions available.

To **Enable timed cleanup**, enter a positive whole duration, choose Days, Hours
or Seconds, and check the permanent-deletion confirmation. No duration is filled
in automatically. Existing individual entry archives receive a fresh full window;
new entry archives receive the chosen window. Cleanup removes due entry contents
and revision history while retaining body-free proofs/audit; backup retention is
separate. The policy belongs to the workspace and survives its configuring key’s
expiry or revocation. See the [retention contract](workspace-retention.md).

An enabled policy offers **Disable timed cleanup**. Disable before choosing another
duration; subsequent enable starts fresh windows for existing archives. Archived
workspaces show the policy read-only until reactivation. Instance keys cannot
borrow workspace roles to configure it. Durations/revisions beyond JavaScript’s
exact integer range require the API rather than a rounded browser request.

Retention changes keep a frozen UUID, expected revision, request and session proof.
On conflict or a lost response the original request stays visible, controls freeze,
and **Read current state** loads a separate policy snapshot. **Check / retry exact
policy change** deliberately resends the same request under the same session;
it does not generate a new UUID. Confirmed saved and currently loaded policy
revisions are reported separately. A failed refresh does not undo acknowledgement.

Archive/reactivate endpoints have no operation UUID or exact retry receipt. The
panel sends each captured lifecycle attempt once. If its response is uncertain,
it offers **Read current state** and a copy of the original endpoint/target,
with no retry button. A matching observed state does not prove which request
changed it. **Forget local settings attempt** explicitly discards local tracking;
it does not cancel delayed execution. Keep a copy if needed, then deliberately
reload current settings before another action. Drafts/unknown requests block
navigation. Discard/reload keeps the local draft until the relevant state or
policy is read successfully; lifecycle-only recovery does not require a policy
read. Failed policy reads retain configuration text for copying or a separate
explicit discard. These remain memory-only.


## Manage administrators and keys as the owner

Open **Settings → Manage administrators & keys** using this workspace’s owner
key. Instance and scoped administrator keys cannot read this security inventory.
The roster shows principal UUIDs alongside labels; keys separately show expiry,
revocation and recorded role activity. Role activity does not make an expired
or revoked key valid. Use **Next administrators** and **Next keys** independently;
pages are current observations and can change while you browse.

Choose **Appoint administrator**, then explicitly choose a new principal label
or an existing principal UUID. A matching label never selects an existing
identity. Appointment is available only in active workspaces. Existing roles
can receive an additional key through **Issue key**, including while archived.
Leave optional expiry empty for valid until revoked; a supplied local date/time
is converted to a future Unix timestamp. Creating a new key leaves other keys
valid. Copy the one-time secret privately, verify delivery, then deliberately
revoke the old key if rotating. Secrets cannot be retrieved from the inventory.
See [owner administration](workspace-owner-administration.md) for permissions.

**Revoke administrator** removes that workspace role and revokes all its scoped
keys. **Revoke key** affects just that key. Revoking your current login key ends
its authority; the panel reports the confirmed action and returns through the
session-lock sign-in flow. Have another owner key available. Otherwise the host
operator can use [local owner recovery](local-workspace-administration.md).

These security routes have no operation nonce or safe replay receipt. The panel
sends each action once. On an unconfirmed response, controls freeze and retain
the exact original endpoint and target. Read current security metadata separately;
this is observation, not proof that your request caused a change. There is no
retry button. Forgetting locally does not cancel a delayed server action. After
copying and investigating, explicitly discard the local copy to reload the full
paginated view. If reads remain unavailable, **Discard local security copy**
clears the private copy without claiming cancellation or a successful read.
For a lost key secret, inspect the relevant principal’s keys,
issue a replacement and revoke undelivered keys. Do not repeat new-principal
appointment by label to recover a secret.

Acknowledged key creation is recorded before checking the session again. If that
check fails, the panel clears private server views and reports the confirmed
workspace/principal/key IDs with recovery directions; it does not describe the
creation as failed. The unavailable secret is not included in recovery text.
Drafts and unknown actions remain local memory only; closing the tab loses them.
