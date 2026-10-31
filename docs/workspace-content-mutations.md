# Edit workspace entries with the client

Use an enrolled [workspace profile](workspace-client.md), an explicit workspace
UUID and a book or entry UUID from the [content reads](workspace-content-client.md).
These unreleased library and CLI operations create, append, revise, archive and
restore entries through the [content HTTPS API](workspace-content-http.md).
Use [shelf/book creation](workspace-content-client.md#create-shelves-and-books)
to prepare the hierarchy. The [workspace MCP bridge](workspace-mcp.md) exposes
content reads, shelf/book creation and [entry-intent/outcome tools](workspace-mcp.md#edit-entries-and-recover-outcomes).

[Documentation index](README.md) · [Ownership and revisions](workspace-content.md) ·
[Server operation resolution](workspace-operation-resolution.md)

## Save and submit one intent

Choose a fresh operation UUID once for the intended change. Keep that UUID and
the exact request file for retries. UUIDs below are illustrative; replace them
with your actual targets and chosen operation UUID. Run from an installed client
or replace `ccp-client` with `target/release/client` in a checkout.

```bash
umask 077
cat > entry-change.json <<'JSON'
{
  "workspace_id": "11111111-1111-4111-8111-111111111111",
  "operation_id": "55555555-5555-4555-8555-555555555555",
  "agent": "research agent",
  "change": {
    "action": "append",
    "entry_id": "44444444-4444-4444-8444-444444444444",
    "expected_revision": 2,
    "text": "\nAdditional finding."
  }
}
JSON
ccp-client workspace content --profile ./my-profile mutate \
  --request-file ./entry-change.json
```

The private UTF-8 JSON file must be a regular file, without a symlink, no larger
than 2 MiB. Unix permissions must exclude group and other access. Fields are
strict: no certificate, client-ID or admin-key claims are accepted in the request.
All UUIDs use canonical lowercase hyphenated form; the operation UUID cannot be nil.
The optional `agent` is attribution and does not grant authority.

The result contains `entry_id`, `revision` and body-free `operation` metadata:
instance, registered client, original credential, workspace, operation UUID,
request hash and terminal state. Save this metadata with the request when useful.
Failures produce no partial JSON on stdout and do not echo remote entry bodies.

## Choose an action

Use the same top-level envelope and replace `change` with the action's fields.

| Action | Fields in `change` besides `action` |
| --- | --- |
| `create` | `book_id`, `name`, `content` |
| `append` | `entry_id`, `expected_revision`, `text` |
| `revise` | `entry_id`, `expected_revision`, `content` |
| `archive` | `entry_id`, `expected_revision` |
| `restore` | `entry_id`, `expected_revision` |

`content` has `text`, `description` and `labels`, for example:

```json
{
  "action": "create",
  "book_id": "33333333-3333-4333-8333-333333333333",
  "name": "findings",
  "content": {
    "text": "Initial finding.",
    "description": "Research notes",
    "labels": ["research"]
  }
}
```

Text is at most 256 KiB of UTF-8, description at most 4096 bytes, and labels at
most 32 distinct nonempty strings of at most 128 bytes without control characters.
Append text must be nonempty. Entry names have a nonempty trimmed value of at
most 256 bytes without control characters. Agent names are nonempty, at most
256 bytes and contain no control characters. Text itself preserves Unicode,
whitespace and control characters exactly within the bounds.

Creation returns revision 1. Every other action requires the current positive
`expected_revision` and advances it exactly once. A stale revision fails; inspect
the latest entry before deliberately preparing a new intent. The client never
guesses the next revision or substitutes a new operation UUID.

Current writers can create entries and append to other clients' entries.
Ordinary writers revise, archive and restore only entries owned by their
registered client. Workspace owners/admins have the agreed scoped overrides.
Ownership and history remain intact. Global admin keys do not become workspace
writers. Archived workspaces permit reads and refuse content writes; archived
entries expose summaries until an authorized restore.

## Resume or resolve an uncertain result

The client saves the complete original request and credential binding before
posting a mutation. It holds the private profile lease throughout. A lost reply
triggers body-free server resolution: the server either proves the original
commit or cancels an uncommitted intent against a delayed write. If it cannot
prove either outcome, the client keeps the pending request.

```bash
ccp-client workspace content --profile ./my-profile pending
ccp-client workspace content --profile ./my-profile resume
ccp-client workspace content --profile ./my-profile resolve
```

`pending` prints metadata or null. `resume` resends the exact pending intent
under its original credential, then resolves an unsuccessful response.
`resolve` requests only its outcome and can cancel an intent that never committed.
These controls do not accept a new body, revision or nonce. A cancellation or
binding conflict is terminal locally and returns an error from mutation/resume;
inspect current state before explicitly deciding on another operation. Network,
authorization, malformed-response and persistence failures retain uncertain state.

For shared-profile automation, `resume` and `resolve` also accept an optional
expected binding. Supply all four flags together: `--workspace-id`,
`--operation-id`, `--credential-id`, `--request-hash`. The values come from the
original operation metadata. They are checked against the current pending or
most recent terminal binding while the profile lease is held, before any request
or recovery-profile opening. A stale/mismatched or partial guard fails. Existing
operator invocations without these flags keep their current profile-slot behaviour;
the MCP controls always supply a complete guard and never fall back without it.

While content is pending, normal authorized reads remain available. Automatic
renewal pauses, and explicit renewal, admission and other credential transitions
are blocked. Before a new intent, the client performs an authorized read preflight
and any routine due renewal, then captures its original credential. A pending
credential renewal resumes first; other pending credential operations must finish.
Resolution of the most recent terminal intent also resumes an already pending
credential renewal; it does not initiate a new routine renewal.

If the original credential expires or loses access, use a separately recovered
profile for the same registered client and pinned instance/CA:

```bash
ccp-client workspace content --profile ./my-profile resolve \
  --via-profile ./recovered-profile
```

Only resolution uses the replacement credential. The original key and other
workspace permissions remain separate. The replacement must currently have read
access to this workspace; recovery does not restore revoked grants or bypass a
global disable. A different client or trust pin fails. See [client recovery](workspace-recovery.md).

## Local retention and library use

Profile format 5 stores one pending full entry request and one most recent
terminal entry binding/result, with separate status metadata. Pending entry,
status and credential lifecycle operations exclude one another. It is not a cumulative operation archive. An exact retry of the
most recent request resolves its saved original binding even after renewal.
An older operation UUID that has fallen out of this local cache can conflict
after credential rotation; the client never silently gives it a fresh UUID.

The file is private but not encrypted. Clearing pending removes its live record;
SQLite free pages, request files and backups may retain text. No secure-erasure
claim is made. The profile file limit is 16 MiB, each saved pending record is
bounded to 2 MiB plus 4 KiB of metadata, and the last result to 4 KiB.
[Profile upgrades and backups](workspace-client.md#profile-storage-and-recovery-limits)
explain migration of formats 1–4 and refusal by older clients.

Rust callers use `WorkspaceProfile::mutate_entry(EntryMutation)`,
`resume_entry_operation`, `resolve_entry_operation` and
`resolve_entry_operation_via(&mut other_profile)`. Types are exported from
`client::workspace::content`; `pending_entry_operation` and `info` expose safe
metadata. Keep the profile open until the operation completes. Caller selections,
API version 1 and the native protocol stay unchanged. Server schema 19 retains
the schema-17 cancellation fences and schema-18 shelf lifecycle records, while
making body-free entry receipts independent of payload versions. Receipts stay
historical proof and do not establish current content availability.
