# Client Design Specification

## Scope

The CCP client is a Rust CLI that stores enrollment material on disk and talks to a CCP server over the framed mTLS protocol.

The client is not a stateful coordinator. Its local responsibility is limited to these principles:

- enrollment metadata
- CA certificate
- issued client certificate
- client private key generated locally

## Design Goals

Specifically for the client we had these goals in mind:

- local key ownership
- explicit, argument-driven CLI usage
- simple install-once workflow
- session selection from saved enrollments
- strong mTLS identity for all normal protocol traffic

## Local Storage Model

Default client home:

- `~/.ccp-client`

Override:

- `CCP_CLIENT_HOME`

Enrollment layout:

- `enrollments/<session-name>--<access>--<client-cn>/metadata.json`
- `enrollments/<session-name>--<access>--<client-cn>/ca.pem`
- `enrollments/<session-name>--<access>--<client-cn>/client.pem`
- `enrollments/<session-name>--<access>--<client-cn>/client.key`
- `enrollments/<session-name>--<access>--<client-cn>/identity.pem`

Each enrollment stores:

- `session_name`
- `session_id`
- `session_description`
- `owner`
- `labels`
- `visibility`
- `purpose`
- `access`
- `client_cn`
- `mtls_endpoint`
- `client_cert_expires_at`
- `enrolled_at`

## Enrollment Flow

Bootstrap enrollment is token-based. Normal protocol traffic is certificate-based.

Flow:

1. A trusted server-side admin path issues a time-limited enrollment token.
2. The client receives:
   - `redeem_url`
   - `token`
3. The client generates its own private key locally.
4. The client generates a CSR from that key.
5. The client sends:
   - `POST /auth/redeem`
   - JSON body: `{"token":"...","csr_pem":"..."}`
6. The server atomically consumes the token.
7. The server signs the CSR and returns JSON containing:
   - session metadata
   - access level
   - client common name
   - mTLS endpoint
   - client cert expiry
   - CA certificate PEM
   - signed client certificate PEM
8. The client stores the enrollment material locally.

Important properties:

- the token is reusable until expiry
- the token expires if unused
- the server does not return a client private key
- after enrollment, the saved client identity is intended to remain usable for normal local use without routine renewal

## CLI Interface

Enrollment:

```bash
client enroll --redeem-url https://host/auth/redeem --token <token>
```

Session discovery:

```bash
client sessions
```

Forget a saved session locally:

```bash
client delete-session <session>
```

### Operation Classifications

All normal operations should exist in the MCP.

Normal operations:

- `list`
- `get`
- `search-entries`
- `search-shelves`
- `search-books`
- `search-context`
- `search-deleted`
- `add-shelf`
- `add-book`
- `add-entry`
- `append`
- `history`
- `export`

Destructive operations (CLI-only, not available through MCP):

- `delete`
- `delete-shelf`
- `restore`
- `import`
- `revoke-cert`

## Rust SDK

The `crates/client` crate also builds as a Rust library.

Example:

```rust
use client::{AddBookRequest, AddEntryRequest, AddShelfRequest, CcpClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = CcpClient::new();
    let session = client.writable_session("my-session")?;

    session
        .add_shelf(AddShelfRequest {
            shelf_name: "engineering".to_string(),
            shelf_description: "Platform engineering notes".to_string(),
        })
        .await?;

    session
        .add_book(AddBookRequest {
            shelf_name: "engineering".to_string(),
            book_name: "runbooks".to_string(),
            book_description: "Operational references".to_string(),
        })
        .await?;

    let entry = session
        .add_entry(AddEntryRequest {
            shelf_name: "engineering".to_string(),
            book_name: "runbooks".to_string(),
            entry_name: "build-notes".to_string(),
            entry_description: "Local SDK example".to_string(),
            entry_labels: vec!["rust".to_string()],
            entry_data: "typed client call".to_string(),
        })
        .await?;

        println!("created {}", entry.name);
    Ok(())
}
```

The SDK uses the same saved enrollment store as the CLI and respects `CCP_CLIENT_HOME`.

## Transport Design

The client uses a binary framed protocol over TLS with client certificate authentication.

Properties:

- persistent connection
- multiple requests per TLS session
- 4-byte big-endian frame length header
- bincode payload encoding
- shared request/response model through the `protocol` crate

The client verifies:

- server cert chain against the enrolled CA
- server name against the enrolled endpoint host

The server verifies:

- client cert chain against the same CA
- session scope in certificate identity
- access scope in certificate identity
- revocation status

## Session Resolution

The client resolves a session selector against saved enrollments.

Accepted selectors:

- `session_name`
- `session_id`

Resolution rules:

- read operations accept `read`, `read_write`, or `admin`
- write operations require `read_write` or `admin`
- revoke-cert: requires `read_write` or `admin`; read-only clients need an admin to revoke their cert
- if multiple compatible enrollments exist, the newest compatible one wins
- `delete-session` removes all saved enrollments matching the selected `session_name` or `session_id`

## Certificate Lifetime

Client certificates have explicit expiry, but the default is intentionally long-lived so installation is effectively one-and-done for typical local use.

Default policy:

- client cert TTL: `10 years`
- warning window: disabled by default

Client behavior:
- if the cert is expired, the client refuses to connect and tells you to request a new enrollment token
- optional warning behavior can be enabled later through `CCP_CERT_WARNING_WINDOW_SECONDS`

## Entry Model

Active coordination data is hierarchical:

- shelves
- books within shelves
- entries within books

An active entry contains:

- `name`
- `shelf_name`
- `book_name`
- `description`
- `labels`
- `context`
- append history

Constraints:

- entries are unique by `(shelf_name, book_name, name)` within a session
- deleted entries are archived separately and may reuse names after deletion
- missing shelf/book values default to `main/default`

## Search Behavior

`search-entries`

- searches active `name + description + labels`
- also matches `shelf_name + book_name`
- returns entry summaries with shelf/book metadata

`search-context`

- searches active `context`
- returns:
  - `name`
  - `shelf_name`
  - `book_name`
  - `description`
  - context snippets around matches

`search-deleted`

- searches archived deleted entries in SQLite
- matches `shelf_name + book_name + name + description + labels`

## Append Metadata

Append operations can include:

- `agent_name`
- `host_name`
- `reason`

The client supplies these through environment variables:

- `CCP_AGENT_NAME`
- `CCP_AGENT_HOST`
- `CCP_APPEND_REASON`

The server stores them in append history together with:

- `operation_id`
- `client_common_name`
- `created_at`

## Export / Import

Export produces a signed JSON bundle containing:

- session metadata
- transfer selector (scope + history flag)
- export timestamp
- selected entries with shelf/book placement, labels, and history
- SHA-256 integrity hash over the entries

### Export scopes

| Flag combination | Scope |
|---|---|
| _(none)_ | full session |
| `--shelf <shelf>` | one shelf |
| `--shelf <shelf> --book <book>` | one book |
| `--shelf <shelf> --book <book> --entry <name>` | specific entries (repeatable) |
| `--no-history` | strip append history from bundle |

### Conflict policies

Import accepts `--policy` to control collision handling:

| Policy | Behaviour |
|---|---|
| `error` _(default)_ | abort if any entry already exists |
| `overwrite` | replace existing entries |
| `skip` | leave existing entries untouched |
| `merge-history` | keep existing entry, union history rows by operation\_id |

Import verifies the SHA-256 hash before applying any changes, and rolls back in-memory state if the SQLite persist fails. Requires `read_write` access.

## Failure Model

Client-side failure modes:

- missing compatible enrollment
- auth bootstrap failure
- TLS handshake failure
- framed protocol decode failure
- server-side auth or validation errors surfaced as typed errors

The client does not attempt local write recovery because it is not the source of truth.

## Security Model

- time-limited tokens bootstrap enrollment
- all normal operations require mTLS
- access level is enforced server-side
- cert revocation is server-controlled
- client private keys are generated and stored locally

## Current Limitations

- enrollment storage is file-based rather than OS keychain-backed
- there is no interactive prompt layer; CLI usage is explicit
- there is no automated renewal workflow; long-lived certs are the default