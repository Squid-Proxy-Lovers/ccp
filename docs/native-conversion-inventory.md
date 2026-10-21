# Inspect a native database before conversion

Unreleased local command. This is an **inventory**, not a converter or approval
to activate workspace mode. It neither assigns ownership nor grants access.

Stop the producing native server and its tools cleanly first, and preserve the
complete matching data directory and producing binaries. Then use an absolute
path:

```bash
server legacy-inspect --data-dir /absolute/path/to/stopped-native-data
```

The command emits one JSON object with format version 1, the observed schema,
database byte length and SHA-256, the stored data mode, and aggregate native and
workspace record counts. It does not emit entry bodies, names, labels, historical
client common names, certificates, token values or key material. Existing
workspace counts are reported separately; they are not inferred native grants.
`observation_only` is true and `conversion_performed` is false.

Counts cover native sessions, physical shelves/books, active entries, entry
archives, current/archived append records, native certificates, tokens and
transfer records. Counts include records absent from the native memory cache;
expired/revoked security records are not described as usable credentials.

## Source preservation

Inspection never initializes or opens the original database through SQLite,
starts native recovery, creates a directory-mode ledger, changes a mode marker,
replays/truncates a journal, generates keys or enrolls clients. It streams the
main database into a temporary inspection directory, hashes its exact bytes,
queries only that copy, then rechecks the source bytes and sidecar/journal state
before returning an observation. Temporary copies use 0700 directories and 0600
files on Unix under the system temporary directory (`$TMPDIR` on macOS), in
`ccp-native-inspection-*` directories. Normal returns remove these copies. An
interrupted or killed process can leave a copy, including native tokens and
certificate records; remove that run's directory manually after the process has
stopped. This is logical cleanup, not forensic secure erasure; Windows ACL
behaviour has not been verified.

The command refuses:

- A missing, relative, nonregular or final-symlink source directory/database.
- Any SQLite `ccp.sqlite3-wal`, `ccp.sqlite3-shm` or `ccp.sqlite3-journal` file,
  including an empty sidecar.
- A nonempty, nonregular or symlink `runtime-journal.jsonl`.
- An invalid SQLite header, failed integrity/foreign-key check, or missing
  required native tables/columns.
- Historical HTTP holding schemas 3/4 and formats newer than this server supports.
- A database over this inspector’s **2 GiB copy limit**. This is an inspection
  resource bound, not a limit of native storage.
- Source-byte changes or new journal/sidecar work observed during inspection.

Inspection refusals emit no inventory and do not repair the source. Recover a pending
journal using its matching producing software, then stop it cleanly; never delete
journals or WAL files just to pass inspection. Keep all source files and keys
with their producing binary. See the [backup and activation boundary](workspace-runtime-mode.md#explicit-workspace-activation).

## What the observation does not establish

An empty journal and absent sidecars do not prove that the old producer stopped,
that missing recovery files never existed, or that a future writer cannot change
the source. Accepted native-main binaries do not honor the new directory lease.
The fingerprint describes the bytes inspected; it is not a lock or authorization
token. Any later conversion must revalidate its source under its own explicit
offline operation.

The schema gate admits native 0–2 and this server’s workspace schema line, but the
full required native layout must also exist. No migration fills missing columns,
so older layouts can be refused. This does not establish tested conversion support
for every admitted schema, validate every value against workspace field bounds,
parse historical timestamp strings or reconstruct missing creation history.

Native creator identities were not recorded as registered workspace clients.
Owner/CN/agent labels must not become owners or grants merely because they match
names. The later converter must preserve raw history, archives and provenance
without inventing ownership or earlier full snapshots. Actual conversion remains
unimplemented; existing legacy-linked workspace content remains guarded. See the
[workspace content boundary](workspace-content.md#schema-and-legacy-data).
