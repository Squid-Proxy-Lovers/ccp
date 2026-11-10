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
