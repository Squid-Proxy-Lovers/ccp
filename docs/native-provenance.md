# Native provenance storage

Unreleased server schema25. This storage contract supports the explicit
[native converter](native-conversion.md). Migration creates empty tables and starts no conversion, changes no
permissions and does not remove the legacy-content gate. The existing
[inventory command](native-conversion-inventory.md) remains observation-only.
There is no new HTTP, client, MCP or panel reader in this storage phase.

## Preserve observations without inventing history

`ccp_native_imports` records the source database SHA-256, schema, original numeric
session ID and the administrator/time of the conversion operation. Those fields
identify the import, not an original author or creation time. Raw owner, CN,
agent and host labels never grant access or become registered-client identities.

Source rows retain their original column order and typed values: NULL, signed
integer, real, TEXT and BLOB. Integers serialize as decimal strings and reals as
16 lowercase hex digits containing their exact IEEE754 bits, avoiding JSON number
rounding. Valid UTF-8 text stays text; invalid UTF-8 TEXT bytes remain distinct
from BLOB bytes. Original timestamps, IDs, labels and append records need not be
normalized or turned into fabricated historical snapshots. Capturing a source
row does not establish that its values fit normal workspace content limits.

Entry and append rows belong in `ccp_native_entry_rows`, bound to their exact
workspace and entry. Session, shelf, book, certificate, token and transfer rows
belong in `ccp_native_workspace_rows`; entry payload must not be placed there.
Native security records are retained provenance, not usable workspace credentials.
The converter validates ordinary source tables and assigns authoritative source
rows to their deletion scopes before publishing any output.

A source JSON row is limited to 2 MiB, its source key to 8192 bytes and its
column count to 256. These are support bounds, not historical native storage
limits. Conversion must refuse values it cannot preserve, leaving the source
intact, rather than truncating them. The converter also enforces normal workspace
field bounds and verifies complete authoritative source-row accounting.

## Visibility and sealing

An import starts unsealed; the converter records its rows and seals it in the
same transaction as adopting the content. Sealing is a one-way transition.
Rows cannot be updated or replaced, and a sealed import accepts no additional
rows. These guards protect recorded provenance; they are not remote authorization
or proof that the source producer stopped.

The internal `AccessStore::native_entry_history` method rechecks current workspace
read permission and the same entry/book/shelf visibility as ordinary snapshots.
Individually archived entries remain body-inaccessible until restored, including
their raw history. An archived workspace remains readable by authorized callers.
Revoked access, disabled clients and purged workspaces fail normal checks.
Unsealed imports and fresh entries without source rows return no provenance.

The reader exposes only the selected entry's sealed rows and import metadata.
It never exposes workspace-level native tokens, certificates or transfer records.
Ordinal pages contain at most 100 records, with a conservative 4 MiB budget
checked from stored lengths before loading payloads. Malformed stored rows fail
without returning a partial page or echoing source values.

## Retention, deletion and upgrades

Restoring or revising an entry retains its raw provenance. A privileged individual
purge or an authorized timed purge removes that entry's source rows through a
foreign-key cascade in the same transaction. A failure rolls back the purge and
its proof together. Sibling entries and workspace-level provenance survive.

[Confirmed whole-workspace deletion](workspace-deletion.md) removes all three
provenance tables for that workspace in bounded batches before removing ordinary
content. The completion predicate includes them; the terminal UUID fence blocks
new rows. Shared identities and other workspaces retain their data. Logical
deletion does not erase old SQLite pages, journals or external backups securely.

Back up the complete stopped directory before upgrading24 to25. The additive
migration preserves existing records and pending deletion jobs, and updates
cleanup guards atomically. No imports, policies or cleanup jobs are seeded.
Schema24 binaries refuse25; rollback requires the complete matching old backup
and its producing binary. Never lower the schema marker or copy selected tables
to force a downgrade. Conversion is an explicit offline operation; external
provenance access adapters remain separate work.
