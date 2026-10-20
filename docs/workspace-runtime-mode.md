# Isolate native and workspace runtime modes

The unreleased workspace server needs a different authorization and persistence
path from the legacy native runtime. A data directory must not be served through
both at once. Mode leases now cover current native startup, session discovery,
health and token issuance, and provide an exclusive boundary for
[workspace HTTPS startup](workspace-http-api.md) and
[local provisioning/reset](local-workspace-administration.md). The workspace
runtime activates only with the explicit `--activate-workspace` flag. See the
[documentation index](README.md) for supported interfaces.

## Preserve the supported native flow

Native mode remains the default. Native runtime and local tools acquire shared
leases before opening the data database or journal. Matching protected directories
are refused by explicit startup/tool paths; legacy directory scans omit protected
SQLite fallbacks without reading their sessions. Normal native CLI, enrollment,
MCP and persistence formats remain supported. Name-based scans may initialize
mode metadata in candidate native directories; unexpected sibling errors abort
the scan. Explicit data-directory selection avoids discovery fallback.

A bootstrap keeps its lease. The runtime passes a clone to the journal handle
and writer thread, and state/handlers retain the journal handle. Background
writes, final drain, snapshots and crash persistence therefore cannot outlive
the mode protection. The guarded journal also checks that its path belongs to
the leased canonical directory. Filesystem aliases of that directory share the
same lease boundary.

## Explicit workspace activation

`DirectoryLease::activate_workspace` is the trusted local primitive used by
explicit workspace startup. It requires exclusive access; every shared native
lease must first be released. Stop all native processes cleanly and retain a
complete matching backup before activation. A nonempty legacy journal blocks
activation, so existing native recovery can replay it and complete a clean
shutdown first. Never discard a journal just to pass this check.

Under the exclusive lease, activation upgrades the data schema and commits a
workspace marker and local-operator audit in the main database. It then commits
the workspace marker in the separate mode ledger while retaining its exclusive
kernel lock. The main marker commits first: an interrupted or failed second
commit cannot reopen native access. Retrying current workspace activation or
exclusive maintenance can reconcile the stronger persisted mode.

Activation preserves current clients, keys, workspaces and grants. It does not
convert legacy contents, generate a new CA, issue credentials, open group
enrollment or start a network listener. Content migration and the independently
trusted workspace transport remain subsequent work.

After activation, current native server, health and token-issuance commands
refuse the directory even when no workspace process is running. Removing only
the separate mode ledger cannot downgrade the main database's workspace marker.
There is no in-place deactivation API. Rollback uses the complete pre-activation
backup with its producing binary after every server process has stopped.

## Local maintenance and lock lifetime

`DirectoryLease::maintenance` obtains exclusive access for local
provisioning/reset operations. It preserves workspace activation and does not
grant an administrative role, clear grants or reset enrollment. These methods
must never be exposed as unauthenticated remote operations.

Cloned leases share one held connection. Locks release when the last relevant
owner exits, including a background writer, or when its process dies. There is
no PID-file override or clock-based lease expiry. Separate data directories are
independent. Contention fails promptly with an in-use error; unexpected I/O,
corrupt metadata, unsupported versions and journal formats fail closed.

The private `ccp-directory-mode.sqlite3` ledger has its own format version 1.
It uses the bundled SQLite VFS's shared/exclusive kernel locks. Native leases
retain a normal read transaction. Workspace/maintenance connections acquire an
exclusive transaction before reading metadata and retain exclusive locking mode
across the durable commit. These mechanics follow SQLite's
[locking-mode](https://www.sqlite.org/pragma.html#pragma_locking_mode) and
[file-locking](https://www.sqlite.org/lockingv3.html) contracts. Use a filesystem
that supports SQLite's locking requirements.

Low-level trusted storage APIs remain building blocks; they are not transport
or mode authentication. Future runtimes must retain the appropriate lease for
all reads, writes and tasks. Client/admin permission checks remain separate.

## Storage and compatibility

Schema 12 adds the main database's `ccp_runtime_mode` marker. The separate ledger
checks its application ID, format, row and rollback-journal mode. Neither unknown
nor corrupt nonempty ledgers are silently replaced. Schema-11 identity, key and
grant records are preserved; the
[foundation backup and migration rules](workspace-foundation.md#database-upgrade)
apply. Backups must include the complete data directory, including both mode
records, database journals, legacy journal and protected key material.

Binaries from before this mode-guard change do not participate in its locking
protocol. Supported earlier foundation binaries reject the newer main schema,
but the old stable schema-2 binary does not enforce that newer-version refusal.
Host operators must stop older binaries and use matching backup/version
procedures. This mechanism does not protect against a trusted host operator
modifying or replacing the data files.

Tests cover real separate-process shared/exclusive exclusion, process kill and
restart, durable markers, missing ledger and interrupted second commit, audit
rollback, schema-11 identity preservation, aliases, contention races and journal
lifetime. Child-process tests exercise actual native runtime, initializer, health,
token and discovery entry points against protected directories with unreadable
native-format sentinels. Workspace HTTP and new content flows remain
unverified until implemented. Local setup/reset has separate command tests.
