# Configure timed entry retention

Archived entries are retained until privileged manual purge by default. In the
unreleased workspace server, current scoped owners/admins can explicitly enable
timed cleanup through HTTPS. The workspace-mode worker then removes due individual
archives. An upgrade never enables a policy; previously enabled policies resume
when that workspace server runs. The [desktop Settings tab](workspace-browser-session.md#workspace-settings)
provides policy controls; no CLI/MCP configuration command is provided.

[Documentation index](README.md) · [Entry purge](workspace-content.md#permanently-purge-an-archived-entry) ·
[Workspace foundation](workspace-foundation.md)

## Read and change the policy over HTTPS

Start the [workspace HTTPS server](workspace-http-api.md#prepare-and-start).
Use its independently verified CA and the protected owner/admin header file from
[the API setup](workspace-http-api.md#try-a-protected-request). These routes are
HTTPS admin-only; client mTLS has no retention endpoint. A global key can inspect
policy metadata but cannot change a workspace policy through borrowed roles.

POST to `/api/v1/admin/content/retention/get` with your actual workspace UUID:

```bash
umask 077
cat > retention-target.json <<'JSON'
{"workspace_id":"11111111-1111-4111-8111-111111111111"}
JSON
curl --cacert ./ccp-ca.pem --header @./owner-auth-header.txt \
  --header 'Content-Type: application/json' --data-binary @./retention-target.json \
  https://127.0.0.1:8443/api/v1/admin/content/retention/get
```

The API1 `data` object contains `workspace_id`, current `revision`,
`retention_seconds` and `changed_at`. Revision 0/null duration/null time is the
initial manual policy. A null duration at a positive revision is a later explicit
disable.

Choose a duration and explicitly confirm permanent timed deletion before creating
the change request. Existing individual archives receive the full chosen window.
The following one-day duration is illustrative, not a default. Replace both UUIDs
and use the current policy revision returned by Get:

```json
{
  "workspace_id":"11111111-1111-4111-8111-111111111111",
  "operation_id":"22222222-2222-4222-8222-222222222222",
  "expected_revision":0,
  "retention_seconds":86400,
  "confirm_timed_purge":true
}
```

Save it privately as `retention-change.json`, then send the complete request:

```bash
curl --cacert ./ccp-ca.pem --header @./owner-auth-header.txt \
  --header 'Content-Type: application/json' --data-binary @./retention-change.json \
  https://127.0.0.1:8443/api/v1/admin/content/retention/change
```

The response `data` contains your `operation_id` and a `policy` object with the
historical result. For a lost response, retry the exact saved request with the
same operation and key while authorized; do not generate a replacement nonce.
Read Get separately for current state. A stale revision or changed replay returns
409 `content_conflict`; an invalid shape/confirmation returns400. Demoted/revoked
keys fail current authentication or scope checks. Internal failures return a
sanitized503. The64KiB request/1MiB response/no-store limits remain unchanged.

To disable, use a fresh operation and current expected revision, with REQUIRED
`retention_seconds: null` and `confirm_timed_purge: false`. Omitting the duration
field is invalid. Unknown fields and caller identity claims are rejected.
Configuration is frozen while the workspace is archived. The desktop panel
requires explicit confirmation before enabling timed cleanup. The API flag records
the authenticated caller’s intent; it does not prove that a particular human clicked.

## Configure an explicit policy

The underlying trusted server storage API `AccessStore::change_entry_retention` takes an
already authenticated actor, an explicit workspace and `EntryRetentionChange`.
Only current workspace owners/admins may configure retention in an active,
converted workspace. A global key cannot borrow its principal's workspace role;
ordinary client read/write permissions never include policy management.

The request binds an operation UUID, expected policy revision and either a chosen
positive `retention_seconds` or manual mode (`None`). Enabling timed deletion also
requires `confirm_timed_purge: true`; disabling uses `false`. There is no default
duration. Seconds must fit a signed 64-bit integer. Confirmation records explicit
intent; it does not prove that a human clicked a confirmation control.

Enabling gives **every existing individually archived entry a fresh full window**
starting at policy enable time, regardless of its original archive age. Entries
archived afterward receive the full window in their archive transaction. Ordinary
archiving cannot create a policy implicitly.

Disable before enabling a different duration. Disabling removes all clocks and
returns to manual retention; re-enabling starts fresh full windows for existing
archives. A fresh enable request while already enabled conflicts, including the
same duration. Policy revisions reserve the last representable revision for
turning timed cleanup off.

Each new accepted policy transition is audited with its configuring actor/key and
principal. Exact retries with the original operation, actor, expected revision
and duration return the historical policy result after current authorization.
They never reapply a transition or reset a clock. Call `entry_retention` separately
for current configuration. This administrative read requires current workspace
read authority, including global administrative metadata reads; it is not a client
metadata endpoint.

Policy belongs to the workspace. The configuring key's later expiry/revocation
does not silently disable it; current workspace owners/admins can disable it.
Ownership changes likewise do not reset clocks. Configuring an archived workspace
is refused until reactivation. Explicit manual purge of an already archived entry
remains separately permitted to scoped owners/admins while the workspace is archived.

## Follow the remaining retention window

Each durable clock binds an exact entry archive revision and enabled policy
revision. Restoring an entry removes that clock. Archiving it again starts a new
full window; an old clock cannot delete its restored or newer revision.

| Event | Clock behavior |
| --- | --- |
| Enable timed policy | Existing individual archives get a full new window |
| Archive an entry under a timed policy | Start its full window atomically with the archive |
| Restore an entry | Remove its clock |
| Archive a workspace | Persist remaining seconds and pause |
| Reactivate a workspace | Resume the saved remaining seconds |
| Disable timed policy | Remove clocks; retain entries manually |
| Explicitly purge an entry | Remove its clock with its payload; retain purge proofs |

Shelf archival is a parent visibility overlay. It does not start clocks for
individually active entries hidden by that shelf and does not pause clocks of
entries already individually archived. Timed whole-shelf deletion is not part
of this implementation.

Active workspace downtime counts as elapsed UTC wall time. Paused clocks survive
restart without consuming time. A backward clock change consumes no negative
elapsed duration; a forward jump can make an active clock due. The host operator
must maintain the server clock. Clocks store start plus remaining seconds and
compare elapsed time without adding an overflowing deadline.

## Remove only currently due archives

The internal `purge_due_entry_archives` method is for a trusted server worker,
not a remote caller identity or an admin override endpoint. The core accepts one
workspace, trusted server time and a batch bound from 1 to 32. The running
workspace server schedules it automatically for explicitly enabled policies.

Within one write transaction, each purge requires an active, nonlegacy workspace,
its current enabled policy, the matching clock, the exact individually archived
entry head/version and enough elapsed time. Any write failure rolls back the
whole captured batch. Workspace archival and restoration serialize against cleanup;
cleanup cannot consume a paused workspace's remaining window.

The worker runs serially on a one-second cadence with missed ticks skipped.
Each cycle inspects at most16 active timed-policy workspaces in UUID order and
purges at most8 entries per workspace. Its cursor wraps, so a large instance can
need multiple cycles; expiry is eligibility, not an exact deletion-time promise.
A failed workspace does not prevent another workspace in that page from running.
Failures retry later with a static warning and count, without entry text, secrets
or raw SQL errors. Native mode does not start this worker.

There is one blocking cleanup job at a time, separate from the64 session slots.
It retains its directory lease and permit even if its async waiter is cancelled.
Shutdown stops new scheduling and waits for existing database work; a drain timeout
returns an error while any remaining job continues to hold its lease. There is no
promise of a fixed purge runtime or power-loss durability beyond the storage
contract.

Cleanup removes the entry and all its versions, with its clock. It retains an
immutable body-free purge proof linked to the exact policy version. That proof is
the automatic purge audit event; the worker does not impersonate the configuring
admin or key. Policy history preserves who enabled the policy. The proof prevents
entry UUID reuse, while original entry-operation receipts and cancellation fences
remain available for authorized outcome resolution. Retrying an old committed
entry write returns its historical result without recreating content.

These are logical database deletions. SQLite pages, journals and backups are not
promised securely erased. Backup retention and whole-workspace purge proof lifetime
remain separate work.

## Upgrade without enabling cleanup

Schema 22 creates empty retention policy, head, clock and purge-proof tables.
Existing entries, archives, versions, manual purge proofs, guidance and original
operation receipts remain unchanged. No policy or clock is inferred from archive
age. Unconverted native content remains protected by its conversion gate.

Schema-21 software refuses schema 22. Rollback requires the complete matching
pre-upgrade backup and producing software; a downgrade of the live database is
unsupported. See [verification](../tests/README.md) for the focused lifecycle,
transaction-fault, migration and compatibility checks.
