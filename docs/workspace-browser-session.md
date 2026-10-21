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
these actions do not have a browser retry mechanism. Reported work has its own
[status view](workspace-status.md#inspect-reported-work-in-the-desktop-panel), with
explicit clear controls for scoped owners/admins in active workspaces. Unknown
clear outcomes retain their original UUID/revision for deliberate exact retry.
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
Use **Browse archived shelves** for separately paged, body-free summaries.
Select **Inspect shelf archive state** from an active shelf, or inspect an archived
shelf summary. Workspace owners/admins can archive or restore shelves while the
workspace is active. Archiving hides descendants without deleting them; restoration
preserves individual entry archives. Same-name collisions are refused. Global keys
can inspect this metadata but cannot perform shelf transitions.

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


## Transfer workspace ownership

Using the current workspace owner key, open **Settings → Manage administrators &
keys**. In an active workspace, choose **Transfer ownership** on the intended
enabled administrator’s row. Appoint them first if necessary. Verify privately
that the recipient can use their workspace key. The server checks for a current
scoped key record; that record alone does not prove delivery.

The form names the workspace, former owner and exact recipient principal UUID.
Confirm delivery/access with the initially unchecked checkbox, enter the new
owner’s principal ID and review the final confirmation. A successful transfer
revokes every former-owner key in this workspace, including your current login.
The panel immediately locks with confirmed workspace/new-owner IDs before any
further read. The recipient’s existing workspace key now has owner powers; no new
secret is returned. Unrelated roles, keys, grants and content remain intact.
Transfer is unavailable while archived, and instance/admin keys cannot initiate
it. See [transfer semantics](workspace-owner-administration.md#transfer-ownership).

This endpoint has no operation ID or safe retry. An unconfirmed response retains
the original endpoint, workspace, former-owner and recipient IDs, with no retry
button. Reading current ownership is a separate observation, not attribution.
If the transfer committed, the old key can no longer inspect it: the session
lock preserves the original request and directs you to the intended new owner,
an instance admin’s owner-metadata read, or the host operator’s local inspection
and recovery. Do not retry under a different key. Forgetting or discarding the
local copy does not cancel a delayed transfer. Explicit local discard removes
private form/copy DOM and permits navigation even if metadata is unavailable.


## Inspect and manage invitations

**Clients & access** shows a separate, paginated invitation inventory to a current
workspace owner/admin key, including while archived. Instance read access does
not expose this security inventory. Rows show the full ID, kind, requested data
role, uses/capacity, deadline, observed expiry, revocation and current issuer
authority separately. Readmission rows also show their target client UUID.
**Next invitations** advances only that inventory. A failed metadata read does
not imply an empty list; an earlier table remains labelled as a prior observation.

Issuer authority is a current check. It is false while archived or after issuer
permission/key loss, separately from permanent invitation revocation. Metadata
is not a redemption lease or a causal operation receipt. No secret can be
retrieved. An empty list does not establish initial-group eligibility. The separate initial
group section reads the real window state before offering its controls.

In an active workspace, choose the requested data access and a 1–168 hour lifetime,
then **Create invitation**. Copy its one-time secret privately before navigating
away. Creation does not select the workspace for the client. **Revoke invitation**
is available to scoped owners/admins while archived too; it preserves existing
client grants/content. Revoking an initial-group invitation closes its window.

These creation/revocation endpoints have no operation ID or safe retry. An
unconfirmed response freezes the original endpoint/body/deadline and offers a
separate current observation. Creation observations show the first page only;
a new row does not prove which request created it. After investigating and
explicitly discarding locally, use the full pager. A known revocation target
can be inspected directly. There is no retry button. Forget/discard does not
cancel delayed execution. Local-only discard clears both copied request and
stale private form DOM, even if metadata is unavailable.

Acknowledgement clears the pending draft before any further read. A creation
secret is shown only after checking the same browser session; if that check
fails, the lock reports confirmed workspace/invitation IDs and directs you to
inspect/revoke the undelivered invitation before deliberately issuing another.
It never describes confirmed creation as failed. Drafts and unknown requests
remain in tab memory only. Client paging and workspace-grant revocation also
respect invitation drafts before redrawing this shared view.


## Open or close initial group enrollment

In **Clients & access**, a workspace owner/admin sees the real initial-window
state, history and archive status. Issuing an individual invitation permanently
seals an unused initial group phase, even before any client redeems it.
**Configure initial group** appears only when the status read currently permits
opening. It opens a separate form so a group draft cannot silently replace an
individual-invitation draft. Choose a positive
admission capacity, a positive whole lifetime in hours, or both. Neither numeric
field has a default. Confirm the initially unchecked single-phase acknowledgement
and review the exact bounds before creating the one-time group invitation.
Extreme deadlines outside the browser’s date range require the API.

Copy and deliver the secret privately before leaving. A lost group secret cannot
be retrieved or replaced by reopening the phase. Inspect/revoke that invitation
or close the window, then use individual invitations. A known-open group suppresses
the individual-create form until closure; existing invitation history stays visible.

**Close initial enrollment permanently** works even while archived and can seal
an unused phase before any group link is created. It cannot reopen. Existing client
grants and individual invitations are preserved. A status-read failure never
assumes an unused/open/closed state or exposes group actions; other authorized
workflows retain their own server checks.

Group create/close actions send once, preserve the original endpoint/body/session
binding, and have no operation ID or retry button. After an unconfirmed response,
read current window state as a separate observation, not attribution. Forgetting
or discarding locally does not cancel delayed execution. Explicit local discard
clears the request and stale form even if reads are unavailable. An acknowledged
creation is recorded before the next session check; if that check fails, the lock
reports confirmed IDs with close/revoke and individual-invitation recovery directions,
without showing the unavailable secret or describing creation as failed.


## Recover or explicitly readmit a registered client

Choose **Manage client access** on the exact grant row in **Clients & access**.
The detail shows the registered client UUID and the current key’s recovery scope.
An instance key recovers across all of that client’s still-authorized workspaces;
a workspace key recovers this workspace only, including readable archived context.
The history list contains invitations in that exact scope, not a union of global
and workspace records. Use its independent pager and current status facts; those
facts do not prove fresh recovery is currently possible.

**Issue workspace recovery invitation** requires a current grant row. For a
historical revoked row, choose a current grant row if one exists. An instance key
can request global recovery even from a historical row, but the server rejects an
empty current capture. Globally disabled clients get no fresh recovery/readmission
action; their security history can still be revoked.

Recovery preserves the client UUID and entry ownership. Issuance captures existing
still-authorized grants; later grants are never added and revoked grants are excluded
at redemption. Issuance alone does not rotate credentials. Redeeming the secret with
a fresh key replaces older bindings for captured grants only. Keep the original
profile for unrelated workspaces and use a separate empty recovery profile; do not
merge aliases or permissions. Issuing another invitation does not revoke older
ones. See [client recovery](workspace-client.md#recover-into-a-separate-profile).

**Explicitly readmit client** is a different action: an active workspace owner/admin
chooses a new data role and issues targeted admission for the same client UUID.
Every prior grant there must be revoked; the server rechecks the complete history.
Old grant revocations remain terminal. An active initial group must close first;
issuing a targeted readmission invitation permanently closes an unused initial
group phase, even if it is never redeemed or is later revoked. A later admission
makes a pending ticket stale even if that later grant is revoked. The client redeems using a fresh
key in an otherwise unenrolled profile; unrelated access and context selection do
not change. Instance keys cannot borrow this workspace admission power.

Both forms name the target client/scope, use a 1–168 hour lifetime, and show the
consequences before issuance. Copy the one-time secret privately before leaving.
**Revoke recovery invitation** blocks redemption and receipt retrieval, including
after redemption; it does not remove already-issued credentials or membership.
Use workspace-grant revocation or instance disable for those access changes.
Readmission invitation status/revocation remains in workspace invitation inventory.

Admin actions send once with frozen target/scope/body/session proof, without an
operation ID or replay button. Unknown outcomes keep the original request and
allow a separate first-page observation, never causal attribution. Known recovery
revocations inspect the exact requested invitation. Forget/discard is local, not
cancellation; direct discard clears copied/stale form DOM even if reads fail.
Acknowledgement clears pending work before any postcheck; a failed session check
reports confirmed invitation/client/scope IDs and concrete inspect/revoke recovery
without a secret or false creation failure. No automatic refresh discards a secret.

## Manage shelf archives and permanently purge entries

Shelf inspection displays the immutable shelf/workspace IDs and the last-read
lifecycle revision. **Archive shelf** and **Restore shelf** require a current
workspace owner/admin key and active workspace. Their final confirmation names
that exact target and revision. This changes visibility, preserving descendant
IDs, ownership, revisions and individual archive states. It neither purges shelf
payloads nor enables timed entry cleanup.

For an individually archived entry summary, expand **Permanently purge archived
entry**. Type the exact entry ID, check the irreversible-removal consent, then
choose **Permanently purge entry** and confirm the named workspace and revision.
A current workspace owner/admin can perform this explicit action even while the
workspace is archived. Instance keys cannot borrow that power. An active entry
hidden only by a shelf archive is not eligible. Body/history remain unavailable
throughout this workflow.

Purge removes all entry payload and revision rows from live storage. It cannot
be restored through CCP. Retained operation proofs and audit remain; SQLite free
pages, journals and separate backups are not securely erased by this action.
See [the purge contract](workspace-content-http.md#permanently-purge-an-archived-entry).

Both shelf transitions and purge freeze the original operation UUID, target,
expected revision, JSON and session proof. After a lost response, **Check / retry
exact archive operation** deliberately resends those exact bytes in the same
session. No automatic mutation retry, replacement UUID or revision adjustment
occurs. A returned receipt describes the original historical action; separately
read current metadata before deciding what to do next. A current observation
does not attribute an uncertain action. Entry observations show only one page.
Conflicts require a new decision after explicitly discarding the old local intent.

A confirmed receipt remains confirmed if the next session check or metadata read
fails. Copy the original request before changing keys; pending operations cannot
be replayed under another browser session. **Forget archive operation locally**
does not cancel it. Explicitly discarding the retained local copy clears its DOM
without requiring a successful metadata read. Purge drafts also block navigation
until submitted or explicitly discarded. Nothing is stored persistently in the
browser.
