# Stage and activate a managed client bundle

These unreleased commands keep a client executable and its MCP environment in one
private generation. They support manual staging/activation/rollback, official
release updates and deliberately enabled local automatic checks. The
[read-only client preflight](workspace-client.md#inspect-update-compatibility-without-changing-a-profile)
checks each configured profile before an update selects different software.

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
exclusive nonblocking lock for short state changes. A conflicting operation
returns a busy error without changing the selected generation. Staging releases
the global lock during download verification, pip installation and health checks,
so existing launches and status can continue while a candidate is built.

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
unchanged and removes only a definitely unfinished candidate. If its ready marker
became visible before a later error, the generation is retained. Staging performs
no profile, enrollment or server operation. Interrupted unfinished directories are
never selected automatically. Selection requires explicit activation or the
guarded update policy below.

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
without migration or renewal. The `update` command below adds all configured
profiles and channel/pin/source-order checks. Automatic checking remains off until
you explicitly enable it locally.

## Configure official updates

First stage and activate a verified bundle. Use an absolute, owner-only workspace
MCP configuration file listing every profile used with this store. Inspect policy
before editing it; an unconfigured store starts with expected revision 0:

```bash
./ccp-bootstrap/bin/ccp-managed --store "$STORE" policy-show
./ccp-bootstrap/bin/ccp-managed --store "$STORE" policy-set \
  --config "/absolute/path/to/private/workspace-mcp.json" \
  --channel stable --automatic off --expected-revision 0
./ccp-bootstrap/bin/ccp-managed --store "$STORE" update
```

`update` explicitly checks official releases, stages a verified client/MCP pair and
selects it only after every configured profile passes the candidate's read-only
preflight. Pending work, migration needs, missing servers or unknown contracts
defer selection. A same-source result reports `up_to_date` without rebuilding or
probing profiles. Workspace access and credential validity remain separate.

Use `--channel nightly` for dated official main snapshots. Add `--pin-tag` to hold
an exact published tag. A pin binds its authenticated source commit when that tag
is already active or first selected; a later moved tag is refused. Omitting the
pin in a subsequent explicit `policy-set` removes it. Use the observed policy
revision for each change. The configuration's exact bytes are bound to the policy;
after deliberately editing its aliases or paths, repeat `policy-set` to confirm
the new scope. The manager never rewrites that file or searches for other profiles.

Downloads use fixed official GitHub routes and bounded HTTPS redirects. Manifest
provenance is checked before executable downloads, and private staged copies are
verified again before building. GitHub commit ancestry must show the candidate is
descended from both current software and a retained source floor. Release dates
and version labels do not establish that order. Unknown, older or divergent
history refuses selection. Discovery is bounded; an unavailable pin or channel
never falls back to another release or source.

The source floor advances before software selection. A failed acknowledgement
can leave that floor advanced while the pointer is old, or leave the new pointer
visible. Retain both generations and inspect `policy-show`, `status` and
`update-status` before another decision. The floor is never automatically restored
or lowered, including after manual rollback. Missing or corrupt policy after its
creation marker exists is refused rather than reset.

## Enable optional automatic checks

Automatic checks require a deliberate local policy change; workspace/server
administrators cannot enable them remotely. Replace the revision with the current
value from `policy-show` and repeat the same intended channel, pin and config:

```bash
./ccp-bootstrap/bin/ccp-managed --store "$STORE" policy-set \
  --config "/absolute/path/to/private/workspace-mcp.json" \
  --channel stable --automatic on --expected-revision 1
./ccp-bootstrap/bin/ccp-managed --store "$STORE" update-status
```

At a managed client/MCP launch, the launcher captures its current pair first. If
enabled and due, it starts a separate bootstrap-manager check with no inherited
stdio. The launching process continues with its captured pair; a successful
update affects later launches. Running MCP processes keep their original client
and environment. Failed checks or scheduling leave that pair usable.

Checks run at most once per 24 hours, when a managed launch occurs. Policy edits
and a backward clock change do not bypass that interval. No daemon or recurring
system job is installed; use `update` for an immediate explicit check. Results are
available through `update-status`; a stale `scheduled` result after a crash does
not establish success. This status describes automatic attempts; an explicit
`update` returns its result directly. The worker rechecks policy revision/opt-in and pointer CAS
before selection. A policy change or disable observed at that check prevents the
attempt from selecting software. A build already started may finish as a retained candidate.

Disable locally even if the configured file or servers are unavailable:

```bash
./ccp-bootstrap/bin/ccp-managed --store "$STORE" policy-show
./ccp-bootstrap/bin/ccp-managed --store "$STORE" policy-disable --expected-revision 2
```

Use the revision actually observed; a successful update also advances it. This
does not change server software/data, migrate profiles, renew or enroll clients,
change selections, restart tasks or upgrade the bootstrap manager. Older official
releases lacking the required provenance/preflight endpoint are refused. Positive
hosted signing/publication of these unreleased components remains a release check.

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
