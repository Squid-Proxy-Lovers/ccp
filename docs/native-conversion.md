# Convert an offline native instance

Unreleased server schema25. `server legacy-convert` converts a complete native
schema-2 database into a **fresh workspace instance at a new destination**. The
source directory stays intact for rollback. It does not upgrade an existing
workspace instance, merge databases or carry native authentication into the new
instance.

## Prepare and run

Stop the producing server and tools cleanly. Retain its complete directory,
matching binary and client registrations. Recover pending work using that
producing software before stopping it; never delete journals or SQLite sidecars
to force conversion. Run the [inventory command](native-conversion-inventory.md)
and use its exact database SHA-256 below.

```bash
umask 077
server legacy-inspect --data-dir /absolute/native-source
server legacy-convert \
  --data-dir /absolute/native-source \
  --destination /absolute/new-workspace-instance \
  --expect-source-sha256 <SHA256_FROM_INVENTORY> \
  --operator 'Your name' \
  --confirm-offline > /absolute/private/bootstrap-keys.json
```

The destination must not exist, including an empty directory or symlink. Its
parent must already exist, remain stable during the operation and be separate
from the source directory. `--confirm-offline` confirms the operator stopped the
producer; it cannot prove exclusion of old binaries that do not honor new leases.
The fingerprint and final source checks are observations, not a future-writer lock.

The operator argument explicitly creates a new global administrator and makes
that person the initial owner of every imported workspace. Output contains a
body-free conversion receipt, a fresh global key and a scoped owner key for each
workspace. These are one-time secrets, as with `workspace-init`; keep the output
private and outside the server data directory. Old owner, CN, agent and host
labels never select this administrator or become registrations or grants.

Provision fresh instance TLS, export its trust information and start the
[workspace server](workspace-http-api.md). Do not run `workspace-init` on the
converted instance: administration is already initialized.

```bash
server workspace-tls-init --data-dir /absolute/new-workspace-instance --name ccp.example.test
server workspace-serve --data-dir /absolute/new-workspace-instance
```

Use the new scoped keys in the desktop panel. Enroll clients through explicit new
invitations and pinned workspace profiles. Native certificates, tokens and CA
keys grant no access to the new instance. The conversion command does not touch
existing client registrations or select workspaces for them.

## What is preserved

All authoritative rows from sessions, shelves, books, active and archived entries,
both append-history tables, certificates, tokens and transfer records are accounted
for. Their original columns and typed values are retained as
[scoped native provenance](native-provenance.md), including IDs, arbitrary timestamp
strings, raw labels, optional fields and binary values. Physical empty containers
survive. Required parents missing from the physical hierarchy are created from
valid exact entry paths; their new operational metadata does not claim recovered
historical creation information.

Each native session becomes an active workspace. A stopped session's `is_active`
flag does not mean it was archived. Each entry gets a revision-1 current snapshot,
recorded by the new operator at conversion time. Imported archived entries remain
archived. Entries have no registered-client owner and remain admin-managed;
authorized writers may append according to normal permissions. No initial body,
historical creator or old full-snapshot revisions are invented. Exact old append
events remain separate raw source records.

Raw native token and certificate rows are deliberately retained as inert private
workspace provenance for fidelity. Normal entry-history readers cannot read those
workspace records. They grant no workspace authority and are removed by confirmed
whole-workspace deletion. Original CA/private-key/auth-secret files stay only in
the original directory; they are not copied into the new instance. The schema
ledger and derived FTS index remain in that original backup, rather than becoming
operational workspace content.

Imported workspace bindings are independent of native sessions, so reopening does
not recreate legacy workspaces. Entry purges remove the corresponding raw entry
history; confirmed workspace deletion removes all its raw and operational content.
Original source directories and external backups have separate retention and are
not erased by these operations. External provenance adapters remain separate work.

## Supported inputs and limits

The first converter supports complete native schema2, including data from the
accepted native server. It refuses native0/1, historical HTTP3/4, newer formats and
mixed databases already containing workspace state. Those files remain intact;
normal additive workspace schema upgrades are a separate operation. Never lower a
schema marker or copy selected tables to force admission.

Authoritative source tables must be ordinary tables. Views, virtual-table
masquerades and unknown application tables are refused; known derived FTS tables
are not treated as source content. Source keys must be globally unique and non-null
within each authoritative table, with integer IDs and bounded text archive keys.
Duplicate IDs across sessions are refused before a destination is built. Every
child reference must use the canonical key type and identify an existing parent
by exact binary comparison, independent of declared source collations. Orphan
rows are refused. Source DDL is never executed in the target.

Conversion supports at most 2 GiB of source database bytes, 256 sessions, 100,000
authoritative rows, 256 columns per row, 2 MiB per encoded raw row, 2 GiB of raw
JSON in total and a 4 GiB target database. Operational names must fit unchanged
within 256 bytes without surrounding whitespace or controls. Entry bodies are
limited to 256 KiB, descriptions to 4096 bytes and labels to 32 distinct nonempty
values of at most 128 bytes each. CSV labels are split without trimming,
deduplication or dropping empty values. Incompatible inputs refuse the complete
conversion; nothing is silently truncated. These are converter support bounds,
not historical native storage limits.

## Publication, failures and rollback

The converter builds a fresh private sibling staging directory, using 0700
directories and a 0600 database on Unix. It verifies row accounting, integrity and
foreign keys, persists workspace mode, checkpoints its own target and closes it
in DELETE-journal mode. It rechecks the source before creating the new destination
directory and hard-linking the complete database without replacement. This gives
atomic visibility of a ready database, not an atomic rename of an entire directory.
No source database copy, control ledger or old credentials are published alongside it.

A pre-publication failure produces no partial destination database. Another
writer's destination is never overwritten or removed. After the database link
succeeds, a synchronization or output failure can leave an already complete
destination with an unconfirmed result. Inspect it with `admin-info`; recover lost
key output through the existing [local key reset](local-workspace-administration.md).
Do not retry against, delete or reinitialize that destination automatically.

Normal returns clean up the converter's private staging directory. Interruption
can leave `.ccp-native-conversion-*` beside the destination or an inspection copy
in the system temporary directory; after the process stops, remove only that run's
owned staging files. They can contain source content and inert legacy secrets.
Logical cleanup is not forensic erasure. Filesystem durability under power loss,
hostile directory replacement and Windows ACL behavior are not established here.

Rollback uses the untouched complete original directory, its matching producing
binary and original client registrations. It does not merge later workspace
writes back into the native source. Stop the new instance before deliberately
returning to the old deployment.
