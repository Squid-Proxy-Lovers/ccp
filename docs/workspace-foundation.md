# Workspace access foundation

This unreleased foundation adds persistent identity and authorization records
for the workspace redesign. Phase 1 introduced schema 5; the scoped-key batch
adds schema 6; invitation policy records add schema 7, atomic certificate
redemption adds schema 8, credential renewal adds schema 9, targeted recovery
adds schema 10, explicit targeted readmission adds schema 11 and data-directory
mode isolation adds schema 12; the public instance-TLS pin adds schema 13;
immutable TLS renewal generations add schema 14. Schema 15 adds the separate
[workspace shelf/book storage foundation](workspace-content.md); schema 16 adds
client-owned entries, immutable versions and recoverable entry archives there.
Schema 17 adds [body-free operation resolution](workspace-operation-resolution.md)
for the original client after renewal or recovery. Schema 18 adds internal
[shelf archive/restore](workspace-content.md#recover-a-whole-shelf) lifecycle
revisions and exact admin retry receipts. Schema 19 makes entry-operation
receipts a retained body-free ledger independent of version rows. It prepares
future purge safety without deleting content. Schema 20 adds the internal
[guidance publication foundation](#guidance-publication-foundation). Verified
legacy content conversion remains subsequent work. Schema 21 adds
[explicit archived-entry purge](workspace-content.md#permanently-purge-an-archived-entry)
with retained proof; migration itself deletes no content.
See [instance TLS setup](instance-tls-identity.md) for its separate private
bundle. The existing native mTLS client protocol and CLI remain in service in
native mode. The separate [workspace HTTPS API](workspace-http-api.md)
now exposes TLS-only invitation/enrollment/discovery and scoped admin surfaces.
The [browser-session API](workspace-browser-session.md) provides the authentication
boundary and a [desktop login/access/context panel](workspace-browser-session.md#use-the-desktop-panel).
Other panel workflows remain subsequent work as documented there.
[Content endpoints](workspace-content-http.md) expose hierarchy and entry actions;
[Shelf archive/restore](workspace-content-http.md#archive-or-restore-a-shelf) is
also exposed through scoped admin HTTPS, with state/archive summaries available
to current readers.

## Database upgrade

Schema 5 adds an instance UUID, workspace UUIDs, administrative principals and
roles, registered clients, credential records, per-workspace data grants,
credential-to-grant bindings and administrative audit records. The UUIDs are
independent of display names and TLS CA rotation.

Startup maps each existing session to a stable workspace UUID. Existing session
IDs and contents stay unchanged. A legacy `sessions.owner` string remains
display metadata: migration does not make that name an administrator, invent
an entry creator or grant access to a new client. Legacy workspace ownership
starts unassigned until an authorized initial-owner operation appoints it.
Legacy mappings may share a display name with a new workspace; UUIDs distinguish
them. A naming collision must not prevent an existing session from starting.
Names remain unique among new, non-purged workspaces.

Legacy follow-up migrations, foundation creation, legacy mappings and the
current schema-version record commit in one immediate transaction, reserving
the writer before reading migration state
so another session cannot invalidate its snapshot. Failure rolls those additions
back; an older recorded schema
version is not advanced. Databases with a newer schema version are refused.
Legacy table rebuilds run with foreign keys temporarily disabled on the private
initialization connection; normal runtime/access-store connections enforce them.

This phase supports the stable native database line through schema 2 and its
own schemas 5 through 24. Versions 3 and 4 are reserved for the historical HTTP holding
draft, including its different journal/checkpoint recovery behavior. Those
databases are explicitly refused before migration; this phase does not provide
an automatic conversion from that implementation. Keep their database, journal
and keys together with the exact producing binary. Do not change version numbers
or reuse that data directory with this server. A tested offline conversion is
required before supporting that transition. Merely retaining the old commits
in the draft branch's ancestry does not make their storage formats compatible.

Instance-hosted software downloads are not part of this server. The old draft's
`downloads/` artifacts are absent from this branch; installs continue to use the
official release/image paths on `main`, separate from workspace enrollment.

Back up the producing server's data directory, including database, journal and
protected key material, after a clean shutdown before upgrade. Rollback uses
that backup and its matching producing version; do not edit the schema version
to downgrade or assume old software understands the new records. Migration
cannot reconstruct creator identities or status rows absent from the old data.

## Effective authority

The `server::foundation` APIs are internal server building blocks. An `Actor`
must come from a trusted authentication layer; accepting a caller-supplied
principal UUID would not authenticate it. Scoped admin-key primitives are documented in [Workspace admin keys](workspace-admin-keys.md).
Internal certificate enrollment and renewal are documented in
[Workspace enrollment](workspace-enrollment.md). Their limited
[workspace HTTPS adapters](workspace-http-api.md) now support actual TLS requests;
the [workspace client](workspace-client.md) adds initial trust/profiles/enrollment
and discovery. Remaining client lifecycle, selection and MCP remain later work.

A client data action requires a live, unexpired, nonrevoked, unsuperseded credential, an
enabled client, a live workspace grant bound to that credential, a suitable
current role and a workspace state that permits the action. One client identity
can have different grants in several workspaces. Binding foreign keys prevent
attaching another client's grant to a credential. Revocation is terminal for a
grant record; re-admission uses a new grant ID. Other credentials or later grants
do not widen a credential's scope implicitly.

Global administrators can read workspace context and perform their explicit
instance/lifecycle actions. Global read access does not confer workspace-board
publication, client admission, grant revocation or owner-only admin-team powers.
Workspace owners and admins act only within their current scopes. There is one
owner reference per workspace; initial assignment cannot replace an owner.
Targeted recovery and retained binding revocations are documented in
[Client recovery](workspace-recovery.md).
[Owner administration](workspace-owner-administration.md) adds scoped team/key
management and ownership transfer.
[Explicit archived-entry purge](workspace-content.md#permanently-purge-an-archived-entry)
now has an internal core and a
[scoped HTTPS adapter](workspace-content-http.md#permanently-purge-an-archived-entry).
Human confirmation screens and timed cleanup remain subsequent work.
Initial-owner appointment is an explicit, audited global-admin operation for an
unowned workspace. The appointee may be that same global administrator; they
then exercise workspace powers through the separately recorded owner role.

Archived workspaces remain readable. Content writes, board saves and new client
admission are denied, while scoped security revocation and existing-client
recovery capabilities remain available. A purge capability check requires an
archived workspace, but that check is not a payload-deletion implementation.

Mutating foundation APIs check authority and write audit records inside an
immediate SQLite transaction. Audit failure rolls the mutation back. Discovery
uses a consistent read snapshot and includes only effectively accessible
workspaces. Standalone `authorize` calls must be composed with the protected
operation inside the same transaction; they do not issue cached permission
leases. Entry-specific ownership/version checks are a later content-layer
requirement in addition to the coarse workspace write capability.

## Local provisioning boundary

`bootstrap_admin` is an initial local-operator primitive, not a web signup API.
Any existing global-admin record prevents a second bootstrap, even when that
principal is disabled. An operator-facing command must enforce the correct OS
and data-directory boundary before calling it. No startup auto-claim by a web
visitor or automatic owner reset is introduced here.

## Verification scope

The new tests exercise role/scope matrices, cross-workspace denial, credential
bindings, expiry and revocation, global disable, archived read/write boundaries,
owner-only administration, audit rollback, persistent reopening, legacy-data
preservation, migration rollback and refusal of a future schema. Native CLI/MCP
and compatibility checks still apply. Separate actual localhost TLS suites
exercise the implemented management/enrollment adapters. Normal client/MCP,
content, browser and production-deployment flows remain incomplete.

## Runtime mode isolation

[Directory mode leases](workspace-runtime-mode.md) exclude native runtime and
local native tools from an activated workspace data directory. This storage
boundary does not grant any client/admin role, implement a new transport or
convert legacy contents. Native mode remains the default.

The [local administration CLI](local-workspace-administration.md) implements the
exclusive OS/data-directory boundary for first-instance provisioning and scoped
key recovery. It does not expose these operations through client or admin
network routes, change ownership or reopen enrollment.

## Guidance publication foundation

Internal server callers can use `AccessStore::save_guidance` to save the global
board or an explicitly named workspace board. The trusted authentication layer
supplies the actor. Global admins publish global guidance; workspace owners and
admins publish their workspace guidance. An instance-wide admin key cannot use
its principal's workspace role to publish there. Ordinary clients cannot publish.
Archived workspaces keep readable guidance but reject workspace saves and retries.

Supply a non-nil operation UUID, the expected revision (0 for an unpublished
board) and at most 8192 UTF-8 bytes of exact text. A successful new operation
immediately creates an immutable revision and makes it live. Saving identical
text with a new operation still records a new revision. Saving empty text
explicitly clears the published board; its positive revision and publisher
distinguish it from an unpublished board. A stale expected revision conflicts.

An exact retry rechecks current publishing authority and returns its original
body-free receipt without applying the save again. The operation binds its scope,
expected revision, exact text and authenticated publisher principal or key.
Operation namespaces are separate for each board. A changed request or publisher
under the same board and operation conflicts. The historical
receipt does not describe current board state or prove delivery or obedience.
Version, live pointer, receipt and publisher audit commit in one transaction.

`AccessStore::guidance` returns global and optional explicitly targeted workspace
guidance in one authorized database snapshot. Global guidance requires a current
authenticated identity; workspace guidance also requires current context-read
access. Source, revision, actual publisher, principal and server timestamp are
separate metadata. The bundle labels CCP precedence as global first, with
workspace text adding compatible detail. Host instructions and actual server
permissions remain authoritative; board text cannot grant permissions.

Schema 20 adds version, live-head and retained body-free operation tables.
Existing native content is preserved and is never promoted into guidance.
A schema-19 server refuses schema 20; rollback requires the complete matching
pre-upgrade backup and producing software.
[Authenticated guidance reads and scoped saves](workspace-http-api.md#read-and-save-guidance)
now expose these APIs over TLS, with
[normal client guidance reads](workspace-client.md#read-current-guidance) and
[explicit MCP refresh](workspace-mcp.md#refresh-guidance-explicitly). The bridge also
returns [automatic initial/changed guidance and unavailable warnings](workspace-mcp.md#read-automatic-guidance-alongside-results).
Desktop editors remain subsequent work.

The internal `client_guidance` read additionally returns authenticated instance,
client, credential and current bound-grant IDs in the same snapshot. It requires
a verified client actor; administrative keys cannot claim this client context.
It changes no authorization or persistent records. The
[delivery-context API](workspace-http-api.md#retrieve-an-authenticated-guidance-context)
and opt-in client output support the MCP adapter's caller-specific emission stamps.


## Timed entry-retention core

Schema 22 adds explicit scoped retention-policy history, live heads, durable
per-entry clocks and body-free timed-purge audit/proofs. Manual retention remains
the default. Enabling a chosen timed policy gives existing individual archives a
fresh full window; workspace archive pauses remaining duration across restart.
Restore removes the clock; rearchive starts a new full window. Current scoped
owners/admins configure policies; a trusted bounded cleanup method rechecks exact
due archive/policy state in its deletion transaction. Original write receipts and
cancellation fences survive; purge proofs prevent entry UUID reuse.

No policy is seeded or cleanup activated by migration. The workspace HTTPS
server now exposes scoped configuration and schedules current due cleanup with
bounded jobs, retained leases and shutdown draining. See the
[retention contract](workspace-retention.md) for confirmation, policy changes,
clock limits, audit and backup/rollback requirements.


## Ephemeral browser sessions

The HTTPS browser-session adapter exchanges the same existing admin key for an
opaque bounded in-memory session. Trusted key-ID metadata lookup rechecks current
live key scope/roles; the protected operation still checks its actor inside its
transaction. Cookie authority never comes from request IDs or display labels.
Session scope stays separate from registered clients and entry ownership. No
schema or underlying key lifetime changes. See the
[browser-session API](workspace-browser-session.md) for Origin/CSRF, expiry,
resource bounds, logout and current backend/frontend limits.


## Explicit work-status storage core

Schema 23 adds empty current-message and body-free operation-proof tables.
Client-owned slots retain their last message and timestamp when stale; explicit
clear removes text while retaining ownership/revision fences. Current read/write
permissions, CAS, credential-bound exact replay and audit commit together.
No automatic contact collection, task management, message-history archive or
cleanup starts on migration. The status HTTP adapter, client/MCP commands and
Reported work panel are available. Scoped owners/admins can explicitly clear a
client message while preserving ownership and historical proof; archived
workspaces remain read-only.
See the [status contract](workspace-status.md).

## Whole-workspace permanent deletion

Schema24 adds [confirmed archived-workspace deletion](workspace-deletion.md),
using a lifecycle revision, terminal UUID fence and bounded resumable cleanup.
Only the workspace owner or an instance admin can start it. Shared identities and
other workspace grants survive. Existing entry-purge retry protections remain
until a separately confirmed whole-workspace job retires that namespace.
