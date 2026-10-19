# Workspace access foundation

This unreleased foundation adds persistent identity and authorization records
for the workspace redesign. Phase 1 introduced schema 5; the scoped-key batch
adds schema 6; invitation policy records add schema 7 and atomic certificate redemption adds schema 8. The existing native mTLS client protocol and CLI
remain in service. No new HTTP listener, admin login, invitation endpoint or
workspace browser is exposed by this phase.

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
own schemas 5 through 8. Versions 3 and 4 are reserved for the historical HTTP holding
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
Certificate issuance, invitations and the new HTTP transport remain subsequent
integration work.

A client data action requires a live, unexpired, nonrevoked credential, an
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
Ownership transfer, recovery credentials and payload purge are not implemented
by this phase's foundation primitives.
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
and compatibility checks still apply. Passing these checks does not establish
the new HTTP, browser or enrollment flows before those phases are implemented.
