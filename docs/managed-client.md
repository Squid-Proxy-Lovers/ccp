# Stage and activate a managed client bundle

These unreleased commands keep a client executable and its MCP environment in one
private generation. Manual staging, activation, rollback and pinned launch are
implemented. A separate [read-only client preflight](workspace-client.md#inspect-update-compatibility-without-changing-a-profile)
can inspect one profile and its pinned server. Download discovery, local opt-in
automatic checks and activation policy remain subsequent work.

[Documentation index](README.md) · [Release bundle verification](client-release-bundles.md) ·
[Workspace MCP](workspace-mcp.md)

## Prepare a candidate

Use Linux or macOS, Python 3.10+ and GitHub CLI with the required provenance flags.
Install the matching checkout's MCP package into a separate bootstrap environment:

```bash
python3 -m venv ./ccp-bootstrap
./ccp-bootstrap/bin/python -m pip install ./mcp
```

Keep that bootstrap environment available. Routine bundle activation does not
upgrade the bootstrap manager itself. Obtain the immutable official release's
files as described in [bundle verification](client-release-bundles.md). Older
releases without the required manifest/provenance cannot be staged. No production
release with these new components is claimed available yet.

Choose an absolute store directory. The manager requires owner-only POSIX
permissions and rejects symlink metadata. Launch and status use shared read locks,
so simultaneous readers can proceed. Staging, activation and rollback use an
exclusive nonblocking lock. A conflicting operation returns a busy error without
changing the selected generation. Staging holds that lock throughout the build,
including pip installation, and temporarily blocks launches and status.

Only a writer initializes a store. Launch and status refuse an uninitialized store
without creating directories, lock files or activation metadata. Missing activation
metadata beside existing generations is also refused; retain those generations.

```bash
STORE="$HOME/.local/share/ccp-managed"
RELEASE_TAG="YOUR_PUBLISHED_TAG"
./ccp-bootstrap/bin/ccp-managed --store "$STORE" stage \
  --directory ./release-stage --channel stable --tag "$RELEASE_TAG"
./ccp-bootstrap/bin/ccp-managed --store "$STORE" status
```

Replace the illustrative tag with a published stable tag, or use the exact dated
tag and `--channel nightly`. Staging determines the current host platform. It
copies bounded regular files into its own private directory and verifies those
copies before building. Changing the download folder afterward cannot replace
the installed candidate.

The candidate must declare supported native wire/API/profile contracts, and its
side-effect-free `workspace software-info` must match the manifest. The manager
extracts only bounded regular MCP files/directories under the expected archive
root, builds a separate Python environment, installs the authenticated MCP source
noneditable, and checks its package version and workspace tool registry. Dependencies
are installed by pip using the host's configured package index; their bytes are
not covered by CCP's artifact attestation. The completed environment is retained
with this generation instead of being upgraded underneath a running process.

A failed signature, hash, archive, build or health check leaves the active pointer
unchanged and removes only the new unfinished candidate. Staging performs no
profile, enrollment or server operation. A completed candidate is selected only
by an explicit activation. Interrupted unfinished directories are never selected
automatically.

## Activate with the observed revision

Use the generation UUID returned by staging and the current revision from
`status`. An empty store begins at revision 0:

```bash
GENERATION_ID="YOUR_STAGED_GENERATION_UUID"
./ccp-bootstrap/bin/ccp-managed --store "$STORE" activate \
  --generation "$GENERATION_ID" --expected-revision 0
./ccp-bootstrap/bin/ccp-managed --store "$STORE" status
```

Activation changes one atomic local pointer and increments its revision. A stale
revision cannot overwrite a different selection. Retrying the exact target after
an unconfirmed write may return the observed already-active generation without
another revision; this confirms current selection, not attribution of a past call.

If writing or synchronizing the pointer fails, its outcome may be uncertain.
Keep both ready generations and inspect `status`; do not delete the candidate or
restore a profile/database. A rename may already be visible even if its directory
sync reported an error. No power-loss guarantee is established by local tests.

This manual activation checks the candidate's declared local contracts. It does
not invoke preflight or migrate profiles. The candidate client's explicit
`workspace update-preflight --profile /absolute/profile` can inspect a profile
without migration or renewal. Automatic activation will require all configured
profiles to pass the later compatibility and channel/pin policy. No automatic mode
is enabled here.

## Launch a fixed pair

Use the same store for client commands or the workspace MCP entry:

```bash
./ccp-bootstrap/bin/ccp-managed --store "$STORE" client -- workspace software-info
CCP_WORKSPACE_CONFIG="/absolute/path/to/private/workspace-mcp.json" \
  ./ccp-bootstrap/bin/ccp-managed --store "$STORE" mcp
```

The managed MCP launcher reads the existing host configuration and pins
`CCP_CLIENT_BIN` to the selected generation's real client path, overriding a moving
or mismatched value in its environment. It executes that generation's Python in
isolated mode. The already running MCP process keeps that same client and Python
environment after another generation is activated or rolled back.

The manager does not rewrite MCP-host configuration, profile files, trust,
credentials or workspace selections. Configure the host to invoke the explicit
bootstrap manager/store path when adopting this launcher. Software installation
and authorized workspace enrollment remain separate.

## Select the previous software generation

Inspect status and use its current revision:

```bash
./ccp-bootstrap/bin/ccp-managed --store "$STORE" rollback --expected-revision 2
./ccp-bootstrap/bin/ccp-managed --store "$STORE" status
```

Rollback selects the retained previous software generation. It does not restore
profile/server data, restore revoked permissions, or restart running callers.
Repeating a stale rollback request is refused; inspect the current state first.
Only supported local software contracts can be selected. This is not proof that
an older release is still an appropriate security choice for a running instance.

Ready generations are retained, including those used by older processes. There
is no automatic pruning or uninstall command in this phase. Preserve them while
callers may still use them. A missing/corrupt current or ready record fails closed;
the manager never guesses a replacement or selects an unfinished generation.
