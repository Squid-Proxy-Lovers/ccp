# Read workspace context through MCP

This unreleased stdio bridge connects an agent host to configured workspace
client profiles. It exposes current workspace metadata and explicit caller
selections, shelves, books, entries and revision history, plus shelf/book creation.
It also exposes durable entry mutations and guarded outcome controls.
Current guidance can also be refreshed explicitly. Automatic guidance delivery,
status and the browser panel remain later work. The supported native
`ccp-mcp-server` entry point stays available.

[Documentation index](README.md) · [Workspace client](workspace-client.md) ·
[Selection contract](workspace-selection.md)

## Configure the local bridge

Create and enroll or recover the private client profiles first. Use matching
workspace client and MCP builds. From a development checkout, install the package
with `python -m pip install -e ./mcp`; it provides `ccp-workspace-mcp` alongside the
native command.

Create an owner-only JSON configuration file. Use absolute paths and aliases
containing letters, numbers, underscores or hyphens:

```json
{
  "format": 1,
  "profiles": {
    "office": "/absolute/path/to/private/original-profile",
    "office-recovered": "/absolute/path/to/private/recovered-profile"
  }
}
```

On Unix, set the configuration file to mode `0600`. It must be a regular file,
not a symlink, at most 64 KiB, with one to 32 aliases. Duplicate or unknown JSON
fields and unsupported formats are refused. Configuration is fixed for the bridge
process; restart to change it. The profile paths remain local and are not returned
by the tools.

Configure only profiles this MCP host may use. Every participant using this local
bridge can invoke its configured aliases; selection handles do not create
separate permission roles.

Configure your MCP host with the installed command and two explicit environment
variables. Replace every example path with its actual absolute path:

```json
{
  "mcpServers": {
    "ccp-workspace": {
      "command": "/absolute/path/to/ccp-workspace-mcp",
      "env": {
        "CCP_WORKSPACE_CONFIG": "/absolute/path/to/workspace-mcp.json",
        "CCP_CLIENT_BIN": "/absolute/path/to/ccp-client"
      }
    }
  }
}
```

The bridge runs over local stdio. It has no HTTP MCP listener, automatic binary
discovery, source build, enrollment or administrative tools. Tool arguments cannot
choose local paths or executables. The Rust client uses the profile's pinned
CA/name/instance and actual mTLS credential; it never falls back to native access.

## Keep an explicit selection handle

Call `workspace_selection_open` with no arguments. It returns a new random
`selection_id`. Retain that ID for subsequent selection calls. Each agent can
open an independent handle even when the host shares one MCP connection; sharing
a handle deliberately shares its preferences. The ID identifies local choices,
not a person, agent, registered client or server permission.

Selections begin empty for each handle/profile alias. They do not import saved
CLI choices, pick a default at enrollment, or add newly granted workspaces.
Calling `workspace_list` discovers access without changing any choices.

| Tool | Arguments and result |
| --- | --- |
| `workspace_profiles` | No arguments; returns configured aliases without paths |
| `workspace_profile` | `profile`; returns public local trust/identity/expiry metadata, not current authority |
| `workspace_list` | `profile`, optional `limit` (1–500) and `after`; returns current accessible metadata and cursor |
| `workspace_selection_open` | No arguments; returns a new `selection_id` |
| `workspace_selection` | `profile`, `selection_id`; returns choices and their revision, initially 0 |
| `workspace_select` | `profile`, `selection_id`, `choice`; replaces the whole set/default after current discovery |
| `workspace_resolve` | `profile`, `selection_id`; returns all selected targets or an error |
| `workspace_target` | `profile` and either explicit `workspace_id` or `selection_id` for its default; returns one current target |

For `workspace_select`, the nested choice has this shape. Replace the workspace
UUIDs with accessible IDs, use the handle issued by the bridge, and inspect the
current revision before updating:

```json
{
  "profile": "office",
  "selection_id": "00000000-0000-4000-8000-000000000001",
  "choice": {
    "workspace_ids": ["00000000-0000-4000-8000-000000000002"],
    "default_workspace_id": "00000000-0000-4000-8000-000000000002",
    "expected_revision": 0
  }
}
```

A default must be in the selected set; omitting it leaves it unset. A successful
update increments the revision. A stale revision fails instead of overwriting
newer choices. An empty set clears preferences locally. An explicit target does
not need a selection handle and never changes any handle's choices.

## Refresh guidance explicitly

Call `workspace_guidance` with a configured profile alias. Omit `scope` for global
guidance only; omission never uses the selected set or default:

```json
{"profile":"office"}
```

Supply one explicit workspace UUID, or a bridge-issued handle for its default:

```json
{"profile":"office","scope":{"workspace_id":"11111111-1111-4111-8111-111111111111"}}
```

```json
{"profile":"office","scope":{"selection_id":"00000000-0000-4000-8000-000000000001"}}
```

Replace illustrative UUIDs with an accessible workspace or your issued handle.
If both are supplied, the explicit UUID takes precedence without changing choices.
A missing handle default is an error, with no fallback. The resolved workspace
is captured once for the request. New grants are never selected automatically.

The result contains `precedence: "global_first"`, a `global` board and optional
`workspace` board, with separate source, revision, text and historical publisher
key/principal/time. Global CCP guidance wins workspace conflicts; host instructions
and actual server permissions retain priority. Text cannot grant authority or
change the structural source labels. Revision 0 with no publisher is unpublished;
positive empty text is an explicitly cleared board. Controls and UTF-8 text are
preserved up to 8192 bytes per board and returned as JSON data.

The configured Rust client authenticates through pinned mTLS and rechecks current
workspace access. The bridge validates known metadata before returning it, drops
additive bundle/board/publisher fields, and refuses malformed or unexpected scope
without echoing the prose. Unknown request fields and identity claims are refused
before SDK parsing. Archived workspace guidance remains readable; recovery,
revocation and instance-wide disable do not widen or preserve stale authority.
See the [client guidance contract](workspace-client.md#read-current-guidance) for
renewal and pending-operation behavior. Reads preserve caller choices and pending
intents; authorized renewal may update the credential.

This tool performs an explicit current read with no delivery cache or emission
cursor. Automatic initial/changed guidance attached to other tool responses and
stale/unavailable sideband warnings remain subsequent work. A failed explicit
read remains an error; guidance publication uses separate scoped admin APIs.

## Read content in one workspace

The nine content readers take `profile` and a nested `scope`. Supply either
`scope.workspace_id` explicitly or `scope.selection_id` to use that handle's
default. An explicit workspace takes precedence when both are present; the
handle's choices are not consulted or changed. Missing/unknown handles and absent
defaults fail. Each call captures one target before reading, checks current
access again through the Rust client, and returns `workspace_id` plus `data`.
Even empty pages and history are labeled with the resolved workspace UUID.

| Tool | Other arguments; result in `data` |
| --- | --- |
| `workspace_shelves` | Optional `query: {after, limit}`; active shelf page |
| `workspace_archived_shelves` | Optional `query: {after, limit}`; safe archived shelf summaries |
| `workspace_shelf_state` | `node_id`; current shelf lifecycle revision and archive state |
| `workspace_books` | `query: {shelf_id, after?, limit?}`; book page |
| `workspace_shelf` | `node_id`; shelf metadata |
| `workspace_book` | `node_id`; book metadata and parent |
| `workspace_entries` | `query: {book_id, archived?, after?, limit?}`; entry summaries |
| `workspace_entry` | `query: {entry_id, revision?}`; exact current or historical snapshot |
| `workspace_history` | `query: {entry_id, after?, limit?}`; revision metadata without bodies |

For example, call `workspace_entry` using the handle you opened and an actual
entry UUID. These identifiers are illustrative:

```json
{
  "profile": "office",
  "scope": {
    "selection_id": "00000000-0000-4000-8000-000000000001"
  },
  "query": {
    "entry_id": "00000000-0000-4000-8000-000000000003",
    "revision": 2
  }
}
```

Use `scope: {"workspace_id": "YOUR_WORKSPACE_UUID"}` for an explicit target
without a selection. It may be accessible without being selected. No read adds
new grants to selections, changes a default, aggregates workspaces or tries a
different alias after denial. The bridge never combines recovery-profile grants.

Shelf lifecycle readers use the same resolved workspace and current permissions.
Archived shelf summaries expose only IDs, name, lifecycle revision and creation
time, with no descriptions or descendant content. Archived shelves disappear from
active shelf lists; their body/history reads remain unavailable until restoration.
The Rust client validates known response fields and drops unknown additions
before returning typed JSON. Lifecycle metadata does not grant authority to
archive or restore; those operations remain on the scoped admin HTTPS API.

Pages default to 32 items and accept 1–32. Follow `data.next_cursor` as `query.after`;
history uses positive revision numbers, other pages use canonical lowercase
hyphenated UUIDs. `revision` and history cursors must fit a positive signed64-bit
integer. Omit the entry revision for its current version. A historical snapshot
separates current entry metadata from the requested revision and exact text,
description, labels and original append contribution.

`archived: true` lists summaries without text/descriptions/labels. Full entry,
historical-body and history reads fail while that entry is archived. An archived
workspace remains readable to authorized clients. Current revocation and global
disable still apply. The [client content guide](workspace-content-client.md)
explains response validation, bounded snapshots and renewal; creation and durable
entry writes and outcome controls are described below. The creation tools
prepare shelves and books.

## Create shelves and books

The two creation tools take `profile` and a typed `request`. Every write names its
workspace UUID explicitly. To use a selected default, first call `workspace_target`
and retain the returned UUID in the request. A later default change cannot retarget
that request. Writes do not fan out, modify preferences or borrow another alias's
authority.

| Tool | Fields inside `request` |
| --- | --- |
| `workspace_create_shelf` | `workspace_id`, `name`, optional `description` |
| `workspace_create_book` | `workspace_id`, `shelf_id`, `name`, optional `description` |

For example, call `workspace_create_shelf` after replacing the illustrative UUID:

```json
{
  "profile": "office",
  "request": {
    "workspace_id": "00000000-0000-4000-8000-000000000002",
    "name": "notes",
    "description": "Research notes"
  }
}
```

Creation returns `workspace_id` and `data` containing the new node's metadata.
Names are trimmed, nonempty and at most 256 UTF-8 bytes without control characters;
descriptions are at most 4096 bytes and preserve text. Encoded requests are limited
to 64 KiB. Unknown fields, identity claims and implicit workspace targets are
rejected. Creation arguments are checked before SDK parsing, and invalid requests
use static errors rather than echoing rejected values. The Rust client enforces its current authority and pending-operation
guards, including a pending entry intent or credential admission. A pending
renewal can resume before creation. See [client creation](workspace-content-client.md#create-shelves-and-books).

Each call sends one creation POST. There is no operation receipt, saved creation
intent, automatic retry or automatic claim of an existing name. A duplicate
conflicts. If the result is uncertain, inspect `workspace_shelves` or
`workspace_books` under the original explicit workspace and parent before deciding
to retry. Use `workspace_profile` to inspect pending-entry metadata when a shared
profile blocks a new operation. Successful node metadata includes descriptions;
errors and partial stdout do not expose payloads, child stderr or private paths.

## Edit entries and recover outcomes

Use matching client and MCP builds. These four tools share the client's
[durable entry-intent contract](workspace-content-mutations.md). Every mutation
names an explicit workspace and caller-chosen operation UUID. Retain that UUID,
request and expected revision for retries; the bridge never invents replacements.
An `agent` name is attribution; the registered client owns the entry.

| Tool | Arguments and result |
| --- | --- |
| `workspace_mutate_entry` | `profile`, typed `request`; returns workspace, receipt in `data`, ready-to-copy `target` |
| `workspace_entry_pending` | `profile`; returns body-free `pending`, `last` and a `target` for pending, otherwise the most recent terminal intent |
| `workspace_entry_resume` | `profile`, original `target`; retries/resolves only that saved binding |
| `workspace_entry_resolve` | `profile`, original `target`, optional configured `via_profile` alias; proves commit or cancels an uncommitted intent |

For example, call `workspace_mutate_entry` with your actual identifiers and a
UUID chosen once for this append. These identifiers are illustrative:

```json
{
  "profile": "office",
  "request": {
    "workspace_id": "00000000-0000-4000-8000-000000000002",
    "operation_id": "00000000-0000-4000-8000-000000000004",
    "agent": "research agent",
    "change": {
      "action": "append",
      "entry_id": "00000000-0000-4000-8000-000000000003",
      "expected_revision": 2,
      "text": "\nAdditional finding."
    }
  }
}
```

`change.action` selects `create`, `append`, `revise`, `archive` or `restore`.
Fields match the [action table](workspace-content-mutations.md#choose-an-action).
Create/revise content requires `text`; omitted `description` and `labels` become
empty. Append text must be nonempty. Text is at most 256 KiB UTF-8, descriptions
4096 bytes, at most 32 distinct labels of 128 bytes without control characters,
and agent/name limits are 256 bytes. Encoded entry requests are at most 2 MiB.
Unknown fields and identity claims fail before SDK parsing or file staging.
The client saves the full original intent before POST and checks current permissions.

Use the returned `target` unchanged with resume/resolve. Its four fields are
`workspace_id`, `operation_id`, original `credential_id` and `request_hash`.
They identify the saved intent and do not authenticate it. The Rust client checks
all four under the same exclusive profile lease used by the operation, before
network requests, renewal or opening another recovery profile. If another intent
has replaced it in a shared profile, the call fails. No unguarded fallback is used.
Check that inspected metadata's operation UUID matches your intended request.

`pending`/`last` expose no body or key and do not prove current authority. The
profile retains one pending body and one most recent terminal binding. Older
bindings cannot be controlled through this cache after replacement. A cancelled
intent is terminal; inspect current state before deliberately choosing another
UUID/revision. Failures never imply a successful replacement or task completion.

If the original credential expires or loses its grant, resolve through a separate
configured recovery alias with `via_profile`. The original key and unrelated grants
stay separate. The replacement must be the same registered client and pinned
instance/CA with current read access; global disable still wins. The guard is
checked before the replacement is opened. See [recovery](workspace-recovery.md).

Pending/profile inspection remains available offline. Mutation/resume errors
return static guidance without partial receipts or body fragments. Preserve the
original intent and inspect metadata rather than blindly choosing a new nonce or
expected revision. Temporary input files are cleaned independently of the durable
Rust pending record; a failed local outcome save or lost MCP response can be
recovered with the exact returned/inspected target.

## Scope, lifetime and failures

The server still checks current credential bindings, grants, workspace lifecycle
and global client disabling. An unavailable selected target fails resolution
without a partial result or fallback. Scoped recovery of A does not grant a
recovered profile access to B: choose the original B profile alias explicitly.
Aliases have separate choices within one handle and are not merged automatically.

Handles last for this bridge process, including across sessionless MCP requests.
Reconnecting to a surviving process can keep a known handle. Restarting the bridge
invalidates all old handles; open a fresh one and select deliberately. Unknown
handles fail without creating state. There are at most 64 handles per process.
The bridge does not infer caller identity from SDK session objects or agent labels.
If the first selection initialization is interrupted after its file commits,
that handle/alias may require a fresh handle; the credential profile stays intact.

Generated private caller directories contain preferences only. Normal shutdown
removes them; an abrupt stop may leave a private directory that is never selected
or resumed automatically. Existing credential profiles are retained. Unix
permissions and the [profile storage limits](workspace-client.md#profile-storage-and-recovery-limits)
apply; Windows ACL and directory durability are not independently verified here.

Content payloads travel through bridge-generated private files, never argv or
tool-selected paths. A process slot is acquired before staging, limiting active
request files to eight. On Unix the runtime directory is private and request files
use mode `0600`. Files are closed before the Rust child opens them and removed
after it exits or is killed and reaped, including cancellation during launch.
An abrupt host stop or cleanup failure can leave private request files; they are
never automatically resumed. Clearing a file does not promise secure erasure of
disk pages or backups. The Rust profile and its durable entry intent are separate
and are never removed with these temporary files.

The bridge runs at most eight Rust children, waits at most two seconds for a
process slot, and gives each child 60 seconds. Only the fixed entry-snapshot
command gets 4 MiB of stdout, preserving escaped text and append contributions;
all other commands retain 1 MiB. There is no tool argument to raise these limits.
Selection operations using one handle/alias serialize. A content read captures
the default and then reads that explicit target; a subsequent selection change
does not retarget an in-flight read. Timeout, cancellation or excess output kills and
reaps the child. CLI stderr and malformed output are not echoed through MCP;
use the CLI for detailed local errors. A timed-out renewal may have committed on
the server: preserve the profile and let the client's exact retry recover it.
The underlying target resolution also has its documented page bounds.

The bridge exposes 24 tools: eight metadata/selection tools, one guidance reader,
nine content reads,
two container creations and four entry-intent/outcome tools. It does not publish
guidance, manage tasks or start a server.
