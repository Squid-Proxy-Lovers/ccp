# Server guide

Deploy the server, issue enrollment tokens, and inspect stored sessions. Each
process starts one selected session. Clients use HTTP for enrollment and a framed
binary protocol over mutually authenticated TLS (mTLS) for data operations.

[Documentation index](README.md) · [Client guide](client.md) · [Nightly builds](nightly-builds.md)

- [Deployment](#deployment)
- [Configuration](#configuration-surface)
- [Local administration](#local-admin-controls)
- [Tokens and certificates](#authentication-model)
- [Architecture and data](#architecture-and-data)
- [Compatibility and limits](#compatibility-and-limits)

## Deployment

The [root README](../README.md#install) covers binary and Docker installation.
Binaries install to `~/.local/bin`; change this with `--install-dir`. For an explicit
source build, install Rust through rustup, Git, and Make, then run:

```bash
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --from-source
```

For a native local server, keep the process running while clients use it:

```bash
ccp-server my-session
```

### Docker

The installer pulls the published GHCR image. A failed pull stops installation;
choose `--docker --from-source` to build locally. The `ccp-server-data` volume stores
data. The installer preserves existing containers.

```bash
docker logs -f ccp-server
docker stop ccp-server
```

For a separate checkout-based Compose deployment, run from the repository root:

```bash
docker compose up -d --build
docker compose logs -f ccp-server
docker compose down
```

Keep the named volume when stopping or replacing a container. `docker compose down`
retains it; adding `--volumes` deletes the persistent data volume.

Set `CCP_SESSION_NAME` for the Compose session. Set `CCP_ADVERTISE_HOST` to the
hostname or IP clients should reach. Compose publishes ports on loopback by default;
`CCP_PUBLISH_HOST` changes that bind address. `CCP_AUTH_PORT` and `CCP_MTLS_PORT`
change the container and published ports. See [docker-compose.yml](../docker-compose.yml)
and the [container entrypoint](../docker/server-entrypoint.sh) for the full mapping.

### Remote enrollment

Set the listener addresses and advertised URLs before starting a remote server.
Client-facing URLs must be reachable by those clients, and the mTLS hostname must
be covered by the server certificate's `CCP_TLS_SERVER_NAMES`.

Protect remote enrollment with HTTPS termination or a trusted tunnel. The native
HTTP listener defaults to loopback and requires
`CCP_ALLOW_NON_LOOPBACK_AUTH_LISTENER=1` for a non-loopback bind. The container
entrypoint enables that setting for its internal listeners; restrict published
ports and configure the external enrollment path appropriately. Normal data traffic
uses mTLS. See the [client guide](client.md#enroll-and-select-a-session) for redemption.

## Configuration surface

Defaults below apply to the native server. Docker's entrypoint supplies its own
listener, data-directory, and advertised-host defaults.

| Variable | Native default | Purpose |
| --- | --- | --- |
| `CCP_SERVER_DATA_DIR` | A discovered existing directory or `sessions/<session-slug>/` | Database, journal, and CA material |
| `CCP_SERVER_HOME` | `sessions` | Parent of per-session directories |
| `CCP_AUTH_BASE_URL` | `http://127.0.0.1:1337` | Advertised enrollment URL base |
| `CCP_MTLS_BASE_URL` | `https://localhost:1338` | Advertised mTLS endpoint |
| `CCP_AUTH_LISTENER_ADDR` | `127.0.0.1:1337` | HTTP bind address |
| `CCP_MTLS_LISTENER_ADDR` | `127.0.0.1:1338` | mTLS bind address |
| `CCP_TLS_SERVER_NAMES` | No extra names | Extra certificate names; localhost and the advertised mTLS host are included |
| `CCP_ALLOW_NON_LOOPBACK_AUTH_LISTENER` | Disabled | Allow HTTP enrollment outside loopback |
| `CCP_AUTO_ISSUE_INITIAL_TOKENS` | `1` | Print initial read/write tokens; set `0` to disable |
| `CCP_ENROLLMENT_TOKEN_TTL_SECONDS` | `3600` | Default token lifetime |
| `CCP_CLIENT_CERT_TTL_SECONDS` | `315360000` | Default client certificate lifetime: ten years |
| `CCP_CA_CERT_TTL_DAYS` | `3650` | Default session CA lifetime |
| `CCP_CERT_WARNING_WINDOW_SECONDS` | `0` | Optional expiry warning window |

`CCP_SESSION_OWNER`, `CCP_SESSION_LABELS`, `CCP_SESSION_VISIBILITY`, and
`CCP_SESSION_PURPOSE` configure session metadata. Existing legacy `data/` storage
can be discovered when it matches the requested session. See
[init.rs](../crates/server/src/init.rs) for resolution and configuration rules.

## Local admin controls

Use the same data directory as the running server when issuing tokens or checking
health (`CCP_SERVER_DATA_DIR` overrides discovery). Replace `my-session` as needed:

```bash
ccp-server issue-token my-session read
ccp-server issue-token my-session read_write
ccp-server issue-token my-session admin --ttl 3600
ccp-server health my-session
```

For an installed Docker container:

```bash
docker exec ccp-server server issue-token my-session read_write
docker exec ccp-server server health my-session
```

Token issuance is a local administrative operation. The [client access table](client.md#session-resolution)
explains `read`, `read_write`, and `admin`, including certificate revocation.

### Health check and diagnostics

Health output includes status, session name, protocol/schema versions, active
session count, issued/revoked certificate counts and lists, and database, journal,
CA, and server certificate paths. Certificate records include identity, access,
and expiry. Use the paths to confirm you are operating on the intended data.

The local health command reads stored session information. The MCP `server_health`
tool runs that server command locally and needs a server binary and access to its
storage. See the [tool-call reference](tool-call-api.md#server_health).

## Authentication model

1. A trusted operator issues a token scoped to a session and access level.
2. The client generates a private key and certificate signing request (CSR).
3. It sends `POST /auth/redeem` with `token` and `csr_pem` JSON fields.
4. The server validates the token, signs the CSR with the session CA, and records
   the issued certificate.
5. The response supplies session metadata, access, client common name, mTLS
   endpoint, expiry, CA certificate, and signed client certificate.
6. Later requests use the saved certificate. The server checks session/access
   grants and revocation; private keys stay on the client.

### Token lifecycle

Tokens are random secrets stored as hashes, scoped to one session and access level.
They default to one hour and remain reusable until expiry. Each redemption issues
a distinct client certificate. Protect the token while it is valid.

### Certificate lifecycle

Each session has its own CA, server certificate, and issued client certificates.
CA and client lifetimes default to ten years; expiry warnings default to disabled.
Expired certificates fail authentication. Operators can revoke certificates;
clients with expired credentials need a new enrollment token. Current main has no
automatic renewal workflow.

## Architecture and data

### Runtime architecture

The HTTP listener redeems tokens and signs CSRs. The mTLS listener verifies client
certificates and processes multiple framed requests on persistent connections.
Server-enforced permissions apply to every normal data operation.

Source entry points are [init.rs](../crates/server/src/init.rs) for bootstrap,
[lib.rs](../crates/server/src/lib.rs) for listeners,
[message.rs](../crates/server/src/message.rs) for dispatch,
[state.rs](../crates/server/src/state.rs) for caches,
[database.rs](../crates/server/src/database.rs) for persistence, and
[journal.rs](../crates/server/src/journal.rs) for mutation replay.

### Session model

A session namespaces shelves, books, and entries. Metadata includes ID, name,
description, owner, labels, visibility, purpose, active state, and lifecycle times.
A process activates or creates its named session. The default layout gives each
session its own directory and `ccp.sqlite3` database. Use separate process endpoints
and enrollments for separate server sessions.

### Wire protocol

TLS carries a four-byte big-endian frame length followed by a Bincode payload.
The [protocol crate](../crates/protocol/README.md) defines requests, responses,
and a handshake that checks protocol compatibility after TLS authentication.

### In-memory state model

Runtime caches hold sessions, certificate grants, revocations, active entries,
and append history. SQLite supplies persisted state on startup. Live reads and
searches use the in-memory state.

### Persistence model

SQLite stores sessions, token records, issued/revoked certificates, active and
deleted entries, and history. A journal records mutations for replay. The server
loads state at startup and writes snapshots during shutdown. Back up the complete
data directory, including CA material, after a clean shutdown.

### Message pack model

The [client entry model](client.md#entry-model) describes the hierarchy and fields.
Active entries are unique by `(session_id, shelf_name, book_name, name)`; omitted
locations select `main/default`. Entries include creation/update times and history
in server state. Deleted content and history use separate archive tables; keys
include shelf, book, name, and deletion time, allowing multiple archived versions.

### Search model

Metadata searches match names, descriptions, labels, shelves, and books and return
summaries. Context searches return matching snippets. Deleted-entry searches query
the SQLite archive. See [client search/history](client.md#search-and-history).

### Export / import model

Export selects a session, shelf, book, or named entries, with optional history.
Import checks the entry digest, requires write access, and applies the chosen
conflict policy. It records a `transfer_log` row and rolls back in-memory import
changes if persistence fails. See the [droplet format](droplet-format.md) for the
bundle fields, selectors, and conflict policies.

### Concurrency and shutdown

Connections run concurrently; a request tracker coordinates shutdown. On a clean
stop, the server stops accepting work, waits for in-flight requests, marks sessions
stopped, stops the journal, writes a SQLite snapshot, and truncates the replay
journal. A panic hook attempts a snapshot on a best-effort basis.

### FastMCP layer

The [Python bridge](../mcp/README.md) invokes the Rust client for enrollment,
discovery, reads, searches, create/append operations, history, export, and briefings.
Destructive operations and server lifecycle management remain CLI-only.

## Compatibility and limits

### Versioning

`PROTOCOL_VERSION` in the protocol crate tracks wire compatibility. Clients send a
handshake; the server accepts matching versions or rejects incompatible requests.
`SCHEMA_VERSION` in `init.rs` tracks database migrations, which apply on startup
and are recorded in `schema_version`. Health output reports both versions.
Changes to these contracts need versioning and migration checks described in
[CONTRIBUTING.md](../CONTRIBUTING.md#dependencies-and-versions).

### SQLite schema responsibilities

Core tables are `sessions`, `auth_tokens`, `issued_client_certs`, `message_packs`,
`message_history`, `deleted_message_packs`, `deleted_message_history`, and
`schema_version`. They hold recovery state, certificate authorization, and archives.
The [schema](../crates/server/src/init-db.sql) defines the complete layout.

### Known limits

The server uses a single-node SQLite model. There is no distributed consensus or
built-in multi-host relay. Panic snapshots are best effort; recovery also uses
journal replay. Archived restoration selects a particular deleted entry by key.
The [archived HTTP branch](archive-notes.md) has different session, transport,
management, and distribution behavior.
