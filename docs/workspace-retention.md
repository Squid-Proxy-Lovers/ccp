# Understand timed entry retention

Archived entries are retained until privileged manual purge by default. Optional
timed entry cleanup now has an internal storage implementation. This unreleased
slice exposes no HTTP/CLI setting and starts no production cleanup worker; an
upgrade alone never enables retention or deletes content. Public configuration
and worker activation are subsequent work.

[Documentation index](README.md) · [Entry purge](workspace-content.md#permanently-purge-an-archived-entry) ·
[Workspace foundation](workspace-foundation.md)

## Configure an explicit policy

The trusted server storage API `AccessStore::change_entry_retention` takes an
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
not a remote caller identity or an admin override endpoint. It accepts one explicit
workspace, trusted server time and a batch bound from 1 to 32. No background task
is started by this API.

Within one write transaction, each purge requires an active, nonlegacy workspace,
its current enabled policy, the matching clock, the exact individually archived
entry head/version and enough elapsed time. Any write failure rolls back the
whole captured batch. Workspace archival and restoration serialize against cleanup;
cleanup cannot consume a paused workspace's remaining window.

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
