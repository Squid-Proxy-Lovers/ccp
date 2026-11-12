# Changelog

## [0.2.0] - 2026-11-12

### Added

- Scoped workspaces with registered clients, explicit per-workspace grants, and
  owner, admin and instance-admin roles. One client can access several workspaces
  without sharing their administration.
- A TLS-only workspace HTTPS API under `/api/v1`, separate from native sessions,
  with scoped admin keys and registered mTLS client authentication.
- `ccp-server up --data-dir` starts a workspace instance and prints a browser
  first-run setup link. The setup wizard creates the first workspace and returns
  its owner and instance admin keys once. Later starts preserve the identity.
- Enrollment links issued in the panel and redeemed through `ccp enroll`, with
  hidden paste, bounded stdin and explicit-link input. The client verifies the
  CA pin, server name and instance identity before transmitting secrets.
- A public `/join` page with installation and enrollment instructions. Invitation
  fragments are removed from the address bar; opening the page does not redeem them.
- Scoped invitation controls with a 30-minute default, custom expiry, confirmed
  long-lived or non-expiring links, use limits and an initial-group enrollment window.
- Local `server workspace-init`, `admin-info`, `reset-instance-key` and
  `reset-owner-key` commands for provisioning and stopped-server key recovery.
- `server workspace-tls-init`, `workspace-tls-info` and `workspace-tls-renew`
  provision, export and renew instance TLS identity while preserving its CA.
- `ccp-client workspace` adds `trust`, `enroll`, `join`, `renew`, `recover`
  and `selection` commands. Private profiles pin instance trust; recovery creates
  a separate profile and workspace choices remain explicit.
- Workspace owners can appoint admins, issue and revoke scoped keys and transfer
  ownership. Scoped client recovery, targeted readmission and instance-wide disable
  preserve the boundaries of existing grants.
- Transactional shelf, book and client-owned entry storage with revision checks,
  immutable history, archive and restore. Shelf archive/restore recovers containers
  together; archived workspaces remain readable and reject content writes.
- Durable client entry mutations and operation resolution retain retry proof
  independently of entry bodies, so uncertain writes can be resolved after renewal
  or recovery without silently creating another write.
- Confirmed permanent purge of archived entries and optional timed retention.
  Manual retention is the default; workspace archive pauses retention clocks.
- Confirmed deletion of an already archived workspace, with resumable cleanup and
  retained deletion state. Other workspaces and shared client identities survive.
- Global and workspace guidance boards with revision-checked publication, explicit
  client reads and automatic initial or changed guidance alongside MCP results.
- Explicit client-owned work-status messages, guarded update/clear retries and
  scoped administrator moderation. Messages retain their last timestamp when stale.
- A Svelte admin panel at `/admin` with Overview, Context, Clients & access,
  Guidance, Reported work, Settings and instance Workspaces views. It provides
  content editing/history, access management, retention and deletion tracking.
- `POST /api/v1/admin/workspaces/overview` returns scoped content and access counts,
  daily activity and top writers without returning entry bodies or names.
- Panel light, dark and system themes, loading placeholders, locally bundled fonts,
  keyboard focus handoff and an activity chart with a table alternative.
- A separate `ccp-workspace-mcp` stdio entry point configured through private
  `CCP_WORKSPACE_CONFIG` profiles and an explicit `CCP_CLIENT_BIN`.
- Workspace MCP discovery and selection tools, including `workspace_profiles`,
  `workspace_list`, `workspace_selection_open`, `workspace_select` and `workspace_target`.
- Workspace MCP content tools, including `workspace_shelves`, `workspace_books`,
  `workspace_entry`, `workspace_history`, `workspace_create_shelf` and
  `workspace_create_book`, plus shelf lifecycle readers.
- Workspace MCP `workspace_mutate_entry` and pending/resume/resolve controls,
  `workspace_guidance`, and status read, update, clear and pending-operation tools.
- Offline native inventory with `server legacy-inspect` and conversion with
  `server legacy-convert`. Scoped native provenance preserves original source
  rows; HTTP, CLI, MCP `workspace_native_history` and panel readers expose entry history.
- Authenticated release manifests bind matching client/MCP artifacts and build
  metadata to GitHub build provenance. Managed bundles support verified staging,
  pinned launch, compatibility preflight, manual updates and opt-in automatic updates.
- A Windows client (`ccp-client-windows-x86_64.exe`) and PowerShell installer.
  It verifies release assets before installation, installs per user, backs up MCP
  configuration and rolls back failed upgrades. Re-run it to update; the server
  is not supported on Windows.
- Tested nightly builds from `main`, with stable/nightly channel selection and
  exact nightly pins; release candidates require explicit verified installation.
- An optional WebSocket server carrier, off by default. Client support is not
  included, so treat it as experimental.

### Changed

- Native-mode startup loads stored sessions in near-linear time. A session with
  26,000 entries and an 80,000-record journal now starts in under a second
  instead of several minutes in the recorded release test.
- Docker defaults to workspace first-run setup on ports 8443 and 8444 with data
  at `/var/lib/ccp/server`. Explicit native-session arguments remain supported.
- Docker images are published only for release tags. `:latest` now always
  points to the newest stable release, not to the latest push to `main`, and
  release candidates get only their own tag, such as `:v0.2.0-rc.2`.
- Source builds require Rust 1.89+ and Node.js 24/npm for the embedded panel.
  Panel assets are built from locked sources; installed servers need no Node.js.
- Release manifests accept additional future platform metadata while retaining
  strict verification of the selected platform and known artifact contracts.

### Fixed

- Native checkpoints now save the current state before retiring journal records,
  preserving acknowledged entries, appends and newly issued certificates.
- Native journal replay tracks entry generations and recorded operations to avoid
  replaying superseded entries and duplicate appends in the covered recovery cases.
- A failed native import, restore or delete no longer rolls back another client's
  concurrent change. Deleted entries no longer reappear through that rollback race.
- A failed native import with the same path listed twice restores its original
  value, including history when merging append records.
- Native certificate revocation no longer blocks unrelated requests while saving.
- Docker release publishing works with the required lower-case GHCR image name;
  Docker installation no longer requires Compose or silently falls back to a source build.
- Failed staged installs preserve the previously matched binaries and MCP package.
  Installer destination validation rejects unsafe directory and linked-parent cases.
- Stable managed-update discovery and the Windows installer reject prerelease
  tags instead of treating them as stable updates.
- Panel creation drafts survive navigation, confirmed creates remain acknowledged
  when refresh fails, and newly created shelves/books remain visible beyond page one.
- Panel refresh and navigation discard stale responses and move keyboard focus
  to the active view or item. Discarded append notes are removed from recovery copies.
- One-time owner keys and enrollment links remain available until explicitly
  dismissed, and cannot cross an administrator identity switch. Pending access
  writes settle even after their view unmounts or session locks.
- Panel requests have a 60-second deadline and preserve an uncertain write for
  explicit recovery. Key-fragment sign-in asks before switching identities or losing drafts.

### Release candidates

- RC publication marks GitHub releases as prereleases and keeps stable downloads
  and the Docker `:latest` tag unchanged. Docker publishing requires successful
  version validation and provenance verification.
- Testers can install matching verified client/MCP assets or pin an exact RC
  Docker image; see [RC installation](docs/client-release-bundles.md#install-a-release-candidate).

### Security

- Includes the 0.1.1 certificate revocation fix: repeated or concurrent native
  revokes cannot clear another request's in-memory revocation mark and restore access.
- Native revoke, import, restore and delete serialize state changes so a failed
  persistence rollback cannot undo another client's acknowledged mutation.
- Bundled SQLite is now 3.54.1 (rusqlite 0.41.1), fixing RUSTSEC-2026-0459.
- Updated rustls and related dependencies include certificate distrust protections.
- Workspace mutations check current authority and record audit events within the
  same transaction. Native/workspace directory leases prevent mixing runtime modes.
- Browser admin sessions use bounded in-memory cookies, current scoped-key checks,
  Origin validation and CSRF protection. The panel CSP permits only same-origin
  assets and disallows inline scripts/styles, framing and form submission.
- Managed client bundles and Windows installs verify publisher provenance and
  SHA-256 before activation. Workspace enrollment links cannot authorize software downloads.
- The panel build pins `@oss-cve-patches/deepmerge@4.3.2` under the Svelte Vite
  plugin, fixing prototype pollution (GHSA-gcqx-w48m-w2h4) without an audit exclusion.
  This is a build-time override, recorded in the npm lockfile.

### Removed

- The old embedded admin page is replaced by the source-built Svelte panel.
- Pushes to `main` no longer publish the Docker stable `:latest` image; use the
  separate nightly channel for previews.

### Upgrading from 0.1.x

- Building from source now needs Rust 1.89 or later; 0.1.x needed 1.88. Install
  the panel's npm dependencies before building the server.
- Back up the complete data directory after a clean shutdown and retain the
  matching producing binary. Rollback requires that backup; do not lower schema markers.
- The native wire protocol stays at version 1, so existing native deployments
  keep working. Workspaces use schema 27 and do not adopt native data in place.
- To move a native instance, stop it and run `server legacy-convert` into a new
  location, following [the conversion guide](docs/native-conversion.md).
  Conversion leaves source files unchanged and preserves native source history.
- The converted instance needs fresh TLS provisioning and administration. Old
  client certificates and tokens grant no access; enroll each client again.
  Imported entries have no registered-client owner and remain admin-managed.
- Only complete native schema-2 data can be converted. Data from the unreleased
  shared-HTTP schemas 3/4 and mixed workspace databases cannot.
- Configure `ccp-workspace-mcp` explicitly for workspace profiles. The existing
  `ccp-mcp-server` remains the native session entry point.

## [0.1.1] - 2026-11-11

### Security

**Revoked client certificates could regain access (native mode).** Severity: Medium, CVSS 4.0 `CVSS:4.0/AV:N/AC:L/AT:P/PR:L/UI:N/VC:L/VI:L/VA:N/SC:N/SI:N/SA:N`. Affects 0.1.0 and earlier. Fixed in 0.1.1 and 0.2.0. No CVE was requested.

- **Cause.** The revoke handler marked a certificate as revoked before it checked that the certificate still had a grant. If it found no grant, it removed the mark again.
- **Impact.** Any of the following could leave a revoked certificate accepted in memory, with its previous read or write access, until the server restarted:
  - a repeated revoke;
  - two concurrent revokes;
  - a client sending concurrent revoke requests for its own certificate. This one needs no admin rights, and afterwards no admin revoke worked until a restart.

  The database kept the revocation, so a restart re-applied it.
- **Fix.** The handler now checks the grant while holding the grant lock, marks the certificate revoked only after that check, and undoes only a mark it added itself. Native mutations are serialized, so a rollback can no longer undo another client's concurrent write.
- **Workaround for 0.1.0.** Restart the server after revoking a certificate. A `revoke-cert` that reports "not found" for a certificate whose `revoked_at` is set in the database means the in-memory revocation may have been lost until the next restart.
- **Credit.** Reported by [kta1kri](https://github.com/kta1kri).

### Fixed

- A failed database write during an import, restore or delete could roll back another client's concurrent change. A deleted entry could reappear until restart, or an acknowledged append could be lost.
- Revoking a certificate no longer makes every other request wait while the revocation is saved.
- Checkpoints no longer drop acknowledged writes that were only in the journal, such as new entries, appends or newly issued certificates.
- A failed import of a bundle that lists the same path twice now restores the original value.
- Docker images are published again for release tags (`ghcr.io/squid-proxy-lovers/ccp:latest` and `:v0.1.1`). The image name was not lower case, so every publish since 0.1.0 had failed.

### Known issues

- In rare crash or concurrency windows, native-mode journal recovery can replay a write twice or restore a just-deleted entry. Version 0.2.0 replaces the journal recovery design.
