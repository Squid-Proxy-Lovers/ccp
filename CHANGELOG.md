# Changelog

## 0.2.0

### Added

- Scoped workspaces with registered clients, explicit grants, and owner, admin,
  and instance-admin roles.
- Transactional workspace, shelf, book, and entry storage with revisions,
  history, archive and restore, retention, and confirmed deletion.
- A desktop admin panel at `/admin`.
- `ccp-server up` with browser-based first-run setup, and enrollment links
  redeemed with `ccp enroll`.
- A separate workspace MCP entry point, with guidance delivery and status
  messages.
- Managed client installs and updates that verify release provenance.
- Offline conversion of native instances with `server legacy-convert`.
- An optional WebSocket server carrier, off by default. Client support is not
  included, so treat it as experimental.

### Security

- Bundled SQLite is now 3.54.1 (rusqlite 0.41.1), fixing RUSTSEC-2026-0459.

### Upgrading from 0.1.0

Building from source now needs Rust 1.89 or later; 0.1.0 needed 1.88.

The native wire protocol stays at version 1, so existing native deployments
keep working. Workspaces use a new database schema (27) and do not adopt native
data in place.

To move a native instance to workspaces, stop it and run
`server legacy-convert` into a new location, following
[the conversion guide](docs/native-conversion.md). Conversion leaves the source
files unchanged. The new instance has its own TLS identity and administration,
so old client certificates do not work on it. Enroll each client again.

Only complete native schema 2 data can be converted. Data from the unreleased
shared-HTTP drafts and mixed workspace data cannot.

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
