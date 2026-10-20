# Use workspace content with the client

Use an enrolled [workspace profile](workspace-client.md) and an explicit workspace
UUID to read shelves, books, entries and their history. These unreleased commands
use the authenticated [content HTTPS API](workspace-content-http.md). Entry
mutations have their own [client guide](workspace-content-mutations.md).
The client also creates shelves and books. The [workspace MCP bridge](workspace-mcp.md)
exposes these reads and shelf/book creation with explicit scopes. MCP entry mutations are later work.

[Documentation index](README.md) · [Caller selections](workspace-selection.md) ·
[Content ownership and revisions](workspace-content.md)

## Create shelves and books

Use an installed client or `target/release/client` from a checkout. Save a private
UTF-8 JSON request file and specify its path. UUIDs here are illustrative: replace
them with your accessible workspace and the shelf UUID returned by creation.

```bash
umask 077
cat > shelf.json <<'JSON'
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "name": "notes",
  "description": "Project notes"
}
JSON
ccp-client workspace content --profile ./my-profile create-shelf \
  --request-file ./shelf.json

cat > book.json <<'JSON'
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "shelf_id": "22222222-2222-4222-8222-222222222222",
  "name": "findings",
  "description": "Research findings"
}
JSON
ccp-client workspace content --profile ./my-profile create-book \
  --request-file ./book.json
```

Each command returns the created node's UUID, explicit workspace and parent,
normalized name, exact description and creation time. Names are trimmed, nonempty,
at most 256 UTF-8 bytes and contain no control characters. Descriptions preserve
text up to 4096 bytes. The encoded request and file are limited to 64 KiB; unknown
fields, nonprivate Unix files and symlinks fail. No client/admin identity claims
are accepted. Use the book UUID with [entry mutations](workspace-content-mutations.md).

Creation sends one POST and has no operation receipt, saved intent or automatic
retry. If a reply is lost, refused or inconsistent, inspect `shelves` in the
explicit workspace or `books` under the explicit shelf before deciding to retry.
A duplicate name conflicts and never overwrites or claims an existing node.
Failure emits no partial JSON and no remote body fragments. This workflow is
separate from durable entry mutation/resume controls.

The client rejects creation while an entry intent or admission/recovery operation
is pending. A pending renewal resumes first. An authorized parent read can perform
routine due renewal before creation; a denied read never starts it. A read-only
or archived workspace can pass the read preflight and still refuse the creation
POST. The server checks current write permissions, revocation and disable at
creation. Selections and recovery-profile authority stay separate. All formats
and dependency versions remain unchanged by these creation commands.

## Browse a workspace

Run these commands from an installed client or replace `ccp-client` with
`target/release/client` in a checkout. Replace the illustrative UUIDs with your
own. Choose a currently accessible workspace from `workspace workspaces`; a
new grant never changes any caller's selection automatically.

```bash
ccp-client workspace content --profile ./my-profile shelves \
  --workspace-id 11111111-1111-4111-8111-111111111111

ccp-client workspace content --profile ./my-profile books \
  --workspace-id 11111111-1111-4111-8111-111111111111 \
  --shelf-id 22222222-2222-4222-8222-222222222222

ccp-client workspace content --profile ./my-profile entries \
  --workspace-id 11111111-1111-4111-8111-111111111111 \
  --book-id 33333333-3333-4333-8333-333333333333
```

All content read commands require `--workspace-id`; none falls back to a saved or
shared default, edits a selection, or discovers another target after failure.
They return the underlying typed JSON data without the HTTP envelope.

| Command | Required target flags | Other flags |
| --- | --- | --- |
| `shelves` | Workspace | `--after UUID`, `--limit N` |
| `books` | Workspace, `--shelf-id UUID` | `--after UUID`, `--limit N` |
| `shelf`, `book` | Workspace, `--node-id UUID` | None |
| `entries` | Workspace, `--book-id UUID` | `--archived`, `--after UUID`, `--limit N` |
| `entry` | Workspace, `--entry-id UUID` | `--revision N` |
| `history` | Workspace, `--entry-id UUID` | `--after REVISION`, `--limit N` |

Lists default to 32 items and accept 1–32. Use `next_cursor` as `--after` until
it is null. History cursors are revision numbers; other cursors are UUIDs.
Request UUIDs must use the canonical lowercase hyphenated form.

## Read an entry or earlier version

```bash
ccp-client workspace content --profile ./my-profile entry \
  --workspace-id 11111111-1111-4111-8111-111111111111 \
  --entry-id 44444444-4444-4444-8444-444444444444 --revision 2
```

Omit `--revision` for the current version. The response separates current
`entry` metadata from the requested `revision`, `content` and original
`appended_text`. A historical version can therefore have a lower revision than
the current entry head. Revision history lists contain attribution and timestamps,
with no bodies. Original ownership and contributions survive later revisions.

Use `entries --archived` for archived summaries. These contain IDs, name, owner,
revision, status and times; no text, descriptions or labels. Full entry, version
and history reads fail while the entry is archived, for all roles. Workspace
archival still permits authorized reads. Revoked grants and disabled clients
cannot use remembered IDs or prior ownership to bypass current authorization.

## Trust, validation and renewal

The client holds its private profile lease through the request. It verifies the
saved credential, pinned CA, hostname and intended instance before posting the
content target. Reads use current mTLS authority; there are no admin keys,
proxy identities, plaintext transport or native fallback in this namespace.

Responses are checked for canonical IDs, echoed target and parent fields,
metadata bounds, archive state, revision/attribution consistency and page order.
History responses carry revision metadata and a cursor without echoed target
IDs; their workspace/entry association comes from the verified TLS request.
Snapshot reads allow up to 4 MiB of encoded JSON to preserve full escaped text
and its original append contribution. Other content responses and all existing
client response paths keep their 1 MiB limit. Invalid, oversized, truncated,
encoded or incompatible responses fail without printing remote body fragments.

A pending renewal resumes before an ordinary content request. An authorized,
validated read can trigger routine renewal when due, then repeats the read with
the new credential. Empty authorized pages count; denied requests do not start
renewal. A different pending operation suppresses automatic renewal and retains
its state. If renewal fails, the read fails and preserves the pending request
for resumption; it does not silently enroll another identity. See
[client lifecycle and recovery](workspace-client.md).

Caller-selection format 1 and native command behavior are unchanged. The client
uses [profile format 4](workspace-client.md#profile-storage-and-recovery-limits).
Pending content mutations pause automatic renewal while authorized reads remain
usable. Reads do not cache content or merge authority across recovery profiles.
Use the profile that still has access to the explicit target workspace. Command
failures return no partial content on stdout; HTTP refusals report their status.
These are localhost TLS-tested client commands, with no throughput, deployment,
Windows ACL or power-loss guarantee.
