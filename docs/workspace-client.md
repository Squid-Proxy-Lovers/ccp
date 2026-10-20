# Enroll a workspace client

This unreleased client supports trusted workspace profiles, initial enrollment
and paged workspace discovery over HTTPS/mTLS, same-registration joining,
credential renewal and recovery into a separate profile. It also accepts a fresh
targeted readmission invitation into an unenrolled profile. Explicit
[caller workspace selection](workspace-selection.md) is available separately.
The [workspace MCP bridge](workspace-mcp.md) connects discovery and caller choices;
the [normal client content commands](workspace-content-client.md) now support
reads, shelf/book creation and [entry mutations](workspace-content-mutations.md).
The workspace MCP bridge exposes content reads, shelf/book creation and guarded
entry mutations. Existing
native commands and their saved session files keep their current behavior.

[Documentation index](README.md) · [Server HTTPS setup](workspace-http-api.md) ·
[Enrollment contract](workspace-enrollment.md)

## Establish server trust first

Obtain the instance UUID and public CA certificate from the host operator through
a trusted channel. Verify the CA file's SHA-256 fingerprint against the operator's
public trust output before sending an invitation. Do not download trust from an
invitation's unverified server response or install the CA as an OS/browser root.
The operator must already have started the workspace HTTPS and mTLS listeners.

Choose an explicit private profile directory. Replace the UUID and origins below
with the operator's actual values:

```bash
ccp-client workspace trust \
  --profile "$HOME/.ccp-client/workspaces/office" \
  --instance-id 00000000-0000-4000-8000-000000000001 \
  --https-origin https://ccp.example:8443 \
  --mtls-origin https://ccp.example:8444 \
  --ca-file ./ccp-ca.pem
```

The origins must use HTTPS, with no credentials, non-root path, query or fragment.
The client uses only the supplied root, verifies the server name and checks the
intended instance UUID through API version 1 health before publishing the profile.
It disables proxies, redirects, early data and plaintext fallback. HTTP/1.1 is
explicit. On Unix a new profile directory is 0700 and its SQLite file is 0600;
an existing directory/file with group or other access is refused. Windows uses
inherited ACLs: choose a local directory accessible only to your account. This
implementation does not inspect or install Windows ACLs.

Trust setup prints public profile metadata, including `ca_sha256`. An existing
profile is never replaced, even when setup is repeated with different settings.
There is no implicit default workspace profile or fallback to a native session.
`CCP_CLIENT_HOME` does not override the explicit workspace profile path.

## Redeem an invitation

Ask the workspace owner/admin for a current initial or individual invitation.
Store it in an owner-only regular file using a private editor or secure transfer;
do not put the bearer value in command-line arguments. On Unix, create an empty
private file before entering the invitation:

```bash
install -m 600 /dev/null ./workspace-invitation.txt
```

After saving the invitation in that file:

```bash
ccp-client workspace enroll \
  --profile "$HOME/.ccp-client/workspaces/office" \
  --invitation-file ./workspace-invitation.txt
```

The client durably saves a fresh request UUID, private key and signed CSR before
sending the enrollment request. It stores an invitation hash, not the bearer
value. It rechecks HTTPS health and the intended instance before sending that
value or CSR. The response must match the pinned CA, instance UUID, pending key,
client/credential UUID URI identities, serial, validity and ClientAuth-only
certificate profile. Only then does the client atomically activate it and clear
the pending request. No private key or invitation is printed in metadata/errors.

After a command error, inspect `workspace info` first. If a client UUID is already
present, activation committed and the saved profile can be used. Otherwise, if
a response was lost or activation failed, retain the profile and original
invitation file and run the same enrollment command again. The same UUID, CSR
and key retrieve the original committed public certificate without another
registration or quota use. A different invitation cannot silently replace a
pending request. If that invitation is no longer usable, keep the directory and
ask the administrator to inspect the admission; do not discard its pending key
on the assumption that enrollment failed at the server.

An enrolled profile refuses another initial enrollment. Reuse its registration
for later workspace access; do not create another registration merely to join a
second workspace on the same instance. Use `workspace join` below for a new workspace grant.
A separately authorized [targeted readmission](workspace-readmission.md) can use
an otherwise unenrolled profile with a fresh key; the server captures the
existing client identity and creates a new scoped grant.

## Join another workspace or renew

Use a fresh invitation for a second workspace on the same instance:

```bash
ccp-client workspace join \
  --profile "$HOME/.ccp-client/workspaces/office" \
  --invitation-file ./second-workspace-invitation.txt
ccp-client workspace renew --profile "$HOME/.ccp-client/workspaces/office"
```

Join proves the existing identity through actual mTLS, with a signed CSR for its
current key. The client UUID, credential UUID, certificate and key remain the
same; the server grants only the invitation's workspace/role. The new workspace
becomes discoverable but is not automatically selected for routine context reads.
A known completed invitation is refused locally before creating another pending
operation. That history records past completion, not current access; inspect
workspaces to check today's authorization.

Renewal creates a fresh pending key and request UUID, uses the old actual TLS
peer on the renewal endpoint, and requires the returned certificate to preserve
the client UUID and rotate the credential. The old key remains saved until the
new material activates atomically. A committed renewal supersedes the old peer
for ordinary requests. If activation or a response fails, the next discovery
resumes the exact renewal receipt before attempting an ordinary request. It does
not send the superseded peer to other routes or replace the pending key.

Successful nonempty discovery automatically renews when seven days or less
remain and the root permits a later expiry, then reloads the requested page
using the new credential. Empty discovery does not queue a grantless renewal.
A root-capped certificate is not repeatedly rotated automatically. Manual renew
can still rotate its key, but cannot extend it past the CA's expiry. There is no
background updater or timer; refresh happens on client use.

A pending join requires its original invitation to finish. Discovery may still
use the current credential, while automatic renewal is suppressed until that
join resolves. Another join/invitation or explicit renewal cannot silently
replace it. A pending renewal must finish before joining or ordinary discovery.
Errors preserve the operation for retry and require current server permission;
there is no fallback to a possibly superseded credential. Expired certificates
cannot authenticate a renewal or admission. Ask an authorized admin for fresh
recovery instead. Preserve every existing profile until recovery is complete:
a scoped recovery may replace A while the old credential still authorizes B.
Use the separate-profile recovery flow below to retain both credentials.

A persistently refused renewal keeps its saved operation and blocks later
discovery and joining, including a newly issued join invitation. Preserve that
profile. Ask an authorized admin for recovery of still-authorized grants into a
separate profile. If membership was revoked, an admin must instead issue fresh
[targeted readmission](workspace-readmission.md). Neither a new ordinary
invitation nor restoring a backup bypasses the saved operation or server state.

## Recover into a separate profile

Obtain a fresh recovery invitation and the existing client UUID from an authorized
admin. Verify the intended instance UUID, CA and origins as in initial trust.
Create a separate empty profile, for example with the same verified trust values:

```bash
ccp-client workspace trust --profile "$HOME/.ccp-client/workspaces/recovered" \
  --instance-id "$INSTANCE_ID" --https-origin "$HTTPS_ORIGIN" \
  --mtls-origin "$MTLS_ORIGIN" --ca-file ./ccp-ca.pem
ccp-client workspace recover --profile "$HOME/.ccp-client/workspaces/recovered" \
  --expected-client-id "$CLIENT_ID" --invitation-file ./recovery-invitation.txt
ccp-client workspace workspaces --profile "$HOME/.ccp-client/workspaces/recovered"
```

Protect the invitation file with owner-only permissions. Recovery uses the fresh
bearer invitation on the verified HTTPS listener; an old or expired certificate
does not authorize it. The expected UUID checks identity continuity and does not
grant permission. The server captures the allowed grants and rechecks their
current state, issuer authority and global client disable.

Recovery saves its exact target, nonce, new key, CSR and invitation hash before
sending. Retry with the same profile, expected UUID and invitation after a lost
response or local activation failure. A different target, invitation or operation
cannot replace the pending state. The client validates the complete certificate
receipt and expected UUID before atomic activation. A wrong target refuses
activation; preserve the profile and consult the issuing admin.

An active profile is never overwritten by recovery. Scoped recovery of A gives
the new profile A only; keep using the original profile for any still-authorized
B. Choose the profile explicitly on each command. The client does not merge
profiles or automatically select newly recovered workspaces. Original private
material stays intact even when recovery supersedes its server bindings. The
[recovery contract](workspace-recovery.md) describes scope and retry limits.

## Inspect and discover

```bash
ccp-client workspace info --profile "$HOME/.ccp-client/workspaces/office"
ccp-client workspace workspaces --profile "$HOME/.ccp-client/workspaces/office"
```

Info is local metadata: format, pinned origins/instance/CA fingerprint, pending
request UUID/operation/expected target and optional client/credential UUIDs, expiry and renewal-due
status. It can show expired credentials as expired, but is not proof of current
server authorization.
Online discovery revalidates the certificate and uses actual mTLS; the server
checks current disable, credential and grant state. A denial never triggers
implicit registration or a trust downgrade.

Discovery returns `workspaces` and optional `next_cursor`. The default limit is
100, maximum 500. Pass `--limit` and the returned opaque string as `--after` for
the next page. Listings expose only workspaces authorized to that credential.
They do not select workspaces for routine context reads. Each page is a current
snapshot; concurrent changes can change ordering. Response bodies are limited
to 1 MiB, connect timeout 5s and total request/body timeout 20s. Errors include only
local descriptions and HTTP status, without echoing an untrusted server body.

## Profile storage and recovery limits

Workspace profile format 4 uses `workspace-profile.sqlite3` with a separate
application ID, DELETE journaling and FULL synchronization. Open profiles hold
an exclusive SQLite lease across commits and network calls. Another invocation
can fail busy after 2s; close the first profile before retrying. This serializes
profile changes without stale PID lock files. Missing, corrupt, foreign or future
formats fail closed; they do not regenerate keys or trust.

The private key is stored locally in the owner-protected SQLite file. It is not
encrypted by a separate keychain, and key erasure from free pages/backups is not
promised. Stop clients and close every profile lease before copying the entire
private profile directory for backup. Keep backups private. The file system
must support SQLite locks, hard links and the required local file operations.
Host operators remain trusted against path/file replacement races. Unix
publication syncs the directory; Windows directory-sync and ACL behavior have
not been independently verified locally.

Format 1, 2 and 3 profiles upgrade when opened with this client. Before migration,
the exclusive lease protects a complete owner-only, content-addressed snapshot
named `workspace-profile-vVERSION-HASH.sqlite3`, with its original format number.
Publication is durable on supported Unix local file systems and never
overwrites another snapshot.
The transaction then preserves the key, CSR, nonce, invitation hash and identity
and existing completed-invitation history while adding a bounded pending-entry
request and most recent terminal result. Format 1/2 also gain the recovery state
introduced in format 3. Existing format-3 profile/history records stay byte-exact.
A failure leaves the previous format and its snapshot intact. Snapshots are
retained without automatic cleanup. Older format-1/2/3 clients refuse format 4;
restore a deliberately chosen snapshot into a separate private directory for a
controlled rollback. Restoring a local snapshot never restores revoked or
superseded server authority. No backup is selected automatically if the main
profile is missing or corrupt.

Format 4 permits a profile file up to 16 MiB to hold escaped pending text; old
formats keep their 4 MiB input limit. Per-record limits and pending-content
renewal controls are described in [entry mutations](workspace-content-mutations.md).

A client whose certificate has expired or been revoked cannot use that old
certificate to regain access. Fresh administrative recovery authority is required.
This draft is not a production rollout or a throughput/power-loss guarantee.


## Read workspace content

The [content commands](workspace-content-client.md) read shelves, books, active
entries, earlier versions and archived summaries through current mTLS access.
Each command names an explicit workspace; it does not alter caller selections.
Use [entry mutations](workspace-content-mutations.md) to edit entries and retain
the original request safely across uncertain results.
Entry mutations and workspace MCP content tools remain subsequent work.

## Read current guidance

Use an enrolled profile and the trusted instance to retrieve current guidance.
Omitting the workspace requests global guidance only. Supply an accessible
workspace UUID to retrieve its board alongside global guidance:

```bash
ccp-client workspace guidance --profile ./my-profile
ccp-client workspace guidance --profile ./my-profile \
  --workspace-id 11111111-1111-4111-8111-111111111111
```

These commands return the validated bundle as JSON: `precedence`, `global` and
optional `workspace`. Each board carries its exact text, source, revision and
historical publisher key/principal/time. Revision 0 with empty text and no
publisher means unpublished; a positive revision with empty text is an explicit
clear. Global guidance takes precedence within CCP; host instructions and actual
server permissions retain priority. The client preserves text, including controls,
up to 8192 UTF-8 bytes per board. Terminal or UI renderers should display that
JSON as data. See the [guidance API contract](workspace-http-api.md#read-and-save-guidance).

The library exposes `WorkspaceProfile::guidance(Option<&str>)`. Omission never
uses or changes caller selections. Reads require the pinned CA, verified server
name, expected instance and current authenticated client. Explicit workspace
reads recheck current grants; archived workspace guidance remains readable.
Malformed source/revision/publisher metadata fails without partial output or
remote body fragments. Known metadata is validated before output or renewal.
Additive bundle, board and publisher fields are dropped; source descriptors and
publisher kinds remain closed typed values and reject unknown forms.

An authorized, validated workspace response may trigger one due renewal and
reread. A global-only response proves identity without proving a usable workspace
grant, so it does not initiate renewal, including after all grants are revoked.
Already pending renewal resumes first. Pending admission and uncertain entry
intents stay intact and suppress new renewal; reads do not replay an entry write.
Scoped recovery profiles read only their captured still-authorized workspaces.

This explicit reader does not publish guidance or implement automatic initial/
changed guidance delivery with other tool results. The MCP bridge supports
[explicit refresh](workspace-mcp.md#refresh-guidance-explicitly) and
[automatic initial/changed guidance](workspace-mcp.md#read-automatic-guidance-alongside-results)
with unavailable warnings beside other successful tool results. Desktop editors
remain later work.

## Retrieve a guidance delivery context

Use `--delivery-context` when building an adapter that needs to distinguish
registered identity and grant changes alongside current guidance:

```bash
ccp-client workspace guidance --profile ./my-profile --delivery-context \
  --workspace-id 11111111-1111-4111-8111-111111111111
```

The library equivalent is `WorkspaceProfile::guidance_delivery(Option<&str>)`.
It returns a format-1 wrapper with `context` and `guidance`; the latter has the
normal bundle shape. The context contains instance, client, credential and
optional workspace grant UUIDs from the server's authorized snapshot. Omitting
the workspace requests global-only context with no grant UUID. The default CLI
command and `WorkspaceProfile::guidance` output remain unchanged.

The client validates the context format, canonical non-nil IDs, exact pinned
instance/client/current credential and grant presence before output or a new
renewal. Guidance source, revision, text and publisher checks still apply.
After an authorized due renewal, the reread must match the new credential,
including when an earlier committed renewal is being resumed. Global-only reads
still never initiate renewal. Pending admission and entry guards are preserved.
Malformed or refused responses never become delivery evidence or partial output.

Re-admission can replace the grant ID while leaving the credential and board
revisions unchanged. Combine these facts with a separate caller-specific
emission context before deciding that guidance has already been returned. The
IDs are not permission tokens or leases and cannot replace current server checks.
This opt-in format adds no cache, cursor, automatic tool response or obedience
tracking itself; the [MCP adapter](workspace-mcp.md#read-automatic-guidance-alongside-results)
uses it for bounded caller-specific emission. See the [server context contract](workspace-http-api.md#retrieve-an-authenticated-guidance-context).
