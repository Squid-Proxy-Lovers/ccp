# Server Design Specification

## Scope

The CCP server is a Rust coordination service for inter-agent communication. It exposes:

- an HTTP enrollment endpoint
- a persistent framed binary protocol over mTLS
- SQLite-backed persistence
- in-memory runtime state for the hot path
- optional FastMCP management through the Python bridge

## Design Goals

- fast runtime reads and writes from memory
- durable recovery through SQLite
- explicit bootstrap-to-mTLS auth lifecycle
- session-scoped coordination state
- simple single-node deployment
- straightforward local-first onboarding

## Runtime Architecture

The server has two listeners:

- auth listener
- mTLS message listener

Auth listener responsibilities:

- redeem time-limited enrollment tokens
- validate CSR-based enrollment requests
- issue signed client certificates

mTLS listener responsibilities:

- accept client-cert-authenticated TLS connections
- keep connections open
- process multiple framed requests per connection

Core runtime modules:

- `init.rs`
- `auth_request.rs`
- `lib.rs`
- `message.rs`
- `state.rs`
- `journal.rs`

## Session Model

A session is the primary namespace boundary.

Session metadata includes:

- `id`
- `name`
- `description`
- `owner`
- `labels`
- `visibility`
- `purpose`
- `is_active`
- lifecycle timestamps

Each server process is started with a session name and activates or creates that session.

For standalone operation, the server now defaults to one session data directory per session name, so each session gets its own `ccp.sqlite3` file unless `CCP_SERVER_DATA_DIR` is explicitly set.

## Authentication Model

CCP uses a bootstrap-and-cert flow.

Bootstrap:

- server exposes one fixed redeem endpoint: `POST /auth/redeem`
- enrollment tokens are issued only by a trusted local server-side admin utility
- MCP tools cannot mint tokens, manage servers, or perform destructive operations (those are CLI-only)

Enrollment flow:

1. client receives:
   - `auth_redeem_url`
   - a time-limited token
2. client generates a private key locally
3. client generates a CSR locally
4. client sends JSON:
   - `{"token":"...","csr_pem":"..."}`
5. server atomically validates and consumes the token
6. server signs the CSR with the session CA
7. server records the issued cert and returns JSON containing:
   - session metadata
   - access level
   - client common name
   - mTLS endpoint
   - client cert expiry
   - CA certificate PEM
   - signed client certificate PEM

Normal access:

- client connects to the mTLS endpoint
- rustls verifies the chain
- server extracts session/access identity from the client certificate
- server authorizes each request against issued-cert and revocation state

## Token Lifecycle

Enrollment tokens are not reusable session URLs.

Token properties:

- opaque random secret
- stored only as a hash
- scoped to one session
- scoped to one access level
- reusable until expiry
- short-lived

Default policy:

- token TTL: `1 hour`

Redemption rules:

- token must exist
- token must not be expired
- token must not be consumed

If redemption fails after the token is consumed, the token stays invalid and a new one must be issued.

## Certificate Lifecycle

The server maintains three certificate lifetimes:

- session CA
- server certificate
- client certificates

Default policy:

- CA TTL: `10 years`
- client cert TTL: `10 years`
- warning window: disabled by default

Practical model:

- a redeemed client identity is intended to remain usable for normal local deployment without routine renewal
- expired certificates still fail closed
- manual revocation remains available
- future renewal policy can be added later without changing the bootstrap flow

## Local Admin Controls

Local admin utility:

```bash
server issue-token <session> <read|read_write|admin> [--ttl <seconds>]
server health <session>
```

This utility is the trusted path for issuing enrollment tokens.

Other admin controls:

- revoke client certificate
- inspect server status
- start and stop managed local servers

## Local-First Operation

A local deployment is a first-class mode.

Typical local flow:

1. start the server for a session
2. issue a token locally on the server host
3. redeem the token from the client
4. use the saved client cert for all normal work after that

This keeps onboarding simple while preserving a strict time-limited bootstrap step.

## Wire Protocol

The server uses a framed binary protocol defined in the shared `protocol` crate.

Transport format:

- TCP or tunneled TCP under TLS
- 4-byte big-endian frame length
- bincode payload
- persistent request/response loop per TLS session

## In-Memory State Model

Hot runtime state lives in memory:

- sessions
- issued cert cache
- revocation set
- active message packs
- append history

Normal protocol authorization should not require SQLite queries on every request.

## Persistence Model

SQLite is the durable backing store.

The server loads durable state at startup and snapshots runtime state back to SQLite.

In the normal local layout, there is one SQLite database per session data directory.

Durable state includes:

- sessions
- auth token records
- issued client cert records
- revoked cert records
- active message packs
- deleted message packs
- append history

The runtime journal captures hot-path mutations for crash recovery.

## Message Pack Model

Active message packs contain:

- `name`
- `shelf_name`
- `book_name`
- `description`
- `labels`
- `context`
- `created_at`
- `updated_at`
- append history

Runtime uniqueness:

- one active chapter per `(session_id, shelf_name, book_name, name)`
- missing shelf/book values normalize to `main/default`

Deleted-entry archive:

- deleted entries move to `deleted_message_packs`
- deleted history moves to `deleted_message_history`
- deleted primary key is `shelf::book::name::deleted_at`

This allows:

- active runtime uniqueness by full chapter path
- multiple archived deleted versions of the same chapter path in SQLite

## Search Model

Active entry search:

- `search_entries` matches `shelf_name + book_name + name + description + labels`
- returns summaries only

Active context search:

- `search_context` matches `context`
- returns `name`, `shelf_name`, `book_name`, `description`, and snippets around matches

Deleted entry search:

- searches archived deleted entries in SQLite
- matches `shelf_name + book_name + name + description + labels`

## Security Model

- bootstrap tokens are separate from normal transport auth
- client private keys stay on the client host
- normal operations require mTLS
- access is scoped by issued client certificate
- revocation is server-controlled
- additional enrollments require a valid time-limited token

## Health Check and Diagnostics

The server exposes health check capabilities through the local admin utility:

```bash
server health <session>
```

This returns JSON containing:

- `status` - server status (e.g., "healthy")
- `session_name` - session identifier
- `active_sessions` - number of active sessions in the database
- `issued_certs` - count of non-revoked issued client certificates
- `revoked_certs` - count of revoked certificates
- `database_path` - SQLite database file path
- `journal_path` - write-ahead journal file path
- `ca_cert_path` - CA certificate path
- `server_cert_path` - server certificate path
- `issued_certs_list` - array of certificate details (CN, access level, expiry)
- `revoked_certs_list` - array of revoked certificate common names

This is useful for:

- monitoring server uptime
- tracking issued and revoked certificates
- inspecting persistence layer paths
- debugging session state

## Export / Import Model

Export bundle contains:

- source session metadata
- transfer selector (scope + history flag)
- export timestamp
- selected entries with shelf/book placement and labels
- optional append history
- SHA-256 integrity hash over the serialised entries

Export scopes: full session, shelf, book, or named entries within a book.

Import behavior:

- verifies SHA-256 hash before applying any changes
- target session-scoped; requires `read_write`
- four conflict policies: `error` (default), `overwrite`, `skip`, `merge-history`
- rolls back in-memory state if SQLite persist fails
- records an audit row in `transfer_log`

## Concurrency and Shutdown

The server accepts connections concurrently but coordinates shutdown through a request tracker.

Shutdown flow:

1. begin shutdown signal
2. stop accepting new work
3. wait for in-flight requests to finish
4. mark sessions stopped
5. stop journal
6. write SQLite snapshot
7. truncate replay journal

There is also a panic hook that attempts a best-effort snapshot.

## Versioning

Two version numbers matter at runtime:

- `PROTOCOL_VERSION` (defined in the `protocol` crate) tracks the wire format. The client sends a `Handshake` request with its protocol version after the TLS handshake. The server responds with `HandshakeOk` if compatible or `HandshakeRejected` if not. Bump this when `ClientRequest` or `ServerResponse` change in a backward-incompatible way.

- `SCHEMA_VERSION` (defined in `init.rs`) tracks the SQLite schema. Recorded in the `schema_version` table after migrations run. The health endpoint includes both versions.

Migrations are applied on every startup. The `schema_version` table tracks which version has been applied so migrations only run forward.

## SQLite Schema Responsibilities

Core tables:

- `schema_version`
- `sessions`
- `auth_tokens`
- `issued_client_certs`
- `message_packs`
- `message_history`
- `deleted_message_packs`
- `deleted_message_history`

SQLite roles:

- long-term durability
- restart recovery base image
- deleted-entry archive
- revocation state
- rotated token state

## FastMCP Layer

The Python FastMCP bridge is not the core server. It is an operator and client-access bridge.

It provides:

- enrollment helper access
- session discovery
- entry read/search/create/append operations
- history and export
- server health diagnostics

Destructive operations (delete, import, revoke, restore) and server management (start, stop, restart, rename, delete session) are not exposed to agents. Those go through the CLI.

The Rust server remains the source of truth for protocol behavior.

## Configuration Surface

Important environment controls:

- `CCP_SERVER_DATA_DIR`
- `CCP_AUTH_BASE_URL`
- `CCP_MTLS_BASE_URL`
- `CCP_AUTH_LISTENER_ADDR`
- `CCP_MTLS_LISTENER_ADDR`
- `CCP_TLS_SERVER_NAMES`
- `CCP_SESSION_OWNER`
- `CCP_SESSION_LABELS`
- `CCP_SESSION_VISIBILITY`
- `CCP_SESSION_PURPOSE`

## Security Properties

- no message operation without mTLS
- read and read-write access levels enforced server-side
- client identity bound to certificate CN
- revocation handled by server state
- auth bootstrap tokens separate from message transport

## Known Limits

- single-node design
- SQLite durability model, not distributed consensus
- no built-in multi-host relay layer yet
- crash snapshot is best-effort; journal replay handles hard-stop recovery
- deleted-entry primary key is timestamp-based, so restore is archive-entry specific rather than semantic-version aware
