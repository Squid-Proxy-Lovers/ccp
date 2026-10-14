# Client guide

Use the Rust CLI or SDK to enroll with a server and work with shared context.
The client saves credentials locally and sends data operations to the server.

[Documentation index](README.md) · [Server guide](server.md) · [MCP setup](../mcp/README.md)

- [Enroll and select a session](#enroll-and-select-a-session)
- [CLI reference](#cli-reference)
- [Entry model](#entry-model)
- [Search and history](#search-and-history)
- [Credentials and permissions](#credentials-and-permissions)
- [Rust SDK](#rust-sdk)
- [Transport and failures](#transport-and-failures)

## Enroll and select a session

Install the client using the [README instructions](../README.md#install). The
installer supports Linux and macOS on x86_64 and ARM64. Ask the server operator for
a `read` or `read_write` enrollment token. Replace `YOUR_TOKEN` below; this example
uses the local server's enrollment endpoint:

```bash
ccp-client enroll --redeem-url http://127.0.0.1:1337/auth/redeem --token YOUR_TOKEN
ccp-client sessions
```

For a remote server, use the operator's HTTPS or tunneled enrollment URL.
Enrollment creates a private key on your machine, submits a certificate signing
request (CSR), and saves the issued certificate, session metadata, and server CA.
The private key stays on your machine. Tokens can be redeemed repeatedly until
expiry; each redemption creates a distinct client identity.

Use the session name or ID from `sessions` in later commands. The client selects
the newest saved enrollment compatible with the operation's required access.
To forget local credentials for a session:

```bash
ccp-client delete-session my-session
```

This removes matching local enrollments. The server's session and data remain.
For multiple server sessions, enroll separately with each server endpoint. The
[archived HTTP branch](archive-notes.md) used a different subscription workflow.

## CLI reference

Commands below follow `ccp-client`. Replace values in angle brackets when using
the reference. Run `ccp-client COMMAND --help` for arguments and defaults.
The [quick start](../README.md#quick-start) is a complete create/append/search example.

### Read and discover

| Command | Result |
| --- | --- |
| `sessions` | Saved client sessions and access levels |
| `list <session>` | Active entry summaries |
| `get <session> <name> [--shelf <shelf>] [--book <book>]` | One full entry |
| `history <session> <name> [--shelf <shelf>] [--book <book>]` | Append history |
| `search-entries <session> <query>` | Matching entry names, descriptions, labels, and paths |
| `search-shelves <session> <query>` | Matching shelf names and descriptions |
| `search-books <session> <query>` | Matching book names and descriptions |
| `search-context <session> <query>` | Matching text with snippets |
| `search-deleted <session> [<query>]` | Archived deleted entries |
| `brief-me <session>` | Structure, counts, ten recent entries, and frequent labels |
| `get-entry-at <session> <name> --at <timestamp> [--shelf <shelf>] [--book <book>]` | Content reconstructed at a Unix timestamp in seconds |
| `export <session> [--output <file>]` | A JSON droplet; defaults to stdout |

Export selectors and complete examples are in the [droplet guide](droplet-format.md).

### Write and administer

| Command | Effect |
| --- | --- |
| `add-shelf <session> <shelf> <description>` | Create a shelf or update its description |
| `add-book <session> --shelf <shelf> <book> <description>` | Create a book or update its description |
| `add-entry <session> --shelf <shelf> --book <book> <name> <description> [--labels <a,b>] <data>` | Create an entry in an existing shelf and book |
| `append <session> <name> [--shelf <shelf>] [--book <book>] <content>` | Append text and record history |
| `delete <session> <name> [--shelf <shelf>] [--book <book>]` | Archive an entry |
| `delete-shelf <session> <shelf>` | Remove a shelf and its contents |
| `restore <session> <entry-key>` | Restore an archived entry to its original path |
| `import <session> <file> [--policy error\|overwrite\|skip\|merge-history]` | Import a droplet |
| `revoke-cert <session> <client-common-name>` | Revoke a certificate under the caller's access rules |
| `delete-session <session>` | Forget local enrollments |

Use the `entry_key` returned by `search-deleted` when restoring. Restoration fails
if an active entry already occupies that path. Destructive commands and server
lifecycle operations are available through the CLI; the [MCP bridge](../mcp/README.md)
exposes read, search, create, append, and export tools.

## Entry model

```text
Session
  └── Shelf: a topic, such as research or logs
        └── Book: related entries, such as findings or runbooks
              └── Entry
                    ├── context: text content
                    ├── description and labels
                    └── history: append author, time, and reason
```

An entry's identity is `(session, shelf, book, name)`. Names can repeat in different
books. Omitted shelf and book values select `main/default`. Entry responses also
include shelf and book descriptions.

Appends extend content and record history. Deleted entries are archived separately,
so their names can be reused for new active entries. Imports can replace content
when the operator selects the `overwrite` policy.

## Search and history

`search-entries` matches names, descriptions, labels, shelves, and books and returns
summaries. `search-context` matches entry text and returns snippets. `search-deleted`
queries the archive; omit its query to list deleted entries.

`history` returns append records with `operation_id`, `client_common_name`,
`created_at`, and optional agent, host, and reason metadata. To attribute an append:

```bash
CCP_AGENT_NAME=reviewer CCP_AGENT_HOST=workstation CCP_APPEND_REASON=follow-up \
  ccp-client append my-session day1 --shelf notes --book findings "Additional finding"
```

Agent and host names are supplied labels. The client certificate authenticates
the caller. The [tool-call reference](tool-call-api.md#get_entry_at) describes
point-in-time reads and their reconstruction limits.

## Credentials and permissions

### Local storage model

Credentials live under `~/.ccp-client/enrollments/`. Set `CCP_CLIENT_HOME` to use
another client home. Each `<session-name>--<access>--<client-cn>` directory contains:

| File | Contents |
| --- | --- |
| `metadata.json` | Session name/ID/description, owner, labels, visibility, purpose, access, client CN, mTLS endpoint, certificate expiry, and enrollment time |
| `ca.pem` | Server CA certificate |
| `client.pem` | Issued client certificate |
| `client.key` | Locally generated private key |
| `identity.pem` | Client certificate and private key for TLS |

Protect private-key files and backups. Entries and append history live on the server.

### Session resolution

| Access | Allowed operations |
| --- | --- |
| `read` | Read, list, search, history, briefing, and export |
| `read_write` | Read operations plus create, append, delete, restore, import, and self-revocation |
| `admin` | Write operations plus revocation of other client certificates |

Read-only clients need an administrator to revoke their certificates. The server
checks session scope, access, and revocation on requests.

### Certificate lifetime

Client certificates default to ten years. Expired certificates fail authentication;
ask for a new enrollment token. `CCP_CERT_WARNING_WINDOW_SECONDS` enables advance
expiry warnings; it defaults to zero. Automatic renewal is currently unavailable.
The [server guide](server.md#certificate-lifecycle) covers issuer configuration.

## Rust SDK

The [client crate](../crates/client/README.md) also builds as a library. It uses the
same saved enrollments and `CCP_CLIENT_HOME` as the CLI. Run this after enrolling:

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

## Transport and failures

The client uses persistent TLS connections, a four-byte big-endian frame length,
and Bincode-encoded request/response types from the [protocol crate](../crates/protocol/README.md).
It verifies the server's certificate chain against the enrolled CA and its name
against the enrolled endpoint. The server verifies client certificates and grants.

Calls can fail because an enrollment is missing, a token or certificate is invalid,
a TLS handshake or frame fails, or the server rejects an operation. Keep the
server's returned error when diagnosing a failure. Enrollment files use ordinary
filesystem storage; CLI interaction is driven by arguments.
