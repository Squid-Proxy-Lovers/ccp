# ccp-server

The CCP server. Listens for client connections over mTLS, handles enrollment via HTTP, and stores session data in SQLite.

## What it does

Two listeners run at the same time. The HTTP auth listener (default port 1337) handles token redemption and certificate enrollment. The mTLS listener (default port 1338) handles all client operations over a binary frame protocol.

Each session gets its own self-signed CA. Client certificates are issued during enrollment and validated on every connection. Commands include shelf/book/entry CRUD, append, search, delete/restore, import/export, history, and certificate revocation.

## Usage

```bash
ccp-server my-session
```

Or build from source:

```bash
cargo build --release -p server
./target/release/server my-session
```

Issue enrollment tokens:

```bash
ccp-server issue-token <session-name> read
ccp-server issue-token <session-name> read_write
ccp-server issue-token <session-name> admin --ttl 3600
```

## Environment variables

| Variable | Default | What it does |
| --- | --- | --- |
| `CCP_SERVER_DATA_DIR` | `data/` | SQLite database and CA material location |
| `CCP_AUTH_BASE_URL` | `http://127.0.0.1:1337` | Public URL for the auth endpoint |
| `CCP_MTLS_BASE_URL` | `https://localhost:1338` | Public address for mTLS connections |
| `CCP_AUTH_LISTENER_ADDR` | `127.0.0.1:1337` | Bind address for the auth listener |
| `CCP_MTLS_LISTENER_ADDR` | `127.0.0.1:1338` | Bind address for the mTLS listener |
| `CCP_TLS_SERVER_NAMES` | `localhost` | Extra SANs for the server TLS cert |
| `CCP_AUTO_ISSUE_INITIAL_TOKENS` | `1` | Set to `0` to skip auto token generation |
