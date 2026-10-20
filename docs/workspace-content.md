# Workspace content storage foundation

This unreleased internal server API stores workspace shelves and books in shared
SQLite. Entry bodies, immutable revisions, archive/restore commands and the
HTTP/client/MCP content adapters are subsequent slices. The workspace MCP bridge
currently returns metadata and selections, not content.

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

## Schema and legacy data

Schema 15 adds `ccp_content_shelves` and `ccp_content_books`. It leaves native
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

Keep a complete protected backup before upgrading. A schema-14 binary refuses
the newer database; rollback uses the matching whole pre-upgrade backup and its
producing software. Do not lower the version or discard new tables to force an
older server to open it. No power-loss, throughput or full-content migration claim
is made by this storage foundation.
