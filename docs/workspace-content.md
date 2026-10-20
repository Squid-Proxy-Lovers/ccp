# Workspace content storage foundation

This unreleased internal server API stores workspace shelves, books and
client-owned entries in shared SQLite, with immutable revisions and recoverable
entry archives. The [authenticated HTTP adapter](workspace-content-http.md) now
exposes this store. The [normal client](workspace-content-client.md) now supports
content reads, shelf/book creation and [entry mutations](workspace-content-mutations.md).
The [workspace MCP bridge](workspace-mcp.md) also returns content reads alongside
metadata and selections; MCP writes follow later.

[Documentation index](README.md) · [Workspace access](workspace-foundation.md) ·
[Workspace MCP](workspace-mcp.md)

## Current internal contract

`foundation::AccessStore` provides `create_shelf`, `create_book`, `get_shelf`,
`get_book`, `shelves` and `books`. Callers must supply an actor authenticated by
the trusted transport, never an identity claimed in a request body.

Ordinary current read/write clients and scoped workspace owners/admins can create
non-destructive shelves and books. Read-only clients can read them. Global admin
keys retain cross-workspace reads but cannot borrow their principal's workspace
write role. Whole-shelf archival and permanent purge remain separately privileged
future operations; creating a shelf does not grant those powers.

Names are trimmed and contain 1–256 UTF-8 bytes without control characters.
Descriptions preserve their exact text and are limited to 4096 UTF-8 bytes.
Names are case-sensitive. Active shelf names are unique within a workspace;
book names are unique within a particular shelf. UUIDs identify the records,
so identical names in different parents do not select another target.

Read checks and queries share one SQLite snapshot. New writes check current
credentials, bindings, grants, workspace state and scope inside the same immediate
transaction as the node insert and audit. Audit failure rolls back the node.
Archived workspaces remain readable, but content writes fail. Revoked or disabled
clients cannot use old record IDs to bypass current authorization. Already-running
read snapshots are not promised to be retroactively cancelled.

Lists use UUID cursors, with limits from 1 through 500 and no fallback to another
workspace or parent. A book cannot reference a shelf from a different workspace:
the database enforces the composite parent/workspace foreign key. Archived shelf
rows retain their books and are excluded from active reads; there is no public
shelf-archive command in this slice.

## Entries, revisions and retries

`mutate_entry` accepts a fresh operation UUID, optional agent attribution and one
of `Create`, `Append`, `Revise`, `Archive` or `Restore`. Creation names an existing
book and sets text, description and labels. Later operations name the entry UUID
and its expected positive revision. A stale revision conflicts; the caller must
read current state before deciding how to proceed. Append adds the exact supplied
text without inserting a separator and preserves description and labels. Revise
replaces text, description and labels together while retaining prior snapshots.
Entry name, parent and owner remain fixed in this slice.

The authenticated registered client owns its entries; all agents using that
client share ownership. Other current writers can append, but only the owning
client with current write access can revise, archive or restore. Scoped workspace
owners/admins may override those actions with authenticated audit attribution.
Admin-created entries have explicit admin provenance and no client owner; an
ordinary client cannot claim them by appending. Agent names are labels in history,
never identities or permission selectors.

Each mutation stores a full immutable version, authenticated actor/credential or
admin key and principal, optional agent, trusted server timestamp and original
append contribution. Owner revisions preserve earlier contributions and versions.
All versions, the current head, the exact-retry receipt and the audit commit in
one immediate transaction; a failure rolls back all of them. Stored versions
reject SQL UPDATE; this does not make data tamper-proof against the host operator.
Timestamps record the supplied server clock, without claiming a monotonic clock.

An operation UUID binds the exact typed request, agent label and authenticated
credential/key within its workspace. An exact retry with current write authority
returns the original entry/revision receipt without appending again. Changing the
request or credential under that UUID conflicts. New credentials, including
recovery credentials, need fresh operation IDs for new mutations. Lost write
access blocks retries too. Receipts contain no body and do not expose archived
content. Retain the original operation ID until the result is known; the HTTP adapter accepts this receipt contract, while normal client automatic
retry integration remains later work.

Text is limited to 256 KiB of UTF-8; description to 4096 bytes. Labels retain their
order and exact case, with at most 32 distinct nonempty values of 1–128 bytes
without control characters. Agent attribution, when present, is 1–256 bytes
without controls. Text and descriptions preserve exact content and may contain
controls; future UIs must escape them. HTTP adapters must enforce encoded byte
budgets as well as these field bounds, since JSON escaping can increase size.

`get_entry` and `entry_version` return full snapshots only while the current
entry and its parent are active. A historical snapshot includes current entry
identity/head metadata and the explicitly requested version's separate revision
metadata. `entry_history` pages revision/actor metadata, without bodies. It also
requires the entry currently be active. Readers can page active or archived
summaries through `entries`; archive summaries contain IDs, name, owner, revision,
status and timestamps, with no body. While an entry is archived, no read method exposes its full body. All reads recheck current workspace access in their snapshot.
Lists use limits 1–500; entry cursors are UUIDs and history cursors are revisions.

Archive removes an entry from active context and preserves its ID, owner, body
and all versions. Restore requires current owner write access or a scoped admin.
A same-name active entry in the original book causes a conflict: restoration
never replaces it. The caller can resolve the collision separately and retry the
uncommitted restore operation. Workspace archival still allows reads but blocks
content writes, including archive/restore and replay. Whole-shelf lifecycle,
permanent purge and optional timed retention remain subsequent work; manual
retention is the default and this slice starts no cleanup worker.

## Schema and legacy data

Schema 15 adds `ccp_content_shelves` and `ccp_content_books`; schema 16 adds
entries, immutable versions and operation receipts. Schema 17 adds permanent
body-free [client operation resolution fences](workspace-operation-resolution.md).
These additions leave native
session payload, history, archives and journal storage intact. Instance identities,
CA material, admin keys, grants and client profiles retain their existing formats.
Native wire protocol 1 and workspace JSON API 1 are unchanged.

The internal hierarchy API refuses a workspace mapped from a legacy native
session until its content conversion is verified. It does not report an empty
workspace, copy unreplayed native SQLite, or infer ownership from legacy text.
Conversion must account for the journal under exclusive activation before those
contents can be exposed through the new path. Continue using the supported native
interface with its original data until that transition is implemented.
Do not activate workspace mode for legacy content yet: activation blocks native
access and has no in-place deactivation. See the
[activation and backup boundary](workspace-runtime-mode.md#explicit-workspace-activation).

Keep a complete protected backup before upgrading. A schema-16 binary refuses
schema 17; rollback uses the matching whole pre-upgrade backup and its
producing software. Do not lower the version or discard new tables to force an
older server to open it. No power-loss, throughput or full-content migration claim
is made by this storage foundation.
