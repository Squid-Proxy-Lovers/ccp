# Select workspaces for one caller

This unreleased client stores explicit caller preferences separately from its
credentials. A selection contains workspace UUIDs and an optional default for one
verified instance and registered client. Granting access, joining, renewing or
recovering does not select another workspace automatically.

[Documentation index](README.md) · [Workspace client setup](workspace-client.md)

## Create an empty caller selection

Use an enrolled profile and choose a separate private directory for each caller.
Callers intentionally using the same directory share its choices. No implicit
global directory or default is used.

```bash
ccp-client workspace selection --profile "$HOME/.ccp-client/workspaces/office" \
  init --selection-dir "$HOME/.ccp-client/callers/research"
ccp-client workspace selection --profile "$HOME/.ccp-client/workspaces/office" \
  show --selection-dir "$HOME/.ccp-client/callers/research"
```

Initialization starts at revision 0 with no selected workspaces or default.
It refuses to replace an existing selection. `show` reads local preferences and
does not establish current permission. It works with expired credentials because
it does not contact the server.

## Replace the selected set deliberately

Use UUIDs from authenticated workspace discovery. Names are display text and may
change. Replace the example UUIDs and expected revision with the actual values:

```bash
ccp-client workspace selection --profile "$HOME/.ccp-client/workspaces/office" \
  set --selection-dir "$HOME/.ccp-client/callers/research" \
  --expected-revision 0 \
  --workspace-id 00000000-0000-4000-8000-000000000001 \
  --workspace-id 00000000-0000-4000-8000-000000000002 \
  --default-workspace-id 00000000-0000-4000-8000-000000000001
```

Every set replaces the entire selected set and default. Omitting the default
leaves it unset; the client never picks the first workspace implicitly. A default
must also appear in the selected set. UUIDs must be canonical and unique, with
at most 100 selected workspaces. The client checks current authenticated discovery
before saving nonempty choices. Another caller's selection stays unchanged.

A successful set increments the revision. If another invocation changed this
directory, the expected revision fails. Inspect the current choices and decide
the next update instead of overwriting them with a stale command.

Simultaneous updates can also fail busy while acquiring the exclusive SQLite
lease, which waits up to two seconds. Inspect the saved choices before retrying:
another invocation may already have committed. Lock contention does not guarantee
a revision-conflict error; both failures preserve the newer saved choices.

To clear the set and default, run `set` with the current expected revision and
no workspace flags. Clearing local preferences is available offline, including
after server-side disabling; it never changes server grants.

## Resolve current targets

```bash
ccp-client workspace selection --profile "$HOME/.ccp-client/workspaces/office" \
  resolve --selection-dir "$HOME/.ccp-client/callers/research"
ccp-client workspace selection --profile "$HOME/.ccp-client/workspaces/office" \
  target --selection-dir "$HOME/.ccp-client/callers/research"
ccp-client workspace selection --profile "$HOME/.ccp-client/workspaces/office" \
  target --workspace-id 00000000-0000-4000-8000-000000000002
```

`resolve` returns current authorized metadata for every selected workspace, in
saved order. An empty set or any unavailable target fails the whole resolution;
there is no silently incomplete result. `target` returns exactly one explicit
UUID, or the configured default when no UUID is supplied. An explicit target
does not read or rewrite caller preferences and needs no selection directory.
An unset or unavailable default fails without choosing a different workspace.

Resolution uses current mTLS discovery and credential restrictions. Revocation,
purging or global disabling does not erase preferences, but can make their
resolution fail. Renaming keeps UUID-based choices intact. Archived workspaces
remain readable when authorized. Returned metadata is a discovery snapshot;
future content requests still need the server's current permission checks.
These commands do not read or write context contents.

Each operation uses the choice snapshot read at its start. Discovery scans at
most 20 pages of 500 workspaces, stops when every requested UUID is found, and
rejects repeated cursors or exhaustion. A failure does not redirect the request
or change choices. An individual request has the existing client time/body bounds;
the page limit bounds the number of requests, rather than a single total deadline.

## Credentials, recovery and storage

Selection is bound to instance and client UUIDs, not a mutable profile name or
stored role. A profile for another instance or registered client is refused.
Renewing the same registration preserves selection. Scoped recovery may leave
A in a new profile and B in the original profile. Choose a credential profile
that authorizes the target; an A-only profile cannot resolve B merely because
B appears in saved preferences. Profiles are not merged automatically.

`caller-selection.sqlite3` uses a separate application ID and format 1. It stores
IDs, a default and a revision, with no credentials, tokens, server roles or
profile paths. Private atomic publication, DELETE journaling, FULL synchronization,
exclusive leases, revision checks and no-create opens protect local updates.
Selections keep format 1 independently of the credential profile, now
[format 5](workspace-client.md#profile-storage-and-recovery-limits).
Missing, foreign, future, corrupt, symlinked or loosely permissioned final state
is refused under the same host/Unix/Windows limits as the
[profile storage contract](workspace-client.md#profile-storage-and-recovery-limits).

Keep caller directories private and back them up only with clients stopped.
No automatic repair, old-state fallback or preference import is performed.
The [workspace MCP bridge](workspace-mcp.md) uses explicit selection handles for
callers, including sessionless requests. Context content operations remain later
work; the supported native MCP path keeps its current behavior.
