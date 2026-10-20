# Connect workspace discovery and selection to MCP

This unreleased stdio bridge connects an agent host to configured workspace
client profiles. It exposes current workspace metadata and explicit caller
selections. Workspace content tools, boards, status and the browser panel remain
later work. The supported native `ccp-mcp-server` entry point stays available.

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

The bridge runs at most eight Rust children, waits at most two seconds for a
process slot, and gives each child 60 seconds and 1 MiB of stdout. Requests using
one handle/alias serialize. Timeout, cancellation or excess output kills and
reaps the child. CLI stderr and malformed output are not echoed through MCP;
use the CLI for detailed local errors. A timed-out renewal may have committed on
the server: preserve the profile and let the client's exact retry recover it.
The underlying target resolution also has its documented page bounds.

These tools return metadata and preferences. They do not read or write workspace
entries, publish guidance, manage tasks or start a server.
